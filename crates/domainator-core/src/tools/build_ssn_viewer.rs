//! `build-ssn-viewer`: build the compact, MST-based `.ssnv` bundle (and the
//! standalone HTML shell) for the interactive SSN viewer. Mirrors
//! `domainator.build_ssn_viewer`: the bundle carries only the MST, an
//! MST-component hierarchy, and threshold merge events (all O(n)); the browser
//! reconstructs clusters client-side by cutting the tree.

use std::collections::HashSet;
use std::io;

use libdeflater::{CompressionLvl, Compressor};
use serde_json::{json, Map, Value};

use crate::matrix::ssn::{
    build_mst_component_hierarchy, component_size_summary_by_threshold, filter_merge_event_rows,
    threshold_merge_event_rows, HierNode, MaxTree, MergeEvent, MERGE_IMPACT_MIN_CHILD,
};
use crate::matrix::DataMatrix;
use crate::tools::build_ssn::{read_metadata, subset_matrix};

pub const SSN_VIEWER_BUNDLE_FORMAT: &str = "domainator_ssn_viewer_bundle";
pub const SSN_VIEWER_BUNDLE_VERSION: i64 = 1;

/// Options for [`build_ssn_viewer_bundle`].
pub struct BuildSsnViewerOptions {
    pub name: String,
    pub metadata_files: Vec<String>,
    pub subset_labels: Option<HashSet<String>>,
    pub merge_impact_metric: String,
    pub max_merge_events: usize,
    pub color_by: Option<String>,
    pub label_by: Option<String>,
}

impl Default for BuildSsnViewerOptions {
    fn default() -> Self {
        BuildSsnViewerOptions {
            name: "Domainator SSN Viewer".to_string(),
            metadata_files: Vec::new(),
            subset_labels: None,
            merge_impact_metric: MERGE_IMPACT_MIN_CHILD.to_string(),
            max_merge_events: crate::matrix::ssn::DEFAULT_MAX_MERGE_EVENTS,
            color_by: None,
            label_by: None,
        }
    }
}

/// A finite `f64` as a JSON number, or `null` for non-finite values (mirrors
/// Python's `_json_ready` inf handling).
fn num_or_null(x: f64) -> Value {
    if x.is_finite() {
        json!(x)
    } else {
        Value::Null
    }
}

fn merge_event_json(e: &MergeEvent) -> Value {
    json!({
        "edge_index": e.edge_index,
        "summary_row_from": e.summary_row_from,
        "summary_row_to": e.summary_row_to,
        "threshold_from_value": num_or_null(e.threshold_from_value),
        "threshold_from": e.threshold_from,
        "threshold_to": e.threshold_to,
        "threshold_value": num_or_null(e.threshold_value),
        "merge_impact": num_or_null(e.merge_impact),
        "delta_largest": num_or_null(e.delta_largest),
        "delta_avg_non_singleton": num_or_null(e.delta_avg_non_singleton),
    })
}

fn slider_stops_json(events: &[MergeEvent]) -> Value {
    let mut stops = vec![json!({
        "edge_index": -1,
        "threshold_label": "∞",
        "threshold_value": Value::Null,
    })];
    for e in events {
        stops.push(json!({
            "edge_index": e.edge_index,
            "threshold_label": e.threshold_to,
            "threshold_value": num_or_null(e.threshold_value),
        }));
    }
    Value::Array(stops)
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

/// Metadata "int"/"float"/"str" inference over present values (value-based, like
/// Python's `_infer_metadata_type`): non-numeric ⇒ str; all whole numbers ⇒ int;
/// else float.
fn infer_meta_type(present: &[&str]) -> &'static str {
    if present.is_empty() {
        return "str";
    }
    let mut nums = Vec::with_capacity(present.len());
    for s in present {
        match s.parse::<f64>() {
            Ok(v) => nums.push(v),
            Err(_) => return "str",
        }
    }
    if nums.iter().all(|v| v.fract() == 0.0) {
        "int"
    } else {
        "float"
    }
}

/// Build the `metadata` payload (`{columns:[{name,type}], rows:[[...]]}`) for the
/// given node order.
fn metadata_payload(names: &[String], metadata_files: &[String]) -> io::Result<(Value, Vec<String>)> {
    let mut columns: Vec<Value> = Vec::new();
    let mut col_names: Vec<String> = Vec::new();
    // Per-column per-node normalized cell values.
    let mut col_cells: Vec<Vec<Value>> = Vec::new();

    for path in metadata_files {
        let (mcols, map) = read_metadata(path)?;
        for (ci, col) in mcols.iter().enumerate() {
            let per_node: Vec<Option<String>> =
                names.iter().map(|nm| map.get(nm).and_then(|v| v[ci].clone())).collect();
            let present: Vec<&str> = per_node.iter().filter_map(|v| v.as_deref()).collect();
            let ty = infer_meta_type(&present);
            columns.push(json!({"name": col, "type": ty}));
            col_names.push(col.clone());
            let cells: Vec<Value> = per_node
                .iter()
                .map(|v| match v {
                    None => Value::Null,
                    Some(s) => match ty {
                        "int" => json!(s.parse::<f64>().map(|x| x as i64).unwrap_or(0)),
                        "float" => num_or_null(s.parse::<f64>().unwrap_or(f64::NAN)),
                        _ => json!(s),
                    },
                })
                .collect();
            col_cells.push(cells);
        }
    }

    // Transpose col_cells -> row-major rows.
    let mut rows: Vec<Value> = Vec::with_capacity(names.len());
    for i in 0..names.len() {
        let row: Vec<Value> = col_cells.iter().map(|c| c[i].clone()).collect();
        rows.push(Value::Array(row));
    }

    Ok((json!({"columns": columns, "rows": rows}), col_names))
}

/// Build the SSN viewer bundle as a JSON value. Errors if the matrix (after
/// subsetting) is not symmetric or if `color_by`/`label_by` name a missing
/// metadata column.
pub fn build_ssn_viewer_bundle(matrix: &DataMatrix, opts: &BuildSsnViewerOptions) -> io::Result<Value> {
    let matrix = subset_matrix(matrix, &opts.subset_labels);
    if !matrix.symmetric_labels() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Input does not have symmetric axis labels. Can only build an SSN viewer bundle from a symmetric matrix.",
        ));
    }
    let names = matrix.rows.clone();
    let (metadata, col_names) = metadata_payload(&names, &opts.metadata_files)?;

    if let Some(c) = &opts.color_by {
        if !col_names.iter().any(|n| n == c) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Requested color_by column '{c}' was not found in the merged metadata."),
            ));
        }
    }
    if let Some(l) = &opts.label_by {
        if !col_names.iter().any(|n| n == l) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Requested label_by column '{l}' was not found in the merged metadata."),
            ));
        }
    }

    let tree = MaxTree::from_matrix(&matrix);
    let component_summary = component_size_summary_by_threshold(&tree, &opts.merge_impact_metric);
    let merge_events =
        filter_merge_event_rows(&threshold_merge_event_rows(&component_summary), opts.max_merge_events);
    let hierarchy = build_mst_component_hierarchy(&tree);

    let mst_edges: Vec<Value> =
        tree.mst_edges().map(|(i, j, w)| json!([i, j, w])).collect();
    let ccbt: Vec<Value> = tree
        .cluster_count_by_threshold()
        .into_iter()
        .map(|(t, c)| Value::Array(vec![num_or_null(t), num_or_null(c)]))
        .collect();
    let ebt: Vec<Value> = tree
        .edges_by_threshold()
        .into_iter()
        .map(|(e, t)| Value::Array(vec![num_or_null(e), num_or_null(t)]))
        .collect();
    let events_json: Vec<Value> = merge_events.iter().map(merge_event_json).collect();
    let slider_stops = slider_stops_json(&merge_events);
    let hier_nodes: Vec<Value> = hierarchy.nodes.iter().map(hier_node_json).collect();

    let hierarchy_json = json!({
        "nodes": hier_nodes,
        "roots": hierarchy.roots,
        "leaf_order": hierarchy.leaf_order,
    });

    let graph = json!({
        "nodes": names,
        "mst_edges": mst_edges,
        "cluster_count_by_threshold": ccbt,
        "edges_by_threshold": ebt,
        "merge_impact_metric": opts.merge_impact_metric,
        "merge_event_series": events_json,
        "slider_stops": slider_stops,
        "hierarchy": hierarchy_json,
    });

    let mut bundle = Map::new();
    bundle.insert("format".into(), json!(SSN_VIEWER_BUNDLE_FORMAT));
    bundle.insert("version".into(), json!(SSN_VIEWER_BUNDLE_VERSION));
    bundle.insert("name".into(), json!(opts.name));
    bundle.insert("domainator_version".into(), json!(env!("CARGO_PKG_VERSION")));
    bundle.insert("graph".into(), graph);
    bundle.insert("metadata".into(), metadata);
    bundle.insert(
        "defaults".into(),
        json!({
            "color_by": opts.color_by,
            "label_by": opts.label_by,
        }),
    );

    Ok(Value::Object(bundle))
}

/// Serialize a bundle to compact JSON bytes.
pub fn serialize_bundle(bundle: &Value) -> Vec<u8> {
    serde_json::to_vec(bundle).expect("bundle serialization")
}

/// Gzip-compress `data` (the browser decompresses via `DecompressionStream`).
pub fn gzip(data: &[u8]) -> Vec<u8> {
    let mut c = Compressor::new(CompressionLvl::default());
    let bound = c.gzip_compress_bound(data.len());
    let mut out = vec![0u8; bound];
    let n = c.gzip_compress(data, &mut out).expect("gzip");
    out.truncate(n);
    out
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64[(n >> 18 & 63) as usize] as char);
        out.push(B64[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 { B64[(n >> 6 & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { B64[(n & 63) as usize] as char } else { '=' });
    }
    out
}

/// Render the standalone HTML viewer. When `embedded_bundle_gzip` is provided the
/// gzip bundle is base64-embedded so the viewer autoloads it; otherwise the shell
/// opens local `.ssnv` files via its built-in picker.
pub fn render_html(title: &str, embedded_bundle_gzip: Option<&[u8]>) -> String {
    const SHELL: &str = include_str!("../../assets/ssn_viewer_shell.html");
    let mut html = SHELL.replace("@@SSN_VIEWER_TITLE@@", &html_escape(title));
    if let Some(gz) = embedded_bundle_gzip {
        let b64 = base64_encode(gz);
        html = html.replace(
            "const EMBEDDED_BUNDLE_BASE64 = null;",
            &format!("const EMBEDDED_BUNDLE_BASE64 = \"{b64}\";"),
        );
    }
    html
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_reference_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn html_render_substitutes_tokens() {
        let html = render_html("My <SSN>", None);
        assert!(html.contains("My &lt;SSN&gt;"));
        assert!(!html.contains("@@SSN_VIEWER_TITLE@@"));
        assert!(html.contains("const EMBEDDED_BUNDLE_BASE64 = null;"));
        let html2 = render_html("x", Some(b"gzipbytes"));
        assert!(html2.contains("const EMBEDDED_BUNDLE_BASE64 = \""));
        assert!(!html2.contains("const EMBEDDED_BUNDLE_BASE64 = null;"));
    }
}
