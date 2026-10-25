//! `compare_contigs`: pairwise gene-neighborhood similarity from a Domainator-
//! annotated GenBank, via the Jaccard (shared domain *content*) and Adjacency
//! (shared consecutive domain *pairs*) indices, combined by weight. Mirrors
//! `domainator.compare_contigs` (the matrix path).
//!
//! Scope: the JI/AI similarity matrix, including `--mst`/`--knn` (MST ∪
//! OR-symmetric kNN pruning). The hierarchically sorted `--genbank_out` is not
//! ported yet.

use std::collections::{HashMap, HashSet};

use crate::matrix::mst_knn::{apply_mst_knn, TreeTies};
use crate::matrix::{build_sparse_max, DataMatrix, Storage};
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
                entries
                    .into_iter()
                    .filter(|&(_, v)| !(opts.threshold > 0.0 && v <= opts.threshold))
                    .map(|(j, v)| (i as i64, j as i64, v))
                    .collect()
            })
            .collect()
    });

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

/// The combined JI/AI similarity matrix of `records`. Of records sharing an id only
/// the first is scored (upstream `unique_records`).
pub fn compare_contigs(
    records: Vec<SeqRecord>,
    opts: &CompareContigsOptions,
) -> Result<DataMatrix, String> {
    if opts.ji_weight < 0.0 || opts.ai_weight < 0.0 {
        return Err("Metric weights (--ji, --ai) must be non-negative.".into());
    }
    if opts.ji_weight == 0.0 && opts.ai_weight == 0.0 {
        return Err("Please specify a non-zero weight for at least one metric.".into());
    }
    let mut recs: Vec<SeqRecord> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for rec in records {
        if seen.insert(rec.id.clone()) {
            recs.push(rec);
        } else {
            eprintln!("Warning: Duplicate contig names !!! :{}", rec.id);
        }
    }

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

    let matrix = combined_matrix(&recs, opts, opts.max_hits_per_query);
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
}
