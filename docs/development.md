# Developing Domainator

Contributor-facing notes. For using the tools, start at
[Getting started](getting-started.md).

## Build and test

```bash
cargo build                      # debug build of the workspace
cargo build --release            # optimized (lto); use for any timing or benchmarks
cargo test                       # the unit suite; no conda or clone needed
./target/debug/domainator <tool> [opts]
```

**The project holds itself to zero warnings, from both `rustc` and clippy:**

```bash
cargo build --workspace 2>&1 | grep -c warning:              # must be 0
cargo clippy --workspace --all-targets -- -D warnings        # must be clean
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

- `crates/domainator-config` — the config/schema layer is a native Rust format,
  deliberately not jsonargparse-compatible;
- `app_support/` — the GUI seam: workspace, provenance DAG, database registry, HMM logo,
  contig editor, summary dashboard;
- the Tauri command layer.

When adding to any of those, a test is the whole safety net. A few guards earn their keep
in particular:

- **`every_schema_matches_its_config_struct`** (`domainator-config`) —
  `domainator schema <tool>` is both the documented agent interface and what the GUI
  renders its dialogs from, while the config struct is what actually deserializes. They
  are hand-written in different places and drift both ways: a field missing from the
  schema is a flag no agent and no GUI dialog can discover, and a property the struct
  lacks is worse, because every config struct is `deny_unknown_fields`, so a config
  written to match the schema is *rejected*. **Add your tool to `every_tool()` when you
  add a tool.**
- **`merge_cli` coverage** — a struct field the merge forgets is a CLI flag that silently
  does nothing.
- **`every_offered_tool_is_dispatched_and_has_a_schema`** and
  **`every_tools_dialog_params_parse_as_its_cli_config`** (`domainator-gui`) — the GUI tool
  registry, the CLI's subcommands and the schema emitter are three hand-maintained lists
  in three places. GUI jobs run `<tool> --config <params>` through the CLI in a worker
  process, so a menu entry the CLI does not know, or a dialog whose params the config
  struct rejects, fails only when the user presses Run. The tests feed each tool's
  dialog params to `domainator_cli::run` with `--print_config`.
- **`derived_provenance_roles_match_the_golden_table`** (`domainator-gui`) — a job's
  provenance inputs are derived from its tool spec, and `trace_source_sequences` follows
  their roles. A spec edit that changes a role breaks SSN node → sequence tracing without
  any error.
- **`tests/worker.rs`** (`domainator-gui`) — runs the real binary in worker mode: it must
  never start Tauri, Cancel must kill the whole process tree, a worker must exit when the
  GUI's end of its stdin closes, and a worker run must produce the CLI's output.
- **`the_managed_diamond_is_new_enough_for_deduplicate`** and
  **`managed_tools_match_the_core_locator`** (`domainator-gui`) — the External tools panel
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
  Accepts a `.dsnv` directly.
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

`.github/workflows/check.yml` runs build (`-D warnings`), clippy (`-D warnings`) and tests
on the pure-Rust crates, plus a second job that clones upstream at the recorded pin to run
the viewer-shell drift guard and the SSN core probe comparison. It does **not** build the
Tauri crates — webkit2gtk costs more to install than that gate is worth, so the GUI's
guard tests run locally and via `sync_upstream.sh`.

`.github/workflows/build-gui-apps.yml` packages the desktop apps (macOS `.dmg`, Windows
NSIS, Linux `.deb`/AppImage) and the CLI binaries. It runs on `v*` release tags or manual
dispatch, and releases are cut by hand from its artifacts.

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
  appear in `domainator --version`, the About box, the diagnostics blob and every job's
  provenance (`upstream_version`). `UPSTREAM_VERSION` is also what a `.dsnv` bundle's
  `domainator_version` key holds, because upstream reads that key as "which domainator
  release wrote this". Three guards keep the constants honest:
  - a unit test that `UPSTREAM_COMMIT` equals `UPSTREAM.md`'s pin;
  - a unit test that `UPSTREAM_VERSION` equals the version baked into the regenerated
    viewer shell;
  - a `sync_upstream.sh` guard comparing `UPSTREAM_VERSION` with the vendored
    `__version__`.
- **Tags** are `vX.Y.Z`, one per release, covering the CLI and both apps. The packaging
  workflow refuses a tag that differs from `v` + `workspace.package.version`. (The
  `v0.11.0` tag predates this scheme; the first release under it is `v1.0.0`.)

To cut a release:

1. Bump `workspace.package.version` in `Cargo.toml`.
2. Add an entry to `CHANGELOG.md` that names the upstream version and commit it is at
   parity with.
3. Commit, tag `vX.Y.Z` and push the tag. `build-gui-apps.yml` builds the installers and
   CLI binaries.
4. Create the GitHub release from those artifacts, with the changelog entry as its notes.
   The GUI's update check reads the latest non-prerelease release's tag.

## Staying in sync with the Python package

[UPSTREAM.md](https://github.com/nebiolabs/domainatoredux/blob/main/UPSTREAM.md) has the
full workflow. The essentials:

- **The pin.** `UPSTREAM.md` records the upstream commit the port is caught up to, and
  `external/domainator/` is a clone kept there. CI greps that file for the pin, so it stays
  at the repo root.
- **The SSN viewer shell is a generated artifact, never hand-edited.**
  `crates/domainator-core/assets/ssn_viewer_shell.html` is the render of Python's
  `ssn_viewer_html(...)` plus one local patch (the GUI host hook that posts node selection
  to the Tauri host). Regenerate with `scripts/regen_ssn_viewer_shell.py`; `--check` is the
  drift guard. It is the *only* literal-copy artifact — everything else is hand-ported
  logic under parity gates.
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
cd apps/domainator-gui
npm install
npm run tauri dev    # needs a display
npm run build        # frontend only, no display needed
npm run check        # svelte-check
cargo build -p domainator-gui
```

`apps/domainator-gui/README.md` has the architecture, the persistence model, and the
WSL2-versus-Windows validation guidance — Tauri uses **WebView2 on Windows** and
**WebKitGTK on Linux**, and Windows is a hard-requirement target, so validate there before
shipping.

The tool dialogs are described by one spec file per tool,
`apps/domainator-gui/tools/<id>.yml`, embedded at compile time. The spec gives the
fields, their order, and which files each input offers. It lists every CLI option with
its CLI default, and a GUI default can override that for the dialog alone. The format
is in `apps/domainator-gui/tools/README.md`. The tests in `tool_specs.rs` check each spec
against `domainator schema <id>`: every option is listed, and each `cli_default` equals
the schema's.

Three internal design notes live alongside it:
`apps/domainator-gui/PERSISTENT_CACHE_PLAN.md` (a deferred on-disk report cache),
`apps/domainator-gui/EMBEDDED_DB_NOTES.md` (whether `.domainator/` should use an embedded
database; conclusion: not yet) and `apps/domainator-gui/LARGE_FILE_EDITING_PLAN.md` (how
the Text / Table editors would lift their 64 MB cap — a windowed document model, staged so
the read-only phase ships on its own).

## Where things live

See [Architecture](architecture.md) for the crate and module map, and
[Porting status](porting-status.md) for per-tool coverage.
