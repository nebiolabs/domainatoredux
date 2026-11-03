//! `compare_contigs`: pairwise gene-neighborhood similarity from a Domainator-
//! annotated GenBank, via the Jaccard (shared domain *content*) and Adjacency
//! (shared consecutive domain *pairs*) indices, combined by weight. Mirrors
//! `domainator.compare_contigs` (the matrix path).
//!
//! Also the hierarchically sorted `--genbank_out`: [`hierarchical_order`] clusters the
//! contigs on `1 - score / max(score)` (with [`crate::matrix::hierarchy`], a port of
//! scipy's) and orders the leaves.

use std::collections::{HashMap, HashSet};

use crate::matrix::hierarchy::{
    condensed_index, leaves_list, linkage, optimal_leaf_ordering, Merge, Method,
};
use crate::matrix::mst_knn::{apply_mst_knn, TreeTies};
use crate::matrix::{build_sparse_max, csr_index, DataMatrix, Storage};
use crate::seq::SeqRecord;
use crate::DOMAIN_FEATURE_NAME;

/// Which similarity metric a feature set encodes.
#[derive(Clone, Copy)]
pub enum Metric {
    /// Jaccard index over the set of distinct domain names per contig.
    Jaccard,
    /// Adjacency index over the set of consecutive `(domain, previous_domain)`
    /// pairs per contig (directional; assumes consistent contig orientation).
    Adjacency,
}

/// Options for [`compare_contigs`].
pub struct CompareContigsOptions {
    pub ji_weight: f64,
    pub ai_weight: f64,
    /// Restrict to domains from these databases (`None` = all).
    pub databases: Option<HashSet<String>>,
    /// `--max_hits_per_query`: keep each row's best combined scores (applied before
    /// `--mst`/`--knn`).
    pub max_hits_per_query: Option<usize>,
    /// `--threshold`: zero out final scores `<= threshold`.
    pub threshold: f64,
    /// `--mst`: keep the maximum spanning forest's edges (with `knn`, the union).
    pub mst: bool,
    /// `--knn`: keep the OR-symmetric k-nearest-neighbor edges.
    pub knn: Option<usize>,
    /// Worker threads (`0` = all available cores).
    pub cpu: usize,
    /// Draw a bar of the contigs scored (`--progress`).
    pub progress: bool,
}

impl Default for CompareContigsOptions {
    fn default() -> Self {
        CompareContigsOptions {
            ji_weight: 0.5,
            ai_weight: 0.5,
            databases: None,
            max_hits_per_query: None,
            threshold: 0.0,
            mst: false,
            knn: None,
            cpu: 0,
            progress: false,
        }
    }
}

/// Domain `name` qualifiers of a contig's `Domainator` features, in feature
/// order, honoring the optional database filter.
fn domain_names<'a>(rec: &'a SeqRecord, databases: Option<&HashSet<String>>) -> Vec<&'a str> {
    let mut out = Vec::new();
    for f in &rec.features {
        if f.kind != DOMAIN_FEATURE_NAME {
            continue;
        }
        if let Some(dbs) = databases {
            match f.get_first("database") {
                Some(db) if dbs.contains(db) => {}
                _ => continue,
            }
        }
        if let Some(name) = f.get_first("name") {
            out.push(name);
        }
    }
    out
}

/// Per-contig feature set (as interned, sorted, unique feature ids) for `metric`.
/// Returns the per-contig id vectors; feature interning is shared across contigs.
fn feature_sets(
    recs: &[SeqRecord],
    metric: Metric,
    databases: Option<&HashSet<String>>,
) -> Vec<Vec<u32>> {
    let mut intern: HashMap<String, u32> = HashMap::new();
    let mut sets: Vec<Vec<u32>> = Vec::with_capacity(recs.len());

    for rec in recs {
        let mut ids: HashSet<u32> = HashSet::new();
        match metric {
            Metric::Jaccard => {
                for name in domain_names(rec, databases) {
                    let next = intern.len() as u32;
                    let id = *intern.entry(name.to_string()).or_insert(next);
                    ids.insert(id);
                }
            }
            Metric::Adjacency => {
                // Features sorted by start, then consecutive (current, previous) pairs.
                let mut feats: Vec<&crate::seq::SeqFeature> = rec
                    .features
                    .iter()
                    .filter(|f| f.kind == DOMAIN_FEATURE_NAME)
                    .filter(|f| match databases {
                        Some(dbs) => f
                            .get_first("database")
                            .map(|db| dbs.contains(db))
                            .unwrap_or(false),
                        None => true,
                    })
                    .collect();
                feats.sort_by_key(|f| f.location.start());
                let mut last: Option<&str> = None;
                for f in feats {
                    if let Some(name) = f.get_first("name") {
                        if let Some(prev) = last {
                            let key = format!("{name}\u{0}{prev}");
                            let next = intern.len() as u32;
                            let id = *intern.entry(key).or_insert(next);
                            ids.insert(id);
                        }
                        last = Some(name);
                    }
                }
            }
        }
        let mut v: Vec<u32> = ids.into_iter().collect();
        v.sort_unstable();
        sets.push(v);
    }
    sets
}

/// Size of the intersection of two sorted, unique id vectors.
fn intersection_size(a: &[u32], b: &[u32]) -> usize {
    let (mut i, mut j, mut n) = (0, 0, 0);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                n += 1;
                i += 1;
                j += 1;
            }
        }
    }
    n
}

/// Compute the combined JI/AI similarity matrix for `recs`. `k` is the per-row
/// top-N cap (`None` keeps every non-zero entry — used by `--mst_knn`).
fn combined_matrix(
    recs: &[SeqRecord],
    opts: &CompareContigsOptions,
    k: Option<usize>,
) -> DataMatrix {
    let n = recs.len();
    let mut metrics: Vec<(f64, Vec<Vec<u32>>)> = Vec::new();
    if opts.ji_weight != 0.0 {
        metrics.push((
            opts.ji_weight,
            feature_sets(recs, Metric::Jaccard, opts.databases.as_ref()),
        ));
    }
    if opts.ai_weight != 0.0 {
        metrics.push((
            opts.ai_weight,
            feature_sets(recs, Metric::Adjacency, opts.databases.as_ref()),
        ));
    }
    let diag: f64 = metrics.iter().map(|(w, _)| *w).sum();

    // Each row's scores are computed independently (reading the shared, read-only
    // feature sets); parallelize over rows, then concatenate the triples.
    let bar = crate::progress::Progress::new(opts.progress, n, "Scoring contigs");
    let per_row: Vec<Vec<(i64, i64, f64)>> = crate::parallel::with_threads(opts.cpu, || {
        use rayon::prelude::*;
        (0..n)
            .into_par_iter()
            .map(|i| {
                // Off-diagonal combined scores, plus the diagonal (each metric
                // contributes 1.0 * weight). The diagonal competes for a top-k slot
                // and is subject to `lb`, exactly as in the Python combined chunk.
                let mut row_scores: HashMap<usize, f64> = HashMap::new();
                row_scores.insert(i, diag);
                for (w, sets) in &metrics {
                    let si = &sets[i];
                    if si.is_empty() {
                        continue;
                    }
                    for (j, sj) in sets.iter().enumerate() {
                        if j == i || sj.is_empty() {
                            continue;
                        }
                        let shared = intersection_size(si, sj);
                        if shared == 0 {
                            continue;
                        }
                        let union = (si.len() + sj.len() - shared) as f64;
                        let score = shared as f64 / union;
                        if score > 0.0 {
                            *row_scores.entry(j).or_insert(0.0) += w * score;
                        }
                    }
                }

                // Optional top-k per row (by score desc, then column asc — matching
                // Python's `np.lexsort((target_indices, -row_scores))`), then `lb`.
                let mut entries: Vec<(usize, f64)> = row_scores.into_iter().collect();
                if let Some(k) = k {
                    entries.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
                    entries.truncate(k);
                }
                bar.inc(1);
                entries
                    .into_iter()
                    .filter(|&(_, v)| !(opts.threshold > 0.0 && v <= opts.threshold))
                    .map(|(j, v)| (i as i64, j as i64, v))
                    .collect()
            })
            .collect()
    });
    drop(bar);

    let mut rows_v: Vec<i64> = Vec::new();
    let mut cols_v: Vec<i64> = Vec::new();
    let mut vals_v: Vec<f64> = Vec::new();
    for row in per_row {
        for (ri, ci, v) in row {
            rows_v.push(ri);
            cols_v.push(ci);
            vals_v.push(v);
        }
    }

    let storage = build_sparse_max(rows_v, cols_v, vals_v, n, n);
    let names: Vec<String> = recs.iter().map(|r| r.id.clone()).collect();
    DataMatrix {
        rows: names.clone(),
        cols: names,
        row_lengths: None,
        col_lengths: None,
        data_type: "score".to_string(),
        storage,
    }
}

/// `records` with only the first of the records sharing an id, warning about the rest
/// (upstream `unique_records`).
pub fn unique_records(records: Vec<SeqRecord>) -> Vec<SeqRecord> {
    let mut seen: HashSet<String> = HashSet::new();
    records
        .into_iter()
        .filter(|rec| {
            let first = seen.insert(rec.id.clone());
            if !first {
                crate::log::warn(format_args!("Duplicate contig names !!! :{}", rec.id));
            }
            first
        })
        .collect()
}

/// The combined JI/AI similarity matrix of `records`. Of records sharing an id only
/// the first is scored ([`unique_records`]).
pub fn compare_contigs(
    records: &[SeqRecord],
    opts: &CompareContigsOptions,
) -> Result<DataMatrix, String> {
    if opts.ji_weight < 0.0 || opts.ai_weight < 0.0 {
        return Err("Metric weights (--ji, --ai) must be non-negative.".into());
    }
    if opts.ji_weight == 0.0 && opts.ai_weight == 0.0 {
        return Err("Please specify a non-zero weight for at least one metric.".into());
    }
    let mut ids = HashSet::new();
    let deduplicated;
    let recs: &[SeqRecord] = if records.iter().all(|r| ids.insert(r.id.as_str())) {
        records
    } else {
        deduplicated = unique_records(records.to_vec());
        &deduplicated
    };

    // A cap of 0 keeps nothing, not even the diagonal (upstream's early return).
    if opts.max_hits_per_query == Some(0) {
        let names: Vec<String> = recs.iter().map(|r| r.id.clone()).collect();
        return Ok(DataMatrix {
            rows: names.clone(),
            cols: names,
            row_lengths: None,
            col_lengths: None,
            data_type: "score".to_string(),
            storage: Storage::Sparse {
                values: vec![],
                indices: vec![],
                indptr: vec![0; recs.len() + 1],
            },
        });
    }

    let matrix = combined_matrix(recs, opts, opts.max_hits_per_query);
    if opts.mst || opts.knn.is_some() {
        return Ok(apply_mst_knn(
            &matrix,
            opts.knn.unwrap_or(0),
            opts.threshold,
            opts.mst,
            TreeTies::Streaming,
        ));
    }
    Ok(matrix)
}

/// How the leaves of the clustering tree are ordered for `--genbank_out`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LeafOrdering {
    /// Flip subtrees so the most similar contigs meet at each junction.
    #[default]
    Greedy,
    /// scipy's `optimal_leaf_ordering`; slow beyond a few thousand contigs.
    Optimal,
    /// Plain dendrogram order.
    None,
}

impl std::str::FromStr for LeafOrdering {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "greedy" => Ok(LeafOrdering::Greedy),
            "optimal" => Ok(LeafOrdering::Optimal),
            "none" => Ok(LeafOrdering::None),
            _ => Err(format!(
                "Unknown leaf ordering {s:?}, expected one of greedy, optimal, none"
            )),
        }
    }
}

/// Beyond this many contigs, `--leaf_ordering optimal` warns that it is slow.
const OPTIMAL_LEAF_ORDERING_WARN_CONTIGS: usize = 5000;

/// Peak memory of sorting `n` contigs: the condensed distances plus the copy the
/// linkage makes (upstream `estimate_sort_memory_bytes`).
pub fn estimate_sort_memory_bytes(n: usize) -> u64 {
    (n * n.saturating_sub(1) / 2) as u64 * 8 * 2
}

/// Fail before scoring when sorting `n` contigs for `--genbank_out` would need more
/// memory than is available (unless `check_memory` is off), and warn when the leaf
/// ordering will be very slow (upstream `_check_sort_feasible`).
pub fn check_sort_feasible(
    n: usize,
    ordering: LeafOrdering,
    check_memory: bool,
) -> Result<(), String> {
    use crate::tools::output_guardrails::format_bytes;
    if check_memory {
        let needed = estimate_sort_memory_bytes(n);
        let mut system = sysinfo::System::new();
        system.refresh_memory();
        let available = system.available_memory();
        if needed > available {
            return Err(format!(
                "Sorting {n} contigs for --genbank_out needs about {} of memory, but only \
                 {} is available. To avoid this, drop --genbank_out and write only a matrix \
                 with -o/--hdf_out, sort a subset of contigs with --contigs or \
                 --contigs_file, or deduplicate the input first. Use \
                 --skip_sort_memory_check to try anyway.",
                format_bytes(needed),
                format_bytes(available)
            ));
        }
    }
    if ordering == LeafOrdering::Optimal && n > OPTIMAL_LEAF_ORDERING_WARN_CONTIGS {
        crate::log::warn(format_args!(
            "--leaf_ordering optimal on {n} contigs may take many minutes to hours. Consider \
             --leaf_ordering greedy, which is much faster and usually about as good."
        ));
    }
    Ok(())
}

/// The stored entries of `m` as `(row, col, value)`; a dense matrix stores its
/// nonzeros, as scipy's `csr_array` of it does.
fn stored_entries(m: &DataMatrix) -> Vec<(usize, usize, f64)> {
    let ncols = m.ncols();
    match &m.storage {
        Storage::Dense(values) => values
            .iter()
            .enumerate()
            .filter(|(_, &v)| v != 0.0)
            .map(|(i, &v)| (i / ncols, i % ncols, v))
            .collect(),
        Storage::Sparse {
            values,
            indices,
            indptr,
        } => (0..m.nrows())
            .flat_map(|row| {
                let (lo, hi) = (csr_index(indptr[row]), csr_index(indptr[row + 1]));
                (lo..hi).map(move |e| (row, csr_index(indices[e]), values[e]))
            })
            .collect(),
    }
}

/// The order to write the rows of the square score matrix `scores` in, so that
/// similar contigs are adjacent (upstream `_hierarchical_order`): hierarchical
/// clustering on the distance `1 - score / max(score)`, clipped to `[0, 1]`, where a
/// pair with no score is 1 and an asymmetric pair takes its smaller distance.
pub fn hierarchical_order(
    scores: &DataMatrix,
    method: Method,
    ordering: LeafOrdering,
) -> Result<Vec<usize>, String> {
    let n = scores.nrows();
    let identity: Vec<usize> = (0..n).collect();
    if n <= 2 {
        return Ok(identity);
    }
    let entries = stored_entries(scores);
    let Some(min_score) = entries.iter().map(|e| e.2).reduce(f64::min) else {
        return Ok(identity);
    };
    if min_score < 0.0 {
        return Err(format!(
            "compare_contigs scores must be non-negative to sort contigs, but the minimum \
             score is {min_score:?}."
        ));
    }
    let max_score = entries
        .iter()
        .map(|e| e.2)
        .fold(f64::NEG_INFINITY, f64::max);
    if max_score <= 0.0 {
        return Ok(identity);
    }
    let mut condensed = vec![1.0f64; n * (n - 1) / 2];
    for &(row, col, value) in &entries {
        if row != col {
            let cell = &mut condensed[condensed_index(n, row, col)];
            *cell = cell.min(1.0 - value / max_score);
        }
    }
    for d in &mut condensed {
        *d = d.clamp(0.0, 1.0);
    }
    if condensed.iter().all(|&d| d == 0.0) {
        return Ok(identity);
    }
    let z = linkage(&condensed, method);
    Ok(match ordering {
        LeafOrdering::Optimal => leaves_list(&optimal_leaf_ordering(&z, &condensed)),
        LeafOrdering::Greedy => greedy_leaf_order(&z, &condensed, n),
        LeafOrdering::None => leaves_list(&z),
    })
}

/// Order the leaves of the dendrogram `z` by flipping subtrees: at each merge the two
/// children are oriented so the leaves meeting at the junction are as close as
/// possible, ties going to the earliest option (Gruvaeus and Wainer, 1972; upstream
/// `_greedy_leaf_order`).
fn greedy_leaf_order(z: &[Merge], condensed: &[f64], n: usize) -> Vec<usize> {
    let dist = |i: usize, j: usize| condensed[condensed_index(n, i, j)];
    let mut clusters: Vec<Vec<usize>> = (0..n).map(|i| vec![i]).collect();
    clusters.resize(2 * n - 1, Vec::new());
    for (k, merge) in z.iter().enumerate() {
        let mut a = std::mem::take(&mut clusters[merge.left]);
        let mut b = std::mem::take(&mut clusters[merge.right]);
        let (a0, a1, b0, b1) = (a[0], a[a.len() - 1], b[0], b[b.len() - 1]);
        // a+b, a+rev(b), rev(a)+b, rev(a)+rev(b)
        let options = [dist(a1, b0), dist(a1, b1), dist(a0, b0), dist(a0, b1)];
        let best = (1..4).fold(
            0,
            |best, i| if options[i] < options[best] { i } else { best },
        );
        if best >= 2 {
            a.reverse();
        }
        if best % 2 == 1 {
            b.reverse();
        }
        a.extend(b);
        clusters[n + k] = a;
    }
    std::mem::take(&mut clusters[2 * n - 2])
}

/// `records` in `order`, each renamed with a zero-padded position prefix if
/// `name_by_order` (upstream `write_sorted_genbank`).
pub fn sorted_records(
    records: &[SeqRecord],
    order: &[usize],
    name_by_order: bool,
) -> Vec<SeqRecord> {
    let digits = order.len().to_string().len();
    order
        .iter()
        .enumerate()
        .map(|(position, &i)| {
            let mut rec = records[i].clone();
            if name_by_order {
                rec.id = format!("{position:0digits$}_{}", rec.id);
            }
            rec
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seq::{Annotations, Location, SeqFeature, Topology};

    #[test]
    fn intersection_size_of_sorted_vecs() {
        assert_eq!(intersection_size(&[1, 2, 3], &[2, 3, 4]), 2);
        assert_eq!(intersection_size(&[1, 5, 9], &[2, 6, 10]), 0);
        assert_eq!(intersection_size(&[], &[1, 2]), 0);
    }

    fn contig(id: &str, domains: &[(&str, i64)]) -> SeqRecord {
        let features = domains
            .iter()
            .map(|(name, start)| {
                let mut f =
                    SeqFeature::new(DOMAIN_FEATURE_NAME, Location::range(*start, *start + 30, 1));
                f.set("name", *name);
                f.set("database", "DB");
                f
            })
            .collect();
        SeqRecord {
            id: id.into(),
            name: id.into(),
            description: String::new(),
            seq: vec![b'M'; 300],
            molecule_type: Some("protein".into()),
            topology: Topology::Linear,
            annotations: Annotations::default(),
            features,
        }
    }

    #[test]
    fn jaccard_only_combined_matrix() {
        // A={X,Y}, B={Y,Z}: shared 1, union 3 -> JI = 1/3.
        let recs = vec![
            contig("A", &[("X", 10), ("Y", 50)]),
            contig("B", &[("Y", 10), ("Z", 50)]),
        ];
        let opts = CompareContigsOptions {
            ji_weight: 1.0,
            ai_weight: 0.0,
            ..Default::default()
        };
        let d = combined_matrix(&recs, &opts, None).densify();
        // [A-A, A-B, B-A, B-B]
        assert!((d[0] - 1.0).abs() < 1e-9); // diagonal = sum of weights = 1.0
        assert!((d[1] - 1.0 / 3.0).abs() < 1e-9);
        assert!((d[2] - 1.0 / 3.0).abs() < 1e-9); // symmetric
        assert!((d[3] - 1.0).abs() < 1e-9);
    }

    #[test]
    fn adjacency_uses_consecutive_pairs() {
        // A: X->Y->Z, B: X->Y->W. Adjacency sets {XY,YZ} vs {XY,YW}: shared 1, union 3.
        let recs = vec![
            contig("A", &[("X", 10), ("Y", 50), ("Z", 90)]),
            contig("B", &[("X", 10), ("Y", 50), ("W", 90)]),
        ];
        let opts = CompareContigsOptions {
            ji_weight: 0.0,
            ai_weight: 1.0,
            ..Default::default()
        };
        let d = combined_matrix(&recs, &opts, None).densify();
        assert!((d[1] - 1.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn no_shared_domains_gives_no_offdiagonal_edge() {
        let recs = vec![contig("A", &[("X", 10)]), contig("B", &[("Z", 10)])];
        let opts = CompareContigsOptions {
            ji_weight: 0.5,
            ai_weight: 0.5,
            ..Default::default()
        };
        let d = combined_matrix(&recs, &opts, None).densify();
        assert_eq!(d[1], 0.0);
        assert_eq!(d[2], 0.0);
        assert!((d[0] - 1.0).abs() < 1e-9); // diagonal still 1.0 (0.5+0.5)
    }

    /// A dense score matrix: 1 within a block, 0.1 between blocks.
    fn block_scores(block_ids: &[usize]) -> DataMatrix {
        let n = block_ids.len();
        let mut values = vec![0.0; n * n];
        for i in 0..n {
            for j in 0..n {
                values[i * n + j] = if block_ids[i] == block_ids[j] {
                    1.0
                } else {
                    0.1
                };
            }
        }
        let names: Vec<String> = (0..n).map(|i| i.to_string()).collect();
        DataMatrix {
            rows: names.clone(),
            cols: names,
            row_lengths: None,
            col_lengths: None,
            data_type: "score".into(),
            storage: Storage::Dense(values),
        }
    }

    /// Upstream `test_hierarchical_order_keeps_blocks_contiguous`.
    #[test]
    fn hierarchical_order_keeps_blocks_contiguous() {
        let block_ids = [0, 1, 2, 0, 1, 2, 0, 1, 2, 0, 1, 2];
        for method in ["average", "ward", "complete", "single", "weighted"] {
            for ordering in [
                LeafOrdering::Greedy,
                LeafOrdering::Optimal,
                LeafOrdering::None,
            ] {
                let order = hierarchical_order(
                    &block_scores(&block_ids),
                    method.parse().unwrap(),
                    ordering,
                )
                .unwrap();
                let mut sorted = order.clone();
                sorted.sort_unstable();
                assert_eq!(sorted, (0..block_ids.len()).collect::<Vec<_>>());
                let blocks: Vec<usize> = order.iter().map(|&i| block_ids[i]).collect();
                let changes = blocks.windows(2).filter(|w| w[0] != w[1]).count();
                assert_eq!(changes, 2, "{method} {ordering:?}: {blocks:?}");
            }
        }
    }

    #[test]
    fn hierarchical_order_keeps_input_order_without_distances() {
        let order =
            |m: &DataMatrix| hierarchical_order(m, Method::Average, LeafOrdering::Greedy).unwrap();
        assert_eq!(order(&block_scores(&[0, 1])), [0, 1]);
        // Every score equal: every distance 0.
        assert_eq!(order(&block_scores(&[0, 0, 0, 0])), [0, 1, 2, 3]);
        let mut zero = block_scores(&[0, 1, 2]);
        zero.storage = Storage::Dense(vec![0.0; 9]);
        assert_eq!(order(&zero), [0, 1, 2]);
    }

    #[test]
    fn hierarchical_order_rejects_negative_scores() {
        let mut m = block_scores(&[0, 0, 1, 1]);
        if let Storage::Dense(values) = &mut m.storage {
            values[2] = -0.1;
            values[8] = -0.1;
        }
        let err = hierarchical_order(&m, Method::Average, LeafOrdering::Greedy).unwrap_err();
        assert!(err.contains("non-negative"), "{err}");
    }

    #[test]
    fn sorted_records_prefix_their_position() {
        let recs = vec![contig("a", &[]), contig("b", &[])];
        let ids: Vec<String> = sorted_records(&recs, &[1, 0], true)
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(ids, ["0_b", "1_a"]);
        assert_eq!(estimate_sort_memory_bytes(0), 0);
        assert_eq!(estimate_sort_memory_bytes(3), 48);
        assert!(check_sort_feasible(3, LeafOrdering::Greedy, true).is_ok());
    }
}
