//! `build-ssn`: build a Cytoscape XGMML sequence-similarity network from a
//! symmetric score matrix. Mirrors `domainator.build_ssn`: three edge modes
//! (all-edges above `--lb`, `--mst`, `--mst_knn`), optional connected-component
//! clustering (`SSN_cluster`), TSV node metadata, and node colouring.

use std::collections::{HashMap, HashSet};
use std::io;

use crate::matrix::cytoscape::{Cell, ColType, NodeTable};
use crate::matrix::ssn::{mst_knn_edge_list, MaxTree};
use crate::matrix::{DataMatrix, Storage};

pub const SCORE_COLUMN: &str = "SSN_SCORE";
pub const CLUSTER_COLUMN: &str = "SSN_cluster";

/// Options for [`build_ssn`].
#[derive(Default)]
pub struct BuildSsnOptions {
    /// Exclude edges with weight `<= lb`.
    pub lb: f64,
    /// Add an `SSN_cluster` column of size-ranked connected-component ids.
    pub cluster: bool,
    /// Keep only maximum-spanning-tree edges.
    pub mst: bool,
    /// Keep MST + OR-symmetric kNN edges (`k > 1`).
    pub mst_knn: Option<usize>,
    /// TSV metadata files keyed by node id (first column).
    pub metadata_files: Vec<String>,
    /// Keep only these labels (rows/cols) if set.
    pub subset_labels: Option<HashSet<String>>,
    /// Colour nodes by this metadata column.
    pub color_by: Option<String>,
    /// Explicit value → hex colour map (`--color_table`).
    pub color_table: Option<HashMap<String, String>>,
    /// Give distinct colours only to this many of the largest `--color_by` groups;
    /// the rest share the neutral grey. `None` colours every group.
    pub max_color_groups: Option<usize>,
}

/// The assembled network, ready for the CLI to write (XGMML / cluster TSV /
/// color-table TSV).
pub struct BuildSsnResult {
    pub table: NodeTable,
    pub edges: Vec<(usize, usize, f64)>,
    pub node_fills: Option<Vec<Option<String>>>,
    pub resolved_color_table: Option<HashMap<String, String>>,
    /// Size-ranked cluster id per node (aligned to `table.names`), when clustered.
    pub cluster_labels: Option<Vec<i64>>,
}

/// Colour for a node whose `--color_by` value is missing, or whose group fell outside
/// `--max_color_groups`. Same neutral grey Python uses.
use crate::palette::OTHER_COLOR as DEFAULT_COLOR;

pub(crate) fn subset_matrix(m: &DataMatrix, subset: &Option<HashSet<String>>) -> DataMatrix {
    let subset = match subset {
        Some(s) => s,
        None => {
            return DataMatrix {
                rows: m.rows.clone(),
                cols: m.cols.clone(),
                row_lengths: m.row_lengths.clone(),
                col_lengths: m.col_lengths.clone(),
                data_type: m.data_type.clone(),
                storage: Storage::Dense(m.densify()),
            }
        }
    };
    let keep: Vec<usize> = (0..m.nrows()).filter(|&i| subset.contains(&m.rows[i])).collect();
    let dense = m.densify();
    let nc = m.ncols();
    let mut sub = Vec::with_capacity(keep.len() * keep.len());
    for &r in &keep {
        for &c in &keep {
            sub.push(dense[r * nc + c]);
        }
    }
    let names: Vec<String> = keep.iter().map(|&i| m.rows[i].clone()).collect();
    let row_lengths = m.row_lengths.as_ref().map(|v| keep.iter().map(|&i| v[i]).collect());
    DataMatrix {
        rows: names.clone(),
        cols: names,
        row_lengths: row_lengths.clone(),
        col_lengths: row_lengths,
        data_type: m.data_type.clone(),
        storage: Storage::Dense(sub),
    }
}

/// Size-ranked labels (1-based) from union-find over edges, matching Python's
/// `rename_labels_by_frequency(...) + 1`: assign component ids in node order,
/// then relabel by descending size (ties by ascending original id).
fn size_ranked_labels(n: usize, roots: &[usize]) -> Vec<i64> {
    // Component id per node in first-appearance (node) order.
    let mut root_to_id: HashMap<usize, usize> = HashMap::new();
    let mut comp: Vec<usize> = vec![0; n];
    let mut next = 0;
    for i in 0..n {
        let id = *root_to_id.entry(roots[i]).or_insert_with(|| {
            let v = next;
            next += 1;
            v
        });
        comp[i] = id;
    }
    // Sizes per component id.
    let mut counts: Vec<usize> = vec![0; next];
    for &c in &comp {
        counts[c] += 1;
    }
    // Order ids by size desc, ties by id asc.
    let mut order: Vec<usize> = (0..next).collect();
    order.sort_by(|&a, &b| counts[b].cmp(&counts[a]).then(a.cmp(&b)));
    let mut rank = vec![0i64; next];
    for (new_id, &old_id) in order.iter().enumerate() {
        rank[old_id] = new_id as i64;
    }
    comp.iter().map(|&c| rank[c] + 1).collect()
}

struct Uf {
    parent: Vec<usize>,
}
impl Uf {
    fn new(n: usize) -> Self {
        Uf { parent: (0..n).collect() }
    }
    fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            self.parent[x] = self.parent[self.parent[x]];
            x = self.parent[x];
        }
        x
    }
    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.parent[ra] = rb;
        }
    }
}

fn cluster_from_tree(tree: &MaxTree, lb: f64) -> Vec<i64> {
    let mut uf = Uf::new(tree.n_nodes);
    for e in &tree.edges {
        if e.weight <= lb {
            break; // edges are descending
        }
        uf.union(e.i, e.j);
    }
    let roots: Vec<usize> = (0..tree.n_nodes).map(|i| uf.find(i)).collect();
    size_ranked_labels(tree.n_nodes, &roots)
}

fn cluster_from_graph(m: &DataMatrix, lb: f64) -> Vec<i64> {
    let n = m.nrows();
    let d = m.densify();
    let mut uf = Uf::new(n);
    for r in 1..n {
        for c in 0..r {
            if d[r * n + c] > lb || d[c * n + r] > lb {
                uf.union(r, c);
            }
        }
    }
    let roots: Vec<usize> = (0..n).map(|i| uf.find(i)).collect();
    size_ranked_labels(n, &roots)
}

/// Default-path edges: every off-diagonal pair with `max(M[i,j],M[j,i]) > lb`,
/// oriented so the source has the lexicographically smaller name.
fn default_edges(m: &DataMatrix, lb: f64) -> Vec<(usize, usize, f64)> {
    let n = m.nrows();
    let d = m.densify();
    let mut edges = Vec::new();
    for r in 1..n {
        for c in 0..r {
            let score = d[r * n + c].max(d[c * n + r]);
            if score <= lb {
                continue;
            }
            let (mut s, mut t) = (r, c);
            if m.rows[t] < m.rows[s] {
                std::mem::swap(&mut s, &mut t);
            }
            edges.push((s, t, score));
        }
    }
    edges
}

fn orient_by_name(m: &DataMatrix, a: usize, b: usize) -> (usize, usize) {
    if m.rows[b] < m.rows[a] {
        (b, a)
    } else {
        (a, b)
    }
}

/// Parse a TSV metadata file into (column names, per-index value map). The first
/// column is the index; remaining columns are values keyed by the index string.
pub(crate) fn read_metadata(
    path: &str,
) -> io::Result<(Vec<String>, HashMap<String, Vec<Option<String>>>)> {
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

/// Infer the XGMML column type from the file's present values, upcasting an
/// integer column to real when the left-merge leaves any node without a value
/// (mirrors pandas' NaN-driven int→float upcast).
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

/// Build the SSN. `matrix` must have symmetric labels after subsetting.
pub fn build_ssn(matrix: &DataMatrix, opts: &BuildSsnOptions) -> io::Result<BuildSsnResult> {
    let matrix = subset_matrix(matrix, &opts.subset_labels);
    if !matrix.symmetric_labels() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Input does not have symmetric axis labels. Can only build an SSN from a symmetric matrix.",
        ));
    }
    let n = matrix.nrows();
    let names = matrix.rows.clone();

    let tree = if opts.mst || opts.mst_knn.is_some() {
        Some(MaxTree::from_matrix(&matrix))
    } else {
        None
    };

    // Cluster labels (SSN_cluster column).
    let cluster_labels: Option<Vec<i64>> = if opts.cluster {
        Some(match &tree {
            Some(t) => cluster_from_tree(t, opts.lb),
            None => cluster_from_graph(&matrix, opts.lb),
        })
    } else {
        None
    };

    // Edges per mode.
    let edges: Vec<(usize, usize, f64)> = if opts.mst {
        let t = tree.as_ref().unwrap();
        let mut out = Vec::new();
        for e in &t.edges {
            if e.weight <= opts.lb {
                break;
            }
            let (s, tt) = orient_by_name(&matrix, e.i, e.j);
            out.push((s, tt, e.weight));
        }
        out
    } else if let Some(k) = opts.mst_knn {
        mst_knn_edge_list(&matrix, k, opts.lb)
            .into_iter()
            .map(|(a, b, s)| {
                let (x, y) = orient_by_name(&matrix, a, b);
                (x, y, s)
            })
            .collect()
    } else {
        default_edges(&matrix, opts.lb)
    };

    // Node table: SSN_cluster first (if any), then metadata columns.
    let mut columns: Vec<String> = Vec::new();
    let mut coltypes: Vec<ColType> = Vec::new();
    let mut cells: Vec<Vec<Cell>> = vec![Vec::new(); n];

    if let Some(labels) = &cluster_labels {
        columns.push(CLUSTER_COLUMN.to_string());
        coltypes.push(ColType::Integer);
        for i in 0..n {
            cells[i].push(Cell::Int(labels[i]));
        }
    }

    for path in &opts.metadata_files {
        let (mcols, map) = read_metadata(path)?;
        for (ci, col) in mcols.iter().enumerate() {
            // Gather present values in node order; note missing nodes.
            let mut present: Vec<&str> = Vec::new();
            let mut has_missing = false;
            let per_node: Vec<Option<String>> = names
                .iter()
                .map(|nm| match map.get(nm).and_then(|v| v[ci].clone()) {
                    Some(s) => Some(s),
                    None => {
                        has_missing = true;
                        None
                    }
                })
                .collect();
            for v in per_node.iter().flatten() {
                present.push(v);
            }
            let ct = infer_type(&present, has_missing);
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

    let table = NodeTable { names: names.clone(), columns, coltypes, cells };

    // Colour resolution.
    let (node_fills, resolved_color_table) = resolve_colors(&table, opts);

    Ok(BuildSsnResult { table, edges, node_fills, resolved_color_table, cluster_labels })
}

fn resolve_colors(
    table: &NodeTable,
    opts: &BuildSsnOptions,
) -> (Option<Vec<Option<String>>>, Option<HashMap<String, String>>) {
    let color_by = match &opts.color_by {
        Some(c) => c,
        None => return (None, None),
    };
    let col_idx = match table.columns.iter().position(|c| c == color_by) {
        Some(i) => i,
        None => return (None, None),
    };
    let ct = table.coltypes[col_idx];
    let value_of = |i: usize| -> Option<String> {
        match &table.cells[i][col_idx] {
            Cell::Missing => None,
            Cell::Int(v) => Some(v.to_string()),
            Cell::Real(v) => Some(crate::matrix::text::py_float_str(*v)),
            Cell::Text(s) => Some(s.clone()),
        }
    };
    let _ = ct;

    // Resolve/extend the colour table.
    let mut table_map = opts.color_table.clone().unwrap_or_default();
    if opts.color_table.is_none() {
        // Per-node values, kept with duplicates so `--max_color_groups` can rank
        // groups by frequency the way Python's pandas Series path does.
        let mut present: Vec<String> = Vec::new();
        let mut counts: HashMap<String, usize> = HashMap::new();
        for i in 0..table.names.len() {
            if let Some(v) = value_of(i) {
                *counts.entry(v.clone()).or_insert(0) += 1;
                present.push(v);
            }
        }
        table_map = crate::palette::get_palette(&present, opts.max_color_groups, Some(&counts));
    }

    let fills: Vec<Option<String>> = (0..table.names.len())
        .map(|i| match value_of(i) {
            Some(v) => Some(table_map.get(&v).cloned().unwrap_or_else(|| DEFAULT_COLOR.to_string())),
            None => Some(DEFAULT_COLOR.to_string()),
        })
        .collect();

    (Some(fills), Some(table_map))
}
