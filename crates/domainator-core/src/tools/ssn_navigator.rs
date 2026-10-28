//! `ssn_navigator`: read-only navigation of the network in a Domainator Viewer bundle
//! (`build_ssn --dvb_out`), or an older `.dsnv` network bundle.
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
//!
//! The cut-points offered by `thresholds` and `node` are derived from the bundle's
//! merge order, not read out of it, so `max_merge_events` controls how many are offered
//! (`0` for one per split event) whatever the bundle was built with.

use serde_json::{json, Map, Value};

use crate::matrix::ssn::{MergeEvent, SliderStop, DEFAULT_MAX_MERGE_EVENTS};

use super::ssn_bundle::{
    cluster_for_node, clusters_at_threshold, coarsest_threshold, component_members, component_size,
    merge_event_rows, merge_event_series, network, node_index_by_name, node_names, slider_stops,
    summarize_cluster_metadata,
};

/// The query modes, in the order the CLI lists them.
pub const MODES: [&str; 5] = ["overview", "thresholds", "clusters", "cluster", "node"];

/// The version of the `--json_out` document (upstream `JSON_SCHEMA_VERSION`).
pub const JSON_SCHEMA_VERSION: i64 = 1;

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
            other => Err(format!("Unknown query '{other}'. Choose one of {MODES:?}.")),
        }
    }
}

#[derive(Clone, Debug)]
pub struct SsnNavigatorOptions {
    pub query: NavMode,
    /// `None` means the floor (one cluster per component), **not** ∞.
    pub threshold: Option<String>,
    /// Signed as upstream's `--cluster` is: a negative id is never active.
    pub cluster_id: Option<i64>,
    pub node: Option<String>,
    pub min_size: usize,
    pub top_n: usize,
    pub members: bool,
    /// How many split events the `thresholds` / `node` cut-points are drawn from:
    /// the strongest this many by impact, plus a back-fill so no 5% band of the
    /// threshold range with an event to offer is left without one. `0` offers a
    /// cut-point at every split event.
    pub max_merge_events: usize,
}

impl Default for SsnNavigatorOptions {
    fn default() -> Self {
        SsnNavigatorOptions {
            query: NavMode::Overview,
            threshold: None,
            cluster_id: None,
            node: None,
            min_size: 1,
            top_n: 50,
            members: false,
            max_merge_events: DEFAULT_MAX_MERGE_EVENTS,
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
            // NaN parses, but cuts nothing meaningful; infinity is handled above.
            t.parse::<f64>()
                .ok()
                .filter(|v| !v.is_nan())
                .ok_or_else(|| format!("Could not parse --threshold '{s}' as a number."))
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

/// How much of the network's split-event series this run is reporting.
///
/// The cut-points offered are one per *selected* split event, and `max_merge_events`
/// keeps only the strongest (plus a back-fill so no stretch of the threshold range goes
/// unrepresented). A caller that treats the offered cut-points as the complete set of
/// interesting thresholds is wrong on any large network, so say what was left out.
///
/// The series is derived from the merge order rather than read from the bundle, so the
/// total is exact on **every** bundle version — including pre-v6 files, whose stored
/// series was already capped when it was written.
fn merge_event_coverage(
    event_total: usize,
    selected: usize,
    max_merge_events: usize,
) -> Vec<(String, Value)> {
    vec![
        ("merge_events".to_string(), json!(selected)),
        ("merge_event_total".to_string(), json!(event_total)),
        ("max_merge_events".to_string(), json!(max_merge_events)),
    ]
}

/// The derived split-event data a query needs, replayed at most once.
///
/// Replaying the merge order costs a pass over the MST edges, so only the modes that
/// report cut-points pay for it, and a mode needing both the rows and the stops replays
/// once rather than twice.
#[derive(Default)]
struct Derived {
    rows: Option<Vec<MergeEvent>>,
    selected: Option<Vec<MergeEvent>>,
    stops: Option<Vec<SliderStop>>,
}

impl Derived {
    fn rows(&mut self, bundle: &Value) -> &[MergeEvent] {
        self.rows
            .get_or_insert_with(|| merge_event_rows(bundle, None))
    }

    /// Run `f` over the cached rows without cloning them: a `MergeEvent` owns two
    /// Strings and a Vec, and the networks the cap exists for have ~10^5 of them, so
    /// the obvious `self.rows(bundle).to_vec()` would copy all of that twice per query
    /// just to satisfy the borrow checker.
    fn with_rows<T>(&mut self, bundle: &Value, f: impl FnOnce(&[MergeEvent]) -> T) -> T {
        let rows = self
            .rows
            .take()
            .unwrap_or_else(|| merge_event_rows(bundle, None));
        let out = f(&rows);
        self.rows = Some(rows);
        out
    }

    fn selected(&mut self, bundle: &Value, max_merge_events: usize) -> &[MergeEvent] {
        if self.selected.is_none() {
            self.selected = Some(self.with_rows(bundle, |rows| {
                merge_event_series(bundle, max_merge_events, Some(rows))
            }));
        }
        self.selected.as_deref().unwrap()
    }

    fn stops(&mut self, bundle: &Value, max_merge_events: usize) -> &[SliderStop] {
        if self.stops.is_none() {
            self.stops = Some(self.with_rows(bundle, |rows| {
                slider_stops(bundle, max_merge_events, Some(rows))
            }));
        }
        self.stops.as_deref().unwrap()
    }
}

/// A derived stop's `threshold_value`, as the output JSON carries it: `null` for ∞.
fn stop_value_json(stop: &SliderStop) -> Value {
    match stop.threshold_value {
        Some(v) => json!(v),
        None => Value::Null,
    }
}

fn cluster_records(hierarchy: &Value, active: &[usize], min_size: usize) -> Vec<Value> {
    let mut records: Vec<(usize, usize)> = active
        .iter()
        .map(|&id| (id, component_size(hierarchy, id)))
        .filter(|&(_, size)| size >= min_size)
        .collect();
    records.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    records
        .into_iter()
        .map(|(id, size)| json!({"id": id, "size": size}))
        .collect()
}

fn metadata_row(bundle: &Value, node_index: usize) -> Value {
    let empty = Vec::new();
    let metadata = bundle.get("metadata").cloned().unwrap_or(Value::Null);
    let columns = metadata
        .get("columns")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    let rows = metadata
        .get("rows")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    let Some(row) = rows.get(node_index).and_then(Value::as_array) else {
        return Value::Object(Map::new());
    };
    let mut out = Map::new();
    for (idx, column) in columns.iter().enumerate() {
        let name = column.get("name").and_then(Value::as_str).unwrap_or("");
        out.insert(
            name.to_string(),
            row.get(idx).cloned().unwrap_or(Value::Null),
        );
    }
    Value::Object(out)
}

/// Answer a single navigation query against a bundle with a network view, as
/// [`super::ssn_bundle::load_bundle`] returns it.
pub fn ssn_navigator(bundle: &Value, opts: &SsnNavigatorOptions) -> Result<Value, String> {
    let graph = network(bundle).ok_or("bundle has no network view")?;
    let hierarchy = graph
        .get("hierarchy")
        .ok_or("bundle network has no 'hierarchy'")?;
    let node_names = node_names(bundle);
    let threshold = resolve_threshold(opts.threshold.as_deref(), hierarchy)?;
    let mut derived = Derived::default();

    match opts.query {
        NavMode::Overview => {
            let mut out = Map::new();
            out.insert(
                "name".into(),
                bundle.get("name").cloned().unwrap_or(Value::Null),
            );
            out.insert(
                "domainator_version".into(),
                bundle
                    .get("domainator_version")
                    .cloned()
                    .unwrap_or(Value::Null),
            );
            out.insert("nodes".into(), json!(node_names.len()));
            out.insert(
                "mst_edges".into(),
                json!(graph
                    .get("mst_edges")
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len)),
            );
            out.insert(
                "connected_components".into(),
                json!(hierarchy
                    .get("roots")
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len)),
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
                bundle
                    .get("defaults")
                    .cloned()
                    .unwrap_or_else(|| Value::Object(Map::new())),
            );
            out.insert(
                "merge_impact_metric".into(),
                graph
                    .get("merge_impact_metric")
                    .cloned()
                    .unwrap_or(Value::Null),
            );
            let total = derived.rows(bundle).len();
            let shown = derived.selected(bundle, opts.max_merge_events).len();
            for (k, v) in merge_event_coverage(total, shown, opts.max_merge_events) {
                out.insert(k, v);
            }
            Ok(Value::Object(out))
        }

        NavMode::Thresholds => {
            // The cut-points are one per selected merge event, so they inherit the
            // cap. Saying so is the point: picking a cut-point from a silently
            // truncated list is the failure this guards against. Raise
            // `max_merge_events` (0 for all of them) to see more.
            let stops = derived.stops(bundle, opts.max_merge_events).to_vec();
            let summaries: Vec<Value> = stops
                .iter()
                .map(|stop| {
                    let active = clusters_at_threshold(hierarchy, stop.threshold_value);
                    let sizes: Vec<usize> = active
                        .iter()
                        .map(|&id| component_size(hierarchy, id))
                        .collect();
                    json!({
                        "threshold_value": stop_value_json(stop),
                        "threshold_label": stop.threshold_label,
                        "clusters": active.len(),
                        "non_singleton_clusters": sizes.iter().filter(|&&s| s > 1).count(),
                        "largest_cluster": sizes.iter().max().copied().unwrap_or(0),
                    })
                })
                .collect();
            let mut out = Map::new();
            out.insert("thresholds".into(), Value::Array(summaries));
            let total = derived.rows(bundle).len();
            let shown = derived.selected(bundle, opts.max_merge_events).len();
            for (k, v) in merge_event_coverage(total, shown, opts.max_merge_events) {
                out.insert(k, v);
            }
            Ok(Value::Object(out))
        }

        NavMode::Clusters => {
            let active = clusters_at_threshold(hierarchy, Some(threshold));
            let records = cluster_records(hierarchy, &active, opts.min_size);
            let singletons = records
                .iter()
                .filter(|r| r.get("size").and_then(Value::as_u64) == Some(1))
                .count();
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
                cluster_for_node(hierarchy, idx, threshold)
                    .ok_or_else(|| format!("No active cluster found for node '{name}'."))?
            } else if let Some(id) = opts.cluster_id {
                match usize::try_from(id).ok().filter(|id| active.contains(id)) {
                    Some(id) => id,
                    None => {
                        return Err(format!(
                            "Cluster id {id} is not an active cluster at threshold \
                             {threshold:?}; use --query clusters to list valid ids."
                        ))
                    }
                }
            } else {
                return Err("--query cluster requires either --cluster or --node.".into());
            };

            let member_indices = component_members(hierarchy, cluster_id);
            let metadata = bundle.get("metadata").cloned().unwrap_or(Value::Null);
            let mut out = Map::new();
            out.insert("threshold".into(), threshold_out(threshold));
            out.insert("cluster_id".into(), json!(cluster_id));
            out.insert("size".into(), json!(member_indices.len()));
            out.insert(
                "metadata_summary".into(),
                Value::Array(summarize_cluster_metadata(
                    &member_indices,
                    &metadata,
                    opts.top_n,
                )),
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
            let name = opts.node.as_ref().ok_or("--query node requires --node.")?;
            let by_name = node_index_by_name(bundle);
            let node_index = *by_name
                .get(name.as_str())
                .ok_or_else(|| format!("Node '{name}' not found in the bundle."))?;
            let stops = derived.stops(bundle, opts.max_merge_events).to_vec();
            let memberships: Vec<Value> = stops
                .iter()
                .map(|stop| {
                    // One walk up from this node's leaf per cut-point, rather than
                    // rebuilding the whole partition at each one only to look up a
                    // single member.
                    let cid = cluster_for_node(
                        hierarchy,
                        node_index,
                        stop.threshold_value.unwrap_or(f64::INFINITY),
                    );
                    json!({
                        "threshold_value": stop_value_json(stop),
                        "threshold_label": stop.threshold_label,
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

    /// A real bundle, so the cut-points under test are the ones a `.dvb` would give.
    fn six_node_bundle() -> Value {
        let mut dense = vec![0.0f64; 36];
        for &(i, j, w) in &[
            (0usize, 1usize, 10.0f64),
            (2, 3, 9.0),
            (1, 2, 5.0),
            (4, 5, 8.0),
            (3, 4, 4.0),
            (0, 5, 2.0),
        ] {
            dense[i * 6 + j] = w;
            dense[j * 6 + i] = w;
        }
        let labels: Vec<String> = (0..6).map(|i| format!("n{i}")).collect();
        let m = crate::matrix::DataMatrix {
            rows: labels.clone(),
            cols: labels,
            row_lengths: None,
            col_lengths: None,
            data_type: "score".into(),
            storage: crate::matrix::Storage::Dense(dense),
        };
        let view = super::super::ssn_bundle::ssn_view(
            &crate::matrix::ssn::MaxTree::from_matrix(&m),
            0.0,
            crate::matrix::ssn::MERGE_IMPACT_MIN_CHILD,
        )
        .unwrap();
        json!({
            "format": super::super::viewer_bundle::VIEWER_BUNDLE_FORMAT, "version": 1,
            "name": "t", "nodes": m.rows, "metadata": {"columns": [], "rows": []},
            "defaults": {}, "views": {"ssn": view},
        })
    }

    fn run(bundle: &Value, opts: SsnNavigatorOptions) -> Value {
        ssn_navigator(bundle, &opts).unwrap()
    }

    #[test]
    fn node_membership_matches_the_full_partition_at_every_cut() {
        // `--query node` walks up from the leaf instead of rebuilding the partition at
        // each cut-point. The two must agree everywhere, or the cheap path is a lie.
        let bundle = six_node_bundle();
        let hierarchy = &bundle["views"]["ssn"]["hierarchy"];
        let out = run(
            &bundle,
            SsnNavigatorOptions {
                query: NavMode::Node,
                node: Some("n0".into()),
                max_merge_events: 0,
                ..Default::default()
            },
        );
        let memberships = out["memberships"].as_array().unwrap();
        assert!(memberships.len() > 2, "∞ + one per split event + floor");
        for m in memberships {
            let cut = m["threshold_value"].as_f64();
            let active = clusters_at_threshold(hierarchy, cut);
            let expected = active
                .iter()
                .copied()
                .find(|&id| component_members(hierarchy, id).contains(&0usize))
                .unwrap();
            assert_eq!(m["cluster_id"], json!(expected), "at cut {cut:?}");
            assert_eq!(
                m["cluster_size"],
                json!(component_size(hierarchy, expected))
            );
        }
        // The ∞ stop leads, where every node stands alone.
        assert_eq!(memberships[0]["threshold_value"], Value::Null);
        assert_eq!(memberships[0]["cluster_size"], json!(1));
    }

    #[test]
    fn the_cap_is_applied_at_query_time_not_baked_into_the_file() {
        // The whole point of bundle v6: the same file answers at any cap, because the
        // series is derived from its merge order rather than read out of it.
        let bundle = six_node_bundle();
        let count = |max: usize| -> usize {
            run(
                &bundle,
                SsnNavigatorOptions {
                    query: NavMode::Thresholds,
                    max_merge_events: max,
                    ..Default::default()
                },
            )["thresholds"]
                .as_array()
                .unwrap()
                .len()
        };
        let uncapped = count(0);
        assert!(uncapped > 3, "need a series worth capping");
        assert!(count(1) <= uncapped, "a cap can only remove cut-points");
        assert_eq!(count(0), uncapped, "and 0 means no cap at all");
    }

    #[test]
    fn coverage_is_reported_on_every_bundle_version() {
        // Derived, so the total is exact even on a pre-v6 file whose stored series was
        // already capped when it was written — there is no "unknown denominator" case
        // left to fall back to.
        let mut bundle = six_node_bundle();
        for version in [5, 6] {
            bundle["version"] = json!(version);
            let out = run(
                &bundle,
                SsnNavigatorOptions {
                    query: NavMode::Overview,
                    max_merge_events: 2,
                    ..Default::default()
                },
            );
            assert_eq!(out["max_merge_events"], json!(2));
            assert!(out["merge_event_total"].as_u64().unwrap() > 0);
            assert!(out["merge_events"].as_u64().unwrap() > 0);
        }
    }
}
