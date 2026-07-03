# Domainator GUI

A Geneious-inspired Tauri v2 desktop app for the Domainator suite: a folder tree,
a file table, and a tabbed viewer dock, backed by `domainator-core` library
functions. Frontend is **Svelte 5 + Vite + TypeScript**; backend is thin
`#[tauri::command]`s delegating to `domainator_core::app_support`.

The full design + phasing lives in the approved plan
(`~/.claude/plans/let-s-build-a-nice-peppy-valiant.md`). This file tracks
implementation status and deferred details.

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
tree → double-click / click a file in the table to open a viewer tab.

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
- **WebView2-sensitive spots to check first:** the SSN `<iframe srcdoc>` shell
  (blob Web Worker + `DecompressionStream`, `sandbox="allow-scripts
  allow-same-origin"`) and the SVG contig/logo rendering.
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

## Persistence model

In-place browsing: the app points at a real directory; the folder tree is derived
from the filesystem (so CLI-created files always appear). Data files are never
rewritten — GUI metadata (description/tags/provenance) goes in a sibling
`<name>.dom.json`, workspace settings in `workspace.json` at the root. Everything
stays Python/CLI-readable.

## Phase status

| Phase | Scope | Status |
| --- | --- | --- |
| 0 | serde DTOs (`contig_view`), `workspace` module | ✅ done |
| 1 | shell + folder browser + read-only contig viewer | ✅ done |
| 2 | matrix report tab + SSN viewer tab + multi-view registry | ✅ done |
| 3 | config crate + tool forms + off-thread run + provenance + cross-view nav | ✅ done (curated tools) |
| 4 | HMM logo view + enum-report tab + databases panel + contig editing | ✅ done |

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
- **Tool execution is a curated set, not generic.** `run_tool` (`cmd_tools.rs`)
  handles **`transform-matrix`**, **`build-ssn`**, and **`build-ssn-viewer`**
  (run build-ssn-viewer → the new `.ssnv` appears in the folder → opens in the SSN
  tab). Adding `enum_report`/`domainate`/`domain_search`/… is future work
  (`domainate`/`domain_search` need HMM-reference + multi-input I/O handling).
  Forms are schema-driven (`tool_schema` → `ToolDialog.svelte`), so adding a tool
  is: entry in `list_tools` + a `run_*` arm.
- **Jobs run off-thread** (`spawn_blocking`), tracked in a `JobRegistry` managed
  state, and broadcast as `job-update` events; `job_status`/`list_jobs`/`cancel_job`
  commands exist. Each run writes provenance (`jobs/<id>/job.json` + output
  `.dom.json` with `created_by_job`). Cancellation is **best-effort** (core tools
  aren't interruptible — a cancelled job's result is discarded; these matrix tools
  finish in ms anyway). No dedicated jobs panel yet — the events are emitted for a
  future one; `ToolDialog` shows the run result inline.
- **Cross-view navigation — DONE.** The SSN shell posts selected node labels to the
  host (one additive `postMessage` hook in `assets/ssn_viewer_shell.html`);
  `SsnView` shows an "Open sequence(s)" bar; `resolve_record` scans the workspace's
  GenBank/FASTA for a record whose id/name matches the label, and the contig viewer
  opens focused on it (scroll + highlight). Verified end-to-end at the data layer
  (`FeSOD_dist` SSN node → `FeSOD_20.gb` contig 0). Opening in a **detached OS
  window** (vs a focused tab) is a deferred enhancement — needs multi-window
  capabilities tested with a display. `resolve_record` currently scans files (fine
  for modest workspaces); a `.didx`/provenance-indexed fast path can replace it.
- **Contig viewer** draws each feature's *overall span*, not its individual parts
  — joined/multi-exon features render as one arrow (fine for prokaryotic data;
  add per-part rendering later). Circular contigs render **linearly** with a
  "circular" badge; origin-wrap / arc rendering is deferred.
- **No feature virtualization.** Acceptable for the "few hundred contigs / few Mb"
  scope; add viewport virtualization + level-of-detail if large genomes lag.
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
  sub-tabs; the Enum Report tab (`run_enum_report`) offers by contig/cds/domain
  with a sortable grid inferring numeric vs text columns from the JSON value kinds
  (columns default per granularity; a column picker is future work). A **Summary**
  dashboard tab is still to come (GUI-native stats, since the CLI `summary_report`
  is matplotlib-deferred).
- **HMM logo viewer** (`.hmm` → Logo tab): `app_support/hmm_logo.rs` computes
  per-position IC (`log2 K − H`, uniform background), letter heights, and HMMER
  occupancy; `HmmLogo.svelte` renders stacked SVG glyphs + an occupancy strip.
  Multi-model files show the first profile only (a profile picker + true
  expected-insert-length bars are future work; do NOT port GPL `Bio-HMM-Logo`).
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
