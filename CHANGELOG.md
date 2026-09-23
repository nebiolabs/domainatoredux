# Changelog

This project uses its own [SemVer](https://semver.org/) numbering, independent of the
Python `domainator` package. Each entry says which upstream release (and commit) it is at
parity with. How versions are assigned: `docs/development.md`, "Versioning and releases".

## Unreleased — 1.0.0

Parity: Python domainator 0.11.0 @ `9c70934`.

- First release under this versioning scheme. The earlier `v0.11.0` tag predates it.
- `domainator --version`, the GUI About box, the diagnostics blob and job provenance now
  report the upstream version and commit the build is at parity with.
- `.dsnv` bundles record the upstream version in `domainator_version`, matching what the
  Python tool and the viewer write, so the bundle is now identical to Python's field for
  field.
