#!/usr/bin/env python3
"""Keep the committed fixture copy in step with upstream's `test/data`.

`test/data/` is a committed subset of `external/domainator/test/data` -- exactly
the files the harvested invocations reference. Committing them means the
differential harness (and anything else that wants a real GenBank/HMM/matrix to
chew on) works without the gitignored clone, and that the fixtures cannot shift
underfoot between runs. Upstream is MIT-licensed; see `test/data/README.md`.

  --check   fail if a committed fixture no longer matches upstream, or if the
            harvest references a fixture we do not hold
  --update  copy the referenced set over from the clone
  --prune   with --update, also delete committed fixtures nothing references
"""
import argparse
import filecmp
import pathlib
import shutil
import subprocess
import sys
import tempfile

REPO = pathlib.Path(__file__).resolve().parents[2]


def referenced_fixtures(test_dir):
    """Fixture paths (relative to the data dir) that harvested invocations use, and
    the hand-written ones in `extra_invocations.tsv`. The harvest covers the tools
    `target/debug/domainatoredux` offers, so build it first."""
    with tempfile.NamedTemporaryFile(suffix=".tsv") as tmp:
        subprocess.run([sys.executable, str(REPO / "scripts/parity/harvest_invocations.py"),
                        "--test_dir", str(test_dir), "-o", tmp.name],
                       check=True, stderr=subprocess.DEVNULL)
        rows = pathlib.Path(tmp.name).read_text().splitlines()
    extra = REPO / "scripts/parity/extra_invocations.tsv"
    rows += [r for r in extra.read_text().splitlines() if r and not r.startswith("#")]
    needed = set()
    for row in rows:
        for arg in row.split("\t")[3:]:
            if arg.startswith("@DATA@/"):
                needed.add(arg[len("@DATA@/"):])
    return needed


def _git_tracked(src):
    """Paths under `src` that upstream's git index knows about, or None."""
    proc = subprocess.run(["git", "-C", str(src), "ls-files"], capture_output=True, text=True)
    if proc.returncode != 0:
        return None
    return set(proc.stdout.split())


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--upstream", default=str(REPO / "external/domainator/test"),
                    help="upstream test directory (the vendored clone)")
    ap.add_argument("--dest", default=str(REPO / "test/data"))
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--update", action="store_true")
    ap.add_argument("--prune", action="store_true",
                    help="with --update, delete fixtures nothing references")
    args = ap.parse_args(argv)
    if not (args.check or args.update):
        ap.error("pass --check or --update")

    upstream = pathlib.Path(args.upstream)
    src = upstream / "data"
    dest = pathlib.Path(args.dest)
    if not src.is_dir():
        sys.exit(f"no upstream fixtures at {src} -- clone it first "
                 f"(see scripts/sync_upstream.sh)")

    # An invocation may reference a *directory* (`--taxonomy .../taxdmp`); expand
    # it to the files it holds so everything below works on files alone.
    tracked = _git_tracked(src)
    needed = set()
    for rel in referenced_fixtures(upstream):
        if (src / rel).is_dir():
            # A directory reference (`--taxonomy .../taxdmp`) expands to its files,
            # minus anything a previous test run dropped there (`.lock` files and
            # the like) -- which is why this consults upstream's git index.
            needed |= {str(f.relative_to(src)) for f in (src / rel).rglob("*")
                       if f.is_file() and (tracked is None or str(f.relative_to(src)) in tracked)}
        else:
            needed.add(rel)
    held = {str(p.relative_to(dest)) for p in dest.rglob("*")
            if p.is_file() and p.name != "README.md"}

    missing = sorted(f for f in needed if f not in held)
    stale = sorted(f for f in needed & held if not filecmp.cmp(src / f, dest / f, shallow=False))
    absent_upstream = sorted(f for f in needed if not (src / f).exists())
    unreferenced = sorted(held - needed)

    if args.check:
        problems = False
        for label, items in (("not committed", missing),
                             ("differs from upstream", stale),
                             ("referenced but absent upstream", absent_upstream)):
            if items:
                problems = True
                print(f"  {label}: {len(items)}")
                for item in items:
                    print(f"    {item}")
        if unreferenced:
            # Not a failure: a fixture may be used by a hand-written Rust test.
            print(f"  note: {len(unreferenced)} committed fixture(s) no longer referenced "
                  f"by any harvested or extra invocation (--update --prune to drop them)")
        if problems:
            print("\nFAILED: run scripts/parity/sync_fixtures.py --update")
            return 1
        print(f"OK -- {len(needed)} fixtures in step with upstream")
        return 0

    dest.mkdir(parents=True, exist_ok=True)
    for rel in missing + stale:
        if not (src / rel).exists():
            continue
        (dest / rel).parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(src / rel, dest / rel)
    print(f"copied {len(missing)} new and {len(stale)} changed fixture(s)")
    if args.prune:
        for rel in unreferenced:
            (dest / rel).unlink()
        print(f"pruned {len(unreferenced)} unreferenced fixture(s)")
    elif unreferenced:
        print(f"kept {len(unreferenced)} unreferenced fixture(s) (--prune to drop them)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
