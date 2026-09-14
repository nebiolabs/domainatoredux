//! `matrix-report`: summarize a similarity matrix to help choose an SSN edge
//! threshold. Mirrors `domainator.matrix_report` (text path) and adds an
//! agent-friendly `--json` payload with the threshold→cluster curve, ranked
//! merge events, and suggested cutoffs. (`--include_mst_knn` projections and the
//! interactive HTML are deferred.)

use serde_json::{json, Value};

use crate::matrix::ssn::{
    component_size_summary_by_threshold, edge_scores_desc, filter_merge_event_rows,
    format_threshold_value, merge_event_moving_sum, threshold_merge_event_rows,
    threshold_slider_stops, MaxTree, MovingSum, SliderStop, MOVING_SUM_GRID_POINTS,
    MOVING_SUM_WINDOW_FRACTION,
    MergeEvent, DEFAULT_MAX_MERGE_EVENTS, MERGE_IMPACT_MIN_CHILD,
};
use crate::matrix::DataMatrix;

/// Computed report over a similarity matrix.
pub struct MatrixReport {
    pub n_nodes: usize,
    pub n_edges: usize,
    pub mean: f64,
    pub median: f64,
    pub min: f64,
    pub max: f64,
    pub scores_desc: Vec<f64>,
    pub cluster_count_by_threshold: Vec<(f64, f64)>,
    pub cluster_count_by_edge_count: Vec<(f64, f64)>,
    pub edges_by_threshold: Vec<(f64, f64)>,
    pub merge_impact_metric: String,
    /// The capped selection of split events (see `merge_event_total`).
    pub merge_events: Vec<MergeEvent>,
    /// How many split events the network actually has, before `max_merge_events`
    /// capped `merge_events`. Without it, a stretch of axis with nothing plotted on
    /// it is indistinguishable from a stretch where nothing happens.
    pub merge_event_total: usize,
    /// The cap that was applied; `0` means none.
    pub max_merge_events: usize,
    /// Centred moving sum of `merge_impact`, derived from the **uncapped** rows.
    pub merge_moving_sum: MovingSum,
    /// Threshold cuts the report's slider offers, ending at a reachable floor.
    pub slider_stops: Vec<SliderStop>,
    pub mst_edges: Vec<(usize, usize, f64)>,
    /// Distinct thresholds, descending — lets a consumer map a threshold back to its
    /// row in the tables above without holding the whole `MaxTree`.
    pub thresholds: Vec<f64>,
}

/// Build the report. `merge_impact_metric` is `min_child`/`product`;
/// `max_merge_events` caps the retained split events (0 = all).
pub fn matrix_report(
    matrix: &DataMatrix,
    merge_impact_metric: &str,
    max_merge_events: usize,
) -> MatrixReport {
    let scores = edge_scores_desc(matrix); // descending
    let n = scores.len();
    let mean = if n > 0 { scores.iter().sum::<f64>() / n as f64 } else { 0.0 };
    let median = if n == 0 {
        0.0
    } else if n % 2 == 1 {
        scores[n / 2]
    } else {
        (scores[n / 2 - 1] + scores[n / 2]) / 2.0
    };
    let max = scores.first().copied().unwrap_or(0.0);
    let min = scores.last().copied().unwrap_or(0.0);

    let tree = MaxTree::from_matrix(matrix);
    let summary = component_size_summary_by_threshold(&tree, merge_impact_metric);
    // The uncapped rows are kept: `merge_event_total` reports them, and the moving sum
    // MUST be computed from them. Filtering first keeps the top rows by `merge_impact`,
    // so a moving sum taken afterwards undercounts precisely the small events the
    // series exists to reveal.
    let all_merge_events = threshold_merge_event_rows(&summary);
    let merge_moving_sum = merge_event_moving_sum(
        &all_merge_events,
        MOVING_SUM_WINDOW_FRACTION,
        MOVING_SUM_GRID_POINTS,
    );
    let merge_event_total = all_merge_events.len();
    let merge_events = filter_merge_event_rows(&all_merge_events, max_merge_events);
    let slider_stops = threshold_slider_stops(&merge_events, Some(&tree));

    MatrixReport {
        n_nodes: tree.n_nodes,
        n_edges: n,
        mean,
        median,
        min,
        max,
        scores_desc: scores,
        cluster_count_by_threshold: tree.cluster_count_by_threshold(),
        cluster_count_by_edge_count: tree.cluster_count_by_edge_count(),
        edges_by_threshold: tree.edges_by_threshold(),
        merge_impact_metric: merge_impact_metric.to_string(),
        merge_events,
        merge_event_total,
        max_merge_events,
        merge_moving_sum,
        slider_stops,
        mst_edges: tree.export_mst_edges(),
        thresholds: tree.thresholds().to_vec(),
    }
}

fn num_or_null(x: f64) -> Value {
    if x.is_finite() {
        json!(x)
    } else {
        Value::Null
    }
}

/// A few indices to sample in the text tables: first three, middle, and last
/// three (deduplicated, in order), matching the Python text writer.
fn sample_indices(len: usize) -> Vec<usize> {
    let mut idx = vec![0usize, 1, 2, len / 2];
    for d in [3usize, 2, 1] {
        if len >= d {
            idx.push(len - d);
        }
    }
    let mut seen = std::collections::HashSet::new();
    idx.into_iter().filter(|&i| i < len && seen.insert(i)).collect()
}

/// A very small ASCII histogram (bashplotlib is not replicated exactly).
fn ascii_histogram(scores_desc: &[f64], bins: usize, width: usize) -> String {
    if scores_desc.is_empty() {
        return String::new();
    }
    let max = scores_desc[0];
    let min = scores_desc[scores_desc.len() - 1];
    let span = (max - min).max(f64::MIN_POSITIVE);
    let mut counts = vec![0usize; bins];
    for &s in scores_desc {
        let mut b = ((s - min) / span * bins as f64) as usize;
        if b >= bins {
            b = bins - 1;
        }
        counts[b] += 1;
    }
    let peak = counts.iter().copied().max().unwrap_or(1).max(1);
    let mut out = String::from("Edge counts by score:\n");
    for (i, &c) in counts.iter().enumerate() {
        let lo = min + span * i as f64 / bins as f64;
        let bar = c * width / peak;
        out.push_str(&format!("  {lo:8.2} | {} {}\n", "#".repeat(bar), c));
    }
    out
}

/// Render the human-readable text report.
pub fn render_text(r: &MatrixReport) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "Matrix Report\nNodes: {}\nNon-zero edges: {}\nMean: {:.1}\nMedian: {:.1}\nMin: {:.1}\nMax: {:.1}\n\n",
        r.n_nodes, r.n_edges, r.mean, r.median, r.min, r.max
    ));
    s.push_str(&ascii_histogram(&r.scores_desc, 20, 50));

    if r.cluster_count_by_threshold.len() > 1 {
        s.push_str("\nCluster count at key thresholds:\n");
        for i in sample_indices(r.cluster_count_by_threshold.len()) {
            let (t, c) = r.cluster_count_by_threshold[i];
            if t.is_infinite() {
                s.push_str(&format!("  Threshold: ∞ → Clusters: {}\n", c as i64));
            } else {
                s.push_str(&format!("  Threshold: {t:.2} → Clusters: {}\n", c as i64));
            }
        }
    }
    if r.cluster_count_by_edge_count.len() > 1 {
        s.push_str("\nCluster count at key edge counts:\n");
        for i in sample_indices(r.cluster_count_by_edge_count.len()) {
            let (e, c) = r.cluster_count_by_edge_count[i];
            s.push_str(&format!("  Edges: {} → Clusters: {}\n", e as i64, c as i64));
        }
    }
    if !r.edges_by_threshold.is_empty() {
        s.push_str("\nEdge counts at key thresholds:\n");
        for i in sample_indices(r.edges_by_threshold.len()) {
            let (e, t) = r.edges_by_threshold[i];
            s.push_str(&format!("  Threshold: {t:.2} → Cumulative edges: {}\n", e as i64));
        }
    }

    s.push_str(&format!(
        "\nStrongest MST split events (metric={}):\n",
        r.merge_impact_metric
    ));
    // Rank by impact for the "strongest" table (top 25), like Python.
    let mut ranked: Vec<&MergeEvent> = r.merge_events.iter().collect();
    ranked.sort_by(|a, b| {
        b.merge_impact
            .total_cmp(&a.merge_impact)
            .then(b.delta_largest.total_cmp(&a.delta_largest))
    });
    ranked.truncate(25);
    ranked.sort_by(|a, b| b.threshold_from_value.total_cmp(&a.threshold_from_value));
    if ranked.is_empty() {
        s.push_str("  No split events found.\n");
    } else {
        s.push_str(&format!("  {:>8}  {:>8}  {:>8}\n", "From", "To", "Impact"));
        for e in ranked {
            s.push_str(&format!(
                "  {:>8}  {:>8}  {:>8}\n",
                e.threshold_from, e.threshold_to, e.merge_impact.round() as i64
            ));
        }
    }
    s
}

fn merge_event_json(e: &MergeEvent) -> Value {
    json!({
        "edge_index": e.edge_index,
        "threshold_from": e.threshold_from,
        "threshold_to": e.threshold_to,
        "threshold_from_value": num_or_null(e.threshold_from_value),
        "threshold_value": num_or_null(e.threshold_value),
        "merge_impact": num_or_null(e.merge_impact),
        "delta_largest": num_or_null(e.delta_largest),
        "delta_avg_non_singleton": num_or_null(e.delta_avg_non_singleton),
    })
}

/// The agent-friendly JSON payload: summary stats, the threshold→cluster curve,
/// edge counts, ranked merge events, and a short list of suggested cutoffs.
pub fn to_json(r: &MatrixReport) -> Value {
    let ccbt: Vec<Value> = r
        .cluster_count_by_threshold
        .iter()
        .map(|&(t, c)| Value::Array(vec![num_or_null(t), json!(c as i64)]))
        .collect();
    let ccbe: Vec<Value> = r
        .cluster_count_by_edge_count
        .iter()
        .map(|&(e, c)| Value::Array(vec![json!(e as i64), json!(c as i64)]))
        .collect();
    let ebt: Vec<Value> = r
        .edges_by_threshold
        .iter()
        .map(|&(e, t)| Value::Array(vec![json!(e as i64), num_or_null(t)]))
        .collect();
    let events: Vec<Value> = r.merge_events.iter().map(merge_event_json).collect();

    // Suggested thresholds: the strongest split events, with the resulting
    // cluster count when cutting at that threshold.
    let mut suggested: Vec<&MergeEvent> = r.merge_events.iter().collect();
    suggested.sort_by(|a, b| {
        b.merge_impact
            .total_cmp(&a.merge_impact)
            .then(b.delta_largest.total_cmp(&a.delta_largest))
    });
    suggested.truncate(10);
    let suggested_json: Vec<Value> = suggested
        .iter()
        .map(|e| {
            let cluster_count = (r.n_nodes as i64 - (e.edge_index + 1)).max(0);
            json!({
                "threshold": num_or_null(e.threshold_value),
                "threshold_label": format_threshold_value(e.threshold_value),
                "cluster_count": cluster_count,
                "merge_impact": num_or_null(e.merge_impact),
            })
        })
        .collect();

    json!({
        "n_nodes": r.n_nodes,
        "n_edges": r.n_edges,
        "stats": {"mean": r.mean, "median": r.median, "min": r.min, "max": r.max},
        "cluster_count_by_threshold": ccbt,
        "cluster_count_by_edge_count": ccbe,
        "edges_by_threshold": ebt,
        "merge_impact_metric": r.merge_impact_metric,
        "merge_event_series": events,
        "suggested_thresholds": suggested_json,
        "mst_edges": r.mst_edges.iter().map(|&(i, j, w)| json!([i, j, w])).collect::<Vec<_>>(),
    })
}

/// Defaults re-exported for the CLI.
pub const DEFAULT_MERGE_IMPACT_METRIC: &str = MERGE_IMPACT_MIN_CHILD;
pub const DEFAULT_MAX_MERGE: usize = DEFAULT_MAX_MERGE_EVENTS;
