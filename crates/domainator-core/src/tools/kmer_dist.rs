//! `kmer_dist`: similarity between sequence records from exact k-mer sets (upstream
//! `kmer_dist.py`). No hashing or sketching: two records share a k-mer only if they
//! share the exact word, so the scores are exact and so is parity.
//!
//! Protein records use protein k-mers. Nucleotide records use the k-mers of their CDS
//! translations (`--gene_call` to call genes first), or, with `--nucleotide_kmers`,
//! strand-invariant nucleotide k-mers: each k-mer and its reverse complement count as
//! one, the smaller of the two in alphabet order.
//!
//! Letters follow upstream's rules: anything that is not alphanumeric (`*`, `-`, spaces)
//! is dropped without breaking a run, while an alphanumeric letter outside the alphabet
//! (`X`, `B`, a digit) ends the run, so no k-mer spans it.

use std::collections::{HashMap, HashSet};
use std::io;
use std::str::FromStr;

use rayon::prelude::*;

use crate::io::{fasta, genbank::read_genbank, FileType};
use crate::matrix::mst_knn::{apply_mst_knn, sparsify_option_label, TreeTies};
use crate::matrix::transform::apply_mode;
use crate::matrix::{build_sparse_max, DataMatrix};
use crate::progress::Progress;
use crate::seq::SeqRecord;
use crate::tools::domainate::{clean_rec_nucleotide, MAX_PROTEIN_SIZE};
use crate::tools::gene_call::{call_genes, GeneCall, GeneCallScope};

/// `--ksize` for protein and translated-CDS k-mers when none is given.
pub const DEFAULT_PROTEIN_KSIZE: usize = 5;
/// `--ksize` for `--nucleotide_kmers` when none is given.
pub const DEFAULT_NUCLEOTIDE_KSIZE: usize = 15;
/// The `--mode`s `--mst`/`--knn` accept, in the sorted order upstream's error prints.
pub const SPARSIFY_MODES: [&str; 2] = ["bool", "score"];

const PROTEIN_ALPHABET: &[u8; 20] = b"ACDEFGHIKLMNPQRSTVWY";
const NUCLEOTIDE_ALPHABET: &[u8; 15] = b"ACGTRYMKBDHVNSW";
const NUCLEOTIDE_COMPLEMENTS: &[u8; 15] = b"TGCAYRKMVHDBNSW";
/// A letter outside the alphabet: it ends the current run.
const BREAK: u8 = u8::MAX;

/// `--metric`: how shared k-mers become a similarity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Metric {
    /// Shared / the smaller set.
    #[default]
    MaxContainment,
    /// Shared / the larger set.
    MinContainment,
    /// Shared / the union.
    Jaccard,
}

impl FromStr for Metric {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "max_containment" => Ok(Metric::MaxContainment),
            "min_containment" => Ok(Metric::MinContainment),
            "jaccard" => Ok(Metric::Jaccard),
            other => Err(format!("Unsupported metric: {other}")),
        }
    }
}

impl Metric {
    /// `shared / denominator`, 0 when the denominator is (upstream `_score_shared_counts`).
    fn score(self, shared: u32, query: usize, reference: usize) -> f64 {
        let (shared, query, reference) = (f64::from(shared), query as f64, reference as f64);
        let denominator = match self {
            Metric::Jaccard => query + reference - shared,
            Metric::MaxContainment => query.min(reference),
            Metric::MinContainment => query.max(reference),
        };
        if denominator > 0.0 {
            shared / denominator
        } else {
            0.0
        }
    }
}

/// Options for [`kmer_dist`], named like the command's.
#[derive(Clone, Debug)]
pub struct KmerDistOptions {
    /// `--max_hits_per_query`: keep each row's highest values only (`Some(0)` keeps none).
    pub max_hits_per_query: Option<usize>,
    pub metric: Metric,
    /// `--mode`: `score`, `bool` or `score_dist`.
    pub mode: String,
    /// Similarities `<=` this are dropped before the mode transform (`0` keeps all).
    pub threshold: f64,
    /// `--alphabet`: the molecule type of FASTA records.
    pub alphabet: String,
    /// `--ksize`; `None` picks [`DEFAULT_PROTEIN_KSIZE`] or [`DEFAULT_NUCLEOTIDE_KSIZE`].
    pub ksize: Option<usize>,
    pub nucleotide_kmers: bool,
    pub gene_call: Option<GeneCall>,
    pub mst: bool,
    pub knn: Option<usize>,
    /// Worker threads (`0` = one per physical core).
    pub cpu: usize,
    /// Draw bars for reading and scoring (`--progress`).
    pub progress: bool,
}

impl Default for KmerDistOptions {
    fn default() -> Self {
        KmerDistOptions {
            max_hits_per_query: None,
            metric: Metric::default(),
            mode: "score".to_string(),
            threshold: 0.0,
            alphabet: "protein".to_string(),
            ksize: None,
            nucleotide_kmers: false,
            gene_call: None,
            mst: false,
            knn: None,
            cpu: 0,
            progress: false,
        }
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

/// Compare the k-mer content of every `input` record with every `reference` record:
/// a matrix of `--mode` values with a row per input and a column per reference.
/// `reference` `None` compares the input with itself. A format of `None` is read off
/// the file name.
pub fn kmer_dist(
    input: &str,
    input_format: Option<FileType>,
    reference: Option<&str>,
    reference_format: Option<FileType>,
    opts: &KmerDistOptions,
) -> io::Result<DataMatrix> {
    let input_format = resolve_file_type(input, input_format, "Input")?;
    let (reference, reference_format) = match reference {
        None => (input, input_format),
        Some(path) => (
            path,
            resolve_file_type(path, reference_format, "Reference")?,
        ),
    };
    let self_comparison = input == reference && input_format == reference_format;

    let mut query_records = read_records(input, input_format, &opts.alphabet)?;
    let mut reference_records = if self_comparison {
        Vec::new()
    } else {
        read_records(reference, reference_format, &opts.alphabet)?
    };
    let input_molecule = molecule_type(input, &query_records)?;
    let reference_molecule = if self_comparison {
        input_molecule.clone()
    } else {
        molecule_type(reference, &reference_records)?
    };
    if input_molecule != reference_molecule {
        return Err(invalid(
            "Input and reference must have the same molecule type.",
        ));
    }

    if opts.mst || opts.knn.is_some() {
        let option = sparsify_option_label(opts.mst, opts.knn);
        if !self_comparison {
            return Err(invalid(format!(
                "{option} requires a square, symmetric comparison (reference must be the same \
                 file as input)."
            )));
        }
        if !SPARSIFY_MODES.contains(&opts.mode.as_str()) {
            return Err(invalid(format!(
                "{option} is only supported with --mode ['{}']. For a distance matrix, \
                 sparsify with --mode score and convert the result with transform_matrix.",
                SPARSIFY_MODES.join("', '")
            )));
        }
    }

    let ksize = match opts.ksize {
        Some(0) => return Err(invalid("--ksize must be at least 1")),
        Some(k) => k,
        None if input_molecule.as_deref() == Some("protein") || !opts.nucleotide_kmers => {
            DEFAULT_PROTEIN_KSIZE
        }
        None => DEFAULT_NUCLEOTIDE_KSIZE,
    };

    let mut table = KmerTable::default();
    let query = represent(
        input,
        input_format,
        &mut query_records,
        &mut table,
        ksize,
        opts,
        "query",
    )?;
    drop(query_records);
    let reference = if self_comparison {
        None
    } else {
        let representation = represent(
            reference,
            reference_format,
            &mut reference_records,
            &mut table,
            ksize,
            opts,
            "reference",
        )?;
        drop(reference_records);
        Some(representation)
    };

    let feature_count = table.ids.len();
    let reference_rep = reference.as_ref().unwrap_or(&query);
    let (nrows, ncols) = (query.names.len(), reference_rep.names.len());
    let k = opts.max_hits_per_query;
    // Every metric is symmetric, so an uncapped self-comparison scores the upper
    // triangle only. With --mst/--knn those are the cells the spanning forest is built
    // from; otherwise they are mirrored.
    let upper_only = self_comparison && k.is_none();
    let entries: Vec<(usize, usize, f64)> = if k == Some(0) {
        Vec::new()
    } else if feature_count == 0 {
        // No k-mer anywhere: a self-comparison is the identity, as upstream returns it
        // before any cap or threshold.
        if self_comparison {
            (0..nrows).map(|i| (i, i, 1.0)).collect()
        } else {
            Vec::new()
        }
    } else {
        crate::parallel::with_threads(opts.cpu, || {
            score_rows(
                &query.kmers,
                &reference_rep.kmers,
                feature_count,
                opts,
                self_comparison,
                upper_only,
            )
        })
    };

    let sparsify = opts.mst || opts.knn.is_some();
    let (mut rows_v, mut cols_v, mut vals_v) = (Vec::new(), Vec::new(), Vec::new());
    for &(r, c, v) in &entries {
        rows_v.push(r as i64);
        cols_v.push(c as i64);
        vals_v.push(v);
        if upper_only && !sparsify && r != c {
            rows_v.push(c as i64);
            cols_v.push(r as i64);
            vals_v.push(v);
        }
    }
    let mut matrix = DataMatrix {
        rows: query.names.clone(),
        cols: reference_rep.names.clone(),
        row_lengths: Some(query.lengths.clone()),
        col_lengths: Some(reference_rep.lengths.clone()),
        data_type: opts.mode.clone(),
        storage: build_sparse_max(rows_v, cols_v, vals_v, nrows, ncols),
    };
    if sparsify {
        // The threshold was applied while scoring, so nothing below it is left to
        // select from.
        matrix = apply_mst_knn(
            &matrix,
            opts.knn.unwrap_or(0),
            0.0,
            opts.mst,
            TreeTies::Entries,
        );
    }
    apply_mode(&mut matrix, &opts.mode);
    Ok(matrix)
}

/// The format of `path`: `format`, else its extension; only FASTA and GenBank qualify.
fn resolve_file_type(path: &str, format: Option<FileType>, label: &str) -> io::Result<FileType> {
    format
        .or_else(|| crate::io::detect_filetype(std::path::Path::new(path)))
        .ok_or_else(|| invalid(format!("{label} must be a fasta or genbank file.")))
}

fn read_records(path: &str, format: FileType, alphabet: &str) -> io::Result<Vec<SeqRecord>> {
    match format {
        FileType::Fasta => fasta::read_fasta(path, Some(alphabet)),
        FileType::Genbank => read_genbank(path),
    }
}

/// The molecule type of a file, read off its first record as upstream does (so an
/// empty file has none and fails the same way a mixed one would).
fn molecule_type(path: &str, records: &[SeqRecord]) -> io::Result<Option<String>> {
    match records.first() {
        Some(record) => Ok(record.molecule_type.clone()),
        None => Err(invalid(format!(
            "Mixed molecule types within a single input are not supported: {path}"
        ))),
    }
}

/// Every distinct k-mer seen so far, numbered in order of first appearance (upstream's
/// `feature_to_idx`, shared by the input and the reference).
#[derive(Default)]
struct KmerTable {
    ids: HashMap<Box<[u8]>, u32>,
}

impl KmerTable {
    fn id(&mut self, kmer: &[u8]) -> u32 {
        if let Some(&id) = self.ids.get(kmer) {
            return id;
        }
        let id = u32::try_from(self.ids.len()).expect("fewer than 2^32 distinct k-mers");
        self.ids.insert(kmer.into(), id);
        id
    }
}

/// A file's records as k-mer sets (upstream `SequenceRepresentation`).
struct Representation {
    names: Vec<String>,
    lengths: Vec<i64>,
    /// Each record's distinct k-mer ids, ascending.
    kmers: Vec<Vec<u32>>,
}

fn represent(
    path: &str,
    format: FileType,
    records: &mut [SeqRecord],
    table: &mut KmerTable,
    ksize: usize,
    opts: &KmerDistOptions,
    label: &str,
) -> io::Result<Representation> {
    if opts.alphabet == "nucleotide"
        && format == FileType::Fasta
        && !opts.nucleotide_kmers
        && opts.gene_call.is_none()
    {
        return Err(invalid(
            "Nucleotide FASTA input requires either --gene_call to create CDS annotations or \
             --nucleotide_kmers to compare whole-contig nucleotide k-mers.",
        ));
    }
    let mut seen = HashSet::new();
    if records
        .iter()
        .any(|record| !seen.insert(record.id.as_str()))
    {
        return Err(invalid(format!("Duplicate record ids found in {path}")));
    }
    if records.is_empty() {
        return Err(invalid(format!("No sequence records found in {path}")));
    }
    if !opts.nucleotide_kmers {
        prepare_translations(records, opts.gene_call.as_ref())?;
    }

    let bar = Progress::new(
        opts.progress,
        records.len(),
        &format!("Parsing {label} k-mers"),
    );
    let mut kmers = Vec::with_capacity(records.len());
    for record in records.iter() {
        let mut words = Words::default();
        if record.is_protein() {
            words.add_protein(&record.seq);
        } else if opts.nucleotide_kmers {
            words.set_nucleotide(&record.seq);
        } else {
            for (_, peptide) in record.cds_peptides(&HashSet::new()) {
                if peptide.len() > MAX_PROTEIN_SIZE {
                    crate::log::warn(format_args!(
                        "Skipping protein longer than {MAX_PROTEIN_SIZE} aa: {}",
                        record.id
                    ));
                } else {
                    words.add_protein(peptide.as_bytes());
                }
            }
        }
        let mut ids: Vec<u32> = words
            .kmers(ksize)
            .into_iter()
            .map(|kmer| table.id(kmer))
            .collect();
        ids.sort_unstable();
        kmers.push(ids);
        bar.inc(1);
    }
    Ok(Representation {
        names: records.iter().map(|r| r.id.clone()).collect(),
        lengths: records.iter().map(|r| r.len() as i64).collect(),
        kmers,
    })
}

/// Give every CDS of the nucleotide records its translation, calling genes first where
/// `gene_call` asks (upstream `_normalize_record_for_translated_kmers`).
fn prepare_translations(records: &mut [SeqRecord], gene_call: Option<&GeneCall>) -> io::Result<()> {
    let mut unannotated = Vec::new();
    for (i, record) in records.iter_mut().enumerate() {
        if record.is_protein() {
            continue;
        }
        if gene_call.is_some_and(|g| g.scope == GeneCallScope::All) {
            record.features.retain(|f| f.kind != "CDS");
        }
        if clean_rec_nucleotide(record) == 0 {
            unannotated.push(i);
        }
    }
    if let Some(gene_call) = gene_call {
        call_genes(records, &unannotated, &gene_call.options)?;
    }
    Ok(())
}

/// One record's letters as alphabet indices ([`BREAK`] for one outside the alphabet,
/// and between CDSs, so no k-mer spans two), and for nucleotides their reverse
/// complement, so that every k-mer is a slice of one or the other.
#[derive(Default)]
struct Words {
    forward: Vec<u8>,
    reverse: Option<Vec<u8>>,
}

impl Words {
    /// Add a protein sequence (upstream `_iter_protein_kmers`).
    fn add_protein(&mut self, sequence: &[u8]) {
        encode(sequence, PROTEIN_ALPHABET, false, &mut self.forward);
        self.forward.push(BREAK);
    }

    /// Set the nucleotide sequence, whose k-mers are strand-invariant (upstream
    /// `_iter_nucleotide_kmers`).
    fn set_nucleotide(&mut self, sequence: &[u8]) {
        encode(sequence, NUCLEOTIDE_ALPHABET, true, &mut self.forward);
        let complement = complement_indices();
        self.reverse = Some(
            self.forward
                .iter()
                .rev()
                .map(|&d| {
                    if d == BREAK {
                        BREAK
                    } else {
                        complement[usize::from(d)]
                    }
                })
                .collect(),
        );
    }

    /// The distinct k-mers. A nucleotide k-mer is the smaller of itself and its reverse
    /// complement, in alphabet order, which is how upstream's integer codes compare.
    fn kmers(&self, k: usize) -> HashSet<&[u8]> {
        let forward = &self.forward;
        let n = forward.len();
        let mut set = HashSet::new();
        for (run_start, run_end) in runs(forward) {
            if run_end - run_start < k {
                continue;
            }
            for s in run_start..=run_end - k {
                let word = &forward[s..s + k];
                set.insert(match &self.reverse {
                    Some(reverse) => word.min(&reverse[n - s - k..n - s]),
                    None => word,
                });
            }
        }
        set
    }
}

/// Each nucleotide letter's index mapped to its complement's.
fn complement_indices() -> [u8; 15] {
    let mut out = [0u8; 15];
    for (i, c) in NUCLEOTIDE_COMPLEMENTS.iter().enumerate() {
        out[i] = NUCLEOTIDE_ALPHABET
            .iter()
            .position(|a| a == c)
            .and_then(|p| u8::try_from(p).ok())
            .expect("every complement is in the alphabet");
    }
    out
}

/// Append the alphabet indices of `sequence`'s letters to `out` (upstream
/// `_clean_sequence_letters`): non-alphanumeric characters are dropped, letters
/// upper-cased (`U` read as `T` in nucleotides), and anything else becomes [`BREAK`].
fn encode(sequence: &[u8], alphabet: &[u8], nucleotide: bool, out: &mut Vec<u8>) {
    let index = |c: u8| -> u8 {
        let c = if nucleotide && c == b'U' { b'T' } else { c };
        alphabet
            .iter()
            .position(|&a| a == c)
            .and_then(|p| u8::try_from(p).ok())
            .unwrap_or(BREAK)
    };
    if sequence.is_ascii() {
        for &b in sequence {
            if b.is_ascii_alphanumeric() {
                out.push(index(b.to_ascii_uppercase()));
            }
        }
        return;
    }
    // Python's `str.isalnum`/`upper` are Unicode-aware; so is this slow path.
    for c in String::from_utf8_lossy(sequence).chars() {
        if c.is_alphanumeric() {
            for upper in c.to_uppercase() {
                out.push(u8::try_from(upper).map_or(BREAK, index));
            }
        }
    }
}

/// The `[start, end)` spans of `digits` without a [`BREAK`].
fn runs(digits: &[u8]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, &d) in digits.iter().enumerate() {
        if d == BREAK {
            if i > start {
                out.push((start, i));
            }
            start = i + 1;
        }
    }
    if digits.len() > start {
        out.push((start, digits.len()));
    }
    out
}

/// Score every query row against the references (upstream `_compute_metric_chunk`):
/// `(row, column, similarity)` cells, row by row, each row capped to its top
/// `max_hits_per_query` and pruned to values above the threshold. A self-comparison
/// puts 1 on the diagonal, which counts toward the cap; with `upper_only` it scores
/// only the columns after the row.
fn score_rows(
    query: &[Vec<u32>],
    reference: &[Vec<u32>],
    feature_count: usize,
    opts: &KmerDistOptions,
    self_comparison: bool,
    upper_only: bool,
) -> Vec<(usize, usize, f64)> {
    // Which references hold each k-mer, in reference order.
    let mut offsets = vec![0usize; feature_count + 1];
    for kmers in reference {
        for &id in kmers {
            offsets[id as usize + 1] += 1;
        }
    }
    for i in 0..feature_count {
        offsets[i + 1] += offsets[i];
    }
    let mut holders = vec![0u32; offsets[feature_count]];
    let mut fill = offsets.clone();
    for (r, kmers) in reference.iter().enumerate() {
        for &id in kmers {
            holders[fill[id as usize]] = u32::try_from(r).expect("fewer than 2^32 references");
            fill[id as usize] += 1;
        }
    }

    let bar = Progress::new(opts.progress, query.len(), "Scoring");
    let rows: Vec<Vec<(usize, f64)>> = query
        .par_iter()
        .enumerate()
        .map_init(
            || (vec![0u32; reference.len()], Vec::new()),
            |(shared, touched), (q, kmers)| {
                for &id in kmers {
                    let id = id as usize;
                    for &r in &holders[offsets[id]..offsets[id + 1]] {
                        let r = r as usize;
                        if self_comparison && (r == q || (upper_only && r < q)) {
                            continue;
                        }
                        if shared[r] == 0 {
                            touched.push(r);
                        }
                        shared[r] += 1;
                    }
                }
                touched.sort_unstable();
                let mut row: Vec<(usize, f64)> = Vec::with_capacity(touched.len() + 1);
                if self_comparison {
                    row.push((q, 1.0));
                }
                for &r in touched.iter() {
                    let score = opts
                        .metric
                        .score(shared[r], kmers.len(), reference[r].len());
                    if score > 0.0 {
                        row.push((r, score));
                    }
                    shared[r] = 0;
                }
                touched.clear();
                if let Some(k) = opts.max_hits_per_query {
                    if row.len() > k {
                        row.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
                        row.truncate(k);
                    }
                }
                if opts.threshold > 0.0 {
                    row.retain(|&(_, v)| v > opts.threshold);
                }
                bar.inc(1);
                row
            },
        )
        .collect();
    rows.into_iter()
        .enumerate()
        .flat_map(|(q, row)| row.into_iter().map(move |(c, v)| (q, c, v)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn protein_set(seq: &str, k: usize) -> Vec<String> {
        let mut words = Words::default();
        words.add_protein(seq.as_bytes());
        let mut out: Vec<String> = words
            .kmers(k)
            .into_iter()
            .map(|w| {
                w.iter()
                    .map(|&d| PROTEIN_ALPHABET[usize::from(d)] as char)
                    .collect()
            })
            .collect();
        out.sort();
        out
    }

    fn nucleotide_set(seq: &str, k: usize) -> Vec<String> {
        let mut words = Words::default();
        words.set_nucleotide(seq.as_bytes());
        let mut out: Vec<String> = words
            .kmers(k)
            .into_iter()
            .map(|w| {
                w.iter()
                    .map(|&d| NUCLEOTIDE_ALPHABET[usize::from(d)] as char)
                    .collect()
            })
            .collect();
        out.sort();
        out
    }

    /// `*` and `-` are dropped without breaking a run; `X` and digits break it.
    #[test]
    fn noncanonical_letters_follow_upstream() {
        assert_eq!(protein_set("AC-D*e", 2), ["AC", "CD", "DE"]);
        assert_eq!(protein_set("ACXDE", 2), ["AC", "DE"]);
        assert_eq!(protein_set("AC1DE", 2), ["AC", "DE"]);
        assert_eq!(protein_set("ACD", 4), Vec::<String>::new());
    }

    /// A k-mer and its reverse complement are one k-mer, the smaller in alphabet order
    /// (`ACGTRYMKBDHVNSW`, not ASCII), and `U` reads as `T`.
    #[test]
    fn nucleotide_kmers_are_canonical() {
        assert_eq!(nucleotide_set("AAC", 3), nucleotide_set("GTT", 3));
        assert_eq!(nucleotide_set("AAC", 3), ["AAC"]);
        assert_eq!(nucleotide_set("AAU", 3), nucleotide_set("ATT", 3));
        // "RA" is [4, 0] and its reverse complement "TY" is [3, 5], so "TY" is the
        // canonical one, though "RA" sorts first in ASCII.
        assert_eq!(nucleotide_set("RA", 2), ["TY"]);
        assert_eq!(nucleotide_set("TY", 2), ["TY"]);
    }

    /// Upstream's `ACXDE`/`ACYDE` pair (`noncanonical.faa`) at k = 2: `{AC, DE}` and
    /// `{AC, CY, YD, DE}` give a Jaccard of 0.5.
    #[test]
    fn scores_use_shared_counts() {
        let query = vec![vec![0, 1], vec![0, 1, 2, 3]];
        let opts = KmerDistOptions {
            metric: Metric::Jaccard,
            ..Default::default()
        };
        let cells = score_rows(&query, &query, 4, &opts, true, true);
        assert_eq!(cells, [(0, 0, 1.0), (0, 1, 0.5), (1, 1, 1.0)]);
        let opts = KmerDistOptions {
            metric: Metric::MaxContainment,
            max_hits_per_query: Some(1),
            ..Default::default()
        };
        // The diagonal counts toward the cap; of tied scores the lower column stays.
        let cells = score_rows(&query, &query, 4, &opts, true, false);
        assert_eq!(cells, [(0, 0, 1.0), (1, 0, 1.0)]);
    }
}
