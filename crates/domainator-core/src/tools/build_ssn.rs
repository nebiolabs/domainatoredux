//! `build_ssn`: build a sequence-similarity network from a symmetric score matrix, for
//! Cytoscape (XGMML) or the Domainator Viewer (a `.dvb` bundle and its page). Mirrors
//! `domainator.build_ssn`: three edge modes (all edges above `--threshold`, `--mst`,
//! `--knn`, the last two combinable), optional MST edge marks, connected-component
//! clustering (`SSN_cluster`), TSV node metadata, and node coloring. The viewer always
//! shows the network's maximum spanning tree above `--threshold`, whatever the edge mode.

use std::collections::{HashMap, HashSet};
use std::io;

use serde_json::Value;

use crate::matrix::cytoscape::{Cell, ColType, NodeTable};
use crate::matrix::mst_knn::check_sparsify_data_type;
use crate::matrix::ssn::{mst_knn_edge_list, MaxTree, MERGE_IMPACT_MIN_CHILD};
use crate::matrix::{require_similarity, DataMatrix, Storage};
use crate::tools::node_table::{read_node_table, resolve_colors, ColorOptions, LeadingColumn};
use crate::tools::ssn_bundle::ssn_view;
use crate::tools::viewer_bundle::{viewer_bundle, viewer_colors, viewer_defaults, ViewerDefaults};

pub const SCORE_COLUMN: &str = "SSN_SCORE";
pub const CLUSTER_COLUMN: &str = "SSN_cluster";

/// What goes into the Domainator Viewer bundle (`--html_out`/`--dvb_out`).
pub struct ViewerOptions {
    /// The network's name, shown in the viewer.
    pub name: String,
    /// How the viewer's split chart measures a merge.
    pub merge_impact_metric: String,
    /// The metadata column the viewer labels nodes by at first.
    pub label_by: Option<String>,
    /// Numeric metadata columns the viewer colors as categories, not a gradient.
    pub categorical: Vec<String>,
}

impl Default for ViewerOptions {
    fn default() -> Self {
        ViewerOptions {
            name: String::new(),
            merge_impact_metric: MERGE_IMPACT_MIN_CHILD.to_string(),
            label_by: None,
            categorical: Vec::new(),
        }
    }
}

/// Options for [`build_ssn`].
#[derive(Default)]
pub struct BuildSsnOptions {
    /// `--threshold`: exclude edges with weight `<= threshold`.
    pub threshold: f64,
    /// Add an `SSN_cluster` column of size-ranked connected-component ids.
    pub cluster: bool,
    /// Keep only maximum-spanning-tree edges.
    pub mst: bool,
    /// Keep OR-symmetric kNN edges (`k >= 1`); with `mst`, the union. On its own it
    /// may split a cluster the full graph held together, since it does not preserve
    /// connected components.
    pub knn: Option<usize>,
    /// `--mark_mst`: give every edge a boolean `MST` property.
    pub mark_mst: bool,
    /// TSV metadata files keyed by node id (first column).
    pub metadata_files: Vec<String>,
    /// Keep only these labels (rows/cols) if set.
    pub subset_labels: Option<HashSet<String>>,
    /// Color nodes by this metadata column.
    pub color_by: Option<String>,
    /// Explicit value → hex color map (`--color_table`).
    pub color_table: Option<HashMap<String, String>>,
    /// Give distinct colors only to this many of the largest `--color_by` groups;
    /// the rest share the neutral gray. `None` colors every group.
    pub max_color_groups: Option<usize>,
    /// Skip this many palette colors before coloring the `--color_by` groups.
    /// Ignored with a `color_table`.
    pub palette_offset: usize,
    /// Build the viewer bundle too (`--html_out`/`--dvb_out`).
    pub viewer: Option<ViewerOptions>,
}

/// The assembled network, ready for the CLI to write (XGMML / cluster TSV /
/// color-table TSV).
pub struct BuildSsnResult {
    pub table: NodeTable,
    pub edges: Vec<(usize, usize, f64)>,
    /// With `--mark_mst`, whether each edge is in the maximum spanning tree.
    pub edge_mst: Option<Vec<bool>>,
    pub node_fills: Option<Vec<Option<String>>>,
    pub resolved_color_table: Option<HashMap<String, String>>,
    /// Whether some node has no `--color_by` value while the palette was built here
    /// (not given with `--color_table`): upstream's palette then holds a `None` entry
    /// in the neutral gray, which `--color_table_out` writes last.
    pub palette_has_missing: bool,
    /// Size-ranked cluster id per node (aligned to `table.names`), when clustered.
    pub cluster_labels: Option<Vec<i64>>,
    /// The Domainator Viewer bundle, when [`BuildSsnOptions::viewer`] asked for one.
    pub bundle: Option<Value>,
}

pub fn subset_matrix(m: &DataMatrix, subset: &Option<HashSet<String>>) -> DataMatrix {
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
    let keep: Vec<usize> = (0..m.nrows())
        .filter(|&i| subset.contains(&m.rows[i]))
        .collect();
    let dense = m.densify();
    let nc = m.ncols();
    let mut sub = Vec::with_capacity(keep.len() * keep.len());
    for &r in &keep {
        for &c in &keep {
            sub.push(dense[r * nc + c]);
        }
    }
    let names: Vec<String> = keep.iter().map(|&i| m.rows[i].clone()).collect();
    let row_lengths = m
        .row_lengths
        .as_ref()
        .map(|v| keep.iter().map(|&i| v[i]).collect());
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
        Uf {
            parent: (0..n).collect(),
        }
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

pub(crate) fn cluster_from_graph(m: &DataMatrix, lb: f64) -> Vec<i64> {
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

/// Connected components of an explicit edge list, size-ranked like the other two
/// clustering paths. Used for `--knn` without `--mst`, where the selected edges are
/// the only evidence of what is connected.
fn cluster_from_edges(n: usize, edges: &[(usize, usize, f64)]) -> Vec<i64> {
    let mut uf = Uf::new(n);
    for &(a, b, _) in edges {
        uf.union(a, b);
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

/// Build the SSN. `matrix` must have symmetric labels after subsetting.
pub fn build_ssn(matrix: &DataMatrix, opts: &BuildSsnOptions) -> io::Result<BuildSsnResult> {
    let invalid = |msg: String| io::Error::new(io::ErrorKind::InvalidInput, msg);
    check_sparsify_data_type(&matrix.data_type, opts.mst, opts.knn).map_err(invalid)?;
    let matrix = subset_matrix(matrix, &opts.subset_labels);
    if !matrix.symmetric_labels() {
        return Err(invalid(
            "Input does not have symmetric axis labels. Can only build an SSN from a symmetric matrix."
                .into(),
        ));
    }
    require_similarity(&matrix.data_type, "build_ssn").map_err(invalid)?;
    let n = matrix.nrows();
    let names = matrix.rows.clone();
    let (use_mst, knn_k) = (opts.mst, opts.knn);

    // `--mark_mst` only annotates edges, so it keeps the graph-derived clustering. The
    // viewer's network is the spanning tree.
    let tree =
        (use_mst || opts.mark_mst || opts.viewer.is_some()).then(|| MaxTree::from_matrix(&matrix));

    // Edges per mode.
    let edges: Vec<(usize, usize, f64)> = if use_mst && knn_k.is_none() {
        let t = tree.as_ref().unwrap();
        let mut out = Vec::new();
        for e in &t.edges {
            if e.weight <= opts.threshold {
                break;
            }
            let (s, tt) = orient_by_name(&matrix, e.i, e.j);
            out.push((s, tt, e.weight));
        }
        out
    } else if let Some(k) = knn_k {
        mst_knn_edge_list(&matrix, k, opts.threshold, use_mst)
            .into_iter()
            .map(|(a, b, s)| {
                let (x, y) = orient_by_name(&matrix, a, b);
                (x, y, s)
            })
            .collect()
    } else {
        default_edges(&matrix, opts.threshold)
    };

    // `--mark_mst`: which edges are in the spanning tree (above the threshold).
    let edge_mst = opts.mark_mst.then(|| {
        let t = tree.as_ref().expect("a tree is built for --mark_mst");
        let in_tree: HashSet<(usize, usize)> = t
            .edges
            .iter()
            .take_while(|e| e.weight > opts.threshold)
            .map(|e| (e.i.min(e.j), e.i.max(e.j)))
            .collect();
        edges
            .iter()
            .map(|&(a, b, _)| in_tree.contains(&(a.min(b), a.max(b))))
            .collect()
    });

    // Cluster labels (SSN_cluster column). A pure `--knn` graph drops edges the MST
    // would have kept, so its components have to be read off the edges actually
    // selected rather than off the input graph or the spanning tree.
    let cluster_labels: Option<Vec<i64>> = if opts.cluster {
        Some(if !use_mst && knn_k.is_some() {
            cluster_from_edges(n, &edges)
        } else {
            match &tree {
                Some(t) if use_mst => cluster_from_tree(t, opts.threshold),
                _ => cluster_from_graph(&matrix, opts.threshold),
            }
        })
    } else {
        None
    };

    // Node table: SSN_cluster first (if any), then metadata columns.
    let leading = cluster_labels
        .iter()
        .map(|labels| LeadingColumn {
            name: CLUSTER_COLUMN.to_string(),
            coltype: ColType::Integer,
            cells: labels.iter().map(|&l| Cell::Int(l)).collect(),
        })
        .collect();
    let table = read_node_table(&names, leading, &opts.metadata_files)?;

    let colors = resolve_colors(
        &table,
        &ColorOptions {
            color_by: opts.color_by.as_deref(),
            color_table: opts.color_table.as_ref(),
            max_color_groups: opts.max_color_groups,
            palette_offset: opts.palette_offset,
        },
    );

    let bundle = match &opts.viewer {
        None => None,
        Some(viewer) => {
            let defaults = viewer_defaults(
                &table,
                &ViewerDefaults {
                    color_by: opts.color_by.as_deref(),
                    color_table: viewer_colors(
                        opts.color_table.as_ref(),
                        colors.as_ref().map(|c| &c.table),
                        opts.palette_offset,
                        opts.max_color_groups,
                    ),
                    label_by: viewer.label_by.as_deref(),
                    categorical: &viewer.categorical,
                },
            )?;
            let tree = tree.as_ref().expect("a tree is built for the viewer");
            let view = ssn_view(tree, opts.threshold, &viewer.merge_impact_metric)?;
            Some(viewer_bundle(
                &viewer.name,
                &names,
                &table,
                defaults,
                "ssn",
                view,
            ))
        }
    };

    let (node_fills, resolved_color_table, palette_has_missing) = match colors {
        Some(c) => (Some(c.fills), Some(c.table), c.palette_has_missing),
        None => (None, None, false),
    };
    Ok(BuildSsnResult {
        table,
        edges,
        edge_mst,
        node_fills,
        resolved_color_table,
        palette_has_missing,
        cluster_labels,
        bundle,
    })
}
