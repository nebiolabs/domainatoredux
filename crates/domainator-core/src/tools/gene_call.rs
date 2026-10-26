//! Gene calling: add prodigal-style `CDS` features to nucleotide contigs.
//!
//! Upstream calls genes in-process with pyrodigal (`domainate.prodigal_CDS_annotate`,
//! behind `domainate`/`domain_search --gene_call`). Every Prodigal implementation is
//! GPL, so this port drives **Orphos** (a Rust reimplementation of Prodigal,
//! GPL-3.0-or-later) as an external binary instead: a subprocess, located by
//! `find_binary`, never linked or bundled -- the same arrangement as cd-hit and
//! usearch in `deduplicate_genbank`.
//!
//! Orphos only reports gene coordinates usefully: its GenBank output has no strand or
//! partial markers, it has no protein output, and `-t` is a no-op. So it is run with
//! `-f gff`, and everything upstream derives from a pyrodigal gene -- the edge-extended
//! partial location, `/codon_start`, the translation, and the origin-crossing genes of
//! a circular contig -- is rebuilt here from the GFF, function for function.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;
use std::sync::{Mutex, OnceLock};

use super::external::{spawn_error, write_fasta_ids, TempDir};
use super::neighborhood::cds_unique_name;
use crate::seq::location::{CompoundOp, Location, Part};
use crate::seq::translate::{genetic_code, is_searchable_cds, revcomp, translate_codon_with};
use crate::seq::{SeqFeature, SeqRecord, Topology};
use crate::DOMAIN_FEATURE_NAME;

/// The Orphos executable's name (`orphos.exe` on Windows, added by `find_binary`).
pub const ORPHOS_BIN_NAME: &str = "orphos";
/// Environment variable naming the Orphos executable; checked first.
pub const ORPHOS_ENV_VAR: &str = "DOMAINATOREDUX_ORPHOS";
/// The oldest Orphos whose flags mean what this module passes: before 0.4.0 `-g` was
/// ignored and `-c`/`-m`/`-q` took values.
pub const MIN_ORPHOS_VERSION: &str = "0.4.0";
/// The values `--mode` accepts.
pub const GENE_CALL_MODES: [&str; 2] = ["meta", "single"];
/// The values `--gene_call` (and the `gene_call` tool's `--scope`) accept.
pub const GENE_CALL_SCOPES: [&str; 2] = ["all", "unannotated"];

/// Bases on each side of the origin of a circular contig searched for genes crossing
/// it (upstream `CIRCULAR_GENE_CALL_WINDOW`). Longer than nearly all genes.
const CIRCULAR_GENE_CALL_WINDOW: usize = 20000;
/// Prodigal never predicts genes overlapping by more than this on the same strand
/// (`MAX_SAM_OVLP`), or on opposite strands (`MAX_OPP_OVLP`). An origin-crossing gene
/// overlapping a main-pass gene by more conflicts with it.
const MAX_SAME_STRAND_OVERLAP: i64 = 60;
const MAX_OPPOSITE_STRAND_OVERLAP: i64 = 200;

/// Prodigal's two procedures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GeneCallMode {
    /// Pre-trained metagenomic models, chosen per sequence (`-p meta`). Upstream's
    /// only mode; right for metagenomes, plasmids, phages and short contigs.
    #[default]
    Meta,
    /// Train on the input itself (`-p single`), every record together -- right for
    /// one complete isolate genome.
    Single,
}

impl FromStr for GeneCallMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "meta" => Ok(GeneCallMode::Meta),
            "single" => Ok(GeneCallMode::Single),
            other => Err(format!(
                "mode must be one of {}, not {other}",
                GENE_CALL_MODES.join(", ")
            )),
        }
    }
}

/// Which contigs get genes called (upstream's `--gene_call` values).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeneCallScope {
    /// Delete every existing `CDS` (and the `Domainator` annotations anchored on
    /// them) and re-call every nucleotide contig.
    All,
    /// Call only the nucleotide contigs with no (non-pseudo) CDS; keep the rest.
    Unannotated,
}

impl FromStr for GeneCallScope {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "all" => Ok(GeneCallScope::All),
            "unannotated" => Ok(GeneCallScope::Unannotated),
            other => Err(format!(
                "gene_call must be one of {}, not {other}",
                GENE_CALL_SCOPES.join(", ")
            )),
        }
    }
}

/// How Orphos is run.
#[derive(Debug, Clone, Default)]
pub struct GeneCallOptions {
    pub mode: GeneCallMode,
    /// `-g`: the genetic code. `None` = Orphos's default (11 in single mode; meta mode
    /// picks each sequence's table from its model, 11 or 4).
    pub translation_table: Option<u8>,
    /// `-c`: genes may not run off the edges of a sequence.
    pub closed: bool,
    /// `-m`: treat runs of `N` as masked (no genes across them).
    pub mask: bool,
    /// Threads for Orphos (`0` = all cores), passed as `RAYON_NUM_THREADS`.
    pub cpu: usize,
    /// Explicit Orphos executable, overriding the locator.
    pub bin_path: Option<String>,
}

/// A gene-calling request: which contigs, and how.
#[derive(Debug, Clone)]
pub struct GeneCall {
    pub scope: GeneCallScope,
    pub options: GeneCallOptions,
}

/// The standalone `gene_call` tool: clear existing annotations when the scope is
/// [`GeneCallScope::All`], then call genes on every nucleotide contig left with no
/// searchable CDS. Returns the number of CDS features added.
pub fn gene_call(records: &mut [SeqRecord], request: &GeneCall) -> io::Result<usize> {
    if request.scope == GeneCallScope::All {
        clear_cds_annotations(records);
    }
    let targets: Vec<usize> = records
        .iter()
        .enumerate()
        .filter(|(_, rec)| !rec.is_protein() && !rec.features.iter().any(is_searchable_cds))
        .map(|(i, _)| i)
        .collect();
    call_genes(records, &targets, &request.options)
}

/// Drop every `CDS` and `Domainator` feature (upstream `clean_rec` with
/// `clear_CDS_annotations` and `clear_domainator_annotations`, which `gene_call='all'`
/// sets together: a domain annotation is anchored on a CDS that is about to be
/// replaced).
pub fn clear_cds_annotations(records: &mut [SeqRecord]) {
    for rec in records {
        rec.features
            .retain(|f| f.kind != "CDS" && f.kind != DOMAIN_FEATURE_NAME);
    }
}

/// Call genes on `records[i]` for each `i` in `targets` with one Orphos run, appending
/// the CDS features to each record (upstream `prodigal_CDS_annotate`). Protein and
/// empty records are skipped. Returns the number of CDS features added.
pub fn call_genes(
    records: &mut [SeqRecord],
    targets: &[usize],
    opts: &GeneCallOptions,
) -> io::Result<usize> {
    let targets: Vec<usize> = targets
        .iter()
        .copied()
        .filter(|&i| !records[i].is_protein() && !records[i].seq.is_empty())
        .collect();
    if targets.is_empty() {
        return Ok(0);
    }
    if let Some(table) = opts.translation_table {
        if genetic_code(table).is_none() {
            return Err(invalid(format!(
                "translation table {table} is not supported (Prodigal accepts 1-6, 9-16 and 21-25)"
            )));
        }
    }

    // Sequences go to Orphos under their position in `targets` ("0", "1", ...), never
    // the user's own ids, which may hold spaces or repeat; a circular contig's origin
    // window follows as "w<n>".
    let mut fasta: Vec<(String, Vec<u8>)> = Vec::new();
    let mut windows: HashMap<usize, OriginWindow> = HashMap::new();
    for (n, &i) in targets.iter().enumerate() {
        let seq = records[i].seq.to_ascii_uppercase();
        if records[i].topology == Topology::Circular {
            let window = OriginWindow::new(seq.len());
            fasta.push((format!("w{n}"), window.sequence(&seq)));
            windows.insert(n, window);
        }
        fasta.push((n.to_string(), seq));
    }
    // The main sequences first, then the windows: Orphos numbers its output in input
    // order, and in single mode the windows are training data too, so keep them last.
    fasta.sort_by_key(|(id, _)| id.starts_with('w'));

    let tmp = TempDir::new("domainatoredux_genecall")?;
    let input = tmp.path().join("input.fasta");
    write_fasta_ids(&fasta, &input)?;
    drop(fasta);
    let gff_path = tmp.path().join("genes.gff");
    run_orphos(opts, &input, &gff_path)?;
    let gff = std::fs::read_to_string(&gff_path)?;
    let mut calls = parse_gff(&gff)?;

    let mut added = 0;
    for (n, &i) in targets.iter().enumerate() {
        let rec = &mut records[i];
        let seq = rec.seq.to_ascii_uppercase();
        let main = calls.remove(&n.to_string()).unwrap_or_default();
        let table = main.table.or(opts.translation_table).unwrap_or(11);
        let code = table_code(table)?;
        let circular = windows.contains_key(&n);

        // (location, translation, codon_start)
        let mut genes: Vec<(Location, String, usize)> = Vec::new();
        for pred in &main.genes {
            if circular && (pred.partial_begin || pred.partial_end) {
                continue; // called whole from the origin window, if it crosses the origin
            }
            let (location, codon_start) = linear_location(pred, seq.len());
            genes.push((location, pred.translate(&seq, code), codon_start));
        }
        if let Some(window) = windows.get(&n) {
            let window_calls = calls.remove(&format!("w{n}")).unwrap_or_default();
            let window_seq = window.sequence(&seq);
            let window_code = table_code(window_calls.table.unwrap_or(table))?;
            let existing: Vec<Location> = genes.iter().map(|g| g.0.clone()).collect();
            for (location, pred) in window.origin_genes(&window_calls.genes, &existing) {
                genes.push((location, pred.translate(&window_seq, window_code), 1));
            }
        }

        for (k, (location, translation, codon_start)) in genes.into_iter().enumerate() {
            let mut feature = SeqFeature::new("CDS", location);
            feature.set("translation", translation);
            feature.set("gene_id", format!("{}_{}", rec.id, k + 1));
            if codon_start != 1 {
                feature.set("codon_start", codon_start.to_string());
            }
            let cds_id = cds_unique_name(&feature);
            feature.set("cds_id", cds_id);
            rec.features.push(feature);
            added += 1;
        }
    }
    Ok(added)
}

fn invalid(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, msg)
}

fn table_code(table: u8) -> io::Result<&'static [u8; 64]> {
    genetic_code(table).ok_or_else(|| {
        io::Error::other(format!(
            "orphos reported translation table {table}, which is not a known genetic code"
        ))
    })
}

/// The Orphos executable: `bin_path`, else `$DOMAINATOREDUX_ORPHOS`/`$ORPHOS`, a bundled
/// `external/orphos/orphos`, or `orphos` on PATH.
fn orphos_binary(opts: &GeneCallOptions) -> PathBuf {
    match &opts.bin_path {
        Some(p) => PathBuf::from(p),
        None => super::external::ORPHOS.find(),
    }
}

const INSTALL_HINT: &str = "Install orphos (https://github.com/FullHuman/orphos/releases, \
     or the GUI's External tools panel) or set $DOMAINATOREDUX_ORPHOS.";

fn run_orphos(opts: &GeneCallOptions, input: &Path, gff: &Path) -> io::Result<()> {
    let bin = orphos_binary(opts);
    check_version(&bin)?;
    let mut cmd = Command::new(&bin);
    super::external::hide_console(&mut cmd);
    // `-p` is always passed: Orphos defaults to single mode, and reads any value other
    // than `meta` as single.
    cmd.arg("-i")
        .arg(input)
        .arg("-o")
        .arg(gff)
        .arg("-f")
        .arg("gff")
        .arg("-p")
        .arg(match opts.mode {
            GeneCallMode::Meta => "meta",
            GeneCallMode::Single => "single",
        })
        .arg("-q");
    if let Some(table) = opts.translation_table {
        cmd.arg("-g").arg(table.to_string());
    }
    if opts.closed {
        cmd.arg("-c");
    }
    if opts.mask {
        cmd.arg("-m");
    }
    // Orphos has no threads flag; it runs on Rayon's global pool, which reads this.
    cmd.env(
        "RAYON_NUM_THREADS",
        crate::parallel::resolve_cpu(opts.cpu).to_string(),
    );
    let out = cmd
        .output()
        .map_err(|e| spawn_error(e, &bin, INSTALL_HINT))?;
    if !out.status.success() {
        return Err(io::Error::other(format!(
            "orphos failed:\n{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(())
}

/// Refuse an Orphos older than [`MIN_ORPHOS_VERSION`], once per executable per process.
fn check_version(bin: &Path) -> io::Result<()> {
    static CHECKED: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    let checked = CHECKED.get_or_init(|| Mutex::new(HashSet::new()));
    if checked.lock().map(|c| c.contains(bin)).unwrap_or(false) {
        return Ok(());
    }
    let mut cmd = Command::new(bin);
    super::external::hide_console(&mut cmd);
    let out = cmd
        .arg("--version")
        .output()
        .map_err(|e| spawn_error(e, bin, INSTALL_HINT))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let version = text.split_whitespace().nth(1).unwrap_or("");
    if !out.status.success() || !version_at_least(version, MIN_ORPHOS_VERSION) {
        return Err(io::Error::other(format!(
            "'{}' reports version '{}'; gene calling needs orphos {MIN_ORPHOS_VERSION} or newer. \
             {INSTALL_HINT}",
            bin.display(),
            text.trim()
        )));
    }
    if let Ok(mut c) = checked.lock() {
        c.insert(bin.to_path_buf());
    }
    Ok(())
}

/// Dot-separated numeric comparison (`"0.10.0" >= "0.4.0"`); a non-number is 0.
pub fn version_at_least(version: &str, min: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> {
        v.trim_start_matches('v')
            .split('.')
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    };
    let (have, want) = (parse(version), parse(min));
    for k in 0..have.len().max(want.len()) {
        let (h, w) = (
            have.get(k).copied().unwrap_or(0),
            want.get(k).copied().unwrap_or(0),
        );
        if h != w {
            return h > w;
        }
    }
    true
}

/// One predicted gene, as Orphos reports it: 1-based inclusive `begin..=end`, and
/// whether the gene runs off the left/right end of the sequence (GFF `partial=XY`,
/// which is in coordinate order regardless of strand -- pyrodigal's
/// `partial_begin`/`partial_end`).
#[derive(Debug, Clone, PartialEq)]
struct Pred {
    begin: usize,
    end: usize,
    strand: i32,
    partial_begin: bool,
    partial_end: bool,
}

impl Pred {
    /// Is the gene's 5' end partial (it has no start codon)?
    fn five_prime_partial(&self) -> bool {
        if self.strand == -1 {
            self.partial_end
        } else {
            self.partial_begin
        }
    }

    /// The protein, as pyrodigal's `Gene.translate()` gives it: the predicted span
    /// translated with `code`, its first codon read as `M` when it is a real start
    /// (Prodigal starts genes at ATG, GTG or TTG, all initiator methionine), the stop
    /// kept as a trailing `*`, and `X` for a codon with an ambiguous base.
    fn translate(&self, seq: &[u8], code: &[u8; 64]) -> String {
        let span = &seq[self.begin - 1..self.end.min(seq.len())];
        let nt = if self.strand == -1 {
            revcomp(span)
        } else {
            span.to_vec()
        };
        let mut protein: Vec<u8> = nt
            .as_chunks::<3>()
            .0
            .iter()
            .map(|&[c0, c1, c2]| translate_codon_with(code, c0, c1, c2))
            .collect();
        if !self.five_prime_partial() {
            if let Some(first) = protein.first_mut() {
                *first = b'M';
            }
        }
        String::from_utf8_lossy(&protein).into_owned()
    }
}

/// Orphos's calls for one sequence.
#[derive(Debug, Default)]
struct SeqCalls {
    /// `transl_table` from the sequence's `# Model Data` line.
    table: Option<u8>,
    genes: Vec<Pred>,
}

/// Parse Orphos's (Prodigal-layout) GFF into calls per sequence id. Every sequence
/// gets a `# Sequence Data: ...;seqhdr="<header>"` line and a `# Model Data:
/// ...;transl_table=N;...` line, genes or not.
fn parse_gff(text: &str) -> io::Result<HashMap<String, SeqCalls>> {
    let bad = |line: &str| io::Error::other(format!("unexpected orphos GFF line: {line}"));
    let mut out: HashMap<String, SeqCalls> = HashMap::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("# Sequence Data:") {
            let header = attribute(rest, "seqhdr").unwrap_or_default();
            let id = header
                .trim_matches('"')
                .split_whitespace()
                .next()
                .unwrap_or("");
            current = Some(id.to_string());
            out.entry(id.to_string()).or_default();
        } else if let Some(rest) = line.strip_prefix("# Model Data:") {
            if let (Some(id), Some(table)) = (&current, attribute(rest, "transl_table")) {
                let table = table.parse().map_err(|_| bad(line))?;
                out.entry(id.clone()).or_default().table = Some(table);
            }
        } else if line.starts_with('#') || line.trim().is_empty() {
            continue;
        } else {
            let cols: Vec<&str> = line.split('\t').collect();
            if cols.len() < 9 {
                return Err(bad(line));
            }
            let partial = attribute(cols[8], "partial").ok_or_else(|| bad(line))?;
            let partial = partial.as_bytes();
            if partial.len() != 2 {
                return Err(bad(line));
            }
            let pred = Pred {
                begin: cols[3].parse().map_err(|_| bad(line))?,
                end: cols[4].parse().map_err(|_| bad(line))?,
                strand: match cols[6] {
                    "-" => -1,
                    _ => 1,
                },
                partial_begin: partial[0] == b'1',
                partial_end: partial[1] == b'1',
            };
            if pred.begin == 0 || pred.end < pred.begin {
                return Err(bad(line));
            }
            out.entry(cols[0].to_string()).or_default().genes.push(pred);
        }
    }
    Ok(out)
}

/// The value of `key` in a `k=v;k=v` attribute list.
fn attribute(list: &str, key: &str) -> Option<String> {
    list.trim().split(';').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k.trim() == key).then(|| v.to_string())
    })
}

/// The location of a gene on a linear sequence, and its `/codon_start`
/// (upstream `_prodigal_linear_location`).
///
/// A gene that runs off the edge of the sequence is extended to the edge (prodigal
/// stops it at the last whole codon) and the extended end is marked `<`/`>`;
/// `codon_start` records how many bases the extension added to the 5' end.
fn linear_location(pred: &Pred, len: usize) -> (Location, usize) {
    let len = len as i64;
    let mut start = pred.begin as i64 - 1;
    let mut end = pred.end as i64;
    let mut five_prime_extension = 0;
    if pred.partial_begin && start < 3 {
        if pred.strand != -1 {
            five_prime_extension = start;
        }
        start = 0;
    }
    if pred.partial_end && len - end < 3 {
        if pred.strand == -1 {
            five_prime_extension = len - end;
        }
        end = len;
    }
    let part: Part = (
        start,
        end,
        pred.strand,
        pred.partial_begin,
        pred.partial_end,
    );
    (
        Location {
            operator: None,
            between: false,
            parts: vec![part],
        },
        usize::try_from(five_prime_extension).unwrap_or(0) + 1,
    )
}

/// The stretch of a circular contig that spans its origin: the last `left` bases
/// followed by the first `right` (upstream `_prodigal_origin_genes`). Short contigs
/// are rotated whole, so no base is used twice.
#[derive(Debug, Clone, Copy)]
struct OriginWindow {
    len: usize,
    left: usize,
    right: usize,
}

impl OriginWindow {
    fn new(len: usize) -> Self {
        OriginWindow {
            len,
            left: CIRCULAR_GENE_CALL_WINDOW.min(len - len / 2),
            right: CIRCULAR_GENE_CALL_WINDOW.min(len / 2),
        }
    }

    fn sequence(&self, seq: &[u8]) -> Vec<u8> {
        let mut window = seq[self.len - self.left..].to_vec();
        window.extend_from_slice(&seq[..self.right]);
        window
    }

    /// The window's genes that cross the junction, mapped back onto the contig as
    /// joins across the origin, in order along the gene. The rest of the window's
    /// genes are ignored -- the main pass called them with the whole sequence -- and
    /// so is a gene overlapping a main-pass gene by more than prodigal itself allows.
    fn origin_genes<'a>(
        &self,
        preds: &'a [Pred],
        existing: &[Location],
    ) -> Vec<(Location, &'a Pred)> {
        let (len, left) = (self.len as i64, self.left as i64);
        let mut out = Vec::new();
        for pred in preds {
            let start = pred.begin as i64 - 1;
            let end = pred.end as i64;
            if pred.partial_begin || pred.partial_end || !(start < left && left < end) {
                continue;
            }
            let before_origin: Part = (len - left + start, len, pred.strand, false, false);
            let after_origin: Part = (0, end - left, pred.strand, false, false);
            let parts = if pred.strand != -1 {
                vec![before_origin, after_origin]
            } else {
                vec![after_origin, before_origin]
            };
            let location = Location {
                operator: Some(CompoundOp::Join),
                between: false,
                parts,
            };
            let conflict = existing.iter().any(|other| {
                let max_overlap = if other.strand() == location.strand() {
                    MAX_SAME_STRAND_OVERLAP
                } else {
                    MAX_OPPOSITE_STRAND_OVERLAP
                };
                let overlap: i64 = location
                    .intervals()
                    .iter()
                    .flat_map(|a| other.intervals().into_iter().map(move |b| (*a, b)))
                    .map(|(a, b)| (a.1.min(b.1) - a.0.max(b.0)).max(0))
                    .sum();
                overlap > max_overlap
            });
            if !conflict {
                out.push((location, pred));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seq::translate::genetic_code;
    use crate::seq::Annotations;

    fn pred(begin: usize, end: usize, strand: i32, pb: bool, pe: bool) -> Pred {
        Pred {
            begin,
            end,
            strand,
            partial_begin: pb,
            partial_end: pe,
        }
    }

    #[test]
    fn parses_orphos_gff() {
        // Orphos 0.4.0's own output on pDONR201 (meta mode), trimmed attributes, plus
        // an empty second sequence and a Mycoplasma-model third.
        let gff = "##gff-version  3\n\
# Sequence Data: seqnum=1;seqlen=4470;seqhdr=\"0 some description\"\n\
# Model Data: version=Orphos.v1.0.0;run_type=Metagenomic;model=\"47|X|B|44.1|11|1\";gc_cont=44.20;transl_table=11;uses_sd=1\n\
0\tOrphos_v1.0.0\tCDS\t2\t106\t3.3\t+\t0\tID=1_1;partial=10;start_type=Edge;\n\
0\tOrphos_v1.0.0\tCDS\t959\t1264\t29.9\t-\t0\tID=1_2;partial=00;start_type=ATG;\n\
##gff-version  3\n\
# Sequence Data: seqnum=1;seqlen=0;seqhdr=\"1\"\n\
# Model Data: version=Orphos.v1.0.0;run_type=Metagenomic;transl_table=11;uses_sd=1\n\
##gff-version  3\n\
# Sequence Data: seqnum=1;seqlen=900;seqhdr=\"w2\"\n\
# Model Data: version=Orphos.v1.0.0;run_type=Metagenomic;transl_table=4;uses_sd=1\n\
w2\tOrphos_v1.0.0\tCDS\t10\t99\t1.0\t-\t0\tID=1_1;partial=01;\n";
        let calls = parse_gff(gff).unwrap();
        assert_eq!(calls["0"].table, Some(11));
        assert_eq!(
            calls["0"].genes,
            vec![
                pred(2, 106, 1, true, false),
                pred(959, 1264, -1, false, false)
            ]
        );
        assert!(calls["1"].genes.is_empty());
        assert_eq!(calls["w2"].table, Some(4));
        assert_eq!(calls["w2"].genes, vec![pred(10, 99, -1, false, true)]);
        assert!(parse_gff("0\tx\tCDS\t1\t9\t0\t+\t0\tID=1;\n").is_err());
    }

    #[test]
    fn linear_location_extends_partial_ends_to_the_edge() {
        // Forward, 5' partial 1 bp from the edge: extended, codon_start 2.
        let (loc, cs) = linear_location(&pred(2, 106, 1, true, false), 4470);
        assert_eq!(loc.parts, vec![(0, 106, 1, true, false)]);
        assert_eq!(cs, 2);
        // Forward, 3' partial: extended, codon_start unaffected.
        let (loc, cs) = linear_location(&pred(4375, 4469, 1, false, true), 4470);
        assert_eq!(loc.parts, vec![(4374, 4470, 1, false, true)]);
        assert_eq!(cs, 1);
        // Reverse, 5' end (the right end) partial 2 bp short: codon_start 3.
        let (loc, cs) = linear_location(&pred(802, 2686, -1, false, true), 2688);
        assert_eq!(loc.parts, vec![(801, 2688, -1, false, true)]);
        assert_eq!(cs, 3);
        // Reverse, 3' end (the left end) partial: no 5' extension.
        let (loc, cs) = linear_location(&pred(3, 300, -1, true, false), 1000);
        assert_eq!(loc.parts, vec![(0, 300, -1, true, false)]);
        assert_eq!(cs, 1);
        // A partial end 3+ bp from the edge keeps its coordinate but still gets a marker.
        let (loc, cs) = linear_location(&pred(5, 300, 1, true, false), 1000);
        assert_eq!(loc.parts, vec![(4, 300, 1, true, false)]);
        assert_eq!(cs, 1);
    }

    #[test]
    fn translation_follows_pyrodigal() {
        let code = genetic_code(11).unwrap();
        // TTG start reads as M; the stop stays.
        let seq = b"CCTTGAAATGAGG";
        assert_eq!(pred(3, 11, 1, false, false).translate(seq, code), "MK*");
        // A 5'-partial gene keeps its first codon.
        assert_eq!(pred(3, 11, 1, true, false).translate(seq, code), "LK*");
        // Reverse strand: revcomp of TCATTTCAA is TTGAAATGA.
        let rev = b"TCATTTCAA";
        assert_eq!(pred(1, 9, -1, false, false).translate(rev, code), "MK*");
        // The 5' end of a reverse gene is its right end.
        assert_eq!(pred(1, 9, -1, false, true).translate(rev, code), "LK*");
        // Ambiguous bases and table 4.
        assert_eq!(
            pred(1, 9, 1, true, false).translate(b"TGANNNTGG", code),
            "*XW"
        );
        let myco = genetic_code(4).unwrap();
        assert_eq!(
            pred(1, 9, 1, true, false).translate(b"TGANNNTGG", myco),
            "WXW"
        );
    }

    #[test]
    fn origin_window_rotates_short_contigs_whole() {
        let w = OriginWindow::new(9);
        assert_eq!((w.left, w.right), (5, 4));
        assert_eq!(w.sequence(b"ABCDEFGHI"), b"EFGHIABCD");
        let w = OriginWindow::new(100_000);
        assert_eq!((w.left, w.right), (20000, 20000));
    }

    #[test]
    fn origin_genes_become_joins_and_respect_main_pass_genes() {
        let w = OriginWindow::new(1000); // left = right = 500
        let preds = [
            pred(491, 520, 1, false, false),  // crosses the junction (500)
            pred(481, 510, -1, false, false), // crosses, reverse
            pred(10, 90, 1, false, false),    // entirely before it: ignored
            pred(495, 530, 1, true, false),   // partial: ignored
        ];
        let genes = w.origin_genes(&preds, &[]);
        assert_eq!(genes.len(), 2);
        assert_eq!(genes[0].0.operator, Some(CompoundOp::Join));
        assert_eq!(
            genes[0].0.parts,
            vec![(990, 1000, 1, false, false), (0, 20, 1, false, false)]
        );
        // In order along the gene: the after-origin part first on the minus strand.
        assert_eq!(
            genes[1].0.parts,
            vec![(0, 10, -1, false, false), (980, 1000, -1, false, false)]
        );

        // A same-strand main-pass gene overlapping by 61 bp conflicts; by 60 it doesn't.
        let long = [pred(301, 700, 1, false, false)]; // contig [800,1000)+[0,200)
        let overlapping = |n: i64| [Location::range(200 - n, 400, 1)];
        assert!(w.origin_genes(&long, &overlapping(61)).is_empty());
        assert_eq!(w.origin_genes(&long, &overlapping(60)).len(), 1);
        // Opposite strands may overlap by up to 200.
        assert_eq!(
            w.origin_genes(&long, &[Location::range(0, 200, -1)]).len(),
            1
        );
        let two_parts = Location {
            operator: Some(CompoundOp::Join),
            between: false,
            parts: vec![(0, 200, -1, false, false), (999, 1000, -1, false, false)],
        };
        assert!(w.origin_genes(&long, &[two_parts]).is_empty());
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(version_at_least("0.4.0", "0.4.0"));
        assert!(version_at_least("0.10.0", "0.4.0"));
        assert!(version_at_least("1.0", "0.4.0"));
        assert!(!version_at_least("0.3.9", "0.4.0"));
        assert!(!version_at_least("", "0.4.0"));
    }

    #[test]
    fn scope_all_clears_cds_and_domain_annotations() {
        let mut rec = SeqRecord {
            id: "c".into(),
            name: "c".into(),
            description: String::new(),
            seq: b"ACGT".to_vec(),
            molecule_type: Some("DNA".into()),
            topology: Topology::Linear,
            annotations: Annotations::default(),
            features: vec![
                SeqFeature::new("CDS", Location::range(0, 3, 1)),
                SeqFeature::new(DOMAIN_FEATURE_NAME, Location::range(0, 3, 1)),
                SeqFeature::new("gene", Location::range(0, 3, 1)),
            ],
        };
        clear_cds_annotations(std::slice::from_mut(&mut rec));
        let kinds: Vec<&str> = rec.features.iter().map(|f| f.kind.as_str()).collect();
        assert_eq!(kinds, ["gene"]);
    }

    #[test]
    fn nothing_to_call_never_needs_orphos() {
        // A protein record and an empty contig: no orphos run, so this passes on a
        // machine without it.
        let mut recs = vec![
            SeqRecord {
                id: "p".into(),
                name: "p".into(),
                description: String::new(),
                seq: b"MKV".to_vec(),
                molecule_type: Some("protein".into()),
                topology: Topology::Linear,
                annotations: Annotations::default(),
                features: Vec::new(),
            },
            SeqRecord {
                id: "e".into(),
                name: "e".into(),
                description: String::new(),
                seq: Vec::new(),
                molecule_type: Some("DNA".into()),
                topology: Topology::Circular,
                annotations: Annotations::default(),
                features: Vec::new(),
            },
        ];
        let opts = GeneCallOptions {
            bin_path: Some("/nonexistent/orphos".into()),
            ..Default::default()
        };
        assert_eq!(call_genes(&mut recs, &[0, 1], &opts).unwrap(), 0);
    }
}
