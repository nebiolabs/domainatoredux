//! Tool listing / schema / execution.
//!
//! Forms are driven by `domainator-config`'s per-tool JSON Schema; the form's
//! values deserialize straight back into the same `*Config` struct, which is
//! resolved (shared with the CLI via `domainator_config::resolve`) and run off the
//! UI thread. Each run records provenance (`jobs/<id>/job.json` + output sidecars).
//!
//! Phase-3 scope: a curated matrix-tool set (`transform-matrix`,
//! `build-ssn-viewer`) that pairs with the existing viewers. Generic dispatch for
//! all tools, a job registry with live progress, and cancellation are deferred.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use domainator_config::resolve::{resolve_build_ssn, resolve_build_ssn_viewer, resolve_transform_matrix};
use domainator_config::{schema, BuildSsnConfig, BuildSsnViewerConfig, TransformMatrixConfig};
use domainator_core::app_support::provenance::{new_job_id, now_ms, write_job, IoRef, JobRecord};
use domainator_core::matrix::{cytoscape, hdf5_io, read_matrix, text};
use domainator_core::tools::build_ssn::build_ssn;
use domainator_core::tools::build_ssn_viewer::{build_ssn_viewer_bundle, gzip, serialize_bundle};
use domainator_core::tools::transform_matrix::transform_matrix;
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter};

/// Live state of a tool run, tracked in the [`JobRegistry`] managed state and
/// broadcast to the UI via the `job-update` event.
#[derive(Serialize, Clone)]
pub struct JobInfo {
    pub job_id: String,
    pub tool: String,
    /// `running` | `completed` | `failed` | `cancelled`.
    pub status: String,
    pub message: String,
    pub started_at_ms: String, // u128 as string (JSON-safe)
    pub outputs: Vec<String>,
}

/// Managed state: all jobs seen this session + a cancel flag set.
#[derive(Default)]
pub struct JobRegistry {
    jobs: Mutex<HashMap<String, JobInfo>>,
    cancelled: Mutex<std::collections::HashSet<String>>,
}

fn ms_str(ms: u128) -> String {
    ms.to_string()
}

#[derive(Serialize)]
pub struct ToolMeta {
    pub id: String,
    pub label: String,
    pub description: String,
}

fn meta(id: &str, label: &str, description: &str) -> ToolMeta {
    ToolMeta { id: id.into(), label: label.into(), description: description.into() }
}

/// Tools currently runnable from the GUI.
#[tauri::command]
pub fn list_tools() -> Vec<ToolMeta> {
    vec![
        meta(
            "transform-matrix",
            "Transform matrix",
            "Rescale or prune a similarity matrix (mode, lower bound, MST-kNN) and write a new matrix.",
        ),
        meta(
            "build-ssn",
            "Build SSN (XGMML)",
            "Build a Cytoscape XGMML sequence-similarity network from a matrix (edges, clustering).",
        ),
        meta(
            "build-ssn-viewer",
            "Build SSN viewer",
            "Build a .ssnv bundle from a similarity matrix for the interactive network viewer.",
        ),
    ]
}

/// The draft-07 JSON Schema for a tool's parameters (drives the form).
#[tauri::command]
pub fn tool_schema(tool: String) -> Result<String, String> {
    schema(&tool).map_err(|e| e.to_string())
}

#[derive(Serialize)]
pub struct RunResult {
    pub job_id: String,
    pub outputs: Vec<String>,
    pub message: String,
}

/// The result the caller awaits; also mirrored into the `JobRegistry` and emitted
/// as `job-update` events (so any panel can react — a running-jobs indicator, etc.).
///
/// Runs off the UI thread. `params` is the tool's `*Config` as JSON (from the
/// form); `workspace_root` is where `jobs/` lives.
#[tauri::command]
pub async fn run_tool(
    app: AppHandle,
    registry: tauri::State<'_, JobRegistry>,
    tool: String,
    params: Value,
    workspace_root: String,
) -> Result<RunResult, String> {
    let started = now_ms();
    let job_id = new_job_id(started);

    let running = JobInfo {
        job_id: job_id.clone(),
        tool: tool.clone(),
        status: "running".into(),
        message: String::new(),
        started_at_ms: ms_str(started),
        outputs: Vec::new(),
    };
    set_job(&registry, &app, running);

    // Off-thread work. Core tools aren't interruptible, so cancellation is
    // best-effort: if the job was cancelled while running, we discard the result
    // and don't write outputs/provenance.
    let jid = job_id.clone();
    let tool_c = tool.clone();
    let root_c = workspace_root.clone();
    let params_c = params.clone();
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        run_one(&tool_c, params_c, &root_c, started, &jid)
    })
    .await
    .map_err(|e| format!("job failed to run: {e}"))?;

    let cancelled = registry.cancelled.lock().unwrap().contains(&job_id);
    let mut info = JobInfo {
        job_id: job_id.clone(),
        tool: tool.clone(),
        status: String::new(),
        message: String::new(),
        started_at_ms: ms_str(started),
        outputs: Vec::new(),
    };
    match outcome {
        _ if cancelled => {
            info.status = "cancelled".into();
            info.message = "cancelled".into();
            set_job(&registry, &app, info.clone());
            return Err("job cancelled".into());
        }
        Ok(r) => {
            info.status = "completed".into();
            info.message = r.message.clone();
            info.outputs = r.outputs.clone();
            set_job(&registry, &app, info);
            Ok(r)
        }
        Err(e) => {
            info.status = "failed".into();
            info.message = e.clone();
            set_job(&registry, &app, info);
            Err(e)
        }
    }
}

/// Record a job's state and broadcast it to the UI.
fn set_job(registry: &JobRegistry, app: &AppHandle, info: JobInfo) {
    registry.jobs.lock().unwrap().insert(info.job_id.clone(), info.clone());
    let _ = app.emit("job-update", &info);
}

/// Status of one job (for polling).
#[tauri::command]
pub fn job_status(registry: tauri::State<'_, JobRegistry>, job_id: String) -> Option<JobInfo> {
    registry.jobs.lock().unwrap().get(&job_id).cloned()
}

/// All jobs seen this session (newest state).
#[tauri::command]
pub fn list_jobs(registry: tauri::State<'_, JobRegistry>) -> Vec<JobInfo> {
    registry.jobs.lock().unwrap().values().cloned().collect()
}

/// Best-effort cancel: flags the job so its result is discarded on completion.
#[tauri::command]
pub fn cancel_job(registry: tauri::State<'_, JobRegistry>, job_id: String) {
    registry.cancelled.lock().unwrap().insert(job_id);
}

/// The actual tool dispatch + provenance write (runs on the blocking pool).
fn run_one(tool: &str, params: Value, root: &str, started: u128, job_id: &str) -> Result<RunResult, String> {
    let (inputs, outputs, message) = match tool {
        "transform-matrix" => run_transform_matrix(params.clone())?,
        "build-ssn" => run_build_ssn(params.clone())?,
        "build-ssn-viewer" => run_build_ssn_viewer(params.clone())?,
        other => return Err(format!("tool not runnable from the GUI yet: {other}")),
    };
    let record = JobRecord {
        job_id: job_id.to_string(),
        tool: tool.to_string(),
        params,
        inputs,
        outputs: outputs.clone(),
        started_at_ms: started,
        finished_at_ms: now_ms(),
        status: "completed".into(),
        engine_version: env!("CARGO_PKG_VERSION").into(),
    };
    write_job(Path::new(root), &record, &format!("{tool} output"))
        .map_err(|e| format!("writing provenance: {e}"))?;
    Ok(RunResult { job_id: job_id.to_string(), outputs: outputs.into_iter().map(|o| o.path).collect(), message })
}

type RunOutput = (Vec<IoRef>, Vec<IoRef>, String);

fn run_transform_matrix(params: Value) -> Result<RunOutput, String> {
    let cfg: TransformMatrixConfig =
        serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    let input = cfg.input.clone().ok_or("missing input matrix")?;
    let opts = resolve_transform_matrix(&cfg).map_err(|e| e.to_string())?;
    let m = read_matrix(&input).map_err(|e| format!("reading matrix: {e}"))?;
    let m = transform_matrix(m, &opts).map_err(|e| format!("transform_matrix: {e}"))?;

    let mut outputs = Vec::new();
    if let Some(p) = &cfg.dense {
        hdf5_io::write_dense(p, &m).map_err(|e| format!("writing dense: {e}"))?;
        outputs.push(IoRef { path: p.clone(), role: "dense".into() });
    }
    if let Some(p) = &cfg.dense_text {
        text::write_dense_text(p, &m).map_err(|e| format!("writing dense_text: {e}"))?;
        outputs.push(IoRef { path: p.clone(), role: "dense_text".into() });
    }
    if let Some(p) = &cfg.sparse {
        hdf5_io::write_sparse(p, &m).map_err(|e| format!("writing sparse: {e}"))?;
        outputs.push(IoRef { path: p.clone(), role: "sparse".into() });
    }
    if outputs.is_empty() {
        return Err("no output specified (set dense, dense_text, or sparse)".into());
    }
    let inputs = vec![IoRef { path: input, role: "matrix".into() }];
    Ok((inputs, outputs, format!("wrote {}×{} matrix", m.nrows(), m.ncols())))
}

fn run_build_ssn(params: Value) -> Result<RunOutput, String> {
    let cfg: BuildSsnConfig =
        serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    let input = cfg.input.clone().ok_or("missing input matrix")?;
    let xgmml = cfg.xgmml.clone().ok_or("missing output (--xgmml path)")?;
    if cfg.mst && cfg.mst_knn.is_some() {
        return Err("mst and mst_knn are mutually exclusive".into());
    }
    // GUI form doesn't expose subset/color-table files yet; pass none.
    let opts = resolve_build_ssn(&cfg, None, None);
    let m = read_matrix(&input).map_err(|e| format!("reading matrix: {e}"))?;
    let result = build_ssn(&m, &opts).map_err(|e| format!("build_ssn: {e}"))?;

    let name =
        Path::new(&xgmml).file_stem().and_then(|s| s.to_str()).unwrap_or("network").to_string();
    let mut buf: Vec<u8> = Vec::new();
    cytoscape::write_xgmml_to(&mut buf, &name, &result.table, &result.edges, result.node_fills.as_deref())
        .map_err(|e| format!("writing XGMML: {e}"))?;
    std::fs::write(&xgmml, &buf).map_err(|e| format!("writing {xgmml}: {e}"))?;
    let mut outputs = vec![IoRef { path: xgmml, role: "xgmml".into() }];

    if let Some(path) = &cfg.cluster_tsv {
        let labels = result.cluster_labels.as_ref().ok_or("no cluster labels (set cluster)")?;
        let mut out = String::new();
        if !cfg.no_cluster_header {
            out.push_str("contig\tcluster\n");
        }
        for (name, id) in result.table.names.iter().zip(labels) {
            out.push_str(name);
            out.push('\t');
            out.push_str(&id.to_string());
            out.push('\n');
        }
        std::fs::write(path, out).map_err(|e| format!("writing cluster tsv: {e}"))?;
        outputs.push(IoRef { path: path.clone(), role: "cluster_tsv".into() });
    }

    let inputs = vec![IoRef { path: input, role: "matrix".into() }];
    Ok((inputs, outputs, format!("wrote {} nodes, {} edges", result.table.names.len(), result.edges.len())))
}

fn run_build_ssn_viewer(params: Value) -> Result<RunOutput, String> {
    let cfg: BuildSsnViewerConfig =
        serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    let input = cfg.input.clone().ok_or("missing input matrix")?;
    let output = cfg.output.clone().ok_or("missing output (.ssnv path)")?;
    let name = Path::new(&output)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("SSN")
        .to_string();
    let opts = resolve_build_ssn_viewer(&cfg, name, None);
    let m = read_matrix(&input).map_err(|e| format!("reading matrix: {e}"))?;
    let bundle = build_ssn_viewer_bundle(&m, &opts).map_err(|e| format!("building bundle: {e}"))?;
    let compressed = gzip(&serialize_bundle(&bundle));
    std::fs::write(&output, &compressed).map_err(|e| format!("writing .ssnv: {e}"))?;

    let inputs = vec![IoRef { path: input, role: "matrix".into() }];
    let outputs = vec![IoRef { path: output, role: "ssnv".into() }];
    Ok((inputs, outputs, format!("wrote {}-node .ssnv bundle", m.nrows())))
}
