//! `structure_dist` (upstream `structure_dist.py`): a similarity or distance matrix
//! between protein structures, the structure counterpart of `seq_dist`.
//!
//! Every chain of every input is a row, hit or not, so the labels are exactly the
//! record ids `structure_to_genbank` writes. The input is the QUERY and the reference
//! the TARGET (the opposite of the other structure tools), as diamond runs in
//! `seq_dist`. The score-based `--mode`s behave as in `seq_dist`; the structural ones
//! (`tmscore`, `lddt`, `fident`) are already in [0, 1], and their `*_dist` forms are
//! `1 - value`.

use std::collections::HashMap;
use std::io;

use super::structure_lib::{
    prepare_databases, warn_on_saturation, Aligner, SearchRequest, StructureDb,
};
use crate::matrix::mst_knn::{apply_mst_knn, TreeTies};
use crate::matrix::transform::{apply_mode, symmetrize, LOG10_2};
use crate::matrix::{build_sparse_max, csr_index, csr_row, DataMatrix, Storage};
use crate::progress::Progress;

/// The score-based modes (upstream `transform_matrix.MODES`).
pub const BITSCORE_MODES: [&str; 7] = [
    "score",
    "bool",
    "norm_score",
    "row_norm_score",
    "score_dist",
    "efi_score",
    "efi_score_dist",
];

/// Every `--mode`, sorted as upstream offers them.
pub const ALL_MODES: [&str; 13] = [
    "bool",
    "efi_score",
    "efi_score_dist",
    "fident",
    "fident_dist",
    "lddt",
    "lddt_dist",
    "norm_score",
    "row_norm_score",
    "score",
    "score_dist",
    "tmscore",
    "tmscore_dist",
];

/// Modes whose cell is computable from one hit, so `--mst`/`--knn` can stream them.
pub const SPARSIFY_STREAMABLE_MODES: [&str; 6] =
    ["bool", "efi_score", "fident", "lddt", "score", "tmscore"];

/// foldseek's `alntmscore` overshoots slightly on a perfect self-alignment; only a
/// value this far outside [0, 1] is worth reporting.
const METRIC_CLAMP_TOLERANCE: f64 = 0.1;

/// Above this many reference entries, say so before an uncapped all-vs-all.
const SATURATION_PREFLIGHT_THRESHOLD: usize = 10_000;

/// Whether `mode` is one of the structural modes, `tmscore`..`fident_dist`.
pub fn is_structural_mode(mode: &str) -> bool {
    !BITSCORE_MODES.contains(&mode)
}

/// The hit field whose value fills the matrix for `mode`.
fn mode_value_field(mode: &str) -> &'static str {
    match mode {
        "tmscore" | "tmscore_dist" => "tmscore",
        "lddt" | "lddt_dist" => "lddt",
        "fident" | "fident_dist" => "fident",
        _ => "bits",
    }
}

/// The structural metrics to request from the backend for `mode`.
pub fn mode_metrics(mode: &str) -> Vec<String> {
    match mode {
        "tmscore" | "tmscore_dist" => vec!["tmscore".into()],
        "lddt" | "lddt_dist" => vec!["lddt".into()],
        _ => Vec::new(),
    }
}

/// Refuse a `--mode` the backend cannot produce, naming `--mode` rather than the
/// metric (upstream `reject_mode_on_backend`).
pub fn reject_mode_on_backend(aligner: &Aligner, mode: &str) -> io::Result<()> {
    let name = aligner.name();
    if matches!(mode, "efi_score" | "efi_score_dist") && name == "reseek" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "--mode {mode} is computed from an alignment bit score, which backend 'reseek' \
                 does not report (structure_lib records -log10(p-value) in its place, so the \
                 2^(-score) term would be meaningless). Use --algorithm foldseek, or a --mode \
                 that only needs the values to be ordered, such as score, bool, norm_score, \
                 row_norm_score or score_dist."
            ),
        ));
    }
    let supported = aligner.supports_metrics();
    let missing: Vec<String> = mode_metrics(mode)
        .into_iter()
        .filter(|m| !supported.contains(&m.as_str()))
        .collect();
    if !missing.is_empty() {
        let shown = if supported.is_empty() {
            "none".to_string()
        } else {
            let mut s: Vec<String> = supported.iter().map(|m| format!("'{m}'")).collect();
            s.sort();
            format!("[{}]", s.join(", "))
        };
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "--mode {mode} is computed from the {} metric, which backend '{name}' cannot \
                 produce (supported metrics: {shown}). Use --algorithm foldseek, or a --mode \
                 based on the alignment score (score, bool, norm_score, row_norm_score, \
                 score_dist) or on sequence identity (fident, fident_dist), all of which every \
                 backend supports.",
                missing.join("/")
            ),
        ));
    }
    Ok(())
}

/// `structure_dist`'s options, named like its command line.
#[derive(Debug, Clone)]
pub struct StructureDistOptions {
    pub mode: String,
    pub evalue: f64,
    pub max_hits_per_query: Option<usize>,
    pub threshold: f64,
    pub symmetrize: String,
    pub mst: bool,
    pub knn: Option<usize>,
    pub alignment_type: Option<i64>,
    pub max_seqs: Option<usize>,
    pub db_out: Option<String>,
    pub hits_tsv_out: Option<String>,
    pub progress: bool,
}

impl Default for StructureDistOptions {
    fn default() -> Self {
        StructureDistOptions {
            mode: "score".into(),
            evalue: 0.001,
            max_hits_per_query: None,
            threshold: 0.0,
            symmetrize: "none".into(),
            mst: false,
            knn: None,
            alignment_type: None,
            max_seqs: None,
            db_out: None,
            hits_tsv_out: None,
            progress: false,
        }
    }
}

/// A database's entries: name -> index, the names, and their lengths.
type Entries = (HashMap<String, usize>, Vec<String>, Vec<i64>);

/// Every entry of a database: `(name -> index, names, lengths)`. A duplicate name is
/// warned about and skipped.
fn enumerate_entries(aligner: &Aligner, db: &StructureDb, label: &str) -> io::Result<Entries> {
    let mut index = HashMap::new();
    let mut names = Vec::new();
    let mut lengths = Vec::new();
    for (name, sequence, _) in aligner.iter_sequences(db)? {
        if index.contains_key(&name) {
            crate::log::warn(format!(
                "Duplicate entry name '{name}' in the {label} database; keeping the first and \
                 skipping the rest."
            ));
            continue;
        }
        index.insert(name.clone(), names.len());
        names.push(name);
        lengths.push(sequence.chars().count() as i64);
    }
    if names.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("The {label} database contains no structures."),
        ));
    }
    Ok((index, names, lengths))
}

/// Reads one hit field, counting missing and far-out-of-range values to report once.
struct MetricReader {
    field: &'static str,
    bounded: bool,
    missing: usize,
    far_out_of_range: usize,
}

impl MetricReader {
    fn new(field: &'static str) -> MetricReader {
        MetricReader {
            field,
            bounded: field != "bits",
            missing: 0,
            far_out_of_range: 0,
        }
    }

    fn value(&mut self, hit: &super::structure_lib::StructureHit) -> Option<f64> {
        let Some(mut value) = hit.field(self.field) else {
            // foldseek writes a non-number when it cannot compute a metric for a pair.
            self.missing += 1;
            return None;
        };
        if self.bounded && !(0.0..=1.0).contains(&value) {
            if !(-METRIC_CLAMP_TOLERANCE..=1.0 + METRIC_CLAMP_TOLERANCE).contains(&value) {
                self.far_out_of_range += 1;
            }
            value = value.clamp(0.0, 1.0);
        }
        Some(value)
    }

    fn warn(&self) {
        if self.missing > 0 {
            crate::log::warn(format!(
                "The aligner reported no {} for {} alignment(s); those pairs are left as zero \
                 in the matrix.",
                self.field, self.missing
            ));
        }
        if self.far_out_of_range > 0 {
            crate::log::warn(format!(
                "{} {} value(s) fell well outside [0, 1] (more than {METRIC_CLAMP_TOLERANCE} \
                 beyond it) and were clamped.",
                self.far_out_of_range, self.field
            ));
        }
    }
}

/// Each row's `cap` highest-valued columns from a stream of cells (upstream
/// `sparsify.PerQueryTopK`): a repeated cell keeps its maximum, ties go to the lower
/// column.
struct PerQueryTopK {
    cap: usize,
    rows: Vec<HashMap<usize, f64>>,
}

impl PerQueryTopK {
    fn new(n: usize, cap: usize) -> PerQueryTopK {
        PerQueryTopK {
            cap,
            rows: vec![HashMap::new(); n],
        }
    }

    fn top(&self, hits: &HashMap<usize, f64>) -> Vec<(usize, f64)> {
        let mut ranked: Vec<(usize, f64)> = hits.iter().map(|(&c, &v)| (c, v)).collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        ranked.truncate(self.cap);
        ranked
    }

    fn add(&mut self, row: usize, col: usize, value: f64) {
        if self.cap == 0 {
            return;
        }
        let hits = &mut self.rows[row];
        let slot = hits.entry(col).or_insert(f64::NEG_INFINITY);
        if value > *slot {
            *slot = value;
        }
        // Exact: anything dropped is outranked by `cap` values that can only grow.
        if hits.len() > 2 * self.cap {
            let kept = self.top(&self.rows[row]);
            self.rows[row] = kept.into_iter().collect();
        }
    }

    fn edges(&self) -> Vec<(usize, usize, f64)> {
        let mut out = Vec::new();
        for (row, hits) in self.rows.iter().enumerate() {
            out.extend(self.top(hits).into_iter().map(|(c, v)| (row, c, v)));
        }
        out
    }
}

/// Keep each row's `k` highest values, ties to the lower column (upstream
/// `sparse_scores.keep_top_k_per_row`).
fn keep_top_k_per_row(m: &mut DataMatrix, k: usize) {
    let (n, ncols) = (m.nrows(), m.ncols());
    let mut rows_v = Vec::new();
    let mut cols_v = Vec::new();
    let mut vals_v = Vec::new();
    if let Storage::Sparse {
        values,
        indices,
        indptr,
    } = &m.storage
    {
        for r in 0..n {
            let mut row: Vec<(usize, f64)> = csr_row(indptr, r)
                .map(|i| (csr_index(indices[i]), values[i]))
                .collect();
            row.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
            row.truncate(k);
            for (c, v) in row {
                rows_v.push(r as i64);
                cols_v.push(c as i64);
                vals_v.push(v);
            }
        }
    }
    m.storage = build_sparse_max(rows_v, cols_v, vals_v, n, ncols);
}

/// Align every input structure against every reference structure; a matrix of `mode`
/// values, a row per input and a column per reference. `references` of `None` compares
/// the input against itself.
pub fn structure_dist(
    inputs: &[String],
    references: Option<&[String]>,
    aligner: &Aligner,
    opts: &StructureDistOptions,
) -> io::Result<DataMatrix> {
    let sparsify = opts.mst || opts.knn.is_some();
    let self_comparison = references.is_none();
    let metrics = mode_metrics(&opts.mode);
    let field = mode_value_field(&opts.mode);

    let prepared = prepare_databases(aligner, inputs, references, opts.db_out.as_deref())?;
    aligner.check_capabilities(
        opts.alignment_type,
        &metrics,
        &[&prepared.input_db, &prepared.reference_db],
    )?;
    let (row_index, row_names, row_lengths) =
        enumerate_entries(aligner, &prepared.input_db, "input")?;
    let (col_index, col_names, col_lengths) = if self_comparison {
        (row_index.clone(), row_names.clone(), row_lengths.clone())
    } else {
        enumerate_entries(aligner, &prepared.reference_db, "reference")?
    };

    // foldseek's own default caps hits at 1000 per query, which would silently truncate
    // a larger all-vs-all; the reference count is the smallest cap that cannot.
    let effective_max_seqs = opts.max_seqs.unwrap_or(col_names.len());
    if opts.max_seqs.is_none() && effective_max_seqs > SATURATION_PREFLIGHT_THRESHOLD {
        crate::log::info(format!(
            "Comparing {} structures against {}, with no --max_seqs cap so that no row is \
             truncated. If this is too slow, bound the search with --evalue, or cap it with \
             --max_seqs and accept that rows reaching the cap are incomplete.",
            row_names.len(),
            col_names.len()
        ));
    }
    let hits = aligner.search(
        &prepared.input_db,
        &prepared.reference_db,
        &SearchRequest {
            evalue: opts.evalue,
            max_seqs: Some(effective_max_seqs),
            want_tseq: false,
            alignment_type: opts.alignment_type,
            metrics: &metrics,
            sort_by_target: false,
        },
        prepared.work_dir.path(),
    )?;

    let mut reader = MetricReader::new(field);
    let mut hit_counts: HashMap<String, usize> = HashMap::new();
    let mut top_hits = opts
        .max_hits_per_query
        .filter(|_| sparsify)
        .map(|cap| PerQueryTopK::new(row_names.len(), cap));
    let (mut rows_v, mut cols_v, mut vals_v) = (Vec::new(), Vec::new(), Vec::new());
    let bar = Progress::counter(opts.progress, "Scoring pairs");
    let lookup = |index: &HashMap<String, usize>, name: &str| {
        index.get(name).copied().ok_or_else(|| {
            io::Error::other(format!(
                "The aligner reported a hit for '{name}', which is not in the database."
            ))
        })
    };
    for hit in hits {
        let hit = hit?;
        bar.inc(1);
        *hit_counts.entry(hit.query.clone()).or_default() += 1;
        let Some(value) = reader.value(&hit).filter(|&v| v > opts.threshold) else {
            continue;
        };
        let (r, c) = (
            lookup(&row_index, &hit.query)?,
            lookup(&col_index, &hit.target)?,
        );
        match top_hits.as_mut() {
            Some(top) => top.add(r, c, value),
            None => {
                rows_v.push(r);
                cols_v.push(c);
                vals_v.push(value);
            }
        }
    }
    drop(bar);
    if let Some(top) = &top_hits {
        for (r, c, v) in top.edges() {
            rows_v.push(r);
            cols_v.push(c);
            vals_v.push(v);
        }
    }
    reader.warn();
    // Only a cap the user asked for can be "saturated"; the derived default is the
    // column count, which a fully connected row reaches legitimately.
    warn_on_saturation(&hit_counts, opts.max_seqs, "input record");
    if let Some(path) = &opts.hits_tsv_out {
        aligner.copy_hits_tsv(prepared.work_dir.path(), path)?;
    }
    drop(prepared);

    let (nrows, ncols) = (row_names.len(), col_names.len());
    if sparsify && opts.mode == "efi_score" {
        // What --mst/--knn select on, per hit, since they see no maxima. A
        // self-comparison, so both lengths are row lengths.
        let logs: Vec<f64> = row_lengths.iter().map(|&l| (l as f64).log10()).collect();
        let mut kept = 0;
        for i in 0..vals_v.len() {
            let v = vals_v[i] * LOG10_2 - (logs[rows_v[i]] + logs[cols_v[i]]);
            if v > 0.0 {
                rows_v[kept] = rows_v[i];
                cols_v[kept] = cols_v[i];
                vals_v[kept] = v;
                kept += 1;
            }
        }
        rows_v.truncate(kept);
        cols_v.truncate(kept);
        vals_v.truncate(kept);
    }
    let to_i64 = |v: Vec<usize>| v.into_iter().map(|x| x as i64).collect::<Vec<i64>>();
    let mut matrix = DataMatrix {
        rows: row_names,
        cols: col_names,
        row_lengths: Some(row_lengths),
        col_lengths: Some(col_lengths),
        data_type: opts.mode.clone(),
        storage: build_sparse_max(to_i64(rows_v), to_i64(cols_v), vals_v, nrows, ncols),
    };

    if sparsify {
        // The streaming accumulator's forest, over exactly the cells it was fed.
        matrix = apply_mst_knn(
            &matrix,
            opts.knn.unwrap_or(0),
            0.0,
            opts.mst,
            TreeTies::Entries,
        );
        if opts.mode == "bool" {
            if let Storage::Sparse { values, .. } = &mut matrix.storage {
                values.iter_mut().for_each(|v| *v = 1.0);
            }
        }
        matrix.data_type = opts.mode.clone();
        return Ok(matrix);
    }

    symmetrize(&mut matrix, &opts.symmetrize).map_err(io::Error::other)?;
    if let Some(k) = opts.max_hits_per_query {
        keep_top_k_per_row(&mut matrix, k);
    }
    if BITSCORE_MODES.contains(&opts.mode.as_str()) {
        apply_mode(&mut matrix, &opts.mode);
    } else if opts.mode.ends_with("_dist") {
        // A structural distance: necessarily dense, since an absent pair is the maximum
        // distance rather than zero.
        let mut d = matrix.densify();
        d.iter_mut().for_each(|v| *v = 1.0 - *v);
        if self_comparison {
            // A structure is at distance 0 from itself, even when its self-hit fell
            // below --evalue.
            for i in 0..nrows.min(ncols) {
                d[i * ncols + i] = 0.0;
            }
        }
        matrix.storage = Storage::Dense(d);
    }
    matrix.data_type = opts.mode.clone();
    Ok(matrix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_tables_agree() {
        for mode in BITSCORE_MODES {
            assert!(ALL_MODES.contains(&mode), "{mode}");
            assert_eq!(mode_value_field(mode), "bits", "{mode}");
        }
        for mode in SPARSIFY_STREAMABLE_MODES {
            assert!(ALL_MODES.contains(&mode), "{mode}");
        }
        let mut sorted = ALL_MODES;
        sorted.sort_unstable();
        assert_eq!(sorted, ALL_MODES);
        for (mode, field) in [
            ("tmscore", "tmscore"),
            ("tmscore_dist", "tmscore"),
            ("lddt_dist", "lddt"),
            ("fident", "fident"),
            ("fident_dist", "fident"),
            ("score_dist", "bits"),
            ("efi_score_dist", "bits"),
        ] {
            assert_eq!(mode_value_field(mode), field, "{mode}");
        }
    }

    #[test]
    fn per_query_top_k_keeps_the_best_columns() {
        let mut top = PerQueryTopK::new(2, 2);
        for (c, v) in [
            (3, 5.0),
            (1, 5.0),
            (2, 9.0),
            (3, 7.0),
            (0, 1.0),
            (4, 2.0),
            (5, 3.0),
        ] {
            top.add(0, c, v);
        }
        top.add(1, 0, 1.0);
        assert_eq!(top.edges(), [(0, 2, 9.0), (0, 3, 7.0), (1, 0, 1.0)]);
    }
}
