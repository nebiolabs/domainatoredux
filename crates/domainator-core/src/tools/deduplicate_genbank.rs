//! `deduplicate_genbank`: collapse redundant sequences from GenBank/FASTA input.
//!
//! - `hash` (pure Rust): exact-duplicate removal. Sequences are upper-cased and
//!   stripped of leading/trailing `*`, then grouped by equality (optionally over
//!   both nucleotide strands). This is a faithful port of the Python `hash`
//!   algorithm — identical grouping without needing SHA-256, since the hashes
//!   never appear in the output; equal sequences map to the same representative.
//! - `cd-hit` / `cd-hit-est` / `diamond` / `usearch`: shell out to the external
//!   clustering binary. `cd-hit` switches to `cd-hit-est` when the input is not
//!   protein, mirroring Python.
//!
//! The representative of each cluster is the first input record that created it
//! (`hash`) or the cluster centroid the tool reports (external backends).
//! Output is always emitted in input order (representatives only).

use std::collections::HashMap;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::seq::translate::revcomp;
use crate::seq::SeqRecord;

use super::external::{spawn_error, write_fasta_ids, TempDir};

/// Options for [`deduplicate`].
pub struct DeduplicateOptions {
    /// `hash` | `cd-hit` | `cd-hit-est` | `diamond` | `usearch`.
    pub algorithm: String,
    /// Identity threshold in `[0, 1]` (must be `1.0` for `hash`).
    pub id: f64,
    /// Consider both strands (nucleotide only).
    pub both_strands: bool,
    /// Default molecule type for FASTA / unit-less input.
    pub fasta_type: String,
    /// Worker threads for the external clusterer (`0` = all cores).
    pub cpu: usize,
    /// The clusterer's executable (`--cdhit_path`, `--usearch_path`,
    /// `--diamond_path`), instead of locating it.
    pub bin_path: Option<String>,
    /// cd-hit-est's executable (`--cdhit_est_path`), used for nucleotide input with
    /// `cd-hit`; `bin_path` when not given, since the `cdhit` program runs both.
    pub cdhit_est_path: Option<String>,
    /// Extra arguments for the external clusterer (`--cdhit_args`, `--usearch_args`,
    /// `--diamond_args`, already split into words). One that sets an option this
    /// tool sets itself is refused.
    pub extra_args: Vec<String>,
}

impl Default for DeduplicateOptions {
    fn default() -> Self {
        DeduplicateOptions {
            algorithm: "hash".into(),
            id: 1.0,
            both_strands: false,
            fasta_type: "protein".into(),
            cpu: 0,
            bin_path: None,
            cdhit_est_path: None,
            extra_args: Vec::new(),
        }
    }
}

/// Result of clustering: representative record indices (input order, which is the
/// order records are emitted in) and the `(representative_index, member_indices)`
/// clusters **in the order the clustering reported them**, which is what the
/// `--cluster_table` rows follow. For `hash` that is input order; for
/// `usearch`/`diamond` it is the tool's own output order (usearch sorts by length),
/// and sorting it would be a gratuitous difference from the reference table.
pub struct DedupResult {
    pub representatives: Vec<usize>,
    pub clusters: Vec<(usize, Vec<usize>)>,
}

/// Cluster `records` and return which are representatives. Does no I/O on the
/// records themselves (naming / writing is the caller's job).
pub fn deduplicate(
    records: &[SeqRecord],
    opts: &DeduplicateOptions,
) -> Result<DedupResult, String> {
    if opts.both_strands && opts.fasta_type != "nucleotide" {
        return Err("--both_strands is only supported for nucleotide input.".into());
    }
    if opts.id < 0.0 || opts.id > 1.0 {
        return Err(format!("id must be between 0 and 1, not {}", opts.id));
    }
    match opts.algorithm.as_str() {
        "hash" => hash_dedup(records, opts),
        "diamond" => external_dedup(records, opts, ExternalTool::Diamond),
        "usearch" => external_dedup(records, opts, ExternalTool::Usearch),
        // Python's argparse only offers `cd-hit`, but its class registry accepts
        // `cd-hit-est` too, and the caller substitutes it for non-protein input.
        "cd-hit" => external_dedup(records, opts, ExternalTool::CdHit { force_est: false }),
        "cd-hit-est" => external_dedup(records, opts, ExternalTool::CdHit { force_est: true }),
        other => Err(format!(
            "unknown --algorithm {other:?} (have: hash, cd-hit, cd-hit-est, diamond, usearch)"
        )),
    }
}

/// The normalized sequence key: upper-cased, with leading/trailing `*` stripped.
fn hash_key(seq: &[u8]) -> Vec<u8> {
    let upper: Vec<u8> = seq.iter().map(|b| b.to_ascii_uppercase()).collect();
    let start = upper.iter().position(|&b| b != b'*').unwrap_or(upper.len());
    let end = upper
        .iter()
        .rposition(|&b| b != b'*')
        .map_or(start, |i| i + 1);
    upper[start..end].to_vec()
}

/// The keys identifying a sequence for exact-duplicate detection (upstream
/// `model.sequence_hashes`, with the normalized sequences standing in for their
/// SHA-256 digests): the [`hash_key`], plus its reverse complement with
/// `both_strands`. Two sequences are duplicates if their keys share one. A plain
/// complement or reverse is not the other strand, so neither counts (upstream
/// `09ddc4d`).
pub(crate) fn sequence_keys(seq: &[u8], both_strands: bool) -> Vec<Vec<u8>> {
    let key = hash_key(seq);
    if both_strands {
        let rc = revcomp(&key);
        vec![key, rc]
    } else {
        vec![key]
    }
}

fn hash_dedup(records: &[SeqRecord], opts: &DeduplicateOptions) -> Result<DedupResult, String> {
    if opts.id != 1.0 {
        return Err(format!(
            "hash algorithm only valid for id = 1, not {}.",
            opts.id
        ));
    }
    // Sequence key -> representative index. For both_strands a record registers
    // its forward and reverse-complement keys (mirroring Python).
    let mut seen: HashMap<Vec<u8>, usize> = HashMap::new();
    let mut members: HashMap<usize, Vec<usize>> = HashMap::new();
    // Python keys `cluster_members` off `seen.values()`, i.e. first-occurrence
    // order -- which for the hash path is just input order.
    let mut order: Vec<usize> = Vec::new();

    for (i, rec) in records.iter().enumerate() {
        let keys = sequence_keys(&rec.seq, opts.both_strands);
        let rep = keys.iter().find_map(|k| seen.get(k).copied());
        let rep = match rep {
            Some(r) => r,
            None => {
                for k in &keys {
                    seen.entry(k.clone()).or_insert(i);
                }
                i
            }
        };
        members.entry(rep).or_default().push(i);
        if rep == i {
            order.push(i);
        }
    }

    finalize(order, members)
}

/// The oldest diamond this port supports for `--algorithm diamond`.
///
/// Upstream raised its prereq here and dropped the longest-sequence fallback that
/// used to paper over the single-cluster crash, so anything older fails on an input
/// that collapses into one cluster. Anything that *offers* to install diamond --
/// the GUI's External tools panel -- has to honor this; a test in that crate
/// checks the managed download against it.
pub const MIN_DIAMOND_VERSION: &str = "2.2.5";

// The locator identity of each external binary: the file name [`find_binary`]
// looks for, and the environment variable that overrides the search. Anything
// that *installs* one of these -- the GUI's External tools panel writes it to
// `<app_data>/bin/<bin name>` and exports the env var -- has to agree with these
// or the download lands somewhere the core never looks; a test in that crate
// checks the managed specs against them.
pub const DIAMOND_BIN_NAME: &str = "diamond";
pub const DIAMOND_ENV_VAR: &str = "DOMAINATOREDUX_DIAMOND";
pub const USEARCH_BIN_NAME: &str = "usearch";
pub const USEARCH_ENV_VAR: &str = "DOMAINATOREDUX_USEARCH";
/// cd-hit is driven through the `cdhit` Rust port, a single multiplexer binary
/// whose first argument is the program name (`cd-hit`, `cd-hit-est`, ...). When no
/// `cdhit` is found, C cd-hit's `cd-hit`/`cd-hit-est` on `PATH` stand in
/// ([`ExternalProgram::alt_names`](super::external::ExternalProgram::alt_names)).
pub const CDHIT_BIN_NAME: &str = "cdhit";
pub const CDHIT_ENV_VAR: &str = "DOMAINATOREDUX_CDHIT";

enum ExternalTool {
    Diamond,
    Usearch,
    /// `force_est` is set when the caller asked for `cd-hit-est` by name; otherwise
    /// the choice follows the input's molecule type.
    CdHit {
        force_est: bool,
    },
}

fn external_dedup(
    records: &[SeqRecord],
    opts: &DeduplicateOptions,
    tool: ExternalTool,
) -> Result<DedupResult, String> {
    // Molecule type of the first record decides protein-vs-nucleotide behavior
    // (mirrors Python's `seqtype = first rec molecule_type`).
    let seqtype = records
        .first()
        .and_then(|r| r.molecule_type.clone())
        .unwrap_or_else(|| opts.fasta_type.clone());
    if matches!(tool, ExternalTool::Diamond) && seqtype != "protein" {
        return Err("diamond deduplication is only supported for protein input.".into());
    }

    let tmp = TempDir::new("domainatoredux_dedup").map_err(|e| e.to_string())?;
    let input_fasta = tmp.path().join("input.fasta");
    // id=str(i), sequence upper-cased with `*` -> `f` (matches Python's fasta write).
    let cleaned: Vec<(String, Vec<u8>)> = records
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let seq: Vec<u8> = r
                .seq
                .iter()
                .map(|b| {
                    let u = b.to_ascii_uppercase();
                    if u == b'*' {
                        b'f'
                    } else {
                        u
                    }
                })
                .collect();
            (i.to_string(), seq)
        })
        .collect();
    write_fasta_ids(&cleaned, &input_fasta).map_err(|e| e.to_string())?;

    let cpus = crate::parallel::resolve_cpu(opts.cpu);
    let clusters_by_str = match tool {
        ExternalTool::Diamond => run_diamond(opts, &tmp, &input_fasta, cpus)?,
        ExternalTool::Usearch => run_usearch(opts, &tmp, &input_fasta, cpus, records)?,
        // Python substitutes cd-hit-est for cd-hit whenever the input is not protein.
        ExternalTool::CdHit { force_est } => run_cdhit(
            opts,
            &tmp,
            &input_fasta,
            cpus,
            force_est || seqtype != "protein",
        )?,
    };

    // Map str-index cluster keys back to numeric indices, keeping the tool's own
    // cluster order and its member order within each cluster (Python appends them
    // straight out of the `.uc` / diamond table).
    let mut members: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut order: Vec<usize> = Vec::new();
    for (rep, mem) in clusters_by_str {
        let rep_i: usize = rep.parse().map_err(|_| format!("bad cluster id {rep:?}"))?;
        let mem_i: Vec<usize> = mem
            .iter()
            .map(|m| {
                m.parse::<usize>()
                    .map_err(|_| format!("bad member id {m:?}"))
            })
            .collect::<Result<_, _>>()?;
        order.push(rep_i);
        members.insert(rep_i, mem_i);
    }
    finalize(order, members)
}

/// Turn a `rep -> members` map into a [`DedupResult`].
///
/// `order` is the order the clustering reported its clusters in, which the
/// `--cluster_table` follows; the representatives are sorted independently
/// because records are always *emitted* in input order.
fn finalize(order: Vec<usize>, members: HashMap<usize, Vec<usize>>) -> Result<DedupResult, String> {
    let clusters: Vec<(usize, Vec<usize>)> =
        order.iter().map(|&r| (r, members[&r].clone())).collect();
    let mut reps: Vec<usize> = order;
    reps.sort_unstable();
    Ok(DedupResult {
        representatives: reps,
        clusters,
    })
}

fn run_diamond(
    opts: &DeduplicateOptions,
    tmp: &TempDir,
    input_fasta: &Path,
    cpus: usize,
) -> Result<Vec<(String, Vec<String>)>, String> {
    let bin = match &opts.bin_path {
        Some(p) => PathBuf::from(p),
        None => super::external::DIAMOND.find(),
    };
    let out_tsv = tmp.path().join("output.tsv");
    let mut cmd = Command::new(&bin);
    super::external::hide_console(&mut cmd);
    cmd.arg("cluster")
        .arg("-d")
        .arg(input_fasta)
        .arg("-o")
        .arg(&out_tsv)
        .arg("--approx-id")
        .arg(format!("{}", opts.id * 100.0))
        .arg("--threads")
        .arg(cpus.to_string());
    cmd.args(extra_args(
        &opts.extra_args,
        &["-d", "-o", "--approx-id", "--threads"],
        "--diamond_args",
    )?);
    let out = cmd.output().map_err(|e| {
        spawn_error(e, &bin, "Install diamond or set $DOMAINATOREDUX_DIAMOND.").to_string()
    })?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        // Before MIN_DIAMOND_VERSION the cluster writer crashed when the whole input
        // collapsed into a single cluster, reporting "max_oid not set" even though the
        // clustering itself had succeeded. Upstream dropped its longest-sequence
        // fallback when it raised the prereq; name the version here so a user on an
        // older diamond gets an actionable error rather than a bare crash message.
        if ["max_oid not set", "Record count not set"]
            .iter()
            .any(|e| stderr.contains(e))
        {
            return Err(format!(
                "diamond cluster failed on an input that collapses into a single cluster. \
                 This is a bug in diamond before {MIN_DIAMOND_VERSION}; upgrade diamond \
                 (or set $DOMAINATOREDUX_DIAMOND to a newer binary).\n{stderr}"
            ));
        }
        return Err(format!("diamond cluster failed:\n{stderr}"));
    }
    parse_diamond_tsv(&out_tsv)
}

fn parse_diamond_tsv(path: &Path) -> Result<Vec<(String, Vec<String>)>, String> {
    let mut order: Vec<String> = Vec::new();
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    for line in std::io::BufReader::new(file).lines() {
        let line = line.map_err(|e| e.to_string())?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 2 {
            continue;
        }
        let (rep, member) = (parts[0], parts[1]);
        if rep.eq_ignore_ascii_case("representative") && member.eq_ignore_ascii_case("member") {
            continue;
        }
        let entry = map.entry(rep.to_string()).or_insert_with(|| {
            order.push(rep.to_string());
            Vec::new()
        });
        if !entry.contains(&member.to_string()) {
            entry.push(member.to_string());
        }
    }
    // Ensure each representative is a member of its own cluster.
    for rep in &order {
        let mem = map.get_mut(rep).unwrap();
        if !mem.contains(rep) {
            mem.insert(0, rep.clone());
        }
    }
    Ok(order
        .into_iter()
        .map(|r| (r.clone(), map.remove(&r).unwrap()))
        .collect())
}

/// cd-hit's `-n` word size, which Python derives from the identity threshold
/// rather than exposing (`CdHitProtein` / `CdHitEST`). The thresholds are
/// strictly-greater except at the bottom of each table, and below that cd-hit's
/// seeding cannot work at all -- hence the hard error rather than a clamp.
fn cdhit_word_size(id: f64, est: bool) -> Result<u32, String> {
    if est {
        if id > 0.95 {
            Ok(10)
        } else if id > 0.9 {
            Ok(9)
        } else if id > 0.88 {
            Ok(7)
        } else if id > 0.85 {
            Ok(6)
        } else if id >= 0.8 {
            Ok(5)
        } else {
            Err("For cd-hit on nucleotide sequences, id must be >= 0.8".into())
        }
    } else if id > 0.7 {
        Ok(5)
    } else if id > 0.6 {
        Ok(4)
    } else if id > 0.5 {
        Ok(3)
    } else if id >= 0.4 {
        Ok(2)
    } else {
        Err("For cd-hit on protein sequences, id must be >= 0.4".into())
    }
}

/// Resolve `bin` into the command to actually run: the executable, and the program
/// name to pass as its first argument (or `None` to run it directly).
///
/// The Rust cd-hit port is one `cdhit` multiplexer whose first argument selects the
/// program (`cd-hit`, `cd-hit-est`, ...), while the original C++ suite ships those as
/// separate executables side by side -- and `cdhit` also works symlinked to those
/// names. So a binary *called* `cd-hit`/`cd-hit-est` is run directly with no program
/// argument, and anything else is treated as the multiplexer.
///
/// The trap is a **name that does not match the program we need**: pointing
/// `$DOMAINATOREDUX_CDHIT` at `cd-hit` and then feeding nucleotide input would run the
/// protein program over DNA and return a plausible-looking wrong answer. Since the
/// C++ suite installs both executables together, look for the sibling first; only if
/// it is genuinely absent is this an error, and it is an error rather than a silent
/// substitution. (Python has the same hazard and does substitute silently, but no
/// upstream invocation passes `--cdhit_path`, so nothing here is parity-relevant.)
fn cdhit_command(bin: &Path, est: bool) -> Result<(PathBuf, Option<&'static str>), String> {
    let want = if est { "cd-hit-est" } else { "cd-hit" };
    let name = bin.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let (stem, ext) = match name.to_ascii_lowercase().strip_suffix(".exe") {
        Some(s) => (s.to_string(), ".exe"),
        None => (name.to_ascii_lowercase(), ""),
    };
    if stem != "cd-hit" && stem != "cd-hit-est" {
        return Ok((bin.to_path_buf(), Some(want)));
    }
    if stem == want {
        return Ok((bin.to_path_buf(), None));
    }
    let sibling = bin.with_file_name(format!("{want}{ext}"));
    if sibling.is_file() {
        return Ok((sibling, None));
    }
    Err(format!(
        "{} is the {stem} program, but this input needs {want}, and no {want}{ext} sits \
         beside it. Point $DOMAINATOREDUX_CDHIT (or --cdhit_path) at the `cdhit` multiplexer \
         or at {want}{ext}.",
        bin.display()
    ))
}

fn run_cdhit(
    opts: &DeduplicateOptions,
    tmp: &TempDir,
    input_fasta: &Path,
    cpus: usize,
    est: bool,
) -> Result<Vec<(String, Vec<String>)>, String> {
    let path = if est {
        opts.cdhit_est_path.as_ref().or(opts.bin_path.as_ref())
    } else {
        opts.bin_path.as_ref()
    };
    let bin = match path {
        Some(p) => PathBuf::from(p),
        None => super::external::CDHIT.find(),
    };
    let word_size = cdhit_word_size(opts.id, est)?;
    // cd-hit derives the cluster file's name from `-o`, so both live in the temp dir.
    let out_fasta = tmp.path().join("output.fasta");
    let clstr = tmp.path().join("output.fasta.clstr");

    let (exe, program) = cdhit_command(&bin, est)?;
    let mut cmd = Command::new(&exe);
    super::external::hide_console(&mut cmd);
    if let Some(program) = program {
        cmd.arg(program);
    }
    // Argument order mirrors Python's `CdHit.search_args`. `-d 0` is load-bearing:
    // it stops cd-hit truncating names in the `.clstr`, which is what the parser
    // reads the cluster membership out of.
    cmd.arg("-n")
        .arg(word_size.to_string())
        .arg("-M")
        .arg("0")
        .arg("-T")
        .arg(cpus.to_string())
        .arg("-d")
        .arg("0")
        .arg("-c")
        .arg(format!("{}", opts.id))
        .arg("-i")
        .arg(input_fasta)
        .arg("-o")
        .arg(&out_fasta);
    let mut extra = extra_args(
        &opts.extra_args,
        &["-n", "-M", "-T", "-d", "-c", "-i", "-o"],
        "--cdhit_args",
    )?;
    // Only cd-hit-est has a both-strands mode; an explicit `--cdhit_args` `-r` wins.
    if est && opts.both_strands && !extra.iter().any(|a| a == "-r") {
        extra.extend(["-r".to_string(), "1".to_string()]);
    }
    cmd.args(extra);

    let out = cmd.output().map_err(|e| {
        let hint = format!(
            "Install cdhit: {}; or set ${CDHIT_ENV_VAR} to its path.",
            super::external::CDHIT.install
        );
        spawn_error(e, &exe, &hint).to_string()
    })?;
    if !out.status.success() {
        return Err(format!(
            "{} failed:\n{}{}",
            if est { "cd-hit-est" } else { "cd-hit" },
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    parse_clstr(&clstr)
}

/// Parse a cd-hit `.clstr` file into `(representative, members)` clusters, in the
/// file's own order (cd-hit sorts clusters by representative length).
///
/// A member line is `<n>\t<len>aa, ><name>... <*|at [+/-/]<pct>%>`; the
/// representative is the one marked `*`, and it is also a member of its own
/// cluster. `-d 0` guarantees `<name>` is the full id up to the first whitespace.
fn parse_clstr(path: &Path) -> Result<Vec<(String, Vec<String>)>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut clusters: Vec<(String, Vec<String>)> = Vec::new();
    let mut center: Option<String> = None;
    let mut members: Vec<String> = Vec::new();
    // Python would silently key a cluster on `None`; a cluster with no `*` means the
    // file is malformed, and saying so beats emitting a nonsense representative.
    let mut flush =
        |center: &mut Option<String>, members: &mut Vec<String>| -> Result<(), String> {
            if members.is_empty() {
                return Ok(());
            }
            let c = center.take().ok_or_else(|| {
                format!(
                    "{}: cluster with no representative ('*') entry",
                    path.display()
                )
            })?;
            clusters.push((c, std::mem::take(members)));
            Ok(())
        };

    for line in std::io::BufReader::new(file).lines() {
        let line = line.map_err(|e| format!("{}: {e}", path.display()))?;
        if line.starts_with('>') {
            flush(&mut center, &mut members)?;
            continue;
        }
        if line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 4 {
            return Err(format!(
                "{}: malformed cluster line {line:?}",
                path.display()
            ));
        }
        // Exactly one leading `>` and exactly one trailing `...` (Python's
        // `parts[2][1:-3]`) -- trimming greedily would eat a dot that is part of an
        // accession such as `NZ_CP012345.`.
        let raw = fields[2];
        let name = raw
            .strip_prefix('>')
            .and_then(|n| n.strip_suffix("..."))
            .ok_or_else(|| format!("{}: unexpected cluster entry {raw:?}", path.display()))?
            .to_string();
        if fields[3] == "*" {
            center = Some(name.clone());
        }
        members.push(name);
    }
    flush(&mut center, &mut members)?;
    Ok(clusters)
}

fn run_usearch(
    opts: &DeduplicateOptions,
    tmp: &TempDir,
    input_fasta: &Path,
    cpus: usize,
    _records: &[SeqRecord],
) -> Result<Vec<(String, Vec<String>)>, String> {
    let bin = match &opts.bin_path {
        Some(p) => PathBuf::from(p),
        None => super::external::USEARCH.find(),
    };
    let centroids = tmp.path().join("output.fasta");
    let uc = tmp.path().join("output.uc");
    let mut cmd = Command::new(&bin);
    super::external::hide_console(&mut cmd);
    cmd.arg("-sort")
        .arg("length")
        .arg("-threads")
        .arg(cpus.to_string())
        .arg("-cluster_fast")
        .arg(input_fasta)
        .arg("-id")
        .arg(format!("{}", opts.id))
        .arg("-centroids")
        .arg(&centroids)
        .arg("-uc")
        .arg(&uc);
    let mut extra = extra_args(
        &opts.extra_args,
        &[
            "-sort",
            "-threads",
            "-cluster_fast",
            "-id",
            "-centroids",
            "-uc",
        ],
        "--usearch_args",
    )?;
    if opts.both_strands && !extra.iter().any(|a| a == "-strand") {
        extra.extend(["-strand".to_string(), "both".to_string()]);
    }
    cmd.args(extra);
    let out = cmd.output().map_err(|e| {
        spawn_error(e, &bin, "Install usearch or set $DOMAINATOREDUX_USEARCH.").to_string()
    })?;
    if !out.status.success() {
        return Err(format!(
            "usearch failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    parse_uc(&uc)
}

/// Parse a USEARCH `.uc` cluster file into `(centroid, members)` clusters.
fn parse_uc(path: &Path) -> Result<Vec<(String, Vec<String>)>, String> {
    let mut order: Vec<String> = Vec::new();
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    for line in std::io::BufReader::new(file).lines() {
        let line = line.map_err(|e| e.to_string())?;
        let parts: Vec<&str> = line.trim().split('\t').collect();
        if parts.is_empty() {
            continue;
        }
        match parts[0] {
            "S" if parts.len() > 8 => {
                let c = parts[8].to_string();
                map.entry(c.clone()).or_insert_with(|| {
                    order.push(c.clone());
                    Vec::new()
                });
                map.get_mut(&c).unwrap().push(parts[8].to_string());
            }
            "H" if parts.len() > 9 => {
                let c = parts[9].to_string();
                map.entry(c.clone()).or_insert_with(|| {
                    order.push(c.clone());
                    Vec::new()
                });
                map.get_mut(&c).unwrap().push(parts[8].to_string());
            }
            _ => {}
        }
    }
    Ok(order
        .into_iter()
        .map(|c| (c.clone(), map.remove(&c).unwrap()))
        .collect())
}

/// The user's extra arguments for a clusterer (upstream `external.append_extra_args`):
/// an error naming `option` if they set one of `set` (the options this tool passes
/// itself), else the arguments.
fn extra_args(extra: &[String], set: &[&str], option: &str) -> Result<Vec<String>, String> {
    let mut conflicts: Vec<&str> = set
        .iter()
        .copied()
        .filter(|flag| extra.iter().any(|a| a == flag))
        .collect();
    conflicts.sort_unstable();
    if conflicts.is_empty() {
        return Ok(extra.to_vec());
    }
    Err(format!(
        "{option} cannot set {}: Domainator sets {} itself.",
        conflicts.join(", "),
        if conflicts.len() == 1 { "it" } else { "them" }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seq::{Annotations, SeqRecord, Topology};

    fn rec(id: &str, seq: &[u8], mol: &str) -> SeqRecord {
        SeqRecord {
            id: id.into(),
            name: id.into(),
            description: String::new(),
            seq: seq.to_vec(),
            molecule_type: Some(mol.into()),
            topology: Topology::Linear,
            annotations: Annotations::default(),
            features: Vec::new(),
        }
    }

    /// The cluster table follows the clustering's own order, but records are
    /// always emitted in input order -- so `clusters` and `representatives` are
    /// ordered independently. usearch sorts by length, which is how its `.uc`
    /// comes back with cluster 5 before cluster 1.
    #[test]
    fn cluster_order_is_the_tools_and_representatives_are_input_order() {
        let members = HashMap::from([(5, vec![5, 7]), (1, vec![1]), (3, vec![3, 4])]);
        let r = finalize(vec![5, 1, 3], members).unwrap();
        assert_eq!(
            r.clusters,
            vec![(5, vec![5, 7]), (1, vec![1]), (3, vec![3, 4])]
        );
        assert_eq!(r.representatives, vec![1, 3, 5]);
    }

    #[test]
    fn hash_dedups_exact_and_star_stripped() {
        // Records 0 and 2 are identical after upper + `*` strip; 1 is distinct.
        let recs = vec![
            rec("a", b"ACDEF", "protein"),
            rec("b", b"GGGGG", "protein"),
            rec("c", b"*acdef*", "protein"),
        ];
        let r = deduplicate(&recs, &DeduplicateOptions::default()).unwrap();
        assert_eq!(r.representatives, vec![0, 1]);
        assert_eq!(r.clusters, vec![(0, vec![0, 2]), (1, vec![1])]);
    }

    #[test]
    fn hash_both_strands_groups_revcomp() {
        // Record 1 is the reverse-complement of record 0 -> same cluster.
        let recs = vec![
            rec("a", b"AAATTTGGG", "nucleotide"),
            rec("b", &revcomp(b"AAATTTGGG"), "nucleotide"),
        ];
        let opts = DeduplicateOptions {
            fasta_type: "nucleotide".into(),
            both_strands: true,
            ..Default::default()
        };
        let r = deduplicate(&recs, &opts).unwrap();
        assert_eq!(r.representatives, vec![0]);
        assert_eq!(r.clusters, vec![(0, vec![0, 1])]);
    }

    /// The `.clstr` is the only place cd-hit reports cluster membership, and its
    /// member lines have two shapes -- `at 96.30%` for protein, `at -/100.00%` for
    /// cd-hit-est, where the extra strand field shifts nothing but is easy to get
    /// wrong. The representative is the `*` entry and is a member of its own cluster.
    #[test]
    fn parse_clstr_reads_clusters_in_file_order() {
        let dir = std::env::temp_dir().join(format!("domainator_clstr_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("output.fasta.clstr");
        std::fs::write(
            &path,
            ">Cluster 0\n\
             0\t254aa, >4... *\n\
             1\t254aa, >10... at 100.00%\n\
             >Cluster 1\n\
             0\t36nt, >0... *\n\
             1\t36nt, >1... at -/100.00%\n\
             >Cluster 2\n\
             0\t26aa, >7... *\n",
        )
        .unwrap();
        let clusters = parse_clstr(&path).unwrap();
        assert_eq!(
            clusters,
            vec![
                ("4".to_string(), vec!["4".to_string(), "10".to_string()]),
                ("0".to_string(), vec!["0".to_string(), "1".to_string()]),
                ("7".to_string(), vec!["7".to_string()]),
            ]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Python keys a `*`-less cluster on `None` and only fails later, somewhere
    /// unrelated. A malformed cluster file should say so where it is read.
    #[test]
    fn parse_clstr_rejects_a_cluster_with_no_representative() {
        let dir = std::env::temp_dir().join(format!("domainator_clstr_bad_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("output.fasta.clstr");
        std::fs::write(&path, ">Cluster 0\n0\t254aa, >4... at 100.00%\n").unwrap();
        let err = parse_clstr(&path).unwrap_err();
        assert!(err.contains("no representative"), "{err}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The word size is derived from `--id`, not supplied, so an off-by-one at a
    /// threshold silently changes cd-hit's seeding and its clustering with it.
    #[test]
    fn cdhit_word_size_matches_the_upstream_tables() {
        for (id, want) in [
            (1.0, 5),
            (0.71, 5),
            (0.7, 4),
            (0.61, 4),
            (0.6, 3),
            (0.51, 3),
            (0.5, 2),
            (0.4, 2),
        ] {
            assert_eq!(cdhit_word_size(id, false).unwrap(), want, "protein id {id}");
        }
        for (id, want) in [
            (1.0, 10),
            (0.96, 10),
            (0.95, 9),
            (0.9, 7),
            (0.88, 6),
            (0.85, 5),
            (0.8, 5),
        ] {
            assert_eq!(cdhit_word_size(id, true).unwrap(), want, "est id {id}");
        }
        assert!(cdhit_word_size(0.39, false).unwrap_err().contains(">= 0.4"));
        assert!(cdhit_word_size(0.79, true).unwrap_err().contains(">= 0.8"));
    }

    /// The Rust port is one `cdhit` multiplexer taking the program name as its first
    /// argument, but `--cdhit_path` may name a C++ `cd-hit`/`cd-hit-est` (or a symlink
    /// to those names), which must be run directly or it sees `cd-hit` as an input.
    #[test]
    fn cdhit_command_only_prefixes_the_multiplexer() {
        let go = |p: &str, est: bool| cdhit_command(Path::new(p), est).unwrap();
        assert_eq!(go("cdhit", false), (PathBuf::from("cdhit"), Some("cd-hit")));
        assert_eq!(
            go("cdhit", true),
            (PathBuf::from("cdhit"), Some("cd-hit-est"))
        );
        assert_eq!(
            go(r"C:\tools\cdhit.exe", true),
            (PathBuf::from(r"C:\tools\cdhit.exe"), Some("cd-hit-est"))
        );
        assert_eq!(
            go("/usr/bin/cd-hit", false),
            (PathBuf::from("/usr/bin/cd-hit"), None)
        );
        assert_eq!(
            go("/opt/conda/bin/cd-hit-est", true),
            (PathBuf::from("/opt/conda/bin/cd-hit-est"), None)
        );
    }

    /// Running the protein program over nucleotides returns a plausible-looking wrong
    /// answer rather than failing, so a name that cannot serve the request has to be
    /// caught here. The sibling is preferred when the C++ suite's layout provides it.
    #[test]
    fn cdhit_command_resolves_or_rejects_a_mismatched_program_name() {
        let dir = std::env::temp_dir().join(format!("domainator_cdhit_sib_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let protein = dir.join("cd-hit");
        std::fs::write(&protein, b"").unwrap();

        // No cd-hit-est beside it yet: refuse rather than cluster DNA as protein.
        let err = cdhit_command(&protein, true).unwrap_err();
        assert!(err.contains("needs cd-hit-est"), "{err}");

        // The real conda/C++ layout ships both; use the right one.
        let est = dir.join("cd-hit-est");
        std::fs::write(&est, b"").unwrap();
        assert_eq!(cdhit_command(&protein, true).unwrap(), (est, None));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Below the word-size tables cd-hit cannot run at all, so the error has to come
    /// out of `deduplicate` itself rather than as a subprocess failure -- and it must
    /// not depend on a binary being installed.
    #[test]
    fn cdhit_rejects_an_identity_below_the_table_without_running_anything() {
        let recs = vec![rec("a", b"ACDEF", "protein")];
        let opts = DeduplicateOptions {
            algorithm: "cd-hit".into(),
            id: 0.3,
            ..Default::default()
        };
        let Err(err) = deduplicate(&recs, &opts) else {
            panic!("id 0.3 should be rejected")
        };
        assert!(err.contains(">= 0.4"), "{err}");
    }

    #[test]
    fn hash_requires_id_one_and_both_strands_needs_nucleotide() {
        let recs = vec![rec("a", b"ACDEF", "protein")];
        let bad_id = DeduplicateOptions {
            id: 0.9,
            ..Default::default()
        };
        assert!(deduplicate(&recs, &bad_id).is_err());
        let bad_strand = DeduplicateOptions {
            both_strands: true,
            fasta_type: "protein".into(),
            ..Default::default()
        };
        assert!(deduplicate(&recs, &bad_strand).is_err());
    }

    /// Upstream `append_extra_args`: extra arguments may add options, never override
    /// one the tool sets, and the error names the option they came from.
    #[test]
    fn extra_args_cannot_set_what_the_tool_sets() {
        let words = |s: &str| s.split(' ').map(String::from).collect::<Vec<_>>();
        assert_eq!(
            extra_args(&words("-s 0.9"), &["-c", "-n"], "--cdhit_args").unwrap(),
            ["-s", "0.9"]
        );
        assert_eq!(
            extra_args(&words("-n 5 -s 0.9 -c 1"), &["-n", "-c"], "--cdhit_args").unwrap_err(),
            "--cdhit_args cannot set -c, -n: Domainator sets them itself."
        );
        assert_eq!(
            extra_args(&words("--threads 2"), &["--threads"], "--diamond_args").unwrap_err(),
            "--diamond_args cannot set --threads: Domainator sets it itself."
        );
    }
}
