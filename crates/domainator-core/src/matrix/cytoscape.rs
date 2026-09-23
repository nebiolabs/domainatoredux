//! Cytoscape XGMML export for SSNs. Mirrors `domainator.cytoscape`: a `<graph>`
//! of `<node>`s (id `n{i}`, a `Description` att, one typed `<att>` per metadata
//! column, and a `<graphics>` element) and `<edge>`s carrying an `SSN_SCORE`
//! real att. Output is well-formed XGMML that Cytoscape opens directly; the
//! parity gate compares it structurally (node/edge sets + atts), not byte-wise.

use std::io::{self, Write};

use super::text::py_float_str;

/// XGMML attribute type for a metadata column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColType {
    Integer,
    Real,
    Str,
}

impl ColType {
    fn xgmml(&self) -> &'static str {
        match self {
            ColType::Integer => "integer",
            ColType::Real => "real",
            ColType::Str => "string",
        }
    }
}

/// A single metadata cell (missing cells fall back to type defaults on write,
/// matching Python's `check_null`).
#[derive(Clone, Debug)]
pub enum Cell {
    Missing,
    Int(i64),
    Real(f64),
    Text(String),
}

/// Node metadata table: one row per node (in output order), columns typed.
pub struct NodeTable {
    /// Node names (the DataFrame index → the `Description` att).
    pub names: Vec<String>,
    /// Metadata column names, in order.
    pub columns: Vec<String>,
    /// Column types, parallel to `columns`.
    pub coltypes: Vec<ColType>,
    /// `cells[row][col]`.
    pub cells: Vec<Vec<Cell>>,
}

/// Escape a string for use in an XML attribute value.
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(c),
        }
    }
    out
}

/// Render a cell to its att value string, applying `check_null` defaults.
fn cell_value(cell: &Cell, ct: ColType) -> String {
    match (cell, ct) {
        (Cell::Missing, ColType::Str) => String::new(),
        (Cell::Missing, ColType::Integer) => "0".to_string(),
        (Cell::Missing, ColType::Real) => "0.0".to_string(),
        (Cell::Int(v), _) => v.to_string(),
        (Cell::Real(v), _) => py_float_str(*v),
        (Cell::Text(s), _) => s.clone(),
    }
}

/// Write an XGMML network to `path`.
///
/// `edges` are `(source_node_index, target_node_index, score)`. `node_fills`, if
/// present, gives an optional hex fill color per node (used with `--color_by`).
pub fn write_xgmml(
    path: &str,
    name: &str,
    table: &NodeTable,
    edges: &[(usize, usize, f64)],
    node_fills: Option<&[Option<String>]>,
) -> io::Result<()> {
    let file = std::fs::File::create(path)?;
    let mut w = io::BufWriter::new(file);
    write_xgmml_to(&mut w, name, table, edges, node_fills)
}

/// Serialize the XGMML network to any writer (used by the CLI to render into a
/// buffer for output-size guardrails before touching the filesystem).
pub fn write_xgmml_to<W: Write>(
    w: &mut W,
    name: &str,
    table: &NodeTable,
    edges: &[(usize, usize, f64)],
    node_fills: Option<&[Option<String>]>,
) -> io::Result<()> {
    w.write_all(b"<?xml version='1.0' encoding='utf-8'?>\n")?;
    writeln!(
        w,
        "<graph label=\"{}\" xmlns=\"http://www.cs.rpi.edu/XGMML\">",
        xml_escape(name)
    )?;

    for (i, node_name) in table.names.iter().enumerate() {
        writeln!(w, "  <node id=\"n{i}\" label=\"n{i}\">")?;
        writeln!(
            w,
            "    <att type=\"string\" name=\"Description\" value=\"{}\"/>",
            xml_escape(node_name)
        )?;
        for (c, col) in table.columns.iter().enumerate() {
            let ct = table.coltypes[c];
            let val = cell_value(&table.cells[i][c], ct);
            writeln!(
                w,
                "    <att type=\"{}\" name=\"{}\" value=\"{}\"/>",
                ct.xgmml(),
                xml_escape(col),
                xml_escape(&val)
            )?;
        }
        let fill = node_fills.and_then(|f| f.get(i)).and_then(|x| x.as_ref());
        match fill {
            Some(color) => writeln!(
                w,
                "    <graphics type=\"ELLIPSE\" x=\"0.0\" y=\"0.0\" w=\"25\" h=\"25\" z=\"0.0\" fill=\"{}\"/>",
                xml_escape(color)
            )?,
            None => writeln!(
                w,
                "    <graphics type=\"ELLIPSE\" x=\"0.0\" y=\"0.0\" w=\"25\" h=\"25\" z=\"0.0\"/>"
            )?,
        }
        writeln!(w, "  </node>")?;
    }

    for (i, &(src, tgt, score)) in edges.iter().enumerate() {
        writeln!(w, "  <edge id=\"e{i}\" label=\"e{i}\" source=\"n{src}\" target=\"n{tgt}\">")?;
        writeln!(
            w,
            "    <att type=\"real\" name=\"SSN_SCORE\" value=\"{}\"/>",
            py_float_str(score)
        )?;
        writeln!(w, "  </edge>")?;
    }

    w.write_all(b"</graph>\n")?;
    w.flush()
}
