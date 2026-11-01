//! `--mst_knn` sparsification: prune a square similarity matrix to the
//! maximum spanning tree (MST) plus the OR-symmetric k-nearest-neighbor (kNN)
//! edges. Mirrors `domainator.transform_matrix.apply_mst_knn_sparsification`
//! (the canonical batch path; the Python tools also have a streaming
//! `StreamingMstKnnAccumulator` that is proven equivalent to it).
//!
//! Used by `seq_dist`/`compare_contigs` to keep SSN output small: the result
//! preserves connected-component structure (what mst_knn is used for in
//! clustering) while dropping the bulk of low-weight edges.

use std::collections::{HashMap, HashSet};

use super::{build_sparse_max, csr_index, csr_row, DataMatrix, Storage};

/// A canonical undirected pair, always `(min_index, max_index)`.
type Pair = (usize, usize);
/// The pairs an MST/kNN pass keeps, in upstream's edge-dict insertion order (the MST
/// in tree order, then each node's ranked neighbors in node order), plus the
/// symmetric score chosen for each.
type KeptEdges = (Vec<Pair>, HashMap<Pair, f64>);

/// How the maximum spanning forest breaks ties between equal-scoring edges, which
/// decides which of two equal edges joins the tree. Upstream builds it two ways and
/// documents that they may pick different, equally valid forests: the batch
/// `MaxTree` (`transform_matrix`, `build_ssn`) and the streaming accumulator
/// (`seq_dist`, `compare_contigs`), so each port follows its tool's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeTies {
    /// `MaxTree`'s order: score descending, ties by row-major index descending.
    Batch,
    /// Score descending, ties by (larger index, smaller index) ascending, which is
    /// what the streaming accumulator ends with.
    Streaming,
    /// Score descending, ties by the stored cell's (row, column) ascending: the
    /// streaming accumulator's own order over the cells it is fed, for a matrix that
    /// holds exactly those cells. `kmer_dist` feeds the upper triangle of an uncapped
    /// self-comparison and whole capped rows otherwise, and keeps its matrix that way
    /// until the forest is built.
    Entries,
}

/// `score > lower_bound` (the `include_equal=False` gate used everywhere here).
#[inline]
fn passes_lb(score: f64, lower_bound: f64) -> bool {
    score > lower_bound
}

/// Visit every stored entry `(row, col, value)` of `m` (dense or sparse).
fn for_each_entry(m: &DataMatrix, mut f: impl FnMut(usize, usize, f64)) {
    let nc = m.ncols();
    match &m.storage {
        Storage::Dense(d) => {
            for r in 0..m.nrows() {
                for c in 0..nc {
                    f(r, c, d[r * nc + c]);
                }
            }
        }
        Storage::Sparse {
            values,
            indices,
            indptr,
        } => {
            for r in 0..m.nrows() {
                for k in csr_row(indptr, r) {
                    f(r, csr_index(indices[k]), values[k]);
                }
            }
        }
    }
}

/// Disjoint-set (union-find) with path compression, used for Kruskal's MST.
/// Shared with the `ssn` module (the SSN `MaxTree` builds the same forest).
pub(crate) struct UnionFind {
    parent: Vec<usize>,
}

impl UnionFind {
    pub(crate) fn new(n: usize) -> Self {
        UnionFind {
            parent: (0..n).collect(),
        }
    }
    pub(crate) fn find(&mut self, mut x: usize) -> usize {
        let mut root = x;
        while self.parent[root] != root {
            root = self.parent[root];
        }
        while self.parent[x] != x {
            let next = self.parent[x];
            self.parent[x] = root;
            x = next;
        }
        root
    }
    /// Merge the sets of `x` and `y`; returns `true` if they were distinct.
    pub(crate) fn union(&mut self, x: usize, y: usize) -> bool {
        let (rx, ry) = (self.find(x), self.find(y));
        if rx != ry {
            self.parent[rx] = ry;
            true
        } else {
            false
        }
    }
}

/// Prune `m` (which must be square) to MST ∪ OR-symmetric kNN edges.
///
/// Edge candidates are the *symmetric* scores `max(M[i,j], M[j,i])`; the MST is
/// the maximum spanning forest over them, and each node contributes its top-`k`
/// neighbors by symmetric score (an edge survives if it is in *either*
/// endpoint's top-`k` — the OR-symmetric rule). The diagonal and the original
/// *directional* off-diagonal values of every kept pair are written back,
/// matching the sparse branch of the Python implementation.
/// Compute the kept undirected edge set (canonical pairs `a < b`) for MST ∪
/// OR-symmetric kNN, along with the symmetric candidate scores `max(M[i,j],
/// M[j,i])` per pair. Shared by [`apply_mst_knn`] (which writes the pruned
/// matrix) and `build_ssn`'s `--mst_knn` edge list, so both select identical
/// edges.
pub(crate) fn mst_knn_kept(
    m: &DataMatrix,
    k: usize,
    lower_bound: f64,
    include_mst: bool,
    ties: TreeTies,
) -> KeptEdges {
    let n = m.nrows();

    // Symmetric (undirected) candidate score per canonical pair (min, max).
    let mut sym: HashMap<(usize, usize), f64> = HashMap::new();
    for_each_entry(m, |r, c, v| {
        if v == 0.0 || r == c {
            return;
        }
        let key = if r < c { (r, c) } else { (c, r) };
        let e = sym.entry(key).or_insert(f64::NEG_INFINITY);
        if v > *e {
            *e = v;
        }
    });

    let mut seen: HashSet<Pair> = HashSet::new();
    let mut kept: Vec<Pair> = Vec::new();
    let mut keep = |pair: Pair| {
        if seen.insert(pair) {
            kept.push(pair);
        }
    };

    // --- MST (Kruskal over the maximum spanning forest) ---
    // Skipped entirely for a pure `--knn` graph, which therefore does NOT preserve
    // the connected components of the input.
    // The forest is built over all nonzero edges; an edge is kept only when it also
    // passes the lower bound.
    if include_mst && ties == TreeTies::Batch {
        for e in &super::ssn::MaxTree::from_matrix(m).edges {
            if passes_lb(e.weight, lower_bound) {
                keep((e.i.min(e.j), e.i.max(e.j)));
            }
        }
    }
    if include_mst && ties == TreeTies::Entries {
        let mut cells: Vec<(f64, usize, usize)> = Vec::new();
        for_each_entry(m, |r, c, v| {
            if v != 0.0 && r != c {
                cells.push((v, r, c));
            }
        });
        cells.sort_by(|x, y| y.0.total_cmp(&x.0).then(x.1.cmp(&y.1)).then(x.2.cmp(&y.2)));
        let mut uf = UnionFind::new(n);
        for &(score, r, c) in &cells {
            if uf.union(r, c) && passes_lb(score, lower_bound) {
                keep((r.min(c), r.max(c)));
            }
        }
    }
    if include_mst && ties == TreeTies::Streaming {
        let mut candidates: Vec<(f64, usize, usize)> =
            sym.iter().map(|(&(a, b), &s)| (s, b, a)).collect(); // (score, larger=b, smaller=a)
        candidates.sort_by(|x, y| y.0.total_cmp(&x.0).then(x.1.cmp(&y.1)).then(x.2.cmp(&y.2)));
        let mut uf = UnionFind::new(n);
        for &(score, larger, smaller) in &candidates {
            if uf.union(larger, smaller) && passes_lb(score, lower_bound) {
                keep((smaller, larger));
            }
        }
    }

    // --- OR-symmetric kNN ---
    // `k == 0` selects no neighbors at all, which with include_mst is how
    // `--mst_knn 0` spells "MST only".
    // Each node's neighbors ranked by descending symmetric score, ties by
    // neighbor index ascending; keep the top-k that pass the lower bound.
    let mut adj: Vec<Vec<(f64, usize)>> = vec![Vec::new(); n];
    for (&(a, b), &s) in &sym {
        adj[a].push((s, b));
        adj[b].push((s, a));
    }
    for (node, neighbors) in adj.iter_mut().enumerate().filter(|_| k > 0) {
        neighbors.sort_by(|x, y| y.0.total_cmp(&x.0).then(x.1.cmp(&y.1)));
        let mut selected = 0;
        for &(score, neighbor) in neighbors.iter() {
            if !passes_lb(score, lower_bound) {
                break;
            }
            let edge = if node < neighbor {
                (node, neighbor)
            } else {
                (neighbor, node)
            };
            keep(edge);
            selected += 1;
            if selected >= k {
                break;
            }
        }
    }

    (kept, sym)
}

/// Prune to the OR-symmetric kNN edges, plus the maximum spanning tree when asked.
///
/// `include_mst = true` is `--mst_knn k`; `false` is a pure `--knn k` graph, which does
/// **not** preserve the connected components of the input.
///
/// Kept edges are written as a strict max over the diagonal: both cells of a pair get
/// `max(M[i,j], M[j,i])`, the same score the kNN ranking and the spanning tree select
/// on. The output is therefore symmetric, and the batch and streaming paths now agree
/// on values, not just on which edges are kept.
pub fn apply_mst_knn(
    m: &DataMatrix,
    k: usize,
    lower_bound: f64,
    include_mst: bool,
    ties: TreeTies,
) -> DataMatrix {
    let n = m.nrows();

    // Directional off-diagonal values and the diagonal (random-access lookups).
    let mut cell: HashMap<(usize, usize), f64> = HashMap::new();
    let mut diagonal: HashMap<usize, f64> = HashMap::new();
    for_each_entry(m, |r, c, v| {
        if v == 0.0 {
            return;
        }
        if r == c {
            diagonal.insert(r, v);
        } else {
            cell.insert((r, c), v);
        }
    });

    let (kept, sym) = mst_knn_kept(m, k, lower_bound, include_mst, ties);

    // --- Assemble the pruned matrix ---
    let mut rows_v: Vec<i64> = Vec::new();
    let mut cols_v: Vec<i64> = Vec::new();
    let mut vals_v: Vec<f64> = Vec::new();
    for (&i, &v) in &diagonal {
        if v != 0.0 {
            rows_v.push(i as i64);
            cols_v.push(i as i64);
            vals_v.push(v);
        }
    }
    // Both cells get the symmetric max, not their own directional value.
    let _ = &cell;
    for &(a, b) in &kept {
        let value = sym.get(&(a, b)).copied().unwrap_or(0.0);
        if value != 0.0 {
            rows_v.push(a as i64);
            cols_v.push(b as i64);
            vals_v.push(value);
            rows_v.push(b as i64);
            cols_v.push(a as i64);
            vals_v.push(value);
        }
    }

    let storage = build_sparse_max(rows_v, cols_v, vals_v, n, n);
    DataMatrix {
        rows: m.rows.clone(),
        cols: m.cols.clone(),
        row_lengths: m.row_lengths.clone(),
        col_lengths: m.col_lengths.clone(),
        data_type: m.data_type.clone(),
        storage,
    }
}
/// `--mst`/`--knn` as the user typed them, e.g. `--mst --knn 5`, for error messages
/// (upstream `sparsify_option_label`).
pub fn sparsify_option_label(mst: bool, knn: Option<usize>) -> String {
    let mut parts = Vec::new();
    if mst {
        parts.push("--mst".to_string());
    }
    if let Some(k) = knn {
        parts.push(format!("--knn {k}"));
    }
    parts.join(" ")
}

/// Refuse `--mst`/`--knn` on distance data (upstream `check_sparsify_data_type`): both
/// keep the *highest* values, which in a distance matrix are the weakest
/// relationships.
pub fn check_sparsify_data_type(
    data_type: &str,
    mst: bool,
    knn: Option<usize>,
) -> Result<(), String> {
    if !mst && knn.is_none() {
        return Ok(());
    }
    match data_type.strip_suffix("_dist") {
        Some(score_type) => Err(format!(
            "{} keeps the highest-valued edges, so it cannot be applied to a '{data_type}' \
             (distance) matrix. Sparsify the '{score_type}' matrix instead.",
            sparsify_option_label(mst, knn)
        )),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A triangle of equal scores has three spanning trees; the tie rule picks one.
    /// Upstream's batch `MaxTree` (row-major index descending) and its streaming
    /// accumulator settle on different ones, and each tool must match its own.
    #[test]
    fn tied_edges_follow_the_tools_tree() {
        let names: Vec<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
        #[rustfmt::skip]
        let m = DataMatrix {
            rows: names.clone(),
            cols: names,
            row_lengths: None,
            col_lengths: None,
            data_type: "score".into(),
            storage: Storage::Dense(vec![
                0.0, 5.0, 5.0,
                5.0, 0.0, 5.0,
                5.0, 5.0, 0.0,
            ]),
        };
        let (batch, _) = mst_knn_kept(&m, 0, 0.0, true, TreeTies::Batch);
        assert_eq!(batch, [(1, 2), (0, 2)]);
        let (streaming, _) = mst_knn_kept(&m, 0, 0.0, true, TreeTies::Streaming);
        assert_eq!(streaming, [(0, 1), (0, 2)]);
    }
}
