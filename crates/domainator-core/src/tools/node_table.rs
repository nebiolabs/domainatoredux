//! Node metadata tables and coloring, shared by the tools that draw nodes: `build_ssn`,
//! `build_tree` and `build_projection`.
//!
//! Upstream merges each `--metadata` TSV into a pandas frame indexed by the node ids
//! (`viewer_bundle.read_node_metadata`, and the same loop inside `build_ssn`), then colors
//! by one of its columns with `colors.get_palette`. This module is that frame: a
//! [`NodeTable`] whose column types follow the dtypes pandas would infer, so the XGMML
//! writer, the color table and the viewer bundle all see the values pandas would.

use std::collections::HashMap;
use std::io;

use crate::matrix::cytoscape::{Cell, ColType, NodeTable};
use crate::palette::OTHER_COLOR;

/// A parsed metadata TSV: the value column names, and per-index-string values
/// positionally aligned to those columns (`None` = the row omitted that column).
type MetadataFile = (Vec<String>, HashMap<String, Vec<Option<String>>>);

/// Parse a TSV metadata file into (column names, per-index value map). The first
/// column is the index; remaining columns are values keyed by the index string.
pub fn read_metadata(path: &str) -> io::Result<MetadataFile> {
    let text = std::fs::read_to_string(path)?;
    let mut lines = text.lines();
    let header = lines.next().unwrap_or_default();
    let cols: Vec<String> = header.split('\t').skip(1).map(|s| s.to_string()).collect();
    let mut map: HashMap<String, Vec<Option<String>>> = HashMap::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let mut it = line.split('\t');
        let key = it.next().unwrap_or_default().to_string();
        let mut vals: Vec<Option<String>> = Vec::with_capacity(cols.len());
        for _ in 0..cols.len() {
            match it.next() {
                Some(v) if !v.is_empty() => vals.push(Some(v.to_string())),
                _ => vals.push(None),
            }
        }
        map.insert(key, vals);
    }
    Ok((cols, map))
}

/// Infer the column type from the file's present values, upcasting an integer column
/// to real when the left-merge leaves any node without a value (mirrors pandas'
/// NaN-driven int→float upcast).
fn infer_type(present: &[&str], has_missing: bool) -> ColType {
    if present.is_empty() {
        return ColType::Str;
    }
    let all_int = present.iter().all(|s| s.parse::<i64>().is_ok());
    let all_num = present.iter().all(|s| s.parse::<f64>().is_ok());
    if all_int && !has_missing {
        ColType::Integer
    } else if all_num {
        ColType::Real
    } else {
        ColType::Str
    }
}

/// A column computed by the tool rather than read from a file (`build_ssn`'s
/// `SSN_cluster`), placed before the metadata columns as upstream's frame has it.
pub struct LeadingColumn {
    pub name: String,
    pub coltype: ColType,
    pub cells: Vec<Cell>,
}

/// The node table for `names`: the `leading` columns, then every column of each
/// `metadata_files` TSV in turn, left-joined on the node id.
pub fn read_node_table(
    names: &[String],
    leading: Vec<LeadingColumn>,
    metadata_files: &[String],
) -> io::Result<NodeTable> {
    let n = names.len();
    let mut columns: Vec<String> = Vec::new();
    let mut coltypes: Vec<ColType> = Vec::new();
    let mut cells: Vec<Vec<Cell>> = vec![Vec::new(); n];

    for column in leading {
        columns.push(column.name);
        coltypes.push(column.coltype);
        for (row, cell) in cells.iter_mut().zip(column.cells) {
            row.push(cell);
        }
    }

    for path in metadata_files {
        let (mcols, map) = read_metadata(path)?;
        for (ci, col) in mcols.iter().enumerate() {
            let per_node: Vec<Option<String>> = names
                .iter()
                .map(|nm| map.get(nm).and_then(|v| v[ci].clone()))
                .collect();
            let present: Vec<&str> = per_node.iter().flatten().map(String::as_str).collect();
            let ct = infer_type(&present, present.len() < n);
            columns.push(col.clone());
            coltypes.push(ct);
            for (i, v) in per_node.into_iter().enumerate() {
                let cell = match (v, ct) {
                    (None, _) => Cell::Missing,
                    (Some(s), ColType::Integer) => Cell::Int(s.parse().unwrap_or(0)),
                    (Some(s), ColType::Real) => Cell::Real(s.parse().unwrap_or(0.0)),
                    (Some(s), ColType::Str) => Cell::Text(s),
                };
                cells[i].push(cell);
            }
        }
    }

    Ok(NodeTable {
        names: names.to_vec(),
        columns,
        coltypes,
        cells,
    })
}

/// A cell as pandas would print it: `str(value)`, or `None` when missing.
pub fn cell_text(cell: &Cell) -> Option<String> {
    match cell {
        Cell::Missing => None,
        Cell::Int(v) => Some(v.to_string()),
        Cell::Real(v) => Some(crate::matrix::text::py_float_str(*v)),
        Cell::Text(s) => Some(s.clone()),
    }
}

/// The colors of a `--color_by` column.
pub struct NodeColors {
    /// The fill of each node, aligned to the table's rows.
    pub fills: Vec<Option<String>>,
    /// Value -> hex color: the given `--color_table`, or the palette built here.
    pub table: HashMap<String, String>,
    /// Whether some node has no value while the palette was built here (not given with
    /// `--color_table`): upstream's palette then holds a `None` entry in the neutral
    /// gray, which `--color_table_out` writes last.
    pub palette_has_missing: bool,
}

/// How to color nodes: by which column, from which table, and how a built palette is
/// shaped.
#[derive(Default)]
pub struct ColorOptions<'a> {
    pub color_by: Option<&'a str>,
    /// An explicit value → hex color map (`--color_table`).
    pub color_table: Option<&'a HashMap<String, String>>,
    /// Give distinct colors only to this many of the largest groups.
    pub max_color_groups: Option<usize>,
    /// Skip this many palette colors. Ignored with a `color_table`.
    pub palette_offset: usize,
}

/// Color the table's nodes by `opts.color_by`, mirroring `get_palette(node_data[color_by],
/// max_groups=..., offset=...)` plus the writer's gray for values the table lacks. `None`
/// when there is no `color_by` or the table has no such column.
pub fn resolve_colors(table: &NodeTable, opts: &ColorOptions<'_>) -> Option<NodeColors> {
    let col_idx = table
        .columns
        .iter()
        .position(|c| Some(c.as_str()) == opts.color_by)?;
    let value_of = |i: usize| cell_text(&table.cells[i][col_idx]);

    let color_table = match opts.color_table {
        Some(given) => given.clone(),
        None => {
            // Per-node values, kept with duplicates so `max_color_groups` can rank groups
            // by frequency the way Python's pandas Series path does.
            let mut present: Vec<String> = Vec::new();
            let mut counts: HashMap<String, usize> = HashMap::new();
            for i in 0..table.names.len() {
                if let Some(v) = value_of(i) {
                    *counts.entry(v.clone()).or_insert(0) += 1;
                    present.push(v);
                }
            }
            crate::palette::get_palette(
                &present,
                opts.max_color_groups,
                Some(&counts),
                opts.palette_offset,
            )
        }
    };

    let fills: Vec<Option<String>> = (0..table.names.len())
        .map(|i| {
            Some(
                value_of(i)
                    .and_then(|v| color_table.get(&v).cloned())
                    .unwrap_or_else(|| OTHER_COLOR.to_string()),
            )
        })
        .collect();
    let palette_has_missing =
        opts.color_table.is_none() && (0..table.names.len()).any(|i| value_of(i).is_none());
    Some(NodeColors {
        fills,
        table: color_table,
        palette_has_missing,
    })
}
