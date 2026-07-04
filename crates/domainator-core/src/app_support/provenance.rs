//! Provenance records for GUI-run tool jobs. Each run writes
//! `<workspace>/.domainator/jobs/<job_id>/job.json` capturing the tool, its
//! resolved parameters, input/output files, and timing — and stamps each output's
//! metadata sidecar with `created_by_job`. Together these form a files↔jobs DAG
//! the UI can walk (e.g. SSN node → source sequence).

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::app_support::workspace::{dot_dir, FileMeta};

/// One input or output file of a job, with its role (e.g. `"matrix"`, `"ssnv"`).
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
