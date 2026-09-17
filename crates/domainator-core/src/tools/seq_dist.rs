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
use crate::matrix::transform::apply_mode;
use crate::matrix::{build_sparse_max, DataMatrix};

/// `--mode` values whose post-transform value is computable per edge (no global
/// matrix maxima), and therefore compatible with `--mst_knn`. Mirrors
/// `seq_dist.MST_KNN_STREAMABLE_MODES`.
pub const MST_KNN_STREAMABLE_MODES: [&str; 3] = ["score", "bool", "efi_score"];
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
    /// Round raw scores `<= lb` down to zero (applies before any mode transform).
    pub lb: f64,
    /// Keep at most this many non-zero entries per input row (top-N by score).
    pub k: Option<usize>,
    /// Prune the output to MST ∪ OR-symmetric kNN edges (integer > 1). Requires a
    /// square symmetric comparison (reference == input) and a streamable `--mode`.
    /// When set, `k` is ignored. Mirrors `seq_dist --mst_knn`.
    pub mst_knn: Option<usize>,
    /// Pure OR-symmetric kNN (`--knn`), without the spanning tree. Mutually exclusive
    /// with `mst_knn`.
    pub knn: Option<usize>,
    /// Extra `(flag, value)` options forwarded verbatim to `diamond blastp`
    /// (`--params`). A `None` value means a bare flag. See [`RESERVED_DIAMOND_PARAMS`].
    pub params: Vec<(String, Option<String>)>,
    /// Worker threads (`0` = all available cores).
    pub cpu: usize,
}

impl Default for SeqDistOptions {
    fn default() -> Self {
        SeqDistOptions {
            algorithm: "diamond_us".to_string(),
            mode: "score".to_string(),
            lb: 0.0,
            k: None,
            mst_knn: None,
            knn: None,
            params: Vec::new(),
            cpu: 0,
        }
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

    if opts.mst_knn.is_some() && opts.knn.is_some() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "--mst_knn and --knn are mutually exclusive (--mst_knn k == --mst --knn k).",
        ));
    }
    if opts.mst_knn.is_some() || opts.knn.is_some() {
        if !MST_KNN_STREAMABLE_MODES.contains(&opts.mode.as_str()) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "{} is only supported with --mode {:?} (modes that normalize \
                     against global matrix maxima cannot be pruned per edge); got '{}'",
                    if opts.knn.is_some() { "--knn" } else { "--mst_knn" },
                    MST_KNN_STREAMABLE_MODES, opts.mode
                ),
            ));
        }
        if !same {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "{} requires a square, symmetric comparison (reference must be the \
                     same file as input)",
                    if opts.knn.is_some() { "--knn" } else { "--mst_knn" }
                ),
            ));
        }
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

    let bool_mode = opts.mode == "bool";

    // Score every (input, reference) pair and reduce to one max value per cell,
    // keyed by (row, col) index. The diamond path streams the subprocess output
    // straight into the cell map (no large intermediate table); the hmmer paths
    // score in a rayon `--cpu` pool.
    let (cells, row_index, col_index): (HashMap<(i64, i64), f64>, NameIndex, NameIndex) =
        match algorithm {
            Algorithm::Diamond(sensitivity) => run_diamond(
                input, input_kind, ref_path, ref_kind, same, sensitivity, opts, bool_mode,
            )?,
            _ => crate::parallel::with_threads(opts.cpu, || -> std::io::Result<_> {
                let (results, row_index, col_index): (Vec<CmpResult>, NameIndex, NameIndex) =
                    if algorithm == Algorithm::HmmerCompare {
                        // hmmer_compare: profile-vs-profile Viterbi alignment.
                        let (q_hmms, row_index) = load_hmms(input)?;
                        if same {
                            let results = hmm_compare_scores(&q_hmms, &q_hmms, opts.lb);
                            let col_index = clone_index(&row_index);
                            (results, row_index, col_index)
                        } else {
                            let (r_hmms, col_index) = load_hmms(ref_path)?;
                            let results = hmm_compare_scores(&q_hmms, &r_hmms, opts.lb);
                            (results, row_index, col_index)
                        }
                    } else {
                        let input_seqs = load_seqs(input, input_kind)?;
                        if ref_kind == InputKind::Hmm {
                            let (hmms, col_index) = load_hmms(ref_path)?;
                            let results = hmmsearch_scores(&input_seqs.records, &hmms, opts.lb);
                            (results, input_seqs.index, col_index)
                        } else if same {
                            let col_index = clone_index(&input_seqs.index);
                            let results =
                                phmmer_scores(&input_seqs.records, &input_seqs.records, opts.lb);
                            (results, input_seqs.index, col_index)
                        } else {
                            let ref_seqs = load_seqs(ref_path, ref_kind)?;
                            let results =
                                phmmer_scores(&input_seqs.records, &ref_seqs.records, opts.lb);
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
                    let v = if bool_mode { 1.0 } else { r.score };
                    let e = cells.entry((ri as i64, ci as i64)).or_insert(f64::NEG_INFINITY);
                    if v > *e {
                        *e = v;
                    }
                }
                Ok((cells, row_index, col_index))
            })?,
        };

    let nrows = row_index.names.len();
    let ncols = col_index.names.len();

    // Optional top-k per row (by score desc, then column asc — matches the
    // Python `(-score, reference)` ordering with column index standing in for
    // the reference name order).
    // `--mst_knn` bypasses per-row top-k so the pruning sees the full candidate
    // set (matching the Python streaming path's `k=None`). The diamond path
    // already capped targets per query via `--max-target-seqs`.
    let k = if opts.mst_knn.is_some() || opts.knn.is_some() || matches!(algorithm, Algorithm::Diamond(_)) {
        None
    } else {
        opts.k
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

    // Prune to MST ∪ OR-symmetric kNN. The raw-score lower bound was already
    // applied during scoring (`opts.lb`), so the pruning runs at lower_bound=0
    // on the post-mode values (matching the Python streaming path).
    if let Some((k, include_mst)) =
        opts.mst_knn.map(|k| (k, true)).or_else(|| opts.knn.map(|k| (k, false)))
    {
        matrix = apply_mst_knn(&matrix, k, 0.0, include_mst);
    }
    Ok(matrix)
}

/// diamond options `seq_dist` sets itself. Overriding one through `--params` would
/// either break parsing of the result stream or silently contradict a `seq_dist` option,
/// so each is rejected with a pointer to the option that does control it.
pub const RESERVED_DIAMOND_PARAMS: [(&str, &str); 21] = [
    ("-q", "-i/--input"),
    ("--query", "-i/--input"),
    ("-d", "-r/--reference"),
    ("--db", "-r/--reference"),
    ("-f", "the fixed tabular output format required to parse the result stream"),
    ("--outfmt", "the fixed tabular output format required to parse the result stream"),
    ("-p", "--cpu"),
    ("--threads", "--cpu"),
    ("-k", "-k"),
    ("--max-target-seqs", "-k"),
    ("--max-hsps", "seq_dist (fixed at 1)"),
    ("--min-score", "--lb"),
    ("--verbose", "seq_dist (required for progress reporting)"),
    ("--fast", "--algorithm"),
    ("--mid-sensitive", "--algorithm"),
    ("--sensitive", "--algorithm"),
    ("--more-sensitive", "--algorithm"),
    ("--very-sensitive", "--algorithm"),
    ("--ultra-sensitive", "--algorithm"),
    ("--algo", "seq_dist (overridable: pass it in --params to replace the default)"),
    ("--tmpdir", "seq_dist (overridable: pass it in --params to replace the default)"),
];

/// Parse a `--params` string into ordered `(flag, value)` pairs.
///
/// The string is a JSON dict body *without* the enclosing braces, matching
/// `deduplicate_genbank --params`: `'"-c":4, "-b":0.4'`. A `null` value means the flag
/// takes no argument: `'"--target-indexed":null'`.
pub fn parse_diamond_params(params_str: &str) -> Result<Vec<(String, Option<String>)>, String> {
    if params_str.trim().is_empty() {
        return Ok(Vec::new());
    }
    let wrapped = format!("{{{params_str}}}");
    let parsed: serde_json::Value = serde_json::from_str(&wrapped).map_err(|e| {
        format!(
            "Could not parse --params as a json dict: {e}\nExpected a json dict body \
             without the outer braces, such as: --params '\"-c\":4, \"-b\":0.4'"
        )
    })?;
    let serde_json::Value::Object(map) = parsed else {
        return Err(
            "--params must be a json dict of diamond options, such as: --params '\"-c\":4'"
                .to_string(),
        );
    };

    let mut out = Vec::with_capacity(map.len());
    for (flag, value) in map {
        // Only the two overridable defaults may be named; everything else in the
        // reserved table is something seq_dist must control.
        if let Some((_, owner)) = RESERVED_DIAMOND_PARAMS
            .iter()
            .find(|(f, _)| *f == flag && !matches!(flag.as_str(), "--algo" | "--tmpdir"))
        {
            return Err(format!(
                "--params may not set '{flag}': seq_dist controls it via {owner}."
            ));
        }
        let rendered = match value {
            serde_json::Value::Null => None,
            serde_json::Value::String(s) => Some(s),
            other => Some(other.to_string()),
        };
        out.push((flag, rendered));
    }
    Ok(out)
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
    bool_mode: bool,
) -> std::io::Result<DiamondCells> {
    let diamond_bin = find_diamond();
    let tmp = TempDir::new("domainator_diamond")?;

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
    let makedb = Command::new(&diamond_bin)
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
        return Err(std::io::Error::other(format!("diamond makedb failed:\n{}", String::from_utf8_lossy(&makedb.stderr)),
        ));
    }

    // `--cpu 0` means "all cores"; resolve to a concrete count for diamond `-p`.
    let threads = if opts.cpu > 0 {
        opts.cpu
    } else {
        std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
    };
    // `--mst_knn` prunes after scoring, so it must see every candidate edge (-k is
    // ignored, matching the hmmer path); otherwise cap targets per query at -k.
    // `--max-target-seqs 0` means "report all targets".
    let max_target_seqs = if opts.mst_knn.is_some() { 0 } else { opts.k.unwrap_or(0) };

    let mut cmd = Command::new(&diamond_bin);
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
    if opts.lb > 0.0 {
        cmd.arg("--min-score").arg(format!("{}", opts.lb));
    }
    for a in sensitivity_args(sensitivity) {
        cmd.arg(a);
    }
    // Defaults --params may replace. --tmpdir defaults into the managed temp dir so
    // diamond's unlinked scratch lands beside the db rather than in the cwd.
    let tmpdir = dbpath.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    for (flag, default) in
        [("--algo", "0".to_string()), ("--tmpdir", tmpdir.to_string_lossy().into_owned())]
    {
        if !opts.params.iter().any(|(f, _)| f == flag) {
            cmd.arg(flag).arg(default);
        }
    }
    for (flag, value) in &opts.params {
        cmd.arg(flag);
        if let Some(v) = value {
            cmd.arg(v);
        }
    }
    // Echo the command, as Python does, so a failing run can be reproduced by hand.
    eprintln!(
        "diamond command: {} {}",
        diamond_bin.display(),
        cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect::<Vec<_>>().join(" ")
    );
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| diamond_spawn_error(e, &diamond_bin))?;
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
        let (Some(q), Some(s), Some(b), None) = (it.next(), it.next(), it.next(), it.next())
        else {
            continue;
        };
        let Ok(score) = b.trim().parse::<f64>() else {
            continue;
        };
        if score <= opts.lb {
            continue;
        }
        let (Some(&ri), Some(&ci)) = (row_index.idx.get(q), col_index.idx.get(s)) else {
            continue;
        };
        let v = if bool_mode { 1.0 } else { score };
        let e = cells.entry((ri as i64, ci as i64)).or_insert(f64::NEG_INFINITY);
        if v > *e {
            *e = v;
        }
    }

    let status = child.wait()?;
    let stderr_text = stderr_handle.join().unwrap_or_default();
    if !status.success() {
        return Err(std::io::Error::other(format!("diamond blastp failed:\n{stderr_text}"),
        ));
    }

    Ok((cells, row_index, col_index))
}

/// Write `(id, residues)` records as FASTA whose header is just the id, so
/// DIAMOND's `qseqid`/`sseqid` columns equal the record id.
fn write_diamond_fasta(records: &[(String, Vec<u8>)], path: &Path) -> std::io::Result<()> {
    super::external::write_fasta_ids(records, path)
}

/// Locate the `diamond` executable: `$DOMAINATOR_DIAMOND` / `$DIAMOND`, then a
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
        "Set $DOMAINATOR_DIAMOND to the diamond binary, place it at external/diamond/diamond, \
         or add diamond to PATH.",
    )
}

use super::external::TempDir;

/// A DIAMOND pass result: `(row, col) -> best score` keyed by matrix index,
/// plus the query and reference name indexes those coordinates refer to.
type DiamondCells = (HashMap<(i64, i64), f64>, NameIndex, NameIndex);
