//! `seq_dist`: compute a similarity matrix between input and reference
//! sequences/profiles. Algorithms (`--algorithm`, mirroring the Python suite):
//! `diamond_*` (DIAMOND `blastp` at varying sensitivity, via the external
//! `diamond` binary — the default), `hmmer` (`hmmsearch` when the reference is an
//! HMM file, `phmmer` when it is a sequence file, including the all-vs-all
//! `input == reference` case that feeds the SSN), and `hmmer_compare`
//! (profile-vs-profile Viterbi; auto-selected for HMM-vs-HMM).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::io::{detect_filetype, fasta, genbank::read_genbank, FileType};
use crate::matrix::mst_knn::apply_mst_knn;
use crate::matrix::transform::{apply_mode, symmetrize, transform_values};
use crate::matrix::{build_sparse_max, DataMatrix};

/// `--mode` values whose post-transform value is computable per edge (no global
/// matrix maxima), and therefore compatible with `--mst`/`--knn`. Mirrors
/// `seq_dist.SPARSIFY_STREAMABLE_MODES`, in the sorted order its errors print.
pub const SPARSIFY_STREAMABLE_MODES: [&str; 3] = ["bool", "efi_score", "score"];
use crate::search::hmm_compare::hmm_compare_scores;
use crate::search::seq_compare::{hmmsearch_scores, phmmer_scores, CmpResult};

/// What kind of file a `seq_dist` input/reference is.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    Fasta,
    Genbank,
    Hmm,
}

/// Detect input kind from a path (`.hmm`, optionally gzipped, -> Hmm, else
/// FASTA/GenBank by extension).
pub fn detect_kind(path: &Path) -> Option<InputKind> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    let unwrapped = crate::io::COMPRESSION_EXTENSIONS
        .iter()
        .find_map(|ext| name.strip_suffix(&format!(".{ext}")))
        .unwrap_or(&name);
    if unwrapped.ends_with(".hmm") {
        return Some(InputKind::Hmm);
    }
    match detect_filetype(path)? {
        FileType::Fasta => Some(InputKind::Fasta),
        FileType::Genbank => Some(InputKind::Genbank),
    }
}

/// Scoring algorithm, selected by `--algorithm`. Mirrors the Python suite's
/// `diamond_*` / `hmmer` / `hmmer_compare` choices.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Algorithm {
    /// DIAMOND `blastp` at the given sensitivity (external `diamond` binary).
    Diamond(Sensitivity),
    /// pyhmmer-style `phmmer` (sequence reference) or `hmmsearch` (HMM reference).
    Hmmer,
    /// Profile-vs-profile Viterbi comparison (HMM input and reference).
    HmmerCompare,
}

/// DIAMOND sensitivity preset (the suffix on `diamond_*`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Sensitivity {
    Fast,
    Default,
    MidSensitive,
    Sensitive,
    MoreSensitive,
    VerySensitive,
    UltraSensitive,
}

/// Parse an `--algorithm` value into an [`Algorithm`].
pub fn parse_algorithm(s: &str) -> Result<Algorithm, String> {
    Ok(match s {
        "diamond_f" => Algorithm::Diamond(Sensitivity::Fast),
        "diamond_d" => Algorithm::Diamond(Sensitivity::Default),
        "diamond_mids" => Algorithm::Diamond(Sensitivity::MidSensitive),
        "diamond_s" => Algorithm::Diamond(Sensitivity::Sensitive),
        "diamond_mors" => Algorithm::Diamond(Sensitivity::MoreSensitive),
        "diamond_vs" => Algorithm::Diamond(Sensitivity::VerySensitive),
        "diamond_us" => Algorithm::Diamond(Sensitivity::UltraSensitive),
        "hmmer" => Algorithm::Hmmer,
        "hmmer_compare" => Algorithm::HmmerCompare,
        other => return Err(format!("unknown algorithm '{other}'")),
    })
}

/// DIAMOND CLI flag(s) for a sensitivity preset (`d`/default takes none). Mirrors
/// `seq_dist.diamond`'s `mode_to_arg`.
fn sensitivity_args(s: Sensitivity) -> &'static [&'static str] {
    match s {
        Sensitivity::Fast => &["--fast"],
        Sensitivity::Default => &[],
        Sensitivity::MidSensitive => &["--mid-sensitive"],
        Sensitivity::Sensitive => &["--sensitive"],
        Sensitivity::MoreSensitive => &["--more-sensitive"],
        Sensitivity::VerySensitive => &["--very-sensitive"],
        Sensitivity::UltraSensitive => &["--ultra-sensitive"],
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
    NameIndex {
        idx: src.idx.clone(),
        names: src.names.clone(),
        lens: src.lens.clone(),
    }
}

impl NameIndex {
    fn new() -> Self {
        NameIndex {
            idx: HashMap::new(),
            names: Vec::new(),
            lens: Vec::new(),
        }
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
        let name = r.id;
        index.push(name.clone(), r.seq.len() as i64);
        records.push((name, r.seq));
    }
    Ok(Seqs { records, index })
}

fn load_hmms(path: &str) -> std::io::Result<(Vec<hmmer_pure_rs::Hmm>, NameIndex)> {
    let hmms = crate::search::hmm_read::read_hmm_file(Path::new(path))
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{e:?}")))?;
    let mut index = NameIndex::new();
    for h in &hmms {
        index.push(h.name.clone(), h.m as i64);
    }
    Ok((hmms, index))
}

/// Options for [`seq_dist`].
pub struct SeqDistOptions {
    /// `--algorithm` (e.g. `"diamond_us"`, `"hmmer"`, `"hmmer_compare"`).
    pub algorithm: String,
    /// Matrix value type / `--mode` (`"score"`, `"bool"`, `"norm_score"`, ...).
    pub mode: String,
    /// `--threshold`: drop raw scores `<= threshold` (before any mode transform).
    pub threshold: f64,
    /// `--max_hits_per_query`: keep each input row's best hits only. Caps the search
    /// itself, before `--mst`/`--knn` and `--symmetrize`.
    pub max_hits_per_query: Option<usize>,
    /// `--mst`: keep the maximum spanning forest's edges (with `knn`, the union).
    /// Requires a square comparison (reference == input) and a streamable `--mode`.
    pub mst: bool,
    /// `--knn`: keep the OR-symmetric k-nearest-neighbor edges.
    pub knn: Option<usize>,
    /// `--symmetrize`: `none`, `max`, `min` or `mean` (see
    /// [`crate::matrix::transform::symmetrize`]).
    pub symmetrize: String,
    /// Extra `(flag, value)` options for `diamond blastp` (`--diamond_args`, from
    /// [`parse_diamond_args`]). A `None` value means a bare flag.
    pub diamond_args: Vec<(String, Option<String>)>,
    /// `--diamond_path`: the diamond executable, instead of locating it.
    pub diamond_path: Option<String>,
    /// Worker threads (`0` = all available cores).
    pub cpu: usize,
}

impl Default for SeqDistOptions {
    fn default() -> Self {
        SeqDistOptions {
            algorithm: "diamond_us".to_string(),
            mode: "score".to_string(),
            threshold: 0.0,
            max_hits_per_query: None,
            mst: false,
            knn: None,
            symmetrize: "none".to_string(),
            diamond_args: Vec::new(),
            diamond_path: None,
            cpu: 0,
        }
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

    let invalid = |msg: String| std::io::Error::new(std::io::ErrorKind::InvalidInput, msg);
    let sparsify = opts.mst || opts.knn.is_some();
    if sparsify {
        let option = sparsify_option_label(opts.mst, opts.knn);
        let mode = &opts.mode;
        // Python's rendering of the list, as upstream prints it.
        let modes = format!("['{}']", SPARSIFY_STREAMABLE_MODES.join("', '"));
        if !SPARSIFY_STREAMABLE_MODES.contains(&mode.as_str()) {
            return Err(invalid(format!(
                "{option} is only supported with --mode {modes} (modes that \
                 normalize against global matrix maxima cannot be streamed). For '{mode}', run \
                 'seq_dist --mode score --hdf_out out.hdf5' then 'transform_matrix -i out.hdf5 \
                 --mode {mode} {option} --hdf_out pruned.hdf5'."
            )));
        }
        if !same {
            return Err(invalid(format!(
                "{option} requires a square, symmetric comparison (reference must be the same \
                 file as input)."
            )));
        }
        if opts.symmetrize != "none" {
            return Err(invalid(format!(
                "{option} output is always max-symmetric, so --symmetrize {} cannot be applied \
                 on top of it.",
                opts.symmetrize
            )));
        }
    }
    if opts.symmetrize != "none" && !same {
        return Err(invalid(
            "--symmetrize requires a square, symmetric comparison (reference must be the same \
             file as input)."
                .into(),
        ));
    }

    // Resolve the algorithm. An HMM-vs-HMM comparison is always hmmer_compare
    // (mirrors the Python suite, which coerces it with a warning).
    let mut algorithm = parse_algorithm(&opts.algorithm)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    if input_kind == InputKind::Hmm
        && ref_kind == InputKind::Hmm
        && algorithm != Algorithm::HmmerCompare
    {
        eprintln!(
            "Warning: input and reference are both HMM files; using --algorithm hmmer_compare."
        );
        algorithm = Algorithm::HmmerCompare;
    }

    // Validate the algorithm against the input/reference kinds.
    match algorithm {
        Algorithm::Diamond(_) => {
            if input_kind == InputKind::Hmm || ref_kind == InputKind::Hmm {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "diamond algorithms require sequence (FASTA/GenBank) input and reference; \
                     for HMM files use --algorithm hmmer or hmmer_compare",
                ));
            }
        }
        Algorithm::Hmmer => {
            if input_kind == InputKind::Hmm {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "--algorithm hmmer requires a sequence (FASTA/GenBank) input; \
                     for HMM-vs-HMM use --algorithm hmmer_compare",
                ));
            }
        }
        Algorithm::HmmerCompare => {
            if input_kind != InputKind::Hmm || ref_kind != InputKind::Hmm {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "--algorithm hmmer_compare requires HMM input and reference",
                ));
            }
        }
    }

    // Score every (input, reference) pair and reduce to one max value per cell,
    // keyed by (row, col) index. The diamond path streams the subprocess output
    // straight into the cell map (no large intermediate table); the hmmer paths
    // score in a rayon `--cpu` pool.
    let (cells, row_index, col_index): (HashMap<(i64, i64), f64>, NameIndex, NameIndex) =
        match algorithm {
            Algorithm::Diamond(sensitivity) => run_diamond(
                input,
                input_kind,
                ref_path,
                ref_kind,
                same,
                sensitivity,
                opts,
            )?,
            _ => crate::parallel::with_threads(opts.cpu, || -> std::io::Result<_> {
                let (results, row_index, col_index): (Vec<CmpResult>, NameIndex, NameIndex) =
                    if algorithm == Algorithm::HmmerCompare {
                        // hmmer_compare: profile-vs-profile Viterbi alignment.
                        let (q_hmms, row_index) = load_hmms(input)?;
                        if same {
                            let results = hmm_compare_scores(&q_hmms, &q_hmms, opts.threshold);
                            let col_index = clone_index(&row_index);
                            (results, row_index, col_index)
                        } else {
                            let (r_hmms, col_index) = load_hmms(ref_path)?;
                            let results = hmm_compare_scores(&q_hmms, &r_hmms, opts.threshold);
                            (results, row_index, col_index)
                        }
                    } else {
                        let input_seqs = load_seqs(input, input_kind)?;
                        if ref_kind == InputKind::Hmm {
                            let (hmms, col_index) = load_hmms(ref_path)?;
                            let results =
                                hmmsearch_scores(&input_seqs.records, &hmms, opts.threshold);
                            (results, input_seqs.index, col_index)
                        } else if same {
                            let col_index = clone_index(&input_seqs.index);
                            let results = phmmer_scores(
                                &input_seqs.records,
                                &input_seqs.records,
                                opts.threshold,
                            );
                            (results, input_seqs.index, col_index)
                        } else {
                            let ref_seqs = load_seqs(ref_path, ref_kind)?;
                            let results = phmmer_scores(
                                &input_seqs.records,
                                &ref_seqs.records,
                                opts.threshold,
                            );
                            (results, input_seqs.index, ref_seqs.index)
                        }
                    };

                // Max-dedup per (row, col).
                let mut cells: HashMap<(i64, i64), f64> = HashMap::new();
                for r in &results {
                    let (Some(&ri), Some(&ci)) =
                        (row_index.idx.get(&r.query), col_index.idx.get(&r.subject))
                    else {
                        continue;
                    };
                    let v = r.score;
                    let e = cells
                        .entry((ri as i64, ci as i64))
                        .or_insert(f64::NEG_INFINITY);
                    if v > *e {
                        *e = v;
                    }
                }
                Ok((cells, row_index, col_index))
            })?,
        };

    let nrows = row_index.names.len();
    let ncols = col_index.names.len();

    // `--max_hits_per_query`: each row's best hits. Upstream keeps the largest
    // `(score, input, reference)` results on a heap, so of tied scores the
    // reference names that sort last stay. The diamond path already capped targets
    // per query via `--max-target-seqs`.
    let k = match algorithm {
        Algorithm::Diamond(_) => None,
        _ => opts.max_hits_per_query,
    };
    let mut rows_v: Vec<i64> = Vec::new();
    let mut cols_v: Vec<i64> = Vec::new();
    let mut vals_v: Vec<f64> = Vec::new();
    if let Some(k) = k {
        let mut by_row: HashMap<i64, Vec<(i64, f64)>> = HashMap::new();
        for (&(ri, ci), &v) in &cells {
            by_row.entry(ri).or_default().push((ci, v));
        }
        for (ri, mut entries) in by_row {
            let name = |c: i64| &col_index.names[usize::try_from(c).expect("a name index")];
            entries.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| name(b.0).cmp(name(a.0))));
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

    if !sparsify {
        // `bool` cells are 1 before `--symmetrize`, which reconciles raw values.
        if opts.mode == "bool" {
            transform_values(&mut matrix, "bool");
        }
        symmetrize(&mut matrix, &opts.symmetrize).map_err(invalid)?;
        apply_mode(&mut matrix, &opts.mode);
        return Ok(matrix);
    }
    // Prune to MST ∪ OR-symmetric kNN. The raw-score `--threshold` was already
    // applied during scoring, so the pruning runs at lower_bound=0 on the per-edge
    // values (matching the Python streaming path). `bool` selects on the raw scores
    // and keeps the selected cells as 1.
    if opts.mode != "bool" {
        apply_mode(&mut matrix, &opts.mode);
    }
    let include_mst = opts.mst;
    matrix = apply_mst_knn(&matrix, opts.knn.unwrap_or(0), 0.0, include_mst);
    if opts.mode == "bool" {
        apply_mode(&mut matrix, "bool");
    }
    Ok(matrix)
}

/// diamond options `seq_dist` sets itself. Overriding one through `--diamond_args` would
/// either break parsing of the result stream or silently contradict a `seq_dist` option,
/// so each is rejected with a pointer to the option that does control it.
pub const RESERVED_DIAMOND_PARAMS: [(&str, &str); 19] = [
    ("-q", "-i/--input"),
    ("--query", "-i/--input"),
    ("-d", "-r/--reference"),
    ("--db", "-r/--reference"),
    (
        "-f",
        "the fixed tabular output format required to parse the result stream",
    ),
    (
        "--outfmt",
        "the fixed tabular output format required to parse the result stream",
    ),
    ("-p", "--cpu"),
    ("--threads", "--cpu"),
    ("-k", "-k"),
    ("--max-target-seqs", "-k"),
    ("--max-hsps", "seq_dist (fixed at 1)"),
    ("--min-score", "--threshold"),
    ("--verbose", "seq_dist (required for progress reporting)"),
    ("--fast", "--algorithm"),
    ("--mid-sensitive", "--algorithm"),
    ("--sensitive", "--algorithm"),
    ("--more-sensitive", "--algorithm"),
    ("--very-sensitive", "--algorithm"),
    ("--ultra-sensitive", "--algorithm"),
];

/// Turn `--diamond_args`, already split like a shell command line, into ordered
/// `(flag, value)` pairs (upstream `parse_diamond_args`). A flag followed by another
/// flag, or by nothing, takes no value. A negative number is a value, not a flag.
pub fn parse_diamond_args(tokens: &[String]) -> Result<Vec<(String, Option<String>)>, String> {
    let is_flag = |t: &str| t.starts_with('-') && !is_number(t);
    let mut out = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let flag = &tokens[i];
        if !is_flag(flag) {
            return Err(format!(
                "--diamond_args must be diamond options starting with '-', but got: '{flag}'"
            ));
        }
        if let Some((_, owner)) = RESERVED_DIAMOND_PARAMS.iter().find(|(f, _)| f == flag) {
            return Err(format!(
                "--diamond_args may not set '{flag}', because seq_dist controls it via {owner}."
            ));
        }
        match tokens.get(i + 1) {
            Some(value) if !is_flag(value) => {
                out.push((flag.clone(), Some(value.clone())));
                i += 2;
            }
            _ => {
                out.push((flag.clone(), None));
                i += 1;
            }
        }
    }
    Ok(out)
}

/// Upstream's `_NUMBER_RE` (`-?\d+(\.\d*)?([eE][-+]?\d+)?`), matched whole.
fn is_number(t: &str) -> bool {
    let t = t.strip_prefix('-').unwrap_or(t);
    let (mantissa, exponent) = match t.find(['e', 'E']) {
        Some(i) => (&t[..i], Some(&t[i + 1..])),
        None => (t, None),
    };
    let (int, frac) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    digits(int)
        && (frac.is_empty() || frac.bytes().all(|b| b.is_ascii_digit()))
        && exponent.is_none_or(|e| digits(e.strip_prefix(['-', '+']).unwrap_or(e)))
}

/// Run DIAMOND `blastp` for the diamond algorithms and reduce the streamed hit
/// table into one max score per (row, col) cell. Mirrors `seq_dist.diamond`:
/// build a DIAMOND db from the reference, then `blastp` the input against it with
/// `--outfmt 6 qseqid sseqid bitscore --algo 0 --max-hsps 1`, parsing the
/// subprocess stdout line by line (no large intermediate file).
#[allow(clippy::too_many_arguments)]
fn run_diamond(
    input: &str,
    input_kind: InputKind,
    ref_path: &str,
    ref_kind: InputKind,
    same: bool,
    sensitivity: Sensitivity,
    opts: &SeqDistOptions,
) -> std::io::Result<DiamondCells> {
    let diamond_bin = opts
        .diamond_path
        .as_ref()
        .map_or_else(find_diamond, PathBuf::from);
    let tmp = TempDir::new("domainatoredux_diamond")?;

    // Normalize input/reference to FASTA whose header first token is the record
    // id, so DIAMOND's `qseqid`/`sseqid` match the name index exactly.
    let input_seqs = load_seqs(input, input_kind)?;
    let input_fasta = tmp.path().join("input.fasta");
    write_diamond_fasta(&input_seqs.records, &input_fasta)?;
    let row_index = input_seqs.index;

    let (ref_fasta, col_index) = if same {
        (input_fasta.clone(), clone_index(&row_index))
    } else {
        let ref_seqs = load_seqs(ref_path, ref_kind)?;
        let ref_fasta = tmp.path().join("ref.fasta");
        write_diamond_fasta(&ref_seqs.records, &ref_fasta)?;
        (ref_fasta, ref_seqs.index)
    };

    // Build the reference database (db.dmnd) under the temp dir.
    let dbpath = tmp.path().join("db");
    let mut makedb_cmd = Command::new(&diamond_bin);
    super::external::hide_console(&mut makedb_cmd);
    let makedb = makedb_cmd
        .arg("makedb")
        .arg("--in")
        .arg(&ref_fasta)
        .arg("-d")
        .arg(&dbpath)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| diamond_spawn_error(e, &diamond_bin))?;
    if !makedb.status.success() {
        return Err(std::io::Error::other(format!(
            "diamond makedb failed:\n{}",
            String::from_utf8_lossy(&makedb.stderr)
        )));
    }

    // `--cpu 0` means "all cores"; resolve to a concrete count for diamond `-p`.
    let threads = if opts.cpu > 0 {
        opts.cpu
    } else {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    };
    // `--max-target-seqs 0` means "report all targets".
    let max_target_seqs = opts.max_hits_per_query.unwrap_or(0);

    let mut cmd = Command::new(&diamond_bin);
    super::external::hide_console(&mut cmd);
    cmd.arg("blastp")
        .arg("-q")
        .arg(&input_fasta)
        .arg("-d")
        .arg(&dbpath)
        .arg("-p")
        .arg(threads.to_string())
        .arg("--outfmt")
        .arg("6")
        .arg("qseqid")
        .arg("sseqid")
        .arg("bitscore")
        .arg("--max-hsps")
        .arg("1")
        .arg("--max-target-seqs")
        .arg(max_target_seqs.to_string());
    if opts.threshold > 0.0 {
        cmd.arg("--min-score").arg(format!("{}", opts.threshold));
    }
    for a in sensitivity_args(sensitivity) {
        cmd.arg(a);
    }
    // Defaults --diamond_args may replace. --tmpdir defaults into the managed temp dir so
    // diamond's unlinked scratch lands beside the db rather than in the cwd.
    let tmpdir = dbpath.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    for (flag, default) in [
        ("--algo", "0".to_string()),
        ("--tmpdir", tmpdir.to_string_lossy().into_owned()),
    ] {
        if !opts.diamond_args.iter().any(|(f, _)| f == flag) {
            cmd.arg(flag).arg(default);
        }
    }
    for (flag, value) in &opts.diamond_args {
        cmd.arg(flag);
        if let Some(v) = value {
            cmd.arg(v);
        }
    }
    // Echo the command, as Python does, so a failing run can be reproduced by hand.
    eprintln!(
        "diamond command: {} {}",
        diamond_bin.display(),
        cmd.get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ")
    );
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| diamond_spawn_error(e, &diamond_bin))?;
    let stdout = child.stdout.take().expect("diamond stdout is piped");
    let stderr = child.stderr.take().expect("diamond stderr is piped");
    // Drain stderr on a side thread so a full stderr pipe can't deadlock stdout.
    let stderr_handle = std::thread::spawn(move || {
        let mut buf = String::new();
        let _ = BufReader::new(stderr).read_to_string(&mut buf);
        buf
    });

    // Stream the tab-separated hit table (qseqid, sseqid, bitscore) straight into
    // the per-cell max map.
    let mut cells: HashMap<(i64, i64), f64> = HashMap::new();
    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        let line = line?;
        let mut it = line.split('\t');
        // Require exactly three fields (matches the Python `len(parts) == 3` gate).
        let (Some(q), Some(s), Some(b), None) = (it.next(), it.next(), it.next(), it.next()) else {
            continue;
        };
        let Ok(score) = b.trim().parse::<f64>() else {
            continue;
        };
        if score <= opts.threshold {
            continue;
        }
        let (Some(&ri), Some(&ci)) = (row_index.idx.get(q), col_index.idx.get(s)) else {
            continue;
        };
        let v = score;
        let e = cells
            .entry((ri as i64, ci as i64))
            .or_insert(f64::NEG_INFINITY);
        if v > *e {
            *e = v;
        }
    }

    let status = child.wait()?;
    let stderr_text = stderr_handle.join().unwrap_or_default();
    if !status.success() {
        return Err(std::io::Error::other(format!(
            "diamond blastp failed:\n{stderr_text}"
        )));
    }

    Ok((cells, row_index, col_index))
}

/// Write `(id, residues)` records as FASTA whose header is just the id, so
/// DIAMOND's `qseqid`/`sseqid` columns equal the record id.
fn write_diamond_fasta(records: &[(String, Vec<u8>)], path: &Path) -> std::io::Result<()> {
    super::external::write_fasta_ids(records, path)
}

/// Locate the `diamond` executable: `$DOMAINATOREDUX_DIAMOND` / `$DIAMOND`, then a
/// bundled `external/diamond/diamond` (searched upward from the running
/// executable and in the current directory), then bare `diamond` on `PATH`.
fn find_diamond() -> PathBuf {
    use super::deduplicate_genbank::{DIAMOND_BIN_NAME, DIAMOND_ENV_VAR};
    super::external::find_binary(DIAMOND_BIN_NAME, &[DIAMOND_ENV_VAR, "DIAMOND"], "diamond")
}

/// Turn a "diamond not found" spawn failure into an actionable error.
fn diamond_spawn_error(e: std::io::Error, bin: &Path) -> std::io::Error {
    super::external::spawn_error(
        e,
        bin,
        "Set $DOMAINATOREDUX_DIAMOND to the diamond binary, place it at external/diamond/diamond, \
         or add diamond to PATH.",
    )
}

use super::external::TempDir;

/// A DIAMOND pass result: `(row, col) -> best score` keyed by matrix index,
/// plus the query and reference name indexes those coordinates refer to.
type DiamondCells = (HashMap<(i64, i64), f64>, NameIndex, NameIndex);

#[cfg(test)]
mod tests {
    use super::{load_seqs, parse_diamond_args, InputKind};

    fn words(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    /// Upstream's `test_parse_diamond_args_*`: a value follows its flag unless it is
    /// itself a flag, a negative number is a value, and options seq_dist sets itself
    /// are refused.
    #[test]
    fn diamond_args_parse_like_upstreams() {
        let parsed = parse_diamond_args(&words("-c 4 -b 0.4 --target-indexed --min-orf -1"));
        let some = |s: &str| Some(s.to_string());
        assert_eq!(
            parsed.unwrap(),
            [
                ("-c".to_string(), some("4")),
                ("-b".to_string(), some("0.4")),
                ("--target-indexed".to_string(), None),
                ("--min-orf".to_string(), some("-1")),
            ]
        );
        assert_eq!(parse_diamond_args(&[]).unwrap(), []);
        assert!(
            parse_diamond_args(&words("--block-size -1.5e3")).unwrap()[0]
                .1
                .is_some()
        );
        for reserved in [
            "--outfmt 6",
            "-p 4",
            "--max-target-seqs 10",
            "--ultra-sensitive",
        ] {
            let err = parse_diamond_args(&words(reserved)).unwrap_err();
            assert!(err.contains("may not set"), "{reserved}: {err}");
        }
        let err = parse_diamond_args(&words("4 -c")).unwrap_err();
        assert!(err.contains("starting with '-'"), "{err}");
    }
    use crate::matrix::{DataMatrix, Storage};
    use crate::tools::build_ssn_viewer::{build_ssn_viewer_bundle, BuildSsnViewerOptions};

    #[test]
    fn genbank_matrix_labels_use_locus_not_version() {
        let path = std::env::temp_dir().join(format!("seq_dist_locus_{}.gb", std::process::id()));
        let record = "LOCUS       protein_3:9 6 aa                     UNK 01-JAN-2000\n\
DEFINITION  Example protein.\n\
ACCESSION   protein\n\
VERSION     protein\n\
FEATURES             Location/Qualifiers\n\
ORIGIN\n\
        1 acdefg\n\
//\n";
        std::fs::write(&path, record).unwrap();
        let result = load_seqs(path.to_str().unwrap(), InputKind::Genbank);
        std::fs::remove_file(&path).unwrap();
        let seqs = result.unwrap();
        assert_eq!(seqs.index.names, ["protein_3:9"]);
        assert_eq!(seqs.records[0].0, "protein_3:9");

        let metadata_path = path.with_extension("tsv");
        std::fs::write(&metadata_path, "contig\tdomains\nprotein_3:9\tDUF262\n").unwrap();
        let matrix = DataMatrix {
            rows: seqs.index.names.clone(),
            cols: seqs.index.names,
            row_lengths: None,
            col_lengths: None,
            data_type: "score".to_string(),
            storage: Storage::Dense(vec![0.0]),
        };
        let opts = BuildSsnViewerOptions {
            metadata_files: vec![metadata_path.to_str().unwrap().to_string()],
            ..BuildSsnViewerOptions::default()
        };
        let bundle = build_ssn_viewer_bundle(&matrix, &opts);
        std::fs::remove_file(&metadata_path).unwrap();
        assert_eq!(bundle.unwrap()["metadata"]["rows"][0][0], "DUF262");
    }
}
