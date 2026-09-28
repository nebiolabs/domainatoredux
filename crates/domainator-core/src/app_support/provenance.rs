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

/// Input roles whose files hold node sequences: the leaves of
/// [`trace_source_sequences`]. The GUI derives each job's input roles from its
/// tool spec, and its golden-table test checks them against these.
pub const SEQUENCE_SOURCE_ROLES: [&str; 3] = ["sequence", "genbank", "fasta"];

/// Input roles [`trace_source_sequences`] follows one job further back.
pub const FOLLOWED_INPUT_ROLES: [&str; 1] = ["matrix"];

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
    /// The Python `domainator` release `engine_version` was at parity with.
    /// Absent (empty) in jobs recorded before it was stamped.
    #[serde(default)]
    pub upstream_version: String,
}

/// Milliseconds since the Unix epoch (monotonic enough for job ordering/ids).
pub fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
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
    Some(
        t.duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0),
    )
}

fn source_status(path: &str, consumed_at_ms: u128) -> SourceFile {
    let p = Path::new(path);
    let present = p.exists();
    let modified_since = present && file_mtime_ms(p).is_some_and(|m| m > consumed_at_ms);
    SourceFile {
        path: path.to_string(),
        present,
        modified_since,
    }
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
        let Some(job_id) = FileMeta::load(root, &p).created_by_job else {
            continue;
        };
        if !seen_jobs.insert(job_id.clone()) {
            continue;
        }
        let Some(job) = load_job(root, &job_id) else {
            continue;
        };
        found_job = true;
        for inp in &job.inputs {
            let role = inp.role.as_str();
            if SEQUENCE_SOURCE_ROLES.contains(&role) {
                if seen_sources.insert(inp.path.clone()) {
                    sources.push(source_status(&inp.path, job.finished_at_ms));
                }
            } else if FOLLOWED_INPUT_ROLES.contains(&role) {
                queue.push(std::path::PathBuf::from(&inp.path));
            }
            // Anything else (references, tables) doesn't hold node sequences.
        }
    }
    SourceTrace {
        has_provenance: found_job && !sources.is_empty(),
        sources,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(name: &str) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("domainator-prov-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn touch(root: &Path, name: &str) -> String {
        let p = root.join(name);
        std::fs::write(&p, b"x").unwrap();
        p.to_string_lossy().into_owned()
    }

    fn io(path: &str, role: &str) -> IoRef {
        IoRef {
            path: path.to_string(),
            role: role.to_string(),
        }
    }

    fn job(id: &str, inputs: Vec<IoRef>, outputs: Vec<IoRef>) -> JobRecord {
        JobRecord {
            job_id: id.to_string(),
            tool: "seq-dist".into(),
            params: serde_json::json!({}),
            inputs,
            outputs,
            started_at_ms: 1_000,
            // Far in the future, so a source written *now* is not "modified since"
            // unless a test says so.
            finished_at_ms: u128::MAX / 2,
            status: "ok".into(),
            engine_version: "test".into(),
            upstream_version: "test".into(),
        }
    }

    #[test]
    fn job_ids_are_the_start_time_in_hex() {
        assert_eq!(new_job_id(0), "job_0");
        assert_eq!(new_job_id(255), "job_ff");
        // Distinct start times give distinct ids, which is what makes the job
        // directory name unique.
        assert_ne!(new_job_id(now_ms()), new_job_id(now_ms() + 1));
    }

    #[test]
    fn a_job_round_trips_and_stamps_its_outputs() {
        let root = workspace("round-trip");
        let out = touch(&root, "m.hdf5");
        let record = job(
            "job_1",
            vec![io("in.gb", "sequence")],
            vec![io(&out, "matrix")],
        );
        write_job(&root, &record, "a distance matrix").unwrap();

        let loaded = load_job(&root, "job_1").expect("job should load");
        assert_eq!(loaded.tool, "seq-dist");
        assert_eq!(loaded.outputs[0].role, "matrix");

        let meta = FileMeta::load(&root, Path::new(&out));
        assert_eq!(meta.created_by_job.as_deref(), Some("job_1"));
        assert_eq!(meta.description, "a distance matrix");
        assert!(load_job(&root, "job_missing").is_none());
    }

    /// The description is a default, not an override: a file the user has already
    /// described must not have that text replaced by a rerun.
    #[test]
    fn an_existing_description_survives_a_rerun() {
        let root = workspace("description");
        let out = touch(&root, "m.hdf5");
        let mut meta = FileMeta::load(&root, Path::new(&out));
        meta.description = "mine".into();
        meta.save(&root, Path::new(&out)).unwrap();

        write_job(
            &root,
            &job("job_2", vec![], vec![io(&out, "matrix")]),
            "generated",
        )
        .unwrap();
        let meta = FileMeta::load(&root, Path::new(&out));
        assert_eq!(meta.description, "mine");
        assert_eq!(meta.created_by_job.as_deref(), Some("job_2"));
    }

    /// The trace follows `.dsnv` -> matrix -> sequences, which is the whole point:
    /// a network's node labels come from the sequence file two jobs back, and
    /// stopping at the first hop would leave extraction disabled.
    #[test]
    fn tracing_follows_matrix_inputs_back_to_sequences() {
        let root = workspace("two-hop");
        let seqs = touch(&root, "contigs.gb");
        let matrix = touch(&root, "d.hdf5");
        let bundle = touch(&root, "n.dsnv");

        write_job(
            &root,
            &job(
                "job_a",
                vec![io(&seqs, "sequence")],
                vec![io(&matrix, "matrix")],
            ),
            "",
        )
        .unwrap();
        write_job(
            &root,
            &job(
                "job_b",
                vec![io(&matrix, "matrix"), io("pfam.hmm", "reference")],
                vec![io(&bundle, "dsnv")],
            ),
            "",
        )
        .unwrap();

        let trace = trace_source_sequences(&root, Path::new(&bundle));
        assert!(trace.has_provenance);
        let paths: Vec<&str> = trace.sources.iter().map(|s| s.path.as_str()).collect();
        // The reference is not a source of node sequences and must not appear.
        assert_eq!(paths, vec![seqs.as_str()]);
        assert!(trace.sources[0].present);
        assert!(!trace.sources[0].modified_since);
    }

    #[test]
    fn a_file_with_no_job_has_no_provenance() {
        let root = workspace("orphan");
        let bundle = touch(&root, "n.dsnv");
        let trace = trace_source_sequences(&root, Path::new(&bundle));
        assert!(!trace.has_provenance);
        assert!(trace.sources.is_empty());

        // A job with no sequence input anywhere in its chain is also unusable,
        // even though a creating job was found.
        write_job(
            &root,
            &job(
                "job_c",
                vec![io("pfam.hmm", "reference")],
                vec![io(&bundle, "dsnv")],
            ),
            "",
        )
        .unwrap();
        assert!(!trace_source_sequences(&root, Path::new(&bundle)).has_provenance);
    }

    /// Two jobs that each claim the other's file as an input would walk forever
    /// without the visited set.
    #[test]
    fn a_cycle_in_the_job_graph_terminates() {
        let root = workspace("cycle");
        let a = touch(&root, "a.hdf5");
        let b = touch(&root, "b.hdf5");
        let seqs = touch(&root, "s.gb");
        write_job(
            &root,
            &job(
                "job_x",
                vec![io(&b, "matrix"), io(&seqs, "sequence")],
                vec![io(&a, "matrix")],
            ),
            "",
        )
        .unwrap();
        write_job(
            &root,
            &job("job_y", vec![io(&a, "matrix")], vec![io(&b, "matrix")]),
            "",
        )
        .unwrap();

        let trace = trace_source_sequences(&root, Path::new(&a));
        assert!(trace.has_provenance);
        assert_eq!(
            trace.sources.len(),
            1,
            "the same source must not be listed twice"
        );
    }

    /// `modified_since` is what warns that node labels may no longer match the
    /// file they came from, so it compares against the *consuming job's* finish
    /// time, not the current time.
    #[test]
    fn a_source_edited_after_its_job_is_flagged() {
        let root = workspace("stale");
        let seqs = touch(&root, "s.gb");
        let matrix = touch(&root, "d.hdf5");
        let mut stale = job(
            "job_s",
            vec![io(&seqs, "sequence")],
            vec![io(&matrix, "matrix")],
        );
        stale.finished_at_ms = 1; // long before the file was written
        write_job(&root, &stale, "").unwrap();

        let trace = trace_source_sequences(&root, Path::new(&matrix));
        assert!(trace.sources[0].modified_since);

        // A source that has been deleted is neither present nor "modified".
        std::fs::remove_file(&seqs).unwrap();
        let trace = trace_source_sequences(&root, Path::new(&matrix));
        assert!(!trace.sources[0].present);
        assert!(!trace.sources[0].modified_since);
    }
}
