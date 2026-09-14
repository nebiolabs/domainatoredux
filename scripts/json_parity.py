#!/usr/bin/env python3
"""Compare two JSON documents structurally and report the FIRST differing path.

Why this exists: the Rust workspace builds `serde_json` with the `preserve_order`
feature, so Rust emits object keys in insertion order, while Python writes bundles
with `sort_keys=True`. Byte-level `diff` (and gzip-byte comparison) is therefore
guaranteed to fail on files that are semantically identical -- byte equality is NOT
part of the on-disk contract. Every parity gate must compare *parsed* JSON, which is
what this script does.

Floats compare with a relative+absolute tolerance (numpy pairwise summation and a
naive Rust sequential sum differ in the last ulp on large inputs). Integral floats
compare equal to the same integer, so `1` and `1.0` are not spurious failures --
except for object *keys*, which are compared as exact strings, because the bundle
contract distinguishes `"1"` from `"1.0"` in `merge_size_counts`.

Usage:
    json_parity.py EXPECTED.json ACTUAL.json [--rtol 1e-9] [--atol 1e-12] [--max 20]

Exit status is 0 when the documents agree, 1 otherwise.
"""

from __future__ import annotations

import argparse
import json
import math
import sys

NUMBER = (int, float)


def _fmt(value) -> str:
    """Short, unambiguous rendering of a scalar for the diff report."""
    if isinstance(value, str):
        return json.dumps(value)
    if isinstance(value, float) and value.is_integer() and abs(value) < 1e16:
        return f"{value!r}"
    return repr(value)


def _kind(value) -> str:
    if value is None:
        return "null"
    if isinstance(value, bool):
        return "bool"
    if isinstance(value, NUMBER):
        return "number"
    if isinstance(value, str):
        return "string"
    if isinstance(value, list):
        return "array"
    if isinstance(value, dict):
        return "object"
    return type(value).__name__


def _numbers_equal(a, b, rtol: float, atol: float) -> bool:
    af, bf = float(a), float(b)
    if math.isnan(af) and math.isnan(bf):
        return True
    if math.isinf(af) or math.isinf(bf):
        return af == bf
    return abs(af - bf) <= max(atol, rtol * max(abs(af), abs(bf)))


def compare(expected, actual, rtol: float, atol: float, path: str = "$", out=None):
    """Append 'path: expected != actual' strings to `out` for every difference."""
    if out is None:
        out = []

    # bool is a subclass of int; keep them from comparing equal to 0/1.
    if isinstance(expected, bool) != isinstance(actual, bool):
        out.append(f"{path}: type {_kind(expected)} != {_kind(actual)}")
        return out

    if isinstance(expected, NUMBER) and isinstance(actual, NUMBER) \
            and not isinstance(expected, bool):
        if not _numbers_equal(expected, actual, rtol, atol):
            out.append(f"{path}: {_fmt(expected)} != {_fmt(actual)}")
        return out

    if _kind(expected) != _kind(actual):
        out.append(f"{path}: type {_kind(expected)} != {_kind(actual)}")
        return out

    if isinstance(expected, dict):
        # Key ORDER is deliberately not compared (preserve_order vs sort_keys);
        # key SETS and key spellings are.
        for key in sorted(set(expected) - set(actual)):
            out.append(f"{path}.{key}: missing from actual")
        for key in sorted(set(actual) - set(expected)):
            out.append(f"{path}.{key}: unexpected in actual")
        for key in expected:
            if key in actual:
                compare(expected[key], actual[key], rtol, atol, f"{path}.{key}", out)
        return out

    if isinstance(expected, list):
        if len(expected) != len(actual):
            out.append(f"{path}: length {len(expected)} != {len(actual)}")
        for idx in range(min(len(expected), len(actual))):
            compare(expected[idx], actual[idx], rtol, atol, f"{path}[{idx}]", out)
        return out

    if expected != actual:
        out.append(f"{path}: {_fmt(expected)} != {_fmt(actual)}")
    return out


def _load(path: str):
    if path == "-":
        return json.load(sys.stdin)
    with open(path, "rb") as handle:
        raw = handle.read()
    if raw[:2] == b"\x1f\x8b":  # a .dsnv bundle handed over directly
        import gzip
        raw = gzip.decompress(raw)
    return json.loads(raw.decode("utf-8"))


def main() -> None:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("expected", help="reference document (usually Python's output)")
    ap.add_argument("actual", help="document under test (usually Rust's output)")
    ap.add_argument("--rtol", type=float, default=1e-9, help="relative float tolerance")
    ap.add_argument("--atol", type=float, default=1e-12, help="absolute float tolerance")
    ap.add_argument("--max", type=int, default=20, dest="max_report",
                    help="how many differences to print before truncating")
    ap.add_argument("--ignore", action="append", default=[], metavar="PATH",
                    help="JSON path to skip, e.g. '$.domainator_version'. Repeatable. "
                         "Use only for fields that legitimately differ between the two "
                         "implementations (the writing build's own version string), "
                         "never to paper over a real divergence.")
    args = ap.parse_args()

    diffs = compare(_load(args.expected), _load(args.actual), args.rtol, args.atol)
    if args.ignore:
        ignored = set(args.ignore)
        diffs = [d for d in diffs if d.split(":", 1)[0] not in ignored]
    if not diffs:
        print(f"OK: {args.expected} == {args.actual} (rtol={args.rtol})")
        return

    print(f"DIVERGED: {len(diffs)} difference(s) "
          f"({args.expected} -> {args.actual})", file=sys.stderr)
    for line in diffs[:args.max_report]:
        print(f"  {line}", file=sys.stderr)
    if len(diffs) > args.max_report:
        print(f"  ... and {len(diffs) - args.max_report} more", file=sys.stderr)
    sys.exit(1)


if __name__ == "__main__":
    main()
