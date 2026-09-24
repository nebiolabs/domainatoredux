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

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use domainator_config::resolve::{resolve_build_ssn, resolve_build_ssn_viewer, resolve_transform_matrix};
use domainator_config::{
    schema, BuildSsnConfig, BuildSsnViewerConfig, CleanSequencesConfig, ColorGenbankConfig,
    ColorTableToLegendConfig, CompareContigsConfig, DeduplicateConfig, DomainSearchConfig, EnumConfig,
    DomainateConfig, ExtractDomainsConfig, ExtractPeptidesConfig, ExtractUnannotatedConfig,
    FilterDomainsConfig, GenbankToFastaConfig, HmmerBuildConfig, HmmerCompareConfig,
    HmmerReportConfig, HmmerSearchConfig, HmmerSelectConfig, SelectByCdsConfig, SelectByCoordConfig,
    SelectByContigConfig, SeqDistConfig, SortContigsConfig, TransformMatrixConfig, TrimContigsConfig,
};
use domainator_core::app_support::provenance::{new_job_id, now_ms, write_job, IoRef, JobRecord};
use domainator_core::io::fasta::{read_fasta, write_fasta};
use domainator_core::io::genbank::{read_genbank, write_genbank};
use domainator_core::io::{detect_filetype, FileType};
use domainator_core::matrix::{cytoscape, hdf5_io, read_matrix, text};
use domainator_core::seq::SeqRecord;
use domainator_core::tools::build_ssn::build_ssn;
use domainator_core::tools::build_ssn_viewer::{build_ssn_viewer_bundle, gzip, serialize_bundle};
use domainator_core::tools::compare_contigs::{compare_contigs, CompareContigsOptions};
use domainator_core::tools::domain_search::{domain_search, DomainSearchOptions};
use domainator_core::tools::domainate::{self, domainate, DomainateOptions};
use domainator_core::tools::enum_report::{enum_report, needs_taxonomy, OutputFormat};
use domainator_core::tools::filter_domains::{filter_domains, AnnotationType, FilterDomainsOptions};
use domainator_core::search::hmm_write::write_hmm_output;
use domainator_core::tools::hmmer_build::{hmmer_build, write_hmm_out, HmmerBuildOptions};
use domainator_core::tools::hmmer_select::{hmmer_select, parse_alphabet, Fields, SelectOptions};
use domainator_core::tools::{
    clean_sequences, color_genbank, color_table_to_legend, deduplicate_genbank, extract_domains,
    extract_peptides, extract_unannotated, hmmer_compare as hmmer_compare_m,
    hmmer_report as hmmer_report_m, hmmer_search as hmmer_search_m, select_by_coord, sort_contigs,
    trim_contigs,
};
use domainator_core::io::fasta::write_fasta_bp;
use domainator_core::tools::select_by_cds::{pad_records, select_by_cds, SelectByCdsOptions};
use domainator_core::tools::select_by_contig::{select_by_contig, SelectByContigOptions};
use domainator_core::tools::matrix_report::{
    matrix_report, render_text, DEFAULT_MAX_MERGE, DEFAULT_MERGE_IMPACT_METRIC,
};
use domainator_core::tools::neighborhood::{validate_range, Range};
use domainator_core::tools::select_common::parse_name_list;
use domainator_core::tools::seq_dist::{detect_kind, seq_dist, SeqDistOptions};
use domainator_core::tools::transform_matrix::transform_matrix;
use domainator_core::app_support::workspace::dot_dir;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter};

/// Live state of a tool run, tracked in the [`JobRegistry`] managed state and
/// broadcast to the UI via the `job-update` event.
#[derive(Serialize, Deserialize, Clone)]
pub struct JobInfo {
    pub job_id: String,
    pub tool: String,
    /// `running` | `completed` | `failed` | `canceled`.
    pub status: String,
    pub message: String,
    pub started_at_ms: String, // u128 as string (JSON-safe)
    pub outputs: Vec<String>,
    /// The params the tool was run with — lets the UI "clone" a job into a fresh
    /// pre-filled dialog.
    pub params: Value,
}

/// Managed state: all jobs seen this session + a cancel flag per running job.
///
/// The flags are `Arc<AtomicBool>` rather than a set of ids so the blocking worker
/// itself can read them — checking only *after* `run_one` returned (as this used
/// to) is too late to stop anything. Entries are removed when a job reaches a
/// terminal state, so the map tracks running jobs rather than growing all session.
#[derive(Default)]
pub struct JobRegistry {
    jobs: Mutex<HashMap<String, JobInfo>>,
    canceled: Mutex<HashMap<String, std::sync::Arc<AtomicBool>>>,
    /// Workspace root for persisting job history to `.domainator/jobs.json`
    /// (set when a workspace is opened via `load_jobs`).
    root: Mutex<Option<String>>,
}

/// Lock that survives a poisoned mutex — see `cmd_files::lock_poison_tolerant`.
fn reg_lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn ms_str(ms: u128) -> String {
    ms.to_string()
}

/// One form field, built from a `tools/<id>.yml` field (see [`crate::tool_specs`]).
///
/// `role` tells `ToolDialog.svelte` how to render it, so it must be one of the
/// six the dialog knows — a role it does not handle renders as nothing at all:
///
/// * `input_file` / `input_files` — a dropdown of registered databases and
///   current-dir files of any of `file_types`, grouped in `sources` order, starting on `select`
/// * `output_file` — a filename (seeded from `output_name`); the app prepends the
///   current directory
/// * `param` — rendered from the tool's JSON schema by type
/// * `columns` — a report's column builder: an ordered list of column objects,
///   whose types and parameters come from the schema's `columns.items.oneOf`
///   (adjusted by the spec's `column_types`); the order is the output column order
///
/// `ROLES` below is the list, and a test checks every field against it.
#[derive(Serialize)]
pub struct ToolField {
    pub name: String,
    pub label: String,
    /// The spec's `help`, which replaces the schema's `description` as the hint.
    pub help: Option<String>,
    pub role: String,
    /// Inputs: the file types the dropdown offers (the spec's `file_type`). Empty
    /// for every other role.
    pub file_types: Vec<String>,
    pub required: bool,
    /// The value the dialog seeds: the spec's `gui_default`, else its `cli_default`.
    pub default: Option<Value>,
    /// The CLI's default, so the dialog can say so when `default` differs.
    pub cli_default: Option<Value>,
    /// Inputs: `databases_first` | `workspace_first`.
    pub sources: Option<String>,
    /// Inputs: `active_file` | `active_file_or_none` | `first` | `none` | `database:<managed id>`.
    pub select: Option<String>,
    /// Outputs: the seeded file name, with `{stem}` for the primary input's stem.
    pub output_name: Option<String>,
    /// `columns`: the spec's per-type overrides for the column builder.
    pub column_types: std::collections::BTreeMap<String, crate::tool_spec_types::ColumnTypeSpec>,
}

#[derive(Serialize)]
pub struct ToolMeta {
    pub id: String,
    pub label: String,
    pub description: String,
    /// The fields the dialog shows, in order (a spec's `show: false` fields are left out).
    pub fields: Vec<ToolField>,
    /// Which menu group this tool belongs to, one of [`TOOL_CATEGORIES`].
    pub category: String,
}

/// Every `role` [`ToolDialog.svelte`] can render. See [`ToolField`].
pub const ROLES: [&str; 5] = ["input_file", "input_files", "output_file", "param", "columns"];

/// The Tools menu's groups, in the order they appear. Mirrors the grouping in the
/// repository's CLAUDE.md so the menu and the docs agree.
pub const TOOL_CATEGORIES: [&str; 7] = [
    "Annotation & search",
    "Selection & extraction",
    "Matrices & networks",
    "Reports",
    "HMM profiles",
    "Color",
    "Conversion",
];

/// Tools currently runnable from the GUI, each with its form field spec.
///
/// `async` so it runs on the async runtime, not Tauri's main thread — a
/// synchronous command can't be serviced while the main thread is busy (a tool
/// running, a big viewer painting), which would leave a tool dialog at "Loading…".
#[tauri::command]
pub async fn list_tools() -> Vec<ToolMeta> {
    let mut tools: Vec<ToolMeta> = crate::tool_specs::specs().iter().map(tool_meta).collect();
    // In menu order, so the toolbar's Tools button can group by first appearance
    // and come out shaped like the native menu. Stable: file order within a
    // category is kept. A category outside TOOL_CATEGORIES is a test failure; in a
    // release build the tool still shows, last, rather than vanishing.
    tools.sort_by_key(|t| {
        TOOL_CATEGORIES.iter().position(|c| *c == t.category).unwrap_or(usize::MAX)
    });
    tools
}

fn tool_meta(spec: &crate::tool_specs::ToolSpec) -> ToolMeta {
    ToolMeta {
        id: spec.id.clone(),
        label: spec.label.clone(),
        description: spec.description.clone(),
        category: spec.category.clone(),
        fields: spec
            .fields
            .iter()
            .filter(|f| f.show)
            .map(|f| {
                debug_assert!(
                    ROLES.contains(&f.role.as_str()),
                    "{}.{}: ToolDialog cannot render role {:?}",
                    spec.id,
                    f.name,
                    f.role
                );
                ToolField {
                    name: f.name.clone(),
                    label: f.label.clone(),
                    help: f.help.clone(),
                    role: f.role.clone(),
                    file_types: f.file_type.clone(),
                    required: f.required,
                    default: f.effective_default().cloned(),
                    cli_default: f.cli_default.clone(),
                    sources: f.sources.map(|s| s.as_str().to_string()),
                    select: f.select.as_ref().map(|s| s.to_string()),
                    output_name: f.output_name.clone(),
                    column_types: f.column_types.clone(),
                }
            })
            .collect(),
    }
}

/// The draft-07 JSON Schema for a tool's parameters (drives the form).
///
/// `async` so a tool dialog's schema fetch never waits on the main thread (see
/// [`list_tools`]) — otherwise the dialog can hang at "Loading…".
#[tauri::command]
pub async fn tool_schema(tool: String) -> Result<String, String> {
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

    // Apply the app-level default thread count, but only where the tool's schema
    // actually has a `cpu` property -- every config struct is
    // `deny_unknown_fields`, so a blind injection would make the run be rejected.
    // Guarded across all tools by `injecting_the_default_cpu_never_breaks_a_tool`.
    let mut params = params;
    if let Ok(path) = crate::cmd_prefs::prefs_path(&app) {
        let default_cpu = domainator_core::app_support::prefs::load_prefs(&path).default_cpu;
        if let Ok(schema_text) = domainator_config::schema(&tool) {
            if let Ok(schema) = serde_json::from_str::<Value>(&schema_text) {
                domainator_core::app_support::tool_params::apply_default_cpu(
                    &mut params,
                    &schema,
                    default_cpu,
                );
            }
        }
    }

    let running = JobInfo {
        job_id: job_id.clone(),
        tool: tool.clone(),
        status: "running".into(),
        message: String::new(),
        started_at_ms: ms_str(started),
        outputs: Vec::new(),
        params: params.clone(),
    };
    set_job(&registry, &app, running);

    // Off-thread work. Cancellation is genuinely best-effort: the core tools have
    // no interruption points, so once a tool has started the most `run_one` can do
    // is refuse to write provenance for a run the user abandoned (and skip the work
    // entirely if the cancel arrived before dispatch). Whatever output file the tool
    // had already written stays on disk — it may have overwritten something, so
    // deleting it behind the user's back would be worse than leaving it. The
    // terminal message below says which of the two happened.
    let flag = std::sync::Arc::new(AtomicBool::new(false));
    reg_lock(&registry.canceled).insert(job_id.clone(), flag.clone());

    let jid = job_id.clone();
    let tool_c = tool.clone();
    let root_c = workspace_root.clone();
    let params_c = params.clone();
    let flag_c = flag.clone();
    // A panicking tool surfaces here as a `JoinError`. It must still reach a
    // terminal state below: returning early (as this used to, via `?`) left the
    // row at `running` -- or `canceling`, if the user had pressed Cancel --
    // for the rest of the session, and persisted that way.
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        run_one(&tool_c, params_c, &root_c, started, &jid, &flag_c)
    })
    .await
    .unwrap_or_else(|e| Err(format!("the tool crashed: {e}")));

    let canceled = flag.load(Ordering::Relaxed);
    reg_lock(&registry.canceled).remove(&job_id);
    let mut info = JobInfo {
        job_id: job_id.clone(),
        tool: tool.clone(),
        status: String::new(),
        message: String::new(),
        started_at_ms: ms_str(started),
        outputs: Vec::new(),
        params: params.clone(),
    };
    match outcome {
        _ if canceled => {
            info.status = "canceled".into();
            info.message = match &outcome {
                Ok(r) if !r.outputs.is_empty() => {
                    info.outputs = r.outputs.clone();
                    "canceled too late to stop the tool — its output was kept, \
                     but the run was not recorded"
                        .into()
                }
                _ => "canceled".to_string(),
            };
            set_job(&registry, &app, info.clone());
            Err("job canceled".into())
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

/// Persisted job-history cap (newest kept), so `.domainator/jobs.json` can't grow
/// without bound.
const MAX_PERSISTED_JOBS: usize = 500;

fn jobs_file(root: &str) -> std::path::PathBuf {
    dot_dir(Path::new(root)).join("jobs.json")
}

/// Write the current job history to `.domainator/jobs.json` (newest first, capped).
/// Best-effort: no-op until a workspace root is known; failures are ignored.
fn persist_jobs(registry: &JobRegistry) {
    let Some(root) = reg_lock(&registry.root).clone() else { return };
    let mut list: Vec<JobInfo> = reg_lock(&registry.jobs).values().cloned().collect();
    list.sort_by_key(|b| std::cmp::Reverse(job_ms(b)));
    list.truncate(MAX_PERSISTED_JOBS);
    let path = jobs_file(&root);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(bytes) = serde_json::to_vec_pretty(&list) {
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, &bytes).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

fn job_ms(j: &JobInfo) -> u128 {
    j.started_at_ms.parse().unwrap_or(0)
}

/// Record a job's state, broadcast it to the UI, and persist the history.
fn set_job(registry: &JobRegistry, app: &AppHandle, info: JobInfo) {
    reg_lock(&registry.jobs).insert(info.job_id.clone(), info.clone());
    let _ = app.emit("job-update", &info);
    persist_jobs(registry);
}

/// Status of one job (for polling).
#[tauri::command]
pub fn job_status(registry: tauri::State<'_, JobRegistry>, job_id: String) -> Option<JobInfo> {
    reg_lock(&registry.jobs).get(&job_id).cloned()
}

/// All jobs seen this session (newest state).
#[tauri::command]
pub fn list_jobs(registry: tauri::State<'_, JobRegistry>) -> Vec<JobInfo> {
    reg_lock(&registry.jobs).values().cloned().collect()
}

/// Load persisted job history for a workspace (called on open). Sets the registry's
/// root (so future updates persist there), replaces the in-memory jobs with the
/// file's, and marks any left-over `running` jobs as interrupted (the process that
/// ran them is gone). Returns the history.
/// Reads and rewrites `.domainator/jobs.json`, so `(async)`: a plain `fn`
/// command would do that on Tauri's main thread.
#[tauri::command(async)]
pub fn load_jobs(registry: tauri::State<'_, JobRegistry>, workspace_root: String) -> Vec<JobInfo> {
    *reg_lock(&registry.root) = Some(workspace_root.clone());
    let loaded: Vec<JobInfo> = std::fs::read(jobs_file(&workspace_root))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    // Jobs this process is still running (reopening the workspace mid-run, say):
    // their in-memory row is the truth, and the file may be behind it. Everything
    // else non-terminal in the file belongs to a process that is gone.
    let live: HashSet<String> = reg_lock(&registry.canceled).keys().cloned().collect();
    let list = {
        let mut map = reg_lock(&registry.jobs);
        let previous = std::mem::take(&mut *map);
        for mut j in loaded {
            match previous.get(&j.job_id) {
                Some(current) if live.contains(&j.job_id) => j = current.clone(),
                _ => settle_orphaned_job(&mut j),
            }
            map.insert(j.job_id.clone(), j);
        }
        map.values().cloned().collect::<Vec<_>>()
    };
    persist_jobs(&registry); // save the corrected (no-longer-running) statuses
    list
}

/// Give a job that no live process owns a terminal status.
///
/// Both non-terminal states need it. `canceling` used to be missed: it reaches
/// `jobs.json` whenever anything persists while a canceled tool is still running,
/// and a job that never finished stayed "canceling…" across every relaunch.
fn settle_orphaned_job(j: &mut JobInfo) {
    match j.status.as_str() {
        "running" => {
            j.status = "failed".into();
            j.message = "interrupted (app closed)".into();
        }
        "canceling" => {
            j.status = "canceled".into();
            j.message = "canceled (the app closed before the tool stopped; any \
                         output it had written was kept)"
                .into();
        }
        _ => {}
    }
}

/// Delete one job from the history.
/// `(async)`: rewrites the whole persisted history (see [`load_jobs`]).
#[tauri::command(async)]
pub fn delete_job(registry: tauri::State<'_, JobRegistry>, job_id: String) {
    reg_lock(&registry.jobs).remove(&job_id);
    persist_jobs(&registry);
}

/// Clear the entire job history.
/// `(async)`: rewrites the whole persisted history (see [`load_jobs`]).
#[tauri::command(async)]
pub fn clear_jobs(registry: tauri::State<'_, JobRegistry>) {
    reg_lock(&registry.jobs).clear();
    persist_jobs(&registry);
}

/// Best-effort cancel. Sets the job's flag (which `run_one` checks at its own
/// boundaries) and flips the visible status to `canceling` right away, so the
/// click has feedback: the core tools can't be interrupted mid-run, and without
/// this the row sat at "running" until the tool finished on its own.
///
/// `(async)`: it now persists the history (see [`load_jobs`]).
#[tauri::command(async)]
pub fn cancel_job(app: AppHandle, registry: tauri::State<'_, JobRegistry>, job_id: String) {
    if let Some(flag) = reg_lock(&registry.canceled).get(&job_id) {
        flag.store(true, Ordering::Relaxed);
    }
    let pending = {
        let mut jobs = reg_lock(&registry.jobs);
        match jobs.get_mut(&job_id) {
            Some(j) if j.status == "running" => {
                j.status = "canceling".into();
                Some(j.clone())
            }
            _ => None,
        }
    };
    if let Some(info) = pending {
        let _ = app.emit("job-update", &info);
        persist_jobs(&registry);
    }
}

/// The actual tool dispatch + provenance write (runs on the blocking pool).
///
/// `canceled` is checked before dispatch (so a job canceled while it was still
/// queued behind others does no work at all) and again before the provenance write
/// (so an abandoned run isn't recorded in the DAG). There is no check *inside* a
/// tool: none of the core tools expose an interruption point.
fn run_one(
    tool: &str,
    params: Value,
    root: &str,
    started: u128,
    job_id: &str,
    canceled: &AtomicBool,
) -> Result<RunResult, String> {
    if canceled.load(Ordering::Relaxed) {
        return Err("job canceled".into());
    }
    let (inputs, outputs, message) = match tool {
        "domain-search" => run_domain_search(params.clone())?,
        "domainate" => run_domainate(params.clone())?,
        "seq-dist" => run_seq_dist(params.clone())?,
        "compare-contigs" => run_compare_contigs(params.clone())?,
        "select-by-cds" => run_select_by_cds(params.clone())?,
        "select-by-contig" => run_select_by_contig(params.clone())?,
        "filter-domains" => run_filter_domains(params.clone())?,
        "hmmer-select" => run_hmmer_select(params.clone())?,
        "hmmer-build" => run_hmmer_build(params.clone())?,
        "hmmer-compare" => run_hmmer_compare(params.clone())?,
        "hmmer-search" => run_hmmer_search(params.clone())?,
        "hmmer-report" => run_hmmer_report(params.clone())?,
        "genbank-to-fasta" => run_genbank_to_fasta(params.clone())?,
        "clean-sequences" => run_clean_sequences(params.clone())?,
        "sort-contigs" => run_sort_contigs(params.clone())?,
        "select-by-coord" => run_select_by_coord(params.clone())?,
        "trim-contigs" => run_trim_contigs(params.clone())?,
        "extract-domains" => run_extract_domains(params.clone())?,
        "extract-peptides" => run_extract_peptides(params.clone())?,
        "extract-unannotated" => run_extract_unannotated(params.clone())?,
        "color-genbank" => run_color_genbank(params.clone())?,
        "color-table-to-legend" => run_color_table_to_legend(params.clone())?,
        "deduplicate" => run_deduplicate(params.clone())?,
        "enum-report" => run_export_enum_report(params.clone())?,
        "matrix-report" => run_export_matrix_report(params.clone())?,
        "transform-matrix" => run_transform_matrix(params.clone())?,
        "build-ssn" => run_build_ssn(params.clone())?,
        "build-ssn-viewer" => run_build_ssn_viewer(params.clone())?,
        other => return Err(format!("tool not runnable from the GUI yet: {other}")),
    };
    if canceled.load(Ordering::Relaxed) {
        // Canceled while the tool ran: report the outputs it produced, but write
        // no provenance — the run is not part of the workspace's history.
        return Ok(RunResult {
            job_id: job_id.to_string(),
            outputs: outputs.into_iter().map(|o| o.path).collect(),
            message,
        });
    }
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
        upstream_version: domainator_core::version::UPSTREAM_VERSION.into(),
    };
    write_job(Path::new(root), &record, &format!("{tool} output"))
        .map_err(|e| format!("writing provenance: {e}"))?;
    Ok(RunResult { job_id: job_id.to_string(), outputs: outputs.into_iter().map(|o| o.path).collect(), message })
}

type RunOutput = (Vec<IoRef>, Vec<IoRef>, String);

/// Load a GenBank or FASTA sequence file (FASTA gets the given molecule type).
fn load_sequence(path: &str, fasta_type: &str) -> Result<Vec<SeqRecord>, String> {
    match detect_filetype(Path::new(path)) {
        Some(FileType::Genbank) => read_genbank(path).map_err(|e| format!("reading GenBank: {e}")),
        Some(FileType::Fasta) => read_fasta(path, Some(fasta_type)).map_err(|e| format!("reading FASTA: {e}")),
        None => Err(format!("unsupported sequence file: {path}")),
    }
}

/// Compile a taxid allow-set from include/exclude lists or a taxonomy expression,
/// using an NCBI taxdump at `path` (mirrors the CLI `compile_allowed_taxids`).
/// Returns `None` when no filter is requested. Requires `path` when a filter is set.
fn compile_allowed_taxids(
    include: &[i64],
    exclude: &[i64],
    expr: Option<&str>,
    path: Option<&str>,
) -> Result<Option<HashSet<i64>>, String> {
    let has_expr = expr.is_some_and(|e| !e.trim().is_empty());
    if include.is_empty() && exclude.is_empty() && !has_expr {
        return Ok(None);
    }
    if has_expr && (!include.is_empty() || !exclude.is_empty()) {
        return Err("taxonomy_expr is mutually exclusive with include_taxids/exclude_taxids".into());
    }
    let path = path
        .filter(|p| !p.trim().is_empty())
        .ok_or("taxonomy filters require ncbi_taxonomy_path (a taxdump directory)")?;
    let tax = crate::taxonomy_cache::load(path)?;
    let allowed = if has_expr {
        tax.compile_taxonomy_expression(expr.unwrap())
    } else {
        let inc: HashSet<i64> = include.iter().copied().collect();
        let exc: HashSet<i64> = exclude.iter().copied().collect();
        tax.compile_taxonomy_filter(
            if inc.is_empty() { None } else { Some(&inc) },
            if exc.is_empty() { None } else { Some(&exc) },
        )
    };
    Ok(allowed)
}

fn ref_inputs(seq: &str, references: &[String]) -> Vec<IoRef> {
    let mut v = vec![IoRef { path: seq.to_string(), role: "sequence".into() }];
    for r in references {
        v.push(IoRef { path: r.clone(), role: "reference".into() });
    }
    v
}

fn run_domain_search(params: Value) -> Result<RunOutput, String> {
    let cfg: DomainSearchConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    let input = cfg.input.clone().ok_or("missing input sequences")?;
    let output = cfg.output.clone().ok_or("missing output (GenBank)")?;
    if cfg.references.is_empty() {
        return Err("select at least one HMM reference".into());
    }
    let dbs = domainate::read_hmm_references(&cfg.references).map_err(|e| format!("reading references: {e}"))?;
    // Neighborhood extraction (--cds_range/--kb_range/--whole_contig), mirroring the CLI.
    let range = validate_range(
        cfg.whole_contig,
        cfg.kb_range,
        cfg.kb_up,
        cfg.kb_down,
        cfg.cds_range,
        cfg.cds_up,
        cfg.cds_down,
    )?;
    if cfg.translate && !matches!(range, Range::None) {
        return Err("translate is incompatible with neighborhood extraction (cds_range/kb_range/whole_contig)".into());
    }
    let strand = match cfg.strand.as_deref() {
        Some("f") => Some(1),
        Some("r") => Some(-1),
        Some(other) => return Err(format!("strand must be 'f' or 'r', got {other:?}")),
        None => None,
    };
    let allowed_taxids = compile_allowed_taxids(
        &cfg.include_taxids,
        &cfg.exclude_taxids,
        cfg.taxonomy_expr.as_deref(),
        cfg.ncbi_taxonomy_path.as_deref(),
    )?;
    let decoys = parse_name_list(&cfg.decoys, cfg.decoys_file.as_deref())
        .map_err(|e| format!("reading decoys: {e}"))?;
    let opts = DomainSearchOptions {
        z: cfg.z.unwrap_or(1000.0),
        evalue: cfg.evalue.unwrap_or(0.001),
        min_evalue: cfg.min_evalue.unwrap_or(0.0),
        max_overlap: cfg.max_overlap.unwrap_or(1.0),
        add_annotations: cfg.add_annotations,
        max_hits: cfg.max_hits,
        max_hits_per_contig: cfg.max_hits_per_contig,
        pad: cfg.pad,
        batch_size: cfg.batch_size.unwrap_or(10000),
        fasta_type: cfg.fasta_type.clone().unwrap_or_else(|| "protein".into()),
        cpu: cfg.cpu.unwrap_or(0),
        range,
        normalize_direction: !cfg.keep_direction,
        strand,
        max_region_overlap: cfg.max_region_overlap.unwrap_or(1.0),
        translate: cfg.translate,
        max_mode: cfg.max_mode,
        allowed_taxids,
        decoys,
    };
    let hits = domain_search(std::slice::from_ref(&input), &dbs, &opts).map_err(|e| format!("domain_search: {e}"))?;
    write_genbank(&output, &hits).map_err(|e| format!("writing GenBank: {e}"))?;
    Ok((
        ref_inputs(&input, &cfg.references),
        vec![IoRef { path: output, role: "genbank".into() }],
        format!("wrote {} hit contigs", hits.len()),
    ))
}

fn run_domainate(params: Value) -> Result<RunOutput, String> {
    let cfg: DomainateConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    let input = cfg.input.clone().ok_or("missing input sequences")?;
    let output = cfg.output.clone().ok_or("missing output (GenBank)")?;
    if cfg.references.is_empty() {
        return Err("select at least one HMM reference".into());
    }
    let fasta_type = cfg.fasta_type.clone().unwrap_or_else(|| "protein".into());
    let records = load_sequence(&input, &fasta_type)?;
    let dbs = domainate::read_hmm_references(&cfg.references).map_err(|e| format!("reading references: {e}"))?;
    if dbs.has_nucleotide() && records.iter().any(|r| r.is_protein()) {
        return Err("cannot search protein inputs with a nucleotide reference".into());
    }
    let max_domains = match cfg.max_domains {
        Some(0) | None => usize::MAX,
        Some(n) => n,
    };
    let allowed_taxids = compile_allowed_taxids(
        &cfg.include_taxids,
        &cfg.exclude_taxids,
        cfg.taxonomy_expr.as_deref(),
        cfg.ncbi_taxonomy_path.as_deref(),
    )?;
    let opts = DomainateOptions {
        z: cfg.z.unwrap_or(1000.0),
        evalue: cfg.evalue.unwrap_or(0.001),
        min_evalue: cfg.min_evalue.unwrap_or(0.0),
        max_hits: max_domains,
        max_overlap: cfg.max_overlap.unwrap_or(1.0),
        no_annotations: cfg.no_annotations,
        hits_only: cfg.hits_only,
        overlap_by_db: cfg.overlap_by_db,
        max_mode: cfg.max_mode,
        allowed_taxids,
        cpu: cfg.cpu.unwrap_or(0),
        ..Default::default()
    };
    let annotated = domainate(records, &dbs, &opts);
    write_genbank(&output, &annotated).map_err(|e| format!("writing GenBank: {e}"))?;
    Ok((
        ref_inputs(&input, &cfg.references),
        vec![IoRef { path: output, role: "genbank".into() }],
        format!("annotated {} records", annotated.len()),
    ))
}

fn run_seq_dist(params: Value) -> Result<RunOutput, String> {
    let cfg: SeqDistConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    let input = cfg.input.clone().ok_or("missing input sequences")?;
    let output = cfg.sparse.clone().ok_or("missing output (matrix .hdf5)")?;
    let input_kind = detect_kind(Path::new(&input)).ok_or_else(|| format!("could not detect input kind: {input}"))?;
    let ref_kind = match cfg.reference.as_ref() {
        Some(p) => Some(detect_kind(Path::new(p)).ok_or_else(|| format!("could not detect reference kind: {p}"))?),
        None => None,
    };
    let opts = SeqDistOptions {
        // The CLI default; the dialog seeds the GUI's own (`tools/seq-dist.yml`).
        algorithm: cfg.algorithm.clone().unwrap_or_else(|| "diamond_us".into()),
        mode: cfg.mode.clone().unwrap_or_else(|| "score".into()),
        lb: cfg.lb.unwrap_or(0.0),
        k: cfg.k,
        mst_knn: cfg.mst_knn,
        knn: cfg.knn,
        params: domainator_core::tools::seq_dist::parse_diamond_params(
            cfg.params.as_deref().unwrap_or(""),
        )?,
        cpu: cfg.cpu.unwrap_or(0),
    };
    let matrix = seq_dist(&input, input_kind, cfg.reference.as_deref(), ref_kind, &opts)
        .map_err(|e| format!("seq_dist: {e}"))?;
    hdf5_io::write_sparse(&output, &matrix).map_err(|e| format!("writing matrix: {e}"))?;
    let mut inputs = vec![IoRef { path: input, role: "sequence".into() }];
    if let Some(r) = cfg.reference {
        inputs.push(IoRef { path: r, role: "reference".into() });
    }
    Ok((inputs, vec![IoRef { path: output, role: "matrix".into() }], format!("wrote {}×{} matrix", matrix.nrows(), matrix.ncols())))
}

fn run_compare_contigs(params: Value) -> Result<RunOutput, String> {
    let cfg: CompareContigsConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    if cfg.input.is_empty() {
        return Err("select at least one GenBank file".into());
    }
    let output = cfg.sparse.clone().ok_or("missing output (matrix .hdf5)")?;
    let ji = cfg.ji.unwrap_or(0.5);
    let ai = cfg.ai.unwrap_or(0.5);
    if ji == 0.0 && ai == 0.0 {
        return Err("set a non-zero Jaccard or Adjacency weight".into());
    }
    let opts = CompareContigsOptions {
        ji_weight: ji,
        ai_weight: ai,
        databases: if cfg.databases.is_empty() { None } else { Some(cfg.databases.iter().cloned().collect()) },
        k: cfg.k,
        lb: cfg.lb.unwrap_or(0.0),
        mst_knn: cfg.mst_knn,
        knn: cfg.knn,
        cpu: cfg.cpu.unwrap_or(0),
    };
    let matrix = compare_contigs(&cfg.input, &opts).map_err(|e| format!("compare_contigs: {e}"))?;
    hdf5_io::write_sparse(&output, &matrix).map_err(|e| format!("writing matrix: {e}"))?;
    let inputs = cfg.input.iter().map(|p| IoRef { path: p.clone(), role: "genbank".into() }).collect();
    Ok((inputs, vec![IoRef { path: output, role: "matrix".into() }], format!("wrote {}×{} matrix", matrix.nrows(), matrix.ncols())))
}

/// Load and concatenate all records from multiple sequence files (FASTA → protein).
fn load_inputs(paths: &[String]) -> Result<Vec<SeqRecord>, String> {
    let mut all = Vec::new();
    for p in paths {
        all.extend(load_sequence(p, "protein")?);
    }
    Ok(all)
}

/// Retain only records whose LOCUS name is in the contig name-list (if any).
fn filter_contigs(
    records: &mut Vec<SeqRecord>,
    contigs: &[String],
    contigs_file: Option<&str>,
) -> Result<(), String> {
    if let Some(set) =
        parse_name_list(contigs, contigs_file).map_err(|e| format!("reading contigs: {e}"))?
    {
        records.retain(|r| set.contains(&r.name));
    }
    Ok(())
}

/// One `f`/`r` strand selector into the internal ±1 (mirrors the CLI).
fn parse_strand(s: Option<&str>) -> Result<Option<i32>, String> {
    match s {
        Some("f") => Ok(Some(1)),
        Some("r") => Ok(Some(-1)),
        None | Some("") => Ok(None),
        Some(other) => Err(format!("strand must be 'f' or 'r', got {other:?}")),
    }
}

fn run_select_by_cds(params: Value) -> Result<RunOutput, String> {
    let cfg: SelectByCdsConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    if cfg.input.is_empty() {
        return Err("select at least one input sequence file".into());
    }
    let output = cfg.output.clone().ok_or("missing output (GenBank)")?;
    let mut records = load_inputs(&cfg.input)?;
    let target_contigs =
        parse_name_list(&cfg.contigs, cfg.contigs_file.as_deref()).map_err(|e| e.to_string())?;
    if let Some(set) = &target_contigs {
        records.retain(|r| set.contains(&r.name));
    }
    let target_cdss = parse_name_list(&cfg.cds, cfg.cds_file.as_deref()).map_err(|e| e.to_string())?;
    let target_domains =
        parse_name_list(&cfg.domains, cfg.domains_file.as_deref()).map_err(|e| e.to_string())?;
    let range = validate_range(
        cfg.whole_contig, cfg.kb_range, cfg.kb_up, cfg.kb_down, cfg.cds_range, cfg.cds_up, cfg.cds_down,
    )?;
    let strand = parse_strand(cfg.strand.as_deref())?;
    let opts = SelectByCdsOptions {
        target_cdss,
        target_domains,
        domain_expr: cfg.domain_expr,
        range,
        normalize_direction: !cfg.keep_direction,
        evalue: cfg.evalue.unwrap_or(100_000_000.0),
        invert: cfg.invert,
        search_hits: cfg.search_hits,
        max_region_overlap: cfg.max_region_overlap.unwrap_or(1.0),
        strand,
        databases: if cfg.databases.is_empty() { None } else { Some(cfg.databases.iter().cloned().collect()) },
        unannotated: cfg.unannotated,
        include_nucleic_acids: cfg.include_nucleic_acids,
    };
    let mut regions = select_by_cds(&records, &opts);
    if !cfg.skip_deduplicate {
        let mut seen = HashSet::new();
        regions.retain(|r| seen.insert(r.record.name.clone()));
    }
    if cfg.pad {
        pad_records(&mut regions);
    }
    let out_records: Vec<SeqRecord> = regions.into_iter().map(|r| r.record).collect();
    write_genbank(&output, &out_records).map_err(|e| format!("writing GenBank: {e}"))?;
    let inputs = cfg.input.iter().map(|p| IoRef { path: p.clone(), role: "sequence".into() }).collect();
    Ok((
        inputs,
        vec![IoRef { path: output, role: "genbank".into() }],
        format!("selected {} regions", out_records.len()),
    ))
}

fn run_select_by_contig(params: Value) -> Result<RunOutput, String> {
    let cfg: SelectByContigConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    if cfg.input.is_empty() {
        return Err("select at least one input sequence file".into());
    }
    let output = cfg.output.clone().ok_or("missing output")?;
    let records = load_inputs(&cfg.input)?;
    let target_contigs =
        parse_name_list(&cfg.contigs, cfg.contigs_file.as_deref()).map_err(|e| e.to_string())?;
    let target_domains =
        parse_name_list(&cfg.domains, cfg.domains_file.as_deref()).map_err(|e| e.to_string())?;
    let needs_tax = !cfg.include_taxids.is_empty()
        || !cfg.exclude_taxids.is_empty()
        || cfg.taxonomy_expr.as_deref().is_some_and(|e| !e.trim().is_empty());
    let tax = if needs_tax {
        let path = cfg
            .ncbi_taxonomy_path
            .as_deref()
            .filter(|p| !p.trim().is_empty())
            .ok_or("taxonomy filters require ncbi_taxonomy_path (a taxdump directory)")?;
        Some(crate::taxonomy_cache::load(path)?)
    } else {
        None
    };
    let opts = SelectByContigOptions {
        target_domains,
        domain_expr: cfg.domain_expr,
        domain_evalue: cfg.evalue.unwrap_or(100_000_000.0),
        first: cfg.first,
        length_lb: cfg.length_lb,
        length_ub: cfg.length_ub,
        cds_count_lb: cfg.cds_count_lb,
        cds_count_ub: cfg.cds_count_ub,
        definition_regex: cfg.definition_regex,
        sequence_regex: cfg.sequence_regex,
        target_contigs,
        contigs_regex: cfg.contigs_regex,
        invert: cfg.invert,
        include_taxids: if cfg.include_taxids.is_empty() { None } else { Some(cfg.include_taxids.iter().copied().collect()) },
        exclude_taxids: if cfg.exclude_taxids.is_empty() { None } else { Some(cfg.exclude_taxids.iter().copied().collect()) },
        taxonomy_expr: cfg.taxonomy_expr,
        databases: if cfg.databases.is_empty() { None } else { Some(cfg.databases.iter().cloned().collect()) },
        unannotated: cfg.unannotated,
        search_score_lb: cfg.search_score_lb,
        search_score_ub: cfg.search_score_ub,
        domain_type: cfg.domain_type.unwrap_or_else(|| "domain".into()),
    };
    let selected = select_by_contig(records, tax.as_deref(), &opts).map_err(|e| e.to_string())?;
    let role = if cfg.fasta_out { "fasta" } else { "genbank" };
    if cfg.fasta_out {
        write_fasta(&output, &selected).map_err(|e| format!("writing FASTA: {e}"))?;
    } else {
        write_genbank(&output, &selected).map_err(|e| format!("writing GenBank: {e}"))?;
    }
    let inputs = cfg.input.iter().map(|p| IoRef { path: p.clone(), role: "sequence".into() }).collect();
    Ok((
        inputs,
        vec![IoRef { path: output, role: role.into() }],
        format!("selected {} contigs", selected.len()),
    ))
}

fn run_filter_domains(params: Value) -> Result<RunOutput, String> {
    let cfg: FilterDomainsConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    if cfg.input.is_empty() {
        return Err("select at least one input sequence file".into());
    }
    let output = cfg.output.clone().ok_or("missing output (GenBank)")?;
    let mut records = load_inputs(&cfg.input)?;
    filter_contigs(&mut records, &cfg.contigs, cfg.contigs_file.as_deref())?;
    let selected_domains =
        parse_name_list(&cfg.domains, cfg.domains_file.as_deref()).map_err(|e| e.to_string())?;
    let annotation_type =
        AnnotationType::parse(cfg.annotation_type.as_deref().unwrap_or("all")).map_err(|e| e.to_string())?;
    let opts = FilterDomainsOptions {
        evalue: cfg.evalue.unwrap_or(100_000_000.0),
        max_overlap: cfg.max_overlap.unwrap_or(1.0),
        databases_keep: if cfg.databases_keep.is_empty() { None } else { Some(cfg.databases_keep.iter().cloned().collect()) },
        databases_filter: if cfg.databases_filter.is_empty() { None } else { Some(cfg.databases_filter.iter().cloned().collect()) },
        selected_domains,
        annotation_type,
    };
    let out = filter_domains(records, &opts);
    write_genbank(&output, &out).map_err(|e| format!("writing GenBank: {e}"))?;
    let inputs = cfg.input.iter().map(|p| IoRef { path: p.clone(), role: "sequence".into() }).collect();
    Ok((
        inputs,
        vec![IoRef { path: output, role: "genbank".into() }],
        format!("wrote {} records", out.len()),
    ))
}

fn run_hmmer_select(params: Value) -> Result<RunOutput, String> {
    let cfg: HmmerSelectConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    if cfg.input.is_empty() {
        return Err("select at least one input .hmm file".into());
    }
    let output = cfg.output.clone().ok_or("missing output (.hmm)")?;
    let fields = Fields::from_field(cfg.field.as_deref().unwrap_or("all")).map_err(|e| e.to_string())?;
    let opts = SelectOptions {
        regex: cfg.regex,
        exact: cfg.exact,
        contains: cfg.contains,
        fields,
        case_sensitive: cfg.case_sensitive,
        alphabet: parse_alphabet(cfg.alphabet.as_deref())?,
    };
    let summary = write_hmm_output(&output, |f| hmmer_select(&cfg.input, &opts, f))?;
    let inputs = cfg.input.iter().map(|p| IoRef { path: p.clone(), role: "hmm".into() }).collect();
    let skipped = if summary.skipped_files > 0 {
        format!(" ({} file(s) skipped by --alphabet)", summary.skipped_files)
    } else {
        String::new()
    };
    Ok((
        inputs,
        vec![IoRef { path: output, role: "hmm".into() }],
        format!("selected {} profiles{skipped}", summary.found),
    ))
}

fn run_hmmer_build(params: Value) -> Result<RunOutput, String> {
    let cfg: HmmerBuildConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    let input = cfg.input.clone().ok_or("missing input MSA (Stockholm)")?;
    let output = cfg.output.clone().ok_or("missing output (.hmm)")?;
    let name = cfg
        .name
        .clone()
        .filter(|n| !n.trim().is_empty())
        .ok_or("a profile name is required")?;
    let opts = HmmerBuildOptions {
        name,
        acc: cfg.acc.clone().filter(|s| !s.trim().is_empty()),
        desc: cfg.desc.clone().filter(|s| !s.trim().is_empty()),
        alphabet: cfg.alphabet.clone().filter(|s| !s.trim().is_empty()),
        window_length: cfg.window_length,
        window_beta: cfg.window_beta,
    };
    write_hmm_output(&output, |f| {
        let hmm = hmmer_build(Path::new(&input), &opts)?;
        write_hmm_out(&hmm, f)
    })?;
    Ok((
        vec![IoRef { path: input, role: "msa".into() }],
        vec![IoRef { path: output, role: "hmm".into() }],
        "built 1 profile".into(),
    ))
}

/// Write records as FASTA using the record **name** as the id (Biopython `.id`),
/// matching the CLI's `write_fasta_bp_out` (used by genbank_to_fasta/extract_*/clean).
fn write_fasta_bp_path(path: &str, records: &[SeqRecord]) -> Result<(), String> {
    let mut w = std::io::BufWriter::new(
        std::fs::File::create(path).map_err(|e| format!("creating {path}: {e}"))?,
    );
    write_fasta_bp(&mut w, records).map_err(|e| format!("writing FASTA: {e}"))?;
    std::io::Write::flush(&mut w).map_err(|e| e.to_string())
}

fn set_of(v: &[String]) -> Option<HashSet<String>> {
    if v.is_empty() { None } else { Some(v.iter().cloned().collect()) }
}

fn run_genbank_to_fasta(params: Value) -> Result<RunOutput, String> {
    let cfg: GenbankToFastaConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    if cfg.input.is_empty() {
        return Err("select at least one input file".into());
    }
    let output = cfg.output.clone().ok_or("missing output (FASTA)")?;
    let records = load_inputs(&cfg.input)?;
    write_fasta_bp_path(&output, &records)?;
    let inputs = cfg.input.iter().map(|p| IoRef { path: p.clone(), role: "sequence".into() }).collect();
    Ok((inputs, vec![IoRef { path: output, role: "fasta".into() }], format!("wrote {} sequences", records.len())))
}

fn run_clean_sequences(params: Value) -> Result<RunOutput, String> {
    let cfg: CleanSequencesConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    if cfg.input.is_empty() {
        return Err("select at least one input file".into());
    }
    let output = cfg.output.clone().ok_or("missing output")?;
    let fasta_type = cfg.fasta_type.clone().unwrap_or_else(|| "protein".into());
    let mut records = Vec::new();
    for p in &cfg.input {
        records.extend(load_sequence(p, &fasta_type)?);
    }
    let opts = clean_sequences::CleanSequencesOptions {
        fasta_type,
        clean_name: cfg.clean_name,
        deduplicate_names: cfg.deduplicate_names,
        strip: cfg.strip,
        filter_by_aa: cfg.filter_by_aa,
    };
    let mut stats = clean_sequences::CleaningStats::default();
    let out = clean_sequences::clean_sequences(records, &opts, &mut stats);
    let role = if cfg.fasta_out { "fasta" } else { "genbank" };
    if cfg.fasta_out {
        write_fasta_bp_path(&output, &out)?;
    } else {
        write_genbank(&output, &out).map_err(|e| format!("writing GenBank: {e}"))?;
    }
    let inputs = cfg.input.iter().map(|p| IoRef { path: p.clone(), role: "sequence".into() }).collect();
    Ok((inputs, vec![IoRef { path: output, role: role.into() }], format!("{} in → {} out", stats.input_count, stats.output_count)))
}

fn run_sort_contigs(params: Value) -> Result<RunOutput, String> {
    let cfg: SortContigsConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    if cfg.input.is_empty() {
        return Err("select at least one input file".into());
    }
    let output = cfg.output.clone().ok_or("missing output (GenBank)")?;
    let records = load_inputs(&cfg.input)?;
    let max_hits = match cfg.max_hits {
        Some(0) | None => None,
        Some(n) => Some(n),
    };
    let domains = set_of(&cfg.domains);
    let databases = if domains.is_none() { None } else { set_of(&cfg.databases) };
    let opts = sort_contigs::SortContigsOptions { max_hits, domains, databases };
    let out = sort_contigs::sort_contigs(records, &opts).map_err(|e| e.to_string())?;
    write_genbank(&output, &out).map_err(|e| format!("writing GenBank: {e}"))?;
    let inputs = cfg.input.iter().map(|p| IoRef { path: p.clone(), role: "sequence".into() }).collect();
    Ok((inputs, vec![IoRef { path: output, role: "genbank".into() }], format!("sorted {} contigs", out.len())))
}

fn run_select_by_coord(params: Value) -> Result<RunOutput, String> {
    let cfg: SelectByCoordConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    if cfg.input.is_empty() {
        return Err("select at least one input file".into());
    }
    let output = cfg.output.clone().ok_or("missing output (GenBank)")?;
    let coords_path = cfg.coords.clone().ok_or("a coordinate table is required")?;
    let table = std::fs::read_to_string(&coords_path).map_err(|e| format!("reading coordinate table: {e}"))?;
    let selections = select_by_coord::parse_coordinate_table(&table).map_err(|e| e.to_string())?;
    let mut records: Vec<(String, SeqRecord)> = Vec::new();
    for p in &cfg.input {
        let base = basename(p);
        for rec in load_sequence(p, "protein")? {
            records.push((base.clone(), rec));
        }
    }
    let out = select_by_coord::select_by_coord(&records, &selections).map_err(|e| e.to_string())?;
    write_genbank(&output, &out).map_err(|e| format!("writing GenBank: {e}"))?;
    let inputs = cfg.input.iter().map(|p| IoRef { path: p.clone(), role: "sequence".into() }).collect();
    Ok((inputs, vec![IoRef { path: output, role: "genbank".into() }], format!("selected {} regions", out.len())))
}

fn run_trim_contigs(params: Value) -> Result<RunOutput, String> {
    let cfg: TrimContigsConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    if cfg.input.is_empty() {
        return Err("select at least one input file".into());
    }
    let output = cfg.output.clone().ok_or("missing output (GenBank)")?;
    let mut records = load_inputs(&cfg.input)?;
    filter_contigs(&mut records, &cfg.contigs, cfg.contigs_file.as_deref())?;
    let (kb_truncation, cds_truncation) = trim_contigs::validate_range(
        cfg.kb_both, cfg.kb_left, cfg.kb_right, cfg.cds_both, cfg.cds_left, cfg.cds_right,
    ).map_err(|e| e.to_string())?;
    let strip_domains = parse_name_list(&cfg.domains, cfg.domains_file.as_deref()).map_err(|e| e.to_string())?;
    let opts = trim_contigs::TrimContigsOptions {
        kb_truncation,
        cds_truncation,
        strip_domains,
        domain_expr: cfg.domain_expr.clone(),
        no_domain: cfg.no_domain,
    };
    let out = trim_contigs::trim_contigs(records, &opts).map_err(|e| e.to_string())?;
    write_genbank(&output, &out).map_err(|e| format!("writing GenBank: {e}"))?;
    let inputs = cfg.input.iter().map(|p| IoRef { path: p.clone(), role: "sequence".into() }).collect();
    Ok((inputs, vec![IoRef { path: output, role: "genbank".into() }], format!("trimmed {} contigs", out.len())))
}

fn run_extract_domains(params: Value) -> Result<RunOutput, String> {
    let cfg: ExtractDomainsConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    if cfg.input.is_empty() {
        return Err("select at least one input file".into());
    }
    let output = cfg.output.clone().ok_or("missing output")?;
    let mut records = load_inputs(&cfg.input)?;
    filter_contigs(&mut records, &cfg.contigs, cfg.contigs_file.as_deref())?;
    let domains = parse_name_list(&cfg.domains, cfg.domains_file.as_deref()).map_err(|e| e.to_string())?;
    let opts = extract_domains::ExtractDomainsOptions {
        evalue: cfg.evalue,
        score: cfg.score,
        domains,
        pad_up: cfg.pad_up.unwrap_or(0),
        pad_down: cfg.pad_down.unwrap_or(0),
        combine: cfg.combine,
        keep_direction: cfg.keep_direction,
        keep_name: cfg.keep_name,
        search_hits: cfg.search_hits,
        databases: set_of(&cfg.databases),
        splice: cfg.splice,
    };
    let out = extract_domains::extract_domains(&records, &opts).map_err(|e| e.to_string())?;
    let role = if cfg.fasta_out { "fasta" } else { "genbank" };
    if cfg.fasta_out {
        write_fasta_bp_path(&output, &out)?;
    } else {
        write_genbank(&output, &out).map_err(|e| format!("writing GenBank: {e}"))?;
    }
    let inputs = cfg.input.iter().map(|p| IoRef { path: p.clone(), role: "sequence".into() }).collect();
    Ok((inputs, vec![IoRef { path: output, role: role.into() }], format!("extracted {} domains", out.len())))
}

fn run_extract_peptides(params: Value) -> Result<RunOutput, String> {
    let cfg: ExtractPeptidesConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    if cfg.input.is_empty() {
        return Err("select at least one input file".into());
    }
    let output = cfg.output.clone().ok_or("missing output")?;
    let mut records = load_inputs(&cfg.input)?;
    filter_contigs(&mut records, &cfg.contigs, cfg.contigs_file.as_deref())?;
    let target_domains = parse_name_list(&cfg.domains, cfg.domains_file.as_deref()).map_err(|e| e.to_string())?;
    let target_cdss = parse_name_list(&cfg.cdss, cfg.cdss_file.as_deref()).map_err(|e| e.to_string())?;
    let mut invert = cfg.invert;
    let extract_all = target_domains.is_none() && target_cdss.is_none() && !cfg.unannotated && !cfg.search_hits;
    if extract_all {
        invert = false;
    }
    let opts = extract_peptides::ExtractPeptidesOptions {
        evalue: cfg.evalue.unwrap_or(100_000_000.0),
        target_domains,
        target_cdss,
        search_hits: cfg.search_hits,
        strand: cfg.strand.clone().filter(|s| !s.is_empty()),
        unannotated: cfg.unannotated,
        invert,
        databases: set_of(&cfg.databases),
        keep_name: cfg.keep_name,
        name_field: cfg.name_field.clone(),
        extract_all,
    };
    let out = extract_peptides::extract_peptides(&records, &opts);
    let role = if cfg.fasta_out { "fasta" } else { "genbank" };
    if cfg.fasta_out {
        write_fasta_bp_path(&output, &out)?;
    } else {
        write_genbank(&output, &out).map_err(|e| format!("writing GenBank: {e}"))?;
    }
    let inputs = cfg.input.iter().map(|p| IoRef { path: p.clone(), role: "sequence".into() }).collect();
    Ok((inputs, vec![IoRef { path: output, role: role.into() }], format!("extracted {} peptides", out.len())))
}

fn run_extract_unannotated(params: Value) -> Result<RunOutput, String> {
    let cfg: ExtractUnannotatedConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    if cfg.input.is_empty() {
        return Err("select at least one input file".into());
    }
    let output = cfg.output.clone().ok_or("missing output")?;
    let mut records = load_inputs(&cfg.input)?;
    filter_contigs(&mut records, &cfg.contigs, cfg.contigs_file.as_deref())?;
    let opts = extract_unannotated::ExtractUnannotatedOptions {
        evalue: cfg.evalue,
        score: cfg.score,
        keep_direction: cfg.keep_direction,
        keep_name: cfg.keep_name,
        search_hits: cfg.search_hits,
        databases: set_of(&cfg.databases),
        largest: cfg.largest,
        lb: cfg.lb,
    };
    let out = extract_unannotated::extract_unannotated(&records, &opts).map_err(|e| e.to_string())?;
    let role = if cfg.fasta_out { "fasta" } else { "genbank" };
    if cfg.fasta_out {
        write_fasta_bp_path(&output, &out)?;
    } else {
        write_genbank(&output, &out).map_err(|e| format!("writing GenBank: {e}"))?;
    }
    let inputs = cfg.input.iter().map(|p| IoRef { path: p.clone(), role: "sequence".into() }).collect();
    Ok((inputs, vec![IoRef { path: output, role: role.into() }], format!("extracted {} regions", out.len())))
}

fn run_color_genbank(params: Value) -> Result<RunOutput, String> {
    let cfg: ColorGenbankConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    if cfg.input.is_empty() {
        return Err("select at least one input file".into());
    }
    let output = cfg.output.clone().ok_or("missing output (GenBank)")?;
    let mut records = load_inputs(&cfg.input)?;
    filter_contigs(&mut records, &cfg.contigs, cfg.contigs_file.as_deref())?;
    let targets = color_genbank::ColorTargets::parse(cfg.mode.as_deref().unwrap_or("cdss")).map_err(|e| e.to_string())?;
    let color_feature = if cfg.domain_database { "database" } else { "name" }.to_string();
    let mut color_map = match &cfg.color_table {
        Some(path) if !path.trim().is_empty() => {
            let text = std::fs::read_to_string(path).map_err(|e| format!("reading color table: {e}"))?;
            let table = color_genbank::read_color_table(&text).map_err(|e| e.to_string())?;
            color_genbank::ColorMap::from_table(table)
        }
        _ => color_genbank::ColorMap::auto(),
    };
    let search_hit_color = match cfg.search_hit_color.as_deref().filter(|c| !c.trim().is_empty()) {
        Some(c) => Some(color_genbank::normalize_color_hex(c).map_err(|e| e.to_string())?),
        None => None,
    };
    let opts = color_genbank::ColorGenbankOptions {
        targets,
        search_hit_color,
        clear: cfg.clear,
        databases: set_of(&cfg.databases),
        color_feature,
    };
    let out = color_genbank::color_genbank(records, &mut color_map, &opts);
    write_genbank(&output, &out).map_err(|e| format!("writing GenBank: {e}"))?;
    let mut outputs = vec![IoRef { path: output, role: "genbank".into() }];
    if let Some(path) = cfg.color_table_out.as_deref().filter(|p| !p.trim().is_empty()) {
        use std::fmt::Write as _;
        let mut t = String::new();
        for (annotation, color) in color_map.items() {
            let _ = writeln!(t, "{annotation}\t{color}");
        }
        std::fs::write(path, t).map_err(|e| format!("writing color table: {e}"))?;
        outputs.push(IoRef { path: path.to_string(), role: "table".into() });
    }
    let inputs = cfg.input.iter().map(|p| IoRef { path: p.clone(), role: "sequence".into() }).collect();
    Ok((inputs, outputs, format!("colored {} records", out.len())))
}

fn run_color_table_to_legend(params: Value) -> Result<RunOutput, String> {
    let cfg: ColorTableToLegendConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    if cfg.input.is_empty() {
        return Err("select at least one color-table file".into());
    }
    let svg_path = cfg.svg.clone().ok_or("missing output (SVG)")?;
    let mut table: Vec<(String, String)> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for p in &cfg.input {
        let text = std::fs::read_to_string(p).map_err(|e| format!("reading {p}: {e}"))?;
        let entries = color_genbank::read_color_table(&text).map_err(|e| e.to_string())?;
        for (k, v) in entries {
            if let Some(&i) = index.get(&k) {
                table[i].1 = v;
            } else {
                index.insert(k.clone(), table.len());
                table.push((k, v));
            }
        }
    }
    let title = cfg.title.as_deref().filter(|t| !t.trim().is_empty()).unwrap_or("Legend");
    let svg = color_table_to_legend::color_table_to_legend(&table, title);
    std::fs::write(&svg_path, svg).map_err(|e| format!("writing {svg_path}: {e}"))?;
    let inputs = cfg.input.iter().map(|p| IoRef { path: p.clone(), role: "table".into() }).collect();
    Ok((inputs, vec![IoRef { path: svg_path, role: "svg".into() }], format!("legend with {} entries", table.len())))
}

fn run_deduplicate(params: Value) -> Result<RunOutput, String> {
    let cfg: DeduplicateConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    if cfg.input.is_empty() {
        return Err("select at least one input file".into());
    }
    let output = cfg.output.clone().ok_or("missing output")?;
    if cfg.prefix_count && cfg.suffix_count {
        return Err("prefix_count and suffix_count are mutually exclusive".into());
    }
    let id = cfg.id.ok_or("an identity threshold (id) is required")?;
    let fasta_type = cfg.fasta_type.clone().unwrap_or_else(|| "protein".into());
    let mut records = Vec::new();
    for p in &cfg.input {
        records.extend(load_sequence(p, &fasta_type)?);
    }
    let opts = deduplicate_genbank::DeduplicateOptions {
        algorithm: cfg.algorithm.clone().unwrap_or_else(|| "cd-hit".into()),
        id,
        both_strands: cfg.both_strands,
        fasta_type: fasta_type.clone(),
        cpu: cfg.cpu.unwrap_or(0),
        bin_path: cfg.bin_path.clone().filter(|s| !s.trim().is_empty()),
        params: Vec::new(),
    };
    let result = deduplicate_genbank::deduplicate(&records, &opts).map_err(|e| e.to_string())?;
    let count_by_rep: HashMap<usize, usize> = result.clusters.iter().map(|(r, m)| (*r, m.len())).collect();
    let mut final_name: HashMap<usize, String> = HashMap::new();
    let mut out_records: Vec<SeqRecord> = Vec::new();
    for &i in &result.representatives {
        let mut rec = records[i].clone();
        let base = rec.name.clone();
        let count = count_by_rep.get(&i).copied().unwrap_or(1);
        let newname = if cfg.prefix_count {
            format!("{count}-{base}")
        } else if cfg.suffix_count {
            format!("{base}-{count}")
        } else {
            base
        };
        rec.name = newname.clone();
        if cfg.fasta_out {
            rec.id = newname.clone();
        }
        final_name.insert(i, newname);
        out_records.push(rec);
    }
    let role = if cfg.fasta_out { "fasta" } else { "genbank" };
    if cfg.fasta_out {
        write_fasta_bp_path(&output, &out_records)?;
    } else {
        write_genbank(&output, &out_records).map_err(|e| format!("writing GenBank: {e}"))?;
    }
    let mut outputs = vec![IoRef { path: output, role: role.into() }];
    if let Some(table_path) = cfg.cluster_table.as_deref().filter(|p| !p.trim().is_empty()) {
        let sep = cfg.cluster_sep.clone().unwrap_or_else(|| " ; ".into());
        let mut t = String::from("representative\tcontigs\n");
        for (rep, members) in &result.clusters {
            let rep_name = final_name.get(rep).cloned().unwrap_or_default();
            let names: Vec<String> = members.iter().map(|&m| records[m].name.clone()).collect();
            t.push_str(&format!("{rep_name}\t{}\n", names.join(&sep)));
        }
        std::fs::write(table_path, t).map_err(|e| format!("writing cluster table: {e}"))?;
        outputs.push(IoRef { path: table_path.to_string(), role: "table".into() });
    }
    let inputs = cfg.input.iter().map(|p| IoRef { path: p.clone(), role: "sequence".into() }).collect();
    Ok((inputs, outputs, format!("{} in → {} representatives", records.len(), out_records.len())))
}

fn run_hmmer_compare(params: Value) -> Result<RunOutput, String> {
    let cfg: HmmerCompareConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    if cfg.input.is_empty() || cfg.reference.is_empty() {
        return Err("select query and reference HMM file(s)".into());
    }
    let output = cfg.output.clone().ok_or("missing output")?;
    let opts = hmmer_compare_m::HmmerCompareOptions {
        score_cutoff: cfg.score_cutoff.unwrap_or(0.0),
        alignments: cfg.alignments,
        k: cfg.k,
        cpu: cfg.cpu.unwrap_or(0),
        max_output_gb: Some(cfg.max_output_gb.unwrap_or(25.0)),
    };
    let mut buf: Vec<u8> = Vec::new();
    hmmer_compare_m::hmmer_compare(&cfg.input, &cfg.reference, &opts, &mut buf).map_err(|e| e.to_string())?;
    std::fs::write(&output, &buf).map_err(|e| format!("writing {output}: {e}"))?;
    let mut inputs: Vec<IoRef> = cfg.input.iter().map(|p| IoRef { path: p.clone(), role: "hmm".into() }).collect();
    inputs.extend(cfg.reference.iter().map(|p| IoRef { path: p.clone(), role: "reference".into() }));
    Ok((inputs, vec![IoRef { path: output, role: "table".into() }], "wrote comparison table".into()))
}

fn run_hmmer_search(params: Value) -> Result<RunOutput, String> {
    let cfg: HmmerSearchConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    if cfg.input.is_empty() || cfg.reference.is_empty() {
        return Err("select query and reference HMM file(s)".into());
    }
    let output = cfg.output.clone().ok_or("missing output (.hmm)")?;
    // As the CLI does: a blank cutoff means the default for the reference alphabet.
    let score_cutoff = match cfg.score_cutoff {
        Some(v) => v,
        None => hmmer_search_m::default_score_cutoff_for(&cfg.reference)?.0,
    };
    let opts = hmmer_search_m::HmmerSearchOptions {
        score_cutoff,
        max_hits: cfg.max_hits,
        cpu: cfg.cpu.unwrap_or(0),
        max_output_gb: cfg.max_output_gb,
    };
    let found = write_hmm_output(&output, |f| hmmer_search_m::hmmer_search(&cfg.input, &cfg.reference, &opts, f))?;
    let mut inputs: Vec<IoRef> = cfg.input.iter().map(|p| IoRef { path: p.clone(), role: "hmm".into() }).collect();
    inputs.extend(cfg.reference.iter().map(|p| IoRef { path: p.clone(), role: "reference".into() }));
    Ok((inputs, vec![IoRef { path: output, role: "hmm".into() }], format!("kept {found} query profiles")))
}

fn run_hmmer_report(params: Value) -> Result<RunOutput, String> {
    let cfg: HmmerReportConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    if cfg.input.is_empty() {
        return Err("select at least one input .hmm file".into());
    }
    let output = cfg.output.clone().ok_or("missing output")?;
    // The declarative columns → core analyses (leading `name` is always emitted).
    let mut analyses = Vec::new();
    for c in &cfg.columns {
        analyses.push(c.to_analysis().map_err(|e| e.to_string())?);
    }
    let format = if cfg.json.is_some() { hmmer_report_m::OutputFormat::Json } else { hmmer_report_m::OutputFormat::Tsv };
    let records = hmmer_report_m::read_hmms_to_iterators(&cfg.input).map_err(|e| e.to_string())?;
    let col_names = cfg.column_names.as_deref();
    let mut f = std::io::BufWriter::new(
        std::fs::File::create(&output).map_err(|e| format!("creating {output}: {e}"))?,
    );
    hmmer_report_m::hmmer_report(&records, &analyses, col_names, format, &mut f).map_err(|e| e.to_string())?;
    std::io::Write::flush(&mut f).map_err(|e| e.to_string())?;
    let n_profiles: usize = records.iter().map(|(_, h)| h.len()).sum();
    let inputs = cfg.input.iter().map(|p| IoRef { path: p.clone(), role: "hmm".into() }).collect();
    Ok((
        inputs,
        vec![IoRef { path: output, role: "table".into() }],
        format!("reported {n_profiles} profiles × {} columns", analyses.len() + 1),
    ))
}

fn str_param<'a>(params: &'a Value, key: &str) -> Result<&'a str, String> {
    params.get(key).and_then(|v| v.as_str()).ok_or_else(|| format!("missing {key}"))
}

fn basename(path: &str) -> String {
    Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn run_export_enum_report(params: Value) -> Result<RunOutput, String> {
    // Resolved exactly as the CLI's arm does: the columns are `ColumnSpec`s, the
    // taxonomy is loaded only when a column needs it.
    let cfg: EnumConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    if cfg.input.is_empty() {
        return Err("select an input sequence file".into());
    }
    let output = cfg.output.clone().ok_or("missing output (TSV)")?;
    if cfg.columns.is_empty() {
        return Err("add at least one column".into());
    }
    let by = cfg.by_enum().map_err(|e| e.to_string())?;
    let analyses = cfg
        .columns
        .iter()
        .map(|c| c.to_analysis().map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, String>>()?;
    let tax = if needs_taxonomy(&analyses) {
        let path = cfg.ncbi_taxonomy_path.as_deref().ok_or(
            "the taxonomy columns need a taxonomy database: install NCBI Taxonomy from Reference Databases, or choose one",
        )?;
        Some(crate::taxonomy_cache::load(path)?)
    } else {
        None
    };
    let targets = parse_name_list(&cfg.contigs, cfg.contigs_file.as_deref())
        .map_err(|e| format!("reading contigs: {e}"))?;
    let mut records = Vec::new();
    for path in &cfg.input {
        // The file name, not the full path, is what the `filename` column shows.
        let fname = basename(path);
        records.extend(load_sequence(path, "protein")?.into_iter().map(|r| (r, fname.clone())));
    }
    if let Some(set) = &targets {
        records.retain(|(r, _)| set.contains(&r.name));
    }
    let databases: Option<HashSet<String>> = set_of(&cfg.databases);
    // TSV to `output`, and NDJSON to `json` as well when it is set, as the CLI does.
    let mut outputs = vec![IoRef { path: output.clone(), role: "report".into() }];
    let mut targets_out = vec![(output, OutputFormat::Tsv)];
    if let Some(json) = cfg.json.clone() {
        outputs.push(IoRef { path: json.clone(), role: "report".into() });
        targets_out.push((json, OutputFormat::Json));
    }
    let n_records = records.len();
    for (path, format) in targets_out {
        let mut f = std::io::BufWriter::new(
            std::fs::File::create(&path).map_err(|e| format!("creating {path}: {e}"))?,
        );
        enum_report(
            records.clone(),
            by,
            &analyses,
            cfg.column_names.as_deref(),
            tax.as_deref(),
            databases.as_ref(),
            format,
            &mut f,
        )?;
        std::io::Write::flush(&mut f).map_err(|e| format!("writing {path}: {e}"))?;
    }
    let inputs = cfg.input.iter().map(|p| IoRef { path: p.clone(), role: "sequence".into() }).collect();
    Ok((inputs, outputs, format!("reported {n_records} records × {} column groups", analyses.len())))
}

fn run_export_matrix_report(params: Value) -> Result<RunOutput, String> {
    let input = str_param(&params, "input")?.to_string();
    let output = str_param(&params, "output")?.to_string();
    let m = read_matrix(&input).map_err(|e| format!("reading matrix: {e}"))?;
    let report = matrix_report(&m, DEFAULT_MERGE_IMPACT_METRIC, DEFAULT_MAX_MERGE);
    std::fs::write(&output, render_text(&report)).map_err(|e| format!("writing report: {e}"))?;
    Ok((
        vec![IoRef { path: input, role: "matrix".into() }],
        vec![IoRef { path: output, role: "report".into() }],
        format!("wrote matrix report ({} nodes, {} edges)", report.n_nodes, report.n_edges),
    ))
}

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
    // Resolve the optional color table (name→color TSV); subset files aren't
    // exposed in the GUI form yet, so pass none for those.
    let color_table = match cfg.color_table.as_deref().filter(|p| !p.trim().is_empty()) {
        Some(path) => {
            let text = std::fs::read_to_string(path).map_err(|e| format!("reading color table: {e}"))?;
            let table = color_genbank::read_color_table(&text).map_err(|e| e.to_string())?;
            Some(table.into_iter().collect::<std::collections::HashMap<_, _>>())
        }
        None => None,
    };
    let opts = resolve_build_ssn(&cfg, None, color_table);
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

    if let Some(path) = cfg.color_table_out.as_deref().filter(|p| !p.trim().is_empty()) {
        let ct = result.resolved_color_table.as_ref().ok_or("no color table resolved (set color_by)")?;
        let mut items: Vec<(&String, &String)> = ct.iter().collect();
        items.sort_by(|a, b| a.0.cmp(b.0));
        let mut out = String::new();
        for (k, v) in items {
            out.push_str(k);
            out.push('\t');
            out.push_str(v);
            out.push('\n');
        }
        std::fs::write(path, out).map_err(|e| format!("writing color table: {e}"))?;
        outputs.push(IoRef { path: path.to_string(), role: "table".into() });
    }

    let inputs = vec![IoRef { path: input, role: "matrix".into() }];
    Ok((inputs, outputs, format!("wrote {} nodes, {} edges", result.table.names.len(), result.edges.len())))
}

fn run_build_ssn_viewer(params: Value) -> Result<RunOutput, String> {
    let cfg: BuildSsnViewerConfig =
        serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    let input = cfg.input.clone().ok_or("missing input matrix")?;
    let output = cfg.output.clone().ok_or("missing output (.dsnv path)")?;
    let name = Path::new(&output)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("SSN")
        .to_string();
    let opts = resolve_build_ssn_viewer(&cfg, name, None);
    let m = read_matrix(&input).map_err(|e| format!("reading matrix: {e}"))?;
    let bundle = build_ssn_viewer_bundle(&m, &opts).map_err(|e| format!("building bundle: {e}"))?;
    let compressed = gzip(&serialize_bundle(&bundle));
    std::fs::write(&output, &compressed).map_err(|e| format!("writing .dsnv: {e}"))?;

    let inputs = vec![IoRef { path: input, role: "matrix".into() }];
    let outputs = vec![IoRef { path: output, role: "dsnv".into() }];
    Ok((inputs, outputs, format!("wrote {}-node .dsnv bundle", m.nrows())))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn tools() -> Vec<ToolMeta> {
        tauri::async_runtime::block_on(list_tools())
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("domainator-gui-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Three hand-maintained lists have to agree, in three different files: the
    /// GUI registry below, `run_one`'s dispatch, and `domainator-config`'s schema
    /// emitter. A tool offered in the menu but missing from the dispatch is a
    /// dialog that fails only when the user presses Run; a dialog field that is
    /// not a config property is worse, because every config struct is
    /// `deny_unknown_fields`, so the run is rejected outright.
    #[test]
    fn every_offered_tool_is_dispatched_and_has_a_schema() {
        let root = scratch("dispatch");
        let root = root.to_string_lossy();
        for tool in tools() {
            let schema = domainator_config::schema(&tool.id)
                .unwrap_or_else(|e| panic!("{} is in the menu but has no schema: {e}", tool.id));
            let parsed: Value = serde_json::from_str(&schema).unwrap();
            let props = parsed["properties"].as_object().unwrap();
            for field in &tool.fields {
                assert!(
                    props.contains_key(&field.name),
                    "{}: dialog field {:?} is not a config property, so a run would be rejected",
                    tool.id,
                    field.name
                );
            }
            // Empty params take every runner down its "missing input" path long
            // before it touches a file, so this reaches the dispatch and nothing
            // more expensive.
            let not_canceled = AtomicBool::new(false);
            if let Err(e) =
                run_one(&tool.id, serde_json::json!({}), &root, 0, "job_test", &not_canceled)
            {
                assert!(
                    !e.contains("not runnable from the GUI"),
                    "{} is offered in the menu but has no run_one arm",
                    tool.id
                );
            }
        }
    }

    /// `cpu` is a config property on only some tools, and every config struct is
    /// `deny_unknown_fields` -- so injecting the preference blindly would not be
    /// ignored, it would make the run be **rejected**. This walks every offered
    /// tool and asserts the injection never produces an unknown-field rejection,
    /// which is the failure a user would otherwise only see on pressing Run.
    #[test]
    fn injecting_the_default_cpu_never_breaks_a_tool() {
        let root = scratch("default-cpu");
        let root = root.to_string_lossy();
        for tool in tools() {
            let schema_text = domainator_config::schema(&tool.id).unwrap();
            let schema: Value = serde_json::from_str(&schema_text).unwrap();
            let mut params = serde_json::json!({});
            domainator_core::app_support::tool_params::apply_default_cpu(&mut params, &schema, 4);

            let has_cpu = schema["properties"].get("cpu").is_some();
            assert_eq!(
                params.get("cpu").is_some(),
                has_cpu,
                "{}: cpu injected iff the schema declares it",
                tool.id
            );

            let not_canceled = AtomicBool::new(false);
            if let Err(e) = run_one(&tool.id, params, &root, 0, "job_cpu", &not_canceled) {
                assert!(
                    !e.contains("unknown field"),
                    "{}: injecting cpu made the config be rejected: {e}",
                    tool.id
                );
            }
        }
    }

    /// Every tool must land in a known group. One outside `TOOL_CATEGORIES` is
    /// sorted last and grouped under its own name in the native menu rather than
    /// disappearing, but that is a bug -- the same class as `ToolField::role`
    /// naming a role `ToolDialog` cannot render, and guarded the same way.
    #[test]
    fn every_tool_has_a_known_category() {
        for tool in tools() {
            assert!(
                TOOL_CATEGORIES.contains(&tool.category.as_str()),
                "{} has category {:?} (tools/{}.yml), which is not in TOOL_CATEGORIES",
                tool.id,
                tool.category,
                tool.id
            );
        }
    }

    /// A category no tool uses is a submenu that would render empty.
    #[test]
    fn every_category_is_used_by_at_least_one_tool() {
        let used: std::collections::HashSet<String> =
            tools().into_iter().map(|t| t.category).collect();
        for cat in TOOL_CATEGORIES {
            assert!(used.contains(cat), "no tool uses category {cat:?}; its submenu would be empty");
        }
    }

    /// A job left non-terminal by a process that has gone must not stay that way.
    /// `canceling` used to be missed, so a job canceled before a quit showed
    /// "canceling…" on every later launch; terminal rows must be left alone.
    #[test]
    fn orphaned_jobs_settle_to_a_terminal_status() {
        let job = |status: &str| JobInfo {
            job_id: "j".into(),
            tool: "t".into(),
            status: status.into(),
            message: "kept".into(),
            started_at_ms: "0".into(),
            outputs: Vec::new(),
            params: serde_json::json!({}),
        };
        for (before, after) in [
            ("running", "failed"),
            ("canceling", "canceled"),
            ("completed", "completed"),
            ("failed", "failed"),
            ("canceled", "canceled"),
        ] {
            let mut j = job(before);
            settle_orphaned_job(&mut j);
            assert_eq!(j.status, after, "{before} should settle to {after}");
            if before == after {
                assert_eq!(j.message, "kept", "a terminal row's message was rewritten");
            }
        }
    }

    /// The frontend's Tools button groups by first appearance; the native menu
    /// groups by `TOOL_CATEGORIES`. They agree only if `list_tools` is sorted.
    #[test]
    fn list_tools_is_in_category_order() {
        let order: Vec<usize> = tools()
            .iter()
            .map(|t| TOOL_CATEGORIES.iter().position(|c| *c == t.category).unwrap_or(usize::MAX))
            .collect();
        assert!(order.windows(2).all(|w| w[0] <= w[1]), "list_tools is not grouped: {order:?}");
    }

    /// A job canceled before dispatch must not run its tool at all. The flag used
    /// to be read only *after* `run_one` returned, so "cancel" could not stop
    /// anything — it just relabeled a run that had already completed and written
    /// its provenance. Dispatching an unknown tool would normally produce the
    /// "not runnable" error; seeing "canceled" instead proves the check happens
    /// first.
    #[test]
    fn a_job_canceled_before_dispatch_does_no_work() {
        let root = scratch("canceled-early");
        let canceled = AtomicBool::new(true);
        let err = run_one(
            "not-a-tool",
            serde_json::json!({}),
            &root.to_string_lossy(),
            0,
            "j",
            &canceled,
        );
        let Err(err) = err else { panic!("a canceled job must not report success") };
        assert!(err.contains("canceled"), "{err}");
    }

    #[test]
    fn an_unknown_tool_is_refused_by_name() {
        let root = scratch("unknown");
        let not_canceled = AtomicBool::new(false);
        let err = match run_one(
            "not-a-tool",
            serde_json::json!({}),
            &root.to_string_lossy(),
            0,
            "j",
            &not_canceled,
        ) {
            Err(e) => e,
            Ok(_) => panic!("an unknown tool must not run"),
        };
        assert!(err.contains("not runnable from the GUI"), "{err}");
    }

    /// A one-record GenBank file for taxon 562, and a taxdump directory for it.
    /// Shared with `cmd_files`' Enum Report tab tests.
    pub(crate) fn enum_report_fixture(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = scratch(name);
        let gb = dir.join("one.gb");
        std::fs::write(
            &gb,
            "LOCUS       rec1                      12 bp    DNA     linear   UNK 01-JAN-2026
DEFINITION  test record.
FEATURES             Location/Qualifiers
     source          1..12
                     /db_xref=\"taxon:562\"
     CDS             1..12
                     /locus_tag=\"c1\"
                     /product=\"widget\"
ORIGIN
        1 atgaaacccg gg
//
",
        )
        .unwrap();
        let tax = dir.join("taxdmp");
        std::fs::create_dir_all(&tax).unwrap();
        for (file, body) in [
            ("nodes.dmp", "1\t|\t1\t|\tno rank\t|\t\n2\t|\t1\t|\tsuperkingdom\t|\t\n561\t|\t2\t|\tgenus\t|\t\n562\t|\t561\t|\tspecies\t|\t\n"),
            ("names.dmp", "1\t|\troot\t|\t\t|\tscientific name\t|\t\n2\t|\tBacteria\t|\t\t|\tscientific name\t|\t\n561\t|\tEscherichia\t|\t\t|\tscientific name\t|\t\n562\t|\tEscherichia coli\t|\t\t|\tscientific name\t|\t\n"),
            ("merged.dmp", ""),
            ("delnodes.dmp", ""),
        ] {
            std::fs::write(tax.join(file), body).unwrap();
        }
        (gb, tax)
    }

    /// The enum-report runner reads the dialog's column objects through
    /// `EnumConfig`, as the CLI does -- parameterized columns and the taxonomy
    /// ones included. It used to take bare names of the static columns only and
    /// pass no taxonomy, so none of these could be run from the GUI.
    #[test]
    fn enum_report_runs_parameterized_and_taxonomy_columns() {
        let (gb, tax) = enum_report_fixture("enum-report");
        let out = gb.with_file_name("out.tsv");
        let params = serde_json::json!({
            "input": [gb.to_string_lossy()],
            "output": out.to_string_lossy(),
            "by": "cds",
            "ncbi_taxonomy_path": tax.to_string_lossy(),
            "columns": [
                {"type": "length"},
                {"type": "qualifier", "feature": "CDS", "qualifier": "product"},
                {"type": "taxname", "ranks": ["species", "genus"]},
                {"type": "taxid", "ranks": ["superkingdom"]},
                {"type": "rank_lineage"},
                {"type": "append", "name": "batch", "value_type": "int", "value": "7"}
            ]
        });
        let root = gb.parent().unwrap().to_string_lossy().into_owned();
        let not_canceled = AtomicBool::new(false);
        run_one("enum-report", params, &root, 0, "job_enum", &not_canceled).unwrap();
        let text = std::fs::read_to_string(&out).unwrap();
        let mut lines = text.lines();
        assert_eq!(
            lines.next().unwrap(),
            "contig\tcds\tlength\tqualifier_CDS_product\ttaxname_species\ttaxname_genus\ttaxid_superkingdom\trank_lineage\tbatch"
        );
        assert_eq!(
            lines.next().unwrap(),
            "rec1\tc1\t12\twidget\tEscherichia coli\tEscherichia\t2\tsuperkingdom; genus; species\t7"
        );
    }

    /// Asking for a taxonomy column with no taxonomy database is refused up front
    /// with what to do about it, not run to an empty column or a raw I/O error.
    #[test]
    fn enum_report_taxonomy_columns_need_a_taxonomy_database() {
        let (gb, _tax) = enum_report_fixture("enum-report-notax");
        let params = serde_json::json!({
            "input": [gb.to_string_lossy()],
            "output": gb.with_file_name("out.tsv").to_string_lossy(),
            "columns": [{"type": "taxname_lineage"}]
        });
        let root = gb.parent().unwrap().to_string_lossy().into_owned();
        let not_canceled = AtomicBool::new(false);
        let Err(err) = run_one("enum-report", params, &root, 0, "job_enum", &not_canceled) else {
            panic!("a taxonomy column with no taxonomy database must be refused");
        };
        assert!(err.contains("taxonomy database"), "{err}");
    }

    /// Required fields are what the dialog gates its Run button on, so every tool
    /// must declare at least one input and one output; a tool with none would
    /// present an empty form that silently does nothing.
    #[test]
    fn every_tool_declares_its_files() {
        for tool in tools() {
            assert!(!tool.label.is_empty() && !tool.description.is_empty(), "{}", tool.id);
            let roles: Vec<&str> = tool.fields.iter().map(|f| f.role.as_str()).collect();
            assert!(
                roles.contains(&"input_file") || roles.contains(&"input_files"),
                "{} declares no input",
                tool.id
            );
            assert!(roles.contains(&"output_file"), "{} declares no output", tool.id);
            // The two report tools build their output columns in the dialog, so
            // that field is the tool -- losing it leaves an empty report.
            if matches!(tool.id.as_str(), "hmmer-report" | "enum-report") {
                assert!(roles.contains(&"columns"), "{} lost its column builder", tool.id);
            }
            for f in &tool.fields {
                // A role ToolDialog.svelte does not handle renders as nothing, so
                // the field silently disappears from the form.
                assert!(
                    ROLES.contains(&f.role.as_str()),
                    "{}: field {:?} has role {:?}, which ToolDialog.svelte cannot render",
                    tool.id,
                    f.name,
                    f.role
                );
            }
            let mut names: Vec<&str> = tool.fields.iter().map(|f| f.name.as_str()).collect();
            names.sort_unstable();
            let before = names.len();
            names.dedup();
            assert_eq!(before, names.len(), "{} has a duplicate field", tool.id);
        }
        // Tool ids are unique, or `list_tools` would offer the same menu entry twice.
        let mut ids: Vec<String> = tools().into_iter().map(|t| t.id).collect();
        ids.sort();
        let n = ids.len();
        ids.dedup();
        assert_eq!(n, ids.len(), "duplicate tool id");
    }

    #[test]
    fn strand_accepts_only_f_and_r() {
        assert_eq!(parse_strand(Some("f")).unwrap(), Some(1));
        assert_eq!(parse_strand(Some("r")).unwrap(), Some(-1));
        // Both "unset" spellings the dialog can produce: absent, or an empty box.
        assert_eq!(parse_strand(None).unwrap(), None);
        assert_eq!(parse_strand(Some("")).unwrap(), None);
        for bad in ["F", "+", "forward", "1"] {
            assert!(parse_strand(Some(bad)).is_err(), "{bad:?} should be rejected");
        }
    }

    /// Taxonomy filtering needs a taxdump, and the expression form cannot be
    /// combined with the include/exclude lists. Both checks run before the
    /// (expensive) taxonomy load, so they are reachable without one.
    #[test]
    fn taxonomy_filters_validate_before_loading_a_taxdump() {
        // Nothing requested: no filter, and no taxdump required.
        assert!(compile_allowed_taxids(&[], &[], None, None).unwrap().is_none());
        assert!(compile_allowed_taxids(&[], &[], Some("   "), None).unwrap().is_none());

        let err = compile_allowed_taxids(&[2], &[], Some("Bacteria"), Some("/tmp")).unwrap_err();
        assert!(err.contains("mutually exclusive"), "{err}");

        for args in [(vec![2], vec![], None), (vec![], vec![9], None)] {
            let err = compile_allowed_taxids(&args.0, &args.1, args.2, None).unwrap_err();
            assert!(err.contains("ncbi_taxonomy_path"), "{err}");
        }
        // An empty path is the same as none -- the dialog sends "" for a blank box.
        let err = compile_allowed_taxids(&[2], &[], None, Some("  ")).unwrap_err();
        assert!(err.contains("ncbi_taxonomy_path"), "{err}");
    }

    /// The provenance DAG walk keys off these roles: `sequence` is a leaf source,
    /// `reference` is ignored. Mislabeling them breaks SSN-node-to-record tracing.
    #[test]
    fn reference_inputs_are_labeled_for_the_provenance_walk() {
        let v = ref_inputs("contigs.gb", &["a.hmm".into(), "b.hmm".into()]);
        let roles: Vec<(&str, &str)> =
            v.iter().map(|i| (i.path.as_str(), i.role.as_str())).collect();
        assert_eq!(
            roles,
            vec![("contigs.gb", "sequence"), ("a.hmm", "reference"), ("b.hmm", "reference")]
        );
        assert_eq!(ref_inputs("x.gb", &[]).len(), 1);
    }

    #[test]
    fn job_timestamps_survive_the_string_round_trip() {
        // Timestamps cross to the frontend as strings because JS numbers cannot
        // hold a u128; sorting job history depends on parsing them back.
        let ms = 1_700_000_000_123u128;
        assert_eq!(ms_str(ms).parse::<u128>().unwrap(), ms);
        let j = JobInfo {
            job_id: "j".into(),
            tool: "t".into(),
            status: "completed".into(),
            message: String::new(),
            started_at_ms: ms_str(ms),
            outputs: Vec::new(),
            params: serde_json::json!({}),
        };
        assert_eq!(job_ms(&j), ms);
    }
}
