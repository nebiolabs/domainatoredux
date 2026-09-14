//! Read-only helpers for navigating a `build-ssn-viewer` `.dsnv` bundle.
//!
//! Mirrors `domainator.ssn_bundle`. The bundle is a gzip-compressed JSON document
//! storing an MST merge tree plus a positional metadata table; these helpers let
//! non-browser callers (notably [`super::ssn_navigator`]) reconstruct the clusters
//! active at a chosen similarity threshold and summarize their members, mirroring the
//! viewer's `activeClustersAtThreshold` / `componentMembers`.
//!
//! A v4 bundle may also carry a top-level `app_state` section written by the HTML
//! viewer's "Save session" button. It holds viewer UI state only and is ignored here;
//! the rest of such a file is an ordinary bundle, so a saved session's `metadata`
//! simply reflects whatever was edited in the viewer.

use std::collections::HashMap;
use std::io;
use std::path::Path;

use serde_json::Value;

use crate::matrix::ssn::floor_threshold_value;

pub use super::build_ssn_viewer::{
    SSN_VIEWER_BUNDLE_FORMAT, SSN_VIEWER_BUNDLE_VERSION, SUPPORTED_SSN_VIEWER_BUNDLE_VERSIONS,
};

fn invalid(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

/// Decompress a gzip member fully via libdeflater, sizing the buffer from the ISIZE
/// trailer. ISIZE is mod 2^32, so a >4 GiB bundle needs the grow loop rather than
/// silently truncating.
fn gunzip(bytes: &[u8]) -> io::Result<Vec<u8>> {
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

/// Load and validate a `.dsnv` bundle (gzip JSON, or plain JSON as a fallback).
///
/// Detection is by the gzip magic number, not the file name, so a bundle written
/// under the pre-0.11 `.ssnv` spelling loads unchanged.
pub fn load_bundle(path: &Path) -> io::Result<Value> {
    let raw = std::fs::read(path)?;
    parse_bundle(&raw, &path.display().to_string())
}

/// Validate an in-memory bundle. `display` only names the source in error messages.
///
/// Split out from [`load_bundle`] so the version/format contract can be tested without
/// touching the filesystem, keeping these tests fixture-free like the rest of the suite.
pub fn parse_bundle(raw: &[u8], display: &str) -> io::Result<Value> {
    let bytes = if raw.starts_with(&[0x1f, 0x8b]) { gunzip(raw)? } else { raw.to_vec() };
    let bundle: Value =
        serde_json::from_slice(&bytes).map_err(|e| invalid(format!("invalid JSON: {e}")))?;

    let fmt = bundle.get("format").and_then(Value::as_str);
    if fmt != Some(SSN_VIEWER_BUNDLE_FORMAT) {
        return Err(invalid(format!(
            "'{display}' is not a Domainator Similarity Network Viewer bundle \
             (format='{}', expected '{SSN_VIEWER_BUNDLE_FORMAT}').",
            fmt.unwrap_or("None")
        )));
    }
    let version = bundle.get("version").and_then(Value::as_i64);
    match version {
        Some(v) if SUPPORTED_SSN_VIEWER_BUNDLE_VERSIONS.contains(&v) => {}
        _ => {
            let supported: Vec<String> =
                SUPPORTED_SSN_VIEWER_BUNDLE_VERSIONS.iter().map(|v| v.to_string()).collect();
            return Err(invalid(format!(
                "Unsupported Domainator Similarity Network Viewer bundle version {}; \
                 this build understands version(s) {}.",
                version.map(|v| v.to_string()).unwrap_or_else(|| "None".into()),
                supported.join(", ")
            )));
        }
    }
    Ok(bundle)
}

fn hier_nodes(hierarchy: &Value) -> &Vec<Value> {
    static EMPTY: Vec<Value> = Vec::new();
    hierarchy.get("nodes").and_then(Value::as_array).unwrap_or(&EMPTY)
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
    let lo = merge_thresholds.iter().cloned().fold(f64::INFINITY, f64::min);
    let hi = merge_thresholds.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
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
        .map(|r| r.iter().filter_map(Value::as_u64).map(|v| v as usize).collect())
        .unwrap_or_default();
    let mut stack: Vec<usize> = roots.into_iter().rev().collect();
    while let Some(id) = stack.pop() {
        let Some(component) = nodes.get(id) else { continue };
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
    let Some(component) = nodes.get(component_id) else { return Vec::new() };
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
    hier_nodes(hierarchy).get(component_id).map(|n| u(n, "size")).unwrap_or(0)
}

/// Map node id string -> position in `graph.nodes` (== metadata row index).
pub fn node_index_by_name(bundle: &Value) -> HashMap<&str, usize> {
    bundle
        .get("graph")
        .and_then(|g| g.get("nodes"))
        .and_then(Value::as_array)
        .map(|names| {
            names.iter().enumerate().filter_map(|(i, v)| v.as_str().map(|s| (s, i))).collect()
        })
        .unwrap_or_default()
}

/// The active cluster whose `leaf_order` range covers `node_index`.
pub fn cluster_for_node(
    hierarchy: &Value,
    active_ids: &[usize],
    node_index: usize,
) -> Option<usize> {
    let leaf_order: Vec<usize> = hierarchy
        .get("leaf_order")
        .and_then(Value::as_array)
        .map(|lo| lo.iter().filter_map(Value::as_u64).map(|v| v as usize).collect())
        .unwrap_or_default();
    let position = leaf_order.iter().position(|&v| v == node_index)?;
    let nodes = hier_nodes(hierarchy);
    active_ids.iter().copied().find(|&id| {
        nodes.get(id).is_some_and(|n| {
            let start = u(n, "leaf_start");
            position >= start && position < start + u(n, "leaf_count")
        })
    })
}

/// `np.percentile` with linear interpolation — **not** nearest-rank.
///
/// `values` must be sorted ascending. Note the navigator's `median` is
/// `percentile(50)`, which differs from `matrix_report`'s edge-score median on
/// even-length inputs.
fn percentile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let pos = (sorted.len() - 1) as f64 * q / 100.0;
    let lo = pos.floor() as usize;
    if lo + 1 >= sorted.len() {
        return sorted[sorted.len() - 1];
    }
    let frac = pos - lo as f64;
    sorted[lo] + frac * (sorted[lo + 1] - sorted[lo])
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
    let columns = metadata.get("columns").and_then(Value::as_array).unwrap_or(&empty);
    let rows = metadata.get("rows").and_then(Value::as_array).unwrap_or(&empty);
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

        let mut summary = serde_json::Map::new();
        summary.insert("name".into(), serde_json::json!(name));
        summary.insert("type".into(), serde_json::json!(ty));
        summary.insert("count".into(), serde_json::json!(present.len()));
        summary.insert("missing".into(), serde_json::json!(missing));

        if matches!(ty, "int" | "float") && !present.is_empty() {
            let mut nums: Vec<f64> = present.iter().filter_map(|v| v.as_f64()).collect();
            nums.sort_by(f64::total_cmp);
            let sum: f64 = nums.iter().sum();
            summary.insert("min".into(), serde_json::json!(nums[0]));
            summary.insert("max".into(), serde_json::json!(nums[nums.len() - 1]));
            summary.insert("mean".into(), serde_json::json!(sum / nums.len() as f64));
            summary.insert("p25".into(), serde_json::json!(percentile(&nums, 25.0)));
            summary.insert("median".into(), serde_json::json!(percentile(&nums, 50.0)));
            summary.insert("p75".into(), serde_json::json!(percentile(&nums, 75.0)));
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
            summary.insert("unique".into(), serde_json::json!(ranked.len()));
            summary.insert(
                "top".into(),
                Value::Array(
                    ranked
                        .iter()
                        .take(top_n)
                        .map(|(value, count)| serde_json::json!({"value": value, "count": count}))
                        .collect(),
                ),
            );
            if ranked.len() > top_n {
                summary.insert("truncated".into(), serde_json::json!(ranked.len() - top_n));
            }
        }
        out.push(Value::Object(summary));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
        // 6 is from the future.
        for v in [1, 2, 6] {
            let err = parse(&tiny(v)).unwrap_err().to_string();
            assert!(err.contains("Unsupported"), "version {v} should be refused: {err}");
            assert!(err.contains("3, 4, 5"), "error should name the supported set: {err}");
        }
    }

    #[test]
    fn rejects_a_non_bundle() {
        let err = parse(&json!({"format": "something_else", "version": 5})).unwrap_err().to_string();
        assert!(err.contains("is not a Domainator Similarity Network Viewer bundle"), "{err}");
    }

    #[test]
    fn reads_gzip_and_plain_json_alike() {
        // Detection is by magic number, not file name, so a pre-0.11 `.ssnv` loads.
        let plain = serde_json::to_vec(&tiny(5)).unwrap();
        let gz = super::super::build_ssn_viewer::gzip(&plain);
        assert_eq!(&gz[..2], &[0x1f, 0x8b]);
        assert_eq!(parse_bundle(&gz, "t").unwrap(), parse_bundle(&plain, "t").unwrap());
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
