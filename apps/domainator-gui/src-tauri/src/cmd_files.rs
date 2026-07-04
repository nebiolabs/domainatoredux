//! File-inspection / viewer-data commands.

use domainator_core::app_support::contig_view::{build_contig_file_view, ContigFileView};
use domainator_core::app_support::hmm_logo::{list_hmms, load_logo_from_file, HmmLogo, HmmSummary};
use domainator_core::app_support::summary_stats::{summarize, SummaryStats};
use domainator_core::app_support::workspace::classify_file;
use serde::Serialize;
use domainator_core::io::{detect_filetype, FileType};
use domainator_core::io::genbank::read_genbank;
use domainator_core::io::fasta::read_fasta;
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
pub fn load_matrix_report(path: String) -> Result<Value, String> {
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
pub async fn load_hmm_logo(path: String, index: Option<usize>) -> Result<HmmLogo, String> {
    tauri::async_runtime::spawn_blocking(move || load_logo_from_file(&path, index.unwrap_or(0)))
        .await
        .map_err(|e| format!("loading HMM logo failed: {e}"))?
}

/// Summaries of every profile in an HMM file (for the browser list).
#[tauri::command]
pub async fn list_hmm_profiles(path: String) -> Result<Vec<HmmSummary>, String> {
    tauri::async_runtime::spawn_blocking(move || list_hmms(&path))
        .await
        .map_err(|e| format!("listing HMMs failed: {e}"))?
}

/// A record located in a workspace sequence file (for SSN → sequence navigation).
#[derive(Serialize)]
pub struct ResolvedRecord {
    pub path: String,
    pub file_type: String,
    pub contig_index: usize,
    pub contig_id: String,
}

/// Resolve an SSN node label (a sequence id/name, from a matrix row) to the
/// sequence file + contig index that contains it, by scanning the workspace's
/// GenBank/FASTA files. Returns the first match (or `None`). This is the
/// cross-view "open the source sequence" resolver; a `.didx`/provenance-indexed
/// fast path can replace the scan later for large workspaces.
#[tauri::command]
pub fn resolve_record(workspace_root: String, label: String) -> Result<Option<ResolvedRecord>, String> {
    let mut stack = vec![std::path::PathBuf::from(&workspace_root)];
    while let Some(dir) = stack.pop() {
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
            for (i, rec) in records.iter().enumerate() {
                if rec.id == label || rec.name == label {
                    return Ok(Some(ResolvedRecord {
                        path: path_str,
                        file_type: classify_file(&path).unwrap_or("genbank").to_string(),
                        contig_index: i,
                        contig_id: rec.id.clone(),
                    }));
                }
            }
        }
    }
    Ok(None)
}

/// Render the self-contained SSN viewer HTML for a `.ssnv` bundle, with the
/// bundle embedded so it autoloads. Hosted by the frontend in an `<iframe>`.
#[tauri::command]
pub fn render_ssn_html(path: String) -> Result<String, String> {
    let bytes = std::fs::read(&path).map_err(|e| format!("reading SSN bundle: {e}"))?;
    let title = std::path::Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "SSN".into());
    Ok(render_html(&title, Some(&bytes)))
}
