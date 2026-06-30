//! `seq_dist`: compute a similarity matrix between input and reference
//! sequences/profiles. M3 scope: the `hmmer` algorithm — `hmmsearch` when the
//! reference is an HMM file, `phmmer` when it is a sequence file (including the
//! all-vs-all `input == reference` case that feeds the SSN). DIAMOND and
//! `hmmer_compare` are deferred with a clear error.

use std::collections::HashMap;
use std::path::Path;

use crate::io::{detect_filetype, fasta, genbank::read_genbank, FileType};
use crate::matrix::transform::apply_mode;
use crate::matrix::{build_sparse_max, DataMatrix};
use crate::search::hmm_compare::hmm_compare_scores;
use crate::search::seq_compare::{hmmsearch_scores, phmmer_scores, CmpResult};

/// What kind of file a `seq_dist` input/reference is.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    Fasta,
    Genbank,
    Hmm,
}

/// Detect input kind from a path (`.hmm` -> Hmm, else FASTA/GenBank by extension).
pub fn detect_kind(path: &Path) -> Option<InputKind> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    if name.ends_with(".hmm") {
        return Some(InputKind::Hmm);
    }
    match detect_filetype(path)? {
        FileType::Fasta => Some(InputKind::Fasta),
        FileType::Genbank => Some(InputKind::Genbank),
    }
}

/// Ordered name->index map with per-entry lengths (sequence length or HMM `M`).
struct NameIndex {
    idx: HashMap<String, usize>,
    names: Vec<String>,
    lens: Vec<i64>,
}

/// Clone a [`NameIndex`] (used for the symmetric `input == reference` case, where
/// the column axis is the same as the row axis).
fn clone_index(src: &NameIndex) -> NameIndex {
    NameIndex { idx: src.idx.clone(), names: src.names.clone(), lens: src.lens.clone() }
}

impl NameIndex {
    fn new() -> Self {
        NameIndex { idx: HashMap::new(), names: Vec::new(), lens: Vec::new() }
    }
    fn push(&mut self, name: String, len: i64) {
        if self.idx.contains_key(&name) {
            eprintln!("Warning: duplicate name found in seq_dist input: {name}");
            return;
        }
        self.idx.insert(name.clone(), self.names.len());
        self.names.push(name);
        self.lens.push(len);
    }
}

/// Loaded sequences: `(name, residues)` in file order, plus the name index.
struct Seqs {
    records: Vec<(String, Vec<u8>)>,
    index: NameIndex,
}

fn load_seqs(path: &str, kind: InputKind) -> std::io::Result<Seqs> {
    let recs = match kind {
        InputKind::Fasta => fasta::read_fasta(path, Some("protein"))?,
        InputKind::Genbank => read_genbank(path)?,
        InputKind::Hmm => unreachable!("load_seqs called for HMM input"),
    };
    let mut index = NameIndex::new();
    let mut records = Vec::new();
    for r in recs {
        index.push(r.id.clone(), r.seq.len() as i64);
        records.push((r.id, r.seq));
    }
    Ok(Seqs { records, index })
}

fn load_hmms(path: &str) -> std::io::Result<(Vec<hmmer_pure_rs::Hmm>, NameIndex)> {
    let hmms = hmmer_pure_rs::hmmfile::read_hmm_file(Path::new(path))
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{e:?}")))?;
    let mut index = NameIndex::new();
    for h in &hmms {
        index.push(h.name.clone(), h.m as i64);
    }
    Ok((hmms, index))
}

/// Options for [`seq_dist`].
pub struct SeqDistOptions {
    /// Matrix value type / `--mode` (`"score"`, `"bool"`, `"norm_score"`, ...).
    pub mode: String,
    /// Round raw scores `<= lb` down to zero (applies before any mode transform).
    pub lb: f64,
    /// Keep at most this many non-zero entries per input row (top-N by score).
    pub k: Option<usize>,
    /// Worker threads (`0` = all available cores).
    pub cpu: usize,
}

impl Default for SeqDistOptions {
    fn default() -> Self {
        SeqDistOptions { mode: "score".to_string(), lb: 0.0, k: None, cpu: 0 }
    }
}

/// Compute the similarity matrix. `input`/`reference` are paths; when `reference`
/// is `None` the input is used as its own reference (square symmetric matrix).
pub fn seq_dist(
    input: &str,
    input_kind: InputKind,
    reference: Option<&str>,
    reference_kind: Option<InputKind>,
    opts: &SeqDistOptions,
) -> std::io::Result<DataMatrix> {
    let ref_path = reference.unwrap_or(input);
    let ref_kind = reference_kind.unwrap_or(input_kind);
    let same = ref_path == input;

    // HMM input is only valid against an HMM reference (the hmmer_compare path).
    if input_kind == InputKind::Hmm && ref_kind != InputKind::Hmm {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "seq_dist: an HMM input requires an HMM reference (hmmer_compare)",
        ));
    }

    // Score and collect results plus the row (input) and column (reference) indices.
    // The scoring is parallelized over the `--cpu` thread pool.
    let (results, row_index, col_index): (Vec<CmpResult>, NameIndex, NameIndex) =
        crate::parallel::with_threads(opts.cpu, || -> std::io::Result<_> {
            if input_kind == InputKind::Hmm {
                // hmmer_compare: profile-vs-profile Viterbi alignment.
                let (q_hmms, row_index) = load_hmms(input)?;
                if same {
                    let results = hmm_compare_scores(&q_hmms, &q_hmms, opts.lb);
                    let col_index = clone_index(&row_index);
                    Ok((results, row_index, col_index))
                } else {
                    let (r_hmms, col_index) = load_hmms(ref_path)?;
                    let results = hmm_compare_scores(&q_hmms, &r_hmms, opts.lb);
                    Ok((results, row_index, col_index))
                }
            } else {
                let input_seqs = load_seqs(input, input_kind)?;
                if ref_kind == InputKind::Hmm {
                    let (hmms, col_index) = load_hmms(ref_path)?;
                    let results = hmmsearch_scores(&input_seqs.records, &hmms, opts.lb);
                    Ok((results, input_seqs.index, col_index))
                } else if same {
                    let col_index = clone_index(&input_seqs.index);
                    let results = phmmer_scores(&input_seqs.records, &input_seqs.records, opts.lb);
                    Ok((results, input_seqs.index, col_index))
                } else {
                    let ref_seqs = load_seqs(ref_path, ref_kind)?;
                    let results = phmmer_scores(&input_seqs.records, &ref_seqs.records, opts.lb);
                    Ok((results, input_seqs.index, ref_seqs.index))
                }
            }
        })?;

    let nrows = row_index.names.len();
    let ncols = col_index.names.len();
    let bool_mode = opts.mode == "bool";

    // Max-dedup per (row, col).
    let mut cells: HashMap<(i64, i64), f64> = HashMap::new();
    for r in &results {
        let (Some(&ri), Some(&ci)) = (row_index.idx.get(&r.query), col_index.idx.get(&r.subject))
        else {
            continue;
        };
        let v = if bool_mode { 1.0 } else { r.score };
        let key = (ri as i64, ci as i64);
        let e = cells.entry(key).or_insert(f64::NEG_INFINITY);
        if v > *e {
            *e = v;
        }
    }

    // Optional top-k per row (by score desc, then column asc — matches the
    // Python `(-score, reference)` ordering with column index standing in for
    // the reference name order).
    let mut rows_v: Vec<i64> = Vec::new();
    let mut cols_v: Vec<i64> = Vec::new();
    let mut vals_v: Vec<f64> = Vec::new();
    if let Some(k) = opts.k {
        let mut by_row: HashMap<i64, Vec<(i64, f64)>> = HashMap::new();
        for (&(ri, ci), &v) in &cells {
            by_row.entry(ri).or_default().push((ci, v));
        }
        for (ri, mut entries) in by_row {
            entries.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
            entries.truncate(k);
            for (ci, v) in entries {
                rows_v.push(ri);
                cols_v.push(ci);
                vals_v.push(v);
            }
        }
    } else {
        for (&(ri, ci), &v) in &cells {
            rows_v.push(ri);
            cols_v.push(ci);
            vals_v.push(v);
        }
    }

    let storage = build_sparse_max(rows_v, cols_v, vals_v, nrows, ncols);
    let mut matrix = DataMatrix {
        rows: row_index.names,
        cols: col_index.names,
        row_lengths: Some(row_index.lens),
        col_lengths: Some(col_index.lens),
        data_type: opts.mode.clone(),
        storage,
    };

    apply_mode(&mut matrix, &opts.mode);
    Ok(matrix)
}
