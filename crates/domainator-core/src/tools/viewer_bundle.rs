//! The Domainator Viewer bundle (`.dvb`) format, and the parts of it every view shares.
//!
//! Mirrors `domainator.viewer_bundle`. A bundle is gzip-compressed JSON that the
//! Domainator Viewer ([`super::viewer_html`]) opens: `{format, version, name,
//! domainator_version, nodes, metadata, defaults, views}`, where `views` maps a view name
//! (`"ssn"`, written by `build_ssn`; `"tree"`, by `build_tree`; `"projection"`, by
//! `build_projection`) to that view's data about `nodes`. A bundle may also carry an
//! `app_state` section written by the viewer's "Save session".
//!
//! Networks were written to a format of their own before, `.dsnv`; see
//! [`super::ssn_bundle`], which reads those as the `.dvb` bundle they correspond to.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::io;

use libdeflater::{CompressionLvl, Compressor};
use serde_json::{json, Map, Value};

use crate::matrix::cytoscape::{Cell, ColType, NodeTable};

pub const VIEWER_BUNDLE_FORMAT: &str = "domainator_viewer_bundle";
/// Bump for any change to the schema, keeping the old version in
/// [`SUPPORTED_VIEWER_BUNDLE_VERSIONS`] when the change is additive.
///
/// Version 2 (upstream `aa7078c`) adds optional saved-session fields only: ordered
/// node/label styles in `app_state.view` and categorical shape assignments in
/// `app_state.shapes`. Nothing this port writes or reads changed.
pub const VIEWER_BUNDLE_VERSION: i64 = 2;
/// Versions this build reads.
pub const SUPPORTED_VIEWER_BUNDLE_VERSIONS: [i64; 2] = [1, 2];

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, msg.into())
}

/// A finite `f64` as a JSON number, or `null` for non-finite values (upstream's
/// `json_ready` turns infinities into `None`, and JSON has no NaN).
pub fn num_or_null(x: f64) -> Value {
    if x.is_finite() {
        json!(x)
    } else {
        Value::Null
    }
}

/// `"int"`, `"float"` or `"str"`: how the viewer reads a metadata column
/// (`infer_metadata_type`). Value-based, so a numeric column with missing values, which
/// pandas holds as floats, is still `"int"` when every value is whole.
pub fn metadata_column_type(table: &NodeTable, column: usize) -> &'static str {
    match table.coltypes[column] {
        ColType::Integer => "int",
        ColType::Str => "str",
        ColType::Real => {
            let whole = table.cells.iter().all(|row| match row[column] {
                Cell::Real(v) => v.fract() == 0.0,
                _ => true,
            });
            if whole {
                "int"
            } else {
                "float"
            }
        }
    }
}

/// A bundle's `metadata`: `{columns: [{name, type}], rows}`, one row per node in the
/// table's order (`metadata_payload`).
pub fn metadata_payload(table: &NodeTable) -> Value {
    let types: Vec<&str> = (0..table.columns.len())
        .map(|c| metadata_column_type(table, c))
        .collect();
    let columns: Vec<Value> = table
        .columns
        .iter()
        .zip(&types)
        .map(|(name, ty)| json!({"name": name, "type": ty}))
        .collect();
    let rows: Vec<Value> = table
        .cells
        .iter()
        .map(|row| {
            Value::Array(
                row.iter()
                    .zip(&types)
                    .map(|(cell, ty)| match (cell, *ty) {
                        (Cell::Missing, _) => Value::Null,
                        (Cell::Int(v), _) => json!(v),
                        (Cell::Real(v), "int") => json!(*v as i64),
                        (Cell::Real(v), _) => num_or_null(*v),
                        (Cell::Text(s), _) => json!(s),
                    })
                    .collect(),
            )
        })
        .collect();
    json!({"columns": columns, "rows": rows})
}

/// The settings the viewer opens with.
#[derive(Default)]
pub struct ViewerDefaults<'a> {
    /// The metadata column the viewer colors nodes by at first.
    pub color_by: Option<&'a str>,
    /// Maps `color_by` values to hex colors, the column's starting palette in the viewer.
    /// Values not in it are gray. Requires `color_by`.
    pub color_table: Option<&'a HashMap<String, String>>,
    /// The metadata column the viewer labels nodes by at first.
    pub label_by: Option<&'a str>,
    /// Numeric metadata columns to color as categories rather than as a gradient.
    pub categorical: &'a [String],
}

/// A bundle's `defaults` (`viewer_defaults`), checking the named columns against the
/// node table.
pub fn viewer_defaults(table: &NodeTable, defaults: &ViewerDefaults<'_>) -> io::Result<Value> {
    let has = |name: &str| table.columns.iter().any(|c| c == name);
    if let Some(c) = defaults.color_by.filter(|c| !has(c)) {
        return Err(invalid(format!(
            "Requested color_by column '{c}' was not found in the merged metadata."
        )));
    }
    if let Some(l) = defaults.label_by.filter(|l| !has(l)) {
        return Err(invalid(format!(
            "Requested label_by column '{l}' was not found in the merged metadata."
        )));
    }
    let mut categorical_columns = defaults.categorical.to_vec();
    if let Some(c) = categorical_columns.iter().find(|c| !has(c)) {
        return Err(invalid(format!(
            "Requested categorical column '{c}' was not found in the merged metadata."
        )));
    }
    let color_table = match (defaults.color_table, defaults.color_by) {
        (None, _) => None,
        (Some(_), None) => {
            return Err(invalid(
                "color_table requires color_by, the metadata column the table colors.",
            ))
        }
        (Some(table_colors), Some(color_by)) => {
            // A color table is a discrete mapping, so a numeric column it colors is a
            // categorical one.
            let column = table.columns.iter().position(|c| c == color_by);
            let numeric = column.is_some_and(|i| metadata_column_type(table, i) != "str");
            if numeric && !categorical_columns.iter().any(|c| c == color_by) {
                categorical_columns.push(color_by.to_string());
            }
            Some(table_colors.iter().collect::<BTreeMap<_, _>>())
        }
    };

    let mut out = json!({
        "color_by": defaults.color_by,
        "label_by": defaults.label_by,
        // Numeric columns the viewer should color with discrete category colors instead
        // of a gradient (e.g. integer cluster numbers). Toggleable in the viewer's color
        // picker; this only sets the initial state.
        "categorical_columns": categorical_columns,
    });
    if let Some(table_colors) = color_table {
        // value -> "#RRGGBB" for the color_by column; the viewer starts from it in place
        // of its default palette. Optional, so bundles built without one are unchanged.
        out["color_table"] = json!(table_colors);
    }
    Ok(out)
}

/// The colors to record in a bundle's `defaults` (upstream's `viewer_colors`). The
/// viewer assigns default colors the way `get_palette` does, so a palette built here
/// only goes in when the viewer could not reproduce it: when it was offset or capped. A
/// given `--color_table` always goes in.
pub fn viewer_colors<'a>(
    given: Option<&'a HashMap<String, String>>,
    resolved: Option<&'a HashMap<String, String>>,
    palette_offset: usize,
    max_color_groups: Option<usize>,
) -> Option<&'a HashMap<String, String>> {
    if given.is_some() || (palette_offset == 0 && max_color_groups.is_none()) {
        given
    } else {
        resolved
    }
}

/// A `domainator_viewer_bundle` (`viewer_bundle`): `view` is the one view this bundle
/// holds, under `view_name`, and `table`'s rows describe `nodes` in the same order.
pub fn viewer_bundle(
    name: &str,
    nodes: &[String],
    table: &NodeTable,
    defaults: Value,
    view_name: &str,
    view: Value,
) -> Value {
    let mut views = Map::new();
    views.insert(view_name.to_string(), view);
    let mut bundle = Map::new();
    bundle.insert("format".into(), json!(VIEWER_BUNDLE_FORMAT));
    bundle.insert("version".into(), json!(VIEWER_BUNDLE_VERSION));
    bundle.insert("name".into(), json!(name));
    // Upstream's meaning of this key is "which domainator release wrote it"; the viewer
    // writes the same value into the bundles it saves.
    bundle.insert(
        "domainator_version".into(),
        json!(crate::version::UPSTREAM_VERSION),
    );
    bundle.insert("nodes".into(), json!(nodes));
    bundle.insert("metadata".into(), metadata_payload(table));
    bundle.insert("defaults".into(), defaults);
    bundle.insert("views".into(), Value::Object(views));
    Value::Object(bundle)
}

/// The bundle as a bundle file holds it: compact JSON, gzip-compressed
/// (`serialize_bundle`). Key order is insertion order here and sorted upstream; readers
/// compare parsed JSON.
pub fn serialize_bundle(bundle: &Value) -> Vec<u8> {
    gzip(&serde_json::to_vec(bundle).expect("bundle serialization"))
}

/// Gzip-compress `data` (the browser decompresses via `DecompressionStream`). The
/// header's mtime is zero, as upstream's `gzip.compress(..., mtime=0)`.
pub fn gzip(data: &[u8]) -> Vec<u8> {
    let mut c = Compressor::new(CompressionLvl::default());
    let bound = c.gzip_compress_bound(data.len());
    let mut out = vec![0u8; bound];
    let n = c.gzip_compress(data, &mut out).expect("gzip");
    out.truncate(n);
    out
}

/// The name a viewer output gets when `--name` is not given: the stem of the html
/// output, or failing that the dvb output.
pub fn default_name(html_out: Option<&str>, dvb_out: Option<&str>) -> String {
    html_out
        .or(dvb_out)
        .and_then(|p| std::path::Path::new(p).file_stem())
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> NodeTable {
        NodeTable {
            names: vec!["a".into(), "b".into()],
            columns: vec!["group".into(), "size".into(), "score".into()],
            coltypes: vec![ColType::Str, ColType::Real, ColType::Real],
            cells: vec![
                vec![Cell::Text("g1".into()), Cell::Real(3.0), Cell::Real(0.5)],
                vec![Cell::Missing, Cell::Missing, Cell::Real(f64::INFINITY)],
            ],
        }
    }

    #[test]
    fn metadata_types_follow_the_values_not_the_storage() {
        // `size` is stored as floats (a missing value upcast it, as pandas does) but every
        // value is whole, so the viewer reads it as int.
        let payload = metadata_payload(&table());
        assert_eq!(
            payload["columns"],
            json!([
                {"name": "group", "type": "str"},
                {"name": "size", "type": "int"},
                {"name": "score", "type": "float"},
            ])
        );
        assert_eq!(payload["rows"], json!([["g1", 3, 0.5], [null, null, null]]));
    }

    #[test]
    fn a_color_table_makes_its_numeric_column_categorical() {
        let colors: HashMap<String, String> = [("3".to_string(), "#FF0000".to_string())].into();
        let defaults = viewer_defaults(
            &table(),
            &ViewerDefaults {
                color_by: Some("size"),
                color_table: Some(&colors),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            defaults,
            json!({"color_by": "size", "label_by": null, "categorical_columns": ["size"],
                   "color_table": {"3": "#FF0000"}})
        );
        let err = viewer_defaults(
            &table(),
            &ViewerDefaults {
                label_by: Some("nope"),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("label_by column 'nope'"));
    }

    #[test]
    fn only_an_offset_or_capped_palette_goes_into_the_defaults() {
        let built: HashMap<String, String> = HashMap::new();
        assert!(viewer_colors(None, Some(&built), 0, None).is_none());
        assert!(viewer_colors(None, Some(&built), 2, None).is_some());
        assert!(viewer_colors(None, Some(&built), 0, Some(3)).is_some());
    }
}
