//! File-inspection / viewer-data commands.

use domainator_config::ColumnSpec;
use domainator_core::app_support::contig_view::{build_contig_file_view, ContigFileView};
use domainator_core::app_support::hmm_logo::{
    index_hmm_file, logo_from_hmm_bytes, read_hmm_byte_ranges, HmmEntry, HmmLogo, HmmSummary,
};
use domainator_core::app_support::provenance::{trace_source_sequences, SourceTrace};
use domainator_core::app_support::summary_stats::SummaryStats;
use domainator_core::app_support::text_file::{detect_eol, from_lf, to_lf, Eol};
use domainator_core::app_support::workspace::{classify_file, find_workspace_root};
use domainator_core::io::fasta::{read_fasta, scan_fasta_ids};
use domainator_core::io::genbank::{read_genbank, scan_genbank_ids, write_genbank};
use domainator_core::io::{detect_filetype, FileType};
use domainator_core::matrix::read_matrix;
use domainator_core::search::hmm_write::write_hmm_output;
use domainator_core::seq::partial::Partial;
use domainator_core::seq::SeqRecord;
use domainator_core::tools::build_ssn_viewer::render_html;
use domainator_core::tools::enum_report::{
    enum_report_with_targets, needs_taxonomy, By, OutputFormat,
};
use domainator_core::tools::matrix_report::{
    matrix_report, to_viewer_json, DEFAULT_MAX_MERGE, DEFAULT_MERGE_IMPACT_METRIC,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Load a GenBank or FASTA file as a drawable [`ContigFileView`] (features with
/// resolved colors, flattened filter keys, tooltips). Errors for non-sequence
/// file types (matrices/HMMs use their own viewers).
#[tauri::command]
pub async fn load_contig_view(path: String) -> Result<ContigFileView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let kind = detect_filetype(std::path::Path::new(&path))
            .ok_or_else(|| format!("not a sequence file: {path}"))?;
        let records = match kind {
            FileType::Genbank => {
                read_genbank(&path).map_err(|e| format!("reading GenBank: {e}"))?
            }
            FileType::Fasta => {
                read_fasta(&path, None).map_err(|e| format!("reading FASTA: {e}"))?
            }
        };
        Ok(build_contig_file_view(&path, &records))
    })
    .await
    .map_err(|e| format!("loading contigs failed: {e}"))?
}

/// Build a matrix report for the SSN threshold-selection view. Returns the same
/// payload as the CLI's `matrix-report --json` (via `to_json`, which is
/// parity-locked to the Python output) plus a GUI-only `histogram` of edge scores
/// computed here from `MatrixReport.scores_desc`.
#[tauri::command]
pub async fn load_matrix_report(path: String) -> Result<Value, String> {
    // Off the UI thread: reading + reporting a large matrix is expensive, and a
    // synchronous command runs on Tauri's main thread — blocking it stalls ALL
    // other IPC (so every tool dialog would sit at "Loading…" meanwhile).
    tauri::async_runtime::spawn_blocking(move || {
        let matrix = read_matrix(&path).map_err(|e| format!("reading matrix: {e}"))?;
        let report = matrix_report(&matrix, DEFAULT_MERGE_IMPACT_METRIC, DEFAULT_MAX_MERGE);
        let histogram = histogram(&report.scores_desc, 40);
        // The viewer payload, not the agent-facing --json contract: this tab draws
        // the per-threshold curves that upstream's --json does not carry.
        let mut payload = to_viewer_json(&report);
        if let Value::Object(map) = &mut payload {
            map.insert("histogram".into(), histogram);
            map.insert("n_rows".into(), json!(matrix.nrows()));
            map.insert("n_cols".into(), json!(matrix.ncols()));
            map.insert("data_type".into(), json!(matrix.data_type));
            map.insert("symmetric".into(), json!(matrix.symmetric_labels()));
        }
        Ok(payload)
    })
    .await
    .map_err(|e| format!("matrix report failed: {e}"))?
}

/// Bin `scores` (any order) into `bins` equal-width buckets over [min,max].
/// Returns `[[bin_low, count], ...]`. Empty for fewer than 2 distinct values.
fn histogram(scores: &[f64], bins: usize) -> Value {
    let out: Vec<Value> = domainator_core::stats::histogram(scores, bins)
        .into_iter()
        .map(|(low, c)| json!([low, c]))
        .collect();
    json!(out)
}

// Phase-2 note: the matrix report assumes a symmetric SSN-style matrix (MaxTree);
// query×reference matrices aren't meaningfully handled yet.

/// Pure-data summary statistics for a sequence file (Summary dashboard).
#[tauri::command]
pub async fn run_summary_stats(
    path: String,
    ncbi_taxonomy_path: Option<String>,
) -> Result<SummaryStats, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let kind = detect_filetype(std::path::Path::new(&path))
            .ok_or_else(|| format!("not a sequence file: {path}"))?;
        let records = match kind {
            FileType::Genbank => {
                read_genbank(&path).map_err(|e| format!("reading GenBank: {e}"))?
            }
            FileType::Fasta => {
                read_fasta(&path, None).map_err(|e| format!("reading FASTA: {e}"))?
            }
        };
        let taxonomy = ncbi_taxonomy_path
            .as_deref()
            .filter(|path| !path.is_empty())
            .map(crate::taxonomy_cache::load)
            .transpose()?;
        Ok(
            domainator_core::app_support::summary_stats::summarize_with_taxonomy(
                &records,
                taxonomy.as_deref(),
            ),
        )
    })
    .await
    .map_err(|e| format!("summary failed: {e}"))?
}

/// Run `enum_report` on a sequence file and return `{rows, targets}` for the Enum
/// Report tab's grid: `rows` is an array of typed row objects, and `targets[i]` is
/// `[contig_index, feat_index | null]`, the record and feature row `i` describes
/// (the indices `load_contig_view` reports), so a row can be tied to a Contig View
/// selection. `by` is `contig` | `cds` | `domain`
/// (default `cds`). `columns` are column objects, `{type, ...parameters}`, as the
/// config's `ColumnSpec` reads them -- the same shape the export's column builder
/// sends -- and a per-granularity default set when empty. `ncbi_taxonomy_path` is
/// needed only by the taxonomy columns, and is served from `taxonomy_cache`.
/// Column order is preserved (serde_json `preserve_order`); the grid infers numeric
/// vs text columns from the JSON value kinds.
#[tauri::command]
pub async fn run_enum_report(
    path: String,
    by: Option<String>,
    columns: Option<Vec<Value>>,
    ncbi_taxonomy_path: Option<String>,
) -> Result<Value, String> {
    // Off the UI thread: an 11 MB GenBank has thousands of CDS and takes a while,
    // and a first taxonomy load several seconds more.
    tauri::async_runtime::spawn_blocking(move || {
        run_enum_report_blocking(path, by, columns.unwrap_or_default(), ncbi_taxonomy_path)
    })
    .await
    .map_err(|e| format!("enum report failed: {e}"))?
}

/// Default columns per granularity.
pub(crate) fn default_columns(by: &str) -> Vec<ColumnSpec> {
    let names: &[&str] = match by {
        "contig" => &["length", "cds_count", "domains"],
        "domain" => &["start", "end", "strand", "score", "identity"],
        _ => &["domains", "architecture", "length"],
    };
    names
        .iter()
        .filter_map(|n| ColumnSpec::from_static_flag(n))
        .collect()
}

fn run_enum_report_blocking(
    path: String,
    by: Option<String>,
    columns: Vec<Value>,
    ncbi_taxonomy_path: Option<String>,
) -> Result<Value, String> {
    let by_str = by.as_deref().unwrap_or("cds");
    let by = match by_str {
        "contig" => By::Contig,
        "domain" => By::Domain,
        _ => By::Cds,
    };
    let specs: Vec<ColumnSpec> = if columns.is_empty() {
        default_columns(by_str)
    } else {
        columns
            .into_iter()
            .map(|c| serde_json::from_value(c).map_err(|e| format!("bad column: {e}")))
            .collect::<Result<_, _>>()?
    };
    let analyses = specs
        .iter()
        .map(|c| c.to_analysis().map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    // Before reading the file, so a missing taxonomy is reported at once.
    let tax = if needs_taxonomy(&analyses) {
        let tax_path = ncbi_taxonomy_path.as_deref().filter(|p| !p.trim().is_empty()).ok_or(
            "the taxonomy columns need a taxonomy database: install NCBI Taxonomy from Reference Databases, then choose it here",
        )?;
        Some(crate::taxonomy_cache::load(tax_path)?)
    } else {
        None
    };

    let kind = detect_filetype(std::path::Path::new(&path))
        .ok_or_else(|| format!("not a sequence file: {path}"))?;
    let records = match kind {
        FileType::Genbank => read_genbank(&path).map_err(|e| format!("reading GenBank: {e}"))?,
        FileType::Fasta => read_fasta(&path, None).map_err(|e| format!("reading FASTA: {e}"))?,
    };
    let filename = std::path::Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let recs: Vec<_> = records.into_iter().map(|r| (r, filename.clone())).collect();

    let mut buf: Vec<u8> = Vec::new();
    let targets = enum_report_with_targets(
        recs,
        by,
        &analyses,
        None,
        tax.as_deref(),
        None,
        Partial::Include,
        OutputFormat::Json,
        &mut buf,
    )?;
    let targets: Vec<Value> = targets
        .iter()
        .map(|t| serde_json::json!([t.contig, t.feature]))
        .collect();
    Ok(serde_json::json!({ "rows": ndjson_to_array(&buf)?, "targets": targets }))
}

/// Collect streaming NDJSON (one compact object per line) into a single JSON
/// array.
///
/// `OutputFormat::Json` is the agent-facing *newline-delimited* contract, not a
/// JSON document — feeding the whole buffer to `serde_json::from_slice` fails on
/// the second row ("trailing characters at line 2 column 1"), which is what the
/// grid used to do.
fn ndjson_to_array(buf: &[u8]) -> Result<Value, String> {
    let text =
        std::str::from_utf8(buf).map_err(|e| format!("enum_report output not utf-8: {e}"))?;
    let mut rows = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        rows.push(
            serde_json::from_str::<Value>(line)
                .map_err(|e| format!("parsing enum_report json: {e}"))?,
        );
    }
    Ok(Value::Array(rows))
}

/// Load an HMM file and return skylign-style logo data for one profile
/// (`index`, default 0 — the first profile in a multi-model file).
#[tauri::command]
pub async fn load_hmm_logo(
    hmm_index: tauri::State<'_, HmmIndexState>,
    path: String,
    index: Option<usize>,
) -> Result<HmmLogo, String> {
    let state = hmm_index.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let idx = index.unwrap_or(0);
        let entries = cached_hmm_index(&state, &path)?;
        let entry = entries
            .get(idx)
            .ok_or_else(|| format!("no HMM at index {idx} (file has {})", entries.len()))?;
        // Parse ONLY this profile's bytes instead of the whole (possibly huge) file.
        let bytes = read_hmm_byte_ranges(&path, &[(entry.byte_start, entry.byte_end)])?
            .pop()
            .unwrap_or_default();
        logo_from_hmm_bytes(&bytes)
    })
    .await
    .map_err(|e| format!("loading HMM logo failed: {e}"))?
}

/// Single-file cache of every contig's uppercased sequence, so the contig viewer
/// can fetch residues while scrolling without re-parsing the whole file per call
/// (a big multi-contig file made that O(contigs) full parses). Holds one file at a
/// time (replaced when the path or its mtime changes) to bound memory.
#[derive(Default)]
pub struct SeqCacheState(pub std::sync::Arc<std::sync::Mutex<Option<SeqCacheEntry>>>);

pub struct SeqCacheEntry {
    path: String,
    mtime_ns: u128,
    seqs: Vec<String>,
}

/// Lock that survives a poisoned mutex. Every mutex in this module guards a plain
/// cache — a panic elsewhere can't leave one half-updated in a way that matters —
/// so `.lock().unwrap()` would only turn a transient panic into a permanently
/// broken feature for the rest of the session.
pub(crate) fn lock_poison_tolerant<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn file_mtime_ns(path: &str) -> Result<u128, String> {
    let m = std::fs::metadata(path).map_err(|e| format!("stat {path}: {e}"))?;
    let t = m.modified().map_err(|e| format!("mtime {path}: {e}"))?;
    Ok(t.duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0))
}

fn file_size(path: &str) -> Result<u64, String> {
    Ok(std::fs::metadata(path)
        .map_err(|e| format!("stat {path}: {e}"))?
        .len())
}

/// Cached header-only index of recently-touched HMM libraries, so listing profiles,
/// loading a single logo, and extracting profiles don't each re-scan (or, worse,
/// fully re-parse) the whole file. On Pfam-scale libraries (20k+ models, gigabytes)
/// the first access scans once; every access after that is O(1) seeks.
///
/// Holds a few files (not one) because the logo viewer, the profile table and a
/// popped-out window can be looking at different libraries at once — a single slot
/// made them evict each other and re-scan gigabytes on every switch.
type HmmIndexCache = std::sync::Arc<std::sync::Mutex<Vec<HmmIndexEntry>>>;

/// How many HMM libraries' indexes to keep (LRU: most recent first).
const HMM_INDEX_CACHE_SLOTS: usize = 4;

#[derive(Default)]
pub struct HmmIndexState(pub HmmIndexCache);

pub struct HmmIndexEntry {
    path: String,
    size: u64,
    mtime_ns: u128,
    entries: std::sync::Arc<Vec<HmmEntry>>,
}

/// Return the (cached) profile index for `path`, rebuilding it if the file is new
/// or has changed on disk (size or mtime).
///
/// The scan runs with the lock RELEASED: `index_hmm_file` reads the whole library,
/// and holding the mutex across it serialized every other HMM command behind a
/// gigabyte-scale read. Two callers racing on the same cold path both scan, and the
/// second simply refreshes the entry — cheaper than making everyone wait.
fn cached_hmm_index(
    cache: &HmmIndexCache,
    path: &str,
) -> Result<std::sync::Arc<Vec<HmmEntry>>, String> {
    let size = file_size(path)?;
    let mtime_ns = file_mtime_ns(path)?;
    if let Some(hit) = lookup_hmm_index(cache, path, size, mtime_ns) {
        return Ok(hit);
    }
    let entries = std::sync::Arc::new(index_hmm_file(path)?);
    let mut guard = lock_poison_tolerant(cache);
    guard.retain(|e| e.path != path);
    guard.insert(
        0,
        HmmIndexEntry {
            path: path.to_string(),
            size,
            mtime_ns,
            entries: entries.clone(),
        },
    );
    guard.truncate(HMM_INDEX_CACHE_SLOTS);
    Ok(entries)
}

/// Cache hit for `path` if it's present and still matches the file on disk; also
/// promotes it to most-recently-used.
fn lookup_hmm_index(
    cache: &HmmIndexCache,
    path: &str,
    size: u64,
    mtime_ns: u128,
) -> Option<std::sync::Arc<Vec<HmmEntry>>> {
    let mut guard = lock_poison_tolerant(cache);
    let i = guard.iter().position(|e| e.path == path)?;
    if guard[i].size != size || guard[i].mtime_ns != mtime_ns {
        guard.remove(i);
        return None;
    }
    let entry = guard.remove(i);
    let hit = entry.entries.clone();
    guard.insert(0, entry);
    Some(hit)
}

/// Per-file cache of `(id, name)` record identities, keyed by path with a
/// (size, mtime_ns) staleness stamp. Lets label→record resolution skip re-scanning
/// unchanged files across repeated "Extract selected" clicks. Bounded by the
/// workspace's sequence-file count (modest), so no eviction is needed.
#[derive(Default)]
pub struct RecordIdIndexState(
    pub std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, RecordIdCacheEntry>>>,
);

#[derive(Clone)]
pub struct RecordIdCacheEntry {
    size: u64,
    mtime_ns: u128,
    ids: std::sync::Arc<Vec<(String, String)>>,
}

/// Return each record's `(id, name)` for `path`, using a fast header-only scan for
/// uncompressed GenBank/FASTA and falling back to a full parse for compressed files
/// (the scanners read raw bytes). Results are cached per file (size+mtime stamped).
fn cached_record_ids(
    cache: &std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, RecordIdCacheEntry>>>,
    path: &str,
    kind: FileType,
) -> Option<std::sync::Arc<Vec<(String, String)>>> {
    let size = file_size(path).ok()?;
    let mtime_ns = file_mtime_ns(path).ok()?;
    if let Some(e) = lock_poison_tolerant(cache).get(path) {
        if e.size == size && e.mtime_ns == mtime_ns {
            return Some(e.ids.clone());
        }
    }
    let lower = path.to_ascii_lowercase();
    let compressed = lower.ends_with(".gz") || lower.ends_with(".bgz") || lower.ends_with(".bgzf");
    let ids: Vec<(String, String)> = match (kind, compressed) {
        (FileType::Genbank, false) => scan_genbank_ids(path).ok()?,
        (FileType::Fasta, false) => scan_fasta_ids(path).ok()?,
        // Compressed: no cheap byte scan — full parse, then keep just id/name.
        (FileType::Genbank, true) => read_genbank(path)
            .ok()?
            .iter()
            .map(|r| (r.id.clone(), r.name.clone()))
            .collect(),
        (FileType::Fasta, true) => read_fasta(path, None)
            .ok()?
            .iter()
            .map(|r| (r.id.clone(), r.name.clone()))
            .collect(),
    };
    let ids = std::sync::Arc::new(ids);
    cache.lock().unwrap().insert(
        path.to_string(),
        RecordIdCacheEntry {
            size,
            mtime_ns,
            ids: ids.clone(),
        },
    );
    Some(ids)
}

fn parse_all_seqs(path: &str) -> Result<Vec<String>, String> {
    let kind = detect_filetype(std::path::Path::new(path))
        .ok_or_else(|| format!("not a sequence file: {path}"))?;
    let records = match kind {
        FileType::Genbank => read_genbank(path).map_err(|e| format!("reading GenBank: {e}"))?,
        FileType::Fasta => read_fasta(path, None).map_err(|e| format!("reading FASTA: {e}"))?,
    };
    Ok(records
        .iter()
        .map(|r| String::from_utf8_lossy(&r.seq).to_ascii_uppercase())
        .collect())
}

/// Uppercased sequences for contigs `[start, end)` (clamped). Parses the file once
/// and caches all sequences; subsequent calls (e.g. while scrolling) are served
/// from memory. The viewer batches its visible window into a single call.
#[tauri::command]
pub async fn get_contig_sequences(
    state: tauri::State<'_, SeqCacheState>,
    path: String,
    start: usize,
    end: usize,
) -> Result<Vec<String>, String> {
    let cache = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mtime = file_mtime_ns(&path)?;
        let slice = |seqs: &[String]| -> Vec<String> {
            let e = end.min(seqs.len());
            if start >= e {
                Vec::new()
            } else {
                seqs[start..e].to_vec()
            }
        };
        {
            let guard = lock_poison_tolerant(&cache);
            if let Some(entry) = guard.as_ref() {
                if entry.path == path && entry.mtime_ns == mtime {
                    return Ok(slice(&entry.seqs));
                }
            }
        }
        let seqs = parse_all_seqs(&path)?;
        let out = slice(&seqs);
        *lock_poison_tolerant(&cache) = Some(SeqCacheEntry {
            path,
            mtime_ns: mtime,
            seqs,
        });
        Ok(out)
    })
    .await
    .map_err(|e| format!("reading sequences failed: {e}"))?
}

/// Per-file record-count cache (path → (size, mtime_ns, count)), so the file
/// table's "records" column doesn't re-stream files on every folder revisit.
/// Bounded only by the number of distinct files browsed in a session.
/// `(size, mtime_ns, record count)` — the count is `None` for file types without
/// a meaningful one. Size and mtime are the staleness check.
type RecordCountEntry = (u64, u128, Option<u64>);

type RecordCountCache =
    std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, RecordCountEntry>>>;

#[derive(Default)]
pub struct RecordCountState(pub RecordCountCache);

/// Record counts for `paths` (streamed once per file, then cached). `null` for
/// types without a meaningful count (matrices/tables) or unreadable files. Called
/// lazily by the file table only when the "records" column is shown, so plain
/// folder listing stays instant.
#[tauri::command]
pub async fn count_records(
    state: tauri::State<'_, RecordCountState>,
    paths: Vec<String>,
) -> Result<Vec<Option<u64>>, String> {
    // Off the runtime's async threads: `workspace::count_records` streams each file
    // end to end, so a folder of large GenBanks would pin a worker for seconds.
    let map = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || count_records_blocking(&map, &paths))
        .await
        .map_err(|e| format!("counting records failed: {e}"))
}

fn count_records_blocking(map: &RecordCountCache, paths: &[String]) -> Vec<Option<u64>> {
    let mut out = Vec::with_capacity(paths.len());
    for p in paths {
        let stamp = std::fs::metadata(p).ok().map(|m| {
            let mt = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            (m.len(), mt)
        });
        let Some((size, mtime)) = stamp else {
            out.push(None);
            continue;
        };
        if let Some(&(cs, cm, cnt)) = lock_poison_tolerant(map).get(p) {
            if cs == size && cm == mtime {
                out.push(cnt);
                continue;
            }
        }
        let count = domainator_core::app_support::workspace::count_records(std::path::Path::new(p));
        lock_poison_tolerant(map).insert(p.clone(), (size, mtime, count));
        out.push(count);
    }
    out
}

/// Read a text file (e.g. a TSV/CSV table) as a UTF-8 string. Rejects files over
/// ~64 MB so the table editor can't accidentally slurp a giant blob into the UI.
#[tauri::command]
pub async fn read_text_file(path: String) -> Result<TextFile, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let meta = std::fs::metadata(&path).map_err(|e| format!("stat {path}: {e}"))?;
        // The editors hold the whole file as a JS string, and it crosses the IPC
        // bridge as JSON, so there is a real ceiling. Refuse above it rather than
        // hand the webview something that will take the window down with it — and
        // say what the limit is, since the file is still perfectly usable as a
        // tool input.
        const MAX_EDIT_BYTES: u64 = 64 * 1024 * 1024;
        if meta.len() > MAX_EDIT_BYTES {
            return Err(format!(
                "file is {} MB; the editor handles up to {} MB. It can still be used as                  an input to any tool.",
                meta.len() / 1024 / 1024,
                MAX_EDIT_BYTES / 1024 / 1024,
            ));
        }
        let raw = std::fs::read_to_string(&path).map_err(|e| format!("reading {path}: {e}"))?;
        // The editors work in LF (a `<textarea>` normalizes its value to LF no
        // matter what you put in it), so hand back LF plus the terminator to put
        // back on save.
        let eol = detect_eol(&raw);
        Ok(TextFile { text: to_lf(&raw).into_owned(), eol: eol.as_str().to_string() })
    })
    .await
    .map_err(|e| format!("read failed: {e}"))?
}

/// Overwrite a text file in place (used by the table editor's Save). Writes
/// atomically via a temp file + rename so a crash mid-write can't truncate it.
/// `contents` is LF-separated; `eol` (optional, default LF) is the terminator to
/// write with — pass back the one [`read_text_file`] reported to round-trip a file
/// unchanged.
#[tauri::command]
pub async fn write_text_file(
    path: String,
    contents: String,
    eol: Option<String>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let contents = from_lf(&contents, eol_or_lf(eol.as_deref()));
        let p = std::path::Path::new(&path);
        let tmp = p.with_extension(format!(
            "{}.tmp",
            p.extension().and_then(|e| e.to_str()).unwrap_or("txt")
        ));
        std::fs::write(&tmp, contents.as_bytes()).map_err(|e| format!("writing {path}: {e}"))?;
        std::fs::rename(&tmp, p).map_err(|e| format!("finalizing {path}: {e}"))?;
        Ok(())
    })
    .await
    .map_err(|e| format!("write failed: {e}"))?
}

/// Summaries of every profile in an HMM file (for the browser list).
#[tauri::command]
pub async fn list_hmm_profiles(
    hmm_index: tauri::State<'_, HmmIndexState>,
    path: String,
) -> Result<Vec<HmmSummary>, String> {
    let state = hmm_index.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let entries = cached_hmm_index(&state, &path)?;
        Ok(entries
            .iter()
            .enumerate()
            .map(|(i, e)| HmmSummary {
                index: i,
                name: e.name.clone(),
                acc: e.acc.clone(),
                desc: e.desc.clone(),
                length: e.length,
                alphabet: e.alphabet.clone(),
            })
            .collect())
    })
    .await
    .map_err(|e| format!("listing HMMs failed: {e}"))?
}

/// Pop a native "Save As" dialog and write `contents` to the chosen path. Used by
/// viewers to export generated text (e.g. an SVG logo). Returns the saved path, or
/// `None` if the user canceled. `default_name` seeds the dialog's filename;
/// `extension`/`filter_name` set the type filter (e.g. `"svg"` / `"SVG image"`).
#[tauri::command]
pub async fn save_text_file(
    window: tauri::WebviewWindow,
    default_name: String,
    extension: String,
    filter_name: String,
    contents: String,
) -> Result<Option<String>, String> {
    // `async` + `AsyncFileDialog`: a sync command would show the dialog on Tauri's
    // main thread — see the note on `cmd_workspace::pick_directory`. The write goes
    // to a blocking thread too; `contents` can be a multi-megabyte SVG export.
    let picked = rfd::AsyncFileDialog::new()
        .set_parent(&window)
        .set_file_name(&default_name)
        .add_filter(&filter_name, &[extension.as_str()])
        .save_file()
        .await;
    let Some(handle) = picked else {
        return Ok(None);
    };
    let path = handle.path().to_path_buf();
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::write(&path, contents).map_err(|e| format!("writing file: {e}"))?;
        Ok(Some(path.to_string_lossy().into_owned()))
    })
    .await
    .map_err(|e| format!("saving file failed: {e}"))?
}

/// Pop a native "Save As" dialog and write decoded base64 bytes to the chosen
/// path. The binary twin of [`save_text_file`], used by viewers that export a
/// raster image (PNG) or a gzipped SSN bundle (`.dsnv`). The payload arrives
/// base64-encoded because the IPC bridge is JSON: a `Vec<u8>` would serialize as
/// a decimal array, roughly four bytes of wire per byte of data. Returns the saved path, or `None` if canceled.
#[tauri::command]
pub async fn save_binary_file(
    window: tauri::WebviewWindow,
    default_name: String,
    extension: String,
    filter_name: String,
    contents_base64: String,
) -> Result<Option<String>, String> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(contents_base64.as_bytes())
        .map_err(|e| format!("decoding file data: {e}"))?;
    // `async` + `AsyncFileDialog`: a sync command would show the dialog on Tauri's
    // main thread — see the note on `cmd_workspace::pick_directory`.
    let picked = rfd::AsyncFileDialog::new()
        .set_parent(&window)
        .set_file_name(&default_name)
        .add_filter(&filter_name, &[extension.as_str()])
        .save_file()
        .await;
    let Some(handle) = picked else {
        return Ok(None);
    };
    let path = handle.path().to_path_buf();
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::write(&path, bytes).map_err(|e| format!("writing file: {e}"))?;
        Ok(Some(path.to_string_lossy().into_owned()))
    })
    .await
    .map_err(|e| format!("saving file failed: {e}"))?
}

/// A record located in a workspace sequence file (for SSN → sequence navigation).
#[derive(Serialize, Clone)]
pub struct ResolvedRecord {
    pub path: String,
    pub file_type: String,
    pub contig_index: usize,
    pub contig_id: String,
}

/// Resolve a batch of SSN node labels (sequence id/name, from matrix rows) to the
/// sequence file + contig index that contains each, scanning the workspace's
/// GenBank/FASTA files **once** for the whole set. Returns a vector aligned to
/// `labels` (element is `None` for any label not found).
///
/// This is the cross-view resolver behind "Extract selected". It is a batch (one
/// call for the whole selection) and searches ONLY `files` — the sequence sources
/// the network was traced back to via provenance (see [`trace_network_sources`]).
/// Restricting to the true sources both makes it fast (a header-only id scan,
/// cached per file — no full parse of unrelated genomes) and correct when the same
/// id/name exists in unrelated workspace files. `files` are searched in order, so
/// the earliest source wins if a name is duplicated across the sources themselves.
#[tauri::command]
pub async fn resolve_records(
    record_ids: tauri::State<'_, RecordIdIndexState>,
    files: Vec<String>,
    labels: Vec<String>,
) -> Result<Vec<Option<ResolvedRecord>>, String> {
    let cache = record_ids.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        use std::collections::{HashMap, HashSet};
        let wanted: HashSet<&str> = labels.iter().map(String::as_str).collect();
        // label -> resolved record (first match in `files` order wins).
        let mut found: HashMap<String, ResolvedRecord> = HashMap::new();

        for path_str in &files {
            let path = std::path::Path::new(path_str);
            let Some(kind) = detect_filetype(path) else {
                continue;
            };
            let Some(ids) = cached_record_ids(&cache, path_str, kind) else {
                continue;
            };
            let file_type = classify_file(path).unwrap_or("genbank").to_string();
            for (i, (id, name)) in ids.iter().enumerate() {
                // A record can satisfy a label by either its id or its name.
                for key in [id.as_str(), name.as_str()] {
                    if wanted.contains(key) && !found.contains_key(key) {
                        found.insert(
                            key.to_string(),
                            ResolvedRecord {
                                path: path_str.clone(),
                                file_type: file_type.clone(),
                                contig_index: i,
                                contig_id: id.clone(),
                            },
                        );
                    }
                }
            }
            if found.len() >= wanted.len() {
                break;
            }
        }

        Ok(labels.iter().map(|l| found.get(l).cloned()).collect())
    })
    .await
    .map_err(|e| format!("resolve records failed: {e}"))?
}

/// Trace a network artifact (`.dsnv`/matrix) back to the sequence files that
/// supplied its node labels, via the provenance job DAG. The workspace root is
/// discovered by walking up from `path` for a `.domainator/` dir, so this works in
/// detached windows too. Returns `has_provenance=false` (empty sources) when no
/// creating-job chain reaches sequence inputs — the UI then disables extraction.
#[tauri::command]
pub async fn trace_network_sources(path: String) -> Result<SourceTrace, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let p = std::path::Path::new(&path);
        match find_workspace_root(p) {
            Some(root) => trace_source_sequences(&root, p),
            None => SourceTrace {
                has_provenance: false,
                sources: Vec::new(),
            },
        }
    })
    .await
    .map_err(|e| format!("tracing sources failed: {e}"))
}

/// Render the self-contained SSN viewer HTML for a `.dsnv` bundle, with the
/// bundle embedded so it autoloads. Hosted by the frontend in an `<iframe>`.
#[tauri::command]
pub async fn render_ssn_html(path: String) -> Result<String, String> {
    // Off the UI thread: a .dsnv bundle can be large, and embedding it into the
    // HTML shell is a big string build — keep it off Tauri's main thread.
    tauri::async_runtime::spawn_blocking(move || {
        let bytes = std::fs::read(&path).map_err(|e| format!("reading SSN bundle: {e}"))?;
        let title = std::path::Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "SSN".into());
        Ok(render_html(&title, Some(&bytes)))
    })
    .await
    .map_err(|e| format!("rendering SSN failed: {e}"))?
}

/// One selected record: its source file and 0-based contig index.
#[derive(Deserialize)]
pub struct RecordRef {
    pub path: String,
    pub contig_index: usize,
}

/// Validate a bare filename and join it to `dir`, refusing path separators, the
/// reserved `.domainator` name, and pre-existing targets (caller supplies a name).
fn validated_target(dir: &str, name: &str) -> Result<std::path::PathBuf, String> {
    use domainator_core::app_support::workspace::{
        accepted_types_hint, classify_file, is_reserved_name,
    };
    let clean = name.trim();
    if clean.is_empty()
        || clean.contains('/')
        || clean.contains('\\')
        || clean == "."
        || clean == ".."
    {
        return Err("invalid file name".into());
    }
    if is_reserved_name(clean) {
        return Err(format!("'{clean}' is reserved"));
    }
    let p = std::path::Path::new(dir).join(clean);
    // Refuse unrecognized extensions — the workspace only lists known data types,
    // so a mystery extension would create a file the browser can't show.
    if classify_file(&p).is_none() {
        return Err(format!(
            "unrecognized file type for '{clean}'. {}",
            accepted_types_hint()
        ));
    }
    if p.exists() {
        return Err(format!("already exists: {clean}"));
    }
    Ok(p)
}

fn is_fasta_name(name: &str) -> bool {
    let l = name.to_lowercase();
    [".fasta", ".fa", ".fna", ".faa", ".fap", ".pep"]
        .iter()
        .any(|e| l.ends_with(e))
}

/// Notify every window that files under `dir` changed, so file browsers refresh
/// (e.g. after an Extract from a popped-out window creates a file elsewhere).
fn notify_files_changed(app: &tauri::AppHandle, dir: &str) {
    use tauri::Emitter;
    let _ = app.emit("files-changed", dir.to_string());
}

/// One item of a Contig View selection to extract: contig `contig` (0-based) whole,
/// or its region [`start`, `end`) (0-based half-open) read on `strand` (`-1` gives
/// the reverse complement, so a minus-strand feature comes out reading forward).
#[derive(Debug, Clone, Deserialize)]
pub struct ExtractItem {
    pub contig: usize,
    pub start: Option<usize>,
    pub end: Option<usize>,
    pub strand: Option<i32>,
}

/// Write each selected item, in order, as its own record of a new file
/// `dir/name`: a whole contig is copied as is, and a region is sliced the way
/// `select-by-coord` slices it (overlapping features kept and truncated, the
/// record named `<id>_<start>:<end>`). The format follows `name`'s extension
/// (FASTA vs GenBank). When `translate` is set the records are then run through
/// `extract_peptides`, so the output is a protein GenBank of the CDSs they hold.
#[tauri::command]
pub async fn extract_selection(
    app: tauri::AppHandle,
    dir: String,
    name: String,
    source: String,
    items: Vec<ExtractItem>,
    translate: bool,
) -> Result<String, String> {
    let dir_c = dir.clone();
    let out = tauri::async_runtime::spawn_blocking(move || {
        extract_selection_blocking(&dir, &name, &source, &items, translate)
    })
    .await
    .map_err(|e| format!("extract failed: {e}"))??;
    notify_files_changed(&app, &dir_c);
    Ok(out)
}

fn extract_selection_blocking(
    dir: &str,
    name: &str,
    source: &str,
    items: &[ExtractItem],
    translate: bool,
) -> Result<String, String> {
    use domainator_core::tools::extract_peptides::{extract_peptides, ExtractPeptidesOptions};
    use domainator_core::tools::select_by_coord::{extract_coordinate_region, CoordinateSelection};
    if items.is_empty() {
        return Err("nothing selected".into());
    }
    let recs = load_sequence_any(source)?;
    let mut out: Vec<SeqRecord> = Vec::with_capacity(items.len());
    for item in items {
        let contig = recs
            .get(item.contig)
            .ok_or_else(|| format!("no contig at index {}", item.contig))?;
        match (item.start, item.end) {
            (Some(start), Some(end)) => {
                if end <= start {
                    return Err("empty region".into());
                }
                let sel = CoordinateSelection {
                    contig: contig.id.clone(),
                    start: start as i64 + 1, // 1-based inclusive
                    end: end as i64,
                    strand: if item.strand == Some(-1) { -1 } else { 1 },
                    file: None,
                };
                out.push(extract_coordinate_region(contig, &sel)?);
            }
            _ => out.push(contig.clone()),
        }
    }
    let target = validated_target(dir, name)?;
    let ts = target.to_string_lossy().into_owned();
    if translate {
        let opts = ExtractPeptidesOptions {
            evalue: 100_000_000.0,
            target_domains: None,
            target_cdss: None,
            search_hits: false,
            strand: None,
            unannotated: false,
            invert: false,
            databases: None,
            keep_name: false,
            name_field: None,
            extract_all: true,
            partial: Partial::Include,
        };
        let peps = extract_peptides(&out, &opts);
        if peps.is_empty() {
            return Err("no CDS features in the selection to translate".into());
        }
        write_genbank(&ts, &peps).map_err(|e| format!("writing GenBank: {e}"))?;
    } else if is_fasta_name(name) {
        domainator_core::io::fasta::write_fasta(&ts, &out)
            .map_err(|e| format!("writing FASTA: {e}"))?;
    } else {
        write_genbank(&ts, &out).map_err(|e| format!("writing GenBank: {e}"))?;
    }
    Ok(ts)
}

/// Write the selected profiles (0-based `indices`, in order) from an `.hmm` `source`
/// into a new `.hmm` file `dir/name`.
#[tauri::command]
pub async fn extract_hmms(
    app: tauri::AppHandle,
    hmm_index: tauri::State<'_, HmmIndexState>,
    dir: String,
    name: String,
    source: String,
    indices: Vec<usize>,
) -> Result<String, String> {
    let dir_c = dir.clone();
    let state = hmm_index.0.clone();
    let out = tauri::async_runtime::spawn_blocking(move || {
        use std::io::Write;
        let target = validated_target(&dir, &name)?;
        let ts = target.to_string_lossy().into_owned();
        // Copy each selected profile's raw bytes (exact original formatting) instead
        // of re-parsing and re-serializing the whole source — instant on big libraries.
        // One pass over the source for every selected profile: a gzipped library
        // cannot be seeked, and inflating it once per profile would be quadratic.
        let entries = cached_hmm_index(&state, &source)?;
        let ranges: Vec<(u64, u64)> = indices
            .iter()
            .filter_map(|&i| entries.get(i))
            .map(|e| (e.byte_start, e.byte_end))
            .collect();
        if ranges.is_empty() {
            return Err("no profiles selected".to_string());
        }
        let profiles = read_hmm_byte_ranges(&source, &ranges)?;
        // `.hmm.gz` gets plain gzip and `.hmm.bgz` is refused, as for every other
        // hmm output (HMMER cannot read BGZF).
        write_hmm_output(&ts, |w| {
            profiles
                .iter()
                .try_for_each(|bytes| w.write_all(bytes).map_err(|e| format!("writing {ts}: {e}")))
        })?;
        Ok::<String, String>(ts)
    })
    .await
    .map_err(|e| format!("extract failed: {e}"))??;
    notify_files_changed(&app, &dir_c);
    Ok(out)
}

/// A text file's contents, normalized to LF, plus the line terminator it actually
/// used — so a save can put that terminator back instead of silently rewriting a
/// CRLF file as LF.
#[derive(Serialize)]
pub struct TextFile {
    pub text: String,
    /// The literal terminator (`"\n"`, `"\r\n"` or `"\r"`), to be passed back to
    /// [`write_text_file`] / [`create_text_file`] unchanged.
    pub eol: String,
}

/// Parse a wire `eol`, defaulting to LF when the caller omits it (which is what
/// every caller that does not round-trip a file wants).
fn eol_or_lf(eol: Option<&str>) -> Eol {
    eol.map(Eol::from_str_lossy).unwrap_or_default()
}

/// Create a new text file `dir/name` with `contents` (New-file-from-pasted-text).
/// `contents` is LF-separated; `eol` (optional, default LF) is the terminator to
/// write with.
#[tauri::command]
pub async fn create_text_file(
    app: tauri::AppHandle,
    dir: String,
    name: String,
    contents: String,
    eol: Option<String>,
) -> Result<String, String> {
    let dir_c = dir.clone();
    let out = tauri::async_runtime::spawn_blocking(move || {
        let contents = from_lf(&contents, eol_or_lf(eol.as_deref()));
        let target = validated_target(&dir, &name)?;
        std::fs::write(&target, contents.as_bytes()).map_err(|e| format!("writing file: {e}"))?;
        Ok::<String, String>(target.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| format!("create failed: {e}"))??;
    notify_files_changed(&app, &dir_c);
    Ok(out)
}

fn load_sequence_any(path: &str) -> Result<Vec<SeqRecord>, String> {
    match detect_filetype(std::path::Path::new(path)) {
        Some(FileType::Genbank) => read_genbank(path).map_err(|e| format!("reading {path}: {e}")),
        Some(FileType::Fasta) => read_fasta(path, None).map_err(|e| format!("reading {path}: {e}")),
        None => Err(format!("unsupported sequence file: {path}")),
    }
}

/// Build a single GenBank file containing just the selected records (possibly from
/// several source files) as a real, named workspace file at `dir/name`. Returns its
/// path and notifies windows so browsers refresh. Backs the SSN "Extract selected"
/// action — the result is a permanent workspace file, not a temporary subset.
#[tauri::command]
pub async fn extract_records(
    app: tauri::AppHandle,
    dir: String,
    name: String,
    records: Vec<RecordRef>,
) -> Result<String, String> {
    let dir_c = dir.clone();
    let out = tauri::async_runtime::spawn_blocking(move || {
        use std::collections::HashMap;
        let target = validated_target(&dir, &name)?;
        let ts = target.to_string_lossy().into_owned();
        let mut cache: HashMap<String, Vec<SeqRecord>> = HashMap::new();
        let mut out: Vec<SeqRecord> = Vec::new();
        for r in &records {
            if !cache.contains_key(&r.path) {
                cache.insert(r.path.clone(), load_sequence_any(&r.path)?);
            }
            if let Some(rec) = cache[&r.path].get(r.contig_index) {
                out.push(rec.clone());
            }
        }
        if out.is_empty() {
            return Err("none of the selected records could be read".into());
        }
        if is_fasta_name(&name) {
            domainator_core::io::fasta::write_fasta(&ts, &out)
                .map_err(|e| format!("writing FASTA: {e}"))?;
        } else {
            write_genbank(&ts, &out).map_err(|e| format!("writing GenBank: {e}"))?;
        }
        Ok::<String, String>(ts)
    })
    .await
    .map_err(|e| format!("extract failed: {e}"))??;
    notify_files_changed(&app, &dir_c);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> String {
        let d =
            std::env::temp_dir().join(format!("domainator-files-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.to_string_lossy().into_owned()
    }

    /// A minimal two-profile HMM library: enough for the index scan, which only
    /// looks at the `HMMER3/` line, the header keys and the closing `//`.
    fn hmm_library(dir: &str, name: &str, profiles: &[(&str, usize)]) -> String {
        let mut body = String::new();
        for (pname, len) in profiles {
            body.push_str(&format!(
                "HMMER3/f [3.3.2]\nNAME  {pname}\nLENG  {len}\nALPH  amino\n//\n"
            ));
        }
        let path = std::path::Path::new(dir).join(name);
        std::fs::write(&path, body).unwrap();
        path.to_string_lossy().into_owned()
    }

    /// The HMM index cache used to hold a single file, so the Profiles table and a
    /// logo in another window evicted each other and re-scanned the whole library
    /// (gigabytes, for Pfam) on every switch. Alternating between two files must
    /// hit the cache, not thrash it.
    #[test]
    fn the_hmm_index_cache_holds_more_than_one_library() {
        let dir = scratch("hmm-cache");
        let a = hmm_library(&dir, "a.hmm", &[("AAA", 10), ("BBB", 20)]);
        let b = hmm_library(&dir, "b.hmm", &[("CCC", 30)]);
        let cache: HmmIndexCache = Default::default();

        for _ in 0..3 {
            assert_eq!(cached_hmm_index(&cache, &a).unwrap().len(), 2);
            assert_eq!(cached_hmm_index(&cache, &b).unwrap().len(), 1);
        }
        // Both are resident, and the most recent use leads.
        let guard = lock_poison_tolerant(&cache);
        assert_eq!(guard.len(), 2, "both libraries should stay cached");
        assert_eq!(guard[0].path, b);
    }

    /// Only `HMM_INDEX_CACHE_SLOTS` libraries are kept, so browsing many HMM files
    /// can't grow the cache without bound.
    #[test]
    fn the_hmm_index_cache_is_bounded() {
        let dir = scratch("hmm-cache-bound");
        let cache: HmmIndexCache = Default::default();
        for i in 0..HMM_INDEX_CACHE_SLOTS + 3 {
            let p = hmm_library(&dir, &format!("{i}.hmm"), &[("X", 5)]);
            cached_hmm_index(&cache, &p).unwrap();
        }
        assert_eq!(lock_poison_tolerant(&cache).len(), HMM_INDEX_CACHE_SLOTS);
    }

    /// A rewritten library must be re-scanned, not served from the cache.
    #[test]
    fn the_hmm_index_cache_notices_a_changed_file() {
        let dir = scratch("hmm-cache-stale");
        let a = hmm_library(&dir, "a.hmm", &[("AAA", 10)]);
        let cache: HmmIndexCache = Default::default();
        assert_eq!(cached_hmm_index(&cache, &a).unwrap().len(), 1);
        hmm_library(&dir, "a.hmm", &[("AAA", 10), ("BBB", 20)]);
        assert_eq!(
            cached_hmm_index(&cache, &a).unwrap().len(),
            2,
            "a rewritten library must be re-indexed"
        );
        assert_eq!(
            lock_poison_tolerant(&cache).len(),
            1,
            "no duplicate entry for the same path"
        );
    }

    /// Every "save as" and "extract to" path in the GUI funnels through this, and
    /// the name comes straight from a text box. It is the only thing keeping a
    /// write inside the current directory.
    #[test]
    fn a_target_name_cannot_escape_its_directory() {
        let dir = scratch("escape");
        for bad in [
            "../evil.gb",
            "..",
            ".",
            "sub/evil.gb",
            "/etc/passwd",
            "..\\evil.gb",
            "a\\b.gb",
            "",
            "   ",
        ] {
            assert!(
                validated_target(&dir, bad).is_err(),
                "{bad:?} should not be accepted as a file name"
            );
        }
    }

    #[test]
    fn the_machinery_directory_is_reserved() {
        let dir = scratch("reserved");
        // Case-insensitively, so it cannot be shadowed on a case-folding filesystem.
        for name in [".domainator", ".DOMAINATOR", ".Domainator"] {
            let err = validated_target(&dir, name).unwrap_err();
            assert!(err.contains("reserved"), "{name}: {err}");
        }
    }

    /// The workspace browser only lists file types it recognizes, so writing an
    /// unknown extension would create a file the user cannot see again.
    #[test]
    fn only_recognized_file_types_can_be_created() {
        let dir = scratch("types");
        for ok in ["a.gb", "b.fasta", "c.hmm", "d.tsv", "e.dsnv", "f.hdf5"] {
            assert!(
                validated_target(&dir, ok).is_ok(),
                "{ok} should be accepted"
            );
        }
        let err = validated_target(&dir, "notes.rtf").unwrap_err();
        assert!(err.contains("unrecognized file type"), "{err}");
        // A name with no extension at all is equally unclassifiable.
        assert!(validated_target(&dir, "README").is_err());
    }

    #[test]
    fn an_existing_file_is_never_silently_overwritten() {
        let dir = scratch("existing");
        let target = std::path::Path::new(&dir).join("x.gb");
        std::fs::write(&target, b"original").unwrap();
        let err = validated_target(&dir, "x.gb").unwrap_err();
        assert!(err.contains("already exists"), "{err}");
        assert_eq!(std::fs::read(&target).unwrap(), b"original");
        // Surrounding whitespace is trimmed, so it cannot be used to sneak past.
        assert!(validated_target(&dir, "  x.gb  ").is_err());
    }

    #[test]
    fn fasta_names_cover_the_extensions_the_workspace_accepts() {
        for yes in ["a.fasta", "a.FA", "x.fna", "x.faa", "x.fap", "x.pep"] {
            assert!(is_fasta_name(yes), "{yes}");
        }
        for no in ["a.gb", "a.fasta.gz", "fasta", "a.hmm"] {
            assert!(!is_fasta_name(no), "{no}");
        }
    }

    /// The score histogram behind the matrix report. Distances can be negative or
    /// non-finite, which is where a naive binning divides by zero or panics.
    #[test]
    fn score_histograms_need_two_finite_distinct_values() {
        assert_eq!(histogram(&[], 10), json!([]));
        assert_eq!(histogram(&[1.0], 10), json!([]));
        assert_eq!(histogram(&[2.0, 2.0, 2.0], 10), json!([]), "no spread");
        assert_eq!(histogram(&[f64::NAN, 1.0], 10), json!([]));
        assert_eq!(histogram(&[f64::INFINITY, 1.0], 10), json!([]));
    }

    #[test]
    fn every_score_lands_in_a_bin_including_the_maximum() {
        let scores = [-4.0, -2.0, 0.0, 2.0, 4.0];
        let bins = histogram(&scores, 4);
        let rows = bins.as_array().unwrap();
        assert_eq!(rows.len(), 4);
        // Bins start at the minimum and are equal width across the range.
        assert_eq!(rows[0][0].as_f64().unwrap(), -4.0);
        assert_eq!(rows[1][0].as_f64().unwrap(), -2.0);
        let total: i64 = rows.iter().map(|r| r[1].as_i64().unwrap()).sum();
        assert_eq!(
            total,
            scores.len() as i64,
            "the maximum must not fall off the end"
        );
    }

    /// The Enum Report tab takes the same column objects as the export's builder,
    /// taxonomy columns included; it used to take bare names of the static columns
    /// and load no taxonomy.
    #[test]
    fn enum_report_tab_runs_column_objects_with_a_taxonomy() {
        let (gb, tax) = crate::cmd_tools::tests::enum_report_fixture("enum-tab");
        let columns = vec![
            json!({"type": "length"}),
            json!({"type": "taxname", "ranks": ["genus", "species"]}),
            json!({"type": "taxid_lineage"}),
        ];
        let report = run_enum_report_blocking(
            gb.to_string_lossy().into_owned(),
            Some("contig".into()),
            columns.clone(),
            Some(tax.to_string_lossy().into_owned()),
        )
        .unwrap();
        assert_eq!(
            report,
            json!({
                "rows": [{
                    "contig": "rec1", "length": 12, "taxname_genus": "Escherichia",
                    "taxname_species": "Escherichia coli", "taxid_lineage": "2; 561; 562"
                }],
                "targets": [[0, null]]
            })
        );

        // No taxonomy chosen: refused with what to do, before the file is read.
        let err = run_enum_report_blocking(gb.to_string_lossy().into_owned(), None, columns, None)
            .unwrap_err();
        assert!(err.contains("taxonomy database"), "{err}");

        // No columns: the per-granularity default set.
        let report = run_enum_report_blocking(
            gb.to_string_lossy().into_owned(),
            Some("contig".into()),
            vec![],
            None,
        )
        .unwrap();
        let keys: Vec<&String> = report["rows"][0].as_object().unwrap().keys().collect();
        assert_eq!(keys, ["contig", "length", "cds_count", "domains"]);
    }

    /// A CDS row's target is the CDS's index among the record's features, the
    /// same `feat_index` Contig View reports, so the row's "go to" button lands
    /// on that feature.
    #[test]
    fn enum_report_targets_are_contig_view_feature_indices() {
        let (gb, _) = crate::cmd_tools::tests::enum_report_fixture("enum-targets");
        let path = gb.to_string_lossy().into_owned();
        let report =
            run_enum_report_blocking(path.clone(), Some("cds".into()), vec![], None).unwrap();
        assert_eq!(report["rows"].as_array().unwrap().len(), 1);
        assert_eq!(report["targets"], json!([[0, 1]]));
        let view = build_contig_file_view(&path, &read_genbank(&path).unwrap());
        let feat = view.contigs[0]
            .features
            .iter()
            .find(|f| f.feat_index == 1)
            .unwrap();
        assert_eq!(feat.kind, "CDS");
    }

    /// Each selected item becomes its own record, in order: a whole contig as is,
    /// a region sliced and, on the minus strand, reverse-complemented.
    #[test]
    fn extract_selection_writes_one_record_per_item() {
        let (gb, _) = crate::cmd_tools::tests::enum_report_fixture("extract-sel");
        let dir = gb.parent().unwrap().to_string_lossy().into_owned();
        let item = |start, end, strand| ExtractItem {
            contig: 0,
            start,
            end,
            strand,
        };
        let out = extract_selection_blocking(
            &dir,
            "sel.gb",
            &gb.to_string_lossy(),
            &[
                item(None, None, None),
                item(Some(3), Some(9), Some(-1)),
                item(Some(0), Some(6), None),
            ],
            false,
        )
        .unwrap();
        let recs = read_genbank(&out).unwrap();
        let seqs: Vec<String> = recs
            .iter()
            .map(|r| String::from_utf8_lossy(&r.seq).to_lowercase())
            .collect();
        assert_eq!(seqs, ["atgaaacccggg", "gggttt", "atgaaa"]);
        assert_eq!(recs[0].id, "rec1");

        // The peptide path translates the CDSs the items hold.
        let out = extract_selection_blocking(
            &dir,
            "sel_pep.gb",
            &gb.to_string_lossy(),
            &[item(None, None, None)],
            true,
        )
        .unwrap();
        let peps = read_genbank(&out).unwrap();
        assert_eq!(peps.len(), 1);
        assert!(peps[0].is_protein());

        assert!(
            extract_selection_blocking(&dir, "none.gb", &gb.to_string_lossy(), &[], false).is_err()
        );
    }

    /// `enum_report`'s `OutputFormat::Json` writes NDJSON, so the grid must parse
    /// it line by line. Parsing the buffer as one document worked for a
    /// single-row report and failed for every real one — the exact shape a
    /// single-row unit test would miss, so this asserts on multiple rows.
    #[test]
    fn ndjson_rows_become_a_json_array() {
        let buf = b"{\"contig\":\"a\",\"length\":10}\n{\"contig\":\"b\",\"length\":20}\n";
        let v = ndjson_to_array(buf).expect("multi-row NDJSON must parse");
        let rows = v.as_array().expect("an array");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["contig"], "a");
        assert_eq!(rows[1]["length"], 20);
        // Column order is load-bearing for the grid (serde_json `preserve_order`).
        let keys: Vec<_> = rows[0].as_object().unwrap().keys().collect();
        assert_eq!(keys, ["contig", "length"]);
        // Empty output is an empty grid, not an error.
        assert_eq!(ndjson_to_array(b"").unwrap().as_array().unwrap().len(), 0);
        assert!(ndjson_to_array(b"{not json}\n").is_err());
    }
}
