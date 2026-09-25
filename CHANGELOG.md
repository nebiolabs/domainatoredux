# Changelog

This project uses its own [SemVer](https://semver.org/) numbering, independent of the
Python `domainator` package. Each entry says which upstream release (and commit) it is at
parity with. How versions are assigned: `docs/development.md`, "Versioning and releases".

## Unreleased — 1.0.0

Parity: Python domainator 0.11.0 @ `9c70934`.

- First release under this versioning scheme. The earlier `v0.11.0` tag predates it.
- The app runs each Tools-menu job in its own background process, running the same code as
  the `domainator` CLI. **Cancel now stops a running job** (and any diamond, cd-hit or
  usearch it started) and deletes the partial output files the job created or changed. A
  crashing tool no longer takes the app down with it. Jobs wait in a queue, two at a
  time by default (**Preferences → Jobs at once**). Each job's config and full output are
  kept in `.domainator/jobs/<job>/`. Database index builds also run in the background and
  can be canceled. Setting `DOMAINATOR_INPROCESS_JOBS=1` runs jobs inside the app as
  before; that escape hatch will be removed in the next release.
- Because app jobs now go through the CLI, some tools respect options they used to ignore.
  **matrix-report** honors `merge_impact_metric`, `max_merge_events` and `json`, and
  **hmmer-report** writes its `json` output (it used to write JSON into the TSV path).
  Every tool now uses exactly the CLI's defaults and checks.
- The `domainator-cli` crate is now a library (`domainator_cli::run`) with a thin binary;
  the CLI itself is unchanged.
- `domainator --version`, the GUI About box, the diagnostics blob and job provenance now
  report the upstream version and commit the build is at parity with.
- `.dsnv` bundles record the upstream version in `domainator_version`, matching what the
  Python tool and the viewer write, so the bundle is now identical to Python's field for
  field.
