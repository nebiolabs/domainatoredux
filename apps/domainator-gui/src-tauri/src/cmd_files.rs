//! File-inspection / viewer-data commands.

use domainator_core::app_support::contig_view::{build_contig_file_view, ContigFileView};
use domainator_core::app_support::hmm_logo::{
    index_hmm_file, logo_from_hmm_bytes, HmmEntry, HmmLogo, HmmSummary,
};
use domainator_core::app_support::summary_stats::{summarize, SummaryStats};
use domainator_core::app_support::workspace::{classify_file, ensure_dot_subdir};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use domainator_core::io::{detect_filetype, FileType};
use domainator_core::io::genbank::{read_genbank, write_genbank};
use domainator_core::io::fasta::read_fasta;
use domainator_core::seq::SeqRecord;
use domainator_core::matrix::read_matrix;
use domainator_core::tools::build_ssn_viewer::render_html;
use domainator_core::tools::enum_report::{enum_report, static_analysis, By, OutputFormat};
use domainator_core::tools::matrix_report::{
    matrix_report, to_json, DEFAULT_MAX_MERGE, DEFAULT_MERGE_IMPACT_METRIC,
};
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
            FileType::Genbank => read_genbank(&path).map_err(|e| format!("reading GenBank: {e}"))?,
            FileType::Fasta => read_fasta(&path, None).map_err(|e| format!("reading FASTA: {e}"))?,
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
        let mut payload = to_json(&report);
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
    if scores.len() < 2 {
        return json!([]);
    }
    let min = scores.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = scores.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    if !(min.is_finite() && max.is_finite()) || max <= min {
        return json!([]);
    }
    let width = (max - min) / bins as f64;
    let mut counts = vec![0i64; bins];
    for &s in scores {
        let mut idx = ((s - min) / width) as usize;
        if idx >= bins {
            idx = bins - 1;
        }
        counts[idx] += 1;
    }
    let out: Vec<Value> =
        counts.iter().enumerate().map(|(i, &c)| json!([min + i as f64 * width, c])).collect();
    Value::Array(out)
}

// Phase-2 note: the matrix report assumes a symmetric SSN-style matrix (MaxTree);
// query×reference matrices aren't meaningfully handled yet.

/// Pure-data summary statistics for a sequence file (Summary dashboard).
#[tauri::command]
pub async fn run_summary_stats(path: String) -> Result<SummaryStats, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let kind = detect_filetype(std::path::Path::new(&path))
            .ok_or_else(|| format!("not a sequence file: {path}"))?;
        let records = match kind {
            FileType::Genbank => read_genbank(&path).map_err(|e| format!("reading GenBank: {e}"))?,
            FileType::Fasta => read_fasta(&path, None).map_err(|e| format!("reading FASTA: {e}"))?,
        };
        Ok(summarize(&records))
    })
    .await
    .map_err(|e| format!("summary failed: {e}"))?
}

/// Run `enum_report` on a sequence file and return a JSON array of typed row
/// objects for a data grid. `by` is `contig` | `cds` | `domain` (default `cds`);
/// a sensible default column set is used per granularity. Column order is
/// preserved (serde_json `preserve_order`); the grid infers numeric vs text
/// columns from the JSON value kinds.
#[tauri::command]
pub async fn run_enum_report(
    path: String,
    by: Option<String>,
    columns: Option<Vec<String>>,
) -> Result<Value, String> {
    // Off the UI thread: an 11 MB GenBank has thousands of CDS and takes a while.
    tauri::async_runtime::spawn_blocking(move || run_enum_report_blocking(path, by, columns))
        .await
        .map_err(|e| format!("enum report failed: {e}"))?
}

/// Default columns per granularity (all valid static analyses).
pub(crate) fn default_columns(by: &str) -> Vec<String> {
    let names: &[&str] = match by {
        "contig" => &["length", "cds_count", "domains"],
        "domain" => &["start", "end", "strand", "score", "identity"],
        _ => &["domains", "architecture", "length"],
    };
    names.iter().map(|s| s.to_string()).collect()
}

fn run_enum_report_blocking(
    path: String,
    by: Option<String>,
    columns: Option<Vec<String>>,
) -> Result<Value, String> {
    let kind = detect_filetype(std::path::Path::new(&path))
        .ok_or_else(|| format!("not a sequence file: {path}"))?;
    let records = match kind {
        FileType::Genbank => read_genbank(&path).map_err(|e| format!("reading GenBank: {e}"))?,
        FileType::Fasta => read_fasta(&path, None).map_err(|e| format!("reading FASTA: {e}"))?,
    };
    let by_str = by.as_deref().unwrap_or("cds");
    let by = match by_str {
        "contig" => By::Contig,
        "domain" => By::Domain,
        _ => By::Cds,
    };
    let names = match columns {
        Some(c) if !c.is_empty() => c,
        _ => default_columns(by_str),
    };
    let analyses: Vec<_> = names.iter().filter_map(|n| static_analysis(n)).collect();

    let filename = std::path::Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let recs: Vec<_> = records.into_iter().map(|r| (r, filename.clone())).collect();

    let mut buf: Vec<u8> = Vec::new();
    enum_report(recs, by, &analyses, None, None, None, OutputFormat::Json, &mut buf)?;
    serde_json::from_slice(&buf).map_err(|e| format!("parsing enum_report json: {e}"))
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
        let bytes = read_byte_range(&path, entry.byte_start, entry.byte_end)?;
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

fn file_mtime_ns(path: &str) -> Result<u128, String> {
    let m = std::fs::metadata(path).map_err(|e| format!("stat {path}: {e}"))?;
    let t = m.modified().map_err(|e| format!("mtime {path}: {e}"))?;
    Ok(t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0))
}

fn file_size(path: &str) -> Result<u64, String> {
    Ok(std::fs::metadata(path).map_err(|e| format!("stat {path}: {e}"))?.len())
}

/// Cached header-only index of the most-recently-touched HMM library, so listing
/// profiles, loading a single logo, and extracting profiles don't each re-scan (or,
/// worse, fully re-parse) the whole file. On Pfam-scale libraries (20k+ models,
/// gigabytes) the first access scans once; every access after that is O(1) seeks.
/// Holds one file at a time (replaced on path/size/mtime change) to bound memory.
type HmmIndexCache = std::sync::Arc<std::sync::Mutex<Option<HmmIndexEntry>>>;

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
fn cached_hmm_index(cache: &HmmIndexCache, path: &str) -> Result<std::sync::Arc<Vec<HmmEntry>>, String> {
    let size = file_size(path)?;
    let mtime_ns = file_mtime_ns(path)?;
    let mut guard = cache.lock().unwrap();
    if let Some(e) = guard.as_ref() {
        if e.path == path && e.size == size && e.mtime_ns == mtime_ns {
            return Ok(e.entries.clone());
        }
    }
    let entries = std::sync::Arc::new(index_hmm_file(path)?);
    *guard = Some(HmmIndexEntry { path: path.to_string(), size, mtime_ns, entries: entries.clone() });
    Ok(entries)
}

/// Read the exact byte range `[start, end)` of `path` into a buffer.
fn read_byte_range(path: &str, start: u64, end: u64) -> Result<Vec<u8>, String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).map_err(|e| format!("opening {path}: {e}"))?;
    f.seek(SeekFrom::Start(start)).map_err(|e| format!("seeking {path}: {e}"))?;
    let len = end.saturating_sub(start) as usize;
    let mut buf = vec![0u8; len];
    f.read_exact(&mut buf).map_err(|e| format!("reading {path}: {e}"))?;
    Ok(buf)
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
            if start >= e { Vec::new() } else { seqs[start..e].to_vec() }
        };
        {
            let guard = cache.lock().unwrap();
            if let Some(entry) = guard.as_ref() {
                if entry.path == path && entry.mtime_ns == mtime {
                    return Ok(slice(&entry.seqs));
                }
            }
        }
        let seqs = parse_all_seqs(&path)?;
        let out = slice(&seqs);
        *cache.lock().unwrap() = Some(SeqCacheEntry { path, mtime_ns: mtime, seqs });
        Ok(out)
    })
    .await
    .map_err(|e| format!("reading sequences failed: {e}"))?
}

/// Per-file record-count cache (path → (size, mtime_ns, count)), so the file
/// table's "records" column doesn't re-stream files on every folder revisit.
/// Bounded only by the number of distinct files browsed in a session.
#[derive(Default)]
pub struct RecordCountState(pub std::sync::Mutex<std::collections::HashMap<String, (u64, u128, Option<u64>)>>);

/// Record counts for `paths` (streamed once per file, then cached). `null` for
/// types without a meaningful count (matrices/tables) or unreadable files. Called
/// lazily by the file table only when the "records" column is shown, so plain
/// folder listing stays instant.
#[tauri::command]
pub async fn count_records(
    state: tauri::State<'_, RecordCountState>,
    paths: Vec<String>,
) -> Result<Vec<Option<u64>>, String> {
    let map = &state.0;
    let mut out = Vec::with_capacity(paths.len());
    for p in &paths {
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
        if let Some(&(cs, cm, cnt)) = map.lock().unwrap().get(p) {
            if cs == size && cm == mtime {
                out.push(cnt);
                continue;
            }
        }
        let count = domainator_core::app_support::workspace::count_records(std::path::Path::new(p));
        map.lock().unwrap().insert(p.clone(), (size, mtime, count));
        out.push(count);
    }
    Ok(out)
}

/// Read a text file (e.g. a TSV/CSV table) as a UTF-8 string. Rejects files over
/// ~64 MB so the table editor can't accidentally slurp a giant blob into the UI.
#[tauri::command]
pub async fn read_text_file(path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let meta = std::fs::metadata(&path).map_err(|e| format!("stat {path}: {e}"))?;
        if meta.len() > 64 * 1024 * 1024 {
            return Err(format!("file too large to edit ({} MB)", meta.len() / 1024 / 1024));
        }
        std::fs::read_to_string(&path).map_err(|e| format!("reading {path}: {e}"))
    })
    .await
    .map_err(|e| format!("read failed: {e}"))?
}

/// Overwrite a text file in place (used by the table editor's Save). Writes
/// atomically via a temp file + rename so a crash mid-write can't truncate it.
#[tauri::command]
pub async fn write_text_file(path: String, contents: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
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
/// `None` if the user cancelled. `default_name` seeds the dialog's filename;
/// `extension`/`filter_name` set the type filter (e.g. `"svg"` / `"SVG image"`).
#[tauri::command]
pub fn save_text_file(
    default_name: String,
    extension: String,
    filter_name: String,
    contents: String,
) -> Result<Option<String>, String> {
    let picked = rfd::FileDialog::new()
        .set_file_name(&default_name)
        .add_filter(&filter_name, &[extension.as_str()])
        .save_file();
    match picked {
        Some(path) => {
            std::fs::write(&path, contents).map_err(|e| format!("writing file: {e}"))?;
            Ok(Some(path.to_string_lossy().into_owned()))
        }
        None => Ok(None),
    }
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
/// This is the cross-view "open the source sequence(s)" resolver. It is a batch
/// so that selecting N nodes costs a single workspace scan instead of N — a
/// per-label scan made "Open sequence(s)" appear to hang on real workspaces. A
/// `.didx`/provenance-indexed fast path can replace the scan later.
#[tauri::command]
pub async fn resolve_records(
    workspace_root: String,
    labels: Vec<String>,
) -> Result<Vec<Option<ResolvedRecord>>, String> {
    // Off the UI thread: this reads every GenBank/FASTA in the workspace, so on a
    // real workspace it can take seconds — a synchronous command would freeze the
    // whole app (all IPC) while it scans.
    tauri::async_runtime::spawn_blocking(move || {
        use std::collections::{HashMap, HashSet};
        let wanted: HashSet<&str> = labels.iter().map(String::as_str).collect();
        // label -> resolved record (first match in traversal order wins).
        let mut found: HashMap<String, ResolvedRecord> = HashMap::new();

        let mut stack = vec![std::path::PathBuf::from(&workspace_root)];
        'scan: while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                let Ok(ft) = entry.file_type() else { continue };
                if ft.is_dir() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if !name.starts_with('.') && name != "jobs" {
                        stack.push(path);
                    }
                    continue;
                }
                let Some(kind) = detect_filetype(&path) else { continue };
                let path_str = path.to_string_lossy().into_owned();
                let records = match kind {
                    FileType::Genbank => read_genbank(&path_str).ok(),
                    FileType::Fasta => read_fasta(&path_str, None).ok(),
                };
                let Some(records) = records else { continue };
                let file_type = classify_file(&path).unwrap_or("genbank").to_string();
                for (i, rec) in records.iter().enumerate() {
                    // A record can satisfy a label by either its id or its name.
                    for key in [rec.id.as_str(), rec.name.as_str()] {
                        if wanted.contains(key) && !found.contains_key(key) {
                            found.insert(
                                key.to_string(),
                                ResolvedRecord {
                                    path: path_str.clone(),
                                    file_type: file_type.clone(),
                                    contig_index: i,
                                    contig_id: rec.id.clone(),
                                },
                            );
                        }
                    }
                }
                // Every requested label located — no need to keep scanning.
                if found.len() >= wanted.len() {
                    break 'scan;
                }
            }
        }

        Ok(labels.iter().map(|l| found.get(l).cloned()).collect())
    })
    .await
    .map_err(|e| format!("resolve records failed: {e}"))?
}

/// Render the self-contained SSN viewer HTML for a `.ssnv` bundle, with the
/// bundle embedded so it autoloads. Hosted by the frontend in an `<iframe>`.
#[tauri::command]
pub async fn render_ssn_html(path: String) -> Result<String, String> {
    // Off the UI thread: an .ssnv bundle can be large, and embedding it into the
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

static SUBSET_COUNTER: AtomicU64 = AtomicU64::new(0);

/// One selected record: its source file and 0-based contig index.
#[derive(Deserialize)]
pub struct RecordRef {
    pub path: String,
    pub contig_index: usize,
}

/// Validate a bare filename and join it to `dir`, refusing path separators, the
/// reserved `.domainator` name, and pre-existing targets (caller supplies a name).
fn validated_target(dir: &str, name: &str) -> Result<std::path::PathBuf, String> {
    use domainator_core::app_support::workspace::{accepted_types_hint, classify_file, is_reserved_name};
    let clean = name.trim();
    if clean.is_empty() || clean.contains('/') || clean.contains('\\') || clean == "." || clean == ".." {
        return Err("invalid file name".into());
    }
    if is_reserved_name(clean) {
        return Err(format!("'{clean}' is reserved"));
    }
    let p = std::path::Path::new(dir).join(clean);
    // Refuse unrecognized extensions — the workspace only lists known data types,
    // so a mystery extension would create a file the browser can't show.
    if classify_file(&p).is_none() {
        return Err(format!("unrecognized file type for '{clean}'. {}", accepted_types_hint()));
    }
    if p.exists() {
        return Err(format!("already exists: {clean}"));
    }
    Ok(p)
}

fn is_fasta_name(name: &str) -> bool {
    let l = name.to_lowercase();
    [".fasta", ".fa", ".fna", ".faa", ".fap", ".pep"].iter().any(|e| l.ends_with(e))
}

/// Notify every window that files under `dir` changed, so file browsers refresh
/// (e.g. after an Extract from a popped-out window creates a file elsewhere).
fn notify_files_changed(app: &tauri::AppHandle, dir: &str) {
    use tauri::Emitter;
    let _ = app.emit("files-changed", dir.to_string());
}

/// Write the selected contigs (0-based `indices`, in order) from `source` into a new
/// file `dir/name`. Output format follows `name`'s extension (FASTA vs GenBank).
#[tauri::command]
pub async fn extract_contigs(
    app: tauri::AppHandle,
    dir: String,
    name: String,
    source: String,
    indices: Vec<usize>,
) -> Result<String, String> {
    let dir_c = dir.clone();
    let out = tauri::async_runtime::spawn_blocking(move || {
        let recs = load_sequence_any(&source)?;
        let out: Vec<SeqRecord> = indices.iter().filter_map(|&i| recs.get(i).cloned()).collect();
        if out.is_empty() {
            return Err("no contigs selected".into());
        }
        let target = validated_target(&dir, &name)?;
        let ts = target.to_string_lossy().into_owned();
        if is_fasta_name(&name) {
            domainator_core::io::fasta::write_fasta(&ts, &out).map_err(|e| format!("writing FASTA: {e}"))?;
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

/// Extract a coordinate region [start, end) (0-based half-open) from contig `index`
/// of `source` into a new GenBank file `dir/name`, preserving the features that fall
/// in the region (mirrors `select-by-coord`). When `translate` is set, the sliced
/// region is then run through `extract_peptides` so the output is a protein GenBank
/// of the CDSs in the region (the peptide equivalent of the nucleotide slice).
#[tauri::command]
pub async fn extract_region(
    app: tauri::AppHandle,
    dir: String,
    name: String,
    source: String,
    index: usize,
    start: usize,
    end: usize,
    translate: bool,
) -> Result<String, String> {
    let dir_c = dir.clone();
    let out = tauri::async_runtime::spawn_blocking(move || {
        use domainator_core::tools::extract_peptides::{extract_peptides, ExtractPeptidesOptions};
        use domainator_core::tools::select_by_coord::{extract_coordinate_region, CoordinateSelection};
        let recs = load_sequence_any(&source)?;
        let contig = recs.get(index).ok_or_else(|| format!("no contig at index {index}"))?;
        if end <= start {
            return Err("empty region".into());
        }
        let sel = CoordinateSelection {
            contig: contig.name.clone(),
            start: start as i64 + 1, // 1-based inclusive
            end: end as i64,
            strand: 1,
            file: None,
        };
        let rec = extract_coordinate_region(contig, &sel)?;
        let target = validated_target(&dir, &name)?;
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
            };
            let peps = extract_peptides(std::slice::from_ref(&rec), &opts);
            if peps.is_empty() {
                return Err("no CDS features in the selected region to translate".into());
            }
            write_genbank(&ts, &peps).map_err(|e| format!("writing GenBank: {e}"))?;
        } else {
            write_genbank(&ts, std::slice::from_ref(&rec)).map_err(|e| format!("writing GenBank: {e}"))?;
        }
        Ok::<String, String>(ts)
    })
    .await
    .map_err(|e| format!("extract failed: {e}"))??;
    notify_files_changed(&app, &dir_c);
    Ok(out)
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
        let entries = cached_hmm_index(&state, &source)?;
        let file = std::fs::File::create(&ts).map_err(|e| format!("creating {ts}: {e}"))?;
        let mut w = std::io::BufWriter::new(file);
        let mut n = 0usize;
        for &i in &indices {
            if let Some(e) = entries.get(i) {
                let bytes = read_byte_range(&source, e.byte_start, e.byte_end)?;
                w.write_all(&bytes).map_err(|e| format!("writing {ts}: {e}"))?;
                n += 1;
            }
        }
        w.flush().map_err(|e| format!("flushing {ts}: {e}"))?;
        if n == 0 {
            return Err("no profiles selected".to_string());
        }
        Ok::<String, String>(ts)
    })
    .await
    .map_err(|e| format!("extract failed: {e}"))??;
    notify_files_changed(&app, &dir_c);
    Ok(out)
}

/// Create a new text file `dir/name` with `contents` (New-file-from-pasted-text).
#[tauri::command]
pub async fn create_text_file(
    app: tauri::AppHandle,
    dir: String,
    name: String,
    contents: String,
) -> Result<String, String> {
    let dir_c = dir.clone();
    let out = tauri::async_runtime::spawn_blocking(move || {
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
/// several source files), written under `<root>/.domainator/subsets/`. Returns its
/// path. The SSN viewer opens the result as one combined sequence view (contig /
/// enum / summary tabs) instead of N whole-file windows.
#[tauri::command]
pub async fn extract_records(workspace_root: String, records: Vec<RecordRef>) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        use std::collections::HashMap;
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
        let dir = ensure_dot_subdir(std::path::Path::new(&workspace_root), "subsets")
            .map_err(|e| format!("creating subsets dir: {e}"))?;
        let n = SUBSET_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!("selection_{n}.gb"));
        write_genbank(&path.to_string_lossy(), &out).map_err(|e| format!("writing subset: {e}"))?;
        Ok(path.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| format!("extract failed: {e}"))?
}
