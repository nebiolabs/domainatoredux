# Developing Domainator

Contributor-facing notes. For using the tools, start at
[Getting started](getting-started.md).

## Build and test

```bash
cargo build                      # debug build of the workspace
cargo build --release            # optimized (lto); use for any timing or benchmarks
cargo test                       # the unit suite; no conda or clone needed
./target/debug/domainatoredux <tool> [opts]
```

**The project holds itself to zero warnings, from `rustc`, clippy and rustdoc, with
rustfmt-clean formatting:**

```bash
cargo fmt --all --check                                      # must be clean
cargo build --workspace 2>&1 | grep -c warning:              # must be 0
cargo clippy --workspace --all-targets -- -D warnings        # must be clean
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace   # must be clean
```

Lint levels live in the root `Cargo.toml`'s `[workspace.lints]` (every member opts in
with `[lints] workspace = true`), so these commands enforce the same rules CI does. The
policy adds `unsafe_op_in_unsafe_fn`, `unused_qualifications`, `rust_2018_idioms` and
clippy's `undocumented_unsafe_blocks` (every `unsafe` block carries a `// SAFETY:`
comment). The cast lints (`cast_possible_truncation` and friends) are not on yet: they
flag ~245 sites in the offset and coordinate math, each needing a decision.

Three whole-repo checks need their tools installed (`cargo install cargo-deny
typos-cli`), plus the frontend type checks:

```bash
cargo deny check      # deny.toml: license allowlist, RustSec advisories, git sources
typos                 # _typos.toml: spelling, including the American-English rule
mdbook build          # the docs book; a SUMMARY.md entry with no file fails
(cd apps/domainatoredux-gui && npm run check)                  # svelte-check
(cd apps/domainatoredux-gui && npm test)                       # vitest (tool-dialog file placement)
(cd apps/genbank-summary && npx tsc --noEmit)
(cd wasm_apps/genbank-summary && npx tsc --noEmit)
```

`--all-targets` matters: `cargo build` does not compile `#[cfg(test)]` code, so a broken
test-only initializer is invisible to the build alone. Where a lint is genuinely wrong for
this codebase, silence it with a narrow `#[allow(...)]` **and a one-line reason** — never a
crate-wide allow.

Diagnostic examples live in `crates/domainator-core/examples/`:

```bash
cargo run -p domainator-core --example matrix_probe
cargo run -p domainator-core --example didx_tool
```

## The two verification gates, and why both exist

**1. Parity against the Python tool** is the primary gate for anything with an upstream
counterpart. `scripts/parity/harvest_invocations.py` mines every `main([...])` call out of
upstream's pytest suite and `scripts/parity/differential.py` replays each argument list
through both implementations, diffing every file produced. Upstream's tests are ported as
*invocations, not assertions* — their assertions encode Python-side internals, but the flag
combinations are the part we could never enumerate ourselves.

`scripts/parity/known_gaps.tsv` is the **enforced** list of invocations that don't yet
reproduce: a run fails when reality and that file disagree in either direction, so a new
divergence fails the gate and a fixed one can't linger as a stale entry.

Needs conda and the upstream clone, so it is a `sync_upstream.sh` guard rather than a CI
job; a full sweep takes about six minutes. **Read [the parity harness](parity.md) before
adding tests or chasing a divergence.**

**2. `cargo test` is not a lesser gate** — for roughly a third of the workspace it is the
only one possible. Parity can only ask "is this the same as Python"; everything this port
*adds* has no upstream to compare against:

- `crates/domainator-config` — config-file expansion, `--print_config` and the registry's
  stdio rules (the manifest golden test checks option shapes, not behavior);
- `app_support/` — the GUI seam: workspace, provenance DAG, database registry, HMM logo,
  contig editor, summary dashboard;
- the Tauri command layer.

When adding to any of those, a test is the whole safety net. A few guards earn their keep
in particular:

- **`describe_matches_upstream_up_to_the_allowlist`** (`domainator-cli`) — diffs
  `domainatoredux describe` against upstream's own manifest; every difference must be a
  row of `scripts/parity/manifest_allowlist.tsv`, and every row a real difference. See
  [Parity](parity.md).
- **`every_documented_command_parses`** (`domainator-cli`) — parses every
  `domainatoredux` command in the fenced blocks of `docs/` and `README.md`, so an example
  using a renamed option or a dropped choice fails here rather than for a reader.
- **`every_offered_tool_is_dispatched`** and
  **`every_tools_dialog_params_parse_as_its_cli_config`** (`domainatoredux-gui`) — GUI jobs
  run `<tool> --config <params>` through the CLI in a worker process, so a menu entry
  the CLI does not know, or a dialog whose params the CLI rejects, fails only when the
  user presses Run. The tests feed each tool's dialog params to `domainator_cli::run`
  with `--print_config`.
- **`derived_provenance_roles_match_the_golden_table`** (`domainatoredux-gui`) — a job's
  provenance inputs are derived from its tool spec, and `trace_source_sequences` follows
  their roles. A spec edit that changes a role breaks SSN node → sequence tracing without
  any error.
- **`tests/worker.rs`** (`domainatoredux-gui`) — runs the real binary in worker mode: it must
  never start Tauri, Cancel must kill the whole process tree, a worker must exit when the
  GUI's end of its stdin closes, and a worker run must produce the CLI's output.
- **`the_managed_diamond_is_new_enough_for_deduplicate`** and
  **`managed_tools_match_the_core_locator`** (`domainatoredux-gui`) — the External tools panel
  must not install a binary older than `MIN_DIAMOND_VERSION`, and every `ToolSpec`'s
  `bin_name`/`env_var` must equal the core's constants, since a rename on either side
  installs the binary where nothing looks for it.

The rest of the suite covers the pure, deterministic logic — `BooleanEvaluator`,
`protein_analysis` against BioPython's documented values, `NCBITaxonomy`, the
circular-neighborhood maths, and the GenBank/HMM writer layouts — fixture-free wherever
possible.

## Committed parity harness

Use these before writing an ad-hoc comparison:

- `scripts/json_parity.py EXPECTED ACTUAL [--rtol] [--ignore PATH]` — recursive JSON
  compare reporting the first differing path. Ignores key *order* but not key *spelling*.
  Accepts a `.dvb` or `.dsnv` bundle directly.
- `scripts/ssn_probe.py MATRIX` — the Python twin of
  `cargo run -p domainator-core --example ssn_probe -- MATRIX`. Both print the **same**
  JSON document; keep them in lockstep or the gate silently stops covering a field.
- `scripts/sync_upstream.sh` — runs the viewer-shell drift check, `cargo test`,
  build + clippy, the probe comparison at two `--max_merge_events` caps, the
  committed-fixture check and the differential sweep, and **exits non-zero** if any guard
  trips.

`test/data/` is the committed fixture subset the parity invocations reference, kept in step
by `scripts/parity/sync_fixtures.py --check/--update`. **Do not hand-edit it.**

The Python reference lives at `external/domainator/` (gitignored; a clone kept at the pin
recorded in
[UPSTREAM.md](https://github.com/nebiolabs/domainatoredux/blob/main/UPSTREAM.md)), with a
conda env at `/home/sean/miniforge3/envs/domainator/bin/python` on the author's machine.

## CI

`.github/workflows/check.yml` runs six jobs. It is **manually dispatched** (the Actions
tab, or `gh workflow run check.yml [--ref <branch>]`), not triggered by pushes or PRs:
run the same checks locally before pushing (the commands under "Build / run", plus
`scripts/sync_upstream.sh` for the upstream guards). The jobs:

- **`rust`**: rustfmt, then build, clippy and rustdoc (all `-D warnings`) and tests for
  the pure-Rust crates.
- **`gui`**: installs WebKitGTK, builds both frontends (`tauri::generate_context!` needs
  `dist/` at compile time), then runs workspace-wide clippy and rustdoc and the two apps'
  tests. This is the only CI run of the GUI's registry, tool-spec and reference-database
  guards.
- **`frontend`**: `svelte-check --fail-on-warnings` and `npm test` (vitest) for the GUI, `tsc --noEmit` for both
  genbank-summary frontends, and `npm audit` on shipped (non-dev) dependencies.
- **`policy`**: `cargo deny check`, `typos` (pinned to an exact release) and `mdbook build`.
- **`msrv`**: `cargo check` of the pure-Rust crates at the `rust-version` in `Cargo.toml`.
- **`upstream-drift`**: clones upstream at the recorded pin to run the viewer-shell drift
  guard and the SSN core probe comparison.

Packaging is two more workflows, both manually dispatched too (pushing a `v*` tag does
not start them):

- **`build-release.yml`**: the desktop app (macOS `.dmg`, Windows NSIS installer plus a
  portable `.exe`, Linux `.deb`/AppImage) and the `domainatoredux` CLI binary for the same
  three platforms. The dispatch form has checkboxes for the products (GUI, CLI) and the
  platforms (macOS, Windows, Linux), all ticked by default; a `setup` job filters the
  build matrices to the ticked boxes, so an unticked target never starts a runner. From
  the command line, untick with `-f`:
  `gh workflow run build-release.yml --ref main -f linux=false`. It fails the run when
  dispatched on a tag other than `v` + `workspace.package.version` (on a branch that
  check is skipped).
- **`build-genbank-summary.yml`**: the standalone genbank-summary app (macOS `.dmg`,
  Windows NSIS). It is not part of a release; build it when you want it.

Releases are cut by hand from the artifacts.

### GPU builds

`find_features --tmbed` runs its protein language model with candle
(`crates/domainator-plm`): on the CPU in every build, and on a GPU in a build with the
`cuda` or `metal` feature (forwarded by core, the CLI and the GUI crate). macOS release
builds have `metal`. CUDA builds are separate artifacts, `*-cuda`, made when the release
workflow's `cuda` box is ticked; they need an NVIDIA driver for CUDA 12.4 or newer (550+)
and run anywhere else on the CPU, as every build does.

A local CUDA build needs, besides the NVIDIA driver:

- an `nvcc` **no newer than the driver's CUDA version** (`nvidia-smi` shows it): candle
  compiles its kernels to PTX, and a driver older than the toolkit refuses to load it
  (`CUDA_ERROR_UNSUPPORTED_PTX_VERSION`). candle-kernels' build script does not notice a
  different `nvcc`, so delete `target/*/build/candle-kernels-*` after switching;
- a host C++ compiler the toolkit supports (gcc 13 for CUDA 12.4, via `NVCC_CCBIN`), whose
  headers it can parse: CUDA before 12.9 fails on glibc 2.41's `sinpi`/`cospi`
  declarations, so on a newer distribution use a conda-forge `gxx` (an older sysroot);
- `CUDA_COMPUTE_CAP` (75 is Turing; newer GPUs JIT its PTX) when building on a machine
  without the GPU, and cuBLAS, cuRAND and NVRTC (12.x) on the library path, for linking
  and at run time.

```bash
CUDA_COMPUTE_CAP=75 NVCC_CCBIN=/path/to/x86_64-conda-linux-gnu-g++ \
  cargo build --release -p domainator-cli --features cuda
```

## Versioning and releases

The port has **its own SemVer line**, independent of the Python package's numbers. The
Python release it reproduces is recorded separately and reported next to it.

- **Our version** is `workspace.package.version` in the root `Cargo.toml` and nowhere else.
  The CLI, both Tauri apps (their `tauri.conf.json` has no `version` key, so the bundler
  reads Cargo's), the About box and job provenance (`engine_version`) all take it from
  there. A test in each app fails if a `version` key comes back.
  - **MAJOR:** a breaking change to our CLI flags, config schema or on-disk output.
  - **MINOR:** new tools or features, including an upstream catch-up that adds features.
  - **PATCH:** fixes only.
- **Upstream parity** is `UPSTREAM_VERSION` (upstream's `__version__` at the pin) and
  `UPSTREAM_COMMIT` (the pin itself) in `crates/domainator-core/src/version.rs`. They
  appear in `domainatoredux --version`, the About box, the diagnostics blob and every job's
  provenance (`upstream_version`). `UPSTREAM_VERSION` is also what a `.dvb` bundle's
  `domainator_version` key holds, because upstream reads that key as "which domainator
  release wrote this". Three guards keep the constants honest:
  - a unit test that `UPSTREAM_COMMIT` equals `UPSTREAM.md`'s pin;
  - a unit test that `UPSTREAM_VERSION` equals the version baked into the regenerated
    viewer shell;
  - a `sync_upstream.sh` guard comparing `UPSTREAM_VERSION` with the vendored
    `__version__`.
- **Tags** are `vX.Y.Z`, one per release, covering the CLI and both apps. The release packaging
  workflow refuses a tag that differs from `v` + `workspace.package.version`. (The
  `v0.11.0` tag predates this scheme; the first release under it is `v1.0.0`.)

To cut a release:

1. Bump `workspace.package.version` in `Cargo.toml`.
2. Add an entry to `CHANGELOG.md` that names the upstream version and commit it is at
   parity with.
3. Commit, tag `vX.Y.Z` and push the tag, then dispatch the packaging workflow on it
   with every box ticked (the default): `gh workflow run build-release.yml --ref vX.Y.Z`.
4. Create the GitHub release from those artifacts, with the changelog entry as its notes.
   The GUI's update check reads the latest non-prerelease release's tag.

## Staying in sync with the Python package

[UPSTREAM.md](https://github.com/nebiolabs/domainatoredux/blob/main/UPSTREAM.md) has the
full workflow. The essentials:

- **The pin.** `UPSTREAM.md` records the upstream commit the port is caught up to, and
  `external/domainator/` is a clone kept there. CI greps that file for the pin, so it stays
  at the repo root.
- **The Domainator Viewer shell is a generated artifact, never hand-edited.**
  `crates/domainator-core/assets/viewer_shell.html` is the render of Python's
  `viewer_html(...)` plus one local patch (the GUI host hook that posts node selection
  to the Tauri host). Regenerate with `scripts/regen_viewer_shell.py`; `--check` is the
  drift guard.
- **So are the `--html_out` report shells** in `crates/domainator-core/assets/report_shells/`:
  upstream's `report_html.report_shell(page)`, unpatched, written by
  `scripts/regen_report_shells.py` (`--check` is the drift guard) and filled by
  `tools/report_html.rs`. Porting a report's HTML means producing its payload, not its
  page. These and the viewer shell are the only literal-copy artifacts — everything else
  is hand-ported logic under parity gates.
- **The GUI reuses that same shell** via `include_str!`, so viewer changes reach the app on
  a rebuild.
- **On-disk format changes** upstream require parallel Rust changes in the owning module.
- **Routine:** `scripts/sync_upstream.sh` fast-forwards the vendored copy, prints the
  changelog to triage against `UPSTREAM.md`'s file→owner map, runs the guards, and reminds
  you to advance the pin (and `UPSTREAM_COMMIT` with it; see
  [Versioning and releases](#versioning-and-releases)).

## The docs

`docs/` is an [mdbook](https://rust-lang.github.io/mdBook/). `book.toml` sits at the repo
root and points `src` at `docs/`, so every page is also plain markdown that renders on
GitHub.

```bash
cargo install mdbook
mdbook serve     # live preview at http://localhost:3000
mdbook build     # renders into target/book
```

Add a page by creating `docs/<name>.md` and listing it in `docs/SUMMARY.md` — `book.toml`
sets `create-missing = false`, so a `SUMMARY.md` entry with no file is a build error rather
than a silently generated blank page.

## Working on the desktop app

```bash
cd apps/domainatoredux-gui
npm install
npm run tauri dev    # needs a display
npm run build        # frontend only, no display needed
npm run check        # svelte-check
npm test             # vitest
cargo build -p domainatoredux-gui
```

`apps/domainatoredux-gui/README.md` has the architecture, the persistence model, and the
WSL2-versus-Windows validation guidance — Tauri uses **WebView2 on Windows** and
**WebKitGTK on Linux**, and Windows is a hard-requirement target, so validate there before
shipping.

The tool dialogs are described by one spec file per tool,
`apps/domainatoredux-gui/tools/<id>.yml`, embedded at compile time, layered over the tool's
manifest entry (`domainatoredux describe`). The spec gives the fields' labels, their
order, and which files each input offers, and a GUI default can override the CLI's for
the dialog alone; types, choices, defaults, help and roles come from the manifest. The
format is in `apps/domainatoredux-gui/tools/README.md`. The tests in `tool_specs.rs` check each
spec against the manifest: every option is listed, and every GUI default fits.

Three internal design notes live alongside it:
`apps/domainatoredux-gui/PERSISTENT_CACHE_PLAN.md` (a deferred on-disk report cache),
`apps/domainatoredux-gui/EMBEDDED_DB_NOTES.md` (whether `.domainatoredux/` should use an embedded
database; conclusion: not yet) and `apps/domainatoredux-gui/LARGE_FILE_EDITING_PLAN.md` (how
the Text / Table editors would lift their 64 MB cap — a windowed document model, staged so
the read-only phase ships on its own).

## Where things live

See [Architecture](architecture.md) for the crate and module map, and
[Porting status](porting-status.md) for per-tool coverage.
