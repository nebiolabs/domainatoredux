# Domainator GUI

A Geneious-inspired Tauri v2 desktop app for the Domainator suite: a folder tree,
a file table, and a tabbed viewer dock, backed by `domainator-core` library
functions. Frontend is **Svelte 5 + Vite + TypeScript**; backend is thin
`#[tauri::command]`s delegating to `domainator_core::app_support`.

**This is the developer README** — architecture, persistence model, platform
validation, and deferred work. The end-user guide to the app (workspace model,
every viewer, running tools, the panels, and current limitations) is
[`docs/gui.md`](../../docs/gui.md).

## Run

```bash
cd apps/domainator-gui
npm install          # first time
npm run tauri dev    # launches the desktop window (needs a display)
```

Frontend-only checks (no display needed):

```bash
npm run build        # vite production build
npm run check        # svelte-check (type checking)
cargo build -p domainator-gui   # backend
```

Then: **Open workspace…** → pick a folder of data files → click a folder in the
tree → double-click / click a file in the table to open a viewer tab. After the
first time, the app reopens the workspace you were last in and offers the rest
from a **Recent** list in the left panel.

## Developing on Windows vs WSL2

Keep **WSL2 as the primary dev environment** (fast Rust builds, the `cargo test`
suite, and the Python-parity gate against the conda `domainator` env all live on
Linux) and use Windows only to *validate the GUI*:

- **Iterate the GUI in WSL2 via WSLg** (Windows 11): `npm run tauri dev` opens a
  real Linux window rendered by **WebKitGTK**. Good for fast UI loops.
- **Validate on native Windows before shipping.** Tauri uses a *different webview*
  per OS — **WebView2 (Chromium/Edge) on Windows** vs WebKitGTK on Linux — and
  Windows is a hard-requirement target. Run `cargo tauri dev` on the Windows side
  (Windows hosts WSL2, so it's not either/or) or smoke-test the GitHub Actions
  Windows installer periodically.
- **WebView2-sensitive spots to check first:** anything that creates a window or
  opens a native dialog (see the main-thread rule under "Deferred details" — this
  is where the one confirmed Windows-only hang lived), the SSN `<iframe srcdoc>`
  shell (blob Web Worker + `DecompressionStream`, `sandbox="allow-scripts
  allow-same-origin"`) and the canvas contig/logo rendering.
- **Windows smoke list** after touching any of that: pop out a viewer window *and
  then confirm the main window still loads a file* (a wedged event loop still
  paints and handles clicks, so "the UI responds" is not evidence it is healthy);
  Export SVG from a logo and check the dialog comes up *in front*; quit and
  relaunch to confirm the workspace and window geometry come back.
- **`seq_dist` needs a Windows `diamond.exe`** on Windows (it shells out to the
  external `diamond` binary; the Linux binary in WSL2 won't run natively).

## Architecture

- **Backend commands** (`src-tauri/src/`): `cmd_workspace.rs` (pick_directory,
  open_workspace, list_folder), `cmd_files.rs` (load_contig_view,
  load_matrix_report, render_ssn_html). Wired in `lib.rs`.
- **Reusable logic** in `crates/domainator-core/src/app_support/`:
  `workspace.rs` (file-type classification, folder scan, sidecar `.dom.json` +
  `workspace.json` metadata), `contig_view.rs` (serde `ContigFileView` with
  server-resolved colors + flattened filter keys).
- **Frontend** (`src/`): `App.svelte` (resizable shell), `lib/state.svelte.ts`
  (shared runes store), `lib/FolderTree.svelte`, `lib/FileTable.svelte`,
  `lib/ViewerDock.svelte`, `lib/viewers/*` (per-file-type views), `lib/api.ts`
  (typed `invoke` wrappers), `lib/types.ts` (DTO mirrors).

## Desktop shell (menus, context menus, preferences)

- **Native menu bar** (`src-tauri/src/app_menu.rs`), built from the data table in
  `src-tauri/src/menu_ids.rs`. Native rather than HTML because macOS is not
  opt-out — Tauri installs `Menu::default()` there when none is set, so an HTML
  bar would mean two menu bars unless the default is disabled, which deletes the
  app's only Quit/Hide/Services. Native also gives the only clipboard items that
  work in a webview (`PredefinedMenuItem::{cut,copy,paste,select_all}`) and
  accelerators that fire with focus inside a `<canvas>` or the SSN `<iframe>`.
  - **`on_menu_event` runs on the main thread, so the handler only emits.**
    Anything needing a window or a native dialog goes
    `emit → frontend → invoke(async command)` — the same re-entrancy rule
    `cmd_windows::spawn_window` documents.
  - Routing uses `webview_windows()` + `is_focused()`; `Manager::get_focused_window`
    is behind Tauri's `unstable` feature. Do **not** use
    `WebviewWindow::on_menu_event` to scope per window — Tauri's dispatch loop
    calls *every* registered window listener for *every* event, so two of them run
    each command twice.
  - The **Tools** menu is built from `cmd_tools::list_tools()` and grouped by
    `TOOL_CATEGORIES`, so it cannot drift from what `run_tool` can dispatch.
  - Detached viewer windows get a reduced menu via `WebviewWindowBuilder::menu()`
    on non-macOS; on macOS they share the app menu (`Window::set_menu` is
    unsupported there) and `menu.ts`'s `SHELL_ONLY` set is the backstop.
- **Context menus** are app-drawn HTML (`lib/ContextMenu.svelte` +
  `lib/contextmenu.svelte.ts`), one instance mounted by `App.svelte`. Native
  popups were rejected: the items are per-row/per-job/per-tab, so each right-click
  would build a fresh `Menu` plus N `MenuItem` resources over IPC, and every
  action is a frontend call on `ws` anyway.
- **The browser context menu is suppressed** by one bubble-phase listener in
  `src/main.ts`, which stands down for `defaultPrevented` (so component menus
  compose), for `input`/`textarea`/`contenteditable`, for a live text selection,
  and for `[data-native-menu]`. Those exceptions matter — the browser menu is the
  only thing offering a working **Paste**. The SSN viewer is a separate document,
  so its suppressor is appended to the `srcdoc` string in `SsnView.svelte`;
  **never** edit the generated `ssn_viewer_shell.html`.
- **Opening URLs / revealing files** goes through `tauri-plugin-opener`, which is
  registered but deliberately granted **no `opener:` permission**. Capabilities
  gate the JS→Rust IPC surface, not Rust-side calls, so the webview (including the
  SSN iframe) cannot reach the plugin; the only route out is `cmd_open`'s
  commands, which check the URL against `about::ALLOWED_HOSTS` and bound a reveal
  with `workspace::is_within`.
- **Preferences** (`lib/PreferencesDialog.svelte`) over `Prefs`: reopen-last-workspace,
  theme, default thread count, update checking, and the app's data locations.
  Thread count is app-level, not per-workspace, because it is a property of the
  *machine* — a workspace copied from a 64-core node to a laptop must not carry
  `default_cpu = 64`. It reaches runs through
  `app_support::tool_params::apply_default_cpu`, which consults the tool's schema
  first: every config struct is `deny_unknown_fields`, so injecting `cpu` into a
  tool that has no such property would make the run be *rejected*.
- **Theme** is `app.css`'s `--l-*` / `--d-*` palettes aliased onto the app tokens.
  The `prefers-color-scheme` block stays first so there is no flash before JS
  runs; `:root[data-theme]` then wins in both directions. The SSN viewer's shell
  is generated and does **not** follow the app theme.
- **Two capability lines are load-bearing** in `capabilities/default.json`:
  `core:window:allow-set-title` for the window title, and
  `core:window:allow-destroy` — required merely by *registering*
  `onCloseRequested`, because the JS helper calls `destroy()` itself whenever the
  handler does not `preventDefault()`. Without it even a clean close throws.
- **The contig viewer's sequence copy is `Cmd/Ctrl+Shift+C`**, not plain
  `Ctrl+C`. Edit → Copy owns that, and a menu accelerator is dispatched *before*
  the focused widget sees the key (AppKit checks `performKeyEquivalent:` ahead of
  the responder chain; GTK3 runs `gtk_window_activate_key` ahead of
  `gtk_window_propagate_key_event`), so the old binding would have gone quietly
  dead on macOS and Linux rather than merely double-copying — and
  `preventDefault` could not have helped, since the key never reaches JS. The bp
  selection is canvas state, not a DOM selection, so the webview's own Copy could
  never have served it. `CmdOrCtrl+Shift+C` is reserved in
  `menu_accelerators_are_unique` so no menu command can take it back.
- **Guard tests.** `menu_ids.rs` pins the Rust id table against
  `src/lib/menuIds.ts` with `include_str!` (so editing the TypeScript alone
  re-runs the test), plus uniqueness, accelerator, layout and separator checks;
  `menu.ts`'s `HANDLERS: Record<MenuId, Handler>` is the frontend half, enforced
  by `npm run check`. Do not relax it to `Partial<Record<…>>`.

## Persistence model

In-place browsing: the app points at a real directory; the folder tree is derived
from the filesystem (so CLI-created files always appear). The left panel's Folders
header has a **＋** to create a subdirectory under the selected folder
(`create_folder` + `rescan_tree`), and each file row has a trash button that
deletes the file (and its metadata) via `delete_file` (with a confirm).

**Reserved `.domainator/` directory.** Data files are never rewritten; instead *all*
app machinery lives under a single hidden directory, `<root>/.domainator/`: the
`workspace.json` manifest, the `databases.json` registry, job provenance
(`.domainator/jobs/<id>/`), per-file metadata sidecars mirrored under
`.domainator/meta/<rel>.dom.json`, and combined SSN subsets (`.domainator/subsets/`).
`.domainator` is the ONLY name the app reserves (so no clashes with user files); it
is hidden from the tree and rejected as a user folder name, and `delete_file`
refuses to touch anything inside it. Opening a workspace created before this change
migrates the old top-level `workspace.json`/`databases.json`/`jobs/` into it
automatically. Everything else stays Python/CLI-readable.

## Phase status

| Phase | Scope | Status |
| --- | --- | --- |
| 0 | serde DTOs (`contig_view`), `workspace` module | ✅ done |
| 1 | shell + folder browser + read-only contig viewer | ✅ done |
| 2 | matrix report tab + SSN viewer tab + multi-view registry | ✅ done |
| 3 | config crate + tool forms + off-thread run + provenance + cross-view nav | ✅ done (curated tools) |
| 4 | HMM logo view + enum-report tab + databases panel + contig editing | ✅ done |
| 5 | desktop shell: native menu bar, context menus, About/Preferences, theme, window title, close guard | ✅ done |

## Deferred details / known limitations

Carry these forward — they are intentional Phase-1/2 cuts, not oversights.

- **Config crate refactor — DONE.** `config.rs` lives in
  `crates/domainator-config/` (CLI + GUI both depend on it); verified byte-identical
  `--print_config` and `schema` output across all 28 tools. `resolve.rs` holds the
  shared config→`*Options` resolvers, and the **`transform-matrix`, `build-ssn`, and
  `build-ssn-viewer` CLI arms now call them** (verified: `cluster_tsv` byte-identical,
  `.ssnv`/`.xgmml` structurally identical order-independent — those two have
  pre-existing nondeterministic output ordering). The remaining CLI arms
  (domainate/domain_search/seq_dist/…) still resolve inline in `main.rs`; adding
  their resolvers is future cleanup, needed only when those tools become
  GUI-runnable.
- **Tool execution — the full SSN workflow runs from the GUI.** `run_tool`
  (`cmd_tools.rs`) handles **`domain-search` → `domainate` → `seq-dist` (or
  `compare-contigs`) → `transform-matrix` → `build-ssn` → `build-ssn-viewer`**.
  Matrix-producing tools write `.hdf5` so their output is a recognized type that
  chains into the SSN tools' input dropdowns (e.g. seq-dist → `.hdf5` →
  build-ssn-viewer → `.ssnv` → SSN tab). Each tool declares a **field spec** in
  `list_tools`: `input_file`/`input_files` → a dropdown of current-folder files of
  the type **plus matching registered databases** (so HMM references come from the
  Databases panel or a local `.hmm`); `output_file` → a filename the app prepends
  the current directory to; `param` → rendered from the tool's JSON schema. Adding
  a tool = a `list_tools` entry (fields) + a `run_*` arm.
  `domain-search` exposes **neighborhood extraction** — `whole_contig`,
  `cds_range`/`cds_up`/`cds_down`, `kb_range`/`kb_up`/`kb_down`, `strand` (f/r),
  `keep_direction`, `translate` — so the neighborhood-based SSN path works
  (domain-search with a CDS/kb range → neighborhood GenBank → compare-contigs or
  seq-dist → matrix → build-ssn-viewer). Verified: none→16 CDS, whole_contig→96,
  cds±1→48 on the pDONR fixture.
  `domain-search`/`domainate` expose **taxonomy filters** (`include_taxids` /
  `exclude_taxids` / `taxonomy_expr`, requiring an NCBI **taxdump directory** via
  `ncbi_taxonomy_path`) and `domain-search` also **decoys** (`decoys` list +
  `decoys_file`) — compiled in `cmd_tools.rs` via `NCBITaxonomy::load` +
  `compile_taxonomy_filter`/`_expression` and `parse_name_list` (mirroring the CLI).
  Array params (decoys/taxid lists) render as comma-separated text in the form.
  `seq-dist` defaults to the pure-Rust `hmmer` algorithm; the `diamond_*`
  algorithms need the external `diamond` binary — **auto-downloadable** via the
  **External tools** panel (see below). The workflow-tool option mapping lives
  inline in `cmd_tools.rs` (only transform/build-ssn/build-ssn-viewer share
  `resolve.rs`).
  The Tools menu also runs the **selection/filtering tools**: `select-by-cds`
  (CDSs by name/domain/`domain_expr`/`search_hits`/`unannotated` + neighborhood
  extraction + pad/dedup/strand/invert), `select-by-contig` (whole contigs by
  name/regex, domain content, taxonomy, length or CDS count; `fasta_out` toggles
  FASTA vs GenBank output), and `filter-domains` (strip domain annotations by
  e-value/overlap/name/database, keeping the sequences). Each has a `run_*` arm in
  `cmd_tools.rs` mirroring the CLI arm exactly (shared helpers `load_inputs`,
  `filter_contigs`, `parse_strand`); `input` is a multi-file dropdown (`input_files`).
  The **HMM curation** tools are also in the menu: `hmmer-select` ("HMM select" —
  pull a subset of profiles from `.hmm` file(s) by regex/exact/substring on
  name/desc/acc; pairs with the HMM Logo viewer) and `hmmer-build` ("HMM build" —
  build a profile from a Stockholm MSA). `hmmer-build`'s input is a **Stockholm**
  file (`.sto`/`.stockholm`/`.sth`), a GUI-only `classify_file` type added so MSAs
  show up in the dropdown (no viewer tab — `viewsFor` returns none, which renders
  the graceful "no viewer" message). Both output `.hmm`, so the result opens in the
  HMM Logo viewer and appears in the domain-search/domainate reference dropdowns.
  The menu additionally covers the **sequence utilities / extractors / HMM-vs-HMM**
  tools, each a `run_*` arm mirroring its CLI arm: `genbank-to-fasta`,
  `clean-sequences`, `sort-contigs`, `select-by-coord`, `trim-contigs`,
  `extract-domains`, `extract-peptides`, `extract-unannotated`, `color-genbank`
  (batch color-by-name/database + optional `color_table_out`), `color-table-to-legend`
  (SVG), `deduplicate` (hash backend needs nothing; cd-hit/diamond/usearch need a
  binary), `hmmer-compare`, and `hmmer-search`. FASTA-emitting tools use
  `write_fasta_bp` (record **name** as id, matching the CLI's `write_fasta_bp_out`).
  Two more GUI-only `classify_file` types support these: **`table`** (`.tsv`/`.csv`/
  `.tab`) so coordinate/color tables show in dropdowns, alongside **`stockholm`**.
- **`hmmer-report` with a column builder.** "HMM report (export)" writes a per-profile
  TSV/JSON report from `.hmm` file(s). Its `columns` field uses a bespoke
  **`hmm_columns`** role in `ToolDialog`: an ordered, add/remove list where each row
  is a static column (`source`/`acc`/`desc`/`length`/`consensus`) or an `append`
  constant column (name + value_type + value). `collectParams` serializes it to the
  tagged `Vec<HmmerReportColumn>` the config expects (static → `{type}`, append →
  `{type,name,value_type,value}`; static rows must omit the extras or
  `deny_unknown_fields` rejects them). A leading `name` column is always emitted.
  With this, **every finished CLI tool is reachable from the GUI.**
- **External tools (auto-download).** `external_tools.rs` is a flexible registry:
  each `ToolSpec` describes a binary (per-platform download URL + archive kind +
  member), and `download()` fetches (via `ureq`), extracts (`.tar.gz`/raw), stores
  it under `<app_data>/bin/`, `chmod +x`, and sets the env var the core locator
  checks (`DOMAINATOR_DIAMOND` → `tools::external::find_binary`). The **External
  tools** panel (left sidebar, collapsed) lists each tool with installed/available
  status and a Download button; `ensure_env` at startup wires up already-downloaded
  binaries. Handles `.tar.gz`, `.zip`, and bare (`Raw`) executables. Three entries:
  **diamond v2.2.5** (all platforms), **cdhit v0.1.0** — the Rust cd-hit port, one
  multiplexer binary, no Intel-macOS build so the panel shows `n/a` there — and
  **usearch v12.0-beta1**, whose assets are bare per-platform executables and are the
  one `Archive::Raw` user. cd-hit and usearch are GPL, so they are downloaded at
  runtime and never bundled with the app. Two tests guard the registry:
  diamond must stay at or above `deduplicate_genbank::MIN_DIAMOND_VERSION` (an older
  binary fails on a single-cluster `deduplicate`), and `managed_tools_match_the_core_locator`
  pins every spec's `bin_name`/`env_var` to the core's `*_BIN_NAME`/`*_ENV_VAR`
  constants — a rename on either side would install a binary where nothing looks for
  it. Adding a future dependency (mmseqs2, foldseek, prodigal, …) is just another
  `ToolSpec` in `specs()`.
- **All non-trivial commands run off the main thread — and so does anything that
  spins a nested message loop.** Tauri v2 runs synchronous `#[tauri::command]`s on
  the **main thread**, so a slow one stalls *all* IPC (e.g. every tool dialog would
  sit at "Loading…" while a matrix report or workspace scan runs).

  On Windows the consequence is worse than slowness. Anything that runs a nested
  message pump from a sync command — creating a WebView2 window, or showing a
  native dialog with rfd's *blocking* API — re-enters WebView2 from inside its own
  IPC dispatch. tao's event loop never returns to the top, so queued user events
  and **IPC responses are buffered forever**: the app keeps painting and handling
  clicks (WebView2 renders out of process) while nothing ever loads again. Note
  that `AppHandle::run_on_main_thread` does **not** help: it is
  `send_user_message(Message::Task(..))`, and `send_user_message` short-circuits to
  running the message *inline* when the caller is already on the main thread. The
  only fix is for the command itself to be `async` (or `#[tauri::command(async)]`).
  See the long comment on `cmd_windows::spawn_window`.

  The heavy commands are therefore `async` + `spawn_blocking`: `load_contig_view`,
  `run_enum_report`, `run_summary_stats`, `load_hmm_logo`/`list_hmm_profiles`,
  `load_matrix_report`, `resolve_record` (scans the workspace), `render_ssn_html`,
  `open_workspace`/`list_folder`/`rescan_tree`, `register_database`, and the GenBank
  write-back edits. Only the native pickers and light JSON reads stay synchronous.
  `list_tools`, `tool_schema`, and `list_external_tools` are `async` too: a tool
  dialog fetches its schema via `tool_schema`, and on Linux/WebKitGTK the main thread
  also drives WebView layout/paint — a *synchronous* schema fetch could queue behind
  a heavy viewer render and leave the dialog stuck at "Loading…". Running them on the
  async runtime makes them immune to whatever the main thread is doing. **Window
  creation (`open_sequence_window` / `open_file_window` / `open_hmm_logo_window`)
  and every native picker (`pick_directory`, `pick_files`, `pick_file`,
  `save_text_file`) are `async` for the reason above** — don't make any of them
  synchronous again. The pickers also `set_parent(&window)`, so Windows can't raise
  an ownerless dialog *behind* the app.
  **Belt-and-suspenders for the tool dialog:** tool JSON Schemas are static, so the
  frontend prefetches every tool's schema **at app startup** (before any workspace/
  GenBank viewer is mounted, i.e. while the main thread is idle) into a dedicated
  never-cleared cache (`toolSchemaCached` in `api.ts`, distinct from the viewer
  `cache.ts` that `clearCache()` wipes on workspace-open). Opening a tool dialog then
  reads its schema from memory with **no backend round-trip**, so it can't stall
  behind a large contig view's WebView rendering — the cause of dialogs (esp. the
  GenBank-input tools, opened while a big contig view was on screen) sitting at
  "Loading…". This is a pure-frontend fix, so it takes effect via Vite HMR without a
  backend restart.
- **Jobs run off-thread** (`spawn_blocking`), tracked in a `JobRegistry` managed
  state, and broadcast as `job-update` events. The **Jobs panel** (left sidebar,
  above Databases; `JobsPanel.svelte`) shows live status (running/completed/
  failed/cancelled), the result message, and clickable output files (openable ones
  open in a viewer); running jobs have a Cancel button. **Cancellation is genuinely
  best-effort and the UI says so.** `cancel_job` sets an `Arc<AtomicBool>` the
  worker can see and flips the row to `cancelling…` immediately; `run_one` checks
  it before dispatch (so a job cancelled while queued does no work at all) and
  again before the provenance write (so an abandoned run is not recorded). None of
  the core tools has an interruption point, so a tool already running finishes:
  whatever output file it had written stays on disk — it may have overwritten
  something, so deleting it behind the user's back would be worse — and the job's
  message says so. Killing the `diamond`/`cd-hit`/`usearch` child processes would
  need a `Child` handle plumbed out of `domainator-core`'s `tools/external.rs`; see
  "Deferred details". Each completed run writes provenance (`jobs/<id>/job.json` +
  output `.dom.json`).
- **Report-export tools** (also shown as jobs): `enum-report` writes a per-contig/
  cds/domain **TSV**, `matrix-report` writes a **text** summary + suggested cutoffs.
  (These export files to disk; the in-app Enum Report / Matrix Report *viewer* tabs
  remain the interactive views.)
- **Cross-view navigation — DONE (combined subset view).** The SSN shell posts
  selected node labels to the host (one additive `postMessage` hook in
  `assets/ssn_viewer_shell.html`); `SsnView` shows an "Open sequence(s)" bar. On
  click it resolves *every* selected label (`resolve_record` scans the workspace's
  GenBank/FASTA for a record whose id/name matches) and calls `extract_records`,
  which writes just those records to a single `.domainator/subsets/selection_N.gb`;
  that opens as **one** normal sequence tab with the full Contig / Enum Report /
  Summary sub-tabs — instead of N whole-file windows. Requires the
  `src-tauri/capabilities/default.json` capability (which also enables the Jobs
  panel's `listen("job-update")`). (`open_sequence_window`/`standalone_params` +
  `App.svelte`'s `?view=contig` standalone branch remain for a future detached-window
  option but are no longer used by the SSN bar.) `resolve_record` scans files (fine
  for modest workspaces); a `.didx`/provenance-indexed fast path can replace it.
- **Enum report export has a column picker.** The Tools-menu "Enum report (export)"
  now offers a `columns` checklist (a bespoke `enum_columns` `ToolDialog` role) of
  the taxonomy-free static analyses, in addition to `by`; `run_export_enum_report`
  honors the selection (falling back to the per-`by` default set).
- **Every tool option has help text.** `ToolDialog` renders each option's schema
  `description` as a hint. A shared dictionary (`common_help` in `domainator-config`,
  applied by `fill_common_descriptions` in `schema()`) fills a one-line description
  for any property a per-tool schema left blank, so all 333 options across the tools
  have a tooltip (and the CLI's emitted schema gets them too). Per-tool schemas can
  still override with a more specific description.
- **domainate / domain-search accept HMM *and* FASTA references.** The references
  field uses a `reference` pseudo file-type (`typeMatches` → hmm | fasta), matching
  what the core `read_references` accepts (protein/nucleotide HMM or FASTA; `.cm` and
  GenBank are not references). The dropdown lists folder files and registered
  databases of either kind.
- **HMM files open as a profile table with a Logo tab beside it.** `.hmm` files
  default to `HmmReport.svelte` — a fast, filterable, row-virtualized table of every
  profile (name / length / description) built from `list_hmm_profiles` (which never
  computes a logo, so opening a 25k-profile file is instant). `viewsFor("hmm")`
  gives it a sibling **Logo** tab (`HmmLogo.svelte`, which carries its own filterable
  profile browser). A row's **logo** button sets `ws.focusHmmProfile[path]` and
  switches the dock to that tab — the same consume-and-clear channel as
  `ws.focusContig`; **⧉** still opens the profile in a detached window
  (`open_hmm_logo_window` → `?view=hmmlogo`, where the `index` prop hides the
  browser list). The dock tab is the primary path deliberately: the logo used to be
  the only viewer that required spawning an OS window, which put WebView2 window
  creation in the hot path of an everyday action.
  `ViewerDock`'s per-file sub-view selection lives on the store (`ws.subActive`) so
  a viewer can switch the dock to a sibling tab like this.
- **Contig viewer renders multi-part features + circular wrap.** Each feature is
  drawn as one segment per location part (joined/multi-exon genes show boxes joined
  by thin intron connectors; only the terminal part gets the arrowhead). Because the
  parser stores an origin-spanning circular feature as a multi-part join, this also
  handles **circular wrap** — the two segments render at both ends and the
  origin-crossing connector (a gap > half the contig) is skipped. Circular contigs
  keep their "circular" badge; a dedicated *circular/arc* plot is still a possible
  future view. The density-overview binning also fills per part.
- **Contig viewer is doubly virtualized + LOD.** `load_contig_view` is async.
  *Horizontal:* ≤800 visible features draw fully; denser contigs draw only the
  features intersecting the scroll viewport; a very dense window (zoomed out)
  collapses to a binned **density strip** ("overview · zoom in for detail").
  *Vertical:* files with >150 contigs render only the vertically-visible ones in
  fixed 96px slots (spacers preserve the scrollbar), so 100k-contig files scroll
  smoothly. In vertical-virt mode a contig block is clipped to its slot (tall
  multi-lane contigs show their first ~3 lanes).
- **Viewer results are cached** (`src/lib/cache.ts`): a module-singleton keyed by
  command + path + params memoizes enum report, matrix report, contig view, HMM
  logo, and SSN html, so clicking between file tabs / view sub-tabs is instant
  instead of recomputing. Edits invalidate the file's entries (`invalidatePath`);
  opening a different workspace clears the cache.
- **Color editing / add-remove annotations** (write-back to the `Color`
  qualifier via the GenBank writer) is deferred to Phase 4; the viewer is
  read-only for now.
- **Compressed sequence files**: `load_contig_view` uses `read_genbank`/
  `read_fasta` (uncompressed). BGZF/gzip inputs and streaming large files are not
  handled yet.
- **`matrix_report::to_json` is CLI-parity-locked** (must equal the Python JSON),
  so the histogram for the matrix view is computed GUI-side from the public
  `MatrixReport.scores_desc` instead of being added to `to_json`.
- **SSN viewer** is hosted via `build_ssn_viewer::render_html` in an
  `<iframe srcdoc>`. Cross-linking (selecting an SSN node → open the source
  sequence) needs the one additive `postMessage` hook in
  `assets/ssn_viewer_shell.html` + a provenance map — deferred to Phase 3.
- **Multi-view registry**: sequence files now expose **Contig View + Enum Report**
  sub-tabs; the Enum Report tab (`run_enum_report`, async / off-thread) offers by
  contig/cds/domain with a **row-virtualized** sortable grid (only viewport rows in
  the DOM via spacer rows; scrollbar spans the full set), **per-column text
  filters**, and a **column picker** (toggle any static analysis — domains,
  architecture, score, identity, taxid/taxname lineage, etc.; defaults per
  granularity). The **Summary** tab is a GUI-native stats dashboard
  (`app_support/summary_stats.rs` → `run_summary_stats` → `SummaryView.svelte`):
  stat tiles (contigs, total residues, CDS, domains, distinct domains, length
  mean/median/range), a contig length histogram, top-N bar lists for domains /
  databases / feature kinds / organisms, and a **taxonomy icicle plot** built from
  each record's embedded ORGANISM lineage (`annotations.taxonomy` + organism — no
  external taxdump needed). Pure data + client-side charts — sidesteps the
  matplotlib dependency that keeps the CLI `summary_report` deferred.
  The Enum Report column picker also offers `taxid_self` / `taxname_self` (the
  record's own taxid + organism, from the source `db_xref` / ORGANISM — no taxdump
  needed) alongside the `*_lineage` options.
- **HMM logo viewer** (`.hmm` → Logo tab): multi-profile files show a filterable
  profile list (scales to tens of thousands; list capped at 500 shown, refine by
  filter) and lazily render the selected profile's logo (`list_hmm_profiles` +
  per-index `load_hmm_logo`, both cached). `app_support/hmm_logo.rs` computes
  per-position IC (`log2 K − H`, uniform background), letter heights, and HMMER
  occupancy; `HmmLogo.svelte` renders stacked glyphs + an occupancy strip **to a
  canvas**. It is **column-virtualized**: a full-width spacer div carries the
  scrollbar while the canvas is only viewport-sized and follows `scrollLeft`, so the
  browser never paints a model-wide (10k+ px) layer, and `draw()` touches only the
  visible columns on each rAF. Each `(char, colour)` glyph is rasterized once into
  an offscreen canvas and blitted with `drawImage`; below 11 px/column it falls back
  to coloured rects. **Export SVG** re-emits the full logo as vectors using the same
  ink-box metrics. (Do NOT port GPL `Bio-HMM-Logo`; true expected-insert-length bars
  are future work.)
- **Databases panel — DONE.** Workspace-global registry in `databases.json`
  (`app_support/registry.rs`); the left sidebar's Databases section lists entries
  with a molecule dot + `<molecule> <format>` label and shard count. Add via file
  picker → `classify_database` (format from extension; molecule from the first
  HMM's alphabet / first GenBank record's `molecule_type` / a FASTA residue sniff);
  remove via the × button. Sharded/BGZF dbs resolve via `resolve_database_shards`.
- **Contig editing — DONE (GenBank only).** Click a feature → a panel offers a
  color picker (write-back to the `Color` qualifier) and Remove; an "Add
  annotation" row adds a feature (kind/start/end/strand/note). All persist by
  rewriting the file through `io::genbank::writer` (which regenerates
  `domainator_<db>` rollups + sorts features), so output stays Python-readable;
  the view refetches after each edit. FASTA is read-only (nowhere to store
  features). `set_color_by_name` exists in core but isn't surfaced in the UI yet.
- **Non-symmetric matrices**: the matrix report assumes a symmetric SSN-style
  matrix (via `MaxTree`); query×reference matrices aren't meaningfully handled.
- **App preferences — DONE.** `app_support/prefs.rs` owns the format and the
  recent-workspace list semantics (most-recent-first set, capped at 10, entries
  whose directory is missing are filtered out of the offered list but kept in the
  file — an unplugged drive is *missing*, not *gone*); `cmd_prefs.rs` resolves it to
  `<app_data>/prefs.json`, next to the managed external-tool binaries. On launch the
  shell calls `startup_info` and reopens the most recent workspace, or shows the
  recent list in the left panel. Window size/position/maximized persist via
  `tauri-plugin-window-state`, filtered to the `main` label (detached windows get
  generated labels like `hmm0`, so persisting their geometry is meaningless).
  Splitter positions persist in `localStorage`, like the file table's columns.
  `app_support` has no parity counterpart, so its `cargo test`s are the whole
  safety net — see `CLAUDE.md`.
- **Still deferred, in rough priority order:**
  - **Killing a running tool's subprocess.** Needs a `Child` handle plumbed out of
    `domainator-core`'s `tools/external.rs` (diamond / cd-hit / usearch); the
    pure-Rust tools would still need interruption points of their own.
  - **A job queue with a concurrency cap.** `run_tool` spawns one blocking task per
    invocation with no limit, and the tools use rayon's global pool, so two or three
    concurrent runs saturate every core and starve the renderer.
  - **Per-tool progress events.** `set_job` emits exactly twice per job (start and
    terminal state); there is no progress channel, elapsed time, or output log.
  - **`FileTable` / `FolderTree` virtualization.** Both render every row.
    `TableView`, `EnumReport` and `HmmReport` are virtualized.
  - **Streaming the SSN payload.** `render_ssn_html` returns the whole embedded
    bundle as one JSON string over IPC, which then becomes an `<iframe srcdoc>`.
  - **Viewer cache eviction.** `cache.ts` is a plain `Map` cleared only on workspace
    change, so SSN HTML, enum row arrays and contig views accumulate for the session.
