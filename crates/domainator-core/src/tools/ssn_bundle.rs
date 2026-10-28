//! The network view of a Domainator Viewer bundle: writing it, and read-only helpers for
//! navigating it.
//!
//! Mirrors `domainator.ssn_bundle`. `build_ssn --dvb_out` writes a `.dvb` bundle
//! ([`super::viewer_bundle`]) whose `views.ssn` ([`ssn_view`]) stores an MST merge tree;
//! the bundle's `nodes` and positional `metadata` table describe its nodes. These helpers
//! let non-browser callers (notably [`super::ssn_navigator`]) reconstruct the clusters
//! active at a chosen similarity threshold and summarize their members, mirroring the
//! viewer's `activeClustersAtThreshold` / `componentMembers`.
//!
//! The bundle carries only what cannot be derived from the merge order, so the split-event
//! series, its moving sum and the threshold slider's stops are computed here on demand —
//! [`merge_event_rows`], [`merge_event_series`], [`moving_sum`] and [`slider_stops`] —
//! rather than read out of the file. The viewer does the same thing in JavaScript. A
//! caller therefore picks its own `max_merge_events` at the point of display instead of
//! inheriting one chosen when the file was written.
//!
//! A bundle may also carry a top-level `app_state` section written by the viewer's "Save
//! session" button. It holds viewer UI state only and is ignored here; the rest of such a
//! file is an ordinary bundle, so a saved session's `metadata` simply reflects whatever
//! was edited in the viewer.
//!
//! Earlier versions wrote networks to their own format, `.dsnv`
//! ([`SSN_VIEWER_BUNDLE_FORMAT`]); [`load_bundle`] and the viewer still read those, as the
//! `.dvb` bundle they correspond to ([`upgrade_ssn_viewer_bundle`]). Pre-v6 `.dsnv` files
//! also carry derived keys; they are **ignored in favor of recomputing**, so a capped
//! series written into an old file cannot silently limit what is shown now.

use crate::stats::percentile;
use std::collections::HashMap;
use std::io;
use std::path::Path;

use serde_json::{json, Map, Value};

use crate::matrix::ssn::{
    build_mst_component_hierarchy, component_size_summary_by_threshold, filter_merge_event_rows,
    floor_threshold_value, merge_event_moving_sum, threshold_merge_event_rows,
    threshold_slider_stops, HierNode, MaxTree, MergeEvent, MovingSum, SliderStop,
    MERGE_IMPACT_CHOICES, MERGE_IMPACT_MIN_CHILD, MOVING_SUM_GRID_POINTS,
    MOVING_SUM_WINDOW_FRACTION,
};
use crate::tools::viewer_bundle::{
    num_or_null, SUPPORTED_VIEWER_BUNDLE_VERSIONS, VIEWER_BUNDLE_FORMAT,
};

/// The format of the network bundles (`.dsnv`) that `build_ssn_viewer` wrote before
/// networks moved into the Domainator Viewer bundle. Read, never written.
pub const SSN_VIEWER_BUNDLE_FORMAT: &str = "domainator_ssn_viewer_bundle";
/// The `.dsnv` versions this build reads. v3 added per-event merge sizes and the moving
/// sum; v4 an optional top-level `app_state`; v5 `graph.merge_event_total` and
/// `graph.max_merge_events`; v6 dropped every derived key, leaving `graph` holding only
/// `nodes`, `mst_edges`, `merge_impact_metric` and `hierarchy` — all that `views.ssn`
/// holds now. Everything v6 needs has been in the format since v3, so all of these
/// versions read the same way.
pub const SUPPORTED_SSN_VIEWER_BUNDLE_VERSIONS: [i64; 4] = [3, 4, 5, 6];
/// What a network view holds; the rest of a `.dsnv` graph is derived, and recomputed
/// instead.
const SSN_VIEW_KEYS: [&str; 3] = ["mst_edges", "merge_impact_metric", "hierarchy"];

fn invalid(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

fn hier_node_json(n: &HierNode) -> Value {
    let parent = match n.parent {
        Some(p) => json!(p),
        None => Value::Null,
    };
    if n.is_leaf {
        json!({
            "id": n.id,
            "kind": "leaf",
            "node_index": n.node_index,
            "size": n.size,
            "parent": parent,
            "leaf_start": n.leaf_start,
            "leaf_count": n.leaf_count,
        })
    } else {
        json!({
            "id": n.id,
            "kind": "cluster",
            "left": n.left,
            "right": n.right,
            "threshold": num_or_null(n.threshold.unwrap_or(f64::NAN)),
            "size": n.size,
            "parent": parent,
            "leaf_start": n.leaf_start,
            "leaf_count": n.leaf_count,
        })
    }
}

/// A bundle's `views.ssn` for the network of `tree`'s edges that score above
/// `threshold` (`ssn_view`).
///
/// Only what cannot be derived from the merge order itself. The split-event series, its
/// moving sum and the threshold slider's stops are all a pure function of (nodes,
/// mst_edges, merge_impact_metric), so readers compute them rather than carrying a copy:
/// the viewer via its JavaScript ports, other readers via [`merge_event_rows`] /
/// [`slider_stops`]. That keeps the file a function of the network alone — no selection
/// baked in at build time — and keeps it O(nodes) on a network with hundreds of
/// thousands of split events.
pub fn ssn_view(tree: &MaxTree, threshold: f64, merge_impact_metric: &str) -> io::Result<Value> {
    if !MERGE_IMPACT_CHOICES.contains(&merge_impact_metric) {
        let mut choices = MERGE_IMPACT_CHOICES.to_vec();
        choices.sort_unstable();
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("merge_impact_metric must be one of {choices:?}"),
        ));
    }
    let edges: Vec<(usize, usize, f64)> = tree
        .mst_edges()
        .filter(|&(_, _, score)| score > threshold)
        .collect();
    let hierarchy = build_mst_component_hierarchy(&MaxTree::from_mst_edges(tree.n_nodes, &edges));
    Ok(json!({
        "mst_edges": edges.iter().map(|&(i, j, w)| json!([i, j, w])).collect::<Vec<_>>(),
        "merge_impact_metric": merge_impact_metric,
        "hierarchy": {
            "nodes": hierarchy.nodes.iter().map(hier_node_json).collect::<Vec<_>>(),
            "roots": hierarchy.roots,
            "leaf_order": hierarchy.leaf_order,
        },
    }))
}

/// Decompress a gzip member fully via libdeflater, sizing the buffer from the ISIZE
/// trailer. ISIZE is mod 2^32, so a >4 GiB bundle needs the grow loop rather than
/// silently truncating.
pub fn gunzip(bytes: &[u8]) -> io::Result<Vec<u8>> {
    if bytes.len() < 18 {
        return Err(invalid("truncated gzip".into()));
    }
    let isize_hint = u32::from_le_bytes(bytes[bytes.len() - 4..].try_into().unwrap()) as usize;
    let mut dec = libdeflater::Decompressor::new();
    let mut cap = isize_hint.max(1 << 16);
    loop {
        let mut out = vec![0u8; cap];
        match dec.gzip_decompress(bytes, &mut out) {
            Ok(n) => {
                out.truncate(n);
                return Ok(out);
            }
            Err(libdeflater::DecompressionError::InsufficientSpace) => {
                cap = cap.saturating_mul(2);
                if cap > (1 << 33) {
                    return Err(invalid("bundle too large to decompress".into()));
                }
            }
            Err(e) => return Err(invalid(format!("gzip decompress failed: {e:?}"))),
        }
    }
}

/// The `.dvb` bundle a `.dsnv` one corresponds to, `app_state` included
/// (`upgrade_ssn_viewer_bundle`).
pub fn upgrade_ssn_viewer_bundle(bundle: &Value) -> Value {
    let graph = &bundle["graph"];
    let mut view = Map::new();
    for key in SSN_VIEW_KEYS {
        if let Some(value) = graph.get(key) {
            view.insert(key.into(), value.clone());
        }
    }
    let mut views = Map::new();
    views.insert("ssn".into(), Value::Object(view));
    let mut upgraded = Map::new();
    upgraded.insert("format".into(), json!(VIEWER_BUNDLE_FORMAT));
    upgraded.insert(
        "version".into(),
        json!(SUPPORTED_VIEWER_BUNDLE_VERSIONS.iter().max()),
    );
    upgraded.insert(
        "name".into(),
        bundle.get("name").cloned().unwrap_or(Value::Null),
    );
    upgraded.insert(
        "domainator_version".into(),
        bundle
            .get("domainator_version")
            .cloned()
            .unwrap_or(Value::Null),
    );
    upgraded.insert(
        "nodes".into(),
        graph.get("nodes").cloned().unwrap_or(json!([])),
    );
    upgraded.insert("metadata".into(), bundle["metadata"].clone());
    upgraded.insert(
        "defaults".into(),
        bundle.get("defaults").cloned().unwrap_or(json!({})),
    );
    upgraded.insert("views".into(), Value::Object(views));
    if let Some(app_state) = bundle.get("app_state") {
        upgraded.insert("app_state".into(), app_state.clone());
    }
    Value::Object(upgraded)
}

/// Load a bundle with a network view (gzip JSON, or plain JSON as a fallback).
///
/// A `.dsnv` file is returned as the `.dvb` bundle it corresponds to. Detection is by
/// the gzip magic number and the `format` key, not the file name, so a bundle written
/// under the pre-0.11 `.ssnv` spelling loads unchanged.
pub fn load_bundle(path: &Path) -> io::Result<Value> {
    let raw = std::fs::read(path)?;
    parse_bundle(&raw, &path.display().to_string())
}

/// Parse and check a bundle file's bytes against the formats and versions this build
/// reads, without requiring any particular view. `display` only names the source in
/// error messages. A `.dsnv` file comes back as its `.dvb` equivalent.
pub fn parse_any_bundle(raw: &[u8], display: &str) -> io::Result<Value> {
    let bytes = if raw.starts_with(&[0x1f, 0x8b]) {
        gunzip(raw)?
    } else {
        raw.to_vec()
    };
    let bundle: Value =
        serde_json::from_slice(&bytes).map_err(|e| invalid(format!("invalid JSON: {e}")))?;

    let fmt = bundle.get("format").and_then(Value::as_str);
    let supported: &[i64] = match fmt {
        Some(SSN_VIEWER_BUNDLE_FORMAT) => &SUPPORTED_SSN_VIEWER_BUNDLE_VERSIONS,
        Some(VIEWER_BUNDLE_FORMAT) => &SUPPORTED_VIEWER_BUNDLE_VERSIONS,
        _ => {
            return Err(invalid(format!(
                "'{display}' is not a Domainator Viewer bundle \
                 (format='{}', expected '{VIEWER_BUNDLE_FORMAT}').",
                fmt.unwrap_or("None")
            )))
        }
    };
    let version = bundle.get("version").and_then(Value::as_i64);
    if !version.is_some_and(|v| supported.contains(&v)) {
        let supported: Vec<String> = supported.iter().map(|v| v.to_string()).collect();
        return Err(invalid(format!(
            "Unsupported {} version {}; this build understands version(s) {}.",
            fmt.unwrap_or("None"),
            version
                .map(|v| v.to_string())
                .unwrap_or_else(|| "None".into()),
            supported.join(", ")
        )));
    }
    if fmt == Some(SSN_VIEWER_BUNDLE_FORMAT) {
        return Ok(upgrade_ssn_viewer_bundle(&bundle));
    }
    Ok(bundle)
}

/// Validate an in-memory bundle that must hold a network view. `display` only names the
/// source in error messages.
///
/// Split out from [`load_bundle`] so the version/format contract can be tested without
/// touching the filesystem, keeping these tests fixture-free like the rest of the suite.
pub fn parse_bundle(raw: &[u8], display: &str) -> io::Result<Value> {
    let bundle = parse_any_bundle(raw, display)?;
    if network(&bundle).is_none() {
        let views: Vec<&str> = bundle
            .get("views")
            .and_then(Value::as_object)
            .map(|v| v.keys().map(String::as_str).collect())
            .unwrap_or_default();
        let views = if views.is_empty() {
            "none".to_string()
        } else {
            views.join(", ")
        };
        return Err(invalid(format!(
            "'{display}' has no network view (it has: {views})."
        )));
    }
    Ok(bundle)
}

/// A bundle's network view, `views.ssn`.
pub fn network(bundle: &Value) -> Option<&Value> {
    bundle.get("views")?.get("ssn")
}

/// A bundle's node ids, `nodes`.
pub fn node_names(bundle: &Value) -> &[Value] {
    bundle
        .get("nodes")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// Adapt a loaded bundle to what the `ssn_hierarchy` mirror in [`crate::matrix::ssn`]
/// expects: those functions need only `n_nodes` and the MST edges, and a bundle carries
/// both. Everything else on a real [`MaxTree`] is derived from the input matrix, which
/// a bundle does not carry. Mirrors upstream's `ssn_bundle._BundleTree`.
pub fn bundle_tree(bundle: &Value) -> MaxTree {
    let n_nodes = node_names(bundle).len();
    let edges: Vec<(usize, usize, f64)> = network(bundle)
        .and_then(|g| g.get("mst_edges"))
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    let a = row.as_array()?;
                    Some((
                        a.first()?.as_u64()? as usize,
                        a.get(1)?.as_u64()? as usize,
                        a.get(2)?.as_f64()?,
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    MaxTree::from_mst_edges(n_nodes, &edges)
}

/// The bundle's **complete, uncapped** split-event series.
///
/// Recomputed from `views.ssn.mst_edges` rather than read from the file. Before `.dsnv`
/// v6 a capped copy was stored under `graph.merge_event_series`; deriving it here means a caller is
/// never silently limited to whatever cap the file happened to be built with, and
/// pre-v6 bundles get the full series too.
///
/// The metric defaults to the one the bundle records. Pass `merge_impact_metric` to
/// recompute under the other one — the series is a pure function of the merge order and
/// the metric, so nothing about the file constrains the choice.
pub fn merge_event_rows(bundle: &Value, merge_impact_metric: Option<&str>) -> Vec<MergeEvent> {
    let recorded = network(bundle)
        .and_then(|g| g.get("merge_impact_metric"))
        .and_then(Value::as_str);
    let metric = merge_impact_metric
        .or(recorded)
        .unwrap_or(MERGE_IMPACT_MIN_CHILD);
    let summary = component_size_summary_by_threshold(&bundle_tree(bundle), metric);
    threshold_merge_event_rows(&summary)
}

/// The capped selection of split events, as the viewer's split chart plots it.
///
/// `max_merge_events == 0` means no cap. Pass `event_rows` from [`merge_event_rows`] to
/// avoid replaying the merge order twice.
pub fn merge_event_series(
    bundle: &Value,
    max_merge_events: usize,
    event_rows: Option<&[MergeEvent]>,
) -> Vec<MergeEvent> {
    let derived;
    let rows = match event_rows {
        Some(rows) => rows,
        None => {
            derived = merge_event_rows(bundle, None);
            &derived
        }
    };
    filter_merge_event_rows(rows, max_merge_events)
}

/// The centered moving sum of split impact, over the **uncapped** rows.
pub fn moving_sum(bundle: &Value, event_rows: Option<&[MergeEvent]>) -> MovingSum {
    let derived;
    let rows = match event_rows {
        Some(rows) => rows,
        None => {
            derived = merge_event_rows(bundle, None);
            &derived
        }
    };
    merge_event_moving_sum(rows, MOVING_SUM_WINDOW_FRACTION, MOVING_SUM_GRID_POINTS)
}

/// The threshold cuts a slider over this bundle should offer.
///
/// `∞`, one stop per selected split event, then the floor stop that reaches the fully
/// merged network. Stops carry **no** `threshold_index`: that indexed threshold tables
/// which described the full input graph, and no bundle ever carried those.
pub fn slider_stops(
    bundle: &Value,
    max_merge_events: usize,
    event_rows: Option<&[MergeEvent]>,
) -> Vec<SliderStop> {
    let selected = merge_event_series(bundle, max_merge_events, event_rows);
    threshold_slider_stops(&selected, Some(&bundle_tree(bundle)), false)
}

fn hier_nodes(hierarchy: &Value) -> &Vec<Value> {
    static EMPTY: Vec<Value> = Vec::new();
    hierarchy
        .get("nodes")
        .and_then(Value::as_array)
        .unwrap_or(&EMPTY)
}

fn f(node: &Value, key: &str) -> f64 {
    node.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}

fn u(node: &Value, key: &str) -> usize {
    node.get(key).and_then(Value::as_u64).unwrap_or(0) as usize
}

fn is_leaf(node: &Value) -> bool {
    node.get("kind").and_then(Value::as_str) == Some("leaf")
}

/// The cut below which nothing splits: one cluster per connected component.
pub fn coarsest_threshold(hierarchy: &Value) -> f64 {
    let merge_thresholds: Vec<f64> = hier_nodes(hierarchy)
        .iter()
        .filter(|n| !is_leaf(n))
        .map(|n| f(n, "threshold"))
        .collect();
    if merge_thresholds.is_empty() {
        return 0.0;
    }
    let lo = merge_thresholds
        .iter()
        .cloned()
        .fold(f64::INFINITY, f64::min);
    let hi = merge_thresholds
        .iter()
        .cloned()
        .fold(f64::NEG_INFINITY, f64::max);
    floor_threshold_value(lo, hi)
}

/// Component ids of the clusters active at `threshold`.
///
/// Port of the viewer's `activeClustersAtThreshold`: descend into a cluster node when
/// it merged at or below the cut, otherwise emit the whole component. A merge at
/// **exactly** the cut is split back apart — the comparison has to be `<=` because
/// `build_ssn --lb T` keeps only edges scoring strictly above `T`.
///
/// `None` is the bundle's `∞` slider stop (`threshold_value: null`) and means +∞:
/// every merge is cut, so every node is its own cluster. Higher thresholds yield finer
/// partitions, matching similarity-score semantics.
pub fn clusters_at_threshold(hierarchy: &Value, threshold: Option<f64>) -> Vec<usize> {
    let nodes = hier_nodes(hierarchy);
    let cut = threshold.unwrap_or(f64::INFINITY);
    let mut active = Vec::new();
    let roots: Vec<usize> = hierarchy
        .get("roots")
        .and_then(Value::as_array)
        .map(|r| {
            r.iter()
                .filter_map(Value::as_u64)
                .map(|v| v as usize)
                .collect()
        })
        .unwrap_or_default();
    let mut stack: Vec<usize> = roots.into_iter().rev().collect();
    while let Some(id) = stack.pop() {
        let Some(component) = nodes.get(id) else {
            continue;
        };
        if is_leaf(component) {
            active.push(id);
            continue;
        }
        if f(component, "threshold") <= cut {
            stack.push(u(component, "right"));
            stack.push(u(component, "left"));
            continue;
        }
        active.push(id);
    }
    active
}

/// Node indices belonging to `component_id` (a `leaf_order` slice).
pub fn component_members(hierarchy: &Value, component_id: usize) -> Vec<usize> {
    let nodes = hier_nodes(hierarchy);
    let Some(component) = nodes.get(component_id) else {
        return Vec::new();
    };
    let start = u(component, "leaf_start");
    let count = u(component, "leaf_count");
    hierarchy
        .get("leaf_order")
        .and_then(Value::as_array)
        .map(|lo| {
            lo.iter()
                .skip(start)
                .take(count)
                .filter_map(Value::as_u64)
                .map(|v| v as usize)
                .collect()
        })
        .unwrap_or_default()
}

/// Size (leaf count) of a hierarchy component.
pub fn component_size(hierarchy: &Value, component_id: usize) -> usize {
    hier_nodes(hierarchy)
        .get(component_id)
        .map(|n| u(n, "size"))
        .unwrap_or(0)
}

/// Map node id string -> position in `nodes` (== metadata row index).
pub fn node_index_by_name(bundle: &Value) -> HashMap<&str, usize> {
    node_names(bundle)
        .iter()
        .enumerate()
        .filter_map(|(i, v)| v.as_str().map(|s| (s, i)))
        .collect()
}

/// The cluster containing `node_index` at `threshold`, by walking up from its leaf.
///
/// Asking which cluster holds *one* node does not need the whole partition: climb from
/// the node's leaf while the parent merge scores **strictly above** the cut, and the
/// last node reached is the cluster [`clusters_at_threshold`] would have emitted for
/// it. A cluster's ancestors merged later and therefore score lower, so the condition
/// fails once and the walk stops — no other branch of the tree is ever visited.
///
/// That is Θ(depth) rather than Θ(clusters), which matters because `--mode node` asks
/// the question again at every cut-point. The `>` is the same strictly-above `--lb`
/// rule the descent uses, so a merge sitting exactly on the cut is not taken.
///
/// `threshold` is `f64::INFINITY` for the ∞ cut, where every node stands alone: no
/// parent can score above it, so the walk returns the leaf without a special case
/// (leaf ids equal node indices — see `build_mst_component_hierarchy`).
pub fn cluster_for_node(hierarchy: &Value, node_index: usize, threshold: f64) -> Option<usize> {
    let nodes = hier_nodes(hierarchy);
    let mut current = node_index;
    nodes.get(current)?;
    loop {
        let parent = match nodes
            .get(current)
            .and_then(|n| n.get("parent"))
            .and_then(Value::as_u64)
        {
            Some(p) => p as usize,
            None => return Some(current),
        };
        match nodes.get(parent) {
            Some(n) if f(n, "threshold") > threshold => current = parent,
            _ => return Some(current),
        }
    }
}

/// Summarize the metadata distribution across a set of member node indices.
///
/// String columns get the `top_n` most common values with counts, plus the distinct
/// and missing counts. Numeric columns get count/missing and min/max/mean/quartiles.
pub fn summarize_cluster_metadata(
    member_indices: &[usize],
    metadata: &Value,
    top_n: usize,
) -> Vec<Value> {
    let empty = Vec::new();
    let columns = metadata
        .get("columns")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    let rows = metadata
        .get("rows")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    let mut out = Vec::with_capacity(columns.len());

    for (col_idx, column) in columns.iter().enumerate() {
        let name = column.get("name").and_then(Value::as_str).unwrap_or("");
        let ty = column.get("type").and_then(Value::as_str).unwrap_or("str");
        let values: Vec<&Value> = member_indices
            .iter()
            .filter_map(|&i| rows.get(i))
            .filter_map(|r| r.get(col_idx))
            .collect();
        let present: Vec<&Value> = values.iter().copied().filter(|v| !v.is_null()).collect();
        let missing = values.len() - present.len();

        let mut summary = Map::new();
        summary.insert("name".into(), json!(name));
        summary.insert("type".into(), json!(ty));
        summary.insert("count".into(), json!(present.len()));
        summary.insert("missing".into(), json!(missing));

        if matches!(ty, "int" | "float") && !present.is_empty() {
            let mut nums: Vec<f64> = present.iter().filter_map(|v| v.as_f64()).collect();
            nums.sort_by(f64::total_cmp);
            let sum: f64 = nums.iter().sum();
            summary.insert("min".into(), json!(nums[0]));
            summary.insert("max".into(), json!(nums[nums.len() - 1]));
            summary.insert("mean".into(), json!(sum / nums.len() as f64));
            summary.insert("p25".into(), json!(percentile(&nums, 25.0)));
            summary.insert("median".into(), json!(percentile(&nums, 50.0)));
            summary.insert("p75".into(), json!(percentile(&nums, 75.0)));
        } else {
            let mut counts: HashMap<String, usize> = HashMap::new();
            for v in &present {
                let key = match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                *counts.entry(key).or_insert(0) += 1;
            }
            let mut ranked: Vec<(String, usize)> = counts.into_iter().collect();
            // (-count, value): Rust's byte-wise str Ord matches Python's code-point order.
            ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            summary.insert("unique".into(), json!(ranked.len()));
            summary.insert(
                "top".into(),
                Value::Array(
                    ranked
                        .iter()
                        .take(top_n)
                        .map(|(value, count)| json!({"value": value, "count": count}))
                        .collect(),
                ),
            );
            if ranked.len() > top_n {
                summary.insert("truncated".into(), json!(ranked.len() - top_n));
            }
        }
        out.push(Value::Object(summary));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny(version: i64) -> Value {
        json!({
            "format": SSN_VIEWER_BUNDLE_FORMAT,
            "version": version,
            "name": "t",
            "graph": {"nodes": ["a", "b"], "hierarchy": {"nodes": [], "roots": [], "leaf_order": []}},
            "metadata": {"columns": [], "rows": []},
        })
    }

    fn parse(v: &Value) -> io::Result<Value> {
        parse_bundle(&serde_json::to_vec(v).unwrap(), "test")
    }

    #[test]
    fn accepts_supported_versions_and_rejects_others() {
        for v in SUPPORTED_SSN_VIEWER_BUNDLE_VERSIONS {
            assert!(parse(&tiny(v)).is_ok(), "version {v} should load");
        }
        // 1 is what this port used to write; 2 predates the additive v3 fields;
        // 7 is from the future.
        for v in [1, 2, 7] {
            let err = parse(&tiny(v)).unwrap_err().to_string();
            assert!(
                err.contains("Unsupported"),
                "version {v} should be refused: {err}"
            );
            assert!(
                err.contains("3, 4, 5, 6"),
                "error should name the supported set: {err}"
            );
        }
    }

    #[test]
    fn rejects_a_non_bundle() {
        let err = parse(&json!({"format": "something_else", "version": 5}))
            .unwrap_err()
            .to_string();
        assert!(err.contains("is not a Domainator Viewer bundle"), "{err}");
    }

    #[test]
    fn a_dsnv_bundle_reads_as_the_dvb_bundle_it_corresponds_to() {
        let mut old = tiny(6);
        old["graph"]["mst_edges"] = json!([[0, 1, 5.0]]);
        old["graph"]["merge_impact_metric"] = json!("product");
        old["graph"]["merge_event_series"] = json!([]); // a pre-v6 derived key
        old["app_state"] = json!({"state_version": 1});
        let upgraded = parse(&old).unwrap();
        assert_eq!(upgraded["format"], json!(VIEWER_BUNDLE_FORMAT));
        assert_eq!(upgraded["version"], json!(1));
        assert_eq!(upgraded["nodes"], json!(["a", "b"]));
        assert_eq!(upgraded["defaults"], json!({}));
        assert_eq!(upgraded["app_state"], json!({"state_version": 1}));
        let mut keys: Vec<&String> = upgraded["views"]["ssn"]
            .as_object()
            .unwrap()
            .keys()
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, ["hierarchy", "merge_impact_metric", "mst_edges"]);
    }

    #[test]
    fn a_bundle_without_a_network_is_refused() {
        let tree = json!({
            "format": VIEWER_BUNDLE_FORMAT, "version": 1, "name": "t", "nodes": ["a"],
            "metadata": {"columns": [], "rows": [[]]}, "defaults": {},
            "views": {"tree": {}},
        });
        let err = parse(&tree).unwrap_err().to_string();
        assert!(err.contains("has no network view (it has: tree)"), "{err}");
        assert!(parse_any_bundle(&serde_json::to_vec(&tree).unwrap(), "t").is_ok());
    }

    #[test]
    fn reads_gzip_and_plain_json_alike() {
        // Detection is by magic number, not file name, so a pre-0.11 `.ssnv` loads.
        let plain = serde_json::to_vec(&tiny(5)).unwrap();
        let gz = crate::tools::viewer_bundle::gzip(&plain);
        assert_eq!(&gz[..2], &[0x1f, 0x8b]);
        assert_eq!(
            parse_bundle(&gz, "t").unwrap(),
            parse_bundle(&plain, "t").unwrap()
        );
    }

    /// A real bundle built the way `build_ssn --dvb_out` builds one, so the derivation
    /// under test is exercised against the values the matrix itself produces.
    fn bundle_from_matrix(m: &crate::matrix::DataMatrix) -> Value {
        let view = ssn_view(&MaxTree::from_matrix(m), 0.0, MERGE_IMPACT_MIN_CHILD).unwrap();
        json!({
            "format": VIEWER_BUNDLE_FORMAT, "version": 1, "name": "t", "nodes": m.rows,
            "metadata": {"columns": [], "rows": []}, "defaults": {},
            "views": {"ssn": view},
        })
    }

    fn six_node_matrix() -> crate::matrix::DataMatrix {
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
        crate::matrix::DataMatrix {
            rows: labels.clone(),
            cols: labels,
            row_lengths: None,
            col_lengths: None,
            data_type: "score".into(),
            storage: crate::matrix::Storage::Dense(dense),
        }
    }

    #[test]
    fn a_network_view_carries_only_what_cannot_be_derived() {
        let bundle = bundle_from_matrix(&six_node_matrix());
        let mut keys: Vec<&str> = bundle["views"]["ssn"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, vec!["hierarchy", "merge_impact_metric", "mst_edges"]);
    }

    #[test]
    fn the_view_keeps_only_edges_above_the_threshold() {
        // `build_ssn --threshold 4.5 --dvb_out`: the 4.0 MST edge is cut, so n3 and n4
        // fall into separate components of the view's forest.
        let tree = MaxTree::from_matrix(&six_node_matrix());
        let view = ssn_view(&tree, 4.5, MERGE_IMPACT_MIN_CHILD).unwrap();
        let weights: Vec<f64> = view["mst_edges"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e[2].as_f64().unwrap())
            .collect();
        assert_eq!(weights, [10.0, 9.0, 8.0, 5.0]);
        assert_eq!(view["hierarchy"]["roots"].as_array().unwrap().len(), 2);
        assert!(ssn_view(&tree, 0.0, "nope").is_err());
    }

    #[test]
    fn derived_series_matches_the_matrix_it_was_built_from() {
        use crate::matrix::ssn::{
            component_size_summary_by_threshold, filter_merge_event_rows,
            threshold_merge_event_rows, MaxTree,
        };
        let m = six_node_matrix();
        let bundle = bundle_from_matrix(&m);
        let tree = MaxTree::from_matrix(&m);
        let expected = threshold_merge_event_rows(&component_size_summary_by_threshold(
            &tree,
            MERGE_IMPACT_MIN_CHILD,
        ));

        let rows = merge_event_rows(&bundle, None);
        let shape = |v: &[MergeEvent]| -> Vec<(i64, f64, f64)> {
            v.iter()
                .map(|e| (e.edge_index, e.threshold_value, e.merge_impact))
                .collect()
        };
        assert_eq!(shape(&rows), shape(&expected));
        assert_eq!(
            shape(&merge_event_series(&bundle, 2, None)),
            shape(&filter_merge_event_rows(&expected, 2))
        );
        // Passing the rows in must give the same answer as letting it replay them.
        assert_eq!(
            shape(&merge_event_series(&bundle, 2, Some(&rows))),
            shape(&merge_event_series(&bundle, 2, None))
        );
        assert_eq!(moving_sum(&bundle, Some(&rows)), moving_sum(&bundle, None));

        // The stops end at the floor and carry no threshold_index: a bundle has no
        // threshold tables to index.
        let stops = slider_stops(&bundle, 0, Some(&rows));
        assert!(stops.iter().all(|s| s.threshold_index.is_none()));
        assert_eq!(stops[0].threshold_value, None, "the ∞ stop leads");
        // The 2.0 edge closes a cycle, so the weakest MST edge is 4.0 and the floor
        // sits 1% of the MST weight range below it.
        let weakest = tree
            .mst_edges()
            .map(|(_, _, w)| w)
            .fold(f64::INFINITY, f64::min);
        assert_eq!(weakest, 4.0);
        assert!(
            stops.last().unwrap().threshold_value.unwrap() < weakest,
            "floor must clear the weakest merge"
        );
        assert_eq!(stops.len(), rows.len() + 2, "∞ + one per event + floor");
    }

    #[test]
    fn a_stale_capped_series_in_a_pre_v6_bundle_is_ignored() {
        // The regression this change exists to prevent: a v5 file stored a series
        // capped by whoever wrote it, and honoring that would silently limit what a
        // caller can see now. Everything is recomputed from the merge order instead.
        let bundle = bundle_from_matrix(&six_node_matrix());
        let full = merge_event_rows(&bundle, None);
        assert!(full.len() > 1, "need a series worth truncating");

        let mut graph = bundle["views"]["ssn"].clone();
        graph["nodes"] = bundle["nodes"].clone();
        graph["merge_event_series"] = json!([]);
        graph["merge_event_total"] = json!(1);
        graph["max_merge_events"] = json!(1);
        graph["slider_stops"] = json!([]);
        graph["merge_moving_sum"] = json!({"window": 0.0, "x": [], "y": []});
        let old = json!({
            "format": SSN_VIEWER_BUNDLE_FORMAT, "version": 5, "name": "t",
            "graph": graph, "metadata": {"columns": [], "rows": []},
        });
        let bundle = parse_bundle(&serde_json::to_vec(&old).unwrap(), "t").unwrap();

        assert_eq!(merge_event_rows(&bundle, None).len(), full.len());
        assert_eq!(slider_stops(&bundle, 0, None).len(), full.len() + 2);
        assert!(!moving_sum(&bundle, None).x.is_empty());
    }

    #[test]
    fn the_metric_can_be_overridden_at_read_time() {
        // The series is a pure function of the merge order and the metric, so nothing
        // about the file constrains the choice — the bundle's own value is only a
        // default.
        use crate::matrix::ssn::MERGE_IMPACT_PRODUCT;
        let bundle = bundle_from_matrix(&six_node_matrix());
        let default = merge_event_rows(&bundle, None);
        let product = merge_event_rows(&bundle, Some(MERGE_IMPACT_PRODUCT));
        assert_eq!(
            default.len(),
            product.len(),
            "same events, different weighting"
        );
        assert_ne!(
            default.iter().map(|e| e.merge_impact).collect::<Vec<_>>(),
            product.iter().map(|e| e.merge_impact).collect::<Vec<_>>()
        );
    }

    #[test]
    fn walking_up_from_a_leaf_finds_the_cluster_the_descent_would_emit() {
        // `--mode node` asks this at every cut-point, so it walks instead of
        // partitioning. The two must never disagree — including at the ∞ cut, where
        // every node stands alone, and below the coarsest merge.
        let bundle = bundle_from_matrix(&six_node_matrix());
        let hierarchy = &bundle["views"]["ssn"]["hierarchy"];
        let cuts: Vec<f64> = vec![f64::INFINITY, 10.0, 9.0, 8.0, 5.0, 4.0, 2.0, 1.9, -1.0];
        for cut in cuts {
            let active =
                clusters_at_threshold(hierarchy, if cut.is_infinite() { None } else { Some(cut) });
            for node_index in 0..6 {
                let walked = cluster_for_node(hierarchy, node_index, cut)
                    .unwrap_or_else(|| panic!("no cluster for node {node_index} at {cut}"));
                let descended = active
                    .iter()
                    .copied()
                    .find(|&id| component_members(hierarchy, id).contains(&node_index))
                    .unwrap_or_else(|| panic!("descent lost node {node_index} at {cut}"));
                assert_eq!(walked, descended, "node {node_index} at cut {cut}");
            }
        }
    }

    #[test]
    fn percentile_interpolates_linearly() {
        // np.percentile([1,2,3,4], 25) == 1.75, not 1 or 2.
        let v = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(percentile(&v, 0.0), 1.0);
        assert_eq!(percentile(&v, 25.0), 1.75);
        assert_eq!(percentile(&v, 50.0), 2.5);
        assert_eq!(percentile(&v, 75.0), 3.25);
        assert_eq!(percentile(&v, 100.0), 4.0);
    }

    #[test]
    fn a_merge_exactly_at_the_cut_is_split_apart() {
        // Two leaves joined at 5.0. Cutting at 5.0 must split them, because
        // `--lb 5` keeps only edges scoring strictly above 5.
        let h = json!({
            "nodes": [
                {"kind": "leaf", "size": 1, "leaf_start": 0, "leaf_count": 1},
                {"kind": "leaf", "size": 1, "leaf_start": 1, "leaf_count": 1},
                {"kind": "cluster", "left": 0, "right": 1, "threshold": 5.0,
                 "size": 2, "leaf_start": 0, "leaf_count": 2},
            ],
            "roots": [2],
            "leaf_order": [0, 1],
        });
        assert_eq!(clusters_at_threshold(&h, Some(5.0)), vec![0, 1]);
        assert_eq!(clusters_at_threshold(&h, Some(4.9)), vec![2]);
        // None is the ∞ stop: every merge cut, every node its own cluster.
        assert_eq!(clusters_at_threshold(&h, None), vec![0, 1]);
        assert_eq!(component_members(&h, 2), vec![0, 1]);
    }
}
