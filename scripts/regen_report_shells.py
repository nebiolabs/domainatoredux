#!/usr/bin/env python3
"""Regenerate the vendored HTML report shells from the Python `domainator` source.

The Rust assets ``crates/domainator-core/assets/report_shells/<name>.html`` are NOT
authored by hand. Each is upstream's ``domainator.report_html.report_shell(name)``: the
report page with its title, data and (for a table) rows left as the tokens
``@@REPORT_TITLE@@``, ``@@REPORT_DATA@@`` and ``@@REPORT_ROWS@@``, which
``crates/domainator-core/src/tools/report_html.rs`` fills at runtime. Unlike the SSN viewer
shell (``regen_ssn_viewer_shell.py``) they carry no local patches: the shell is upstream's
page byte for byte.

Usage:
    python scripts/regen_report_shells.py            # write the assets
    python scripts/regen_report_shells.py --check    # verify they are up to date
    python scripts/regen_report_shells.py --src PATH # override vendored src dir

Run with the domainator conda env, e.g.:
    /home/sean/miniforge3/envs/domainator/bin/python scripts/regen_report_shells.py

--check exits non-zero (with a short diff) when a committed shell differs from a fresh
render, or one is missing or extra; it is an upstream-drift guard in CI and
scripts/sync_upstream.sh.
"""

from __future__ import annotations

import argparse
import difflib
import importlib
import sys
from pathlib import Path

# The pages this port writes. Add a name here, and its include_str! in report_html.rs,
# when a report's --html_out is ported.
REPORTS = ["table_report"]

# Must match report_html.rs.
TOKENS = {"@@REPORT_TITLE@@": 2, "@@REPORT_DATA@@": 1}
ROWS_TOKEN = "@@REPORT_ROWS@@"

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_SRC = REPO_ROOT / "external" / "domainator" / "src"
ASSET_DIR = REPO_ROOT / "crates" / "domainator-core" / "assets" / "report_shells"


def _fail(msg: str) -> "NoReturn":  # type: ignore[name-defined]
    sys.exit(f"regen_report_shells: {msg}")


def render_shells(src_dir: Path) -> dict[str, str]:
    src_dir = src_dir.resolve()
    if not (src_dir / "domainator" / "report_html.py").exists():
        _fail(f"no domainator/report_html.py under --src {src_dir}")
    # Make the vendored copy win over any installed `domainator`.
    sys.path.insert(0, str(src_dir))
    for mod in list(sys.modules):
        if mod == "domainator" or mod.startswith("domainator."):
            del sys.modules[mod]
    report_html = importlib.import_module("domainator.report_html")

    contract = (report_html.TITLE_TOKEN, report_html.DATA_TOKEN, report_html.ROWS_TOKEN)
    if contract != ("@@REPORT_TITLE@@", "@@REPORT_DATA@@", ROWS_TOKEN):
        _fail(f"upstream's tokens changed to {contract}; update this script and report_html.rs")

    shells = {}
    for name in REPORTS:
        html = report_html.report_shell(name)
        for token, expected in TOKENS.items():
            if html.count(token) != expected:
                _fail(f"{name}: expected {token} {expected} time(s), found {html.count(token)}")
        if html.count(ROWS_TOKEN) > 1:
            _fail(f"{name}: {ROWS_TOKEN} more than once")
        shells[name] = html
    return shells


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--src", type=Path, default=DEFAULT_SRC,
                    help=f"vendored domainator src dir (default: {DEFAULT_SRC})")
    ap.add_argument("--check", action="store_true",
                    help="verify the committed shells match a fresh render; exit 1 if not")
    args = ap.parse_args()

    shells = render_shells(args.src)

    if args.check:
        stale = []
        committed = {p.stem for p in ASSET_DIR.glob("*.html")} if ASSET_DIR.exists() else set()
        for extra in sorted(committed - set(shells)):
            stale.append(extra)
            print(f"extra shell with no upstream page: {extra}.html")
        for name, rendered in shells.items():
            path = ASSET_DIR / f"{name}.html"
            current = path.read_text(encoding="utf-8") if path.exists() else ""
            if current == rendered:
                print(f"OK: {path.relative_to(REPO_ROOT)} is up to date ({len(rendered)} bytes).")
                continue
            stale.append(name)
            diff = "".join(difflib.unified_diff(
                current.splitlines(keepends=True), rendered.splitlines(keepends=True),
                fromfile=f"committed {name}.html", tofile="render"))
            sys.stdout.write(diff[:8000])
            if len(diff) > 8000:
                sys.stdout.write("\n... (diff truncated) ...\n")
        if stale:
            _fail(f"STALE: {', '.join(stale)} -- run `python scripts/regen_report_shells.py` to update.")
        return

    ASSET_DIR.mkdir(parents=True, exist_ok=True)
    for name, rendered in shells.items():
        path = ASSET_DIR / f"{name}.html"
        # newline="" keeps the bytes as rendered on every platform.
        with open(path, "w", encoding="utf-8", newline="") as handle:
            handle.write(rendered)
        print(f"wrote {path.relative_to(REPO_ROOT)} ({len(rendered)} bytes).")


if __name__ == "__main__":
    main()
