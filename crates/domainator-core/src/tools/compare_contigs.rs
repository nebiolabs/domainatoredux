//! `compare_contigs`: pairwise gene-neighborhood similarity from a Domainator-
//! annotated GenBank, via the Jaccard (shared domain *content*) and Adjacency
//! (shared consecutive domain *pairs*) indices, combined by weight. Mirrors
//! `domainator.compare_contigs` (the matrix path).
//!
//! M4 scope: the JI/AI similarity matrix and sparse/dense/TSV reports, including
//! `--mst_knn` (MST ∪ OR-symmetric kNN pruning). The hierarchically-clustered
//! GenBank report (`-o`, scipy ward `optimal_ordering`) is deferred.

use std::collections::{HashMap, HashSet};

use crate::io::genbank::read_genbank;
use crate::matrix::mst_knn::apply_mst_knn;
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
    /// Keep at most this many non-zero off-diagonal entries per row (top-N).
    pub k: Option<usize>,
    /// Zero out final scores `<= lb`.
    pub lb: f64,
    /// Prune the output to MST ∪ OR-symmetric kNN edges (integer > 1). When set,
    /// `k` is ignored (the pruning sees the full candidate set). Mirrors
    /// `compare_contigs --mst_knn`.
    pub mst_knn: Option<usize>,
    /// Worker threads (`0` = all available cores).
    pub cpu: usize,
}

impl Default for CompareContigsOptions {
    fn default() -> Self {
        CompareContigsOptions { ji_weight: 0.5, ai_weight: 0.5, databases: None, k: None, lb: 0.0, mst_knn: None, cpu: 0 }
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
                        Some(dbs) => f.get_first("database").map(|db| dbs.contains(db)).unwrap_or(false),
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
fn combined_matrix(recs: &[SeqRecord], opts: &CompareContigsOptions, k: Option<usize>) -> DataMatrix {
    let n = recs.len();
    let mut metrics: Vec<(f64, Vec<Vec<u32>>)> = Vec::new();
    if opts.ji_weight != 0.0 {
        metrics.push((opts.ji_weight, feature_sets(recs, Metric::Jaccard, opts.databases.as_ref())));
    }
    if opts.ai_weight != 0.0 {
        metrics.push((opts.ai_weight, feature_sets(recs, Metric::Adjacency, opts.databases.as_ref())));
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
                    .filter(|&(_, v)| !(opts.lb > 0.0 && v <= opts.lb))
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

/// Load contigs from `genbanks` (in order, deduped by id) and compute the
/// combined JI/AI similarity matrix.
pub fn compare_contigs(
    genbanks: &[String],
    opts: &CompareContigsOptions,
) -> std::io::Result<DataMatrix> {
    let mut recs: Vec<SeqRecord> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for path in genbanks {
        for rec in read_genbank(path)? {
            if seen.insert(rec.id.clone()) {
                recs.push(rec);
            } else {
                eprintln!("Warning: duplicate contig name: {}", rec.id);
            }
        }
    }

    // k == 0 -> empty matrix (no self either, matching the Python early return).
    // `--mst_knn` ignores `k`, so this guard does not apply when it is set.
    if opts.mst_knn.is_none() && opts.k == Some(0) {
        let names: Vec<String> = recs.iter().map(|r| r.id.clone()).collect();
        return Ok(DataMatrix {
            rows: names.clone(),
            cols: names,
            row_lengths: None,
            col_lengths: None,
            data_type: "score".to_string(),
            storage: Storage::Sparse { values: vec![], indices: vec![], indptr: vec![0; recs.len() + 1] },
        });
    }

    // `--mst_knn` bypasses per-row top-k so the pruning sees the full candidate set.
    let k = if opts.mst_knn.is_some() { None } else { opts.k };
    let matrix = combined_matrix(&recs, opts, k);

    match opts.mst_knn {
        Some(mst_knn) => Ok(apply_mst_knn(&matrix, mst_knn, opts.lb)),
        None => Ok(matrix),
    }
}
