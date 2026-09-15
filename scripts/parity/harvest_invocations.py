#!/usr/bin/env python3
"""Harvest CLI invocations out of upstream domainator's pytest suite.

Upstream's tests are the catalogue of flag combinations that matter -- far more
combinations than we would think to write by hand, and they grow with upstream.
Rather than porting their *assertions* (which encode Python-side internals), we
mine their `main([...])` calls and replay the same argument lists through both
implementations; `differential.py` does the replaying and the comparing.

Emits one TSV row per invocation:

    <module>\t<test_fn>#<n>\t<file>:<lineno>\t<arg>\t<arg>...

The second column is the stable key (the n-th invocation inside that test
function); the third is human-readable provenance only. Line numbers move every
time upstream touches a test file, so `known_gaps.tsv` keys on column 2.

with three placeholders substituted at replay time:

    @DATA@  the fixture directory  (pytest's `shared_datadir`)
    @TMP@   a per-invocation scratch directory (`output_dir` and friends)
    @OUT@   the output path, given a distinct value per implementation

An invocation is dropped when any argument fails to resolve to a literal (a
loop variable, a helper call, a parametrized fixture). That is deliberate: a
half-resolved command line is worse than a missing one. See `docs/parity.md`
for how to widen the resolver when a newly interesting test is being skipped.
"""
import argparse
import ast
import pathlib
import sys

# Fixture names pytest injects, which the replay substitutes for.
RESERVED = {"shared_datadir": "@DATA@", "output_dir": "@TMP@"}

# Locals that name the *output* path, used only as a fallback when the binding
# itself will not resolve. Upstream is consistent enough about these names that
# a list beats trying to infer direction from the surrounding call.
OUT_NAMES = {
    "out", "output", "out_path", "outfile", "out_file", "out_gb", "out_tsv",
    "out_hmm", "out_json", "out_html", "outpath", "out1", "out2",
}


class Resolver:
    """Renders an AST expression to a literal string, or None if it cannot.

    Scoped to a single test function: `bind` collects that function's simple
    assignments first, so `gb = shared_datadir / "x.gb"` resolves when `gb` is
    later handed to `main([...])`.
    """

    def __init__(self):
        self.env = dict(RESERVED)

    def bind(self, node):
        if not isinstance(node, ast.Assign) or len(node.targets) != 1:
            return
        target = node.targets[0]
        if not isinstance(target, ast.Name) or target.id in RESERVED:
            # `output_dir = "test_out"` must not shadow the scratch placeholder:
            # the two implementations need separate directories to write into.
            return
        value = self.render(node.value)
        if value is not None:
            self.env[target.id] = value

    def render(self, el):
        if isinstance(el, ast.Constant):
            return str(el.value)
        if isinstance(el, ast.Name):
            if el.id in self.env:
                return self.env[el.id]
            # An unresolvable output path still replays: the runner picks a name.
            return "@OUT@" if el.id in OUT_NAMES else None
        if isinstance(el, ast.Call) and isinstance(el.func, ast.Name) and el.func.id == "str":
            return self.render(el.args[0]) if el.args else None
        if isinstance(el, ast.BinOp) and isinstance(el.op, ast.Div):  # pathlib `/`
            left, right = self.render(el.left), self.render(el.right)
            if left is None or right is None:
                return None
            return left.rstrip("/") + "/" + right
        if isinstance(el, ast.BinOp) and isinstance(el.op, ast.Add):
            left, right = self.render(el.left), self.render(el.right)
            return None if left is None or right is None else left + right
        if isinstance(el, ast.JoinedStr):  # f-string
            parts = []
            for value in el.values:
                if isinstance(value, ast.Constant):
                    parts.append(str(value.value))
                elif isinstance(value, ast.FormattedValue):
                    rendered = self.render(value.value)
                    if rendered is None:
                        return None
                    parts.append(rendered)
                else:
                    return None
            return "".join(parts)
        return None


def _is_main_call(node, module):
    """True for `main([...])`, `<module>.main([...])` or `<module>_main([...])`."""
    if isinstance(node.func, ast.Attribute):
        return (
            node.func.attr == "main"
            and isinstance(node.func.value, ast.Name)
            and node.func.value.id == module
        )
    if isinstance(node.func, ast.Name):
        return node.func.id in ("main", f"{module}_main")
    return False


def harvest(path, module):
    tree = ast.parse(pathlib.Path(path).read_text())
    found = []
    for fn in ast.walk(tree):
        if not isinstance(fn, (ast.FunctionDef, ast.AsyncFunctionDef)):
            continue
        resolver = Resolver()
        for node in ast.walk(fn):
            resolver.bind(node)
        for node in ast.walk(fn):
            if not isinstance(node, ast.Call) or not _is_main_call(node, module):
                continue
            # Only the single-list form; anything else is a helper, not an invocation.
            if len(node.args) != 1 or not isinstance(node.args[0], ast.List):
                continue
            args = []
            for el in node.args[0].elts:
                rendered = resolver.render(el)
                if rendered is None:
                    args = None
                    break
                args.append(rendered)
            if args is not None:
                found.append((fn.name, node.lineno, args))
    # `ast.walk` is breadth-first, so sort to get source order -- the ordinal in
    # the stable key depends on it.
    found.sort(key=lambda f: (f[0], f[1]))
    return found


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--test_dir", default="external/domainator/test",
                    help="upstream pytest directory (default: the vendored clone)")
    ap.add_argument("--toolmap", default="scripts/parity/toolmap.tsv",
                    help="TSV of <python module>\\t<domainator subcommand>")
    ap.add_argument("-o", "--output", default="-", help="TSV output path ('-' = stdout)")
    args = ap.parse_args(argv)

    test_dir = pathlib.Path(args.test_dir)
    modules = [line.split("\t")[0]
               for line in pathlib.Path(args.toolmap).read_text().splitlines() if line.strip()]

    rows, skipped = [], []
    for module in modules:
        path = test_dir / f"test_{module}.py"
        if not path.exists():
            skipped.append(module)
            continue
        seen = {}
        for fn_name, lineno, argv_ in harvest(path, module):
            n = seen[fn_name] = seen.get(fn_name, -1) + 1
            rows.append([module, f"{fn_name}#{n}", f"{path.name}:{lineno}"] + argv_)

    # Sort so the harvest is diffable across upstream revisions.
    rows.sort(key=lambda r: (r[0], r[1]))
    out = sys.stdout if args.output == "-" else open(args.output, "w")
    try:
        for row in rows:
            out.write("\t".join(row) + "\n")
    finally:
        if out is not sys.stdout:
            out.close()

    print(f"harvested {len(rows)} invocations from {len(modules) - len(skipped)} test modules",
          file=sys.stderr)
    if skipped:
        print(f"no test module for: {', '.join(skipped)}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
