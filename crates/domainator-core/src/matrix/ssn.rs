//! SSN graph machinery: the maximum spanning tree (`MaxTree`), the MST-component
//! hierarchy, and merge-event analysis. Mirrors `domainator.ssn_edges::MaxTree`
//! and `domainator.ssn_hierarchy`.
//!
//! These are the pure algorithms behind `build_ssn`, `build_ssn_viewer`, and
//! `matrix_report`: the viewer ships only the MST (n-1 edges) plus this hierarchy,
//! and reconstructs clusters client-side by cutting the tree at a threshold.
//!
//! Edge ordering matches the Python `DenseDataMatrix.sorted_undirected_edges`
//! fast path (`agg=max`, `skip_zeros=True`): lower-triangular undirected edges
//! with score `max(M[i,j], M[j,i])`, sorted by **score descending**, ties broken
//! by **row-major index descending** (i.e. larger source index first, then larger
//! target index) — equivalent to a stable ascending sort followed by a reverse.

use std::collections::HashMap;

use super::{csr_index, csr_row, DataMatrix, Storage};

pub const MERGE_IMPACT_MIN_CHILD: &str = "min_child";
pub const MERGE_IMPACT_PRODUCT: &str = "product";
pub const MERGE_IMPACT_CHOICES: [&str; 2] = [MERGE_IMPACT_PRODUCT, MERGE_IMPACT_MIN_CHILD];
pub const DEFAULT_MAX_MERGE_EVENTS: usize = 500;
/// The cap a chart with a viewport uses instead.
///
/// The viewer picks its events from the chart's current window rather than from the
/// whole axis, so a much smaller number shows *more*: zoom in and the cap re-spends
/// itself on the stretch being looked at. A chart with no viewport (`matrix_report`,
/// `ssn_navigator`) has only one shot at the whole range and keeps the larger default
/// above.
pub const DEFAULT_WINDOWED_MAX_MERGE_EVENTS: usize = 50;
/// Equal bands the threshold axis is cut into for the merge-event density back-fill.
pub const MERGE_EVENT_DENSITY_BINS: usize = 20;
/// Moving-sum window, as a fraction of the threshold range.
pub const MOVING_SUM_WINDOW_FRACTION: f64 = 0.05;
/// Points on the moving-sum grid.
pub const MOVING_SUM_GRID_POINTS: usize = 800;
/// How far below the weakest MST edge the floor slider stop sits, as a fraction of the
/// MST weight range.
pub const FLOOR_THRESHOLD_SPAN_FRACTION: f64 = 0.01;

/// `"∞"` for a non-finite threshold, else Python's `f"{x:.2f}"`.
pub fn format_threshold_value(threshold: f64) -> String {
    if threshold.is_infinite() {
        "∞".to_string()
    } else {
        format!("{:.2}", threshold)
    }
}

/// One edge of the maximum spanning tree: `(i, j)` with `i > j` (source is the
/// larger node index, matching Python), the connecting `weight`, and — captured at
/// the edge's **tie-group boundary** — how many graph edges and MST edges score
/// *strictly higher* than it.
///
/// Because the Kruskal scan is descending, "scores strictly higher" is exactly the
/// size of the prefix preceding the edge's tie group, which is what `--lb <weight>`
/// keeps. Mirrors `ssn_edges.MaxTree`'s `_edges_above_by_mst_edge` /
/// `_mst_edges_above_by_mst_edge`.
#[derive(Clone, Copy, Debug)]
pub struct MstEdge {
    pub i: usize,
    pub j: usize,
    pub weight: f64,
    pub edges_above: u64,
    pub mst_edges_above: u64,
}

/// Maximum spanning tree / forest built via Kruskal over the symmetric edges of a
/// [`DataMatrix`]. MST edges are stored in descending-weight order.
///
/// The threshold tables carry **one row per distinct threshold**, and every row
/// describes exactly the graph `build_ssn --lb <threshold>` keeps: the edges scoring
/// *strictly above* it. Keying rows by threshold rather than by MST edge position is
/// what makes a threshold have a single answer when several MST edges share a weight
/// (see `docs/file_formats.md` and upstream commit `9ab625f`).
pub struct MaxTree {
    pub n_nodes: usize,
    pub edges: Vec<MstEdge>,
    /// Total non-zero undirected edges scanned (not just MST edges).
    n_edges: usize,
    /// Weakest non-zero undirected score, or `0.0` when there are none.
    min_score: f64,
    thresholds: Vec<f64>,
    edges_above_threshold: Vec<u64>,
    mst_edges_above_threshold: Vec<u64>,
    /// `threshold.to_bits()` -> row index. Exact float equality, like Python's dict.
    threshold_rows: HashMap<u64, usize>,
}

/// Canonicalize a threshold for use as an exact-equality map key.
///
/// `-0.0` and `0.0` are distinct bit patterns but compare equal in Python's dict, so
/// they must collapse to one key here or the synthetic `threshold = 0` row would be
/// unreachable from a `-0.0` lookup.
fn threshold_key(threshold: f64) -> u64 {
    (if threshold == 0.0 { 0.0 } else { threshold }).to_bits()
}

/// Visit every **stored** non-diagonal entry `(row, col, value)`, zeros included.
///
/// Zeros are deliberately not filtered here: the symmetric max has to see them (see
/// [`symmetric_edges_desc`]). Only the sparse path uses this — dense iterates directly.
fn for_each_offdiag(m: &DataMatrix, mut f: impl FnMut(usize, usize, f64)) {
    match &m.storage {
        Storage::Dense(d) => {
            let nc = m.ncols();
            for r in 0..m.nrows() {
                for c in 0..nc {
                    if r != c {
                        f(r, c, d[r * nc + c]);
                    }
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
                    let c = csr_index(indices[k]);
                    if r != c {
                        f(r, c, values[k]);
                    }
                }
            }
        }
    }
}

/// Canonical lower-triangular undirected edges as `(score, hi, lo)` with `hi > lo`,
/// sorted **score descending, ties by hi descending then lo descending** — the dense
/// fast-path ordering (a stable ascending sort followed by a reverse).
///
/// The symmetric score is `max(M[i,j], M[j,i])` where an entry that is *absent*
/// (sparse) counts as `0.0`, and only then are zero-scoring edges dropped. Taking the
/// max first is load-bearing: for the pair `(0.0, -3.0)` Python yields `0.0` and drops
/// it, whereas skipping the zero slot up front would keep `-3.0`. That difference
/// decides `min_score`, which decides whether the synthetic `threshold = 0` row exists
/// — so it shifts the row count of every threshold table.
fn symmetric_edges_desc(m: &DataMatrix) -> Vec<(f64, usize, usize)> {
    let n = m.nrows();
    let nc = m.ncols();
    let mut edges: Vec<(f64, usize, usize)> = Vec::new();

    match &m.storage {
        Storage::Dense(d) => {
            // Every slot is stored, so the pair max needs no implicit zero.
            for hi in 1..n {
                for lo in 0..hi {
                    let a = d[hi * nc + lo];
                    let b = d[lo * nc + hi];
                    let score = if a > b { a } else { b };
                    if score != 0.0 {
                        edges.push((score, hi, lo));
                    }
                }
            }
        }
        Storage::Sparse { .. } => {
            // Fold in an implicit 0.0 for any pair stored on only one side, matching
            // scipy's `data.maximum(data.T)`.
            let mut sym: HashMap<(usize, usize), (f64, u8)> = HashMap::new();
            for_each_offdiag(m, |r, c, v| {
                let key = if r > c { (r, c) } else { (c, r) };
                let e = sym.entry(key).or_insert((f64::NEG_INFINITY, 0));
                if v > e.0 {
                    e.0 = v;
                }
                e.1 += 1;
            });
            for ((hi, lo), (best, seen)) in sym {
                let score = if seen < 2 && 0.0 > best { 0.0 } else { best };
                if score != 0.0 {
                    edges.push((score, hi, lo));
                }
            }
        }
    }

    edges.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.cmp(&a.1)).then(b.2.cmp(&a.2)));
    edges
}

impl MaxTree {
    /// Build the maximum spanning forest from a square symmetric-scored matrix.
    /// Zero-weight edges are dropped (`skip_zeros`), matching the SSN tools' default.
    pub fn from_matrix(m: &DataMatrix) -> MaxTree {
        let n = m.nrows();
        let edges = symmetric_edges_desc(m);

        let mut uf = super::mst_knn::UnionFind::new(n);
        let mut mst_edges: Vec<MstEdge> = Vec::new();

        // Tie-group prefix scan. `group_score` must start equal to nothing, so the
        // first edge always opens a group: NaN compares unequal to everything,
        // including itself, which is exactly Python's `group_score = None`.
        let mut group_score = f64::NAN;
        let mut group_edges_above: u64 = 0;
        let mut group_mst_above: u64 = 0;
        for (edge_i, &(score, hi, lo)) in edges.iter().enumerate() {
            if score != group_score {
                group_score = score;
                group_edges_above = edge_i as u64;
                group_mst_above = mst_edges.len() as u64;
            }
            if uf.union(hi, lo) {
                mst_edges.push(MstEdge {
                    i: hi,
                    j: lo,
                    weight: score,
                    edges_above: group_edges_above,
                    mst_edges_above: group_mst_above,
                });
            }
        }

        let mut tree = MaxTree {
            n_nodes: n,
            edges: mst_edges,
            n_edges: edges.len(),
            min_score: edges.last().map(|e| e.0).unwrap_or(0.0),
            thresholds: Vec::new(),
            edges_above_threshold: Vec::new(),
            mst_edges_above_threshold: Vec::new(),
            threshold_rows: HashMap::new(),
        };
        tree.build_threshold_tables();
        tree
    }

    /// Rebuild a tree from a stored merge order alone — what a `.dsnv` bundle carries.
    ///
    /// The analogue of upstream's `ssn_bundle._BundleTree`: a bundle stores `nodes` and
    /// `mst_edges` and nothing derived, so the split-event series, its moving sum and
    /// the slider's stops are recomputed from those on demand. Everything that
    /// derivation touches — [`component_size_summary_by_threshold`],
    /// [`build_mst_component_hierarchy`] and the floor stop in
    /// [`threshold_slider_stops`] — reads only `n_nodes` and `edges`.
    ///
    /// What it cannot reconstruct is the threshold tables, which describe the **full
    /// input graph** that no bundle ever carried. They are left empty, so
    /// [`MaxTree::threshold_row_index`] answers `-1` for every threshold and
    /// [`threshold_slider_stops`] must be called with `with_threshold_index: false`.
    /// `edges_above` / `mst_edges_above` are zeroed for the same reason and are **not**
    /// meaningful on such a tree.
    ///
    /// `mst_edges` must arrive weight-descending, as the bundle writes them: the merge
    /// replay is a Kruskal scan and reordering it would change the hierarchy.
    pub fn from_mst_edges(n_nodes: usize, mst_edges: &[(usize, usize, f64)]) -> MaxTree {
        MaxTree {
            n_nodes,
            edges: mst_edges
                .iter()
                .map(|&(i, j, weight)| MstEdge {
                    i,
                    j,
                    weight,
                    edges_above: 0,
                    mst_edges_above: 0,
                })
                .collect(),
            n_edges: 0,
            min_score: 0.0,
            thresholds: Vec::new(),
            edges_above_threshold: Vec::new(),
            mst_edges_above_threshold: Vec::new(),
            threshold_rows: HashMap::new(),
        }
    }

    /// Collapse the per-MST-edge counters into one row per distinct threshold, then
    /// append the synthetic `--lb 0` complete-graph row when every score is positive.
    ///
    /// That last row matters: a cut at the lowest MST weight still drops that weight's
    /// own tie group under the strictly-above rule, so without it the complete graph
    /// would be unreachable. Mirrors `ssn_edges.MaxTree._build_threshold_tables`.
    fn build_threshold_tables(&mut self) {
        if !self.edges.is_empty() {
            for (idx, e) in self.edges.iter().enumerate() {
                if idx == 0 || e.weight != self.edges[idx - 1].weight {
                    self.thresholds.push(e.weight);
                    self.edges_above_threshold.push(e.edges_above);
                    self.mst_edges_above_threshold.push(e.mst_edges_above);
                }
            }
            if self.min_score > 0.0 {
                self.thresholds.push(0.0);
                self.edges_above_threshold.push(self.n_edges as u64);
                self.mst_edges_above_threshold.push(self.edges.len() as u64);
            }
        }
        self.threshold_rows = self
            .thresholds
            .iter()
            .enumerate()
            .map(|(idx, &t)| (threshold_key(t), idx))
            .collect();
    }

    /// MST edges as `(source, target, weight)`, descending weight.
    pub fn mst_edges(&self) -> impl Iterator<Item = (usize, usize, f64)> + '_ {
        self.edges.iter().map(|e| (e.i, e.j, e.weight))
    }

    /// Distinct MST edge weights, descending, plus a trailing `0.0` when every score
    /// is positive. One entry per row of the threshold tables.
    pub fn thresholds(&self) -> &[f64] {
        &self.thresholds
    }

    /// Per threshold row, how many graph edges score strictly above it.
    pub fn edges_above_threshold(&self) -> &[u64] {
        &self.edges_above_threshold
    }

    /// Per threshold row, how many MST edges score strictly above it.
    pub fn mst_edges_above_threshold(&self) -> &[u64] {
        &self.mst_edges_above_threshold
    }

    /// Total non-zero undirected edges in the graph (not just MST edges).
    pub fn n_edges(&self) -> usize {
        self.n_edges
    }

    /// Row index of `threshold` in the threshold tables, or `-1` when absent.
    pub fn threshold_row_index(&self, threshold: f64) -> i64 {
        match self.threshold_rows.get(&threshold_key(threshold)) {
            Some(&idx) => idx as i64,
            None => -1,
        }
    }

    /// `(edges_above_threshold, threshold)`, one row per distinct threshold.
    pub fn edges_by_threshold(&self) -> Vec<(f64, f64)> {
        self.thresholds
            .iter()
            .zip(&self.edges_above_threshold)
            .map(|(&t, &e)| (e as f64, t))
            .collect()
    }

    /// `(threshold, cluster_count)`, one row per distinct threshold. The count is
    /// `n_nodes - mst_edges_above_threshold` — the components left once every MST edge
    /// scoring strictly above the cut has been contracted. There is no leading `∞` row.
    pub fn cluster_count_by_threshold(&self) -> Vec<(f64, f64)> {
        self.thresholds
            .iter()
            .zip(&self.mst_edges_above_threshold)
            .map(|(&t, &m)| (t, (self.n_nodes as u64 - m) as f64))
            .collect()
    }

    /// `(edges_above_threshold, cluster_count)`, one row per distinct threshold.
    ///
    /// Row 0 already *is* `(0, n_nodes)` — nothing scores above the maximum weight —
    /// so no extra leading row is prepended.
    pub fn cluster_count_by_edge_count(&self) -> Vec<(f64, f64)> {
        self.edges_above_threshold
            .iter()
            .zip(&self.mst_edges_above_threshold)
            .map(|(&e, &m)| (e as f64, (self.n_nodes as u64 - m) as f64))
            .collect()
    }

    /// `{ n_nodes, mst_edges: [[i, j, weight], ...] }` for the interactive viewer.
    pub fn export_mst_edges(&self) -> Vec<(usize, usize, f64)> {
        self.mst_edges().collect()
    }
}

/// All non-zero undirected edge scores `max(M[i,j], M[j,i])`, sorted descending —
/// the `edge_table.score` array used by `matrix_report` for its summary stats and
/// histogram.
pub fn edge_scores_desc(m: &DataMatrix) -> Vec<f64> {
    symmetric_edges_desc(m)
        .into_iter()
        .map(|(s, _, _)| s)
        .collect()
}

/// MST ∪ OR-symmetric kNN edges as canonical `(a, b, symmetric_score)` with
/// `a < b`. Mirrors `ssn_edges.mst_knn_edge_index_dict` — the same selection the
/// `--mst_knn` matrix pruner uses.
pub fn mst_knn_edge_list(
    m: &DataMatrix,
    k: usize,
    lower_bound: f64,
    include_mst: bool,
) -> Vec<(usize, usize, f64)> {
    let (kept, sym) = super::mst_knn::mst_knn_kept(m, k, lower_bound, include_mst);
    kept.into_iter()
        .map(|(a, b)| (a, b, sym[&(a, b)]))
        .collect()
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
///
/// The largest component is tracked with a **running maximum**, not a heap. Components
/// only ever grow under union-find — a merge replaces two components with one strictly
/// larger than either — so the largest size is monotonically non-decreasing and nothing
/// ever has to be removed. (Upstream carried a lazy-deletion max-heap here until
/// `34c4b88`, which is the same answer at more cost.)
pub fn component_size_summary_by_threshold(
    tree: &MaxTree,
    merge_impact_metric: &str,
) -> Vec<[f64; 6]> {
    if tree.n_nodes == 0 {
        return Vec::new();
    }
    let n = tree.n_nodes;
    let mut parent: Vec<usize> = (0..n).collect();
    let mut component_sizes: Vec<i64> = vec![1; n];

    let mut largest_cluster: i64 = 1;
    let mut non_singleton_count: i64 = 0;
    let mut non_singleton_sum: i64 = 0;
    let mut summary: Vec<[f64; 6]> = vec![[0.0; 6]; tree.edges.len() + 1];

    fn record(
        summary: &mut [[f64; 6]],
        largest_cluster: i64,
        non_singleton_count: i64,
        non_singleton_sum: i64,
        row_idx: usize,
        threshold: f64,
        merge_impact: f64,
    ) {
        let largest_cluster = largest_cluster as f64;
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
    }

    record(
        &mut summary,
        largest_cluster,
        non_singleton_count,
        non_singleton_sum,
        0,
        f64::INFINITY,
        0.0,
    );

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

            largest_cluster = largest_cluster.max(merged_size);
            non_singleton_count += 1;
            non_singleton_sum += merged_size;
        }

        record(
            &mut summary,
            largest_cluster,
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
    /// The individual per-edge impacts behind `merge_impact`, as `(size, count)` in
    /// first-encounter order. `merge_impact` alone cannot tell one large cluster
    /// splitting off from a swarm of tiny ones; these three fields describe its terms.
    /// Invariant: `sum(size * count) == merge_impact`.
    pub merge_size_counts: Vec<(f64, u64)>,
    pub largest_merge: f64,
    pub merge_count: u64,
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
        let mut merge_size_counts: Vec<(f64, u64)> = Vec::new();
        let mut merge_count: u64 = 0;
        let mut largest_merge = 0.0f64;
        let mut last_row = summary[row_idx];
        while row_idx < summary.len()
            && summary[row_idx][COMPONENT_THRESHOLD_COL] == threshold_value
        {
            let row_impact = summary[row_idx][COMPONENT_MERGE_IMPACT_COL];
            merge_impact += row_impact;
            if row_impact > 0.0 {
                // A zero impact means the edge's endpoints were already in one
                // component, so it joined nothing and is not a merge.
                match merge_size_counts
                    .iter_mut()
                    .find(|(size, _)| *size == row_impact)
                {
                    Some((_, count)) => *count += 1,
                    None => merge_size_counts.push((row_impact, 1)),
                }
                merge_count += 1;
                if row_impact > largest_merge {
                    largest_merge = row_impact;
                }
            }
            last_row = summary[row_idx];
            row_idx += 1;
        }
        // The MST edge index of the last edge scoring strictly above this threshold, so
        // a stop labeled `threshold_to` reproduces the `--lb threshold_to` cut exactly
        // (that threshold's own tie group is excluded, as `--lb` excludes it). Taken
        // from the FIRST row of the group, so the first event is -1, not 0.
        let edge_index = first_summary_row_idx as i64 - 2;
        events.push(MergeEvent {
            edge_index,
            summary_row_from: first_summary_row_idx,
            summary_row_to: row_idx - 1,
            threshold_from_value: previous_row[COMPONENT_THRESHOLD_COL],
            threshold_from: format_threshold_value(previous_row[COMPONENT_THRESHOLD_COL]),
            threshold_to: format_threshold_value(last_row[COMPONENT_THRESHOLD_COL]),
            threshold_value: last_row[COMPONENT_THRESHOLD_COL],
            merge_impact,
            merge_size_counts,
            largest_merge,
            merge_count,
            delta_largest: (last_row[COMPONENT_LARGEST_COL] - previous_row[COMPONENT_LARGEST_COL])
                .abs(),
            delta_avg_non_singleton: (last_row[COMPONENT_AVG_NON_SINGLETON_COL]
                - previous_row[COMPONENT_AVG_NON_SINGLETON_COL])
                .abs(),
        });
        previous_row = last_row;
    }
    events
}

/// Rank key for merge events, strongest first:
/// `(-merge_impact, -delta_largest, -delta_avg_non_singleton, +edge_index)`.
/// Shared by the cap and the density back-fill so the two agree.
fn merge_event_rank_cmp(a: &MergeEvent, b: &MergeEvent) -> std::cmp::Ordering {
    b.merge_impact
        .total_cmp(&a.merge_impact)
        .then(b.delta_largest.total_cmp(&a.delta_largest))
        .then(
            b.delta_avg_non_singleton
                .total_cmp(&a.delta_avg_non_singleton),
        )
        .then(a.edge_index.cmp(&b.edge_index))
}

/// Which band of the plotted axis a threshold falls in, or `-1` if the axis is a
/// point. `hi` itself lands in the last band rather than one past the end.
pub fn merge_event_density_bin(threshold_value: f64, lo: f64, hi: f64, density_bins: usize) -> i64 {
    if density_bins < 1 || !lo.is_finite() || !hi.is_finite() || hi <= lo {
        return -1;
    }
    // `as i64` truncates toward zero, like Python's int().
    let position = (((threshold_value - lo) / (hi - lo)) * density_bins as f64) as i64;
    position.clamp(0, density_bins as i64 - 1)
}

/// The strongest `max_merge_events` events, plus enough to keep the axis covered.
///
/// Ranking by impact alone leaves long stretches of the threshold axis empty: on a
/// connected graph the weak tail of MST edges is individual outliers joining the giant
/// component, whose impact is the smallest there is, so the cap drops every one of them
/// — while the chart's axis and the slider still span the full range, because the
/// moving sum and the floor stop come from the *unfiltered* data.
///
/// So after the top-N pass the axis is cut into `density_bins` equal bands and the
/// strongest event in each otherwise-empty band is added back. Output is bounded at
/// `max_merge_events + density_bins`, and a back-filled row is an **addition, never a
/// replacement**. `max_merge_events == 0` means no cap *and* no back-fill.
///
/// The band edges come from the whole series, not from the top-N — taking them from the
/// kept rows would silently change the selection on exactly the networks this exists for.
///
/// `window` is an optional `(low, high)` threshold range: the part of the axis a
/// zoomable chart is currently showing. It narrows **the top-N pool only**, so zooming
/// in reveals the small events a whole-range ranking buries, while the band back-fill
/// still runs over the whole axis and every stretch of the slider keeps a stop. `None`
/// is the whole range — what a consumer with no viewport (`matrix_report`,
/// `ssn_navigator`) wants, and exactly the behavior this had before windows existed.
///
/// `pinned_threshold` is a threshold whose event must survive the cut whatever the
/// window is. The viewer passes the cut currently in effect: were the selection allowed
/// to drop it, the slider thumb would land on a neighboring stop and zooming would
/// silently re-cluster the network under the user.
pub fn filter_merge_event_rows_binned(
    events: &[MergeEvent],
    max_merge_events: usize,
    density_bins: usize,
    window: Option<(f64, f64)>,
    pinned_threshold: Option<f64>,
) -> Vec<MergeEvent> {
    if max_merge_events == 0 || events.len() <= max_merge_events {
        return events.to_vec();
    }

    // Rank indices rather than cloning the events twice: a MergeEvent owns two Strings
    // and a Vec, and the networks this cap exists for have ~10^5 of them.
    let mut ranked: Vec<usize> = (0..events.len()).collect();
    ranked.sort_by(|&a, &b| merge_event_rank_cmp(&events[a], &events[b]));

    let mut kept: Vec<usize> = match window {
        None => ranked.iter().copied().take(max_merge_events).collect(),
        Some((a, b)) => {
            let (low, high) = if a > b { (b, a) } else { (a, b) };
            ranked
                .iter()
                .copied()
                .filter(|&i| {
                    let t = events[i].threshold_value;
                    low <= t && t <= high
                })
                .take(max_merge_events)
                .collect()
        }
    };
    // A windowed pool is no longer a prefix of `ranked`, so the back-fill has to scan
    // the whole ranking and skip what is already in.
    let mut selected: std::collections::HashSet<usize> = kept.iter().copied().collect();

    let finite: Vec<f64> = events
        .iter()
        .map(|e| e.threshold_value)
        .filter(|v| v.is_finite())
        .collect();
    if !finite.is_empty() && density_bins > 0 {
        let lo = finite.iter().cloned().fold(f64::INFINITY, f64::min);
        let hi = finite.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let mut covered: std::collections::HashSet<i64> = kept
            .iter()
            .map(|&i| merge_event_density_bin(events[i].threshold_value, lo, hi, density_bins))
            .collect();
        // `ranked` is strongest-first, so the first row seen in an empty band is the
        // strongest one available to represent it. Candidates come from the whole
        // series, not the window: a band outside the window still needs its stop.
        for &i in &ranked {
            if selected.contains(&i) {
                continue;
            }
            let bin = merge_event_density_bin(events[i].threshold_value, lo, hi, density_bins);
            if bin < 0 || covered.contains(&bin) {
                continue;
            }
            covered.insert(bin);
            kept.push(i);
            selected.insert(i);
        }
    }

    if let Some(pinned) = pinned_threshold.filter(|p| p.is_finite()) {
        if !kept.iter().any(|&i| events[i].threshold_value == pinned) {
            if let Some(&i) = ranked
                .iter()
                .find(|&&i| events[i].threshold_value == pinned)
            {
                kept.push(i);
            }
        }
    }

    kept.sort_by_key(|&i| events[i].edge_index);
    kept.into_iter().map(|i| events[i].clone()).collect()
}

/// Back-compat wrapper: the default band count, the whole axis, nothing pinned.
pub fn filter_merge_event_rows(events: &[MergeEvent], max_merge_events: usize) -> Vec<MergeEvent> {
    filter_merge_event_rows_binned(
        events,
        max_merge_events,
        MERGE_EVENT_DENSITY_BINS,
        None,
        None,
    )
}

/// A centered moving sum of `merge_impact` over the plotted threshold range.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MovingSum {
    pub window: f64,
    pub x: Vec<f64>,
    pub y: Vec<f64>,
}

/// Centered moving sum of `merge_impact` over the plotted threshold range.
///
/// **Call this with the rows straight from [`threshold_merge_event_rows`], before
/// [`filter_merge_event_rows`] caps them.** The cap keeps the top rows by
/// `merge_impact`, so a moving sum taken afterwards undercounts on any network with
/// more than `max_merge_events` threshold groups — and it undercounts precisely the
/// small events this series exists to reveal.
///
/// Sums `merge_impact` (the per-threshold total), not `largest_merge`: the contrast
/// between the two is what the split chart is for.
pub fn merge_event_moving_sum(
    events: &[MergeEvent],
    window_fraction: f64,
    grid_points: usize,
) -> MovingSum {
    let empty = MovingSum::default();
    if events.is_empty() || grid_points < 2 {
        return empty;
    }

    let mut pairs: Vec<(f64, f64)> = events
        .iter()
        .map(|e| (e.threshold_value, e.merge_impact))
        .filter(|(t, i)| t.is_finite() && i.is_finite())
        .collect();
    if pairs.is_empty() {
        return empty;
    }

    let lo = pairs.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let hi = pairs.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
    if !lo.is_finite() || !hi.is_finite() || hi == lo {
        return empty;
    }

    let window = window_fraction * (hi - lo);
    let half_window = window / 2.0;

    // Prefix sum + binary search turns the window scan into O(n log n + grid)
    // instead of the O(n * grid) a nested loop would cost.
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let sorted_thresholds: Vec<f64> = pairs.iter().map(|p| p.0).collect();
    let mut cumulative = Vec::with_capacity(pairs.len() + 1);
    cumulative.push(0.0);
    let mut running = 0.0;
    for (_, impact) in &pairs {
        running += impact;
        cumulative.push(running);
    }

    // np.linspace: multiply by a precomputed step, and pin the last point to `hi`
    // exactly. Recomputing `lo + i*(hi-lo)/(n-1)` per point drifts in the last ulp.
    let step = (hi - lo) / (grid_points - 1) as f64;
    let mut x = Vec::with_capacity(grid_points);
    for i in 0..grid_points {
        x.push(i as f64 * step + lo);
    }
    x[grid_points - 1] = hi;

    let mut y = Vec::with_capacity(grid_points);
    for &g in &x {
        // 'left'/'right' make the window inclusive at both ends, matching |t-g| <= W/2.
        let start = sorted_thresholds.partition_point(|&t| t < g - half_window);
        let end = sorted_thresholds.partition_point(|&t| t <= g + half_window);
        y.push(cumulative[end] - cumulative[start]);
    }

    MovingSum { window, x, y }
}

/// A cut the threshold slider can land on.
#[derive(Clone, Debug, PartialEq)]
pub struct SliderStop {
    pub edge_index: i64,
    /// Row index in the [`MaxTree`] threshold tables (`-1` when the threshold has no
    /// row), or `None` when the caller has no threshold tables to index at all — a
    /// `.dsnv` bundle carries none, so its stops omit the field rather than pointing
    /// into something that is not there.
    pub threshold_index: Option<i64>,
    pub threshold_label: String,
    /// `None` renders as JSON `null` and means ∞.
    pub threshold_value: Option<f64>,
}

/// The floor threshold: a cut strictly below the weakest merge, so every edge is kept.
///
/// Offset below the weakest merge by 1% of the merge-weight **range**, rather than
/// dropped to `0`: a network scoring 350–650 would otherwise spend more than half its
/// slider track on empty space below the data, and a network with negative scores would
/// not be cleared by `0` at all.
pub fn floor_threshold_value(lowest_merge_threshold: f64, highest_merge_threshold: f64) -> f64 {
    let lowest = lowest_merge_threshold;
    let span = highest_merge_threshold - lowest;
    if span > 0.0 {
        return lowest - FLOOR_THRESHOLD_SPAN_FRACTION * span;
    }
    // Every merge sits at one weight, so there is no range to take a fraction of.
    let step = lowest.abs() * FLOOR_THRESHOLD_SPAN_FRACTION;
    lowest - if step > 0.0 { step } else { 1.0 }
}

/// Slider stops for a merge-event series: `∞`, one per event, then a floor.
///
/// Shared by `matrix_report`, the `.dsnv` viewer (via its JavaScript port) and
/// [`super::super::tools::ssn_bundle`] so their sliders offer the same cuts. **The
/// trailing floor stop is what makes the fully merged network reachable at all**: every
/// other stop excludes its own tie group under the strictly-above `--lb` rule, so the
/// lowest event stop still splits the weakest merge back apart. It is derived from the
/// MST rather than from `events`, which a cap may have thinned.
///
/// `with_threshold_index` asks each stop to carry its row in the tree's threshold
/// tables, which is how `matrix_report` reports a cut's edge count. Only that report
/// has those tables: a bundle-derived tree ([`MaxTree::from_mst_edges`]) has none, so
/// it passes `false` and the stops omit the field entirely.
///
/// Two upstream quirks are reproduced deliberately: the first event stop also carries
/// `edge_index == -1` (colliding with the `∞` stop), and the floor stop's
/// `threshold_index` is `-1` whenever the graph has a non-positive score, because then
/// no `0.0` row exists.
pub fn threshold_slider_stops(
    events: &[MergeEvent],
    tree: Option<&MaxTree>,
    with_threshold_index: bool,
) -> Vec<SliderStop> {
    let index_of = |threshold: f64| -> Option<i64> {
        if with_threshold_index {
            Some(tree.map(|t| t.threshold_row_index(threshold)).unwrap_or(-1))
        } else {
            None
        }
    };

    let mut stops = vec![SliderStop {
        edge_index: -1,
        threshold_index: if with_threshold_index { Some(-1) } else { None },
        threshold_label: "∞".to_string(),
        threshold_value: None,
    }];

    for event in events {
        stops.push(SliderStop {
            edge_index: event.edge_index,
            // Row in the tree's threshold tables for this cut. Those are keyed by
            // distinct threshold, not by MST edge, so the lookup is separate.
            threshold_index: index_of(event.threshold_value),
            threshold_label: event.threshold_to.clone(),
            threshold_value: Some(event.threshold_value),
        });
    }

    if let Some(tree) = tree {
        if !tree.edges.is_empty() {
            let weakest = tree.edges[tree.edges.len() - 1].weight;
            let strongest = tree.edges[0].weight;
            let floor = floor_threshold_value(weakest, strongest);
            stops.push(SliderStop {
                edge_index: tree.edges.len() as i64 - 1,
                // This stop stands for the complete-graph cut, so it reads its edge
                // counts from that row of the threshold tables.
                threshold_index: index_of(0.0),
                threshold_label: format_threshold_value(floor),
                threshold_value: Some(floor),
            });
        }
    }

    stops
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
        if component_min_leaf_by_id[&left_component_id]
            > component_min_leaf_by_id[&right_component_id]
        {
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
            let (left, right) = (
                nodes[component_id].left.unwrap(),
                nodes[component_id].right.unwrap(),
            );
            nodes[component_id].leaf_start = nodes[left].leaf_start;
            nodes[component_id].leaf_count = nodes[left].leaf_count + nodes[right].leaf_count;
            continue;
        }
        let (left, right) = (
            nodes[component_id].left.unwrap(),
            nodes[component_id].right.unwrap(),
        );
        stack.push((component_id, true));
        stack.push((right, false));
        stack.push((left, false));
    }

    Hierarchy {
        nodes,
        roots,
        leaf_order,
    }
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
        // Tie-group prefix counters: nothing scores above 9 (the first candidate);
        // exactly one edge scores above 8. Edge 7 would close a cycle, so it is never
        // an MST edge -- but it is still counted by the `threshold = 0` row below.
        assert_eq!(tree.edges[0].edges_above, 0);
        assert_eq!(tree.edges[0].mst_edges_above, 0);
        assert_eq!(tree.edges[1].edges_above, 1);
        assert_eq!(tree.edges[1].mst_edges_above, 1);

        // Reference vectors generated from Python `MaxTree` at upstream 896a0c8.
        assert_eq!(tree.thresholds(), &[9.0, 8.0, 0.0]);
        assert_eq!(tree.edges_above_threshold(), &[0, 1, 3]);
        assert_eq!(tree.mst_edges_above_threshold(), &[0, 1, 2]);
        assert_eq!(tree.n_edges(), 3);
        assert_eq!(
            tree.cluster_count_by_threshold(),
            vec![(9.0, 4.0), (8.0, 3.0), (0.0, 2.0)]
        );
    }

    #[test]
    fn ties_collapse_to_one_threshold_row() {
        // A 4-cycle where every edge weighs the same. Under the old per-MST-edge
        // keying this produced three rows claiming three different edge counts for
        // one threshold; ties are exactly where that was wrong.
        let m = sym_matrix(4, &[(0, 1, 5.0), (1, 2, 5.0), (2, 3, 5.0), (3, 0, 5.0)]);
        let tree = MaxTree::from_matrix(&m);
        assert_eq!(tree.edges.len(), 3); // spanning tree of a 4-cycle
        assert_eq!(tree.n_edges(), 4);
        // One row for the shared weight, plus the synthetic --lb 0 complete graph.
        assert_eq!(tree.thresholds(), &[5.0, 0.0]);
        assert_eq!(tree.edges_above_threshold(), &[0, 4]);
        assert_eq!(tree.mst_edges_above_threshold(), &[0, 3]);
        assert_eq!(
            tree.cluster_count_by_threshold(),
            vec![(5.0, 4.0), (0.0, 1.0)]
        );
    }

    #[test]
    fn threshold_row_index_roundtrip() {
        let m = sym_matrix(4, &[(0, 1, 9.0), (1, 2, 8.0), (2, 3, 5.0)]);
        let tree = MaxTree::from_matrix(&m);
        for (idx, &t) in tree.thresholds().iter().enumerate() {
            assert_eq!(tree.threshold_row_index(t), idx as i64);
        }
        assert_eq!(tree.threshold_row_index(8.5), -1);
        // -0.0 must reach the 0.0 row, as it does through Python's dict.
        assert_eq!(
            tree.threshold_row_index(-0.0),
            tree.threshold_row_index(0.0)
        );
    }

    #[test]
    fn symmetric_max_sees_zeros_before_dropping_them() {
        // One direction 0, the other negative: Python takes max(0, -3) = 0 and then
        // drops it. Skipping the zero slot first would keep -3, which would also
        // drag `min_score` below zero and delete the synthetic `threshold = 0` row.
        // Row-major 3x3 over (n0, n1, n2); the diagonal and unset pairs stay 0.
        #[rustfmt::skip]
        let d = vec![
            0.0,  0.0, 0.0,  // n0: (0,1) = 0
            -3.0, 0.0, 4.0,  // n1: (1,0) = -3, (1,2) = 4
            0.0,  4.0, 0.0,  // n2: (2,1) = 4
        ];
        let labels: Vec<String> = (0..3).map(|i| format!("n{i}")).collect();
        let m = DataMatrix {
            rows: labels.clone(),
            cols: labels,
            row_lengths: None,
            col_lengths: None,
            data_type: "score".into(),
            storage: Storage::Dense(d),
        };
        let tree = MaxTree::from_matrix(&m);
        assert_eq!(tree.n_edges(), 1); // the (0,1) pair scored 0 and was dropped
        assert_eq!(edge_scores_desc(&m), vec![4.0]);
        assert_eq!(tree.thresholds(), &[4.0, 0.0]);
    }

    #[test]
    fn cluster_count_by_threshold_decreases_by_one_per_edge() {
        let m = sym_matrix(4, &[(0, 1, 9.0), (1, 2, 8.0), (2, 3, 5.0)]);
        let tree = MaxTree::from_matrix(&m);
        // No leading (inf, n_nodes) row any more; a trailing --lb 0 row instead.
        // Reference vectors generated from Python `MaxTree` at upstream 896a0c8.
        let ccbt = tree.cluster_count_by_threshold();
        assert_eq!(ccbt, vec![(9.0, 4.0), (8.0, 3.0), (5.0, 2.0), (0.0, 1.0)]);
        assert_eq!(
            tree.edges_by_threshold(),
            vec![(0.0, 9.0), (1.0, 8.0), (2.0, 5.0), (3.0, 0.0)]
        );
        // Row 0 already is (0, n_nodes): nothing scores above the maximum weight.
        assert_eq!(tree.cluster_count_by_edge_count()[0], (0.0, 4.0));
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
    fn merge_events_carry_their_terms() {
        // Two pairs merge, then join: 0-1 (10), 2-3 (9), then 1-2 (5).
        // Reference values generated from Python ssn_hierarchy at upstream 896a0c8.
        let m = sym_matrix(4, &[(0, 1, 10.0), (2, 3, 9.0), (1, 2, 5.0)]);
        let tree = MaxTree::from_matrix(&m);
        let summary = component_size_summary_by_threshold(&tree, MERGE_IMPACT_MIN_CHILD);
        let rows = threshold_merge_event_rows(&summary);
        assert_eq!(rows.len(), 3);
        // edge_index comes from the FIRST row of the tie group, so the first event is -1.
        assert_eq!(
            rows.iter().map(|e| e.edge_index).collect::<Vec<_>>(),
            vec![-1, 0, 1]
        );
        assert_eq!(rows[0].merge_size_counts, vec![(1.0, 1)]);
        assert_eq!(rows[2].merge_size_counts, vec![(2.0, 1)]);
        assert_eq!(rows[2].largest_merge, 2.0);
        assert_eq!(rows[2].merge_count, 1);
        // The documented invariant: the size histogram reconstructs the sum exactly.
        for e in &rows {
            let total: f64 = e.merge_size_counts.iter().map(|(s, c)| s * *c as f64).sum();
            assert_eq!(total, e.merge_impact);
        }
    }

    #[test]
    fn the_largest_component_column_only_ever_grows() {
        // A merge replaces two components with one strictly larger than either, so the
        // largest size is monotonically non-decreasing — which is what lets a scalar
        // running maximum replace the lazy-deletion heap upstream carried until
        // `34c4b88`. Six nodes whose three strongest edges all tie at 10.0; expected
        // column from Python's component_size_summary_by_threshold at that commit.
        let m = sym_matrix(
            6,
            &[
                (0, 1, 10.0),
                (2, 3, 10.0),
                (4, 5, 10.0),
                (1, 2, 5.0),
                (3, 4, 3.0),
            ],
        );
        let tree = MaxTree::from_matrix(&m);
        let summary = component_size_summary_by_threshold(&tree, MERGE_IMPACT_MIN_CHILD);
        let largest: Vec<f64> = summary.iter().map(|r| r[COMPONENT_LARGEST_COL]).collect();
        assert_eq!(largest, vec![1.0, 2.0, 2.0, 2.0, 4.0, 6.0]);
        assert!(
            largest.windows(2).all(|w| w[0] <= w[1]),
            "must never decrease"
        );
        // And the running maximum is the real maximum, not just the last merge's size:
        // row 3 holds 2.0 while three separate 2-node components exist.
        assert_eq!(summary[3][COMPONENT_LARGEST_COL], 2.0);
    }

    #[test]
    fn tie_group_merges_are_counted_separately() {
        // A 4-cycle at one weight: three separate 1-node merges at a single threshold.
        // merge_impact alone (3.0) cannot distinguish this from one 3-node merge --
        // which is exactly why merge_size_counts/largest_merge/merge_count exist.
        let m = sym_matrix(4, &[(0, 1, 5.0), (1, 2, 5.0), (2, 3, 5.0), (3, 0, 5.0)]);
        let tree = MaxTree::from_matrix(&m);
        let summary = component_size_summary_by_threshold(&tree, MERGE_IMPACT_MIN_CHILD);
        let rows = threshold_merge_event_rows(&summary);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].merge_impact, 3.0);
        assert_eq!(rows[0].merge_size_counts, vec![(1.0, 3)]);
        assert_eq!(rows[0].largest_merge, 1.0);
        assert_eq!(rows[0].merge_count, 3);
    }

    #[test]
    fn moving_sum_matches_numpy_linspace_and_window() {
        let m = sym_matrix(4, &[(0, 1, 10.0), (2, 3, 9.0), (1, 2, 5.0)]);
        let tree = MaxTree::from_matrix(&m);
        let summary = component_size_summary_by_threshold(&tree, MERGE_IMPACT_MIN_CHILD);
        let rows = threshold_merge_event_rows(&summary);
        let ms = merge_event_moving_sum(&rows, MOVING_SUM_WINDOW_FRACTION, MOVING_SUM_GRID_POINTS);
        assert_eq!(ms.x.len(), MOVING_SUM_GRID_POINTS);
        assert_eq!(ms.window, 0.25); // 5% of the 5.0 threshold range
                                     // The grid spans [min, max] with the last point pinned exactly to `hi`.
        assert_eq!(ms.x[0], 5.0);
        assert_eq!(ms.x[MOVING_SUM_GRID_POINTS - 1], 10.0);
        assert_eq!(ms.y[0], 2.0);
        assert_eq!(ms.y[MOVING_SUM_GRID_POINTS - 1], 1.0);
    }

    #[test]
    fn moving_sum_is_empty_without_a_range() {
        // Every merge at one weight: hi == lo, so there is no window to slide.
        let m = sym_matrix(4, &[(0, 1, 5.0), (1, 2, 5.0), (2, 3, 5.0), (3, 0, 5.0)]);
        let tree = MaxTree::from_matrix(&m);
        let summary = component_size_summary_by_threshold(&tree, MERGE_IMPACT_MIN_CHILD);
        let rows = threshold_merge_event_rows(&summary);
        let ms = merge_event_moving_sum(&rows, MOVING_SUM_WINDOW_FRACTION, MOVING_SUM_GRID_POINTS);
        assert_eq!(ms, MovingSum::default());
        assert!(merge_event_moving_sum(&[], MOVING_SUM_WINDOW_FRACTION, 800)
            .x
            .is_empty());
    }

    #[test]
    fn floor_threshold_is_offset_by_span_not_zero() {
        assert_eq!(floor_threshold_value(5.0, 9.0), 4.96); // 1% of the 4.0 span
        assert_eq!(floor_threshold_value(5.0, 5.0), 4.95); // no span -> 1% of |lowest|
        assert_eq!(floor_threshold_value(0.0, 0.0), -1.0); // and no magnitude either
                                                           // A negative-scoring network must still be cleared, which a floor of 0 would not do.
        assert!(floor_threshold_value(-10.0, -2.0) < -10.0);
    }

    #[test]
    fn slider_stops_end_at_a_reachable_floor() {
        let m = sym_matrix(4, &[(0, 1, 10.0), (2, 3, 9.0), (1, 2, 5.0)]);
        let tree = MaxTree::from_matrix(&m);
        let summary = component_size_summary_by_threshold(&tree, MERGE_IMPACT_MIN_CHILD);
        let rows = threshold_merge_event_rows(&summary);
        let stops = threshold_slider_stops(&rows, Some(&tree), true);
        let shape: Vec<(i64, i64, &str)> = stops
            .iter()
            .map(|s| {
                (
                    s.edge_index,
                    s.threshold_index.unwrap(),
                    s.threshold_label.as_str(),
                )
            })
            .collect();
        // Reference from Python threshold_slider_stops at upstream 896a0c8. Note the
        // first event stop also carries edge_index -1, colliding with the inf stop --
        // that is upstream behavior, reproduced deliberately.
        assert_eq!(
            shape,
            vec![
                (-1, -1, "∞"),
                (-1, 0, "10.00"),
                (0, 1, "9.00"),
                (1, 2, "5.00"),
                (2, 3, "4.95"),
            ]
        );
        assert_eq!(stops[0].threshold_value, None);
        // Without the floor stop the fully merged network is unreachable: every other
        // stop excludes its own tie group under the strictly-above --lb rule.
        assert_eq!(stops.last().unwrap().threshold_value, Some(4.95));
    }

    #[test]
    fn density_backfill_adds_without_displacing() {
        // 40 events spread evenly across the axis, capped hard at 2.
        let events: Vec<MergeEvent> = (0..40)
            .map(|i| MergeEvent {
                edge_index: i as i64,
                summary_row_from: 0,
                summary_row_to: 0,
                threshold_from_value: f64::INFINITY,
                threshold_from: "∞".into(),
                threshold_to: format!("{i}"),
                threshold_value: i as f64,
                merge_impact: if i == 0 || i == 39 { 100.0 } else { 1.0 },
                merge_size_counts: vec![(1.0, 1)],
                largest_merge: 1.0,
                merge_count: 1,
                delta_largest: 0.0,
                delta_avg_non_singleton: 0.0,
            })
            .collect();
        let kept = filter_merge_event_rows_binned(&events, 2, MERGE_EVENT_DENSITY_BINS, None, None);
        // Bounded by cap + bands, and strictly more than the cap because bands were empty.
        assert!(kept.len() > 2, "back-fill should have added rows");
        assert!(kept.len() <= 2 + MERGE_EVENT_DENSITY_BINS);
        // The two top-ranked rows survive: a back-filled row is an addition, never a
        // replacement.
        assert!(kept.iter().any(|e| e.edge_index == 0));
        assert!(kept.iter().any(|e| e.edge_index == 39));
        // Output is ordered by edge_index.
        let mut ordered = kept.iter().map(|e| e.edge_index).collect::<Vec<_>>();
        let original = ordered.clone();
        ordered.sort();
        assert_eq!(ordered, original);
        // Every occupied band is represented.
        let lo = 0.0;
        let hi = 39.0;
        let bands: std::collections::HashSet<i64> = kept
            .iter()
            .map(|e| merge_event_density_bin(e.threshold_value, lo, hi, MERGE_EVENT_DENSITY_BINS))
            .collect();
        assert_eq!(bands.len(), MERGE_EVENT_DENSITY_BINS);
        // A cap of 0 means no cap AND no back-fill.
        assert_eq!(
            filter_merge_event_rows_binned(&events, 0, MERGE_EVENT_DENSITY_BINS, None, None).len(),
            40
        );
    }

    /// 40 events evenly spaced on the axis; impact 100 at the two ends and 50 at three
    /// points in the middle, 1 everywhere else. That shape makes the window observable:
    /// a whole-range ranking takes the ends, a windowed one takes the middle bumps.
    fn banded_events() -> Vec<MergeEvent> {
        (0..40)
            .map(|i| MergeEvent {
                edge_index: i as i64,
                summary_row_from: 0,
                summary_row_to: 0,
                threshold_from_value: f64::INFINITY,
                threshold_from: "∞".into(),
                threshold_to: format!("{i}"),
                threshold_value: i as f64,
                merge_impact: match i {
                    0 | 39 => 100.0,
                    18 | 20 | 22 => 50.0,
                    _ => 1.0,
                },
                merge_size_counts: vec![(1.0, 1)],
                largest_merge: 1.0,
                merge_count: 1,
                delta_largest: 0.0,
                delta_avg_non_singleton: 0.0,
            })
            .collect()
    }

    fn kept_indices(rows: &[MergeEvent]) -> Vec<i64> {
        rows.iter().map(|e| e.edge_index).collect()
    }

    #[test]
    fn a_window_narrows_the_top_n_pool() {
        // Band back-fill switched off (density_bins = 0) so this sees the top-N pool
        // alone. Expected values from Python's filter_merge_event_rows at upstream
        // d34ea4c on the same synthetic rows.
        let events = banded_events();
        assert_eq!(
            kept_indices(&filter_merge_event_rows_binned(&events, 3, 0, None, None)),
            vec![0, 18, 39]
        );
        // Zoomed into 15..25, the same three slots are re-spent on that stretch: the
        // small events a whole-range ranking buries become the selection.
        let window = Some((15.0, 25.0));
        assert_eq!(
            kept_indices(&filter_merge_event_rows_binned(&events, 3, 0, window, None)),
            vec![18, 20, 22]
        );
        // An inverted window means the same range.
        assert_eq!(
            kept_indices(&filter_merge_event_rows_binned(
                &events,
                3,
                0,
                Some((25.0, 15.0)),
                None
            )),
            vec![18, 20, 22]
        );
    }

    #[test]
    fn a_window_does_not_narrow_the_band_backfill() {
        // The window narrows what the cap *ranks*, never what the axis has to cover:
        // a band outside it still needs its stop, or the slider goes dead there.
        let events = banded_events();
        let kept = filter_merge_event_rows_binned(
            &events,
            3,
            MERGE_EVENT_DENSITY_BINS,
            Some((15.0, 25.0)),
            None,
        );
        let bands: std::collections::HashSet<i64> = kept
            .iter()
            .map(|e| {
                merge_event_density_bin(e.threshold_value, 0.0, 39.0, MERGE_EVENT_DENSITY_BINS)
            })
            .collect();
        assert_eq!(
            bands.len(),
            MERGE_EVENT_DENSITY_BINS,
            "every band must keep a stop"
        );
        assert!(
            kept.iter().any(|e| e.edge_index == 0),
            "a band below the window still counts"
        );
        assert!(kept.iter().any(|e| e.edge_index == 39), "and one above it");
        // Still bounded, and still ordered by edge_index.
        assert!(kept.len() <= 3 + MERGE_EVENT_DENSITY_BINS);
        assert!(kept_indices(&kept).windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn a_pinned_threshold_survives_a_window_that_excludes_it() {
        // The cut currently in effect must outlive a zoom. Were it droppable, the
        // slider thumb would land on a neighboring stop and the displayed clustering
        // would change as a side effect of zooming.
        let events = banded_events();
        let window = Some((15.0, 25.0));
        assert_eq!(
            kept_indices(&filter_merge_event_rows_binned(
                &events,
                3,
                0,
                window,
                Some(39.0)
            )),
            vec![18, 20, 22, 39]
        );
        // A pin already in the selection is a no-op, not a duplicate.
        assert_eq!(
            kept_indices(&filter_merge_event_rows_binned(
                &events,
                3,
                0,
                window,
                Some(18.0)
            )),
            vec![18, 20, 22]
        );
        // A pin naming no event, and a non-finite one, are both ignored.
        for pin in [1234.5, f64::INFINITY, f64::NAN] {
            assert_eq!(
                kept_indices(&filter_merge_event_rows_binned(
                    &events,
                    3,
                    0,
                    window,
                    Some(pin)
                )),
                vec![18, 20, 22],
                "pin {pin} should change nothing"
            );
        }
    }

    #[test]
    fn stops_omit_the_threshold_index_when_there_are_no_tables() {
        // A .dsnv bundle carries no threshold tables, so its stops must omit the field
        // rather than index something that is not there — while still ending at the
        // floor stop, which comes from the MST and is always available.
        let m = sym_matrix(4, &[(0, 1, 10.0), (2, 3, 9.0), (1, 2, 5.0)]);
        let tree = MaxTree::from_matrix(&m);
        let summary = component_size_summary_by_threshold(&tree, MERGE_IMPACT_MIN_CHILD);
        let rows = threshold_merge_event_rows(&summary);

        let stops = threshold_slider_stops(&rows, Some(&tree), false);
        assert!(stops.iter().all(|s| s.threshold_index.is_none()));
        // Same cuts either way — only the index field differs.
        let indexed = threshold_slider_stops(&rows, Some(&tree), true);
        assert!(indexed.iter().all(|s| s.threshold_index.is_some()));
        let cuts = |v: &[SliderStop]| -> Vec<(i64, Option<f64>, String)> {
            v.iter()
                .map(|s| (s.edge_index, s.threshold_value, s.threshold_label.clone()))
                .collect()
        };
        assert_eq!(cuts(&stops), cuts(&indexed));
        assert_eq!(stops.last().unwrap().threshold_value, Some(4.95));
    }

    #[test]
    fn a_tree_rebuilt_from_stored_mst_edges_derives_the_same_series() {
        // What ssn_bundle does with a .dsnv: the split events, their moving sum and the
        // slider cuts are a pure function of (n_nodes, mst_edges, metric), so a tree
        // carrying only those must reproduce what the matrix-built tree gives.
        let m = sym_matrix(
            6,
            &[
                (0, 1, 10.0),
                (2, 3, 9.0),
                (1, 2, 5.0),
                (4, 5, 8.0),
                (3, 4, 4.0),
                (0, 5, 2.0),
            ],
        );
        let full = MaxTree::from_matrix(&m);
        let stored: Vec<(usize, usize, f64)> = full.mst_edges().collect();
        let rebuilt = MaxTree::from_mst_edges(full.n_nodes, &stored);

        let rows_of = |t: &MaxTree| {
            threshold_merge_event_rows(&component_size_summary_by_threshold(
                t,
                MERGE_IMPACT_MIN_CHILD,
            ))
        };
        let a = rows_of(&full);
        let b = rows_of(&rebuilt);
        assert_eq!(
            a.iter()
                .map(|e| (e.edge_index, e.threshold_value, e.merge_impact))
                .collect::<Vec<_>>(),
            b.iter()
                .map(|e| (e.edge_index, e.threshold_value, e.merge_impact))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            merge_event_moving_sum(&a, MOVING_SUM_WINDOW_FRACTION, MOVING_SUM_GRID_POINTS),
            merge_event_moving_sum(&b, MOVING_SUM_WINDOW_FRACTION, MOVING_SUM_GRID_POINTS)
        );
        // The hierarchy too — that is what the bundle stores rather than derives, so
        // the two must agree or a rebuilt cut would name different clusters.
        let ha = build_mst_component_hierarchy(&full);
        let hb = build_mst_component_hierarchy(&rebuilt);
        assert_eq!(ha.roots, hb.roots);
        assert_eq!(ha.leaf_order, hb.leaf_order);
        // The threshold tables are the one thing it cannot reconstruct, and it says so
        // rather than inventing a row.
        assert_eq!(rebuilt.threshold_row_index(10.0), -1);
        assert!(rebuilt.thresholds().is_empty());
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
        // No edges at all -> no threshold rows (not a lone `inf` row).
        assert!(tree.cluster_count_by_threshold().is_empty());
        assert!(tree.thresholds().is_empty());
        assert_eq!(tree.n_edges(), 0);
        let h = build_mst_component_hierarchy(&tree);
        assert_eq!(h.roots, vec![0]);
        assert_eq!(h.leaf_order, vec![0]);
    }
}
