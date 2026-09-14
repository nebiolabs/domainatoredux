//! `ssn-navigator`: read-only navigation of a `build-ssn-viewer` `.dsnv` bundle.
//!
//! Mirrors `domainator.ssn_navigator`. An agent-friendly companion to the interactive
//! HTML viewer: instead of rendering the whole network it answers targeted questions
//! about it at a chosen similarity threshold and returns compact JSON — which clusters
//! exist, what a cluster contains, and how a cluster's metadata is distributed.
//!
//! Higher thresholds give finer clusters (more splitting), matching the viewer's
//! slider: `--threshold inf` is the "∞" stop at its fine end, where every node is its
//! own cluster, and **omitting `--threshold` uses the floor** at the coarse end, one
//! cluster per connected component.

use serde_json::{json, Map, Value};

use super::ssn_bundle::{
    cluster_for_node, clusters_at_threshold, coarsest_threshold, component_members,
    component_size, node_index_by_name, summarize_cluster_metadata,
};

/// The query modes, in the order the CLI lists them.
pub const MODES: [&str; 5] = ["overview", "thresholds", "clusters", "cluster", "node"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NavMode {
    Overview,
    Thresholds,
    Clusters,
    Cluster,
    Node,
}

impl NavMode {
    pub fn parse(s: &str) -> Result<NavMode, String> {
        match s {
            "overview" => Ok(NavMode::Overview),
            "thresholds" => Ok(NavMode::Thresholds),
            "clusters" => Ok(NavMode::Clusters),
            "cluster" => Ok(NavMode::Cluster),
            "node" => Ok(NavMode::Node),
            other => Err(format!("Unknown mode '{other}'. Choose one of {MODES:?}.")),
        }
    }
}

#[derive(Clone, Debug)]
pub struct SsnNavigatorOptions {
    pub mode: NavMode,
    /// `None` means the floor (one cluster per component), **not** ∞.
    pub threshold: Option<String>,
    pub cluster_id: Option<usize>,
    pub node: Option<String>,
    pub min_size: usize,
    pub top_n: usize,
    pub members: bool,
}

impl Default for SsnNavigatorOptions {
    fn default() -> Self {
        SsnNavigatorOptions {
            mode: NavMode::Overview,
            threshold: None,
            cluster_id: None,
            node: None,
            min_size: 1,
            top_n: 50,
            members: false,
        }
    }
}

/// Resolve `--threshold` to a numeric cut, following the viewer's slider.
///
/// `inf`/`infinity`/`∞` is the slider's ∞ stop — every merge cut, every node its own
/// cluster. Omitting it picks the other end, the floor below every merge, since one
/// cluster per connected component is the more useful thing to see unasked.
fn resolve_threshold(threshold: Option<&str>, hierarchy: &Value) -> Result<f64, String> {
    match threshold {
        None => Ok(coarsest_threshold(hierarchy)),
        Some(s) => {
            let t = s.trim();
            if matches!(t.to_lowercase().as_str(), "inf" | "infinity" | "∞") {
                return Ok(f64::INFINITY);
            }
            t.parse::<f64>().map_err(|_| format!("Could not parse --threshold '{s}' as a number."))
        }
    }
}

/// The threshold as it should appear in output JSON: `null` for the ∞ cut, which is
/// how the bundle's own `slider_stops` spell it and which keeps the output strict JSON
/// (`json.dumps(inf)` would emit the non-standard `Infinity`).
fn threshold_out(threshold: f64) -> Value {
    if threshold.is_infinite() {
        Value::Null
    } else {
        json!(threshold)
    }
}

/// `graph.merge_event_series` is a capped selection, and the slider stops derive from
/// it — so a caller treating either as the complete set of interesting thresholds is
/// wrong on any large network. Returns nothing extra for a pre-v5 bundle, which did
/// not record the uncapped total.
fn merge_event_coverage(graph: &Value) -> Vec<(String, Value)> {
    let mut out = vec![(
        "merge_events".to_string(),
        json!(graph.get("merge_event_series").and_then(Value::as_array).map_or(0, Vec::len)),
    )];
    if let Some(total) = graph.get("merge_event_total") {
        out.push(("merge_event_total".to_string(), total.clone()));
        out.push((
            "max_merge_events".to_string(),
            graph.get("max_merge_events").cloned().unwrap_or(Value::Null),
        ));
    }
    out
}

fn cluster_records(hierarchy: &Value, active: &[usize], min_size: usize) -> Vec<Value> {
    let mut records: Vec<(usize, usize)> = active
        .iter()
        .map(|&id| (id, component_size(hierarchy, id)))
        .filter(|&(_, size)| size >= min_size)
        .collect();
    records.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    records.into_iter().map(|(id, size)| json!({"id": id, "size": size})).collect()
}

fn metadata_row(bundle: &Value, node_index: usize) -> Value {
    let empty = Vec::new();
    let metadata = bundle.get("metadata").cloned().unwrap_or(Value::Null);
    let columns = metadata.get("columns").and_then(Value::as_array).unwrap_or(&empty);
    let rows = metadata.get("rows").and_then(Value::as_array).unwrap_or(&empty);
    let Some(row) = rows.get(node_index).and_then(Value::as_array) else {
        return Value::Object(Map::new());
    };
    let mut out = Map::new();
    for (idx, column) in columns.iter().enumerate() {
        let name = column.get("name").and_then(Value::as_str).unwrap_or("");
        out.insert(name.to_string(), row.get(idx).cloned().unwrap_or(Value::Null));
    }
    Value::Object(out)
}

/// Answer a single navigation query against a loaded `.dsnv` bundle.
pub fn ssn_navigator(bundle: &Value, opts: &SsnNavigatorOptions) -> Result<Value, String> {
    let graph = bundle.get("graph").ok_or("bundle has no 'graph' section")?;
    let hierarchy = graph.get("hierarchy").ok_or("bundle graph has no 'hierarchy'")?;
    let empty = Vec::new();
    let node_names = graph.get("nodes").and_then(Value::as_array).unwrap_or(&empty);
    let threshold = resolve_threshold(opts.threshold.as_deref(), hierarchy)?;

    match opts.mode {
        NavMode::Overview => {
            let mut out = Map::new();
            out.insert("name".into(), bundle.get("name").cloned().unwrap_or(Value::Null));
            out.insert(
                "domainator_version".into(),
                bundle.get("domainator_version").cloned().unwrap_or(Value::Null),
            );
            out.insert("nodes".into(), json!(node_names.len()));
            out.insert(
                "mst_edges".into(),
                json!(graph.get("mst_edges").and_then(Value::as_array).map_or(0, Vec::len)),
            );
            out.insert(
                "connected_components".into(),
                json!(hierarchy.get("roots").and_then(Value::as_array).map_or(0, Vec::len)),
            );
            out.insert(
                "metadata_columns".into(),
                bundle
                    .get("metadata")
                    .and_then(|m| m.get("columns"))
                    .cloned()
                    .unwrap_or_else(|| Value::Array(Vec::new())),
            );
            out.insert(
                "defaults".into(),
                bundle.get("defaults").cloned().unwrap_or_else(|| Value::Object(Map::new())),
            );
            out.insert(
                "merge_impact_metric".into(),
                graph.get("merge_impact_metric").cloned().unwrap_or(Value::Null),
            );
            for (k, v) in merge_event_coverage(graph) {
                out.insert(k, v);
            }
            Ok(Value::Object(out))
        }

        NavMode::Thresholds => {
            // The stops are one per plotted merge event, so they inherit the cap.
            // Saying so is the point: picking a cut-point from a silently truncated
            // list is the failure this guards against.
            let stops = graph.get("slider_stops").and_then(Value::as_array).unwrap_or(&empty);
            let summaries: Vec<Value> = stops
                .iter()
                .map(|stop| {
                    let tv = stop.get("threshold_value").and_then(Value::as_f64);
                    let active = clusters_at_threshold(hierarchy, tv);
                    let sizes: Vec<usize> =
                        active.iter().map(|&id| component_size(hierarchy, id)).collect();
                    json!({
                        "threshold_value": stop.get("threshold_value")
                            .cloned().unwrap_or(Value::Null),
                        "threshold_label": stop.get("threshold_label")
                            .cloned().unwrap_or(Value::Null),
                        "clusters": active.len(),
                        "non_singleton_clusters": sizes.iter().filter(|&&s| s > 1).count(),
                        "largest_cluster": sizes.iter().max().copied().unwrap_or(0),
                    })
                })
                .collect();
            let mut out = Map::new();
            out.insert("thresholds".into(), Value::Array(summaries));
            for (k, v) in merge_event_coverage(graph) {
                out.insert(k, v);
            }
            Ok(Value::Object(out))
        }

        NavMode::Clusters => {
            let active = clusters_at_threshold(hierarchy, Some(threshold));
            let records = cluster_records(hierarchy, &active, opts.min_size);
            let singletons =
                records.iter().filter(|r| r.get("size").and_then(Value::as_u64) == Some(1)).count();
            let mut out = Map::new();
            out.insert("threshold".into(), threshold_out(threshold));
            out.insert("clusters".into(), json!(records.len()));
            out.insert("singletons".into(), json!(singletons));
            out.insert("shown".into(), json!(records.len().min(opts.top_n)));
            out.insert(
                "cluster_list".into(),
                Value::Array(records.iter().take(opts.top_n).cloned().collect()),
            );
            if records.len() > opts.top_n {
                out.insert("truncated".into(), json!(records.len() - opts.top_n));
            }
            Ok(Value::Object(out))
        }

        NavMode::Cluster => {
            let active = clusters_at_threshold(hierarchy, Some(threshold));
            let cluster_id = if let Some(name) = &opts.node {
                let by_name = node_index_by_name(bundle);
                let idx = *by_name
                    .get(name.as_str())
                    .ok_or_else(|| format!("Node '{name}' not found in the bundle."))?;
                cluster_for_node(hierarchy, &active, idx)
                    .ok_or_else(|| format!("No active cluster found for node '{name}'."))?
            } else if let Some(id) = opts.cluster_id {
                if !active.contains(&id) {
                    return Err(format!(
                        "Cluster id {id} is not an active cluster at threshold {threshold}; \
                         use --mode clusters to list valid ids."
                    ));
                }
                id
            } else {
                return Err("cluster mode requires either --id or --node.".into());
            };

            let member_indices = component_members(hierarchy, cluster_id);
            let metadata = bundle.get("metadata").cloned().unwrap_or(Value::Null);
            let mut out = Map::new();
            out.insert("threshold".into(), threshold_out(threshold));
            out.insert("cluster_id".into(), json!(cluster_id));
            out.insert("size".into(), json!(member_indices.len()));
            out.insert(
                "metadata_summary".into(),
                Value::Array(summarize_cluster_metadata(&member_indices, &metadata, opts.top_n)),
            );
            if opts.members {
                out.insert(
                    "members".into(),
                    Value::Array(
                        member_indices
                            .iter()
                            .map(|&i| node_names.get(i).cloned().unwrap_or(Value::Null))
                            .collect(),
                    ),
                );
            }
            Ok(Value::Object(out))
        }

        NavMode::Node => {
            let name = opts.node.as_ref().ok_or("node mode requires --node.")?;
            let by_name = node_index_by_name(bundle);
            let node_index = *by_name
                .get(name.as_str())
                .ok_or_else(|| format!("Node '{name}' not found in the bundle."))?;
            let stops = graph.get("slider_stops").and_then(Value::as_array).unwrap_or(&empty);
            let memberships: Vec<Value> = stops
                .iter()
                .map(|stop| {
                    let tv = stop.get("threshold_value").and_then(Value::as_f64);
                    let active = clusters_at_threshold(hierarchy, tv);
                    let cid = cluster_for_node(hierarchy, &active, node_index);
                    json!({
                        "threshold_value": stop.get("threshold_value")
                            .cloned().unwrap_or(Value::Null),
                        "threshold_label": stop.get("threshold_label")
                            .cloned().unwrap_or(Value::Null),
                        "cluster_id": cid,
                        "cluster_size": cid.map(|c| component_size(hierarchy, c)),
                    })
                })
                .collect();
            Ok(json!({
                "node": name,
                "node_index": node_index,
                "metadata": metadata_row(bundle, node_index),
                "memberships": memberships,
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threshold_defaults_to_the_floor_not_infinity() {
        // Omitting --threshold must give one cluster per connected component, which is
        // the opposite end of the slider from ∞.
        let h = json!({
            "nodes": [
                {"kind": "leaf", "size": 1, "leaf_start": 0, "leaf_count": 1},
                {"kind": "leaf", "size": 1, "leaf_start": 1, "leaf_count": 1},
                {"kind": "cluster", "left": 0, "right": 1, "threshold": 5.0,
                 "size": 2, "leaf_start": 0, "leaf_count": 2},
            ],
            "roots": [2], "leaf_order": [0, 1],
        });
        let floor = resolve_threshold(None, &h).unwrap();
        assert!(floor < 5.0, "floor {floor} must sit below the only merge");
        assert_eq!(clusters_at_threshold(&h, Some(floor)), vec![2]);

        for spelling in ["inf", "Infinity", "∞"] {
            assert!(resolve_threshold(Some(spelling), &h).unwrap().is_infinite());
        }
        assert_eq!(resolve_threshold(Some("7.5"), &h).unwrap(), 7.5);
        assert!(resolve_threshold(Some("banana"), &h).is_err());
    }

    #[test]
    fn infinite_threshold_serializes_as_null() {
        assert_eq!(threshold_out(f64::INFINITY), Value::Null);
        assert_eq!(threshold_out(3.5), json!(3.5));
    }
}
