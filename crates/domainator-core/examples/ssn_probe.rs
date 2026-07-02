//! Dump SSN core structures (MST edges, cluster counts, merge events, hierarchy
//! roots) as JSON for parity comparison against `domainator.ssn_edges` /
//! `domainator.ssn_hierarchy`.
//!
//! Usage: `cargo run -p domainator-core --example ssn_probe -- <matrix_file>`

use domainator_core::matrix::read_matrix;
use domainator_core::matrix::ssn::{
    build_mst_component_hierarchy, component_size_summary_by_threshold, filter_merge_event_rows,
    threshold_merge_event_rows, MaxTree, MERGE_IMPACT_MIN_CHILD,
};

fn main() {
    let path = std::env::args().nth(1).expect("usage: ssn_probe <matrix_file>");
    let m = read_matrix(&path).expect("read matrix");
    let tree = MaxTree::from_matrix(&m);

    let mst: Vec<(usize, usize, f64)> = tree.mst_edges().collect();
    let ccbt = tree.cluster_count_by_threshold();
    let ebt = tree.edges_by_threshold();
    let summary = component_size_summary_by_threshold(&tree, MERGE_IMPACT_MIN_CHILD);
    let events = filter_merge_event_rows(&threshold_merge_event_rows(&summary), 500);
    let hier = build_mst_component_hierarchy(&tree);

    // Emit compact JSON by hand (avoid a serde dep in the example).
    let mst_json: Vec<String> =
        mst.iter().map(|(i, j, w)| format!("[{i},{j},{}]", fmt(*w))).collect();
    let ccbt_json: Vec<String> =
        ccbt.iter().map(|(t, c)| format!("[{},{}]", fmt(*t), fmt(*c))).collect();
    let ebt_json: Vec<String> =
        ebt.iter().map(|(e, t)| format!("[{},{}]", fmt(*e), fmt(*t))).collect();
    let ev_json: Vec<String> = events
        .iter()
        .map(|e| {
            format!(
                "{{\"edge_index\":{},\"threshold_value\":{},\"merge_impact\":{}}}",
                e.edge_index,
                fmt(e.threshold_value),
                fmt(e.merge_impact)
            )
        })
        .collect();
    let roots_json: Vec<String> = hier.roots.iter().map(|r| r.to_string()).collect();

    println!("{{");
    println!("  \"n_nodes\": {},", tree.n_nodes);
    println!("  \"n_mst_edges\": {},", mst.len());
    println!("  \"mst_edges\": [{}],", mst_json.join(","));
    println!("  \"cluster_count_by_threshold\": [{}],", ccbt_json.join(","));
    println!("  \"edges_by_threshold\": [{}],", ebt_json.join(","));
    println!("  \"merge_events\": [{}],", ev_json.join(","));
    println!("  \"n_hier_nodes\": {},", hier.nodes.len());
    println!("  \"roots\": [{}],", roots_json.join(","));
    println!("  \"leaf_order\": [{}]", hier.leaf_order.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(","));
    println!("}}");
}

fn fmt(x: f64) -> String {
    if x.is_infinite() {
        if x < 0.0 { "-Infinity".into() } else { "Infinity".into() }
    } else {
        format!("{x}")
    }
}
