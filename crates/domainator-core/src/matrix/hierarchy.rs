//! Agglomerative clustering on a condensed distance vector: a port of the parts of
//! `scipy.cluster.hierarchy` (1.18) that upstream's `compare_contigs` sorts with --
//! `linkage` (single, complete, average, weighted, ward), `leaves_list` and
//! `optimal_leaf_ordering`. scipy is BSD-3-Clause.
//!
//! The kernels are followed statement for statement (`_hierarchy.pyx`'s `nn_chain`,
//! `mst_single_linkage` and `label`, `_optimal_leaf_ordering.pyx`'s `identify_swaps`),
//! including their floating-point types and their order of operations, because the
//! merge order -- and so the contig order -- turns on exact ties and roundings.

/// A linkage method (`scipy.cluster.hierarchy.linkage`'s `method`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Single,
    Complete,
    Average,
    Weighted,
    Ward,
}

impl std::str::FromStr for Method {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        Ok(match s {
            "single" => Method::Single,
            "complete" => Method::Complete,
            "average" => Method::Average,
            "weighted" => Method::Weighted,
            "ward" => Method::Ward,
            _ => {
                return Err(format!(
                    "Unknown linkage method {s:?}, expected one of average, ward, complete, \
                     single, weighted"
                ))
            }
        })
    }
}

/// One row of a linkage matrix: clusters `left` and `right` (leaves `< n`, the cluster
/// made by row `i` is `n + i`) merged at `height` into a cluster of `size` leaves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Merge {
    pub left: usize,
    pub right: usize,
    pub height: f64,
    pub size: usize,
}

/// The index of pair `(i, j)`, `i != j`, in a condensed distance vector over `n` points.
pub fn condensed_index(n: usize, i: usize, j: usize) -> usize {
    let (i, j) = if i < j { (i, j) } else { (j, i) };
    n * i - (i * (i + 1) / 2) + (j - i - 1)
}

/// The number of points a condensed distance vector of `len` entries covers.
fn num_points(len: usize) -> usize {
    // n (n - 1) / 2 == len
    let n = (1 + 8 * len).isqrt().div_ceil(2);
    debug_assert_eq!(n * (n - 1) / 2, len);
    n
}

/// The distance from cluster `i` to the cluster made of `x` and `y`
/// (`_hierarchy_distance_update.pxi`).
fn new_distance(
    method: Method,
    d_xi: f64,
    d_yi: f64,
    d_xy: f64,
    size_x: i32,
    size_y: i32,
    size_i: i32,
) -> f64 {
    match method {
        Method::Single => d_xi.min(d_yi),
        Method::Complete => d_xi.max(d_yi),
        Method::Average => {
            (f64::from(size_x) * d_xi + f64::from(size_y) * d_yi) / f64::from(size_x + size_y)
        }
        Method::Weighted => 0.5 * (d_xi + d_yi),
        Method::Ward => {
            let t = 1.0 / f64::from(size_x + size_y + size_i);
            (f64::from(size_i + size_x) * t * d_xi * d_xi
                + f64::from(size_i + size_y) * t * d_yi * d_yi
                - f64::from(size_i) * t * d_xy * d_xy)
                .sqrt()
        }
    }
}

/// `scipy.cluster.hierarchy.linkage(condensed, method)`. `condensed` must cover at
/// least two points and hold only finite values.
pub fn linkage(condensed: &[f64], method: Method) -> Vec<Merge> {
    let n = num_points(condensed.len());
    let mut z = match method {
        Method::Single => mst_single_linkage(condensed, n),
        _ => nn_chain(condensed, n, method),
    };
    // Sort by height, stably (`np.argsort(kind='mergesort')`), then relabel.
    z.sort_by(|a, b| a.height.total_cmp(&b.height));
    label(&mut z, n);
    z
}

/// `_hierarchy.nn_chain`: the nearest-neighbor-chain algorithm, rows unsorted and
/// unlabeled (`left`/`right` are the surviving slots, not cluster ids).
fn nn_chain(dists: &[f64], n: usize, method: Method) -> Vec<Merge> {
    let mut z = Vec::with_capacity(n - 1);
    let mut d = dists.to_vec();
    let mut size = vec![1i32; n];
    let mut chain: Vec<usize> = Vec::with_capacity(n);
    let mut y = 0usize;
    for _ in 0..n - 1 {
        if chain.is_empty() {
            chain.push(size.iter().position(|&s| s > 0).unwrap_or(0));
        }
        // Go through the chain of neighbors until two mutual neighbors are found.
        let (x, current_min) = loop {
            let x = chain[chain.len() - 1];
            // Prefer the previous element of the chain as the minimum, to avoid cycles.
            let mut current_min = if chain.len() > 1 {
                y = chain[chain.len() - 2];
                d[condensed_index(n, x, y)]
            } else {
                f64::INFINITY
            };
            for (i, &s) in size.iter().enumerate() {
                if s == 0 || x == i {
                    continue;
                }
                let dist = d[condensed_index(n, x, i)];
                if dist < current_min {
                    current_min = dist;
                    y = i;
                }
            }
            if chain.len() > 1 && y == chain[chain.len() - 2] {
                break (x, current_min);
            }
            chain.push(y);
        };
        // Merge clusters x and y and pop them from the chain.
        chain.truncate(chain.len() - 2);
        let (x, y) = if x > y { (y, x) } else { (x, y) };
        let (nx, ny) = (size[x], size[y]);
        z.push(Merge {
            left: x,
            right: y,
            height: current_min,
            size: 0, // set by `label`
        });
        size[x] = 0;
        size[y] = nx + ny;
        for i in 0..n {
            let ni = size[i];
            if ni == 0 || i == y {
                continue;
            }
            d[condensed_index(n, i, y)] = new_distance(
                method,
                d[condensed_index(n, i, x)],
                d[condensed_index(n, i, y)],
                current_min,
                nx,
                ny,
                ni,
            );
        }
    }
    z
}

/// `_hierarchy.mst_single_linkage`: Prim's algorithm, rows unsorted and unlabeled.
fn mst_single_linkage(dists: &[f64], n: usize) -> Vec<Merge> {
    let mut z = Vec::with_capacity(n - 1);
    let mut merged = vec![false; n];
    let mut d = vec![f64::INFINITY; n];
    let (mut x, mut y) = (0usize, 0usize);
    for _ in 0..n - 1 {
        let mut current_min = f64::INFINITY;
        merged[x] = true;
        for i in 0..n {
            if merged[i] {
                continue;
            }
            let dist = dists[condensed_index(n, x, i)];
            if d[i] > dist {
                d[i] = dist;
            }
            if d[i] < current_min {
                y = i;
                current_min = d[i];
            }
        }
        z.push(Merge {
            left: x,
            right: y,
            height: current_min,
            size: 0, // set by `label`
        });
        x = y;
    }
    z
}

/// `_hierarchy.label`: give each row of a height-sorted dendrogram its cluster ids
/// (smaller first) and size, by union-find over the slots it merged.
fn label(z: &mut [Merge], n: usize) {
    let mut parent: Vec<usize> = (0..2 * n - 1).collect();
    let mut size = vec![1usize; 2 * n - 1];
    let mut next_label = n;
    let find = |parent: &mut Vec<usize>, mut x: usize| {
        let mut p = x;
        while parent[x] != x {
            x = parent[x];
        }
        while parent[p] != x {
            let up = parent[p];
            parent[p] = x;
            p = up;
        }
        x
    };
    for row in z.iter_mut() {
        let x_root = find(&mut parent, row.left);
        let y_root = find(&mut parent, row.right);
        (row.left, row.right) = if x_root < y_root {
            (x_root, y_root)
        } else {
            (y_root, x_root)
        };
        parent[x_root] = next_label;
        parent[y_root] = next_label;
        size[next_label] = size[x_root] + size[y_root];
        row.size = size[next_label];
        next_label += 1;
    }
}

/// `scipy.cluster.hierarchy.leaves_list`: the leaves in dendrogram order, each
/// cluster's `left` before its `right`.
pub fn leaves_list(z: &[Merge]) -> Vec<usize> {
    let n = z.len() + 1;
    let mut out = Vec::with_capacity(n);
    let mut stack = vec![2 * n - 2];
    while let Some(node) = stack.pop() {
        if node < n {
            out.push(node);
        } else {
            let merge = &z[node - n];
            stack.push(merge.right);
            stack.push(merge.left);
        }
    }
    out
}

/// `scipy.cluster.hierarchy.optimal_leaf_ordering`: `z` with the children of some
/// clusters swapped so that the sum of distances between adjacent leaves is minimal
/// (Bar-Joseph, Gifford and Jaakkola, 2001).
pub fn optimal_leaf_ordering(z: &[Merge], condensed: &[f64]) -> Vec<Merge> {
    let n = z.len() + 1;
    // Relabel the leaves by their current order, so every cluster is a range.
    let sorted_leaves = leaves_list(z);
    let mut to_sorted = vec![0usize; n];
    for (sorted_i, &orig_i) in sorted_leaves.iter().enumerate() {
        to_sorted[orig_i] = sorted_i;
    }
    let relabel = |v: usize| if v < n { to_sorted[v] } else { v };
    let sorted_z: Vec<(usize, usize)> = z
        .iter()
        .map(|m| (relabel(m.left), relabel(m.right)))
        .collect();
    let mut sorted_d = vec![0.0f64; n * n];
    for i in 0..n {
        for j in 0..n {
            if i != j {
                sorted_d[i * n + j] =
                    condensed[condensed_index(n, sorted_leaves[i], sorted_leaves[j])];
            }
        }
    }
    let mut ranges = vec![(0usize, 0usize); 2 * n - 1];
    for (i, range) in ranges.iter_mut().enumerate().take(n) {
        *range = (i, i + 1);
    }
    for (link_i, &(v_l, v_r)) in sorted_z.iter().enumerate() {
        ranges[n + link_i] = (ranges[v_l].0, ranges[v_r].1);
    }
    let must_swap = identify_swaps(&sorted_z, &sorted_d, &ranges);
    // A cluster is flipped once per ancestor (itself included) that must swap.
    let mut final_swap = must_swap.clone();
    for i in (0..z.len()).rev() {
        for child in [z[i].left, z[i].right] {
            if child >= n {
                final_swap[child - n] ^= final_swap[i];
            }
        }
    }
    z.iter()
        .zip(final_swap)
        .map(|(m, swap)| {
            if swap {
                Merge {
                    left: m.right,
                    right: m.left,
                    ..*m
                }
            } else {
                *m
            }
        })
        .collect()
}

/// `_optimal_leaf_ordering.identify_swaps`: for each merge, whether its children must
/// be swapped. `M[u, w]` is the best ordering cost of the cluster with `u` and `w` at
/// its ends, in single precision as in scipy.
fn identify_swaps(
    sorted_z: &[(usize, usize)],
    sorted_d: &[f64],
    ranges: &[(usize, usize)],
) -> Vec<bool> {
    let n = sorted_z.len() + 1;
    let mut m_cost = vec![0.0f32; n * n];
    let mut swap_status = vec![[0u8; 2]; n * n];
    let mut must_swap = vec![false; sorted_z.len()];
    let (mut best_u, mut best_w) = (0usize, 0usize);
    let mut m_vals: Vec<f32> = Vec::new();
    let mut m_idx: Vec<usize> = Vec::new();
    let mut k_vals: Vec<f32> = Vec::new();
    let mut k_idx: Vec<usize> = Vec::new();
    for &(v_l, v_r) in sorted_z {
        let (u_clusters, m_clusters): (Vec<usize>, Vec<usize>) = if v_l < n {
            (vec![v_l], vec![v_l])
        } else {
            let (a, b) = sorted_z[v_l - n];
            (vec![a, b], vec![b, a])
        };
        let (w_clusters, k_clusters): (Vec<usize>, Vec<usize>) = if v_r < n {
            (vec![v_r], vec![v_r])
        } else {
            let (a, b) = sorted_z[v_r - n];
            (vec![b, a], vec![a, b])
        };
        for swap_l in 0..u_clusters.len() {
            for swap_r in 0..w_clusters.len() {
                let (u_min, u_max) = ranges[u_clusters[swap_l]];
                let (m_min, m_max) = ranges[m_clusters[swap_l]];
                let (w_min, w_max) = ranges[w_clusters[swap_r]];
                let (k_min, k_max) = ranges[k_clusters[swap_r]];
                let mut min_km_dist: f32 = 1_073_741_824.0;
                for m in m_min..m_max {
                    for k in k_min..k_max {
                        if sorted_d[m * n + k] < f64::from(min_km_dist) {
                            min_km_dist = sorted_d[m * n + k] as f32;
                        }
                    }
                }
                for u in u_min..u_max {
                    sort_slice(&m_cost, n, &mut m_vals, &mut m_idx, m_min, m_max, u);
                    for w in w_min..w_max {
                        sort_slice(&m_cost, n, &mut k_vals, &mut k_idx, k_min, k_max, w);
                        let mut cur_min: f32 = 1_073_741_824.0;
                        for &m in &m_idx {
                            if m_cost[u * n + m] + m_cost[w * n + k_idx[0]] + min_km_dist >= cur_min
                            {
                                break;
                            }
                            for &k in &k_idx {
                                if m_cost[u * n + m] + m_cost[w * n + k] + min_km_dist >= cur_min {
                                    break;
                                }
                                let current = (f64::from(m_cost[u * n + m] + m_cost[w * n + k])
                                    + sorted_d[m * n + k])
                                    as f32;
                                if current < cur_min {
                                    cur_min = current;
                                }
                            }
                        }
                        m_cost[u * n + w] = cur_min;
                        m_cost[w * n + u] = cur_min;
                        let status = [swap_l as u8, swap_r as u8];
                        swap_status[u * n + w] = status;
                        swap_status[w * n + u] = status;
                    }
                }
            }
        }
        let mut cur_min: f32 = 1_073_741_824.0;
        for u in ranges[v_l].0..ranges[v_l].1 {
            for w in ranges[v_r].0..ranges[v_r].1 {
                if m_cost[u * n + w] < cur_min {
                    cur_min = m_cost[u * n + w];
                    best_u = u;
                    best_w = w;
                }
            }
        }
        if v_l >= n {
            must_swap[v_l - n] = swap_status[best_u * n + best_w][0] == 1;
        }
        if v_r >= n {
            must_swap[v_r - n] = swap_status[best_u * n + best_w][1] == 1;
        }
    }
    must_swap
}

/// `_sort_M_slice`: `M[lo..hi, col]` and its row indices, sorted by value with scipy's
/// quicksort (which is not stable, so ties land where it puts them).
fn sort_slice(
    m_cost: &[f32],
    n: usize,
    vals: &mut Vec<f32>,
    idx: &mut Vec<usize>,
    lo: usize,
    hi: usize,
    col: usize,
) {
    vals.clear();
    idx.clear();
    for row in lo..hi {
        vals.push(m_cost[row * n + col]);
        idx.push(row);
    }
    simultaneous_sort(vals, idx);
}

/// `_simultaneous_sort` (taken by scipy from scikit-learn): a quicksort of `dist`
/// applying the same swaps to `idx`.
fn simultaneous_sort(dist: &mut [f32], idx: &mut [usize]) {
    let size = dist.len();
    let swap = |dist: &mut [f32], idx: &mut [usize], a: usize, b: usize| {
        dist.swap(a, b);
        idx.swap(a, b);
    };
    match size {
        0 | 1 => {}
        2 => {
            if dist[0] > dist[1] {
                swap(dist, idx, 0, 1);
            }
        }
        3 => {
            if dist[0] > dist[1] {
                swap(dist, idx, 0, 1);
            }
            if dist[1] > dist[2] {
                swap(dist, idx, 1, 2);
                if dist[0] > dist[1] {
                    swap(dist, idx, 0, 1);
                }
            }
        }
        _ => {
            let pivot_idx = size / 2;
            if dist[0] > dist[size - 1] {
                swap(dist, idx, 0, size - 1);
            }
            if dist[size - 1] > dist[pivot_idx] {
                swap(dist, idx, size - 1, pivot_idx);
                if dist[0] > dist[size - 1] {
                    swap(dist, idx, 0, size - 1);
                }
            }
            let pivot_val = dist[size - 1];
            let mut store_idx = 0;
            for i in 0..size - 1 {
                if dist[i] < pivot_val {
                    swap(dist, idx, i, store_idx);
                    store_idx += 1;
                }
            }
            swap(dist, idx, store_idx, size - 1);
            let pivot_idx = store_idx;
            if pivot_idx > 1 {
                simultaneous_sort(&mut dist[..pivot_idx], &mut idx[..pivot_idx]);
            }
            if pivot_idx + 2 < size {
                simultaneous_sort(&mut dist[pivot_idx + 1..], &mut idx[pivot_idx + 1..]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(z: &[Merge]) -> Vec<(usize, usize, f64, usize)> {
        z.iter()
            .map(|m| (m.left, m.right, m.height, m.size))
            .collect()
    }

    /// Distances between the points 0, 1, 3, 7 and 15 on a line.
    fn line() -> Vec<f64> {
        let x = [0.0, 1.0, 3.0, 7.0, 15.0f64];
        let mut d = Vec::new();
        for i in 0..x.len() {
            for j in i + 1..x.len() {
                d.push((x[i] - x[j]).abs());
            }
        }
        d
    }

    /// Values from `scipy.cluster.hierarchy.linkage(line(), method)`.
    #[test]
    fn linkage_matches_scipy() {
        assert_eq!(
            rows(&linkage(&line(), Method::Single)),
            [
                (0, 1, 1.0, 2),
                (2, 5, 2.0, 3),
                (3, 6, 4.0, 4),
                (4, 7, 8.0, 5)
            ]
        );
        assert_eq!(
            rows(&linkage(&line(), Method::Average)),
            [
                (0, 1, 1.0, 2),
                (2, 5, 2.5, 3),
                (3, 6, 17.0 / 3.0, 4),
                (4, 7, 12.25, 5)
            ]
        );
        assert_eq!(
            rows(&linkage(&line(), Method::Complete)),
            [
                (0, 1, 1.0, 2),
                (2, 5, 3.0, 3),
                (3, 6, 7.0, 4),
                (4, 7, 15.0, 5)
            ]
        );
    }

    #[test]
    fn leaves_list_is_preorder() {
        let z = linkage(&line(), Method::Average);
        assert_eq!(leaves_list(&z), [4, 3, 2, 0, 1]);
    }

    /// The optimal order of the line puts its points in order along it.
    #[test]
    fn optimal_ordering_walks_the_line() {
        let d = line();
        let z = optimal_leaf_ordering(&linkage(&d, Method::Average), &d);
        let order = leaves_list(&z);
        assert!(
            order == [0, 1, 2, 3, 4] || order == [4, 3, 2, 1, 0],
            "{order:?}"
        );
    }
}
