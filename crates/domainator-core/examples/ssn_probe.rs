//! Dump the SSN core (MST, threshold tables, merge events, moving sum, slider stops,
//! hierarchy) as JSON for parity comparison against `domainator.ssn_edges` /
//! `domainator.ssn_hierarchy`.
//!
//! The Python twin is `scripts/ssn_probe.py`; both print the SAME document, so
//! `scripts/json_parity.py` can diff them. **Keep the two in lockstep** — a field added
//! to one must be added to the other, or the gate silently stops covering it.
//!
//! Usage: `cargo run -p domainator-core --example ssn_probe -- <matrix_file>`
//! With no argument it falls back to the vendored FeSOD fixture, which is what
//! `scripts/sync_upstream.sh` relies on.

use std::path::PathBuf;

use domainator_core::matrix::read_matrix;
use domainator_core::matrix::ssn::{
    build_mst_component_hierarchy, component_size_summary_by_threshold,
    filter_merge_event_rows_binned, merge_event_moving_sum, threshold_merge_event_rows,
    threshold_slider_stops, MaxTree, MERGE_EVENT_DENSITY_BINS, MERGE_IMPACT_MIN_CHILD,
    MOVING_SUM_GRID_POINTS, MOVING_SUM_WINDOW_FRACTION,
};

const DEFAULT_FIXTURE: &str = "external/domainator/test/data/FeSOD_dist.dense.hdf5";

/// JSON scalar: non-finite becomes `null`, integral floats become ints — matching the
/// Python probe's `num()` so the two documents compare equal.
fn num(x: f64) -> String {
    if !x.is_finite() {
        return "null".into();
    }
    if x.fract() == 0.0 && x.abs() < 1e16 {
        return format!("{}", x as i64);
    }
    format!("{x}")
}

fn arr(items: &[String]) -> String {
    format!("[{}]", items.join(","))
}

fn main() {
    let path = match std::env::args().nth(1) {
        Some(p) => PathBuf::from(p),
        None => {
            // Resolve the fixture relative to the repo root (this crate is two deep).
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(|p| p.parent())
                .map(PathBuf::from)
                .unwrap_or_default();
            let fallback = root.join(DEFAULT_FIXTURE);
            if !fallback.exists() {
                eprintln!(
                    "ssn_probe: no matrix given and the default fixture is missing:\n  {}\n\
                     Pass a matrix explicitly, or run scripts/sync_upstream.sh to populate\n\
                     external/domainator (it is gitignored).",
                    fallback.display()
                );
                std::process::exit(2);
            }
            fallback
        }
    };

    let m = match read_matrix(&path.to_string_lossy()) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("ssn_probe: could not read {}: {e}", path.display());
            std::process::exit(2);
        }
    };

    let max_merge_events: usize = std::env::args()
        .nth(2)
        .and_then(|v| v.parse().ok())
        .unwrap_or(500);

    let tree = MaxTree::from_matrix(&m);
    let summary = component_size_summary_by_threshold(&tree, MERGE_IMPACT_MIN_CHILD);
    let all_rows = threshold_merge_event_rows(&summary);
    // The moving sum MUST see the uncapped rows; filtering first undercounts exactly
    // the small events the series exists to reveal.
    let moving = merge_event_moving_sum(&all_rows, MOVING_SUM_WINDOW_FRACTION, MOVING_SUM_GRID_POINTS);
    let events = filter_merge_event_rows_binned(&all_rows, max_merge_events, MERGE_EVENT_DENSITY_BINS);
    let stops = threshold_slider_stops(&events, Some(&tree));
    let hier = build_mst_component_hierarchy(&tree);

    let mst: Vec<String> = tree
        .mst_edges()
        .map(|(i, j, w)| format!("[{i},{j},{}]", num(w)))
        .collect();
    let pair = |v: &Vec<(f64, f64)>| -> String {
        arr(&v.iter().map(|(a, b)| format!("[{},{}]", num(*a), num(*b))).collect::<Vec<_>>())
    };
    let events_json: Vec<String> = events
        .iter()
        .map(|e| {
            let counts: Vec<String> = e
                .merge_size_counts
                .iter()
                .map(|(size, count)| format!("\"{}\":{}", num(*size), count))
                .collect();
            format!(
                "{{\"delta_avg_non_singleton\":{},\"delta_largest\":{},\"edge_index\":{},\
                 \"largest_merge\":{},\"merge_count\":{},\"merge_impact\":{},\
                 \"merge_size_counts\":{{{}}},\"threshold_from\":\"{}\",\
                 \"threshold_from_value\":{},\"threshold_to\":\"{}\",\"threshold_value\":{}}}",
                num(e.delta_avg_non_singleton),
                num(e.delta_largest),
                e.edge_index,
                num(e.largest_merge),
                e.merge_count,
                num(e.merge_impact),
                counts.join(","),
                e.threshold_from,
                num(e.threshold_from_value),
                e.threshold_to,
                num(e.threshold_value),
            )
        })
        .collect();
    let stops_json: Vec<String> = stops
        .iter()
        .map(|s| {
            format!(
                "{{\"edge_index\":{},\"threshold_index\":{},\"threshold_label\":\"{}\",\
                 \"threshold_value\":{}}}",
                s.edge_index,
                s.threshold_index,
                s.threshold_label,
                s.threshold_value.map(num).unwrap_or_else(|| "null".into()),
            )
        })
        .collect();
    let usize_arr =
        |v: &[usize]| arr(&v.iter().map(|x| x.to_string()).collect::<Vec<_>>());
    let u64_arr = |v: &[u64]| arr(&v.iter().map(|x| x.to_string()).collect::<Vec<_>>());

    // Keys are emitted in sorted order to match the Python probe's `sort_keys=True`,
    // so a plain `diff` is also readable (json_parity.py ignores key order anyway).
    print!("{{");
    print!("\"cluster_count_by_edge_count\":{},", pair(&tree.cluster_count_by_edge_count()));
    print!("\"cluster_count_by_threshold\":{},", pair(&tree.cluster_count_by_threshold()));
    print!("\"edges_above_threshold\":{},", u64_arr(tree.edges_above_threshold()));
    print!("\"edges_by_threshold\":{},", pair(&tree.edges_by_threshold()));
    print!(
        "\"hierarchy\":{{\"leaf_order\":{},\"n_nodes\":{},\"roots\":{}}},",
        usize_arr(&hier.leaf_order),
        hier.nodes.len(),
        usize_arr(&hier.roots)
    );
    print!("\"max_merge_events\":{},", max_merge_events);
    print!("\"merge_event_total\":{},", all_rows.len());
    print!("\"merge_events\":{},", arr(&events_json));
    print!(
        "\"merge_moving_sum\":{{\"window\":{},\"x\":{},\"y\":{}}},",
        num(moving.window),
        arr(&moving.x.iter().map(|v| num(*v)).collect::<Vec<_>>()),
        arr(&moving.y.iter().map(|v| num(*v)).collect::<Vec<_>>())
    );
    print!("\"mst_edges\":{},", arr(&mst));
    print!("\"mst_edges_above_threshold\":{},", u64_arr(tree.mst_edges_above_threshold()));
    print!("\"n_edges\":{},", tree.n_edges());
    print!("\"n_nodes\":{},", tree.n_nodes);
    print!("\"slider_stops\":{},", arr(&stops_json));
    print!(
        "\"threshold_row_index\":{},",
        arr(&tree
            .thresholds()
            .iter()
            .map(|&t| tree.threshold_row_index(t).to_string())
            .collect::<Vec<_>>())
    );
    print!(
        "\"thresholds\":{}",
        arr(&tree.thresholds().iter().map(|&t| num(t)).collect::<Vec<_>>())
    );
    println!("}}");
}
