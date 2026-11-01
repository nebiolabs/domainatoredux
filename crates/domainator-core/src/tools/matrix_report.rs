//! `matrix_report`: summarize a similarity matrix to help choose an SSN edge
//! threshold. Mirrors `domainator.matrix_report`: the text report, the agent-facing
//! `--json_out` document, the `--html_out` page's data ([`to_html_data`]) and the
//! `--knn_projection` edge counts. [`to_viewer_json`] is this port's own, for the GUI.

use crate::progress::Stages;
use serde_json::{json, Value};

use crate::matrix::ssn::{
    component_size_summary_by_threshold, edge_scores_desc, filter_merge_event_rows,
    format_threshold_value, merge_event_moving_sum, mst_knn_edge_counts_by_threshold,
    threshold_merge_event_rows, threshold_slider_stops, MaxTree, MergeEvent, MovingSum, SliderStop,
    DEFAULT_MAX_MERGE_EVENTS, MERGE_IMPACT_MIN_CHILD, MOVING_SUM_GRID_POINTS,
    MOVING_SUM_WINDOW_FRACTION,
};
use crate::matrix::DataMatrix;
use crate::text_plot::text_histogram;

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
    /// The text report's "strongest split events" table: the top
    /// [`STRONGEST_TABLE_ROWS`] of **all** split events (not of the capped selection),
    /// in descending threshold order (upstream `merge_event_table_rows`).
    pub strongest_merge_events: Vec<MergeEvent>,
    /// Centered moving sum of `merge_impact`, derived from the **uncapped** rows.
    pub merge_moving_sum: MovingSum,
    /// Threshold cuts the report's slider offers, ending at a reachable floor.
    pub slider_stops: Vec<SliderStop>,
    pub mst_edges: Vec<(usize, usize, f64)>,
    /// Distinct thresholds, descending — lets a consumer map a threshold back to its
    /// row in the tables above without holding the whole `MaxTree`.
    pub thresholds: Vec<f64>,
    /// `--knn_projection`'s MST + kNN edge counts, when asked for.
    pub knn_projection: Option<KnnProjection>,
}

/// What `build_ssn --threshold T --mst --knn k` would keep, for every threshold row `T`
/// and `k` from `min_k` to `max_k` (upstream `_get_mst_knn_report_config` and
/// `mst_knn_edge_counts_by_threshold`).
pub struct KnnProjection {
    pub min_k: usize,
    pub max_k: usize,
    /// The `k` the text report and the page start from.
    pub default_k: usize,
    /// `counts[row][k - min_k]`, one row per entry of [`MatrixReport::thresholds`].
    pub counts: Vec<Vec<u64>>,
}

/// The smallest `k` `--knn_projection` counts.
pub const KNN_PROJECTION_MIN_K: usize = 1;
/// The largest, unless the network has fewer other nodes.
pub const KNN_PROJECTION_MAX_K: usize = 25;

fn knn_projection(matrix: &DataMatrix, tree: &MaxTree) -> KnnProjection {
    let max_k = KNN_PROJECTION_MAX_K.min(tree.n_nodes.saturating_sub(1).max(KNN_PROJECTION_MIN_K));
    KnnProjection {
        min_k: KNN_PROJECTION_MIN_K,
        max_k,
        default_k: max_k.min(5),
        counts: mst_knn_edge_counts_by_threshold(matrix, tree, max_k),
    }
}

/// Rows in the text report's strongest-split-events table.
pub const STRONGEST_TABLE_ROWS: usize = 25;

/// Upstream `merge_event_table_rows`: rank every split event by impact (then the two
/// tie-breaks `summarize_merge_events` uses), keep the strongest
/// [`STRONGEST_TABLE_ROWS`], and list them from the highest threshold down. Both sorts
/// are stable, as Python's are.
fn strongest_merge_events(events: &[MergeEvent]) -> Vec<MergeEvent> {
    let mut ranked: Vec<&MergeEvent> = events.iter().collect();
    ranked.sort_by(|a, b| {
        b.merge_impact
            .total_cmp(&a.merge_impact)
            .then(b.delta_largest.total_cmp(&a.delta_largest))
            .then(
                b.delta_avg_non_singleton
                    .total_cmp(&a.delta_avg_non_singleton),
            )
    });
    ranked.truncate(STRONGEST_TABLE_ROWS);
    ranked.sort_by(|a, b| {
        b.threshold_from_value
            .total_cmp(&a.threshold_from_value)
            .then(b.threshold_value.total_cmp(&a.threshold_value))
    });
    ranked.into_iter().cloned().collect()
}

/// Build the report. `merge_impact_metric` is `min_child`/`product`;
/// `max_merge_events` caps the retained split events (0 = all); `with_knn_projection`
/// also counts the MST + kNN edges of every threshold and `k` (`--knn_projection`).
/// `stages` hears about each stage, upstream's `progress_callback` and
/// `stage_timings`.
pub fn matrix_report(
    matrix: &DataMatrix,
    merge_impact_metric: &str,
    max_merge_events: usize,
    with_knn_projection: bool,
    stages: &mut Stages,
) -> MatrixReport {
    let since = stages.begin("building edge table");
    let scores = edge_scores_desc(matrix); // descending
    let n = scores.len();
    let mean = if n > 0 {
        scores.iter().sum::<f64>() / n as f64
    } else {
        0.0
    };
    let median = if n == 0 {
        0.0
    } else if n % 2 == 1 {
        scores[n / 2]
    } else {
        (scores[n / 2 - 1] + scores[n / 2]) / 2.0
    };
    let max = scores.first().copied().unwrap_or(0.0);
    let min = scores.last().copied().unwrap_or(0.0);

    stages.record("build_edge_table", since, || {
        vec![
            ("n_edges", n.to_string()),
            ("n_nodes", matrix.nrows().to_string()),
        ]
    });

    let since = stages.begin("building maximum spanning tree");
    let tree = MaxTree::from_matrix(matrix);
    stages.record("build_max_tree", since, || {
        vec![("n_mst_edges", tree.export_mst_edges().len().to_string())]
    });
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
    let strongest_merge_events = strongest_merge_events(&all_merge_events);
    // `true`: this report, unlike a `.dsnv` bundle, carries the threshold tables and
    // reads each cut's edge count out of them.
    let slider_stops = threshold_slider_stops(&merge_events, Some(&tree), true);
    let knn_projection = with_knn_projection.then(|| {
        let since = stages.begin("building kNN projection threshold counts");
        let projection = knn_projection(matrix, &tree);
        stages.record("build_mst_knn_counts", since, || {
            vec![
                ("thresholds", tree.thresholds().len().to_string()),
                ("max_k", projection.max_k.to_string()),
            ]
        });
        projection
    });

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
        strongest_merge_events,
        merge_moving_sum,
        slider_stops,
        mst_edges: tree.export_mst_edges(),
        thresholds: tree.thresholds().to_vec(),
        knn_projection,
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
    // Python builds `[0, 1, 2, len//2, -3, -2, -1]` and then filters with
    // `if 0 <= i < len(...)`, which silently discards all three negative indices --
    // so it only ever prints rows 0, 1, 2 and the midpoint. Matching that exactly is
    // what makes these tables line-for-line identical rather than merely similar.
    let idx = [0usize, 1, 2, len / 2];
    let mut seen = std::collections::HashSet::new();
    idx.into_iter()
        .filter(|&i| i < len && seen.insert(i))
        .collect()
}

/// Render the human-readable text report.
pub fn render_text(r: &MatrixReport) -> String {
    let mut s = String::new();
    // Upstream prints the header from a triple-quoted string whose last line is eight
    // spaces, then the histogram between two newlines.
    s.push_str(&format!(
        "Matrix Report\nNodes: {}\nNon-zero edges: {}\nMean: {:.1}\nMedian: {:.1}\nMin: {:.1}\nMax: {:.1}\n        \n",
        r.n_nodes, r.n_edges, r.mean, r.median, r.min, r.max
    ));
    s.push('\n');
    if !r.scores_desc.is_empty() {
        s.push_str(&text_histogram(
            &r.scores_desc,
            "Edge counts by score",
            50,
            20,
        ));
    }
    s.push('\n');

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
        s.push_str(
            "\nEdges kept at key thresholds (equivalent to build_ssn --threshold <threshold>):\n",
        );
        for i in sample_indices(r.edges_by_threshold.len()) {
            let (e, t) = r.edges_by_threshold[i];
            s.push_str(&format!("  Threshold: {t:.2} → Edges: {}\n", e as i64));
        }
    }
    if let Some(knn) = r
        .knn_projection
        .as_ref()
        .filter(|knn| !knn.counts.is_empty())
    {
        let k = knn.default_k;
        s.push_str(&format!("\nProjected MST + kNN edge counts (k={k}):\n"));
        for i in sample_indices(knn.counts.len()) {
            s.push_str(&format!(
                "  Threshold: {:.2} → MST + kNN edges: {}\n",
                r.thresholds[i],
                knn.counts[i][k - knn.min_k]
            ));
        }
    }

    // Upstream has split events to list only when the tree has an edge.
    if r.mst_edges.is_empty() {
        return s;
    }
    s.push_str(&format!(
        "\nStrongest MST split events (metric={}):\n",
        r.merge_impact_metric
    ));
    if r.strongest_merge_events.is_empty() {
        s.push_str("  No split events found.\n");
    } else {
        s.push_str(&format!("  {:>8}  {:>8}  {:>8}\n", "From", "To", "Impact"));
        for e in &r.strongest_merge_events {
            s.push_str(&format!(
                "  {:>8}  {:>8}  {:>8}\n",
                e.threshold_from,
                e.threshold_to,
                e.merge_impact.round() as i64
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

/// The **GUI's** payload: summary stats, the threshold→cluster curve, edge counts,
/// ranked merge events, and a short list of suggested cutoffs.
///
/// This is deliberately **not** parity-locked — it is shaped for the Matrix Report
/// viewer tab (`apps/domainatoredux-gui`), which draws the per-threshold curves that
/// upstream's `--json` does not carry. The agent-facing contract is [`to_json`],
/// which matches Python's `SummaryJSONWriter` exactly; keep the two separate rather
/// than trying to serve both from one shape.
pub fn to_viewer_json(r: &MatrixReport) -> Value {
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
            // `edge_index` is no longer a dense MST index (it is the last edge
            // strictly above the threshold, and ties collapse), so derive the count
            // from the threshold tables instead of arithmetic on it.
            let cluster_count = cluster_count_at(r, e.threshold_value);
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

/// Clusters remaining when cutting at `threshold`, read off the threshold tables.
///
/// Returns `-1` when the threshold has no row, which only happens for a value that
/// did not come from this tree.
fn cluster_count_at(r: &MatrixReport, threshold: f64) -> i64 {
    match r.thresholds.iter().position(|&t| t == threshold) {
        Some(idx) => r.cluster_count_by_threshold[idx].1 as i64,
        None => -1,
    }
}

/// Every field of one split event, matching Python's merge-event row.
fn split_event_json(e: &MergeEvent) -> Value {
    json!({
        "edge_index": e.edge_index,
        "summary_row_from": e.summary_row_from,
        "summary_row_to": e.summary_row_to,
        "threshold_from_value": num_or_null(e.threshold_from_value),
        "threshold_from": e.threshold_from,
        "threshold_to": e.threshold_to,
        "threshold_value": num_or_null(e.threshold_value),
        "merge_impact": num_or_null(e.merge_impact),
        "merge_size_counts": merge_size_counts_json(&e.merge_size_counts),
        "largest_merge": num_or_null(e.largest_merge),
        "merge_count": e.merge_count,
        "delta_largest": num_or_null(e.delta_largest),
        "delta_avg_non_singleton": num_or_null(e.delta_avg_non_singleton),
    })
}

/// `{size: count}` with integral sizes keyed as ints (`{"1": 33}`, not `{"1.0": 33}`).
fn merge_size_counts_json(counts: &[(f64, u64)]) -> Value {
    let mut map = serde_json::Map::new();
    for &(size, count) in counts {
        let key = if size.fract() == 0.0 && size.abs() < 1e16 {
            format!("{}", size as i64)
        } else {
            format!("{size}")
        };
        map.insert(key, json!(count));
    }
    Value::Object(map)
}

/// The version of the `--json_out` document (upstream `JSON_SCHEMA_VERSION`).
pub const JSON_SCHEMA_VERSION: i64 = 1;

/// The agent-facing `--json_out` payload, matching Python's `SummaryJSONWriter` field for
/// field. Edge-score stats, the connected-component count, and the bounded
/// strongest-split-event series; deliberately omits the bulky per-node arrays the HTML
/// report embeds for the browser.
///
/// Keys are inserted in Python's own insertion order (serde_json is built with
/// `preserve_order`) so a raw `diff` against Python's output is readable too, though
/// `scripts/json_parity.py` compares parsed JSON and ignores key order.
pub fn to_json(r: &MatrixReport) -> Value {
    let mut payload = serde_json::Map::new();
    payload.insert("schema_version".into(), json!(JSON_SCHEMA_VERSION));
    payload.insert("nodes".into(), json!(r.n_nodes));
    payload.insert("non_zero_edges".into(), json!(r.n_edges));
    // `null`, not an object of zeros, when the matrix has no non-zero edge.
    payload.insert(
        "edge_scores".into(),
        if r.n_edges > 0 {
            json!({
                "mean": r.mean,
                "median": r.median,
                "min": r.min,
                "max": r.max,
            })
        } else {
            Value::Null
        },
    );
    payload.insert(
        "connected_components".into(),
        json!(r.n_nodes - r.mst_edges.len()),
    );
    payload.insert("merge_impact_metric".into(), json!(r.merge_impact_metric));
    payload.insert(
        "split_events".into(),
        Value::Array(r.merge_events.iter().map(split_event_json).collect()),
    );
    // split_events is a capped selection; this is what it was selected from.
    payload.insert("split_event_total".into(), json!(r.merge_event_total));
    payload.insert("max_merge_events".into(), json!(r.max_merge_events));
    Value::Object(payload)
}

/// The `--html_out` page's data (upstream `SummaryHTMLWriter`, read by
/// `web/reports/matrix_report.js`): the [`to_json`] document, and what the page needs to
/// show the network at any threshold — the MST, the threshold tables as columns, the
/// moving sum of split impact, the slider's stops and the `--knn_projection` counts.
pub fn to_html_data(r: &MatrixReport) -> Value {
    let stops: Vec<Value> = r
        .slider_stops
        .iter()
        .map(|stop| {
            json!({
                "edge_index": stop.edge_index,
                "threshold_label": stop.threshold_label,
                "threshold_value": stop.threshold_value.map_or(Value::Null, num_or_null),
                "threshold_index": stop.threshold_index.unwrap_or(-1),
            })
        })
        .collect();
    json!({
        "summary": to_json(r),
        "mst_edges": r.mst_edges.iter().map(|&(i, j, w)| json!([i, j, w])).collect::<Vec<_>>(),
        "threshold_table": {
            "threshold": r.thresholds,
            "edges": r.edges_by_threshold.iter().map(|&(e, _)| e as i64).collect::<Vec<_>>(),
            "clusters": r.cluster_count_by_threshold.iter().map(|&(_, c)| c as i64).collect::<Vec<_>>(),
        },
        "merge_moving_sum": {
            "window": r.merge_moving_sum.window,
            "x": r.merge_moving_sum.x,
            "y": r.merge_moving_sum.y,
        },
        "slider_stops": stops,
        "knn_projection": r.knn_projection.as_ref().map_or(Value::Null, |knn| json!({
            "min_k": knn.min_k,
            "max_k": knn.max_k,
            "default_k": knn.default_k,
            "counts": knn.counts,
        })),
    })
}

/// Defaults re-exported for the CLI.
pub const DEFAULT_MERGE_IMPACT_METRIC: &str = MERGE_IMPACT_MIN_CHILD;
pub const DEFAULT_MAX_MERGE: usize = DEFAULT_MAX_MERGE_EVENTS;
