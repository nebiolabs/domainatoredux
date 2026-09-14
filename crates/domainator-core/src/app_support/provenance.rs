//! Provenance records for GUI-run tool jobs. Each run writes
//! `<workspace>/.domainator/jobs/<job_id>/job.json` capturing the tool, its
//! resolved parameters, input/output files, and timing — and stamps each output's
//! metadata sidecar with `created_by_job`. Together these form a files↔jobs DAG
//! the UI can walk (e.g. SSN node → source sequence).

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::app_support::workspace::{dot_dir, FileMeta};

/// One input or output file of a job, with its role (e.g. `"matrix"`, `"dsnv"`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IoRef {
    pub path: String,
    pub role: String,
}

/// A completed tool run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobRecord {
    pub job_id: String,
    pub tool: String,
    /// Resolved `*Config` snapshot (serde of the tool's config struct).
    pub params: serde_json::Value,
    pub inputs: Vec<IoRef>,
    pub outputs: Vec<IoRef>,
    pub started_at_ms: u128,
    pub finished_at_ms: u128,
    pub status: String,
    pub engine_version: String,
}

/// Milliseconds since the Unix epoch (monotonic enough for job ordering/ids).
pub fn now_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

/// A job id derived from the start time in millis (hex), e.g. `job_18f3a...`.
pub fn new_job_id(started_at_ms: u128) -> String {
    format!("job_{started_at_ms:x}")
}

/// Persist the job record to `<root>/jobs/<job_id>/job.json` and stamp each
/// output's sidecar `.dom.json` with this job id + description.
pub fn write_job(root: &Path, record: &JobRecord, output_description: &str) -> std::io::Result<()> {
    let dir = dot_dir(root).join("jobs").join(&record.job_id);
    std::fs::create_dir_all(&dir)?;
    let json = serde_json::to_vec_pretty(record)?;
    std::fs::write(dir.join("job.json"), json)?;

    for out in &record.outputs {
        let data_file = Path::new(&out.path);
        let mut meta = FileMeta::load(root, data_file);
        meta.created_by_job = Some(record.job_id.clone());
        if meta.description.is_empty() {
            meta.description = output_description.to_string();
        }
        meta.save(root, data_file)?;
    }
    Ok(())
}

/// Load a job record by id from `<root>/.domainator/jobs/<job_id>/job.json`.
pub fn load_job(root: &Path, job_id: &str) -> Option<JobRecord> {
    let path = dot_dir(root).join("jobs").join(job_id).join("job.json");
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// A sequence source file a network was traced back to, with liveness/staleness.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceFile {
    pub path: String,
    /// The file still exists on disk at `path`.
    pub present: bool,
    /// The file's mtime is newer than the job that consumed it — it was edited
    /// after the network was built, so its records may no longer match the node
    /// labels. Only meaningful when `present`.
    pub modified_since: bool,
}

/// Result of tracing a network artifact (`.dsnv`/matrix) back to the sequence files
/// that supplied its node labels, by walking the job DAG.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceTrace {
    /// A creating-job chain reaching at least one sequence input was found. When
    /// false, `sources` is empty and label→record resolution can't be trusted (we
    /// don't know which files hold the node sequences), so extraction is disabled.
    pub has_provenance: bool,
    pub sources: Vec<SourceFile>,
}

fn file_mtime_ms(p: &Path) -> Option<u128> {
    let t = std::fs::metadata(p).ok()?.modified().ok()?;
    Some(t.duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0))
}

fn source_status(path: &str, consumed_at_ms: u128) -> SourceFile {
    let p = Path::new(path);
    let present = p.exists();
    let modified_since = present && file_mtime_ms(p).is_some_and(|m| m > consumed_at_ms);
    SourceFile { path: path.to_string(), present, modified_since }
}

/// Trace a network artifact back to the sequence files that supplied its node
/// labels. Walks the job DAG from `data_file`: sequence/genbank/fasta inputs are
/// leaf sources (their record ids are what appear as node labels); matrix inputs
/// are followed one hop further (e.g. `.dsnv` → matrix → sequences). References,
/// color tables, etc. are ignored — they don't contribute node sequences.
///
/// Each source's `modified_since` compares its current mtime against the finish
/// time of the job that consumed it, so callers can warn that an edited source may
/// no longer match the network.
pub fn trace_source_sequences(root: &Path, data_file: &Path) -> SourceTrace {
    use std::collections::HashSet;
    let mut sources: Vec<SourceFile> = Vec::new();
    let mut seen_sources: HashSet<String> = HashSet::new();
    let mut seen_jobs: HashSet<String> = HashSet::new();
    let mut found_job = false;
    let mut queue: Vec<std::path::PathBuf> = vec![data_file.to_path_buf()];

    while let Some(p) = queue.pop() {
        let Some(job_id) = FileMeta::load(root, &p).created_by_job else { continue };
        if !seen_jobs.insert(job_id.clone()) {
            continue;
        }
        let Some(job) = load_job(root, &job_id) else { continue };
        found_job = true;
        for inp in &job.inputs {
            match inp.role.as_str() {
                "sequence" | "genbank" | "fasta" => {
                    if seen_sources.insert(inp.path.clone()) {
                        sources.push(source_status(&inp.path, job.finished_at_ms));
                    }
                }
                "matrix" => queue.push(std::path::PathBuf::from(&inp.path)),
                _ => {} // references / tables don't hold node sequences
            }
        }
    }
    SourceTrace { has_provenance: found_job && !sources.is_empty(), sources }
}
