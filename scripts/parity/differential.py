#!/usr/bin/env python3
"""Replay upstream's harvested invocations through both implementations and diff.

This is the broad-coverage parity gate. `harvest_invocations.py` mines upstream's
pytest suite for `main([...])` argument lists; this runs each one twice -- once
against the Python package, once against `target/debug/domainatoredux` -- into two
scratch directories, then compares every file the two runs produced.

Outcomes, worst-wins across an invocation's output files:

    identical    byte-for-byte equal
    tolerated    equal once search numerics are allowed to differ (hmmer-pure-rs
                 is a different HMMER3 implementation -- see CLAUDE.md)
    differs      a real divergence
    rust-failed  the Rust tool exited non-zero where Python succeeded
    error        timeout or harness failure
    skipped      Python itself rejected the invocation, or it produced no files

Every invocation that is not identical/tolerated/skipped must be listed in
`known_gaps.tsv`, which is the tracked list of open work. The run exits non-zero
when reality and that list disagree in either direction, so a new divergence
fails the gate and a fixed one is not allowed to linger as a stale entry.

Requires the conda environment and the vendored upstream clone; it is not part
of the CI check job. The Python side runs the clone's `src/` (`--src`), put first on
PYTHONPATH, so a domainator the env has installed -- such as an editable install of a
checkout with work in progress -- never decides what the sweep compares against.
See `docs/parity.md`.
"""
import argparse
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile
import traceback

REPO = pathlib.Path(__file__).resolve().parents[2]

# Search numerics are explicitly not a parity target. The `domainator_<db>`
# rollup packs evalue+score into a single string, so skipping `evalue`/`score`
# alone cannot reach them -- the rollup keys are added per file at compare time.
NUMERIC_QUALIFIERS = {"evalue", "score", "identity"}

OK = ("identical", "tolerated", "skipped")


# --------------------------------------------------------------------------
# comparators: (path_a, path_b) -> (outcome, note)
# --------------------------------------------------------------------------

def _compare_genbank(a, b, helpers, SeqIO):
    skip = set(NUMERIC_QUALIFIERS)
    for path in (a, b):
        try:
            for rec in SeqIO.parse(path, "genbank"):
                for feat in rec.features:
                    skip |= {k for k in feat.qualifiers if k.startswith("domainator_")}
        except Exception:
            pass
    try:
        helpers.compare_seqfiles(str(a), str(b), skip_qualifiers=skip)
    except Exception:
        frame = traceback.extract_tb(sys.exc_info()[2])[-1]
        return "differs", f"compare_seqfiles failed at helpers.py:{frame.lineno}"
    # `compare_seqfiles` checks sequence, annotations, feature *count* and
    # qualifiers -- but not feature coordinates, feature types, or the LOCUS
    # name. Those are exactly where the origin-crossing slice bugs lived, so
    # check them here rather than trusting the upstream comparator alone.
    left, right = list(SeqIO.parse(a, "genbank")), list(SeqIO.parse(b, "genbank"))
    bad_loc = sum(1 for x, y in zip(left, right)
                  for fx, fy in zip(x.features, y.features)
                  if fx.type != fy.type or str(fx.location) != str(fy.location))
    bad_name = sum(1 for x, y in zip(left, right) if x.name != y.name)
    if bad_loc or bad_name:
        return "differs", f"{bad_loc} feature location/type, {bad_name} LOCUS name"
    return "tolerated", ""


def _compare_matrix(a, b):
    import numpy as np
    from domainator.data_matrix import DataMatrix
    ma, mb = DataMatrix.from_file(str(a)), DataMatrix.from_file(str(b))
    if ma.rows != mb.rows or ma.columns != mb.columns:
        return "differs", "matrix labels differ"
    if ma.data_type != mb.data_type:
        return "differs", f"data_type {ma.data_type} != {mb.data_type}"
    if not np.allclose(ma.toarray(), mb.toarray(), rtol=1e-3, atol=1e-6, equal_nan=True):
        return "differs", "matrix values outside tolerance"
    return "tolerated", ""


def _compare_json(a, b):
    # `json_parity.py` gunzips a bundle itself and ignores key *order*, which is
    # required: serde_json is built with `preserve_order`, Python sorts keys.
    script = REPO / "scripts" / "json_parity.py"
    proc = subprocess.run([sys.executable, str(script), str(a), str(b), "--rtol", "1e-3"],
                          capture_output=True, text=True)
    if proc.returncode == 0:
        return "tolerated", ""
    detail = (proc.stdout + proc.stderr).strip().splitlines()
    return "differs", detail[-1][:110] if detail else "json differs"


_REPORT_DATA = re.compile(r'(<script type="application/json" id="report-data">)(.*?)(</script>)', re.S)
_REPORT_ROWS = re.compile(r'(<script type="application/x-ndjson" id="report-rows">\n)(.*?)(</script>)', re.S)


def _compare_html(a, b):
    """An --html_out report page (upstream report_html.py). Both sides fill the same
    generated shell (crates/domainator-core/assets/report_shells/), so the page outside its
    report-data envelope and report-rows block must match byte for byte; the envelope
    (gzip+base64 data decoded) and the rows are compared parsed, like any JSON output."""
    import base64
    import gzip
    import importlib.util
    import json

    spec = importlib.util.spec_from_file_location("json_parity", REPO / "scripts" / "json_parity.py")
    json_parity = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(json_parity)

    def split(path):
        text = pathlib.Path(path).read_text(encoding="utf-8")
        data = _REPORT_DATA.search(text)
        if data is None:
            return text, None, None
        envelope = json.loads(data.group(2))
        if envelope.get("encoding") == "gzip+base64":
            envelope["data"] = json.loads(gzip.decompress(base64.b64decode(envelope["data"])))
        rows = _REPORT_ROWS.search(text)
        row_list = None if rows is None else [json.loads(l) for l in rows.group(2).splitlines()]
        shell = _REPORT_DATA.sub(r"\1@@REPORT_DATA@@\3", text)
        shell = _REPORT_ROWS.sub(r"\1@@REPORT_ROWS@@\3", shell)
        return shell, envelope, row_list

    (shell_a, env_a, rows_a), (shell_b, env_b, rows_b) = split(a), split(b)
    if env_a is None or env_b is None:
        return _compare_text(a, b)  # not a report page
    if shell_a != shell_b:
        la, lb = shell_a.splitlines(), shell_b.splitlines()
        first = next((i for i, (x, y) in enumerate(zip(la, lb)) if x != y), min(len(la), len(lb)))
        return "differs", f"page outside its data differs at line {first + 1}"
    problems = json_parity.compare(env_a, env_b, 1e-3, 1e-12, "$report-data")
    problems += json_parity.compare(rows_a, rows_b, 1e-3, 1e-12, "$report-rows")
    if problems:
        return "differs", problems[0][:110]
    return "tolerated", ""


# Projection coordinates are scaled to span 0..--scale (1000 by default), and upstream
# projects a sparse matrix with scikit-learn's randomized TruncatedSVD, unseeded, so its
# own runs disagree in about the fifth decimal. This port's SVD is exact.
COORDINATE_ATOL = 1e-3


def _compare_xgmml(a, b):
    import math
    import xml.etree.ElementTree as ET

    def summarize(path):
        root = ET.parse(path).getroot()
        tag = lambda e: e.tag.rsplit("}", 1)[-1]
        nodes, edges, graphics = set(), set(), {}
        for el in root:
            atts = frozenset((c.get("name"), c.get("value"))
                             for c in el if tag(c) == "att")
            if tag(el) == "node":
                nodes.add((el.get("label"), atts))
                g = next((c for c in el if tag(c) == "graphics"), None)
                if g is not None:
                    graphics[el.get("label")] = dict(g.attrib)
            elif tag(el) == "edge":
                edges.add((el.get("source"), el.get("target"), atts))
        return nodes, edges, graphics

    (na, ea, ga), (nb, eb, gb) = summarize(a), summarize(b)
    if na != nb:
        return "differs", f"{len(na ^ nb)} node(s) differ"
    if ea != eb:
        return "differs", f"{len(ea ^ eb)} edge(s) differ"
    # Node positions (build_tree's layout, build_projection's coordinates) and fills.
    for label, attrs_a in ga.items():
        attrs_b = gb.get(label, {})
        for key in set(attrs_a) | set(attrs_b):
            va, vb = attrs_a.get(key), attrs_b.get(key)
            if key in ("x", "y", "z") and va is not None and vb is not None:
                if math.isclose(float(va), float(vb), rel_tol=1e-6, abs_tol=COORDINATE_ATOL):
                    continue
            if va != vb:
                return "differs", f"node {label} graphics {key}: {va} != {vb}"
    return "tolerated", ""


def _compare_coords_tsv(a, b):
    """build_projection --coords_out: the same points and axes, coordinates within
    COORDINATE_ATOL."""
    import math

    def rows(path):
        lines = pathlib.Path(path).read_text().splitlines()
        return lines[0], [line.split("\t") for line in lines[1:]]

    (ha, ra), (hb, rb) = rows(a), rows(b)
    if ha != hb or len(ra) != len(rb):
        return "differs", "different axes or points"
    for row_a, row_b in zip(ra, rb):
        if row_a[0] != row_b[0] or len(row_a) != len(row_b):
            return "differs", f"point {row_a[0]} != {row_b[0]}"
        for va, vb in zip(row_a[1:], row_b[1:]):
            if not math.isclose(float(va), float(vb), rel_tol=1e-6, abs_tol=COORDINATE_ATOL):
                return "differs", f"{row_a[0]}: {va} != {vb}"
    return "tolerated", ""


def _bundle_problems(bundle_a, bundle_b):
    """A Domainator Viewer bundle (.dvb), parsed: compared like any JSON output, but
    with projection coordinates held to COORDINATE_ATOL."""
    import importlib.util

    spec = importlib.util.spec_from_file_location("json_parity", REPO / "scripts" / "json_parity.py")
    json_parity = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(json_parity)
    projection = "projection" in (bundle_a.get("views") or {})
    return json_parity.compare(bundle_a, bundle_b, 1e-3, COORDINATE_ATOL if projection else 1e-12,
                               "$bundle")


def _read_bundle(path):
    import gzip
    import json

    raw = pathlib.Path(path).read_bytes()
    return json.loads(gzip.decompress(raw) if raw[:2] == b"\x1f\x8b" else raw)


def _compare_bundle(a, b):
    problems = _bundle_problems(_read_bundle(a), _read_bundle(b))
    return ("differs", problems[0][:110]) if problems else ("tolerated", "")


_VIEWER_BUNDLE = re.compile(r'"embeddedBundleBase64": (null|"[^"]*")')
# The GUI host hook regen_viewer_shell.py adds to our copy of the shell: from its
# comment to the end of its try block.
_GUI_HOST_HOOK = re.compile(
    r"        // Domainator GUI host hook.*?\} catch \(e\) \{ /\* ignore \*/ \}\n", re.S)


def _compare_viewer_html(a, b):
    """A Domainator Viewer page (upstream viewer_html.py). Both sides fill the same
    generated shell (crates/domainator-core/assets/viewer_shell.html, which adds the GUI
    host hook), so the page outside its embedded bundle must match byte for byte once
    the hook is removed; the bundles are compared parsed."""
    import base64
    import gzip
    import json

    def split(path):
        text = _GUI_HOST_HOOK.sub("", pathlib.Path(path).read_text(encoding="utf-8"), count=1)
        match = _VIEWER_BUNDLE.search(text)
        embedded = json.loads(match.group(1))
        bundle = None if embedded is None else json.loads(gzip.decompress(base64.b64decode(embedded)))
        return _VIEWER_BUNDLE.sub('"embeddedBundleBase64": @@BUNDLE@@', text, count=1), bundle

    (shell_a, bundle_a), (shell_b, bundle_b) = split(a), split(b)
    if shell_a != shell_b:
        la, lb = shell_a.splitlines(), shell_b.splitlines()
        first = next((i for i, (x, y) in enumerate(zip(la, lb)) if x != y), min(len(la), len(lb)))
        return "differs", f"viewer page outside its bundle differs at line {first + 1}"
    if (bundle_a is None) != (bundle_b is None):
        return "differs", "one page embeds a bundle and the other does not"
    problems = [] if bundle_a is None else _bundle_problems(bundle_a, bundle_b)
    return ("differs", problems[0][:110]) if problems else ("tolerated", "")


def _normalize_hmm(text):
    # Profiles (HMMs and CMs) carry a build DATE and the COM line that produced them,
    # with temporary paths in it; neither is
    # a parity target. NAME can carry a tool-chosen default too, but that *is*.
    return "\n".join(l for l in text.splitlines()
                     if not l.startswith(("DATE ", "COM   ")))


def _compare_text(a, b, normalize=None):
    ta = pathlib.Path(a).read_text(errors="replace")
    tb = pathlib.Path(b).read_text(errors="replace")
    if normalize:
        ta, tb = normalize(ta), normalize(tb)
    if ta == tb:
        return "tolerated", ""
    la, lb = ta.splitlines(), tb.splitlines()
    if len(la) != len(lb):
        return "differs", f"{len(la)} vs {len(lb)} lines"
    first = next(i for i, (x, y) in enumerate(zip(la, lb)) if x != y)
    return "differs", f"line {first + 1}: {la[first][:40]!r} != {lb[first][:40]!r}"


def compare(a, b, helpers, SeqIO):
    """Classify one pair of output files."""
    if pathlib.Path(a).read_bytes() == pathlib.Path(b).read_bytes():
        return "identical", ""
    ext = pathlib.Path(a).suffix.lower()
    try:
        if ext in (".gb", ".gbk", ".genbank"):
            return _compare_genbank(a, b, helpers, SeqIO)
        if ext in (".hdf5", ".h5"):
            return _compare_matrix(a, b)
        if ext in (".dvb", ".dsnv"):
            return _compare_bundle(a, b)
        if ext in (".json", ".ssnv", ".ndjson"):
            return _compare_json(a, b)
        if pathlib.Path(a).read_text(errors="replace").startswith("\tprojection_X\t"):
            return _compare_coords_tsv(a, b)
        if ext == ".xgmml":
            return _compare_xgmml(a, b)
        if ext in (".hmm", ".cm"):  # a CM carries both its own and its filter HMM's
            return _compare_text(a, b, _normalize_hmm)
        # A test may name its page with no extension (an @OUT@ the harvester could not
        # resolve), so a page is also recognized by its first bytes.
        if ext in (".html", ".htm") or pathlib.Path(a).read_bytes()[:15].lower() == b"<!doctype html>":
            if 'id="viewer-config"' in pathlib.Path(a).read_text(errors="replace"):
                return _compare_viewer_html(a, b)
            return _compare_html(a, b)
        return _compare_text(a, b)
    except Exception as exc:
        return "error", f"comparator raised: {type(exc).__name__}: {exc}"[:110]


# --------------------------------------------------------------------------
# replay
# --------------------------------------------------------------------------

def _rust_error(proc):
    """The useful line of a failed Rust run.

    clap ends its output with `Usage:` and `For more information, try '--help'`,
    so taking the last line hides the actual complaint -- which is usually the
    name of a flag this port has not implemented.
    """
    lines = [l.strip() for l in proc.stderr.decode(errors="replace").splitlines() if l.strip()]
    noise = ("Usage:", "For more information", "Try 'domainatoredux")
    useful = [l for l in lines if not l.startswith(noise)]
    if not useful:
        return f"exit {proc.returncode}"
    errors = [l for l in useful if l.lower().startswith("error")]
    return (errors[0] if errors else useful[0])[:140]


def produced_files(root):
    return sorted(str(p.relative_to(root)) for p in pathlib.Path(root).rglob("*") if p.is_file())


def run_one(row, cfg, helpers, SeqIO):
    module, key, where, argv = row[0], row[1], row[2], row[3:]
    with tempfile.TemporaryDirectory() as scratch:
        py_dir = os.path.join(scratch, "py")
        rs_dir = os.path.join(scratch, "rs")
        os.makedirs(py_dir)
        os.makedirs(rs_dir)

        def substitute(arg, out_dir):
            return (arg.replace("@DATA@", cfg.data_dir)
                       .replace("@GEN@", cfg.gen_dir or "@GEN@")
                       .replace("@TMP@", out_dir)
                       .replace("@OUT@", os.path.join(out_dir, "out")))

        py_argv = [substitute(a, py_dir) for a in argv]
        rs_argv = [substitute(a, rs_dir) for a in argv]
        try:
            # Upstream 1.0 runs every tool through its dispatcher; the per-module
            # `python -m domainator.<module>` form exits 0 having done nothing.
            # An `-i -` invocation (upstream feeds it through monkeypatch) must see an
            # empty stdin, not wait on the sweep's own. Python stamps undated records
            # with the local date and we stamp the UTC one, so Python runs in UTC:
            # otherwise every such record differs in the hours around midnight.
            py = subprocess.run([cfg.python, "-m", "domainator", module] + py_argv,
                                capture_output=True, timeout=cfg.timeout, cwd=py_dir,
                                stdin=subprocess.DEVNULL, env={**cfg.py_env, "TZ": "UTC"})
            rs = subprocess.run([cfg.rust, module] + rs_argv,
                                capture_output=True, timeout=cfg.timeout, cwd=rs_dir,
                                stdin=subprocess.DEVNULL)
        except subprocess.TimeoutExpired:
            return "error", f"timed out after {cfg.timeout}s"
        except Exception as exc:
            return "error", f"{type(exc).__name__}: {exc}"[:110]

        if py.returncode != 0:
            # Upstream rejected its own invocation -- it depends on state an
            # earlier step in the same test function set up, or on a fixture the
            # harvester could not resolve. Nothing to compare.
            return "skipped", "python rejected the invocation"
        if rs.returncode != 0:
            return "rust-failed", _rust_error(rs)

        py_files, rs_files = produced_files(py_dir), produced_files(rs_dir)
        if not py_files:
            return "skipped", "no output files (stdout-only invocation)"
        only_py = sorted(set(py_files) - set(rs_files))
        only_rs = sorted(set(rs_files) - set(py_files))
        if only_py or only_rs:
            return "differs", f"output files differ: -{only_py} +{only_rs}"[:110]

        worst, note = "identical", ""
        rank = {"identical": 0, "tolerated": 1, "error": 2, "differs": 3}
        for rel in py_files:
            outcome, why = compare(os.path.join(py_dir, rel), os.path.join(rs_dir, rel),
                                   helpers, SeqIO)
            if rank[outcome] > rank[worst]:
                worst, note = outcome, (f"{rel}: {why}" if why else rel)
        return worst, note


# --------------------------------------------------------------------------
# the tracked gap list
# --------------------------------------------------------------------------

def load_baseline(path):
    """(module, key) -> (outcome, classification, note)."""
    entries = {}
    if not pathlib.Path(path).exists():
        return entries
    for line in pathlib.Path(path).read_text().splitlines():
        if not line.strip() or line.startswith("#"):
            continue
        module, key, outcome, klass, note = (line.split("\t") + [""] * 5)[:5]
        entries[(module, key)] = (outcome, klass, note)
    return entries


BASELINE_HEADER = """\
# Tracked parity gaps: upstream invocations that do not yet reproduce.
#
# Which rows belong here is machine-maintained: regenerate with
# `scripts/parity/differential.py --update_baseline`, and an ordinary run exits
# non-zero whenever reality and this file disagree -- so a new divergence fails
# the gate, and a fixed one cannot linger here as a stale entry.
#
# The `class` column is the opposite: it is written by hand and carried across
# regenerations, because no script can tell a deliberate deferral from a bug.
# A new row arrives with an empty class; triage it into one of
#
#   deferred   out of scope on purpose, and recorded as such in UPSTREAM.md
#   gap        a real missing feature -- the tracked to-do list
#   upstream   the divergence is in a dependency, not in this port
#   harness    an artifact of how the invocation is replayed, not a divergence
#
# Columns: <python module> <test_fn#n> <outcome> <class> <note>
"""


def write_baseline(path, entries):
    with open(path, "w") as fh:
        fh.write(BASELINE_HEADER)
        for (module, key), (outcome, klass, note) in sorted(entries.items()):
            fh.write(f"{module}\t{key}\t{outcome}\t{klass}\t{note}\n")


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("-i", "--invocations",
                    help="harvested TSV (default: harvest on the fly)")
    ap.add_argument("--test_dir", default=str(REPO / "external" / "domainator" / "test"))
    ap.add_argument("--src", default=str(REPO / "external" / "domainator" / "src"),
                    help="the domainator source the Python side runs (default: the "
                         "vendored clone at the pin)")
    ap.add_argument("--data_dir", default=str(REPO / "test" / "data"),
                    help="fixture directory substituted for @DATA@")
    ap.add_argument("--python", default="/home/sean/miniforge3/envs/domainator/bin/python")
    ap.add_argument("--rust", default=str(REPO / "target" / "debug" / "domainatoredux"))
    ap.add_argument("--extra", default=str(REPO / "scripts" / "parity" / "extra_invocations.tsv"),
                    help="hand-written invocations appended to the harvest; their @GEN@ "
                         "fixtures are built by generated_fixtures.py (pass '' to skip)")
    ap.add_argument("--baseline", default=str(REPO / "scripts" / "parity" / "known_gaps.tsv"))
    ap.add_argument("--update_baseline", action="store_true",
                    help="rewrite the gap list from this run instead of checking it")
    ap.add_argument("--only", action="append", default=[],
                    help="restrict to these python modules (repeatable)")
    ap.add_argument("--timeout", type=int, default=900)
    ap.add_argument("--report", help="write every invocation's outcome to this TSV. "
                                     "Worth reading even on a green run: a `tolerated` row "
                                     "names a file whose bytes differ, which is how the "
                                     "qualifier-wrapping divergence surfaced.")
    ap.add_argument("-v", "--verbose", action="store_true",
                    help="print every invocation as it is classified")
    cfg = ap.parse_args(argv)

    cfg.data_dir = os.path.abspath(cfg.data_dir)
    cfg.rust = os.path.abspath(cfg.rust)
    if not os.path.exists(cfg.rust):
        sys.exit(f"no Rust binary at {cfg.rust} -- run `cargo build` first")

    # `helpers.compare_seqfiles` is the upstream comparator; import it from the
    # vendored clone rather than copying it in, so it can never drift.
    cfg.src = os.path.abspath(cfg.src)
    if not os.path.isdir(os.path.join(cfg.src, "domainator")):
        sys.exit(f"no domainator package under {cfg.src}")
    sys.path.insert(0, cfg.src)
    # Every Python subprocess runs that source, ahead of whatever the env installed.
    cfg.py_env = {**os.environ,
                  "PYTHONPATH": os.pathsep.join(filter(None, [cfg.src, os.environ.get("PYTHONPATH")]))}
    sys.path.insert(0, cfg.test_dir)
    import helpers
    from Bio import SeqIO

    if cfg.invocations:
        rows = [l.split("\t") for l in
                pathlib.Path(cfg.invocations).read_text().splitlines() if l.strip()]
    else:
        harvested = tempfile.NamedTemporaryFile(suffix=".tsv", delete=False)
        harvested.close()
        subprocess.run([sys.executable, str(REPO / "scripts" / "parity" / "harvest_invocations.py"),
                        "--test_dir", cfg.test_dir, "--rust", cfg.rust, "-o", harvested.name],
                       check=True)
        rows = [l.split("\t") for l in
                pathlib.Path(harvested.name).read_text().splitlines() if l.strip()]
        os.unlink(harvested.name)

    if cfg.extra and pathlib.Path(cfg.extra).exists():
        rows += [l.split("\t") for l in pathlib.Path(cfg.extra).read_text().splitlines()
                 if l.strip() and not l.startswith("#")]

    if cfg.only:
        rows = [r for r in rows if r[0] in cfg.only]

    # Fixtures upstream's tests build at test time (`@GEN@`), built once per sweep
    # with upstream's own helpers, in the Python environment under test.
    cfg.gen_dir = None
    if any("@GEN@" in a for r in rows for a in r[3:]):
        cfg.gen_dir = tempfile.mkdtemp(prefix="domainator_gen_")
        subprocess.run([cfg.python, str(REPO / "scripts" / "parity" / "generated_fixtures.py"),
                        "--data_dir", cfg.data_dir, "--test_dir", cfg.test_dir,
                        "-o", cfg.gen_dir], check=True, capture_output=True, env=cfg.py_env)

    tally = {k: 0 for k in ("identical", "tolerated", "differs", "rust-failed", "error", "skipped")}
    found = {}
    rejected = 0
    report = open(cfg.report, "w") if cfg.report else None
    if report:
        report.write("module\tkey\tsource\toutcome\tnote\n")
    for row in rows:
        outcome, note = run_one(row, cfg, helpers, SeqIO)
        tally[outcome] += 1
        rejected += note == "python rejected the invocation"
        if report:
            report.write(f"{row[0]}\t{row[1]}\t{row[2]}\t{outcome}\t{note}\n")
            report.flush()
        if cfg.verbose:
            print(f"  {outcome:12} {row[0]}.{row[1]:52} {note}", flush=True)
        if outcome not in OK:
            found[(row[0], row[1])] = (outcome, "", note)

    if report:
        report.close()
    if cfg.gen_dir:
        shutil.rmtree(cfg.gen_dir, ignore_errors=True)

    print()
    for label, key in (("identical (bytes)", "identical"),
                       ("equal modulo search numerics", "tolerated"),
                       ("REAL DIFFERENCES", "differs"),
                       ("rust failed to run", "rust-failed"),
                       ("harness errors", "error"),
                       ("skipped", "skipped")):
        print(f"  {label:30}: {tally[key]}")

    # A broken Python side does not fail loudly: an entry point that exits 0 having
    # written nothing turns every row into `skipped`, and a sweep that compared
    # nothing would otherwise pass. An `--only` run of one tool can legitimately skip
    # everything, when all its tests build their inputs at test time, but then Python
    # rejected those invocations rather than running them.
    if rows and tally["skipped"] == len(rows) and not (cfg.only and rejected):
        print("\nFAILED: every invocation was skipped -- the Python side ran nothing. "
              f"Check that `{cfg.python} -m domainator <tool>` works.")
        return 2

    baseline = load_baseline(cfg.baseline)

    if cfg.update_baseline:
        # Carry the hand-written classification forward; only membership and the
        # outcome are regenerated.
        found = {k: (v[0], baseline.get(k, ("", "", ""))[1], v[2]) for k, v in found.items()}
        if cfg.only:
            # A filtered run must not silently drop the gaps it never looked at.
            merged = {k: v for k, v in baseline.items() if k[0] not in cfg.only}
            merged.update(found)
            found = merged
        write_baseline(cfg.baseline, found)
        untriaged = sum(1 for v in found.values() if not v[1])
        print(f"\nwrote {len(found)} tracked gaps to {os.path.relpath(cfg.baseline, REPO)}")
        if untriaged:
            print(f"  {untriaged} have an empty `class` column -- triage them by hand")
        return 0

    # Only the outcome is part of the contract; notes carry line numbers and
    # comparator detail that churn without meaning a regression.
    scope = (lambda k: k[0] in cfg.only) if cfg.only else (lambda k: True)
    baseline = {k: v for k, v in baseline.items() if scope(k)}
    new = {k: v for k, v in found.items() if baseline.get(k, ("",))[0] != v[0]}
    fixed = {k: v for k, v in baseline.items() if found.get(k, ("",))[0] != v[0]}
    if not new and not fixed:
        where = os.path.relpath(cfg.baseline, REPO)
        if not found:
            print(f"\nOK -- nothing diverged, and {where} lists nothing for this scope")
            return 0
        print(f"\nOK -- {len(found)} tracked gaps, all as recorded in {where}:")
        by_class = {}
        for key in found:
            by_class.setdefault(baseline[key][1] or "(untriaged)", []).append(key)
        for klass, keys in sorted(by_class.items(), key=lambda kv: (-len(kv[1]), kv[0])):
            print(f"    {len(keys):3}  {klass}")
        return 0
    for (module, key), (outcome, _klass, note) in sorted(new.items()):
        print(f"\n  NEW GAP          {module}.{key} [{outcome}] {note}")
    for (module, key), (outcome, _klass, _note) in sorted(fixed.items()):
        print(f"\n  NO LONGER A GAP  {module}.{key} [{outcome}] -- drop it from the list")
    print("\nFAILED: run with --update_baseline once the change is intended.")
    return 1


if __name__ == "__main__":
    sys.exit(main())
