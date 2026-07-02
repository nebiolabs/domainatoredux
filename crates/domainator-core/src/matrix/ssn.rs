//! SSN graph machinery: the maximum spanning tree (`MaxTree`), the MST-component
//! hierarchy, and merge-event analysis. Mirrors `domainator.ssn_edges::MaxTree`
//! and `domainator.ssn_hierarchy`.
//!
//! These are the pure algorithms behind `build-ssn`, `build-ssn-viewer`, and
//! `matrix-report`: the viewer ships only the MST (n-1 edges) plus this hierarchy,
//! and reconstructs clusters client-side by cutting the tree at a threshold.
//!
//! Edge ordering matches the Python `DenseDataMatrix.sorted_undirected_edges`
//! fast path (`agg=max`, `skip_zeros=True`): lower-triangular undirected edges
//! with score `max(M[i,j], M[j,i])`, sorted by **score descending**, ties broken
//! by **row-major index descending** (i.e. larger source index first, then larger
//! target index) — equivalent to a stable ascending sort followed by a reverse.

use std::collections::{BinaryHeap, HashMap};

use super::{DataMatrix, Storage};

pub const MERGE_IMPACT_MIN_CHILD: &str = "min_child";
pub const MERGE_IMPACT_PRODUCT: &str = "product";
pub const MERGE_IMPACT_CHOICES: [&str; 2] = [MERGE_IMPACT_PRODUCT, MERGE_IMPACT_MIN_CHILD];
pub const DEFAULT_MAX_MERGE_EVENTS: usize = 500;

/// `"∞"` for a non-finite threshold, else Python's `f"{x:.2f}"`.
pub fn format_threshold_value(threshold: f64) -> String {
    if threshold.is_infinite() {
        "∞".to_string()
    } else {
        format!("{:.2}", threshold)
    }
}

/// One edge of the maximum spanning tree: `(i, j)` with `i > j` (source is the
/// larger node index, matching Python), the connecting `weight`, and `gap_count`
/// = number of candidate edges consumed since the previous MST edge (used for
/// `cluster_count_by_edge_count`).
#[derive(Clone, Copy, Debug)]
pub struct MstEdge {
    pub i: usize,
    pub j: usize,
    pub weight: f64,
    pub gap_count: u64,
}

/// Maximum spanning tree / forest built via Kruskal over the symmetric edges of a
/// [`DataMatrix`]. MST edges are stored in descending-weight order.
pub struct MaxTree {
    pub n_nodes: usize,
    pub edges: Vec<MstEdge>,
}

/// Visit every stored non-diagonal, non-zero entry `(row, col, value)`.
fn for_each_offdiag(m: &DataMatrix, mut f: impl FnMut(usize, usize, f64)) {
    let nc = m.ncols();
    match &m.storage {
        Storage::Dense(d) => {
            for r in 0..m.nrows() {
                for c in 0..nc {
                    if r != c {
                        let v = d[r * nc + c];
                        if v != 0.0 {
                            f(r, c, v);
                        }
                    }
                }
            }
        }
        Storage::Sparse { values, indices, indptr } => {
            for r in 0..m.nrows() {
                let (s, e) = (indptr[r] as usize, indptr[r + 1] as usize);
                for k in s..e {
                    let c = indices[k] as usize;
                    if r != c && values[k] != 0.0 {
                        f(r, c, values[k]);
                    }
                }
            }
        }
    }
}

impl MaxTree {
    /// Build the maximum spanning forest from a square symmetric-scored matrix.
    /// `skip_zeros` is always true here (zero-weight edges are dropped), matching
    /// the default the SSN tools use.
    pub fn from_matrix(m: &DataMatrix) -> MaxTree {
        let n = m.nrows();

        // Symmetric candidate score per canonical lower-triangular pair (hi > lo):
        // max over both stored directions.
        let mut sym: HashMap<(usize, usize), f64> = HashMap::new();
        for_each_offdiag(m, |r, c, v| {
            let key = if r > c { (r, c) } else { (c, r) };
            let e = sym.entry(key).or_insert(f64::NEG_INFINITY);
            if v > *e {
                *e = v;
            }
        });

        // (score, hi, lo). Drop zero-weight edges (skip_zeros).
        let mut edges: Vec<(f64, usize, usize)> =
            sym.into_iter().filter(|&(_, s)| s != 0.0).map(|((hi, lo), s)| (s, hi, lo)).collect();
        // score descending; ties by hi descending, then lo descending — the dense
        // fast-path ordering (stable ascending sort + reverse).
        edges.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.cmp(&a.1)).then(b.2.cmp(&a.2)));

        let mut uf = super::mst_knn::UnionFind::new(n);
        let mut mst_edges: Vec<MstEdge> = Vec::new();
        let mut gap: u64 = 0;
        for &(score, hi, lo) in &edges {
            gap += 1;
            if uf.union(hi, lo) {
                mst_edges.push(MstEdge { i: hi, j: lo, weight: score, gap_count: gap });
                gap = 0;
            }
        }

        MaxTree { n_nodes: n, edges: mst_edges }
    }

    /// MST edges as `(source, target, weight)`, descending weight.
    pub fn mst_edges(&self) -> impl Iterator<Item = (usize, usize, f64)> + '_ {
        self.edges.iter().map(|e| (e.i, e.j, e.weight))
    }

    /// `(cumulative_edge_count, threshold)` per MST edge.
    pub fn edges_by_threshold(&self) -> Vec<(f64, f64)> {
        let mut out = Vec::with_capacity(self.edges.len());
        let mut count: u64 = 0;
        for e in &self.edges {
            count += e.gap_count;
            out.push((count as f64, e.weight));
        }
        out
    }

    /// `(threshold, cluster_count)` including the initial `(inf, n_nodes)` row.
    /// At each MST edge threshold, the count is `n_nodes - edges_included`.
    pub fn cluster_count_by_threshold(&self) -> Vec<(f64, f64)> {
        let mut out = Vec::with_capacity(self.edges.len() + 1);
        out.push((f64::INFINITY, self.n_nodes as f64));
        for (i, e) in self.edges.iter().enumerate() {
            out.push((e.weight, (self.n_nodes - (i + 1)) as f64));
        }
        out
    }

    /// `(cumulative_edge_count, cluster_count)` including non-MST edges between
    /// MST edges; starts at `(0, n_nodes)`.
    pub fn cluster_count_by_edge_count(&self) -> Vec<(f64, f64)> {
        let ebt = self.edges_by_threshold();
        let mut out = Vec::with_capacity(ebt.len() + 1);
        out.push((0.0, self.n_nodes as f64));
        for (i, &(edge_count, _)) in ebt.iter().enumerate() {
            out.push((edge_count, (self.n_nodes - (i + 1)) as f64));
        }
        out
    }

    /// `{ n_nodes, mst_edges: [[i, j, weight], ...] }` for the interactive viewer.
    pub fn export_mst_edges(&self) -> Vec<(usize, usize, f64)> {
        self.mst_edges().collect()
    }
}

/// All non-zero undirected edge scores `max(M[i,j], M[j,i])`, sorted descending —
/// the `edge_table.score` array used by `matrix_report` for its summary stats and
/// histogram. Matches the dense fast-path ordering (score desc, then row-major
/// index desc).
pub fn edge_scores_desc(m: &DataMatrix) -> Vec<f64> {
    let mut sym: HashMap<(usize, usize), f64> = HashMap::new();
    for_each_offdiag(m, |r, c, v| {
        let key = if r > c { (r, c) } else { (c, r) };
        let e = sym.entry(key).or_insert(f64::NEG_INFINITY);
        if v > *e {
            *e = v;
        }
    });
    let mut edges: Vec<(f64, usize, usize)> =
        sym.into_iter().filter(|&(_, s)| s != 0.0).map(|((hi, lo), s)| (s, hi, lo)).collect();
    edges.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.cmp(&a.1)).then(b.2.cmp(&a.2)));
    edges.into_iter().map(|(s, _, _)| s).collect()
}

/// MST ∪ OR-symmetric kNN edges as canonical `(a, b, symmetric_score)` with
/// `a < b`. Mirrors `ssn_edges.mst_knn_edge_index_dict` — the same selection the
/// `--mst_knn` matrix pruner uses.
pub fn mst_knn_edge_list(m: &DataMatrix, k: usize, lower_bound: f64) -> Vec<(usize, usize, f64)> {
    let (kept, sym) = super::mst_knn::mst_knn_kept(m, k, lower_bound);
    kept.into_iter().map(|(a, b)| (a, b, sym[&(a, b)])).collect()
}

// --- Component-size summary / merge events (ssn_hierarchy) ---

const COMPONENT_THRESHOLD_COL: usize = 0;
const COMPONENT_LARGEST_COL: usize = 1;
const COMPONENT_AVG_NON_SINGLETON_COL: usize = 2;
const COMPONENT_MERGE_IMPACT_COL: usize = 3;
const COMPONENT_DELTA_LARGEST_COL: usize = 4;
const COMPONENT_DELTA_AVG_NON_SINGLETON_COL: usize = 5;

fn uf_find(parent: &mut [usize], mut x: usize) -> usize {
    let mut root = x;
    while parent[root] != root {
        root = parent[root];
    }
    while parent[x] != x {
        let next = parent[x];
        parent[x] = root;
        x = next;
    }
    root
}

/// Rows of `[threshold, largest, avg_non_singleton, merge_impact, delta_largest,
/// delta_avg_non_singleton]`, one per MST edge plus an initial `inf` row.
/// Mirrors `ssn_hierarchy.component_size_summary_by_threshold`.
pub fn component_size_summary_by_threshold(tree: &MaxTree, merge_impact_metric: &str) -> Vec<[f64; 6]> {
    if tree.n_nodes == 0 {
        return Vec::new();
    }
    let n = tree.n_nodes;
    let mut parent: Vec<usize> = (0..n).collect();
    let mut component_sizes: Vec<i64> = vec![1; n];

    // Lazy-deletion max-heap of component sizes.
    let mut heap: BinaryHeap<i64> = (0..n).map(|_| 1i64).collect();
    let mut heap_deleted: HashMap<i64, i64> = HashMap::new();
    let heap_remove = |deleted: &mut HashMap<i64, i64>, size: i64| {
        *deleted.entry(size).or_insert(0) += 1;
    };
    fn heap_max(heap: &mut BinaryHeap<i64>, deleted: &mut HashMap<i64, i64>) -> f64 {
        while let Some(&s) = heap.peek() {
            let d = deleted.entry(s).or_insert(0);
            if *d > 0 {
                *d -= 1;
                heap.pop();
            } else {
                return s as f64;
            }
        }
        0.0
    }

    let mut non_singleton_count: i64 = 0;
    let mut non_singleton_sum: i64 = 0;
    let mut summary: Vec<[f64; 6]> = vec![[0.0; 6]; tree.edges.len() + 1];

    let record = |summary: &mut Vec<[f64; 6]>,
                      heap: &mut BinaryHeap<i64>,
                      heap_deleted: &mut HashMap<i64, i64>,
                      non_singleton_count: i64,
                      non_singleton_sum: i64,
                      row_idx: usize,
                      threshold: f64,
                      merge_impact: f64| {
        let largest_cluster = heap_max(heap, heap_deleted);
        let avg_non_singleton = if non_singleton_count > 0 {
            non_singleton_sum as f64 / non_singleton_count as f64
        } else {
            0.0
        };
        summary[row_idx][COMPONENT_THRESHOLD_COL] = threshold;
        summary[row_idx][COMPONENT_LARGEST_COL] = largest_cluster;
        summary[row_idx][COMPONENT_AVG_NON_SINGLETON_COL] = avg_non_singleton;
        summary[row_idx][COMPONENT_MERGE_IMPACT_COL] = merge_impact;
        if row_idx > 0 {
            summary[row_idx][COMPONENT_DELTA_LARGEST_COL] =
                largest_cluster - summary[row_idx - 1][COMPONENT_LARGEST_COL];
            summary[row_idx][COMPONENT_DELTA_AVG_NON_SINGLETON_COL] =
                avg_non_singleton - summary[row_idx - 1][COMPONENT_AVG_NON_SINGLETON_COL];
        }
    };

    record(&mut summary, &mut heap, &mut heap_deleted, non_singleton_count, non_singleton_sum, 0, f64::INFINITY, 0.0);

    for (edge_i, e) in tree.edges.iter().enumerate() {
        let row_idx = edge_i + 1;
        let left_root = uf_find(&mut parent, e.i);
        let right_root = uf_find(&mut parent, e.j);
        let mut merge_impact = 0.0;
        if left_root != right_root {
            let left_size = component_sizes[left_root];
            let right_size = component_sizes[right_root];
            merge_impact = if merge_impact_metric == MERGE_IMPACT_PRODUCT {
                (left_size * right_size) as f64
            } else {
                left_size.min(right_size) as f64
            };

            heap_remove(&mut heap_deleted, left_size);
            heap_remove(&mut heap_deleted, right_size);

            if left_size > 1 {
                non_singleton_count -= 1;
                non_singleton_sum -= left_size;
            }
            if right_size > 1 {
                non_singleton_count -= 1;
                non_singleton_sum -= right_size;
            }

            let merged_size = left_size + right_size;
            parent[left_root] = right_root;
            component_sizes[right_root] = merged_size;

            heap.push(merged_size);
            non_singleton_count += 1;
            non_singleton_sum += merged_size;
        }

        record(
            &mut summary,
            &mut heap,
            &mut heap_deleted,
            non_singleton_count,
            non_singleton_sum,
            row_idx,
            e.weight,
            merge_impact,
        );
    }

    summary
}

/// A threshold merge event (one per distinct MST threshold value).
#[derive(Clone, Debug)]
pub struct MergeEvent {
    pub edge_index: i64,
    pub summary_row_from: usize,
    pub summary_row_to: usize,
    pub threshold_from_value: f64,
    pub threshold_from: String,
    pub threshold_to: String,
    pub threshold_value: f64,
    pub merge_impact: f64,
    pub delta_largest: f64,
    pub delta_avg_non_singleton: f64,
}

/// Collapse the per-edge summary into per-threshold merge events. Mirrors
/// `ssn_hierarchy.threshold_merge_event_rows`.
pub fn threshold_merge_event_rows(summary: &[[f64; 6]]) -> Vec<MergeEvent> {
    if summary.len() < 2 {
        return Vec::new();
    }
    let mut events = Vec::new();
    let mut previous_row = summary[0];
    let mut row_idx = 1;
    while row_idx < summary.len() {
        let first_summary_row_idx = row_idx;
        let threshold_value = summary[row_idx][COMPONENT_THRESHOLD_COL];
        let mut merge_impact = 0.0;
        let mut last_row = summary[row_idx];
        while row_idx < summary.len() && summary[row_idx][COMPONENT_THRESHOLD_COL] == threshold_value {
            merge_impact += summary[row_idx][COMPONENT_MERGE_IMPACT_COL];
            last_row = summary[row_idx];
            row_idx += 1;
        }
        let edge_index = row_idx as i64 - 2;
        events.push(MergeEvent {
            edge_index,
            summary_row_from: first_summary_row_idx,
            summary_row_to: row_idx - 1,
            threshold_from_value: previous_row[COMPONENT_THRESHOLD_COL],
            threshold_from: format_threshold_value(previous_row[COMPONENT_THRESHOLD_COL]),
            threshold_to: format_threshold_value(last_row[COMPONENT_THRESHOLD_COL]),
            threshold_value: last_row[COMPONENT_THRESHOLD_COL],
            merge_impact,
            delta_largest: (last_row[COMPONENT_LARGEST_COL] - previous_row[COMPONENT_LARGEST_COL]).abs(),
            delta_avg_non_singleton: (last_row[COMPONENT_AVG_NON_SINGLETON_COL]
                - previous_row[COMPONENT_AVG_NON_SINGLETON_COL])
                .abs(),
        });
        previous_row = last_row;
    }
    events
}

/// Keep the `max_merge_events` strongest events (ranked by
/// `(-merge_impact, -delta_largest, -delta_avg, edge_index)`), then restore
/// `edge_index` order. `max == 0` keeps all. Mirrors `filter_merge_event_rows`.
pub fn filter_merge_event_rows(events: &[MergeEvent], max_merge_events: usize) -> Vec<MergeEvent> {
    if max_merge_events == 0 || events.len() <= max_merge_events {
        return events.to_vec();
    }
    let mut ranked: Vec<MergeEvent> = events.to_vec();
    ranked.sort_by(|a, b| {
        b.merge_impact
            .total_cmp(&a.merge_impact)
            .then(b.delta_largest.total_cmp(&a.delta_largest))
            .then(b.delta_avg_non_singleton.total_cmp(&a.delta_avg_non_singleton))
            .then(a.edge_index.cmp(&b.edge_index))
    });
    ranked.truncate(max_merge_events);
    ranked.sort_by_key(|e| e.edge_index);
    ranked
}

/// One node of the MST-component dendrogram. `leaf` nodes carry `node_index`;
/// `cluster` nodes carry `left`/`right`/`threshold`.
#[derive(Clone, Debug)]
pub struct HierNode {
    pub id: usize,
    pub is_leaf: bool,
    pub size: usize,
    pub parent: Option<usize>,
    pub leaf_start: usize,
    pub leaf_count: usize,
    // leaf-only
    pub node_index: Option<usize>,
    // cluster-only
    pub left: Option<usize>,
    pub right: Option<usize>,
    pub threshold: Option<f64>,
}

/// The MST-component hierarchy (dendrogram) used by the viewer. Mirrors
/// `ssn_hierarchy.build_mst_component_hierarchy`.
pub struct Hierarchy {
    pub nodes: Vec<HierNode>,
    pub roots: Vec<usize>,
    pub leaf_order: Vec<usize>,
}

pub fn build_mst_component_hierarchy(tree: &MaxTree) -> Hierarchy {
    let n = tree.n_nodes;
    let mut parent: Vec<usize> = (0..n).collect();
    let mut nodes: Vec<HierNode> = Vec::with_capacity(2 * n.max(1));
    let mut component_id_by_root: HashMap<usize, usize> = HashMap::new();
    let mut component_size_by_id: HashMap<usize, usize> = HashMap::new();
    let mut component_min_leaf_by_id: HashMap<usize, usize> = HashMap::new();

    for node_index in 0..n {
        nodes.push(HierNode {
            id: node_index,
            is_leaf: true,
            size: 1,
            parent: None,
            leaf_start: 0,
            leaf_count: 0,
            node_index: Some(node_index),
            left: None,
            right: None,
            threshold: None,
        });
        component_id_by_root.insert(node_index, node_index);
        component_size_by_id.insert(node_index, 1);
        component_min_leaf_by_id.insert(node_index, node_index);
    }

    let mut next_component_id = n;
    for e in &tree.edges {
        let left_root = uf_find(&mut parent, e.i);
        let right_root = uf_find(&mut parent, e.j);
        if left_root == right_root {
            continue;
        }
        let mut left_component_id = component_id_by_root[&left_root];
        let mut right_component_id = component_id_by_root[&right_root];
        if component_min_leaf_by_id[&left_component_id] > component_min_leaf_by_id[&right_component_id] {
            std::mem::swap(&mut left_component_id, &mut right_component_id);
        }
        let merged_size =
            component_size_by_id[&left_component_id] + component_size_by_id[&right_component_id];
        let component_id = next_component_id;
        next_component_id += 1;

        nodes[left_component_id].parent = Some(component_id);
        nodes[right_component_id].parent = Some(component_id);
        nodes.push(HierNode {
            id: component_id,
            is_leaf: false,
            size: merged_size,
            parent: None,
            leaf_start: 0,
            leaf_count: 0,
            node_index: None,
            left: Some(left_component_id),
            right: Some(right_component_id),
            threshold: Some(e.weight),
        });
        component_size_by_id.insert(component_id, merged_size);
        let min_leaf = component_min_leaf_by_id[&left_component_id]
            .min(component_min_leaf_by_id[&right_component_id]);
        component_min_leaf_by_id.insert(component_id, min_leaf);

        parent[left_root] = right_root;
        component_id_by_root.remove(&left_root);
        component_id_by_root.insert(right_root, component_id);
    }

    // Roots: descending size, ties by ascending id.
    let mut roots: Vec<usize> = component_id_by_root.values().copied().collect();
    roots.sort_by(|&a, &b| nodes[b].size.cmp(&nodes[a].size).then(a.cmp(&b)));

    // Leaf order via iterative stack DFS (left before right).
    let mut leaf_order: Vec<usize> = Vec::with_capacity(n);
    let mut stack: Vec<(usize, bool)> = Vec::new();
    for &root_id in roots.iter().rev() {
        stack.push((root_id, false));
    }
    while let Some((component_id, visited)) = stack.pop() {
        if nodes[component_id].is_leaf {
            nodes[component_id].leaf_start = leaf_order.len();
            nodes[component_id].leaf_count = 1;
            leaf_order.push(nodes[component_id].node_index.unwrap());
            continue;
        }
        if visited {
            let (left, right) = (nodes[component_id].left.unwrap(), nodes[component_id].right.unwrap());
            nodes[component_id].leaf_start = nodes[left].leaf_start;
            nodes[component_id].leaf_count = nodes[left].leaf_count + nodes[right].leaf_count;
            continue;
        }
        let (left, right) = (nodes[component_id].left.unwrap(), nodes[component_id].right.unwrap());
        stack.push((component_id, true));
        stack.push((right, false));
        stack.push((left, false));
    }

    Hierarchy { nodes, roots, leaf_order }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sym_matrix(n: usize, entries: &[(usize, usize, f64)]) -> DataMatrix {
        let mut d = vec![0.0; n * n];
        for &(r, c, v) in entries {
            d[r * n + c] = v;
            d[c * n + r] = v;
        }
        let labels: Vec<String> = (0..n).map(|i| format!("n{i}")).collect();
        DataMatrix {
            rows: labels.clone(),
            cols: labels,
            row_lengths: None,
            col_lengths: None,
            data_type: "score".into(),
            storage: Storage::Dense(d),
        }
    }

    #[test]
    fn maxtree_picks_highest_edges_and_forms_forest() {
        // 4 nodes: a triangle {0,1,2} with weights 9,8,7 and node 3 isolated.
        // MST of the triangle keeps the two largest edges (9, 8).
        let m = sym_matrix(4, &[(0, 1, 9.0), (1, 2, 8.0), (0, 2, 7.0)]);
        let tree = MaxTree::from_matrix(&m);
        assert_eq!(tree.n_nodes, 4);
        let edges: Vec<(usize, usize, f64)> = tree.mst_edges().collect();
        assert_eq!(edges.len(), 2); // forest: node 3 disconnected
        assert_eq!(edges[0].2, 9.0);
        assert_eq!(edges[1].2, 8.0);
        // gap_count: edge 9 is first candidate (gap 1); edge 8 is second (gap 1);
        // edge 7 would form a cycle so is never emitted.
        assert_eq!(tree.edges[0].gap_count, 1);
        assert_eq!(tree.edges[1].gap_count, 1);
    }

    #[test]
    fn cluster_count_by_threshold_decreases_by_one_per_edge() {
        let m = sym_matrix(4, &[(0, 1, 9.0), (1, 2, 8.0), (2, 3, 5.0)]);
        let tree = MaxTree::from_matrix(&m);
        let ccbt = tree.cluster_count_by_threshold();
        assert_eq!(ccbt[0].0, f64::INFINITY);
        assert_eq!(ccbt[0].1, 4.0);
        assert_eq!(ccbt[1], (9.0, 3.0));
        assert_eq!(ccbt[2], (8.0, 2.0));
        assert_eq!(ccbt[3], (5.0, 1.0));
    }

    #[test]
    fn summary_and_merge_events_track_growth() {
        // Two pairs merge, then join: 0-1 (10), 2-3 (9), then 1-2 (5).
        let m = sym_matrix(4, &[(0, 1, 10.0), (2, 3, 9.0), (1, 2, 5.0)]);
        let tree = MaxTree::from_matrix(&m);
        let summary = component_size_summary_by_threshold(&tree, MERGE_IMPACT_MIN_CHILD);
        assert_eq!(summary.len(), 4); // inf + 3 edges
        // Start: largest = 1.
        assert_eq!(summary[0][COMPONENT_LARGEST_COL], 1.0);
        // After first merge (10): largest = 2, min_child impact = 1.
        assert_eq!(summary[1][COMPONENT_LARGEST_COL], 2.0);
        assert_eq!(summary[1][COMPONENT_MERGE_IMPACT_COL], 1.0);
        // After the final merge (5): everything in one cluster of 4, impact = 2.
        assert_eq!(summary[3][COMPONENT_LARGEST_COL], 4.0);
        assert_eq!(summary[3][COMPONENT_MERGE_IMPACT_COL], 2.0);

        let events = threshold_merge_event_rows(&summary);
        assert_eq!(events.len(), 3);
        assert_eq!(events[2].threshold_value, 5.0);
        assert_eq!(events[2].merge_impact, 2.0);
    }

    #[test]
    fn hierarchy_covers_all_leaves() {
        let m = sym_matrix(4, &[(0, 1, 10.0), (2, 3, 9.0), (1, 2, 5.0)]);
        let tree = MaxTree::from_matrix(&m);
        let h = build_mst_component_hierarchy(&tree);
        // 4 leaves + 3 internal cluster nodes = 7.
        assert_eq!(h.nodes.len(), 7);
        assert_eq!(h.roots.len(), 1); // fully connected
        assert_eq!(h.leaf_order.len(), 4);
        let mut sorted = h.leaf_order.clone();
        sorted.sort();
        assert_eq!(sorted, vec![0, 1, 2, 3]);
        // Root spans all leaves.
        let root = h.roots[0];
        assert_eq!(h.nodes[root].leaf_count, 4);
        assert_eq!(h.nodes[root].size, 4);
    }

    #[test]
    fn empty_and_singleton() {
        let m = sym_matrix(1, &[]);
        let tree = MaxTree::from_matrix(&m);
        assert_eq!(tree.edges.len(), 0);
        assert_eq!(tree.cluster_count_by_threshold(), vec![(f64::INFINITY, 1.0)]);
        let h = build_mst_component_hierarchy(&tree);
        assert_eq!(h.roots, vec![0]);
        assert_eq!(h.leaf_order, vec![0]);
    }
}
