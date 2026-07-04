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
from the filesystem (so CLI-created files always appear). The left panel's Folders
header has a **＋** to create a subdirectory under the selected folder
(`create_folder` + `rescan_tree`). Data files are never
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
- **External tools (auto-download).** `external_tools.rs` is a flexible registry:
  each `ToolSpec` describes a binary (per-platform download URL + archive kind +
  member), and `download()` fetches (via `ureq`), extracts (`.tar.gz`/raw), stores
  it under `<app_data>/bin/`, `chmod +x`, and sets the env var the core locator
  checks (`DOMAINATOR_DIAMOND` → `tools::external::find_binary`). The **External
  tools** panel (left sidebar, collapsed) lists each tool with installed/available
  status and a Download button; `ensure_env` at startup wires up already-downloaded
  binaries. First entry: **diamond** (Linux x64 prebuilt; macOS/Windows show
  "n/a" — no simple official release binary). Adding a future dependency (mmseqs2,
  foldseek, prodigal, …) is just another `ToolSpec` in `specs()`.
- **Jobs run off-thread** (`spawn_blocking`), tracked in a `JobRegistry` managed
  state, and broadcast as `job-update` events. The **Jobs panel** (left sidebar,
  above Databases; `JobsPanel.svelte`) shows live status (running/completed/
  failed/cancelled), the result message, and clickable output files (openable ones
  open in a viewer); running jobs have a Cancel button (`cancel_job`, best-effort —
  core tools aren't interruptible, so a cancelled job's result is discarded). Each
  run writes provenance (`jobs/<id>/job.json` + output `.dom.json`).
- **Report-export tools** (also shown as jobs): `enum-report` writes a per-contig/
  cds/domain **TSV**, `matrix-report` writes a **text** summary + suggested cutoffs.
  (These export files to disk; the in-app Enum Report / Matrix Report *viewer* tabs
  remain the interactive views.)
- **Cross-view navigation — DONE (detached windows).** The SSN shell posts selected
  node labels to the host (one additive `postMessage` hook in
  `assets/ssn_viewer_shell.html`); `SsnView` shows an "Open sequence(s)" bar;
  `resolve_record` scans the workspace's GenBank/FASTA for a record whose id/name
  matches the label; `open_sequence_window` (`cmd_windows.rs`) then opens each in a
  **detached OS `WebviewWindow`** (`?view=contig`; the window reads its own label →
  `standalone_params` → `(path, contig)`, so paths need no URL-encoding), and
  `App.svelte`'s standalone branch renders just a focused `ContigView`. Requires the
  `src-tauri/capabilities/default.json` capability (`core:default` for all windows)
  — which also enables the Jobs panel's `listen("job-update")`. `resolve_record`
  scans files (fine for modest workspaces); a `.didx`/provenance-indexed fast path
  can replace it.
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
- **HMM logo viewer** (`.hmm` → Logo tab): multi-profile files show a filterable
  profile list (scales to tens of thousands; list capped at 500 shown, refine by
  filter) and lazily render the selected profile's logo (`list_hmm_profiles` +
  per-index `load_hmm_logo`, both cached). `app_support/hmm_logo.rs` computes
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
