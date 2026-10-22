#!/usr/bin/env python3
"""Snapshot upstream's tool manifest (`domainator describe`) for the golden test.

Upstream builds its manifest from its argparse parsers, so it is the complete and explicit
description of every command's interface: option names, kinds, types, bounds, defaults,
choices, and which options are files to read or write. This port emits the same format
(`domainatoredux describe`), and a cargo test diffs ours against this snapshot so an
interface difference is a failing test rather than something the parity sweep stumbles
over (see docs/parity.md).

The snapshot is rendered from the vendored clone (external/domainator, kept at the pin),
not from whatever the conda env happens to import, so it always describes the pin. Prose
that churns without changing the interface (help, descriptions, summaries) and the
version are stripped; a default that depends on the machine it was rendered on (a path
under the home directory or a per-user temp directory) is recorded as `default_dynamic`,
as upstream already does for paths under `~`.

Usage:
    scripts/parity/refresh_manifest.py            # write scripts/parity/upstream_manifest.json
    scripts/parity/refresh_manifest.py --check    # exit 1 if the snapshot is stale

Run with the domainator conda env's Python (it needs upstream's dependencies).
"""

import argparse
import json
import os
import pathlib
import subprocess
import sys

REPO = pathlib.Path(__file__).resolve().parents[2]
DEFAULT_SRC = REPO / "external" / "domainator" / "src"
SNAPSHOT = REPO / "scripts" / "parity" / "upstream_manifest.json"

# Keys that are prose or provenance, not interface.
STRIPPED_TOOL_KEYS = ("summary", "description")
STRIPPED_PARAMETER_KEYS = ("help",)


def render(python, src):
    """Upstream's manifest, rendered by the clone at `src`."""
    code = ("import json, sys; sys.path.insert(0, sys.argv[1]); "
            "from domainator.cli.manifest import manifest; "
            "print(json.dumps(manifest()))")
    proc = subprocess.run([python, "-I", "-c", code, str(src)],
                          capture_output=True, text=True, check=True)
    return json.loads(proc.stdout)


def machine_specific(value):
    if not isinstance(value, str):
        return False
    home = os.path.expanduser("~")
    user = os.environ.get("USER") or os.path.basename(home)
    return value.startswith(home) or (value.startswith("/tmp/") and user in value)


def normalize(doc):
    doc.pop("domainator_version", None)
    for tool in doc["tools"]:
        for key in STRIPPED_TOOL_KEYS:
            tool.pop(key, None)
        for parameter in tool["parameters"]:
            for key in STRIPPED_PARAMETER_KEYS:
                parameter.pop(key, None)
            for column in parameter.get("columns") or []:
                column.pop("help", None)
            if machine_specific(parameter.get("default")):
                del parameter["default"]
                parameter["default_dynamic"] = True
    return doc


def dump(doc):
    return json.dumps(doc, indent=1, sort_keys=True) + "\n"


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--python", default=sys.executable,
                    help="interpreter with upstream's dependencies (default: this one)")
    ap.add_argument("--src", default=str(DEFAULT_SRC), help="upstream src/ directory")
    ap.add_argument("-o", "--output", default=str(SNAPSHOT))
    ap.add_argument("--check", action="store_true",
                    help="compare with the committed snapshot instead of writing it")
    cfg = ap.parse_args(argv)

    text = dump(normalize(render(cfg.python, cfg.src)))
    out = pathlib.Path(cfg.output)
    if cfg.check:
        if not out.exists() or out.read_text() != text:
            print(f"{out.relative_to(REPO)} is stale: run scripts/parity/refresh_manifest.py",
                  file=sys.stderr)
            return 1
        print(f"{out.relative_to(REPO)} is up to date")
        return 0
    out.write_text(text)
    print(f"wrote {out.relative_to(REPO)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
