# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

A from-scratch **pure-Rust** rewrite of `domainator`, a Python bioinformatics suite for protein-domain / genome-neighborhood analysis and building sequence similarity networks (SSNs). Goals (see `README.md`, now a user-facing front page): cross-platform (incl. Windows), a single `domainatoredux <tool> [opts]` executable, a Tauri GUI (**built** — `apps/domainatoredux-gui/`), and easier agentic integration.

**Hard requirement:** on-disk compatibility with files produced by the original Python tool — GenBank, HDF5 matrices, and formatted databases (shards / BGZF / `.didx` indexes). Compatibility is **semantic** (re-readable by the original, equal under the Python suite's comparators), *not* byte-identical — see the `preserve_order` note under Architecture for why JSON is compared parsed.

## Build / run

```bash
cargo build                      # debug build of the workspace
cargo build --release            # optimized (lto); use for any timing/benchmarks
cargo build --workspace 2>&1 | grep -c warning:   # keep this at 0
cargo clippy --workspace --all-targets -- -D warnings   # also keep this clean
cargo fmt --all                                         # CI runs --check
./target/debug/domainatoredux <tool> [opts]           # run a tool
```

**Zero warnings means all of them**: `rustc`, `cargo clippy --all-targets -- -D warnings`,
and `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace` (a doc link to a renamed
item compiles fine; only rustdoc notices), plus `cargo fmt --all --check`. Lint levels
live in `[workspace.lints]` in the root `Cargo.toml`, which has every member opted in; every `unsafe` block
needs a `// SAFETY:` comment (`undocumented_unsafe_blocks`). CI also runs `cargo deny
check` (`deny.toml`: license allowlist — no GPL-family crate — plus advisories), `typos`
(`_typos.toml`, `locale = "en-us"`, which enforces the spelling rule below),
`svelte-check`/`tsc` on the frontends, `mdbook build` and an MSRV check; see
`docs/development.md`, "CI".
Use `--all-targets` — `cargo build` does not compile `#[cfg(test)]` code, so a broken
test-only initializer is invisible to the build alone. Where a lint is wrong for this
codebase, silence it with a narrow `#[allow(...)]` **plus a one-line reason** (the
existing ones cover Tauri IPC signatures, parity-mirroring parameter lists, and HMMER's
1-based node indices); never add a crate-wide allow.

**35 tools** (`domainatoredux <tool>`) plus the `describe`, `completion` and `doctor` builtins,
grouped: I/O and dev (`inspect`, `convert`, `genbank_to_fasta`, `format_db`); annotation
and search (`domainate`,
`domain_search`, `gene_call`, `filter_domains`); selection and extraction (`select_by_cds`,
`select_by_contig`, `select_by_coord`, `extract_domains`, `extract_peptides`,
`extract_unannotated`, `trim_contigs`, `rotate_contigs`, `sort_contigs`, `clean_sequences`,
`deduplicate_genbank`); color (`color_genbank`, `color_table_to_legend`); matrices and networks
(`seq_dist`, `compare_contigs`, `transform_matrix`, `build_ssn`, `build_ssn_viewer`,
`matrix_report`, `ssn_navigator`); reports (`enum_report`, `summary_report`); and the
HMM family (`hmmer_compare`, `hmmer_report`, `hmmer_select`, `hmmer_build`,
`hmmer_search`). `docs/porting-status.md` holds the per-tool coverage ledger (keyed by upstream Python
filename), moved there out of `README.md` when the docs became user-facing.

CLI flags use the Python suite's **underscore** names (e.g. `--max_overlap`,
`--dense_text`, `--whole_contig`, `-Z`/`--z`), set explicitly via clap `long = "..."` —
don't assume clap's kebab default. Subcommands are snake_case too (`rename_all` on the
`Command` enum), equal to upstream's tool names.

**The 1.0 interface.** Each
tool's interface is one clap `Args` struct, `crates/domainator-config/src/args/<tool>.rs`,
with upstream's parameter names (`rename_all = "snake_case"`, so id == long flag ==
config key), `CommonArgs` flattened in (`-v/-q/--config/--print_config`), and a
`registry.rs` entry (category, summary, required outputs, stdio
rules). Its body is `crates/domainator-cli/src/commands/<tool>.rs`. `config_file.rs`
turns a `--config` file (upstream's flat format) into command-line arguments before the
one clap parse, so CLI flags win and every check applies to files too; `--print_config`
and `domainatoredux describe` (`manifest.rs`, upstream's manifest v1 format) are read off
the clap command. Outputs go through `crates/domainator-cli/src/output.rs` (`-` = stdout,
atomic temp-file writes, `.gz`/`.bgz` = BGZF). Errors print as one line,
`domainatoredux <tool>: error: ...` (`report_error`, shared with the GUI worker). The
report tools' `--columns` take upstream's `name:arg:arg` tokens, parsed against the
catalogs in `columns.rs` (the clap value type, so a bad token fails at parse time).
An upstream option a tool lacks is listed in its registry entry's
`unsupported`, with upstream's default, so upstream configs that name it still load.
Every tool but the port-only `inspect` and `convert` (which take no config) is on it.

**Agent-facing JSON** (upstream's documented contract, see its `AGENTS.md`):
`enum_report` and `hmmer_report` write **streaming NDJSON** with `--json_out`, one
compact object per row; `matrix_report` and `summary_report` (also `--json_out`)
write **one JSON object** with a stable shape, compact and ASCII-escaped like Python's
`json.dump` (`io::python_json`); `ssn_navigator` always writes JSON. In every case the
option takes a path (`-` = stdout) and is **independent of `-o`**, so a run can produce
TSV/text and JSON together.

**Versioning** (`docs/development.md`, "Versioning and releases"): our own SemVer line is
`workspace.package.version` and nothing else (no `version` in any `tauri.conf.json` or
`package.json`); the Python release we are at parity with is `UPSTREAM_VERSION`/
`UPSTREAM_COMMIT` in `crates/domainator-core/src/version.rs`, shown beside ours and written
as a `.dsnv`'s `domainator_version`. Tags are `vX.Y.Z`; `CHANGELOG.md` names the parity
version per release.

**Documentation** lives in `docs/`, which is an **mdbook** (`book.toml` at the repo root
sets `src = "docs"`, so every page is also plain markdown that renders on GitHub).
User-facing: `installation`, `getting-started`, `concepts`, `cli-reference`, `examples`,
`gui`, `file-formats`, `config-and-agents`, `troubleshooting`. Developer-facing:
`development`, `architecture`, `porting-status`, `parity`. A new page must be listed in
`docs/SUMMARY.md` — `create-missing = false`, so an entry with no file is a build error.
`README.md` is a short user-facing front page; keep developer detail out of it.
Every `domainatoredux` command in a fenced block of `docs/` or `README.md` must parse
(`crates/domainator-cli/tests/doc_examples.rs`); comment a deliberate counterexample
`# error`.
`UPSTREAM.md` stays at the repo root because `.github/workflows/check.yml` greps it for
the upstream pin.

**Spelling: American English everywhere** — prose, doc comments, identifiers, UI
labels and status strings alike (`color`, `behavior`, `neighbor`, `center`, `labeled`,
`gray`, `canceled`/`canceling` but `cancellation`, `catalog`, `honor`, `favor`,
`analyze`, `-ize`/`-ization`). The repo was converted in one pass, so a British spelling
now reads as a typo; match the existing words when you add code or docs. Four places are
exempt because they are not ours to edit: the generated
`crates/domainator-core/assets/ssn_viewer_shell.html` and `assets/report_shells/`, the synced fixtures under
`test/data/`, and `vendor/hdf5-pure/`. Upstream Python names (`--color_by`, `color_genbank`)
were already American and are unaffected.

Dev/diagnostic examples live in `crates/domainator-core/examples/` (`offsets`, `didx_tool`, `shards_tool`, `matrix_probe`, ...): `cargo run -p domainator-core --example matrix_probe`.

## Verification model (two gates, and they cover different code)

Ported logic is checked by **parity against the original Python tool** — the differential
sweep below is the primary gate for anything with an upstream counterpart.

**`cargo test` is not a lesser second gate — for roughly a third of the workspace it is the
only one possible.** Parity can only ask "is this the same as Python"; everything this port
*adds* has no upstream to compare against, so nothing else can check it:
`crates/domainator-config` (config-file expansion, `--print_config`, the registry's
stdio rules; the manifest golden test checks option shapes, not behavior), `app_support/` (the GUI seam: workspace, provenance DAG,
database registry, HMM logo, contig editor, summary dashboard), and the Tauri command layer.
When adding to any of those, a test is the whole safety net. Two guards there earn their
keep in particular:
- **`describe_matches_upstream_up_to_the_allowlist`** (`crates/domainator-cli/tests/upstream_manifest.rs`)
  — diffs `domainatoredux describe` against upstream's `domainator describe`
  (`scripts/parity/upstream_manifest.json`, refreshed by `refresh_manifest.py`); every
  difference is a row of `scripts/parity/manifest_allowlist.tsv`, enforced both ways
  (`UPDATE_MANIFEST_ALLOWLIST=1` regenerates, keeping classes). `tests/interface.rs`
  holds the generic rules for every tool (registry == subcommands, long == id,
  outputs end `_out`, numbers reject nan/inf, `--print_config` round-trips). Both build
  the CLI's `Command` on a 256 MB-stack thread: the derive overflows a test thread in
  debug builds.
- **`every_offered_tool_is_dispatched`** (`domainatoredux-gui`) — the GUI tool registry and the
  CLI's subcommands are two hand-maintained lists. A Tools-menu job runs
  `<tool> --config <dialog params>` through `domainator_cli::run` in a **worker process**
  (a copy of the GUI executable started with `--domainatoredux-worker`,
  `src-tauri/src/worker.rs`), so there is no per-tool GUI runner to drift from the CLI. A
  menu entry the CLI has no subcommand for fails only when the user presses Run
  (`every_tools_dialog_params_parse_as_its_cli_config` feeds every dialog's default
  params to the CLI with `--print_config`). A job's provenance inputs are derived from
  its spec's `role`/`file_type` (`job_io.rs`), and
  `derived_provenance_roles_match_the_golden_table` keeps the roles
  `trace_source_sequences` follows. `tests/worker.rs` runs the real binary: worker mode
  must never start Tauri, and Cancel must kill the whole process tree.
  `ToolField::role` must be one of `ROLES`, because a role `ToolDialog.svelte` cannot
  render makes the field silently disappear from the form. The registry is the tool
  spec files, `apps/domainatoredux-gui/tools/<id>.yml`, one per tool and embedded by
  `build.rs`, which fails the build on a malformed file. A spec is GUI policy over the
  tool's manifest entry (`domainator_config::tool_manifest`, resolved in
  `tool_specs::specs`): it lists **every** manifest parameter but `-v`/`-q`, with labels,
  an optional `gui_default`, and each input's `sources` (databases or workspace first)
  and `select` (its starting entry). Types, choices, defaults, help, the menu category
  and every field's role come from the manifest.
  `tool_specs.rs`'s tests (`every_manifest_parameter_is_listed_once`,
  `gui_defaults_fit_their_property`) catch a new CLI
  option the GUI never decided about and a GUI default the CLI would reject. When you
  add a tool, write its spec file too. The report column builders (`columns` role)
  render from the manifest's column catalog, the same `columns.rs` catalog the CLI
  parses `--columns` with, and send `name:arg` tokens (`columns.ts`, tested by
  `columns.test.ts`).
- **`storage_format_matches_what_the_readers_require`** and
  **`a_gzipped_fasta_is_installed_as_an_indexed_bgzf_database`** (`domainatoredux-gui`,
  `reference_dbs.rs`) — the managed UniProt/Pfam downloads have no upstream counterpart,
  and every way of getting their storage wrong still puts a plausible file on disk: a
  FASTA kept as plain gzip is unseekable (so `domain_search` cannot use the `.didx`
  written beside it), an HMM library stored as BGZF cannot be opened by HMMER (Pfam is
  kept as EBI's plain gzip, `Pfam-A.hmm.gz`), and an index that disagrees with the BGZF
  blocks fails only inside a later run. The tests walk the real chain — gzip in, BGZF +
  `.didx` out, one record read back at each recorded offset. `refs_are_live` (`#[ignore]`d, hits UniProt and EBI) is the only
  thing that notices a publisher moving a path or changing a release file's shape.
- **`the_managed_diamond_is_new_enough_for_deduplicate`** and
  **`managed_tools_match_the_core_locator`** (`domainatoredux-gui`) — the External tools panel
  must not install a binary older than `MIN_DIAMOND_VERSION`, or the app hands the user a
  diamond that fails and then tells them to upgrade it; and every `ToolSpec`'s
  `bin_name`/`env_var` must equal the core's `*_BIN_NAME`/`*_ENV_VAR`, since the two lists
  live in different crates and a rename on either side installs the binary where nothing
  looks for it — the download reports success and the tool still says "not found".

The rest of the suite (`#[cfg(test)] mod tests` in the owning module) covers the **pure,
deterministic** logic — `BooleanEvaluator`, `protein_analysis` (MW / pI / charge / PROSITE)
against BioPython's documented values, `NCBITaxonomy` (inline mini-taxdmp), the
circular-neighborhood/range math, and the GenBank/HMM writer layouts. Fixture-free where it
can be; the `app_support` file-round-trip tests use a scratch directory under
`std::env::temp_dir()` keyed by pid, never the repo. It needs no conda env, so CI runs it.
The Python reference lives at:
- Python env: `/home/sean/miniforge3/envs/domainator/bin/python`
- Vendored Python source: `external/domainator/` (gitignored; a clone kept at the `UPSTREAM.md` pin), with fixtures under `external/domainator/test/data/` and helpers in `test/helpers.py`.

**Differential parity sweep — the broad gate. Read `docs/parity.md` before adding
tests or chasing a divergence.** `scripts/parity/harvest_invocations.py` mines every
`main([...])` call out of upstream's pytest suite; `scripts/parity/differential.py`
replays each argument list through both implementations into two scratch directories
and diffs every file produced. Upstream's tests are ported as **invocations, not
assertions** — their assertions encode Python-side internals, while the flag
combinations are the part we could never enumerate ourselves. `test/data/` is the
committed fixture subset those invocations reference (kept in step by
`scripts/parity/sync_fixtures.py --check/--update`; do not hand-edit it).

`scripts/parity/known_gaps.tsv` is the **enforced** list of invocations that do not
yet reproduce: a run exits non-zero when reality and that file disagree in either
direction, so a new divergence fails and a fixed one cannot linger as a stale entry.
Membership and `outcome` are machine-maintained; the `class` column
(`deferred:` / `gap:` / `upstream:` / `harness:`) is hand-written and carried across
regenerations, because no script can tell a deliberate deferral from a bug. Needs
conda + the clone, so it is a `sync_upstream.sh` guard, not a CI job; a full sweep is
~6 minutes.

A `tolerated` row means the semantic comparator passed while the **bytes differ** —
worth reading, not just counting. Nine divergences surfaced exactly that way, each
invisible to every other gate because both files read back equal; fixing them took
byte-identical invocations from 51 to 133 (136 once the nucleotide builder landed,
141 with the cd-hit backend -- but note those five only run when cd-hit is actually
installed; see `docs/parity.md`, "External binaries").
See the writer invariants below. Exactly **one open `gap:` row** remains —
`gap:feature-order`, five `domain_search` invocations that emit tied-coordinate
features in the other order from Python; everything else the sweep tracks is a
deliberate deferral or a `hmmer-pure-rs` issue.

**Committed parity harness** (use it before writing an ad-hoc comparison):
- `scripts/json_parity.py EXPECTED ACTUAL [--rtol] [--ignore PATH]` — recursive JSON compare that reports the first differing path. Ignores key *order* (required: `preserve_order` vs `sort_keys`) but not key *spelling*, so `{"1": n}` vs `{"1.0": n}` still fails. Accepts a `.dsnv` directly (gunzips it).
- `scripts/ssn_probe.py MATRIX` — the Python twin of `cargo run -p domainator-core --example ssn_probe -- MATRIX`. Both print the **same** JSON document; keep them in lockstep or the gate silently stops covering a field.
- `scripts/sync_upstream.sh` runs the viewer-shell drift check, `cargo test`, build+clippy, the probe comparison at two `--max_merge_events` caps (which also covers the windowed/pinned selection), the committed-fixture check and the differential sweep, and **exits non-zero** if any guard trips.
- CI (`.github/workflows/check.yml`, **manual dispatch only** — run the checks locally before pushing) runs build (`-D warnings`) + clippy (`-D warnings`) + tests, and a second job clones upstream at the pin to run both drift guards.

Typical gate: run the Rust tool and the Python tool on the same fixture, then compare in the conda env:
- **GenBank**: `helpers.compare_seqfiles` / `compare_seqrecords`. These compare `{seq, id, description, name}`, annotations (iterating the **first** record's keys, skipping `date`), feature **count**, and per-feature **qualifiers** — *not* feature coordinates/types, and not the LOCUS name. (`differential.py` checks those separately for exactly that reason; the origin-crossing slice bugs lived in that blind spot.) Pass `skip_qualifiers={'evalue','score','identity'}` for search output (see numeric tolerance below).
- **HDF5/TSV matrices**: `DataMatrix.from_file(...).toarray()` + `np.allclose` and label/`data_type` checks.
- Records often come back in a **different order** than Python (Rust preserves partition/input order; Python uses a process pool), so compare as a **set keyed by id**, not positionally.

Search numerics need **not** match pyhmmer exactly — `hmmer-pure-rs` is a different HMMER3 implementation. Compare structure (which domains hit which CDS, coordinates, which edges exist) and apply tolerance to `evalue`/`score`/`identity` and matrix values. In practice most gates land at exact equality anyway.

## Architecture

Virtual Cargo workspace: three library/binary crates plus two Tauri apps and one
WASM app.

- **`crates/domainator-core/`** — the data model, all I/O, and all tool logic as library functions (GUI-reusable; mirrors the Python convention that every tool is also a library function). Also holds `app_support/` — the GUI-facing seam (workspace file classification, provenance/job DAG, contig view + edit, HMM logo, summary stats).
- **`crates/domainator-config/`** — the interface: the 1.0 layer (`args/`, `registry.rs`, `manifest.rs`, `config_file.rs`, `columns.rs`, `tool_manifest`).
- **`crates/domainator-cli/`** — thin clap front-end; arg parsing + I/O wiring only, calls into core. Binary name `domainatoredux` (upstream's is `domainator`; the subcommand names are identical).
- **`apps/domainatoredux-gui/`** — the Tauri v2 + Svelte 5 desktop app: manifest-driven tool dialogs, a workspace/job model, and a dozen viewers (contig, SSN, matrix report, enum report, HMM logo, summary dashboard, …). Its Rust side is a thin `#[tauri::command]` layer over `app_support`.
- **`apps/genbank-summary/`** and **`wasm_apps/genbank-summary/`** — a small standalone Tauri app and its browser/WASM twin.

`serde_json` is built with **`preserve_order`** workspace-wide, so JSON object key order
is insertion order. Python writes bundles with `sort_keys=True`, which is why every parity
gate compares *parsed* JSON (`scripts/json_parity.py`) and byte equality is not a contract.

Core module map:
- `seq/` — `SeqRecord`, `SeqFeature`, `Annotations`, and the **lean `Location`** model (`parts: Vec<(start,end,strand,before,after)>`, 0-based half-open, complement-joins pre-reversed). `translate.rs` (genetic code, revcomp, location extraction), `taxid.rs`, `taxonomy.rs` (`NCBITaxonomy` taxdmp parser + lineage/rank/name + `compile_taxonomy_filter`/`compile_taxonomy_expression`; reads extracted `.dmp` or `taxdump.tar.gz`), `protein_analysis.rs` (ProtParam isoelectric point / net charge, `molecular_weight`, PROSITE `motif_count` via the `regex` crate).
- `io/` — own GenBank `parse`/`reader`/`writer`/`offsets`, `fasta`, `bgzf` (random access + virtual offsets), `didx` (`.didx` index), `shards`, `partition`.
- `search/` — `hmmer` (hmmsearch for domainate), `seq_compare` (phmmer/hmmsearch scoring), `hmm_compare` (profile-vs-profile Viterbi), `results` (`SearchResult` → GenBank feature).
- `matrix/` — `DataMatrix` (dense/CSR) + `hdf5_io` (upstream's matrix file format 2.0: one `write`, canonical CSR stored as its upper triangle (`SPARSE_CSR_UPPER_TRIANGLE`) when the labels are symmetric and the values exactly so, dense HDF5 never written; reads majors 1 and 2 and mirrors a triangle back. Symmetric labels are inferred from `COL_LABELS` **presence** on read — the actual on-disk contract, robust even if the `SYMMETRIC_LABELS` attr is absent. The vendored `hdf5-pure` was also patched to decode enum attributes so h5py's numpy-bool `SYMMETRIC_LABELS` is no longer silently dropped — see `vendor/hdf5-pure/CHANGELOG.md`), `text` (TSV read+write), `transform` (`--mode`s + `apply_lower_bound`), `mst_knn` (MST∪kNN pruner; shares `UnionFind` and `mst_knn_kept` with `ssn`. Tied MST edges follow the tool's upstream counterpart: `TreeTies::Batch` (row-major index descending, `transform_matrix`/`build_ssn`) or `TreeTies::Streaming` (`seq_dist`/`compare_contigs`); kept edges are ordered MST first, then kNN), `hierarchy` (scipy's `linkage`/`leaves_list`/`optimal_leaf_ordering`, ported kernel for kernel -- single precision and the unstable quicksort included -- because `compare_contigs --genbank_out`'s order turns on exact ties; `examples/hierarchy_probe` prints what a scipy script can diff against), `cytoscape` (XGMML writer), and `ssn` (the SSN engine: `MaxTree` + `component_size_summary_by_threshold`/merge events + `build_mst_component_hierarchy` + `edge_scores_desc`/`mst_knn_edge_list` + `merge_event_moving_sum`/`filter_merge_event_rows_binned` (with the viewer's optional zoom `window` + `pinned_threshold`: the window narrows the top-N pool only, never the band back-fill)/`threshold_slider_stops` (`with_threshold_index` — only `matrix_report` has threshold tables to index; `MaxTree::from_mst_edges` rebuilds a tree from a bundle's stored merge order and has none)/`floor_threshold_value`, mirroring `ssn_edges`/`ssn_hierarchy`). **Threshold tables carry one row per distinct threshold**, and each row describes exactly the graph `build_ssn --lb T` keeps — edges scoring *strictly above* T — with a trailing synthetic `threshold = 0` row when every score is positive. Keying rows by MST edge position instead was upstream bug `9ab625f`. `read_matrix(path)` sniffs HDF5-vs-TSV by extension like Python's `DataMatrix.from_file`. MST edge ordering matches the Python **dense** `sorted_undirected_edges` fast path (score desc, ties by row-major index desc).
- `tools/` — `domainate`, `domain_search`, `seq_dist`, `compare_contigs`, `hmmer_compare`, `transform_matrix` (mode/threshold/`--mst`/`--knn`/`--symmetrize` + dense↔sparse conversion), `build_ssn` (XGMML SSN: default/`--mst`/`--knn` edges, `--mark_mst`, connected-component clustering, TSV metadata, coloring), `build_ssn_viewer` (the O(n) gzip-JSON **`.dsnv`** bundle, schema version 6 = nodes + MST + hierarchy + `merge_impact_metric` and **nothing else**: the split-event series, its moving sum and the slider stops are a pure function of those, so readers derive them — `ssn_bundle`'s `merge_event_rows`/`merge_event_series`/`moving_sum`/`slider_stops` and the shell's JS ports — rather than inherit a cap chosen at build time, which is why `--max_merge_events` lives on `ssn_navigator`/`matrix_report` and the viewer's "Split events" box, not on `build_ssn_viewer`. v3/v4/v5 files still load, and their stored derived keys are deliberately ignored; `.ssnv` is the pre-0.11 spelling, still read but never written, plus the vendored HTML shell in `crates/domainator-core/assets/ssn_viewer_shell.html` filled via string tokens (`@@SSN_VIEWER_TITLE@@`, `@@SSN_VIEWER_HEADING@@`, and the `"embeddedBundleBase64": null` key of its `dsnv-config` JSON block) — that shell is **generated** from Python's `ssn_viewer_html.py` by `scripts/regen_ssn_viewer_shell.py`, never hand-edited; see "Staying in sync" below), `matrix_report` (text report + the agent `--json_out` contract, plus a separate `to_viewer_json` for the GUI tab that is deliberately *not* parity-locked; `--include_mst_knn`/HTML deferred), `summary_report` (JSON, text, and the HTML page's histogram and taxonomy tree), `ssn_bundle` + `ssn_navigator` (read-only `.dsnv` navigation), `neighborhood` (the `select_by_cds` geometry, shared by `domain_search`'s hit path and the standalone `select_by_cds`), `select_by_contig`, `select_by_cds`, `enum_report`, and `select_common` (shared `DomainatorCds.list_from_contig`, `BooleanEvaluator`, `get_sources`/`get_non_domainator_features`, name-list parsing, the `--databases` rollup filter). `seq_dist`'s `diamond_*` algorithms shell out to the external **`diamond`** binary: build a db from the reference, then `blastp --outfmt 6 qseqid sseqid bitscore --algo 0 --max-hsps 1`, with the hit table streamed line-by-line into the per-cell max map (never a temp output file). The binary is located via `$DOMAINATOREDUX_DIAMOND`/`$DIAMOND`, then a bundled `external/diamond/diamond` (searched upward from the exe, then cwd), then bare `diamond` on `PATH`. The normalized query/db FASTA + db live in a self-cleaning temp dir.
- **External binaries** are the only non-pure-Rust runtime dependencies, all optional and all driven as subprocesses through `tools/external.rs`'s `find_binary` (env var → bundled `external/<dir>/<name>` → `PATH`): **`diamond`** for `seq_dist` and `deduplicate_genbank`, **`cdhit`** and **`usearch`** for `deduplicate_genbank`, and **`orphos`** (a Rust Prodigal) for gene calling — `tools/gene_call.rs`, behind the port-only `gene_call` tool and `domainate`/`domain_search --gene_call`. Their locator identity lives in `deduplicate_genbank`'s and `gene_call`'s `*_BIN_NAME`/`*_ENV_VAR` constants, which the GUI's External tools registry is pinned to by a test. cd-hit, usearch and Orphos are GPL: driving them across a process boundary, downloaded at runtime and never bundled, is what keeps this project's licensing unaffected — do not link or vendor any of them (that includes the `orphos-core` crate and every other Prodigal implementation). Orphos is read only through its GFF (`-f gff`; its GenBank output lacks strand and partial markers, it has no protein output, `-t` is a no-op), so `gene_call.rs` rebuilds what `prodigal_CDS_annotate` derives from a pyrodigal gene: edge-extended partial locations with `/codon_start`, the translation (start codon → `M`, trailing `*`, in the table each sequence's model reports — meta mode can pick table 4), and circular contigs' origin-crossing genes from a second "origin window" sequence in the same run. The `--gene_call` parity rows are `extra:` invocations and need `$DOMAINATOREDUX_ORPHOS` set, or the sweep fails on them. `deduplicate_genbank`'s cd-hit backend targets the **`cdhit` Rust port**, one multiplexer binary whose first argument is the program name; `cdhit_command` runs a binary already *called* `cd-hit`/`cd-hit-est` directly instead, and when that name cannot serve the request (protein `cd-hit` on nucleotide input) it prefers the sibling executable and errors rather than silently clustering DNA as protein. `cd-hit` switches to `cd-hit-est` on the **first record's** molecule type, `-n` is derived from `--id` via upstream's two word-size tables, and `-d 0` is load-bearing: cluster membership is read back out of the `.clstr`, whose names it stops cd-hit truncating.
- `log.rs` — stderr messages at upstream's levels: `log::info` (progress, shown by default), `log::warn` (hidden by `-q`), `log::debug` (`-v`); the CLI sets the level after parsing. Use these, never a bare `eprintln!`, so `-q` means errors only.
- `parallel.rs` — `with_threads(cpu, f)` wrapper (`cpu == 0` → `resolve_cpu`: one thread per physical core the process may use, as upstream). Tools parallelize with rayon over their natural unit (profiles / references / partitions / rows); output is **deterministic** regardless of thread count (results funnel through order-independent maps/sorts).

Key cross-cutting invariants (these gate parity — grep the modules for details before changing):
- **Sequence identifiers:** `id` = LOCUS name for GenBank, first header word for FASTA; `name` retains VERSION (or ACCESSION) for GenBank round-tripping. User-facing labels, coordinate suffixes, matrix axes, and `source_contig` use `id`. The Python parser's id/name swap is not an internal Rust constraint.
- **Ordered qualifiers** (`Vec<(String, Vec<String>)>`) — insertion order is load-bearing for the writer and for `cds_id` / `domainator_<db>` rollups.
- **GenBank writer: `/translation` is the only qualifier that wraps.** Upstream patched its vendored Biopython so `/translation` wraps at 80 columns (the 21-space indent included, so a wrapped line is exactly 80) and every other qualifier gets `MAX_WIDTH = 10000000000000` — one line however long (`Bio/SeqIO/InsdcIO.py::_write_feature_qualifier`). Wrapping the rest still *reads back* equal, which is why parity gates never saw it, and it is what made breaking a long no-space token hazardous (the reader rejoins the halves with a spurious space). Three sibling rules in the same function: a qualifier is written **unquoted only if its key is in upstream's `FTQUAL_NO_QUOTE`** list (values are always strings here, so "looks like a number" is not the rule — that strips the quotes off `/gc="49"`); the LOCUS topology column is `annotations["topology"]` and stays **blank when the record never declared one**, not `linear`; and the SOURCE consumer drops one trailing `.` that the writer does not put back, so the ubiquitous `SOURCE      .` round-trips to a bare `SOURCE` line.
- **GenBank writer: a record with no `date` annotation is stamped with today's date**, not Biopython's `01-JAN-1980` fallback — `utils.write_genbank` sets `date.today()` before handing the record to the writer, so a record a tool just built is dated when it was made. Ours uses UTC (no timezone database, no date dependency); Python uses local time, so the two disagree only across local midnight; every parity comparator skips `date`, and `differential.py` runs Python with `TZ=UTC` so the bytes agree too.
- **Compressed `.hmm`** (upstream `utils.open_hmm_file`, `eb2d524`): every hmm read goes
  through `search/hmm_read.rs`/`io::open_decompressed`, which decide gzip vs plain from the
  **magic bytes**, never the name, via `MultiGzDecoder` — the crate's own opener keys on a
  `.gz` extension with a single-member decoder, which reads only the first block of a BGZF
  file and silently drops the rest. `hmmpress` sidecars are never read. Output goes through
  `HmmOutput`/`write_hmm_output`: `.gz` is **plain gzip**, `.bgz`/`.bgzf` is **refused**
  (HMMER cannot open BGZF), and a failed run removes its partial file. Database names use
  `io::db_name_from_path`, so `refs.hmm.gz` labels annotations `refs`, not `refs.hmm`.
- **HMM writer** (`search/hmm_write.rs`, not the `hmmer-pure-rs` one): `HMMER3/f`, a nine-column model-matrix grid, and the `GA`/`TC`/`NC` Pfam cutoffs — the crate's writer drops those, which breaks `hmmsearch --cut_ga` downstream. Its reader also mis-parses them (Pfam's trailing `;`), so read profiles through `search/hmm_read.rs`. `hmmer_select` reproducing its input byte for byte is the gate. **All five annotation columns are written on every match line**, `-` for the ones a profile lacks — MAP included, whose column is present even under `MAP no`: C's reader consumes that token unconditionally and only *stores* it when the flag is set, so omitting it shifts CONS/RF/MM/CS left and makes the file unreadable. Every fixture is `MAP yes`, so no parity gate covers this; the `hmmer_select` round-trip tests are the guard.
- **Number formatting** (`%.1e` via a custom `py_sci`, `%.1f`, NumPy `str(float64)` for TSV) is replicated to match Python output exactly.
- **`.didx` / shard resolution / BGZF offsets** are hard on-disk contracts (`io/didx.rs`, `io/shards.rs`, `io/bgzf.rs`): `.didx` is byte-compatible; staleness is rejected via size+mtime_ns; both unsharded + shards present is an error (refuse to guess).
- **Parser must never reject a record** (handles odd LOCUS layouts, missing molecule unit, `*` stop codons).
- **GenBank DEFINITION**: the reader strips a single trailing `.` (Biopython does), the writer re-appends it — so `record.description` matches `parse_seqfiles` and round-trips.
- **A forward window that wraps the origin keeps its parts in rotation order.** `build_slice_from_window` lists `[lo0, L) + [0, rem)` — the order the bases are read in, and the order Python concatenates `location.parts`. Sorting ascending yields the same arc rotated to a different origin (a silently wrong sequence) *and* makes the region read as starting at the contig's origin, which drops the `_first:last` suffix that marks the new one. A **reverse** wrapped window is the mirror image and does want ascending order, since each part is reverse-complemented before concatenation.
- **`--max_hits` ranks by the focus CDS's hit score**, Python's `hit_best_score` — the `Domain_Search` best-hit feature only, never the `Domainator` annotations. An extracted region spans the focus CDS *and its neighbors*, so ranking by the maximum over all its features scores a weak hit by whatever strong gene sits next to it. Ties keep input order (a stable sort), as upstream's `(score, -arrival)` heap does since 1.0.
- **Every extracted region is named by `neighborhood::region_id`**, upstream's `model.region_id` (`Loc::region_id` for the extract tools): `{id}_{first}:{last}` plus `rc`, where `first`/`last` are the *human-readable* stranded start of `parts[0]` and stranded end of `parts[-1]` — **not** a hand-rolled `start + 1 : end`, which is the forward rule and is off by one in each direction on a reverse slice (upstream `2de50d8`/`f0741bc`). A region flipped back to the parent's direction (`--keep_direction`) passes `flip`, which swaps them and drops `rc`; the whole contig from its own origin is `{id}` or `{id}_rc`. Only `record.id` (LOCUS) changes, never `name`/ACCESSION. Do not grow a second naming rule in a tool.
- **Slicing across the origin remaps into *every* overlapping slice part, then merges.** When a window wraps a circular contig the slice has two parts, and one feature part can reach into both; each match contributes a piece (Python's `overlap[i]` is a set, not a first match). The pieces then go through `merge_touching_parts` — Python's `CompoundLocation.merge_parts` — which coalesces same-strand parts that touch *inclusively*, so the halves a junction cut (`[0,130)` + `[130,840)`) become one range instead of a spurious `join`. Merging is gated on the original operator (which is kept, not forced to `join`), so `order` locations keep their parts apart; pieces are listed in order *along the feature* and never sorted. There is **one** slicer, `neighborhood::slice_features` — every tool reaches it through `slice_record`, `slice_linear_features` or `extract_common::slice_from_location`; do not grow a second. Note `helpers.compare_seqfiles` compares feature *count* and *qualifiers* but **not coordinates or types**, so a parity gate alone will not catch a regression here — the `slice_record` tests are the guard.
- **A slice truncates what it cuts and marks the cut ends** (upstream `8bf3c42`, `slice_record_from_location` with `truncate_features=True`, now the only mode). A feature overlapping the slice is kept, truncated; an end the slice really cuts gets `<`/`>` (`before`/`after`), except on `source`; an uncut end keeps its original marker; a cut `Domainator`/`Domain_Search` is **dropped**. "Really cut" is decided on intervals along the feature, so one split across a rotated origin is not cut and gets no markers. A cut nucleotide CDS gets a recomputed `/codon_start` (written when ≠1 or already present) and a `/translation` trimmed to the fully kept codons, or removed if the kept part is not contiguous. `merge_touching_parts` keeps the markers of the ends it takes. `before`/`after` always mean the **low/high coordinate**, so a minus-strand 5' end is `after`; a reverse slice part swaps them. Fragment status and `--partial` live in `seq/partial.rs`. Gene-caller `/partial="10"` codes are turned into markers (and removed) **on read**, at the end of `parse_record` (upstream `0a8ba4c`), so no tool ever sees one.
- **`/codon_start` is the reading frame everywhere**: protein hits are placed at `codon_offset + start*3` (`domainate::overlay`, which returns `None` where Python's `overlay` raises), missing translations start at that offset, and `extract_peptides` maps positions as `max(0, (p - offset) // 3)`.
- **`domainator_<db>` rollup is regenerated by the writer** (`write_record`) from the CDS's `Domainator` features *in current feature order*, then features are sorted by `location.start` for output — mirroring `utils.write_genbank` (rollup built before its sort). `slice_record` therefore must **not** pre-sort its features (it preserves the input order so the rollup ordering matches), and `domainate` appends domains in score-descending order. This is why pass-through tools (`select_by_*`) reproduce the original's rollup string.

### Vendored `hdf5-pure` (patched dependency)

`vendor/hdf5-pure/` is a local copy of `hdf5-pure` 0.20.0 wired via `[patch.crates-io]` in the root `Cargo.toml`. It carries two additive patches (see `vendor/hdf5-pure/CHANGELOG.md`): (1) `AttrValue::VarLenUtf8(String)` — a scalar variable-length UTF-8 string attribute so h5py reads the matrix attrs (`ARRAY_TYPE`/`DATA_TYPE`/`MATRIX_FILE_VERSION`) back as `str` (the published crate only offers fixed-length strings, which h5py returns as `bytes`); and (2) enum attributes decode to their underlying integer instead of being silently dropped from `attrs()` — h5py stores a numpy-bool attribute (like `SYMMETRIC_LABELS`) as an enum over `int8`. Matrix `SYMMETRIC_LABELS` is written as an int (h5py reads it truthily); on read, symmetry is determined by `COL_LABELS` presence (the on-disk contract), independent of that attr.

### `hmmer-pure-rs` pinned to a fork commit (temporary, pending an upstream merge)

`hmmer-pure-rs` is pinned by `rev` to a commit on **our fork**,
`seanrjohnson-neb/hmmer-pure-rs` branch `aarch64-neon-pipeline` (a second
`[patch.crates-io]` entry; `deny.toml` allows that git source). The branch is the official
`main` (0.8.0, which includes `henriksson-lab/hmmer-pure-rs#2`, the single-sequence
nucleotide builder the nhmmer FASTA path needs) plus one commit that runs the SSE kernels on
**aarch64** through a NEON implementation of the SSE2 intrinsics (`hmmer_pure_rs::simd::sse`).
Without it, Apple Silicon builds fall back to generic DP everywhere: hmmsearch ~25x slower,
~1 domain in 8 with a shifted envelope, and no nhmmer long-target filter at all. The PR
to the official repo is prepared but not opened. Don't "clean up" the pin and don't move it
to a branch head (a later commit would silently change CI). `UPSTREAM.md`'s "Pinned
dependency" section has the re-pin and migration-back steps.

**ARM is testable here** under QEMU user mode (`aarch64-unknown-linux-gnu`; aarch64 Linux
takes the same `cfg(target_arch)` paths as macOS). The SSE kernels are meant to give
byte-identical output on both architectures, so compare ARM against x86, not just against
tolerance.

## Staying in sync with the Python package

See `UPSTREAM.md` for the full workflow. Key points:
- **Pin.** `UPSTREAM.md` records the upstream commit the port is caught up to; `external/domainator/` (gitignored) is a git clone kept at that pin — the repo-local source for regeneration and drift checks.
- **The SSN viewer shell is a generated artifact, never hand-edited.** `crates/domainator-core/assets/ssn_viewer_shell.html` is the render of Python's `ssn_viewer_html(title='@@SSN_VIEWER_TITLE@@', embedded_bundle_json=None)` **plus** one local patch (the "GUI host hook" that `postMessage`s node selection to the Tauri host — load-bearing for `SsnView.svelte`). Regenerate with `scripts/regen_ssn_viewer_shell.py`; `--check` is the drift guard.
- **So are the `--html_out` report shells**, `crates/domainator-core/assets/report_shells/<page>.html`: upstream's `report_html.report_shell(page)` with no local patches, written by `scripts/regen_report_shells.py` (`--check` is the drift guard) and filled by `tools/report_html.rs` (`@@REPORT_TITLE@@`, `@@REPORT_DATA@@` = the `report-data` envelope, `@@REPORT_ROWS@@` = a table's NDJSON rows, `<` escaped as `\u003c`). The page's JS is upstream's; a report's port is only its payload. The viewer shell and these are the **only** literal-copy artifacts; everything else is hand-ported logic under parity gates + fixture-free `cargo test`s.
- **The GUI reuses the same shell** via `render_html`'s `include_str!` — no separate copy — so viewer changes reach the GUI on a rebuild.
- **On-disk format changes** on the Python side (`.dsnv` bundle, HDF5, GenBank, XGMML, TSV) require parallel Rust changes in the owning module (the treemap did **not** — it is pure client-side JS on the existing bundle).
- **Routine:** `scripts/sync_upstream.sh` fast-forwards the vendored copy, prints the `<pin>..<target>` changelog to triage against `UPSTREAM.md`'s file→owner map, runs the guards, and reminds you to advance the pin.

## Scope / status

The approved plan is at `/home/sean/.claude/plans/make-a-plan-for-kind-brooks.md`. The prioritized SSN pipeline (README steps 1–3) is complete and Python-verified: GenBank/FASTA I/O, `domainate`, `domain_search` (shards/BGZF/`.didx`, `-Z 0`, neighborhood extraction for linear **and** circular contigs, `--translate`, strand filter, overlap dedup), `seq_dist` (DIAMOND `diamond_*` via the external binary — now the default `--algorithm`, exact-parity-verified against the Python tool when both use the same diamond version; plus hmmsearch / phmmer / hmmer_compare + dense/sparse/TSV), `compare_contigs` (JI/AI), the standalone `hmmer_compare` (with alignments), and rayon multithreading.

Also complete and Python-verified: `select_by_contig`, `select_by_cds` (standalone CDS selection by name/domain/`--domain_expr`/`--search_hits`/`--unannotated`, neighborhood extraction reusing `neighborhood.rs`, `--pad`/dedup/`--strand`/`--invert`/`--include_nucleic_acids`), and `enum_report` (TSV; all `--by contig|cds|domain` columns including taxonomy, protein chemistry, and PROSITE motif). Parity gates: `select_by_contig` 21/21, `select_by_cds` 27/27, `enum_report` 27/27 (full `compare_seqfiles` / exact TSV equality).

**SSN building & viewing (README step 4 data path)** — complete and Python-verified against
`FeSOD_dist.{dense,sparse}.hdf5`/`.tsv`: `transform_matrix` (all modes incl. `efi_score`,
dense↔sparse, `--mst`/`--knn`; `np.allclose` + byte-identical `dense_text`), `build_ssn`
(XGMML byte-identical for default/`--mst`/`--knn`/`--mark_mst`/`--cluster`/metadata/coloring,
as are `--cluster_tsv_out` and `--color_table_out`), `build_ssn_viewer` (`.dsnv` bundle field-for-field
identical after gunzip, and accepted by Python's own
`ssn_bundle.load_bundle`), `matrix_report` (`--json_out` byte-identical to Python's; text report
line-for-line identical apart from the histogram), `ssn_navigator` (all five modes, against both
implementations' bundles), and `summary_report` (`--json_out` byte-identical incl. co-occurrence and
`--databases`). The SSN core is cross-checked field for field by
`scripts/sync_upstream.sh`'s `ssn_probe` guard.

The one gate worth knowing: threshold rows are proven against `build_ssn --threshold` end to end —
edge count *and* component count, sampled across the table, **on a matrix with deliberate tie
groups**. Ties are exactly where the old per-MST-edge keying was wrong, and a probe-vs-probe
comparison alone would happily reproduce the same misunderstanding on both sides.

Deferred (per plan / no clean path, all tracked in `UPSTREAM.md`): Infernal/ESM, the
`structure_*` subsystem (portable — foldseek/reseek subprocess, no torch — but out of scope;
`SearchResult` already carries its four optional metrics), HTML output for `matrix_report`
(JSON offered instead; the other reports have theirs), `summary_report`'s text layout, `matrix_report`'s
`--knn_projection` (it only feeds the HTML report), and `hmmer_build --window_beta`.
