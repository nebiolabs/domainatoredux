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

/// Escape a string for an element's attribute value, once, the way upstream's
/// `ElementTree` serializer does: `'` stays, and the whitespace an XML parser would
/// normalize away is written as a character reference.
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("&#10;"),
            '\t' => out.push_str("&#09;"),
            '\r' => out.push_str("&#13;"),
            _ => out.push(c),
        }
    }
    out
}

/// Escape the graph label, which upstream writes with `html.escape(quote=True)`.
fn html_escape(s: &str) -> String {
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
    edge_mst: Option<&[bool]>,
    node_fills: Option<&[Option<String>]>,
) -> io::Result<()> {
    let file = std::fs::File::create(path)?;
    let mut w = io::BufWriter::new(file);
    write_xgmml_to(&mut w, name, table, edges, edge_mst, node_fills)
}

/// Serialize the XGMML network to any writer (used by the CLI to render into a
/// buffer for output-size guardrails before touching the filesystem).
pub fn write_xgmml_to<W: Write>(
    w: &mut W,
    name: &str,
    table: &NodeTable,
    edges: &[(usize, usize, f64)],
    edge_mst: Option<&[bool]>,
    node_fills: Option<&[Option<String>]>,
) -> io::Result<()> {
    write_xgmml_with(
        w,
        name,
        table,
        edges,
        &XgmmlExtras {
            edge_mst,
            node_fills,
            ..Default::default()
        },
    )
}

/// The parts of an XGMML network beyond its nodes and scored edges.
pub struct XgmmlExtras<'a> {
    /// The name of the edges' score attribute.
    pub edge_score_name: &'a str,
    /// Whether each edge is in the maximum spanning tree (`--mark_mst`).
    pub edge_mst: Option<&'a [bool]>,
    /// An optional hex fill color per node.
    pub node_fills: Option<&'a [Option<String>]>,
    /// Each node's `(x, y, z)` position, as upstream's `x_col`/`y_col`/`z_col` values:
    /// `y` is negated on the way out, as upstream's writer negates it, and a missing `z`
    /// is written as `0.0`.
    pub positions: Option<&'a [(f64, f64, Option<f64>)]>,
}

impl Default for XgmmlExtras<'_> {
    fn default() -> Self {
        XgmmlExtras {
            edge_score_name: "SSN_SCORE",
            edge_mst: None,
            node_fills: None,
            positions: None,
        }
    }
}

/// [`write_xgmml_to`], with node positions and a choice of edge attribute name.
pub fn write_xgmml_with<W: Write>(
    w: &mut W,
    name: &str,
    table: &NodeTable,
    edges: &[(usize, usize, f64)],
    extras: &XgmmlExtras<'_>,
) -> io::Result<()> {
    w.write_all(b"<?xml version='1.0' encoding='utf-8'?>\n")?;
    writeln!(
        w,
        "<graph label=\"{}\" xmlns=\"http://www.cs.rpi.edu/XGMML\">",
        html_escape(name)
    )?;

    for (i, node_name) in table.names.iter().enumerate() {
        writeln!(w, "  <node id=\"n{i}\" label=\"n{i}\">")?;
        writeln!(
            w,
            "    <att type=\"string\" name=\"Description\" value=\"{}\" />",
            xml_escape(node_name)
        )?;
        for (c, col) in table.columns.iter().enumerate() {
            let ct = table.coltypes[c];
            let val = cell_value(&table.cells[i][c], ct);
            writeln!(
                w,
                "    <att type=\"{}\" name=\"{}\" value=\"{}\" />",
                ct.xgmml(),
                xml_escape(col),
                xml_escape(&val)
            )?;
        }
        let (x, y, z) = match extras.positions.and_then(|p| p.get(i)) {
            Some(&(x, y, z)) => (
                py_float_str(x),
                py_float_str(-y),
                z.map_or_else(|| "0.0".to_string(), py_float_str),
            ),
            None => ("0.0".into(), "0.0".into(), "0.0".into()),
        };
        write!(
            w,
            "    <graphics type=\"ELLIPSE\" x=\"{x}\" y=\"{y}\" w=\"25\" h=\"25\" z=\"{z}\""
        )?;
        match extras
            .node_fills
            .and_then(|f| f.get(i))
            .and_then(|x| x.as_ref())
        {
            Some(color) => writeln!(w, " fill=\"{}\" />", xml_escape(color))?,
            None => writeln!(w, " />")?,
        }
        writeln!(w, "  </node>")?;
    }

    for (i, &(src, tgt, score)) in edges.iter().enumerate() {
        writeln!(
            w,
            "  <edge id=\"e{i}\" label=\"e{i}\" source=\"n{src}\" target=\"n{tgt}\">"
        )?;
        writeln!(
            w,
            "    <att type=\"real\" name=\"{}\" value=\"{}\" />",
            xml_escape(extras.edge_score_name),
            py_float_str(score)
        )?;
        if let Some(mst) = extras.edge_mst {
            writeln!(
                w,
                "    <att type=\"boolean\" name=\"MST\" value=\"{}\" />",
                mst[i]
            )?;
        }
        writeln!(w, "  </edge>")?;
    }

    w.write_all(b"</graph>\n")?;
    w.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Upstream writes each att through `ElementTree` (escaped once, `'` kept,
    /// whitespace as character references) and the graph label through
    /// `html.escape`.
    #[test]
    fn values_are_escaped_once_like_upstream() {
        let table = NodeTable {
            names: vec!["a'b\"c<d>e&f\ng\th\ri".into()],
            columns: vec![],
            coltypes: vec![],
            cells: vec![vec![]],
        };
        let mut out = Vec::new();
        write_xgmml_to(&mut out, "x'&y", &table, &[], None, None).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("<graph label=\"x&#x27;&amp;y\""), "{text}");
        assert!(
            text.contains("value=\"a'b&quot;c&lt;d&gt;e&amp;f&#10;g&#09;h&#13;i\""),
            "{text}"
        );
    }
}
