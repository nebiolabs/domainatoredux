//! Tool listing / execution.
//!
//! Forms are driven by each tool's manifest entry (`domainatoredux describe`) under
//! the GUI policy of its spec file (`tool_specs.rs`), and the form's values are a
//! config file keyed by parameter name, as JSON. A run writes them to
//! `.domainatoredux/jobs/<id>/worker-config.json` and runs `<tool> --config <that
//! file>` through the CLI in a worker copy of this executable (`worker.rs`), so a
//! GUI run takes exactly the code path a CLI run does. The job's inputs and
//! outputs for provenance come from the tool spec (`job_io.rs`); each completed
//! run records `jobs/<id>/job.json` plus output sidecars.
//!
//! Jobs wait in a queue for one of `Prefs::max_concurrent_jobs` slots. Cancel
//! kills the worker's whole process tree and removes the declared outputs the run
//! created or changed.

use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use domainator_config::columns::EnumColumn;
use domainator_core::app_support::provenance::{new_job_id, now_ms, write_job, JobRecord};
use domainator_core::app_support::workspace::dot_dir;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};

use crate::job_io;
use crate::worker;

/// Live state of a tool run, tracked in the [`JobRegistry`] managed state and
/// broadcast to the UI via the `job-update` event.
#[derive(Serialize, Deserialize, Clone)]
pub struct JobInfo {
    pub job_id: String,
    pub tool: String,
    /// `queued` | `running` | `canceling` | `completed` | `failed` | `canceled`.
    pub status: String,
    pub message: String,
    pub started_at_ms: String, // u128 as string (JSON-safe)
    pub outputs: Vec<String>,
    /// The params the tool was run with — lets the UI "clone" a job into a fresh
    /// pre-filled dialog.
    pub params: Value,
}

/// Managed state: all jobs seen this session, the ones still live, and the queue
/// that limits how many run at once.
#[derive(Default)]
pub struct JobRegistry {
    jobs: Mutex<HashMap<String, JobInfo>>,
    /// Jobs this process has not finished: queued or running. Entries are removed
    /// when a job reaches a terminal state, so the map tracks live jobs rather
    /// than growing all session.
    live: Mutex<HashMap<String, Arc<LiveJob>>>,
    /// Workspace root for persisting job history to `.domainatoredux/jobs.json`
    /// (set when a workspace is opened via `load_jobs`).
    root: Mutex<Option<String>>,
    queue: JobQueue,
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
///   current-dir files of any of `file_types`, grouped in `sources` order,
///   pre-filled from the selected files (in `prefill_priority` order), else starting on `select`
/// * `output_file` — a filename (seeded from `output_name`); the app prepends the
///   current directory
/// * `param` — rendered from the manifest parameter's `value_type`, `choices` and
///   `multiple`
/// * `columns` — a report's column builder: an ordered list of column tokens
///   (`qualifier:CDS:product`), whose columns and arguments come from the manifest's
///   catalog (adjusted by the spec's `column_types`); the order is the output order
///
/// `ROLES` below is the list, and a test checks every field against it.
#[derive(Serialize)]
pub struct ToolField {
    pub name: String,
    pub label: String,
    /// The hint: the spec's `help`, else the manifest parameter's.
    pub help: Option<String>,
    pub role: String,
    /// Inputs: the file types the dropdown offers (the spec's `file_type`). Empty
    /// for every other role.
    pub file_types: Vec<String>,
    pub required: bool,
    /// The value the dialog seeds: the spec's `gui_default`, else the CLI default.
    pub default: Option<Value>,
    /// The CLI's default (the manifest's), so the dialog can say so when `default`
    /// differs.
    pub cli_default: Option<Value>,
    /// The manifest parameter's kind: `value`, `flag`, `path` or `columns`.
    pub kind: String,
    /// Its JSON type: of each value, for a multiple parameter.
    pub value_type: String,
    /// It takes a list of values.
    pub multiple: bool,
    /// The values it allows, when they are limited.
    pub choices: Vec<Value>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub exclusive_min: Option<f64>,
    /// `columns`: the report's catalog, `{name, args, repeat_last, help}` each.
    pub columns: Vec<Value>,
    /// Inputs: `databases_first` | `workspace_first`.
    pub sources: Option<String>,
    /// Inputs: `first` | `none` | `database:<managed id>`, for when no selected
    /// file was placed in the field.
    pub select: Option<String>,
    /// Inputs: selected files fill inputs in descending priority, ties in spec
    /// order (the spec's `prefill_priority`, unset being 0).
    pub prefill_priority: i32,
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
    /// Which menu group this tool belongs to, one of [`CATEGORY_LABELS`]' labels.
    pub category: String,
}

/// Every `role` [`ToolDialog.svelte`] can render. See [`ToolField`].
pub const ROLES: [&str; 5] = [
    "input_file",
    "input_files",
    "output_file",
    "param",
    "columns",
];

/// The Tools menu's groups, in the order they appear: the manifest's categories
/// (`domainator_config::registry::CATEGORIES`, upstream's), each with a menu label.
pub const CATEGORY_LABELS: [(&str, &str); 9] = [
    ("annotate", "Annotation & search"),
    ("edit", "Selection & editing"),
    ("convert", "Conversion"),
    ("profile", "HMM profiles"),
    ("compare", "Comparison"),
    ("network", "Networks"),
    ("report", "Reports"),
    ("plot", "Figures"),
    ("database", "Databases"),
];

/// The menu label of a manifest category.
pub fn category_label(category: &str) -> Option<&'static str> {
    CATEGORY_LABELS
        .iter()
        .find(|(key, _)| *key == category)
        .map(|(_, label)| *label)
}

/// The menu labels, in menu order.
pub fn category_order() -> impl Iterator<Item = &'static str> {
    CATEGORY_LABELS.iter().map(|(_, label)| *label)
}

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
    // category is kept. A tool without a category is a test failure; in a release
    // build it still shows, last, rather than vanishing.
    tools.sort_by_key(|t| {
        category_order()
            .position(|c| c == t.category)
            .unwrap_or(usize::MAX)
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
                let p = &f.param;
                let help = f.help.clone().or_else(|| {
                    p["help"]
                        .as_str()
                        .filter(|h| !h.is_empty())
                        .map(str::to_string)
                });
                let list = |key: &str| p[key].as_array().cloned().unwrap_or_default();
                ToolField {
                    name: f.name.clone(),
                    label: f.label.clone(),
                    help,
                    role: f.role.clone(),
                    file_types: f.file_type.clone(),
                    required: f.required,
                    default: f.effective_default().cloned(),
                    cli_default: p.get("default").filter(|d| !d.is_null()).cloned(),
                    kind: p["kind"].as_str().unwrap_or("value").to_string(),
                    value_type: p["type"].as_str().unwrap_or("string").to_string(),
                    multiple: p["multiple"] == true,
                    choices: list("choices"),
                    min: p["min"].as_f64(),
                    max: p["max"].as_f64(),
                    exclusive_min: p["exclusive_min"].as_f64(),
                    columns: list("columns"),
                    sources: f.sources.map(|s| s.as_str().to_string()),
                    select: f.select.as_ref().map(|s| s.to_string()),
                    prefill_priority: f.prefill_priority.unwrap_or(0),
                    output_name: f.output_name.clone(),
                    column_types: f.column_types.clone(),
                }
            })
            .collect(),
    }
}

#[derive(Serialize, Debug)]
pub struct RunResult {
    pub job_id: String,
    pub outputs: Vec<String>,
    pub message: String,
}

/// A job's cancel state and, while its worker runs, the handle that kills it.
/// Also what `cmd_db`'s database indexing uses.
#[derive(Default)]
pub(crate) struct LiveJob {
    canceled: AtomicBool,
    killer: Mutex<Option<worker::Killer>>,
}

impl LiveJob {
    pub(crate) fn cancel(&self) {
        self.canceled.store(true, Ordering::SeqCst);
        if let Some(k) = reg_lock(&self.killer).as_ref() {
            k.kill();
        }
    }

    pub(crate) fn is_canceled(&self) -> bool {
        self.canceled.load(Ordering::SeqCst)
    }

    /// Hold the running worker's killer. A cancel that arrived between the last
    /// check and now finds no killer to call, so it is honored here: `cancel`
    /// sets the flag before it takes the lock, and this reads it after.
    fn attach(&self, killer: worker::Killer) {
        let mut slot = reg_lock(&self.killer);
        if self.is_canceled() {
            killer.kill();
        }
        *slot = Some(killer);
    }

    fn detach(&self) {
        *reg_lock(&self.killer) = None;
    }
}

/// How many jobs may run at once when the preference is unreadable.
const DEFAULT_MAX_CONCURRENT_JOBS: usize =
    domainator_core::app_support::prefs::DEFAULT_MAX_CONCURRENT_JOBS;

/// First come, first served, at most `max` running.
///
/// Each worker defaults to every core, so two jobs at once already oversubscribe
/// the machine; the shared in-process thread pool used to hide that.
#[derive(Default)]
struct JobQueue {
    state: Mutex<QueueState>,
    wake: Condvar,
}

#[derive(Default)]
struct QueueState {
    waiting: VecDeque<String>,
    running: usize,
}

/// A running slot, given back on drop -- also when the job panics.
struct Slot<'a>(&'a JobQueue);

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        reg_lock(&self.0.state).running -= 1;
        self.0.wake.notify_all();
    }
}

impl JobQueue {
    /// Block until `job_id` is first in line and a slot is free. `None` when the
    /// job was canceled while it waited. `max` is asked again at every wake-up,
    /// so raising the preference lets queued jobs start without a restart.
    fn acquire(&self, job_id: &str, live: &LiveJob, max: impl Fn() -> usize) -> Option<Slot<'_>> {
        let mut st = reg_lock(&self.state);
        st.waiting.push_back(job_id.to_string());
        loop {
            if live.is_canceled() {
                st.waiting.retain(|j| j != job_id);
                self.wake.notify_all();
                return None;
            }
            if st.waiting.front().map(String::as_str) == Some(job_id) && st.running < max().max(1) {
                st.waiting.pop_front();
                st.running += 1;
                // The next in line may fit too.
                self.wake.notify_all();
                return Some(Slot(self));
            }
            st = self
                .wake
                .wait_timeout(st, Duration::from_millis(500))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    fn wake_all(&self) {
        self.wake.notify_all();
    }
}

/// How a job did not complete.
#[derive(Debug)]
enum JobFailure {
    /// The message says what the cleanup removed.
    Canceled(String),
    Failed(String),
}

/// Where a job's tool runs.
pub(crate) enum Launcher {
    /// A worker process started from this executable (the normal path).
    Worker(PathBuf),
    /// `domainator_cli::run` on the job's own thread: no kill, no crash
    /// isolation, no captured output. The escape hatch behind
    /// `DOMAINATOREDUX_INPROCESS_JOBS=1`, kept for one release in case worker
    /// processes misbehave on some platform, and what the unit tests use (a test
    /// binary has no worker mode).
    InProcess,
}

/// Set to `1` to run jobs in-process instead of in worker processes.
pub const INPROCESS_ENV: &str = "DOMAINATOREDUX_INPROCESS_JOBS";

pub(crate) fn launcher() -> Result<Launcher, String> {
    if std::env::var(INPROCESS_ENV).is_ok_and(|v| v == "1") {
        return Ok(Launcher::InProcess);
    }
    worker::worker_exe()
        .map(Launcher::Worker)
        .map_err(|e| format!("locating this executable: {e}"))
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
    let prefs_path = crate::cmd_prefs::prefs_path(&app).ok();

    // Apply the app-level default thread count, but only where the tool actually has
    // a `cpu` parameter -- a config naming an unknown parameter is rejected.
    // Guarded across all tools by `injecting_the_default_cpu_never_breaks_a_tool`.
    let mut params = params;
    if let Some(path) = &prefs_path {
        let default_cpu = domainator_core::app_support::prefs::load_prefs(path).default_cpu;
        if let Some(spec) = crate::tool_specs::specs().iter().find(|s| s.id == tool) {
            domainator_core::app_support::tool_params::apply_default_cpu(
                &mut params,
                &spec.manifest,
                default_cpu,
            );
        }
    }

    let job_info = |status: &str, message: String, outputs: Vec<String>| JobInfo {
        job_id: job_id.clone(),
        tool: tool.clone(),
        status: status.into(),
        message,
        started_at_ms: ms_str(started),
        outputs,
        params: params.clone(),
    };
    let live = Arc::new(LiveJob::default());
    reg_lock(&registry.live).insert(job_id.clone(), live.clone());
    set_job(
        &registry,
        &app,
        job_info("queued", String::new(), Vec::new()),
    );

    let (app_c, jid, tool_c, params_c, live_c) = (
        app.clone(),
        job_id.clone(),
        tool.clone(),
        params.clone(),
        live.clone(),
    );
    let root = PathBuf::from(&workspace_root);
    // A panic here surfaces as a `JoinError`. It must still reach a terminal state
    // below: returning early left the row at `running` (or `canceling`) for the
    // rest of the session, and persisted that way.
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        let registry = app_c.state::<JobRegistry>();
        let max = || {
            prefs_path
                .as_deref()
                .map(|p| domainator_core::app_support::prefs::load_prefs(p).max_concurrent_jobs)
                .unwrap_or(DEFAULT_MAX_CONCURRENT_JOBS)
        };
        let Some(_slot) = registry.queue.acquire(&jid, &live_c, max) else {
            return Err(JobFailure::Canceled("canceled before it started".into()));
        };
        mark_running(&registry, &app_c, &jid);
        let launcher = launcher().map_err(JobFailure::Failed)?;
        let job = Job {
            tool: &tool_c,
            params: &params_c,
            root: &root,
            job_id: &jid,
            started,
        };
        run_one(&job, &launcher, &live_c)
    })
    .await
    .unwrap_or_else(|e| Err(JobFailure::Failed(format!("the job crashed: {e}"))));

    reg_lock(&registry.live).remove(&job_id);
    match outcome {
        Ok(r) => {
            set_job(
                &registry,
                &app,
                job_info("completed", r.message.clone(), r.outputs.clone()),
            );
            Ok(r)
        }
        Err(JobFailure::Canceled(message)) => {
            set_job(&registry, &app, job_info("canceled", message, Vec::new()));
            Err("job canceled".into())
        }
        Err(JobFailure::Failed(message)) => {
            set_job(
                &registry,
                &app,
                job_info("failed", message.clone(), Vec::new()),
            );
            Err(message)
        }
    }
}

/// A queued job got its slot. One that was canceled meanwhile stays `canceling`.
fn mark_running(registry: &JobRegistry, app: &AppHandle, job_id: &str) {
    let info = {
        let mut jobs = reg_lock(&registry.jobs);
        match jobs.get_mut(job_id) {
            Some(j) if j.status == "queued" => {
                j.status = "running".into();
                Some(j.clone())
            }
            _ => None,
        }
    };
    if let Some(info) = info {
        let _ = app.emit("job-update", &info);
        persist_jobs(registry);
    }
}

/// Persisted job-history cap (newest kept), so `.domainatoredux/jobs.json` can't grow
/// without bound.
const MAX_PERSISTED_JOBS: usize = 500;

fn jobs_file(root: &str) -> PathBuf {
    dot_dir(Path::new(root)).join("jobs.json")
}

/// Write the current job history to `.domainatoredux/jobs.json` (newest first, capped).
/// Best-effort: no-op until a workspace root is known; failures are ignored.
fn persist_jobs(registry: &JobRegistry) {
    let Some(root) = reg_lock(&registry.root).clone() else {
        return;
    };
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
/// file's, and marks any left-over live jobs as interrupted (the process that
/// ran them is gone). Returns the history.
/// Reads and rewrites `.domainatoredux/jobs.json`, so `(async)`: a plain `fn`
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
    let live: HashSet<String> = reg_lock(&registry.live).keys().cloned().collect();
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
/// Every non-terminal state needs it. `canceling` used to be missed: it reaches
/// `jobs.json` whenever anything persists while a canceled job is stopping, and a
/// job that never finished stayed "canceling…" across every relaunch.
fn settle_orphaned_job(j: &mut JobInfo) {
    match j.status.as_str() {
        "running" => {
            j.status = "failed".into();
            j.message = "interrupted (app closed); its outputs may be incomplete".into();
        }
        "queued" => {
            j.status = "canceled".into();
            j.message = "not run (the app closed while it was queued)".into();
        }
        "canceling" => {
            j.status = "canceled".into();
            j.message = "canceled (the app closed while the job was stopping; \
                         its outputs may be incomplete)"
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

/// Cancel a queued or running job. A running job's worker is killed with
/// everything it started; a queued one leaves the queue. The row shows
/// `canceling` until the job's thread has cleaned up, which takes milliseconds.
///
/// `(async)`: it persists the history (see [`load_jobs`]).
#[tauri::command(async)]
pub fn cancel_job(app: AppHandle, registry: tauri::State<'_, JobRegistry>, job_id: String) {
    let live = reg_lock(&registry.live).get(&job_id).cloned();
    if let Some(live) = live {
        live.cancel();
        registry.queue.wake_all();
    }
    let pending = {
        let mut jobs = reg_lock(&registry.jobs);
        match jobs.get_mut(&job_id) {
            Some(j) if j.status == "running" || j.status == "queued" => {
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

/// One job to run.
struct Job<'a> {
    tool: &'a str,
    params: &'a Value,
    root: &'a Path,
    job_id: &'a str,
    started: u128,
}

/// Run one job to the end: write its config, run the tool, and record provenance.
///
/// A job canceled before this starts does no work at all. One canceled while its
/// tool runs has the declared outputs it created or changed removed, and is not
/// recorded in the provenance DAG.
fn run_one(job: &Job<'_>, launcher: &Launcher, live: &LiveJob) -> Result<RunResult, JobFailure> {
    if live.is_canceled() {
        return Err(JobFailure::Canceled("canceled before it started".into()));
    }
    let spec = crate::tool_specs::specs()
        .iter()
        .find(|s| s.id == job.tool)
        .ok_or_else(|| {
            JobFailure::Failed(format!("tool not runnable from the GUI: {}", job.tool))
        })?;
    let missing = job_io::missing_required_inputs(spec, job.params);
    if !missing.is_empty() {
        return Err(JobFailure::Failed(format!(
            "missing required input: {}",
            missing.join(", ")
        )));
    }
    preflight(job.tool, job.params).map_err(JobFailure::Failed)?;

    let dir = dot_dir(job.root).join("jobs").join(job.job_id);
    let config = dir.join("worker-config.json");
    std::fs::create_dir_all(&dir)
        .and_then(|()| {
            std::fs::write(
                &config,
                serde_json::to_vec_pretty(job.params).unwrap_or_default(),
            )
        })
        .map_err(|e| JobFailure::Failed(format!("writing the job's config: {e}")))?;

    let outputs = job_io::declared_outputs(spec, job.params);
    let snapshot = job_io::OutputSnapshot::take(&outputs);
    let args: Vec<OsString> = vec![job.tool.into(), "--config".into(), config.into_os_string()];
    let ran = run_cli_command(
        launcher,
        &args,
        job.root,
        Some(&dir.join("worker.log")),
        live,
    );
    if live.is_canceled() {
        let cleanup = snapshot.remove_changed();
        return Err(JobFailure::Canceled(job_io::canceled_message(&cleanup)));
    }
    let message = ran.map_err(JobFailure::Failed)?;

    let record = JobRecord {
        job_id: job.job_id.to_string(),
        tool: job.tool.to_string(),
        params: job.params.clone(),
        inputs: job_io::declared_inputs(spec, job.params),
        outputs: job_io::existing_outputs(&outputs),
        started_at_ms: job.started,
        finished_at_ms: now_ms(),
        status: "completed".into(),
        engine_version: env!("CARGO_PKG_VERSION").into(),
        upstream_version: domainator_core::version::UPSTREAM_VERSION.into(),
    };
    write_job(job.root, &record, &format!("{} output", job.tool))
        .map_err(|e| JobFailure::Failed(format!("writing provenance: {e}")))?;
    Ok(RunResult {
        job_id: job.job_id.to_string(),
        outputs: record.outputs.into_iter().map(|o| o.path).collect(),
        message,
    })
}

/// Checks the GUI makes before starting a tool, where its answer is better than
/// the CLI's.
///
/// A taxonomy column with no taxonomy database chosen: the CLI would fall back to
/// Python's `/tmp/ncbi_taxonomy_<user>` default, which a GUI user never set up,
/// and fail there with a file-not-found. The GUI knows the database can be
/// installed from its Reference Databases panel, so it says that instead.
fn preflight(tool: &str, params: &Value) -> Result<(), String> {
    let blank = |key: &str| {
        params
            .get(key)
            .and_then(Value::as_str)
            .is_none_or(|s| s.trim().is_empty())
    };
    if tool == "enum_report" && blank("ncbi_taxonomy_path") {
        let analyses: Vec<_> = params["columns"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| c.as_str()?.parse::<EnumColumn>().ok()?.to_analysis().ok())
            .collect();
        if domainator_core::tools::enum_report::needs_taxonomy(&analyses) {
            return Err(
                "the taxonomy columns need a taxonomy database: install NCBI Taxonomy \
                        from Reference Databases, or choose one"
                    .into(),
            );
        }
    }
    let filters = ["include_taxids", "exclude_taxids", "taxonomy_expr"]
        .iter()
        .any(|k| match params.get(*k) {
            Some(Value::Array(a)) => !a.is_empty(),
            Some(Value::String(s)) => !s.trim().is_empty(),
            _ => false,
        });
    if filters && blank("ncbi_taxonomy_path") {
        return Err(
            "taxonomy filters need a taxonomy database: install NCBI Taxonomy from \
                    Reference Databases, or choose one"
                .into(),
        );
    }
    Ok(())
}

/// Run a CLI command line (without the program name) where `launcher` says, and
/// wait for it. `Ok` is the job message. A cancel through `live` kills it.
pub(crate) fn run_cli_command(
    launcher: &Launcher,
    args: &[OsString],
    cwd: &Path,
    log: Option<&Path>,
    live: &LiveJob,
) -> Result<String, String> {
    match launcher {
        Launcher::Worker(exe) => run_in_worker(exe, args, cwd, log, live),
        Launcher::InProcess => run_in_process(args),
    }
}

fn run_in_worker(
    exe: &Path,
    args: &[OsString],
    cwd: &Path,
    log: Option<&Path>,
    live: &LiveJob,
) -> Result<String, String> {
    let w = worker::spawn_exe(exe, args, Some(cwd), log)
        .map_err(|e| format!("starting the worker process: {e}"))?;
    live.attach(w.killer());
    let exit = w.wait();
    live.detach();
    let exit = exit.map_err(|e| format!("waiting for the worker process: {e}"))?;
    if exit.status.success() {
        Ok(job_io::success_message(&exit.stdout, &exit.stderr))
    } else {
        Err(job_io::failure_message(
            &exit.stderr,
            &job_io::describe_exit(&exit.status),
        ))
    }
}

/// The [`Launcher::InProcess`] path.
fn run_in_process(args: &[OsString]) -> Result<String, String> {
    let argv: Vec<OsString> = std::iter::once(OsString::from("domainatoredux"))
        .chain(args.iter().cloned())
        .collect();
    worker::run_cli(argv)
        .map(|()| "completed".to_string())
        .map_err(|e| format!("{e:#}"))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn tools() -> Vec<ToolMeta> {
        tauri::async_runtime::block_on(list_tools())
    }

    fn scratch(name: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("domainatoredux-gui-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn not_canceled() -> LiveJob {
        LiveJob::default()
    }

    fn run_in_process_job(tool: &str, params: Value, root: &Path) -> Result<RunResult, JobFailure> {
        let job = Job {
            tool,
            params: &params,
            root,
            job_id: "job_test",
            started: 0,
        };
        run_one(&job, &Launcher::InProcess, &not_canceled())
    }

    /// Whether the CLI accepts `params` as `<tool>`'s config file: the worker's
    /// exact command line, with `--print_config` so nothing runs. This is where a
    /// dialog field that is not a config property fails (`deny_unknown_fields`),
    /// and where a tool id the CLI has no subcommand for does.
    fn cli_accepts(dir: &Path, tool: &str, params: &Value) -> Result<(), String> {
        let path = dir.join(format!("{tool}.json"));
        std::fs::write(&path, serde_json::to_vec(params).unwrap()).unwrap();
        let args = [
            "domainatoredux",
            tool,
            "--config",
            &path.to_string_lossy(),
            "--print_config",
        ];
        worker::run_cli(args.map(OsString::from).to_vec()).map_err(|e| format!("{e:#}"))
    }

    /// The GUI registry (`tools/*.yml`) and the CLI's subcommands are two lists. A
    /// tool offered in the menu that the CLI does not know is a dialog that fails only
    /// when the user presses Run.
    #[test]
    fn every_offered_tool_is_dispatched() {
        let dir = scratch("dispatch");
        for tool in tools() {
            if let Err(e) = cli_accepts(&dir, &tool.id, &serde_json::json!({})) {
                panic!(
                    "{} is offered in the menu but the CLI rejects `{} --config`: {e}",
                    tool.id, tool.id
                );
            }
        }
    }

    /// The params a dialog produces on opening, before the user changes
    /// anything: every shown field's seeded default, every file field filled.
    fn dialog_params(spec: &crate::tool_specs::ToolSpec) -> Value {
        let mut out = serde_json::Map::new();
        for f in spec.fields.iter().filter(|f| f.show) {
            let v = match f.role.as_str() {
                "input_file" => Value::from(format!("/data/{}.in", f.name)),
                "input_files" => Value::from(vec![format!("/data/{}.in", f.name)]),
                "output_file" => Value::from(format!("/data/{}.out", f.name)),
                _ => match f.effective_default() {
                    Some(v) => v.clone(),
                    None => continue,
                },
            };
            out.insert(f.name.clone(), v);
        }
        Value::Object(out)
    }

    /// Worker mode hands the dialog's params to the CLI as a `--config` file, so a
    /// shape mismatch is a run rejected outright. For every tool: the params a
    /// dialog produces (manifest defaults, every file field filled, each column
    /// builder's default columns, the default thread count injected) must parse
    /// as its CLI config, and each report column must resolve to an analysis.
    #[test]
    fn every_tools_dialog_params_parse_as_its_cli_config() {
        use domainator_config::columns::HmmerColumn;
        let dir = scratch("dialog-params");
        for spec in crate::tool_specs::specs() {
            let mut params = dialog_params(spec);
            domainator_core::app_support::tool_params::apply_default_cpu(
                &mut params,
                &spec.manifest,
                4,
            );
            if let Err(e) = cli_accepts(&dir, &spec.id, &params) {
                panic!(
                    "{}: the CLI rejects the dialog's params {params}: {e}",
                    spec.id
                );
            }
            for token in params["columns"].as_array().into_iter().flatten() {
                let token = token.as_str().unwrap();
                let analysis = match spec.id.as_str() {
                    "enum_report" => token
                        .parse::<EnumColumn>()
                        .map(|c| c.to_analysis().map(drop)),
                    _ => token
                        .parse::<HmmerColumn>()
                        .map(|c| c.to_analysis().map(drop)),
                };
                analysis
                    .unwrap_or_else(|e| panic!("{}: column {token}: {e}", spec.id))
                    .unwrap_or_else(|e| panic!("{}: column {token}: {e}", spec.id));
            }
        }
    }

    /// `cpu` is a parameter of only some tools, and a config naming an unknown
    /// parameter is rejected -- so injecting the preference blindly would not be
    /// ignored, it would make the run be **rejected**. This walks every offered tool
    /// and asserts the injection never produces a rejection, which is the failure a
    /// user would otherwise only see on pressing Run.
    #[test]
    fn injecting_the_default_cpu_never_breaks_a_tool() {
        let dir = scratch("default-cpu");
        for spec in crate::tool_specs::specs() {
            let mut params = serde_json::json!({});
            domainator_core::app_support::tool_params::apply_default_cpu(
                &mut params,
                &spec.manifest,
                4,
            );
            let has_cpu = crate::tool_specs::manifest_param(&spec.manifest, "cpu").is_some();
            assert_eq!(
                params.get("cpu").is_some(),
                has_cpu,
                "{}: cpu injected iff the tool has it",
                spec.id
            );
            if let Err(e) = cli_accepts(&dir, &spec.id, &params) {
                panic!(
                    "{}: injecting cpu made the config be rejected: {e}",
                    spec.id
                );
            }
        }
    }

    /// The input roles each tool's hand-written runner recorded before jobs moved
    /// to worker processes, captured from those runners before they were deleted.
    /// `(tool, field, role)`.
    const GOLDEN_INPUT_ROLES: &[(&str, &str, &str)] = &[
        ("build_ssn", "input", "matrix"),
        ("build_tree", "input", "matrix"),
        ("build_projection", "input", "matrix"),
        ("clean_sequences", "input", "sequence"),
        ("color_genbank", "input", "sequence"),
        ("color_table_to_legend", "input", "table"),
        ("compare_contigs", "input", "genbank"),
        ("deduplicate_genbank", "input", "sequence"),
        ("domain_search", "input", "sequence"),
        ("domain_search", "reference", "reference"),
        ("domainate", "input", "sequence"),
        ("domainate", "reference", "reference"),
        ("gene_call", "input", "sequence"),
        ("enum_report", "input", "sequence"),
        ("extract_domains", "input", "sequence"),
        ("extract_peptides", "input", "sequence"),
        ("extract_unannotated", "input", "sequence"),
        ("filter_domains", "input", "sequence"),
        ("genbank_to_fasta", "input", "sequence"),
        ("hmmer_build", "input", "msa"),
        ("hmmer_compare", "input", "hmm"),
        ("hmmer_compare", "reference", "reference"),
        ("hmmer_report", "input", "hmm"),
        ("hmmer_search", "input", "hmm"),
        ("hmmer_search", "reference", "reference"),
        ("hmmer_select", "input", "hmm"),
        ("matrix_report", "input", "matrix"),
        ("plot_contigs", "input", "sequence"),
        ("rotate_contigs", "input", "sequence"),
        ("select_by_cds", "input", "sequence"),
        ("select_by_contig", "input", "sequence"),
        ("select_by_coord", "input", "sequence"),
        ("seq_dist", "input", "sequence"),
        ("seq_dist", "reference", "reference"),
        ("sort_contigs", "input", "sequence"),
        ("transform_matrix", "input", "matrix"),
        ("trim_contigs", "input", "sequence"),
    ];

    /// `provenance::trace_source_sequences` walks the job graph by input role, and
    /// that walk is what lets the SSN viewer pull a node's sequence out of the
    /// workspace. The roles are now derived from the tool spec, so a spec edit
    /// (a `file_type` changed, a field renamed) could break tracing with no error
    /// anywhere. Every role the old runners recorded must still be derived; an
    /// input they never recorded (a taxonomy database, a name list) may be added,
    /// but never with a role the walk follows.
    #[test]
    fn derived_provenance_roles_match_the_golden_table() {
        let mut derived: HashMap<(String, String), String> = HashMap::new();
        for spec in crate::tool_specs::specs() {
            for f in spec
                .fields
                .iter()
                .filter(|f| matches!(f.role.as_str(), "input_file" | "input_files"))
            {
                derived.insert((spec.id.clone(), f.name.clone()), job_io::input_role(f));
            }
        }
        for (tool, field, role) in GOLDEN_INPUT_ROLES {
            let got = derived.remove(&(tool.to_string(), field.to_string()));
            assert_eq!(
                got.as_deref(),
                Some(*role),
                "{tool}.{field}: derived role differs from the old runner's"
            );
        }
        use domainator_core::app_support::provenance::{
            FOLLOWED_INPUT_ROLES, SEQUENCE_SOURCE_ROLES,
        };
        for ((tool, field), role) in derived {
            let r = role.as_str();
            assert!(
                !SEQUENCE_SOURCE_ROLES.contains(&r) && !FOLLOWED_INPUT_ROLES.contains(&r),
                "{tool}.{field} is a new input with role {role:?}, which trace_source_sequences follows"
            );
        }
        // And the derivation reads the params the dialog sends.
        let spec = crate::tool_specs::specs()
            .iter()
            .find(|s| s.id == "domainate")
            .unwrap();
        let io = job_io::declared_inputs(
            spec,
            &serde_json::json!({"input": "contigs.gb", "reference": ["a.hmm", "b.hmm"], "genbank_out": "o.gb"}),
        );
        let roles: Vec<(&str, &str)> = io
            .iter()
            .map(|i| (i.path.as_str(), i.role.as_str()))
            .collect();
        assert_eq!(
            roles,
            vec![
                ("contigs.gb", "sequence"),
                ("a.hmm", "reference"),
                ("b.hmm", "reference")
            ]
        );
        assert_eq!(
            job_io::declared_outputs(spec, &serde_json::json!({"genbank_out": "o.gb"})),
            vec!["o.gb"]
        );
    }

    /// Every tool must land in a known group. One outside `CATEGORY_LABELS` is
    /// sorted last and grouped under "Other" in the native menu rather than
    /// disappearing, but that is a bug -- the same class as `ToolField::role`
    /// naming a role `ToolDialog` cannot render, and guarded the same way.
    #[test]
    fn every_tool_has_a_known_category() {
        for tool in tools() {
            assert!(
                category_order().any(|c| c == tool.category),
                "{} has category {:?}, which is not in CATEGORY_LABELS",
                tool.id,
                tool.category
            );
        }
    }

    /// The menu labels are upstream's categories, in upstream's order.
    #[test]
    fn category_labels_follow_the_manifest_categories() {
        let keys: Vec<&str> = CATEGORY_LABELS.iter().map(|(k, _)| *k).collect();
        let upstream: Vec<&str> = domainator_config::registry::CATEGORIES
            .iter()
            .map(|(k, _)| *k)
            .collect();
        assert_eq!(keys, upstream);
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
            ("queued", "canceled"),
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
    /// groups by `CATEGORY_LABELS`. They agree only if `list_tools` is sorted.
    #[test]
    fn list_tools_is_in_category_order() {
        let order: Vec<usize> = tools()
            .iter()
            .map(|t| {
                category_order()
                    .position(|c| c == t.category)
                    .unwrap_or(usize::MAX)
            })
            .collect();
        assert!(
            order.windows(2).all(|w| w[0] <= w[1]),
            "list_tools is not grouped: {order:?}"
        );
    }

    /// A job canceled before it starts must not run its tool at all -- a queued
    /// job must never launch. An unknown tool would normally produce the "not
    /// runnable" error; seeing "canceled" instead proves the check happens first.
    #[test]
    fn a_job_canceled_before_dispatch_does_no_work() {
        let root = scratch("canceled-early");
        let canceled = LiveJob::default();
        canceled.cancel();
        let params = serde_json::json!({});
        let job = Job {
            tool: "not-a-tool",
            params: &params,
            root: &root,
            job_id: "j",
            started: 0,
        };
        let err = run_one(&job, &Launcher::InProcess, &canceled);
        let Err(JobFailure::Canceled(msg)) = err else {
            panic!("a canceled job must not run: {err:?}")
        };
        assert!(msg.contains("canceled"), "{msg}");
        assert!(
            !dot_dir(&root).join("jobs").join("j").exists(),
            "a canceled job wrote its config"
        );
    }

    #[test]
    fn an_unknown_tool_is_refused_by_name() {
        let root = scratch("unknown");
        let err = match run_in_process_job("not-a-tool", serde_json::json!({}), &root) {
            Err(JobFailure::Failed(e)) => e,
            other => panic!("an unknown tool must not run: {other:?}"),
        };
        assert!(err.contains("not runnable from the GUI"), "{err}");
    }

    /// A required input left empty is refused before any process starts: a CLI
    /// tool with no input reads stdin, and a worker's stdin is its lifeline.
    #[test]
    fn a_missing_required_input_is_refused_before_launch() {
        let root = scratch("missing-input");
        let err = match run_in_process_job(
            "domainate",
            serde_json::json!({"genbank_out": "o.gb"}),
            &root,
        ) {
            Err(JobFailure::Failed(e)) => e,
            other => panic!("a job with no input must not run: {other:?}"),
        };
        assert!(
            err.contains("missing required input") && err.contains("Input sequences"),
            "{err}"
        );
    }

    /// The queue runs jobs first come, first served, never more than `max` at
    /// once, and a job canceled while it waits leaves the line.
    #[test]
    fn the_queue_limits_concurrency_and_drops_canceled_jobs() {
        let q = JobQueue::default();
        let (a, b, c) = (LiveJob::default(), LiveJob::default(), LiveJob::default());
        let slot_a = q.acquire("a", &a, || 1).unwrap();
        std::thread::scope(|s| {
            let waiter = s.spawn(|| q.acquire("b", &b, || 1).map(|_slot| "b ran"));
            let canceled = s.spawn(|| {
                std::thread::sleep(Duration::from_millis(50));
                q.acquire("c", &c, || 1).map(|_slot| "c ran")
            });
            std::thread::sleep(Duration::from_millis(150));
            assert_eq!(
                reg_lock(&q.state).running,
                1,
                "a second job started past max = 1"
            );
            c.cancel();
            q.wake_all();
            assert!(
                canceled.join().unwrap().is_none(),
                "a canceled queued job ran"
            );
            drop(slot_a);
            assert_eq!(waiter.join().unwrap(), Some("b ran"));
        });
        assert_eq!(reg_lock(&q.state).running, 0);
        assert!(reg_lock(&q.state).waiting.is_empty());
    }

    /// A one-record GenBank file for taxon 562, and a taxdump directory for it.
    /// Shared with `cmd_files`' Enum Report tab tests.
    pub(crate) fn enum_report_fixture(name: &str) -> (PathBuf, PathBuf) {
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

    /// The enum_report dialog's column tokens run through the CLI -- columns with
    /// arguments and the taxonomy ones included -- and the job is recorded with its
    /// inputs and outputs.
    #[test]
    fn enum_report_runs_parameterized_and_taxonomy_columns() {
        let (gb, tax) = enum_report_fixture("enum_report");
        let out = gb.with_file_name("out.tsv");
        let params = serde_json::json!({
            "input": [gb.to_string_lossy()],
            "tsv_out": out.to_string_lossy(),
            "by": "cds",
            "ncbi_taxonomy_path": tax.to_string_lossy(),
            "columns": [
                "length",
                "qualifier:CDS:product",
                "taxname:species:genus",
                "taxid:superkingdom",
                "rank_lineage",
                "append:batch:int:7"
            ]
        });
        let root = gb.parent().unwrap().to_path_buf();
        let r = run_in_process_job("enum_report", params, &root).unwrap();
        assert_eq!(r.outputs, vec![out.to_string_lossy().into_owned()]);
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
        let job = domainator_core::app_support::provenance::load_job(&root, "job_test").unwrap();
        let inputs: Vec<(&str, &str)> = job
            .inputs
            .iter()
            .map(|i| (i.path.as_str(), i.role.as_str()))
            .collect();
        assert!(
            inputs.contains(&(&*gb.to_string_lossy(), "sequence")),
            "{inputs:?}"
        );
        assert!(dot_dir(&root)
            .join("jobs/job_test/worker-config.json")
            .exists());
    }

    /// Asking for a taxonomy column with no taxonomy database is refused up front
    /// with what to do about it, not run into the CLI's `/tmp` default.
    #[test]
    fn enum_report_taxonomy_columns_need_a_taxonomy_database() {
        let (gb, _tax) = enum_report_fixture("enum_report-notax");
        let params = serde_json::json!({
            "input": [gb.to_string_lossy()],
            "tsv_out": gb.with_file_name("out.tsv").to_string_lossy(),
            "columns": ["taxname_lineage"]
        });
        let root = gb.parent().unwrap().to_path_buf();
        let Err(JobFailure::Failed(err)) = run_in_process_job("enum_report", params, &root) else {
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
            assert!(
                !tool.label.is_empty() && !tool.description.is_empty(),
                "{}",
                tool.id
            );
            let roles: Vec<&str> = tool.fields.iter().map(|f| f.role.as_str()).collect();
            assert!(
                roles.contains(&"input_file") || roles.contains(&"input_files"),
                "{} declares no input",
                tool.id
            );
            assert!(
                roles.contains(&"output_file"),
                "{} declares no output",
                tool.id
            );
            // The two report tools build their output columns in the dialog, so
            // that field is the tool -- losing it leaves an empty report.
            if matches!(tool.id.as_str(), "hmmer_report" | "enum_report") {
                assert!(
                    roles.contains(&"columns"),
                    "{} lost its column builder",
                    tool.id
                );
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
