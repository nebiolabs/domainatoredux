#!/usr/bin/env python3
"""Harvest CLI invocations out of upstream domainator's pytest suite.

Upstream's tests are the catalog of flag combinations that matter -- far more
combinations than we would think to write by hand, and they grow with upstream.
Rather than porting their *assertions* (which encode Python-side internals), we
mine their `main([...])` calls and replay the same argument lists through both
implementations; `differential.py` does the replaying and the comparing.

Emits one TSV row per invocation:

    <module>\t<test_fn>#<n>\t<file>:<lineno>\t<arg>\t<arg>...

The second column is the stable key (the n-th invocation inside that test
function, prefixed with the test module's name when it is not the tool's own
`test_<tool>.py`); the third is human-readable provenance only. Line numbers move every
time upstream touches a test file, so `known_gaps.tsv` keys on column 2.

with three placeholders substituted at replay time:

    @DATA@  the fixture directory  (pytest's `shared_datadir`)
    @TMP@   a per-invocation scratch directory (`output_dir` and friends)
    @OUT@   the output path, given a distinct value per implementation

A `@pytest.mark.parametrize`d test is harvested once per parameter set, keyed
`<test_fn>[<pytest id>]#<n>`, so `pytest -k` finds the same case.

An invocation is dropped when any argument fails to resolve to a literal (a
loop variable, a helper call, a parameter that is not a literal). That is
deliberate: a half-resolved command line is worse than a missing one. See
`docs/parity.md` for how to widen the resolver when a newly interesting test is
being skipped.
"""
import argparse
import ast
import itertools
import json
import pathlib
import subprocess
import sys

REPO = pathlib.Path(__file__).resolve().parents[2]
MANIFEST = REPO / "scripts" / "parity" / "upstream_manifest.json"
DEFAULT_RUST = REPO / "target" / "debug" / "domainatoredux"


def subcommands(rust):
    """The subcommands our binary offers, from the `Commands:` section of its --help."""
    help_text = subprocess.run([str(rust), "--help"], capture_output=True, text=True,
                               check=True).stdout
    section = help_text.split("Commands:", 1)[1].split("\n\n", 1)[0]
    return {line.split()[0] for line in section.splitlines() if line.strip()}


def ported_tools(rust):
    """Upstream's tools (its manifest snapshot) that our binary also offers.

    Subcommand names equal upstream's tool names, so this needs no mapping table. A
    tool upstream has and we do not is not harvested; the manifest golden test is
    where an unported tool is accounted for.
    """
    upstream = {t["name"] for t in json.loads(MANIFEST.read_text())["tools"]}
    return sorted(upstream & subcommands(rust))

# Fixture names pytest injects, which the replay substitutes for.
RESERVED = {"shared_datadir": "@DATA@", "output_dir": "@TMP@", "tmp_path": "@TMP@"}

# Locals that name the *output* path, used only as a fallback when the binding
# itself will not resolve. Upstream is consistent enough about these names that
# a list beats trying to infer direction from the surrounding call.
OUT_NAMES = {
    "out", "output", "out_path", "outfile", "out_file", "out_gb", "out_tsv",
    "out_hmm", "out_json", "out_html", "outpath", "out1", "out2",
}


# A name bound to Python's None (a parameter), which renders as no string but can be
# tested with `is None`.
NONE = object()


def _literal(el):
    """A parameter value: a string, a list of strings, NONE, or None if not a literal."""
    if isinstance(el, ast.Constant):
        if el.value is None:
            return NONE
        return str(el.value) if isinstance(el.value, (str, int, float, bool)) else None
    if isinstance(el, (ast.List, ast.Tuple)):
        items = [_literal(e) for e in el.elts]
        return items if all(isinstance(i, str) for i in items) else None
    return None


def _pytest_id(el, value, name, index):
    """The id pytest gives one parameter value: the value itself for a scalar, else
    `<name><index>`."""
    if isinstance(el, ast.Constant) and isinstance(el.value, (str, int, float, bool, type(None))):
        return str(el.value)
    return f"{name}{index}"


def _parametrize(decorator):
    """A `@pytest.mark.parametrize(names, values)` decorator's cases, each a
    `(pytest id, {name: value})`, or None for any other decorator."""
    if not (isinstance(decorator, ast.Call)
            and ast.unparse(decorator.func) == "pytest.mark.parametrize"
            and len(decorator.args) >= 2
            and isinstance(decorator.args[0], ast.Constant)
            and isinstance(decorator.args[1], (ast.List, ast.Tuple))):
        return None
    names = [n.strip() for n in decorator.args[0].value.split(",") if n.strip()]
    cases = []
    for index, case in enumerate(decorator.args[1].elts):
        explicit_id = None
        if isinstance(case, ast.Call) and ast.unparse(case.func) == "pytest.param":
            explicit_id = next((k.value.value for k in case.keywords
                                if k.arg == "id" and isinstance(k.value, ast.Constant)), None)
            values = list(case.args) if len(names) > 1 else case.args[:1]
        else:
            values = list(case.elts) if len(names) > 1 and isinstance(case, (ast.List, ast.Tuple)) else [case]
        if len(values) != len(names):
            return None
        bound = {}
        for name, el in zip(names, values):
            value = _literal(el)
            if value is not None:
                bound[name] = value
        case_id = explicit_id or "-".join(
            _pytest_id(el, bound.get(name), name, index) for name, el in zip(names, values))
        cases.append((case_id, bound))
    return cases


def parameter_sets(fn):
    """`(id suffix, bindings)` for each case of a test function: `("", {})` alone when it
    is not parametrized. Stacked decorators multiply, the one nearest the function
    first in the id, as pytest names them."""
    decorators = [c for c in (_parametrize(d) for d in reversed(fn.decorator_list)) if c is not None]
    if not decorators:
        return [("", {})]
    out = []
    for combo in itertools.product(*decorators):
        bound = {}
        for _, values in combo:
            bound.update(values)
        out.append(("[" + "-".join(case_id for case_id, _ in combo) + "]", bound))
    return out


class Resolver:
    """Renders an AST expression to a literal string, or None if it cannot.

    Scoped to a single test function (and parameter set): `bind` collects that
    function's simple assignments first, so `gb = shared_datadir / "x.gb"` resolves when
    `gb` is later handed to `main([...])`. Names may also hold lists of strings, so an
    argument list built as `base + ["--hdf_out", out]` resolves too.
    """

    def __init__(self, params=None):
        self.env = dict(RESERVED)
        self.env.update(params or {})

    def bind(self, node):
        if not isinstance(node, ast.Assign) or len(node.targets) != 1:
            return
        target = node.targets[0]
        if isinstance(target, ast.Tuple) and isinstance(node.value, ast.Tuple):
            pairs = list(zip(target.elts, node.value.elts)) if len(target.elts) == len(node.value.elts) else []
        else:
            pairs = [(target, node.value)]
        for name, value_node in pairs:
            if not isinstance(name, ast.Name) or name.id in RESERVED:
                # `output_dir = "test_out"` must not shadow the scratch placeholder:
                # the two implementations need separate directories to write into.
                continue
            value = self.render(value_node)
            if value is None:
                value = self.render_list(value_node)
            if value is not None:
                self.env[name.id] = value

    def render_list(self, el):
        """A list-valued expression as a list of strings, or None."""
        if isinstance(el, (ast.List, ast.Tuple)):
            items = [self.render(e) for e in el.elts]
            return None if None in items else items
        if isinstance(el, ast.Name):
            value = self.env.get(el.id)
            return value if isinstance(value, list) else None
        if isinstance(el, ast.BinOp) and isinstance(el.op, ast.Add):
            left, right = self.render_list(el.left), self.render_list(el.right)
            return None if left is None or right is None else left + right
        if isinstance(el, ast.IfExp):
            test = self.truth(el.test)
            return None if test is None else self.render_list(el.body if test else el.orelse)
        return None

    def truth(self, el):
        """A condition over parameters (`x is None`, `"--cpu" not in args`), or None."""
        if not (isinstance(el, ast.Compare) and len(el.ops) == 1):
            return None
        op, right = el.ops[0], el.comparators[0]
        if isinstance(op, (ast.Is, ast.IsNot)) and isinstance(right, ast.Constant) and right.value is None:
            if not isinstance(el.left, ast.Name) or el.left.id not in self.env:
                return None
            is_none = self.env[el.left.id] is NONE
            return is_none if isinstance(op, ast.Is) else not is_none
        if isinstance(op, (ast.In, ast.NotIn)):
            item, container = self.render(el.left), self.render_list(right)
            if item is None or container is None:
                return None
            return (item in container) == isinstance(op, ast.In)
        return None

    def render(self, el):
        if isinstance(el, ast.Constant):
            return str(el.value)
        if isinstance(el, ast.Name):
            if el.id in self.env:
                value = self.env[el.id]
                return value if isinstance(value, str) else None
            # An unresolvable output path still replays: the runner picks a name.
            return "@OUT@" if el.id in OUT_NAMES else None
        if isinstance(el, ast.Call) and isinstance(el.func, ast.Name) and el.func.id == "str":
            return self.render(el.args[0]) if el.args else None
        if isinstance(el, ast.Call) and ast.unparse(el.func) == "os.path.join" and el.args:
            parts = [self.render(a) for a in el.args]
            return None if None in parts else "/".join(p.rstrip("/") for p in parts)
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


def _is_main_call(node, module, qualified_only=False):
    """True for `main([...])`, `<module>.main([...])` or `<module>_main([...])`.

    `qualified_only` accepts `<module>.main([...])` alone: in another tool's test module a
    bare `main` is that tool's own."""
    if isinstance(node.func, ast.Attribute):
        return (
            node.func.attr == "main"
            and isinstance(node.func.value, ast.Name)
            and node.func.value.id == module
        )
    if isinstance(node.func, ast.Name) and not qualified_only:
        return node.func.id in ("main", f"{module}_main")
    return False


def _is_fixture(fn):
    return any(ast.unparse(d.func if isinstance(d, ast.Call) else d) == "pytest.fixture"
               for d in fn.decorator_list)


def module_fixtures(tree):
    """`{name: value}` for the module's `@pytest.fixture` functions that return a literal
    path, such as `return str(shared_datadir / "structures" / "inputs")`.

    A test taking that fixture as a parameter then resolves it like a local. A fixture may
    use an earlier one."""
    fixtures = {}
    for fn in tree.body:
        if not (isinstance(fn, ast.FunctionDef) and _is_fixture(fn) and fn.body
                and isinstance(fn.body[-1], ast.Return) and fn.body[-1].value is not None):
            continue
        resolver = Resolver({a.arg: fixtures[a.arg] for a in fn.args.args if a.arg in fixtures})
        for node in fn.body[:-1]:
            resolver.bind(node)
        value = resolver.render(fn.body[-1].value)
        if value is not None:
            fixtures[fn.name] = value
    return fixtures


def harvest(path, module, qualified_only=False):
    tree = ast.parse(pathlib.Path(path).read_text())
    fixtures = module_fixtures(tree)
    found = []
    for fn in ast.walk(tree):
        if not isinstance(fn, (ast.FunctionDef, ast.AsyncFunctionDef)):
            continue
        cases = parameter_sets(fn)
        used = {a.arg: fixtures[a.arg] for a in fn.args.args if a.arg in fixtures}
        by_call = {}  # lineno -> [(suffix, args)], one per case that resolved
        for suffix, params in cases:
            resolver = Resolver({**used, **params})
            for node in ast.walk(fn):
                resolver.bind(node)
            for node in ast.walk(fn):
                if not isinstance(node, ast.Call) or not _is_main_call(node, module, qualified_only):
                    continue
                # Only the single-list form; anything else is a helper, not an invocation.
                if len(node.args) != 1:
                    continue
                args = resolver.render_list(node.args[0])
                if args is not None:
                    by_call.setdefault(node.lineno, []).append((suffix, args))
        for lineno, resolved in by_call.items():
            distinct = {tuple(args) for _, args in resolved}
            if len(resolved) == len(cases) and len(distinct) == 1:
                # The call does not depend on the parameters: one row, keyed by the
                # bare test name.
                found.append((fn.name, lineno, resolved[0][1]))
                continue
            emitted = set()
            for suffix, args in resolved:
                if tuple(args) not in emitted:
                    emitted.add(tuple(args))
                    found.append((fn.name + suffix, lineno, args))
    # `ast.walk` is breadth-first, so sort to get source order -- the ordinal in
    # the stable key depends on it.
    found.sort(key=lambda f: (f[0], f[1]))
    return found


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--test_dir", default="external/domainator/test",
                    help="upstream pytest directory (default: the vendored clone)")
    ap.add_argument("--tools", nargs="+",
                    help="tools whose test modules to harvest (default: every upstream "
                         "tool the --rust binary also offers)")
    ap.add_argument("--rust", default=str(DEFAULT_RUST),
                    help="our binary, asked for its subcommands (default: %(default)s)")
    ap.add_argument("-o", "--output", default="-", help="TSV output path ('-' = stdout)")
    args = ap.parse_args(argv)

    test_dir = pathlib.Path(args.test_dir)
    modules = args.tools or ported_tools(args.rust)

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
        # A tool is also run from other tools' test modules (test_viewer.py builds its
        # networks with build_ssn.main), always in the qualified `<tool>.main` form.
        for other in sorted(test_dir.glob("test_*.py")):
            if other == path:
                continue
            seen = {}
            for fn_name, lineno, argv_ in harvest(other, module, qualified_only=True):
                n = seen[fn_name] = seen.get(fn_name, -1) + 1
                rows.append([module, f"{other.stem}.{fn_name}#{n}", f"{other.name}:{lineno}"]
                            + argv_)

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
