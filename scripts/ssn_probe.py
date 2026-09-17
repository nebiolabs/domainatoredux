#!/usr/bin/env python3
"""Python twin of `cargo run -p domainator-core --example ssn_probe`.

Both print the SAME JSON document for the same matrix, so `json_parity.py` can prove
the Rust SSN core (crates/domainator-core/src/matrix/ssn.rs) agrees with
`domainator.ssn_edges` / `domainator.ssn_hierarchy` field for field.

Keep this file and examples/ssn_probe.rs in lockstep: a field added to one must be
added to the other, or the gate silently stops covering it.

Usage:
    ssn_probe.py <matrix_file> [--max_merge_events N] [--metric min_child|product]

Run with the domainator conda env, e.g.
    /home/sean/miniforge3/envs/domainator/bin/python scripts/ssn_probe.py m.hdf5
"""

from __future__ import annotations

import argparse
import json
import math
import os
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_SRC = REPO_ROOT / "external" / "domainator" / "src"

# Prefer the vendored copy (the pinned source of truth) over any installed domainator.
if (DEFAULT_SRC / "domainator" / "ssn_edges.py").exists():
    sys.path.insert(0, str(DEFAULT_SRC))

from domainator.data_matrix import DataMatrix                      # noqa: E402
from domainator.ssn_edges import MaxTree                           # noqa: E402
from domainator.ssn_hierarchy import (                             # noqa: E402
    MERGE_EVENT_DENSITY_BINS,
    MERGE_IMPACT_MIN_CHILD,
    MOVING_SUM_GRID_POINTS,
    MOVING_SUM_WINDOW_FRACTION,
    build_mst_component_hierarchy,
    component_size_summary_by_threshold,
    filter_merge_event_rows,
    merge_event_moving_sum,
    threshold_merge_event_rows,
    threshold_slider_stops,
)


def num(value):
    """JSON-ready scalar: non-finite floats become null, integral floats become ints."""
    if value is None:
        return None
    value = float(value)
    if not math.isfinite(value):
        return None
    if value.is_integer() and abs(value) < 1e16:
        return int(value)
    return value


def pairs(table):
    return [[num(a), num(b)] for a, b in table]


def event_json(row):
    """One merge-event row, every field the bundle and matrix_report carry."""
    return {
        "edge_index": int(row["edge_index"]),
        "threshold_from_value": num(row.get("threshold_from_value")),
        "threshold_from": row["threshold_from"],
        "threshold_to": row["threshold_to"],
        "threshold_value": num(row["threshold_value"]),
        "merge_impact": num(row["merge_impact"]),
        "merge_size_counts": {str(k): int(v) for k, v in row["merge_size_counts"].items()},
        "largest_merge": num(row["largest_merge"]),
        "merge_count": int(row["merge_count"]),
        "delta_largest": num(row["delta_largest"]),
        "delta_avg_non_singleton": num(row["delta_avg_non_singleton"]),
    }


def main() -> None:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("matrix")
    ap.add_argument("--max_merge_events", type=int, default=500)
    ap.add_argument("--metric", default=MERGE_IMPACT_MIN_CHILD)
    args = ap.parse_args()

    matrix = DataMatrix.from_file(args.matrix)
    tree = MaxTree(matrix)

    summary = component_size_summary_by_threshold(tree, args.metric)
    all_rows = threshold_merge_event_rows(summary)
    # The moving sum MUST see the uncapped rows; filtering first undercounts exactly
    # the small events the series exists to reveal.
    moving = merge_event_moving_sum(all_rows, MOVING_SUM_WINDOW_FRACTION, MOVING_SUM_GRID_POINTS)
    events = filter_merge_event_rows(all_rows, args.max_merge_events, MERGE_EVENT_DENSITY_BINS)
    # threshold_index_lookup: only a caller holding the threshold tables can index them,
    # and this probe builds its tree from the matrix, so it does.
    stops = threshold_slider_stops(events, tree=tree,
                                   threshold_index_lookup=tree.threshold_row_index)
    hierarchy = build_mst_component_hierarchy(tree)

    # The windowed/pinned selection the viewer's chart uses. Window and pin are derived
    # from the data so sync_upstream.sh needs no extra flags: the middle 40% of the
    # finite threshold axis, pinned at the first dropped event -- which is what makes the
    # pin actually bite rather than being a no-op that proves nothing.
    finite = [float(r["threshold_value"]) for r in all_rows
              if math.isfinite(float(r["threshold_value"]))]
    window = None
    if finite and max(finite) > min(finite):
        lo, hi = min(finite), max(finite)
        window = (lo + 0.3 * (hi - lo), lo + 0.7 * (hi - lo))
    windowed = filter_merge_event_rows(all_rows, args.max_merge_events,
                                       MERGE_EVENT_DENSITY_BINS, window=window)
    kept_thresholds = [float(r["threshold_value"]) for r in windowed]
    pin = next((float(r["threshold_value"]) for r in all_rows
                if float(r["threshold_value"]) not in kept_thresholds), None)
    windowed_pinned = filter_merge_event_rows(all_rows, args.max_merge_events,
                                              MERGE_EVENT_DENSITY_BINS, window=window,
                                              pinned_threshold=pin)

    document = {
        "n_nodes": int(tree.n_nodes),
        "n_edges": int(tree.n_edges),
        "mst_edges": [[int(i), int(j), num(w)] for i, j, w in tree.iter_mst_edges()],
        "thresholds": [num(t) for t in tree.thresholds],
        "edges_above_threshold": [int(v) for v in tree.edges_above_threshold],
        "mst_edges_above_threshold": [int(v) for v in tree.mst_edges_above_threshold],
        "edges_by_threshold": pairs(tree.edges_by_threshold),
        "cluster_count_by_threshold": pairs(tree.cluster_count_by_threshold),
        "cluster_count_by_edge_count": pairs(tree.cluster_count_by_edge_count),
        "threshold_row_index": [
            int(tree.threshold_row_index(t)) for t in tree.thresholds
        ],
        "merge_event_total": len(all_rows),
        "max_merge_events": int(args.max_merge_events),
        "merge_events": [event_json(r) for r in events],
        "merge_events_windowed": {
            "window": None if window is None else [num(window[0]), num(window[1])],
            "pinned_threshold": num(pin),
            "selected": [int(r["edge_index"]) for r in windowed],
            "selected_pinned": [int(r["edge_index"]) for r in windowed_pinned],
        },
        "merge_moving_sum": {
            "window": num(moving["window"]),
            "x": [num(v) for v in moving["x"]],
            "y": [num(v) for v in moving["y"]],
        },
        "slider_stops": [
            {
                "edge_index": int(s["edge_index"]),
                "threshold_index": int(s["threshold_index"]),
                "threshold_label": s["threshold_label"],
                "threshold_value": num(s["threshold_value"]),
            }
            for s in stops
        ],
        "hierarchy": {
            "n_nodes": len(hierarchy["nodes"]),
            "roots": [int(r) for r in hierarchy["roots"]],
            "leaf_order": [int(v) for v in hierarchy["leaf_order"]],
        },
    }
    json.dump(document, sys.stdout, separators=(",", ":"), sort_keys=True)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
