# Plan: run tools in killable worker processes (self re-exec)

**Status:** implemented 2026-09-24 (phases 1–7). Where the code differs from the
text below:

- **Escape hatch.** `DOMAINATOREDUX_INPROCESS_JOBS=1` runs `domainator_cli::run` on the
  job's thread. It does not restore the `run_*` functions, which are deleted.
- **Output roles** in provenance come from the output file's type (`job_io::output_role`),
  not from each old runner's string. Nothing reads output roles. The **input** roles,
  which `trace_source_sequences` does read, are pinned by the golden-table test.
- **Default concurrency is 2**, not 1.
- **The CLI runs on a 256 MB-stack thread** in every path (`worker::run_cli`). Its
  dispatch function overflows a 2 MB test or tokio thread in debug builds, and would
  overflow Windows' 1 MB main thread too.
- **A worker whose stdin closes kills its own process group** on Unix, so a diamond it
  started stops with it. It does this only when it leads the group.
- **`hmmer-report`'s `json` field** is now an `output_file`, so it is recorded and
  cleaned up like the other outputs.
- **The taxonomy pre-check stays in the GUI** (`cmd_tools::preflight`). With no
  taxonomy database chosen, the CLI falls back to Python's `/tmp/ncbi_taxonomy_<user>`,
  which no GUI user has set up. The GUI's message points to Reference Databases
  instead.
- **Database indexing** (§3.6, phase 7) runs `format-db --index` in a worker and has its
  own Cancel in the Databases panel. It is not a Jobs-panel job, because a database
  can live outside any workspace and so has no `.domainatoredux/jobs/` to record into.

**Goal:** run each Tools-menu job in a child process that is a copy of the GUI
executable itself, started in a hidden "worker" mode. That makes Cancel real, keeps a
crashing tool from taking the app down, and puts GUI runs on the same code path as the
CLI. There is still only one binary to build, bundle and sign.

---

## 1. Why

What happens today (`src-tauri/src/cmd_tools.rs`):

- **Cancel does not stop a running tool.** `run_tool` runs `run_one` on Tauri's
  blocking pool. The core tools have no interruption points, so `cancel_job` can only
  set a flag. That flag stops a job that has not started yet and suppresses the
  provenance record, but a running tool continues to the end. This is documented as a
  known limitation in `docs/gui.md` ("Cancel is best-effort") and
  `docs/troubleshooting.md` ("Canceling a job in the app doesn't stop it").
- **A crash can take the whole app down.** A Rust panic in a tool is caught, and
  `run_tool` now turns it into a failed job. But an out-of-memory abort, a stack
  overflow, or a panic under `panic = "abort"` kills the GUI process, and any unsaved
  editor buffers go with it.
- **Memory stays with the GUI.** Every job's peak allocation goes back to the GUI's
  allocator, not to the OS, and a long session keeps growing.
- **The GUI keeps a second copy of every tool's wiring.** `cmd_tools.rs` has about 30
  hand-written `run_*` functions that deserialize the tool's config, re-state its
  defaults and call core. That code is separate from the CLI's, which the parity sweep
  checks, and it has already drifted. Examples:
  - `run_export_matrix_report` ignores every option and always uses
    `DEFAULT_MERGE_IMPACT_METRIC` and `DEFAULT_MAX_MERGE`.
  - `run_export_enum_report` reads `by`/`columns` loosely with its own defaults
    instead of through `EnumConfig`.
  - `run_domainate` re-states defaults such as `z` (1000.0) and `evalue` (0.001) that
    also live in `resolve.rs` and in the CLI.

## 2. Decision

**The GUI launches copies of itself as workers.** `main.rs` checks for a hidden worker
flag before anything else runs. If the flag is present, it runs the CLI's code in that
process and exits, without ever starting Tauri. The GUI launches one worker per job
with `std::env::current_exe()`.

Alternatives considered and rejected:

| Option | Why not |
| --- | --- |
| Ship the `domainator` CLI as a separate bundled binary (a Tauri sidecar) | A second binary to build per platform, bundle, sign, and notarize on macOS, plus a way for the GUI and CLI versions to disagree. Self re-exec gets the same benefits without any of that. |
| Fork the running process without starting a new program | Not available on Windows. On Unix, forking a process that already runs GTK, WebKit, tokio and rayon threads is unsafe: the copy has only the forking thread, and locks held by the other threads (malloc's included) stay locked forever. |
| Kill the tool's thread | No safe way exists. It leaves locks held and memory in a broken state. |
| Add cancellation checks to the core tools | Touches all 28 tools. Still can't interrupt a hang inside `hmmer-pure-rs` or an external diamond / cd-hit run, and gives no crash isolation. |

## 3. Design

### 3.1 Split `domainator-cli` into a library and a thin binary

`crates/domainator-cli/src/main.rs` is currently one 3,700-line file whose
`fn main() -> anyhow::Result<()>` calls `Cli::parse()`.

- Move everything into `src/lib.rs` and expose
  `pub fn run<I, T>(args: I) -> anyhow::Result<()> where I: IntoIterator<Item = T>, T: Into<OsString> + Clone`.
  It should use `Cli::try_parse_from(args)`. The hand-written parsers for
  `enum-report` and `hmmer-report` (which keep flag order, since column order follows
  it) must take the same argument list instead of reading `std::env::args()`.
- `src/main.rs` becomes `fn main() -> anyhow::Result<()> { domainator_cli::run(std::env::args_os()) }`.
- Add a `[lib]` section to `crates/domainator-cli/Cargo.toml`, and make
  `domainator-cli` a dependency of `apps/domainatoredux-gui/src-tauri`.
- Gate: the full differential parity sweep and `cargo test --workspace` must still
  pass, with the CLI's behavior unchanged byte for byte. This step is mostly moving
  code.

### 3.2 Worker mode in the GUI binary

`apps/domainatoredux-gui/src-tauri/src/main.rs`:

```rust
fn main() {
    // Must stay the first thing main does: nothing before it may touch Tauri,
    // GTK, WebView2 or the window-state plugin, or every worker opens a window.
    if let Some(code) = domainatoredux_gui::worker::maybe_run_worker() {
        std::process::exit(code);
    }
    domainatoredux_gui::run()
}
```

`src-tauri/src/worker.rs` (new):

- Recognizes `argv[1] == "--domainator-worker"`. The rest of the arguments are exactly
  a CLI command line, e.g. `domain-search --config <file>`. It calls
  `domainator_cli::run(["domainator", ...rest])` and returns exit code 0 on success and
  1 on error, with the error chain printed to stderr.
- **Worker exits when the GUI goes away.** Start a thread that reads stdin until end
  of file and then calls `std::process::exit(3)`. The parent keeps the worker's stdin
  pipe open for as long as it wants the job to run, so if the GUI crashes or is killed,
  the pipe closes and the worker stops. This works on every platform, and it also
  covers macOS, which has no equivalent of Linux's "kill me when my parent dies"
  (`PR_SET_PDEATHSIG`).
- `#[cfg(debug_assertions)]` test-only commands that §5's tests need:
  `__sleep <secs>` and `__sleep-with-child <secs>`. The second spawns a grandchild so
  the tests can check that cancel kills the whole tree. They must be compiled out of
  release builds.

The release build is already a windowed app
(`#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`), so a worker
never shows a console window on Windows. Piped stdout and stderr still work.

### 3.3 Launching a job (`run_tool`)

Replace the `spawn_blocking(run_one(...))` in `run_tool` with a launched worker.

1. **Config file.** Use the params after `apply_default_cpu`, which already follow the
   tool's `*Config` shape. Write them to
   `<root>/.domainatoredux/jobs/<job_id>/worker-config.json`. The job directory already
   exists for provenance (`provenance::write_job`), and keeping the file there leaves a
   record of exactly what ran. A file is used rather than command-line arguments
   because Windows limits a command line to about 32K characters, and a multi-file
   input list can exceed that.
2. **Launch** `current_exe() --domainator-worker <tool> --config <that file>`:
   - stdin piped, as the lifeline from §3.2.
   - stdout and stderr piped and copied to `.domainatoredux/jobs/<job_id>/worker.log`. Also
     keep the last few KB in memory for the job's message.
   - **On Linux:** launch `/proc/self/exe` instead of `current_exe()`. That still works
     if the installed binary was replaced by an update while the app was running.
     AppImage is fine: its mount lives as long as the parent process.
   - **On Unix:** `std::os::unix::process::CommandExt::process_group(0)` puts the
     worker in its own process group, and the group is what gets killed.
   - **On Windows:** create a Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, add
     the worker to it right after launch, and keep the handle in the job registry.
     Closing the handle, or the GUI dying, kills the worker and everything it started.
     This needs a direct `windows-sys` dependency with the `Win32_System_JobObjects`,
     `Win32_System_Threading` and `Win32_Foundation` features. The crate is already in
     the dependency tree through Tauri, so it adds nothing to the build.
   - **Environment:** inherited. `external_tools::ensure_env` sets `DOMAINATOREDUX_DIAMOND`
     and the other tool locations with `std::env::set_var`, including after a download
     finishes mid-session. A worker launched afterwards picks those up automatically.
3. **Wait** on a blocking thread for the process to exit, then read its exit status.
4. **On success:** check that each declared output exists, build the provenance record
   (§3.5), call `write_job`, and set the job to `completed`.
5. **On failure:** the job message is the tail of stderr. That is where
   `anyhow`'s error chain goes, so the user sees the same message the CLI prints.

**Concurrency:** N jobs means N workers, each defaulting to all cores. Add a
preference for the maximum number of concurrent jobs (default 1 or 2) and queue the
rest in the registry. Today's shared thread pool hides this oversubscription; separate
processes make it real.

### 3.4 Cancel, and a killed job's partial outputs

- `JobRegistry.canceled: HashMap<String, Arc<AtomicBool>>` becomes a map to a
  `RunningJob { kill: Box<dyn Fn() + Send + Sync>, ... }`, or an explicit per-platform
  handle. `cancel_job` kills the tree: `libc::killpg(pgid, SIGKILL)` on Unix, and
  `TerminateJobObject` on Windows. The job is then shown as `canceled` as soon as the
  process has exited, which takes milliseconds. `canceling` stays as a very short
  transitional state, and `settle_orphaned_job` still covers a quit during it.
- **Partial outputs (decision needed).** A killed tool can leave a truncated output
  that still looks valid, such as a GenBank file cut off at a record boundary. That is
  worse than no file.
  - **Recommended:** before launch, record each declared output's state (missing, or
    its `(size, mtime)`). On cancel, delete every declared output that did not exist
    before the job, or has changed since it started. List the deleted files in the
    job message.
  - Only delete **declared** output paths, the `output_file` fields from the tool
    registry. Never delete by pattern.
  - A pre-existing file the run had already overwritten is gone either way. Say so in
    the message instead of hiding it.
  - The longer-term fix is for the core writers to write to a temporary file and
    rename it at the end. That is out of scope here and belongs to the core crate.

### 3.5 Provenance without the per-tool `run_*` functions

Each `run_*` currently returns `(inputs, outputs, message)` built by hand.
Instead, derive `inputs` and `outputs` from the tool registry (`tool_registry()`: each
`ToolField`'s `role` and `file_type`) together with the params:

- Input fields (`input_file`, `input_files`) become input `IoRef`s, and `output_file`
  fields become output `IoRef`s. Record an output only if it exists after success,
  because many are optional.
- **The role strings are load-bearing.** `provenance::trace_source_sequences` walks
  the job graph using input roles: `"sequence" | "genbank" | "fasta"` are source
  sequences and `"matrix"` is followed further. This is what lets the SSN viewer pull a
  node's sequence out of the workspace. The derived role must therefore reproduce what
  each `run_*` emits today: `sequence` stays `sequence`, `hdf5` becomes `matrix`, `hmm`
  stays `hmm`, and so on.
  - Before deleting any `run_*`, capture every tool's current `(field, role)` pairs as
    a golden table in a test, and assert that the derivation matches it.
- **Job message:** use the worker's last non-empty stdout or stderr line, falling back
  to "completed". Today's per-tool summaries, such as "reported N profiles × M
  columns", come back only where the CLI already prints them. That is an acceptable
  loss. If it matters, add a machine-readable result line to the CLI later (§7).

### 3.6 What stays in-process

Only the Tools-menu jobs move to workers. These stay as they are, because they are
interactive, short, or already cancelable:

- **Viewer data commands** in `cmd_files.rs`: `load_contig_view`, the live
  `run_enum_report`, `run_summary_stats`, `render_ssn_html`, the `extract_*` commands,
  and the others.
- **Contig edits** in `cmd_edit.rs`.
- **Reference-database downloads** (`cmd_refdb.rs`). Their cancel flag is already read
  by the download stream itself.

**Follow-up:** `cmd_db::build_database_index` can take minutes and cannot be canceled.
It is exactly `format-db --index`, so once workers exist it should run as a worker job
too.

### 3.7 Removing the old path

- Once every tool runs through the worker, delete the `run_*` functions and `run_one`'s
  dispatch table. `run_one`'s job becomes "build the config file, launch, record".
- `every_offered_tool_is_dispatched_and_has_a_schema` changes meaning. "Dispatched"
  becomes "the CLI accepts `<tool> --config`": the test can call `domainator_cli::run`
  in-process with `--print_config` on an empty config and check that parsing succeeds.
  Keep the schema half as it is.
- For one release, keep a `DOMAINATOREDUX_INPROCESS_JOBS=1` environment variable that
  restores the in-process path, as an escape hatch if a worker problem appears on one
  platform. Remove it in the release after.

## 4. Pre-work: check that GUI params match the CLI configs

Worker mode sends the dialog's params to the CLI as a `--config` file. Every config
struct is `deny_unknown_fields`, so any mismatch in shape becomes a run that is
rejected outright. Before switching over, add a test that, for every tool in
`tool_registry()`:

- builds the params a dialog would produce: the schema defaults, every input and output
  field filled, and each column builder with its default columns;
- deserializes them into the CLI's config struct for that tool;
- runs the same resolve step the CLI runs (`resolve.rs`) without executing the tool.

Tools most likely to fail, based on their `run_*` bodies:

- **`enum-report`:** the GUI sends `by` and a flat `columns` string list, while the CLI
  reads `EnumConfig` with `ColumnSpec`s.
- **`hmmer-report`:** the `hmm_columns` role's tagged objects must match
  `HmmerReportColumn`.
- **`matrix-report`:** the GUI never passed any options before, so check that the
  dialog's fields are real config properties.

Also list the checks the GUI does that the CLI may not, such as `run_domainate`'s
"cannot search protein inputs with a nucleotide reference". Move each one into core or
the CLI so that both front ends keep it.

**Expected behavior changes:** `enum-report` and `matrix-report` exports will start
respecting every option the CLI supports. That fixes the drift described in §1, but
mention it in the changelog.

## 5. Tests

This is GUI-only code with no Python counterpart, so `cargo test` is the only
safety net (see `CLAUDE.md`, "Verification model"). Put tests that need the real
binary under `src-tauri/tests/`, where Cargo provides the path as
`env!("CARGO_BIN_EXE_domainatoredux-gui")`.

| Test | Guards against |
| --- | --- |
| `worker_mode_never_starts_tauri`: run the binary with `--domainator-worker __sleep 0`, with `DISPLAY` / `WAYLAND_DISPLAY` unset, and expect exit 0 | Worker code creeping below Tauri setup in `main`. A worker that opens a window, or that needs a display (which would break headless CI). |
| `cancel_kills_the_whole_tree`: run `__sleep-with-child 60`, kill, then assert the grandchild's pid is gone | Orphaned diamond / cd-hit processes left running after Cancel. |
| `a_worker_exits_when_its_stdin_closes` | Workers outliving a crashed GUI. |
| `every_tools_dialog_params_parse_as_its_cli_config` (§4) | A tool that fails with `unknown field` the first time the user presses Run. |
| `derived_provenance_roles_match_the_golden_table` (§3.5) | Breaking SSN node → source-sequence tracing without any error. |
| `a_canceled_job_removes_only_the_outputs_it_created` | Deleting a file the job never wrote, or leaving a truncated one behind. |
| A real run end to end: a small `domainate` on a `test/data` fixture through the worker, then compare against the same run through `domainator_cli::run` in-process | The worker path producing different output from the CLI. |

Existing tests to adapt:
- `a_job_canceled_before_dispatch_does_no_work`: still valid, since a canceled queued
  job must never launch.
- `injecting_the_default_cpu_never_breaks_a_tool`: unchanged, since it runs before the
  config file is written.
- `orphaned_jobs_settle_to_a_terminal_status`: unchanged.

**Windows smoke list** (add to `README.md`'s list):
- Cancel a diamond `seq-dist` run. In Task Manager, check that neither
  `domainatoredux-gui.exe` (the worker) nor `diamond.exe` is left behind.
- Kill the GUI from Task Manager in the middle of a job. Check that the worker dies.
- Confirm no console window appears when a job starts.

## 6. Phases

1. **CLI crate split** (§3.1). Exit criteria: `cargo test --workspace`, clippy, and the
   full parity sweep all green, with no change in behavior.
2. **Config-shape check** (§4). Fix every mismatch it finds, in the GUI dialog or in
   the config, whichever is wrong.
3. **Worker mode and launching** (§3.2, §3.3), behind the in-process escape hatch, with
   the tests from §5. Every tool now runs through a worker, but the `run_*` functions
   still exist.
4. **Cancel and output cleanup** (§3.4), plus the concurrency preference.
5. **Derived provenance** (§3.5). Once the golden-table test passes, delete the
   `run_*` functions and the dispatch table (§3.7).
6. **Docs:**
   - `docs/gui.md`: remove "Cancel is best-effort" and describe the partial-output
     policy.
   - `docs/troubleshooting.md`: remove "Canceling a job … doesn't stop it".
   - `README.md` (this directory): update the Windows smoke list.
   - `CLAUDE.md`: update the sentence about `run_one`'s dispatch arms.
7. **Follow-up:** run database indexing as a worker job (§3.6).

## 7. Open questions and future work

- **Progress.** A worker can report progress to the GUI as lines on stdout, but the
  CLI doesn't print any today. A `--progress-json` flag on the long-running tools
  (`domain-search`, `seq-dist`, `deduplicate`) would give the Jobs panel real progress
  bars.
- **Result line.** An optional `--result-json <path>` written by the CLI would bring
  back rich job messages and let the CLI report outputs it created that the GUI can't
  predict, such as shards.
- **Worker startup cost.** On Linux a worker still loads the WebKit shared libraries at
  launch, although it never initializes them: a few tens of milliseconds per job.
  That's negligible for Tools-menu jobs. If it ever matters for a frequent operation,
  split the GUI's library so worker mode avoids pulling in the webview.
- **Antivirus on Windows.** Some scanners inspect every new process. Measure how long
  a small job takes from start to finish once this lands.
