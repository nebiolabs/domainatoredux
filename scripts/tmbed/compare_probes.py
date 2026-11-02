#!/usr/bin/env python3
"""Compare two `tmbed_probe` outputs: labels must agree; report per-protein label
mismatches and the largest probability difference.

    python scripts/tmbed/compare_probes.py EXPECTED.json ACTUAL.json
"""
import json
import sys


def main(expected, actual):
    a, b = json.load(open(expected)), json.load(open(actual))
    assert [p["id"] for p in a] == [p["id"] for p in b], "different proteins"
    worst = 0.0
    mismatched = 0
    for p, q in zip(a, b):
        diff = max((abs(x - y) for r, s in zip(p["probabilities"], q["probabilities"])
                    for x, y in zip(r, s)), default=0.0)
        worst = max(worst, diff)
        bad = [i for i, (x, y) in enumerate(zip(p["labels"], q["labels"])) if x != y]
        if bad or len(p["labels"]) != len(q["labels"]):
            mismatched += 1
            print(f"{p['id']}: labels differ at {bad[:10]}\n  {p['labels']}\n  {q['labels']}")
    print(f"{len(a)} proteins, {mismatched} with label differences, "
          f"max probability difference {worst:.2e}")
    return 1 if mismatched else 0


if __name__ == "__main__":
    sys.exit(main(*sys.argv[1:]))
