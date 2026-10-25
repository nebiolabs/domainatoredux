//! `domainatoredux <tool> [opts]` — the CLI for the Domainator Rust rewrite, as a
//! library so that more than one front end can run it.
//!
//! [`run`] takes a whole command line and does exactly what the `domainator`
//! binary does with it (`src/main.rs` is nothing else). The GUI calls it too: its
//! Tools-menu jobs run in a copy of the GUI executable started in a hidden worker
//! mode (`apps/domainator-gui/src-tauri/src/worker.rs`), which hands the worker's
//! arguments straight to [`run`], so a GUI run and a CLI run take the same code.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};

use config::{load_config, print_config};
use domainator_config as config;

mod commands;
pub mod output;

use domainator_core::io::genbank::writer::write_record;
use domainator_core::io::genbank::{read_genbank, write_genbank};
use domainator_core::io::{detect_filetype, fasta, FileType};
use domainator_core::matrix::read_matrix;
use domainator_core::search::hmm_write::write_hmm_output;
use domainator_core::seq::taxonomy::{default_ncbi_taxonomy_path, NCBITaxonomy};
use domainator_core::seq::SeqRecord;
use domainator_core::tools;
use domainator_core::tools::build_ssn_viewer::{
    build_ssn_viewer_bundle, gzip, render_html, serialize_bundle,
};
use domainator_core::tools::gene_call;
use domainator_core::tools::hmmer_compare::{hmmer_compare, HmmerCompareOptions};
use domainator_core::tools::matrix_report::{matrix_report, render_text, to_json};
use domainator_core::tools::select_common::parse_name_list;
use domainator_core::tools::ssn_bundle;
use domainator_core::tools::ssn_navigator::ssn_navigator;

#[derive(Parser)]
#[command(name = "domainatoredux", version, long_version = domainator_core::version::LONG_VERSION, about = "Domainatoredux: protein-domain / SSN analysis (a Rust port of domainator)")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
#[command(rename_all = "snake_case")]
enum Command {
    /// Parse a sequence file and report per-record summary (dev/diagnostic).
    Inspect {
        /// GenBank or FASTA file (uncompressed).
        input: PathBuf,
        /// Print a one-line summary per record.
        #[arg(long)]
        verbose: bool,
    },
    /// Read a GenBank/FASTA file and write it back out as GenBank (round-trip).
    Convert {
        input: PathBuf,
        output: PathBuf,
    },
    /// Annotate sequences with hits to reference HMMs (protein hmmsearch path).
    Domainate(config::args::DomainateArgs),
    /// Search references against a (possibly sharded/BGZF/indexed) database.
    DomainSearch(config::args::DomainSearchArgs),
    /// Remove redundant sequences (hash exact-dedup, or cd-hit/diamond/usearch clustering).
    DeduplicateGenbank(config::args::DeduplicateGenbankArgs),
    /// Convert GenBank record(s) to FASTA.
    GenbankToFasta(config::args::GenbankToFastaArgs),
    /// Clean/filter sequences: newick-safe names, dedup names, strip/filter residues.
    CleanSequences(config::args::CleanSequencesArgs),
    /// Call genes (Prodigal-style CDS features) on nucleotide contigs with the
    /// external Orphos binary.
    GeneCall {
        /// Input GenBank/FASTA file(s) (GenBank on stdin if omitted).
        #[arg(short, long, num_args = 1..)]
        input: Vec<PathBuf>,
        /// Output GenBank file (stdout if omitted).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Molecule type for FASTA input (default: nucleotide).
        #[arg(long = "fasta_type", value_parser = ["protein", "nucleotide"])]
        fasta_type: Option<String>,
        /// meta: pre-trained models chosen per contig (metagenomes, plasmids, phages,
        /// short contigs). single: train on all input records together (one complete
        /// genome). Default: meta.
        #[arg(long, value_parser = ["meta", "single"])]
        mode: Option<String>,
        /// unannotated: call only contigs with no CDS. all: delete every CDS (and its
        /// Domainator annotations) and re-call every contig. Default: unannotated.
        #[arg(long, value_parser = ["unannotated", "all"])]
        scope: Option<String>,
        /// Genetic code (1-6, 9-16, 21-25). Default: 11, or in meta mode the table of
        /// each contig's best model.
        #[arg(long = "translation_table")]
        translation_table: Option<u8>,
        /// Closed ends: no genes running off the ends of a contig.
        #[arg(long)]
        closed: bool,
        /// No genes across runs of N.
        #[arg(long)]
        mask: bool,
        /// Orphos threads (0 = all cores).
        #[arg(long)]
        cpu: Option<usize>,
        /// Path to the orphos binary (else $DOMAINATOREDUX_ORPHOS, then PATH).
        #[arg(long = "bin_path")]
        bin_path: Option<String>,
        #[arg(long)]
        config: Option<String>,
        #[arg(long = "print_config")]
        print_config: bool,
    },
    SortContigs(config::args::SortContigsArgs),
    /// Extract contig regions using explicit 1-based coordinate ranges from a table.
    SelectByCoord(config::args::SelectByCoordArgs),
    /// Filter Domainator/Domain_Search annotations by evalue, overlap, database, name.
    #[command(allow_negative_numbers = true)]
    FilterDomains(config::args::FilterDomainsArgs),
    /// Assign hex colors to domain and/or CDS features (Color qualifier).
    ColorGenbank(config::args::ColorGenbankArgs),
    /// Generate an SVG legend from a color table.
    ColorTableToLegend {
        /// Color table file(s) (TSV: key, hex; stdin if omitted).
        #[arg(short, long, num_args = 1..)]
        input: Vec<PathBuf>,
        /// Output SVG file (required).
        #[arg(long)]
        svg: Option<String>,
        /// Legend title (default: "Legend").
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        config: Option<String>,
        #[arg(long = "print_config")]
        print_config: bool,
    },
    /// Trim contig ends by kb, CDS count, or by stripping end CDSs by domain.
    TrimContigs(config::args::TrimContigsArgs),
    /// Extract domain-annotated regions from annotated contigs.
    #[command(allow_negative_numbers = true)]
    ExtractDomains(config::args::ExtractDomainsArgs),
    /// Extract peptide sequences from selected CDSs.
    ExtractPeptides(config::args::ExtractPeptidesArgs),
    /// Extract regions of proteins not containing domain annotations.
    #[command(allow_negative_numbers = true)]
    ExtractUnannotated(config::args::ExtractUnannotatedArgs),
    /// Compute a similarity matrix between input and reference sequences/profiles.
    SeqDist(config::args::SeqDistArgs),
    /// Pairwise contig similarity (Jaccard + Adjacency) from annotated GenBank.
    CompareContigs(config::args::CompareContigsArgs),
    /// Normalize / convert / sparsify a score matrix (dense<->sparse, --mode, --mst_knn).
    TransformMatrix(config::args::TransformMatrixArgs),
    /// Build a Cytoscape XGMML sequence-similarity network from a score matrix.
    BuildSsn(config::args::BuildSsnArgs),
    /// Build a compact `.dsnv` bundle (+ standalone HTML) for the interactive SSN viewer.
    BuildSsnViewer {
        /// Symmetric similarity matrix (HDF5 or dense TSV).
        #[arg(short, long)]
        input: Option<PathBuf>,
        /// Write the gzip-JSON `.dsnv` bundle here.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Write a standalone HTML viewer shell here.
        #[arg(long)]
        html: Option<String>,
        /// Embed the bundle into the HTML so the viewer autoloads it.
        #[arg(long = "embed_data")]
        embed_data: bool,
        /// Display name recorded in the bundle (defaults to the output stem).
        #[arg(long)]
        name: Option<String>,
        /// TSV metadata files keyed by node id.
        #[arg(long, num_args = 1..)]
        metadata: Vec<String>,
        /// Only consider labels in this list (additive with --subset_file).
        #[arg(long, num_args = 1..)]
        subset: Vec<String>,
        /// Text file of labels to retain.
        #[arg(long = "subset_file")]
        subset_file: Option<String>,
        /// Default metadata field for node coloring in the viewer.
        #[arg(long = "color_by")]
        color_by: Option<String>,
        /// Default metadata field for node labels in the viewer.
        #[arg(long = "label_by")]
        label_by: Option<String>,
        /// Numeric metadata columns to color as discrete categories rather than with
        /// a gradient (e.g. integer cluster numbers). Sets the viewer's initial state.
        #[arg(long = "categorical", num_args = 1..)]
        categorical: Vec<String>,
        /// Merge-impact metric for split events (min_child or product).
        #[arg(long = "merge_impact_metric", value_parser = ["min_child", "product"])]
        merge_impact_metric: Option<String>,
        /// Fail if the projected .dsnv bundle output exceeds this many GB (default 25; 0 disables).
        #[arg(long = "max_output_gb")]
        max_output_gb: Option<f64>,
        /// Read parameters from a JSON/YAML config file (CLI flags override it).
        #[arg(long)]
        config: Option<String>,
        /// Print the resolved parameters as YAML and exit.
        #[arg(long = "print_config")]
        print_config: bool,
    },
    /// Summarize a similarity matrix (stats, cluster-vs-threshold, merge events) as text or JSON.
    MatrixReport {
        /// Input matrix file (HDF5 or dense TSV).
        #[arg(short, long)]
        input: Option<PathBuf>,
        /// Write the human-readable text report here.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Write the agent-friendly JSON report here.
        #[arg(long)]
        json: Option<PathBuf>,
        /// Merge-impact metric for split events (min_child or product).
        #[arg(long = "merge_impact_metric", value_parser = ["min_child", "product"])]
        merge_impact_metric: Option<String>,
        /// Max strongest merge events to report (0 = all; default 500).
        #[arg(long = "max_merge_events")]
        max_merge_events: Option<usize>,
        /// Read parameters from a JSON/YAML config file (CLI flags override it).
        #[arg(long)]
        config: Option<String>,
        /// Print the resolved parameters as YAML and exit.
        #[arg(long = "print_config")]
        print_config: bool,
    },
    /// Dataset-level statistics: contig/CDS counts, domain frequency, taxonomy.
    SummaryReport {
        /// Input GenBank file(s).
        #[arg(short, long, num_args = 1..)]
        input: Vec<PathBuf>,
        /// Write the human-readable text report here (stdout if no output is given).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Write the full summary as one compact JSON object here (`-` = stdout).
        #[arg(long)]
        json: Option<String>,
        /// Report co-occurrence for these domains.
        #[arg(long, num_args = 1..)]
        domains: Vec<String>,
        /// Only count domains from these databases.
        #[arg(long, num_args = 1..)]
        databases: Vec<String>,
        /// Summarize the taxonomic origins of the contigs.
        #[arg(long)]
        taxonomy: bool,
        /// Path to the NCBI taxonomy database directory (default: /tmp/ncbi_taxonomy_$USER).
        #[arg(long = "ncbi_taxonomy_path")]
        ncbi_taxonomy_path: Option<String>,
        /// Read parameters from a JSON/YAML config file (CLI flags override it).
        #[arg(long)]
        config: Option<String>,
        /// Print the resolved parameters as YAML and exit.
        #[arg(long = "print_config")]
        print_config: bool,
        /// Which CDSs/proteins to count, by fragment status: include (all), exclude (skip fragments) or
        /// only (only fragments). Default: include.
        #[arg(long = "partial", value_parser = ["include", "exclude", "only"])]
        partial: Option<String>,
    },
    /// Navigate a `.dsnv` bundle: clusters, cluster contents, and metadata, as JSON.
    SsnNavigator {
        /// Input `.dsnv` bundle (gzip JSON, or plain JSON).
        #[arg(short, long)]
        input: Option<PathBuf>,
        /// Write the JSON here (default: stdout).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Query to answer.
        #[arg(long, value_parser = ["overview", "thresholds", "clusters", "cluster", "node"])]
        mode: Option<String>,
        /// Similarity cut: a number, or `inf`. Omit for the floor (one cluster per
        /// connected component); higher thresholds give finer clusters.
        #[arg(long)]
        threshold: Option<String>,
        /// Cluster id, for `--mode cluster`.
        #[arg(long)]
        id: Option<usize>,
        /// Node id, for `--mode cluster` or `--mode node`.
        #[arg(long)]
        node: Option<String>,
        /// Ignore clusters smaller than this (`--mode clusters`).
        #[arg(long = "min_size")]
        min_size: Option<usize>,
        /// Bound every list in the output (default 50).
        #[arg(long = "top_n")]
        top_n: Option<usize>,
        /// Include the full member id list in `--mode cluster`.
        #[arg(long)]
        members: bool,
        /// How many split events the `--mode thresholds`/`node` cut-points are drawn
        /// from: the strongest this many by impact, plus a few so that no 5% band of
        /// the threshold range with an event to offer is left without one. 0 offers a
        /// cut-point at every split event. The series is derived from the bundle's
        /// merge order, so any value works on any bundle. (default 500)
        #[arg(long = "max_merge_events")]
        max_merge_events: Option<usize>,
        /// Read parameters from a JSON/YAML config file (CLI flags override it).
        #[arg(long)]
        config: Option<String>,
        /// Print the resolved parameters as YAML and exit.
        #[arg(long = "print_config")]
        print_config: bool,
    },
    /// Print a JSON description of every command and its options (the tool manifest).
    Describe {
        /// Describe only these commands.
        #[arg(long, num_args = 1..)]
        tool: Vec<String>,
        /// Indent the JSON by this many spaces.
        #[arg(long)]
        indent: Option<usize>,
    },
    /// Print a shell completion script.
    Completion {
        /// The shell to complete for.
        shell: clap_complete::Shell,
    },
    EnumReport(config::args::EnumReportArgs),
    /// Extract contig regions around selected CDSs (by name/domain/expr/hits).
    SelectByCds(config::args::SelectByCdsArgs),
    /// Select a subset of contigs from a GenBank/FASTA file by various criteria.
    SelectByContig(config::args::SelectByContigArgs),
    /// Align HMM profiles against reference HMM profiles (hhsearch-like).
    HmmerCompare {
        /// Query `.hmm` file(s).
        #[arg(short, long, num_args = 1..)]
        input: Vec<PathBuf>,
        /// Reference `.hmm` file(s).
        #[arg(short, long, num_args = 1..)]
        reference: Vec<PathBuf>,
        /// Output TSV (default: stdout).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Report hits with score >= this (default: 0).
        #[arg(long = "score_cutoff")]
        score_cutoff: Option<f64>,
        /// Keep at most this many top hits per query.
        #[arg(short = 'k', long)]
        k: Option<usize>,
        /// Emit rendered alignments under each hit.
        #[arg(long)]
        alignments: bool,
        /// Worker threads (0 = all cores).
        #[arg(long)]
        cpu: Option<usize>,
        /// Fail if the projected TSV output exceeds this many GB (default 25; 0 disables).
        #[arg(long = "max_output_gb")]
        max_output_gb: Option<f64>,
        /// Read parameters from a JSON/YAML config file (CLI flags override it).
        #[arg(long)]
        config: Option<String>,
        /// Print the resolved parameters as YAML and exit.
        #[arg(long = "print_config")]
        print_config: bool,
    },
    HmmerReport(config::args::HmmerReportArgs),
    /// Filter an HMM file by text matches on profile NAME/DESC/ACC fields.
    HmmerSelect {
        /// Input `.hmm` file(s). Reads stdin if omitted (unsupported: give files).
        #[arg(short, long, num_args = 1..)]
        input: Vec<PathBuf>,
        /// Output `.hmm` file. Writes to stdout if omitted.
        /// Writes plain gzip if the path ends in `.gz`; `.bgz` is refused because
        /// HMMER cannot read BGZF. Output to stdout is never compressed.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Regex pattern(s) to match.
        #[arg(long, num_args = 1..)]
        regex: Vec<String>,
        /// Exact string(s) to match (whole field).
        #[arg(long, num_args = 1..)]
        exact: Vec<String>,
        /// Substring(s) to match.
        #[arg(long, num_args = 1..)]
        contains: Vec<String>,
        /// Which field(s) to search: name, desc, acc, or all.
        #[arg(long)]
        field: Option<String>,
        /// Make matching case-sensitive.
        #[arg(long = "case_sensitive")]
        case_sensitive: bool,
        /// Only read input files whose profiles use this alphabet (amino, dna, rna),
        /// skipping the rest. An .hmm file holds one alphabet, so this selects whole
        /// files -- which is how a mixed input set is split one alphabet at a time.
        #[arg(long)]
        alphabet: Option<String>,
        /// Read parameters from a JSON/YAML config file (CLI flags override it).
        #[arg(long)]
        config: Option<String>,
        /// Print the resolved parameters as YAML and exit.
        #[arg(long = "print_config")]
        print_config: bool,
    },
    /// Build an HMM profile from a (Stockholm) multiple sequence alignment.
    HmmerBuild {
        /// Input Stockholm MSA file (stdin if omitted).
        #[arg(short, long)]
        input: Option<PathBuf>,
        /// Output `.hmm` file (stdout if omitted).
        /// Writes plain gzip if the path ends in `.gz`; `.bgz` is refused because
        /// HMMER cannot read BGZF. Output to stdout is never compressed.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Profile NAME (required).
        #[arg(long)]
        name: Option<String>,
        /// Profile ACC.
        #[arg(long)]
        acc: Option<String>,
        /// Profile DESC.
        #[arg(long)]
        desc: Option<String>,
        /// Alphabet: amino | dna | rna (inferred if omitted).
        #[arg(long)]
        alphabet: Option<String>,
        /// Max expected hit length (the profile's MAXL field), used by nhmmer for
        /// windowed E-values. Nucleotide alphabets only.
        #[arg(long = "window_length")]
        window_length: Option<i32>,
        /// Tail mass from which to derive the window length. Nucleotide alphabets only;
        /// only one of --window_length and --window_beta may be given.
        #[arg(long = "window_beta")]
        window_beta: Option<f64>,
        /// Read parameters from a JSON/YAML config file (CLI flags override it).
        #[arg(long)]
        config: Option<String>,
        /// Print the resolved parameters as YAML and exit.
        #[arg(long = "print_config")]
        print_config: bool,
    },
    /// Filter query HMM profiles by best score against reference HMM profiles.
    HmmerSearch {
        /// Query `.hmm` file(s) — the set being filtered.
        #[arg(short, long, num_args = 1..)]
        input: Vec<PathBuf>,
        /// Reference `.hmm` file(s) — the filter criterion.
        #[arg(short, long, num_args = 1..)]
        reference: Vec<PathBuf>,
        /// Output `.hmm` file (stdout if omitted).
        /// Writes plain gzip if the path ends in `.gz`; `.bgz` is refused because
        /// HMMER cannot read BGZF. Output to stdout is never compressed.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Keep queries whose best reference score is >= this. The default is 10.0
        /// for amino acid profiles, scaled to the reference alphabet for nucleotides.
        #[arg(long = "score_cutoff")]
        score_cutoff: Option<f64>,
        /// Keep at most this many queries, ranked by best score.
        #[arg(long = "max_hits")]
        max_hits: Option<usize>,
        /// Worker threads (0 = all cores).
        #[arg(long)]
        cpu: Option<usize>,
        /// Fail if projected HMM output exceeds this many GB (0 disables).
        #[arg(long = "max_output_gb")]
        max_output_gb: Option<f64>,
        /// Read parameters from a JSON/YAML config file (CLI flags override it).
        #[arg(long)]
        config: Option<String>,
        /// Print the resolved parameters as YAML and exit.
        #[arg(long = "print_config")]
        print_config: bool,
    },
    /// Shard / BGZF-compress / .didx-index a domain_search database.
    FormatDb {
        /// GenBank or FASTA database file(s) to format (plain or BGZF).
        #[arg(short, long, num_args = 1..)]
        input: Vec<PathBuf>,
        /// Split each input into this many shards (balanced by record count).
        #[arg(long)]
        shards: Option<usize>,
        /// Write BGZF (block-gzip) output.
        #[arg(long, alias = "bgzf")]
        compress: bool,
        /// Write a .didx offset index per output file.
        #[arg(long)]
        index: bool,
        /// Directory to write outputs into (default: alongside each input).
        #[arg(long = "output_dir")]
        output_dir: Option<String>,
        /// Base name for the output database (requires a single input).
        #[arg(long)]
        name: Option<String>,
        /// BGZF/deflate compression level (1-12).
        #[arg(long)]
        level: Option<u8>,
        /// Overwrite existing outputs / rebuild indexes instead of skipping.
        #[arg(long)]
        force: bool,
        /// Log file (default: stderr).
        #[arg(long)]
        log: Option<PathBuf>,
        /// Read parameters from a JSON/YAML config file (CLI flags override it).
        #[arg(long)]
        config: Option<String>,
        /// Print the resolved parameters as YAML and exit.
        #[arg(long = "print_config")]
        print_config: bool,
    },
}

fn load(input: &Path) -> Result<Vec<SeqRecord>> {
    let path = input.to_str().context("non-UTF8 path")?;
    match detect_filetype(input) {
        Some(FileType::Genbank) => read_genbank(path).with_context(|| format!("reading {path}")),
        Some(FileType::Fasta) => {
            fasta::read_fasta(path, Some("protein")).with_context(|| format!("reading {path}"))
        }
        None => anyhow::bail!(
            "could not detect file type from extension: {}",
            input.display()
        ),
    }
}

/// Load and concatenate one or more sequence files; if `inputs` is empty, read
/// GenBank from stdin (matching the Python tools' stdin default).
fn load_inputs(inputs: &[PathBuf]) -> Result<Vec<SeqRecord>> {
    if inputs.is_empty() {
        use std::io::Read;
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("reading stdin")?;
        let dir = std::env::temp_dir();
        let tmp = dir.join(format!("domainator_stdin_{}.gb", std::process::id()));
        std::fs::write(&tmp, &buf)?;
        let recs = read_genbank(tmp.to_str().unwrap()).context("parsing stdin GenBank");
        let _ = std::fs::remove_file(&tmp);
        return recs;
    }
    let mut out = Vec::new();
    for p in inputs {
        out.extend(load(p)?);
    }
    Ok(out)
}

/// Filter `records` in place to those whose LOCUS name (Python's `contig.id`) is
/// in the `--contigs`/`--contigs_file` set. A no-op when both are empty.
pub(crate) fn filter_contigs(
    records: &mut Vec<SeqRecord>,
    contigs: &[String],
    contigs_file: Option<&str>,
) -> Result<()> {
    let set = parse_name_list(contigs, contigs_file)?;
    if let Some(set) = set {
        records.retain(|r| set.contains(&r.id));
    }
    Ok(())
}

/// Convert paths to UTF-8 strings.
fn paths_to_strings(paths: &[PathBuf]) -> Result<Vec<String>> {
    paths
        .iter()
        .map(|p| p.to_str().map(str::to_string).context("non-UTF8 path"))
        .collect()
}

/// Read a color table: a TSV with no header mapping a value to a hex color.
pub(crate) fn read_color_table(path: &str) -> Result<std::collections::HashMap<String, String>> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading color table {path}"))?;
    let mut map = std::collections::HashMap::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut it = line.splitn(2, '\t');
        if let (Some(k), Some(v)) = (it.next(), it.next()) {
            map.insert(k.to_string(), v.trim().to_string());
        }
    }
    Ok(map)
}

/// Convert an optional path to an optional UTF-8 string.
fn opt_path(path: &Option<PathBuf>) -> Result<Option<String>> {
    match path {
        Some(p) => Ok(Some(p.to_str().context("non-UTF8 path")?.to_string())),
        None => Ok(None),
    }
}

/// `output_guardrails.max_output_gb_to_bytes` (the core port): negative or
/// non-finite is an error, `0` disables (returns `None`).
pub(crate) fn max_output_gb_to_bytes(max_gb: f64) -> Result<Option<u64>> {
    tools::output_guardrails::max_output_gb_to_bytes(max_gb).map_err(anyhow::Error::msg)
}

/// Fail when `actual_bytes` of an already-serialized output exceeds the byte
/// budget for `max_gb`. `description` names the output; `mitigation` lists the
/// relevant flags for the failure message.
fn enforce_bytes_gb(
    actual_bytes: u64,
    max_gb: f64,
    description: &str,
    mitigation: &str,
) -> Result<()> {
    let Some(max_bytes) = max_output_gb_to_bytes(max_gb)? else {
        return Ok(());
    };
    if actual_bytes > max_bytes {
        anyhow::bail!(
            "Projected {description} size is {:.2} GB, which exceeds the configured limit of \
             {:.2} GB. Reduce the projected output size or raise the limit with --max_output_gb. \
             Relevant options: {mitigation}.",
            actual_bytes as f64 / 1_000_000_000.0,
            max_bytes as f64 / 1_000_000_000.0
        );
    }
    Ok(())
}

/// Write records as GenBank to a file, or to stdout when `output` is None.
fn write_genbank_out(records: &[SeqRecord], output: Option<&PathBuf>) -> Result<()> {
    match output {
        Some(p) => {
            write_genbank(p.to_str().context("non-UTF8 path")?, records).context("writing GenBank")
        }
        None => {
            let stdout = std::io::stdout();
            let mut h = std::io::BufWriter::new(stdout.lock());
            for rec in records {
                write_record(&mut h, rec)?;
            }
            h.flush()?;
            Ok(())
        }
    }
}

/// Load an `NCBITaxonomy` from `--ncbi_taxonomy_path` (only when a taxonomy
/// filter/column is in use).
fn load_taxonomy(path: &str) -> Result<NCBITaxonomy> {
    NCBITaxonomy::load(Path::new(path)).context("loading NCBI taxonomy")
}

/// Load a sequence file, treating FASTA as `fasta_type` (protein/nucleotide).
pub(crate) fn load_typed(input: &Path, fasta_type: &str) -> Result<Vec<SeqRecord>> {
    let path = input.to_str().context("non-UTF8 path")?;
    match detect_filetype(input) {
        Some(FileType::Genbank) => read_genbank(path).context("reading GenBank"),
        Some(FileType::Fasta) => fasta::read_fasta(path, Some(fasta_type)).context("reading FASTA"),
        None => anyhow::bail!(
            "could not detect file type from extension: {}",
            input.display()
        ),
    }
}

/// Parse a `FromStr` choice (`--mode`, `--scope`), keeping its own error message.
fn parse_choice<T: std::str::FromStr<Err = String>>(s: &str) -> Result<T> {
    s.parse().map_err(|e: String| anyhow::anyhow!(e))
}

/// The `--gene_call` request for `domainate`/`domain_search`: Orphos in meta mode
/// (upstream's only mode) with its default genetic code, or `None` when unset.
pub(crate) fn gene_call_request(
    scope: Option<&str>,
    cpu: usize,
) -> Result<Option<gene_call::GeneCall>> {
    scope
        .map(|scope| {
            Ok(gene_call::GeneCall {
                scope: parse_choice(scope)?,
                options: gene_call::GeneCallOptions {
                    cpu,
                    ..Default::default()
                },
            })
        })
        .transpose()
}

/// Compile `--include_taxids`/`--exclude_taxids`/`--taxonomy_expr` into an
/// allowed-taxid set, loading the taxonomy from `path` when a filter is active.
/// Returns `None` when no taxonomy filtering was requested.
pub(crate) fn compile_allowed_taxids(
    include: &[i64],
    exclude: &[i64],
    expr: Option<&str>,
    path: Option<&str>,
) -> Result<Option<std::collections::HashSet<i64>>> {
    let has_expr = expr.is_some_and(|e| !e.trim().is_empty());
    if include.is_empty() && exclude.is_empty() && !has_expr {
        return Ok(None);
    }
    if has_expr && (!include.is_empty() || !exclude.is_empty()) {
        anyhow::bail!(
            "--taxonomy_expr is mutually exclusive with --include_taxids/--exclude_taxids."
        );
    }
    let tax = load_taxonomy(
        &path
            .map(str::to_string)
            .unwrap_or_else(default_ncbi_taxonomy_path),
    )?;
    let allowed = if has_expr {
        tax.compile_taxonomy_expression(expr.unwrap())
    } else {
        use std::collections::HashSet;
        let inc: HashSet<i64> = include.iter().copied().collect();
        let exc: HashSet<i64> = exclude.iter().copied().collect();
        tax.compile_taxonomy_filter(
            if inc.is_empty() { None } else { Some(&inc) },
            if exc.is_empty() { None } else { Some(&exc) },
        )
    };
    Ok(allowed)
}

/// Run one `domainatoredux` command line. `args` includes the program name first, as
/// `std::env::args_os()` does.
///
/// A command line clap rejects, and `--help`/`--version`, come back as an `Err`
/// holding the [`clap::Error`], unprinted, so the caller decides how to show it:
/// the binary calls [`clap::Error::exit`] (usage on stderr, exit 2; help on
/// stdout, exit 0), exactly as `Cli::parse()` did. Use [`clap_error`] to find it.
pub fn run<I, T>(args: I) -> Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let mut argv: Vec<std::ffi::OsString> = args.into_iter().map(Into::into).collect();
    let mut cmd = command();
    let tool = argv
        .get(1)
        .and_then(|a| a.to_str())
        .and_then(config::registry::tool_info)
        .filter(|t| t.migrated);
    if let Some(info) = tool {
        let sub = cmd
            .find_subcommand(info.name)
            .context("registry tool without a subcommand")?;
        let rest = config::config_file::expand(sub, &argv[2..])?;
        argv.truncate(2);
        argv.extend(rest);
        if argv.iter().any(|a| a == "--print_config") {
            // A config being printed need not be complete, as upstream's need not.
            cmd = cmd.mut_subcommand(info.name, |c| c.mut_args(|a| a.required(false)));
        }
    }
    let matches = cmd.clone().try_get_matches_from(&argv)?;
    if let (Some(info), Some((name, sub_m))) = (tool, matches.subcommand()) {
        let sub = cmd
            .find_subcommand(name)
            .context("parsed an unknown subcommand")?;
        if sub_m.get_flag("print_config") {
            print!("{}", config::config_file::print_config(sub, sub_m, info)?);
            return Ok(());
        }
        check_io(sub, sub_m, info)?;
    }
    dispatch(Cli::from_arg_matches(&matches)?)
}

/// The command line, with each tool's one-line summary from the registry.
pub fn command() -> clap::Command {
    Cli::command().mut_subcommands(|c| match config::registry::tool_info(c.get_name()) {
        Some(info) => c.about(info.summary),
        None => c,
    })
}

/// Upstream's `check_outputs` and `reject_stdin`: at least one of the required
/// outputs, at most one output on standard output, `-` only where it is allowed.
fn check_io(
    sub: &clap::Command,
    m: &clap::ArgMatches,
    info: &config::registry::ToolInfo,
) -> Result<()> {
    let value = |id: &str| -> Vec<String> {
        m.try_get_raw(id)
            .ok()
            .flatten()
            .map(|v| v.map(|s| s.to_string_lossy().into_owned()).collect())
            .unwrap_or_default()
    };
    let flag = |id: &str| {
        sub.get_arguments()
            .find(|a| a.get_id() == id)
            .and_then(|a| a.get_long())
            .map_or_else(|| id.to_string(), |l| format!("--{l}"))
    };
    let usage = |msg: String| -> anyhow::Error {
        let mut sub = sub
            .clone()
            .bin_name(format!("domainatoredux {}", info.name));
        sub.error(clap::error::ErrorKind::MissingRequiredArgument, msg)
            .into()
    };
    let outputs: Vec<&clap::Arg> = sub
        .get_arguments()
        .filter(|a| a.get_id().as_str().ends_with("_out"))
        .collect();
    if let Some(required) = info.required_outputs {
        if required.iter().all(|id| value(id).is_empty()) {
            let names: Vec<String> = required.iter().map(|id| flag(id)).collect();
            return Err(usage(format!(
                "No output specified. Give at least one of: {}",
                names.join(", ")
            )));
        }
    }
    let on_stdout: Vec<String> = outputs
        .iter()
        .map(|a| a.get_id().as_str())
        .filter(|id| value(id).iter().any(|v| output::is_stdout(v)))
        .map(flag)
        .collect();
    for id in info.no_stdout {
        if value(id).iter().any(|v| output::is_stdout(v)) {
            return Err(usage(format!(
                "{} cannot be written to standard output. Give a file name.",
                flag(id)
            )));
        }
    }
    if on_stdout.len() > 1 {
        return Err(usage(format!(
            "Only one output can be written to standard output, but got: {}",
            on_stdout.join(", ")
        )));
    }
    if !info.stdin && value("input").iter().any(|v| output::is_stdin(v)) {
        return Err(usage(
            "This command cannot read standard input, because it reads its input more than \
             once. Give a file instead."
                .to_string(),
        ));
    }
    Ok(())
}

/// Print an error from [`run`] the way the binary does, and return the exit code: a
/// usage error as clap prints it (exit 2, or 0 for `--help`), anything else as one
/// line, `domainatoredux <tool>: error: <message: cause: ...>` (exit 1).
pub fn report_error(e: &anyhow::Error, argv: &[std::ffi::OsString]) -> i32 {
    if let Some(clap_err) = clap_error(e) {
        let _ = clap_err.print();
        return clap_err.exit_code();
    }
    let prog = match argv.get(1).and_then(|a| a.to_str()) {
        Some(tool) if command().find_subcommand(tool).is_some() => {
            format!("domainatoredux {tool}")
        }
        _ => "domainatoredux".to_string(),
    };
    eprintln!("{prog}: error: {e:#}");
    1
}

/// What the binary does: [`run`], then [`report_error`] on failure.
pub fn main_with<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let argv: Vec<std::ffi::OsString> = args.into_iter().map(Into::into).collect();
    match run(argv.clone()) {
        Ok(()) => 0,
        Err(e) => report_error(&e, &argv),
    }
}

/// The [`clap::Error`] inside an error from [`run`], when clap is where it came
/// from (a bad command line, or `--help`/`--version`).
pub fn clap_error(e: &anyhow::Error) -> Option<&clap::Error> {
    e.downcast_ref::<clap::Error>()
}

fn dispatch(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Inspect { input, verbose } => {
            let records = load(&input)?;
            println!("{} records", records.len());
            if verbose {
                for r in &records {
                    println!(
                        "  id={} name={} mol={:?} len={} features={}",
                        r.id,
                        r.name,
                        r.molecule_type,
                        r.len(),
                        r.features.len()
                    );
                }
            }
        }
        Command::Convert { input, output } => {
            let records = load(&input)?;
            let out = output.to_str().context("non-UTF8 path")?;
            write_genbank(out, &records).context("writing GenBank")?;
            println!("wrote {} records to {}", records.len(), output.display());
        }
        Command::Domainate(args) => commands::domainate::run(&args)?,
        Command::DomainSearch(args) => commands::domain_search::run(&args)?,
        Command::DeduplicateGenbank(args) => commands::deduplicate_genbank::run(&args)?,
        Command::GenbankToFasta(args) => commands::genbank_to_fasta::run(&args)?,
        Command::CleanSequences(args) => commands::clean_sequences::run(&args)?,
        Command::GeneCall {
            input,
            output,
            fasta_type,
            mode,
            scope,
            translation_table,
            closed,
            mask,
            cpu,
            bin_path,
            config: config_path,
            print_config: do_print,
        } => {
            let cli = config::GeneCallConfig {
                input: paths_to_strings(&input)?,
                output: opt_path(&output)?,
                fasta_type,
                mode,
                scope,
                translation_table,
                closed,
                mask,
                cpu,
                bin_path,
            };
            let mut cfg = match &config_path {
                Some(p) => load_config::<config::GeneCallConfig>(p)?,
                None => config::GeneCallConfig::default(),
            };
            cfg.merge_cli(cli);
            config::reject_non_finite(&cfg)?;
            if do_print {
                print!("{}", print_config(&cfg)?);
                return Ok(());
            }
            let fasta_type = cfg.fasta_type.as_deref().unwrap_or("nucleotide");
            let mut records: Vec<SeqRecord> = if cfg.input.is_empty() {
                load_inputs(&[])?
            } else {
                let mut records = Vec::new();
                for p in &cfg.input {
                    records.extend(load_typed(&PathBuf::from(p), fasta_type)?);
                }
                records
            };
            let request = gene_call::GeneCall {
                scope: parse_choice(cfg.scope.as_deref().unwrap_or("unannotated"))?,
                options: gene_call::GeneCallOptions {
                    mode: parse_choice(cfg.mode.as_deref().unwrap_or("meta"))?,
                    translation_table: cfg.translation_table,
                    closed: cfg.closed,
                    mask: cfg.mask,
                    cpu: cfg.cpu.unwrap_or(0),
                    bin_path: cfg.bin_path.clone(),
                },
            };
            let added = gene_call::gene_call(&mut records, &request).context("calling genes")?;
            write_genbank_out(&records, cfg.output.as_ref().map(PathBuf::from).as_ref())?;
            eprintln!("called {added} genes on {} records", records.len());
        }
        Command::SortContigs(args) => commands::sort_contigs::run(&args)?,
        Command::SelectByCoord(args) => commands::select_by_coord::run(&args)?,
        Command::FilterDomains(args) => commands::filter_domains::run(&args)?,
        Command::ColorGenbank(args) => commands::color_genbank::run(&args)?,
        Command::ColorTableToLegend {
            input,
            svg,
            title,
            config: config_path,
            print_config: do_print,
        } => {
            let cli = config::ColorTableToLegendConfig {
                input: paths_to_strings(&input)?,
                svg,
                title,
            };
            let mut cfg = match &config_path {
                Some(p) => load_config::<config::ColorTableToLegendConfig>(p)?,
                None => config::ColorTableToLegendConfig::default(),
            };
            cfg.merge_cli(cli);
            config::reject_non_finite(&cfg)?;
            if do_print {
                print!("{}", print_config(&cfg)?);
                return Ok(());
            }
            let svg_path = cfg.svg.as_deref().context("--svg is required")?;

            // Read color table(s), merging in order (later files override keys).
            let mut table: Vec<(String, String)> = Vec::new();
            let mut index: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();
            let texts: Vec<String> = if cfg.input.is_empty() {
                use std::io::Read;
                let mut buf = String::new();
                std::io::stdin()
                    .read_to_string(&mut buf)
                    .context("reading stdin")?;
                vec![buf]
            } else {
                let mut v = Vec::new();
                for p in &cfg.input {
                    v.push(std::fs::read_to_string(p).with_context(|| format!("reading {p}"))?);
                }
                v
            };
            for text in &texts {
                let entries =
                    tools::color_genbank::read_color_table(text).map_err(|e| anyhow::anyhow!(e))?;
                for (k, v) in entries {
                    if let Some(&i) = index.get(&k) {
                        table[i].1 = v;
                    } else {
                        index.insert(k.clone(), table.len());
                        table.push((k, v));
                    }
                }
            }

            let title = cfg.title.as_deref().unwrap_or("Legend");
            let svg = tools::color_table_to_legend::color_table_to_legend(&table, title);
            std::fs::write(svg_path, svg).with_context(|| format!("writing {svg_path}"))?;
        }
        Command::TrimContigs(args) => commands::trim_contigs::run(&args)?,
        Command::ExtractDomains(args) => commands::extract_domains::run(&args)?,
        Command::ExtractPeptides(args) => commands::extract_peptides::run(&args)?,
        Command::ExtractUnannotated(args) => commands::extract_unannotated::run(&args)?,
        Command::SeqDist(args) => commands::seq_dist::run(&args)?,
        Command::CompareContigs(args) => commands::compare_contigs::run(&args)?,
        Command::TransformMatrix(args) => commands::transform_matrix::run(&args)?,
        Command::BuildSsn(args) => commands::build_ssn::run(&args)?,
        Command::BuildSsnViewer {
            input,
            output,
            html,
            embed_data,
            name,
            metadata,
            subset,
            subset_file,
            color_by,
            label_by,
            categorical,
            merge_impact_metric,
            max_output_gb,
            config: config_path,
            print_config: do_print,
        } => {
            let cli = config::BuildSsnViewerConfig {
                input: opt_path(&input)?,
                output: opt_path(&output)?,
                html,
                embed_data,
                name,
                metadata,
                subset,
                subset_file,
                color_by,
                label_by,
                categorical,
                merge_impact_metric,
                max_output_gb,
            };
            let mut cfg = match &config_path {
                Some(p) => load_config::<config::BuildSsnViewerConfig>(p)?,
                None => config::BuildSsnViewerConfig::default(),
            };
            cfg.merge_cli(cli);
            config::reject_non_finite(&cfg)?;
            if do_print {
                print!("{}", print_config(&cfg)?);
                return Ok(());
            }

            let stem = |p: &str| {
                Path::new(p)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string()
            };

            // Arg-combination handling mirrors the Python tool.
            if cfg.input.is_none() {
                if cfg.output.is_some() {
                    anyhow::bail!("-o/--output requires -i/--input.");
                }
                if cfg.embed_data {
                    anyhow::bail!("--embed_data requires -i/--input.");
                }
                let html = cfg
                    .html
                    .as_ref()
                    .context("when -i/--input is omitted, --html is required")?;
                let name = cfg
                    .name
                    .clone()
                    .unwrap_or_else(|| "Domainator SSN Viewer".to_string());
                std::fs::write(html, render_html(&name, None))?;
                println!("wrote standalone HTML viewer to {html}");
                return Ok(());
            }

            if cfg.output.is_none() && !cfg.embed_data {
                anyhow::bail!("when -i/--input is supplied, provide -o/--output or --embed_data.");
            }
            if cfg.embed_data && cfg.html.is_none() {
                anyhow::bail!("--embed_data requires --html.");
            }

            let input = cfg.input.clone().unwrap();
            let name = cfg
                .name
                .clone()
                .or_else(|| cfg.output.as_deref().map(stem))
                .or_else(|| cfg.html.as_deref().map(stem))
                .unwrap_or_else(|| "Domainator SSN Viewer".to_string());

            let subset_labels = if cfg.subset.is_empty() && cfg.subset_file.is_none() {
                None
            } else {
                let mut set: std::collections::HashSet<String> =
                    cfg.subset.iter().cloned().collect();
                if let Some(f) = &cfg.subset_file {
                    for line in std::fs::read_to_string(f)?.lines() {
                        let l = line.trim();
                        if !l.is_empty() {
                            set.insert(l.to_string());
                        }
                    }
                }
                Some(set)
            };

            let matrix = read_matrix(&input).with_context(|| format!("reading matrix {input}"))?;
            let opts = config::resolve::resolve_build_ssn_viewer(&cfg, name.clone(), subset_labels);
            let bundle =
                build_ssn_viewer_bundle(&matrix, &opts).context("building SSN viewer bundle")?;
            let compressed = gzip(&serialize_bundle(&bundle));

            if let Some(out) = &cfg.output {
                // Output-size guardrail on the compressed bundle (default 25 GB, 0 disables).
                enforce_bytes_gb(
                    compressed.len() as u64,
                    cfg.max_output_gb.unwrap_or(25.0),
                    &format!("SSN viewer bundle output '{out}'"),
                    "--subset, --subset_file",
                )?;
                std::fs::write(out, &compressed)?;
                println!("wrote {}-node .dsnv bundle to {out}", matrix.nrows());
            }
            if let Some(html) = &cfg.html {
                let embed = if cfg.embed_data {
                    Some(compressed.as_slice())
                } else {
                    None
                };
                std::fs::write(html, render_html(&name, embed))?;
                println!("wrote HTML viewer to {html}");
            }
        }
        Command::MatrixReport {
            input,
            output,
            json,
            merge_impact_metric,
            max_merge_events,
            config: config_path,
            print_config: do_print,
        } => {
            let cli = config::MatrixReportConfig {
                input: opt_path(&input)?,
                output: opt_path(&output)?,
                json: opt_path(&json)?,
                merge_impact_metric,
                max_merge_events,
            };
            let mut cfg = match &config_path {
                Some(p) => load_config::<config::MatrixReportConfig>(p)?,
                None => config::MatrixReportConfig::default(),
            };
            cfg.merge_cli(cli);
            config::reject_non_finite(&cfg)?;
            if do_print {
                print!("{}", print_config(&cfg)?);
                return Ok(());
            }
            let input = cfg
                .input
                .context("missing required `input` (-i / --input or config)")?;
            if cfg.output.is_none() && cfg.json.is_none() {
                anyhow::bail!("no output specified: pass --output (text) and/or --json");
            }
            let matrix = read_matrix(&input).with_context(|| format!("reading matrix {input}"))?;
            let metric = cfg
                .merge_impact_metric
                .clone()
                .unwrap_or_else(|| "min_child".into());
            let max_events = cfg
                .max_merge_events
                .unwrap_or(config::resolve::DEFAULT_MAX_MERGE_EVENTS);
            let report = matrix_report(&matrix, &metric, max_events);
            if let Some(p) = &cfg.output {
                std::fs::write(p, render_text(&report))?;
            }
            if let Some(p) = &cfg.json {
                let text = serde_json::to_string_pretty(&to_json(&report))?;
                // `--json -` is stdout, as upstream; only `--json` has that spelling.
                if p == "-" {
                    println!("{text}");
                } else {
                    std::fs::write(p, text)?;
                }
            }
            // A status line, so stderr: stdout may be carrying the JSON.
            eprintln!(
                "matrix report: {} nodes, {} edges",
                report.n_nodes, report.n_edges
            );
        }
        Command::SummaryReport {
            input,
            output,
            json,
            domains,
            databases,
            taxonomy,
            ncbi_taxonomy_path,
            partial,
            config: config_path,
            print_config: do_print,
        } => {
            use domainator_core::tools::summary_report as sr;
            let cli = config::SummaryReportConfig {
                input: paths_to_strings(&input)?,
                output: opt_path(&output)?,
                json,
                domains,
                databases,
                taxonomy,
                ncbi_taxonomy_path,
                partial,
            };
            let mut cfg = match &config_path {
                Some(p) => load_config::<config::SummaryReportConfig>(p)?,
                None => config::SummaryReportConfig::default(),
            };
            cfg.merge_cli(cli);
            config::reject_non_finite(&cfg)?;
            if do_print {
                print!("{}", print_config(&cfg)?);
                return Ok(());
            }
            let mut records: Vec<SeqRecord> = Vec::new();
            if cfg.input.is_empty() {
                records.extend(load_inputs(&[])?);
            } else {
                for path in &cfg.input {
                    records.extend(load(&PathBuf::from(path))?);
                }
            }
            let tax = if cfg.taxonomy {
                let p = cfg
                    .ncbi_taxonomy_path
                    .clone()
                    .unwrap_or_else(default_ncbi_taxonomy_path);
                Some(load_taxonomy(&p)?)
            } else {
                None
            };
            let opts = sr::SummaryReportOptions {
                focus_domains: cfg.domains.clone(),
                databases: cfg.databases.clone(),
                report_taxonomy: cfg.taxonomy,
                partial: config::parse_partial(cfg.partial.as_deref())?,
            };
            let report = sr::summary_report(&records, &opts, tax.as_ref());

            if let Some(p) = &cfg.json {
                let text = format!("{}\n", serde_json::to_string(&sr::to_json(&report))?);
                if p == "-" {
                    print!("{text}");
                } else {
                    std::fs::write(p, text)?;
                }
            }
            // Text goes to -o, or to stdout when nothing else was asked for.
            let want_text = cfg.output.is_some() || cfg.json.is_none();
            if want_text {
                let text = sr::render_text(&report);
                match &cfg.output {
                    Some(p) => std::fs::write(p, text)?,
                    None => print!("{text}"),
                }
            }
        }
        Command::SsnNavigator {
            input,
            output,
            mode,
            threshold,
            id,
            node,
            min_size,
            top_n,
            members,
            max_merge_events,
            config: config_path,
            print_config: do_print,
        } => {
            let cli = config::SsnNavigatorConfig {
                input: opt_path(&input)?,
                output: opt_path(&output)?,
                mode,
                threshold,
                id,
                node,
                min_size,
                top_n,
                members,
                max_merge_events,
            };
            let mut cfg = match &config_path {
                Some(p) => load_config::<config::SsnNavigatorConfig>(p)?,
                None => config::SsnNavigatorConfig::default(),
            };
            cfg.merge_cli(cli);
            config::reject_non_finite(&cfg)?;
            if do_print {
                print!("{}", print_config(&cfg)?);
                return Ok(());
            }
            let input = cfg
                .input
                .clone()
                .context("missing required `input` (-i / --input or config)")?;
            let bundle = ssn_bundle::load_bundle(Path::new(&input))
                .with_context(|| format!("reading bundle {input}"))?;
            let opts = config::resolve::resolve_ssn_navigator(&cfg);
            let result = ssn_navigator(&bundle, &opts).map_err(anyhow::Error::msg)?;
            // Compact, single-line JSON on one line, matching Python's separators.
            let text = format!("{}\n", serde_json::to_string(&result)?);
            match &cfg.output {
                Some(p) => std::fs::write(p, text)?,
                None => print!("{text}"),
            }
        }
        Command::Describe { tool, indent } => {
            for t in &tool {
                if !config::registry::tool_info(t).is_some_and(|i| i.in_manifest) {
                    return Err(commands::usage_error(format!(
                        "describe: unknown tool '{t}'; `domainatoredux --help` lists the tools"
                    )));
                }
            }
            let only = (!tool.is_empty()).then_some(tool.as_slice());
            let doc = config::manifest::manifest(&command(), only);
            let mut out = std::io::stdout().lock();
            match indent {
                Some(n) => {
                    let pad = vec![b' '; n];
                    let fmt = serde_json::ser::PrettyFormatter::with_indent(&pad);
                    let mut ser = serde_json::Serializer::with_formatter(&mut out, fmt);
                    serde::Serialize::serialize(&doc, &mut ser)?;
                }
                None => serde_json::to_writer(&mut out, &doc)?,
            }
            writeln!(out)?;
        }
        Command::Completion { shell } => {
            clap_complete::generate(
                shell,
                &mut command(),
                "domainatoredux",
                &mut std::io::stdout(),
            );
        }
        Command::EnumReport(args) => commands::enum_report::run(&args)?,
        Command::SelectByCds(args) => commands::select_by_cds::run(&args)?,
        Command::SelectByContig(args) => commands::select_by_contig::run(&args)?,
        Command::HmmerCompare {
            input,
            reference,
            output,
            score_cutoff,
            k,
            alignments,
            cpu,
            max_output_gb,
            config: config_path,
            print_config: do_print,
        } => {
            let cli = config::HmmerCompareConfig {
                input: paths_to_strings(&input)?,
                reference: paths_to_strings(&reference)?,
                output: opt_path(&output)?,
                score_cutoff,
                k,
                alignments,
                cpu,
                max_output_gb,
            };
            let mut cfg = match &config_path {
                Some(p) => load_config::<config::HmmerCompareConfig>(p)?,
                None => config::HmmerCompareConfig::default(),
            };
            cfg.merge_cli(cli);
            config::reject_non_finite(&cfg)?;
            if do_print {
                print!("{}", print_config(&cfg)?);
                return Ok(());
            }
            let opts = HmmerCompareOptions {
                score_cutoff: cfg.score_cutoff.unwrap_or(0.0),
                alignments: cfg.alignments,
                k: cfg.k,
                cpu: cfg.cpu.unwrap_or(0),
                max_output_gb: Some(cfg.max_output_gb.unwrap_or(25.0)),
            };
            match &cfg.output {
                Some(p) => {
                    // Buffer in memory so a tripped output-size guardrail leaves no
                    // partial file (mirrors the Python tool's temp-file + rename).
                    let mut buf: Vec<u8> = Vec::new();
                    hmmer_compare(&cfg.input, &cfg.reference, &opts, &mut buf)
                        .context("running hmmer_compare")?;
                    std::fs::write(p, &buf)?;
                }
                None => {
                    let stdout = std::io::stdout();
                    let mut h = stdout.lock();
                    hmmer_compare(&cfg.input, &cfg.reference, &opts, &mut h)
                        .context("running hmmer_compare")?;
                }
            }
        }
        Command::HmmerReport(args) => commands::hmmer_report::run(&args)?,
        Command::HmmerSelect {
            input,
            output,
            regex,
            exact,
            contains,
            field,
            case_sensitive,
            alphabet,
            config: config_path,
            print_config: do_print,
        } => {
            use domainator_core::tools::hmmer_select as hs;
            let cli = config::HmmerSelectConfig {
                input: paths_to_strings(&input)?,
                output: opt_path(&output)?,
                regex,
                exact,
                contains,
                field,
                case_sensitive,
                alphabet,
            };
            let mut cfg = match &config_path {
                Some(p) => load_config::<config::HmmerSelectConfig>(p)?,
                None => config::HmmerSelectConfig::default(),
            };
            cfg.merge_cli(cli);
            config::reject_non_finite(&cfg)?;
            if do_print {
                print!("{}", print_config(&cfg)?);
                return Ok(());
            }
            if cfg.input.is_empty() {
                anyhow::bail!("hmmer_select requires at least one -i/--input .hmm file");
            }
            let fields = hs::Fields::from_field(cfg.field.as_deref().unwrap_or("all"))
                .map_err(|e| anyhow::anyhow!(e))?;
            let opts = hs::SelectOptions {
                regex: cfg.regex,
                exact: cfg.exact,
                contains: cfg.contains,
                fields,
                case_sensitive: cfg.case_sensitive,
                alphabet: hs::parse_alphabet(cfg.alphabet.as_deref())
                    .map_err(|e| anyhow::anyhow!(e))?,
            };
            let summary = match &cfg.output {
                Some(p) => write_hmm_output(p, |f| hs::hmmer_select(&cfg.input, &opts, f))
                    .map_err(|e| anyhow::anyhow!(e))?,
                None => {
                    let stdout = std::io::stdout();
                    let mut h = stdout.lock();
                    hs::hmmer_select(&cfg.input, &opts, &mut h).map_err(|e| anyhow::anyhow!(e))?
                }
            };
            eprintln!("Found {} matching profiles.", summary.found);
            if summary.skipped_files > 0 {
                eprintln!(
                    "Skipped {} input file(s) whose alphabet did not match --alphabet.",
                    summary.skipped_files
                );
            }
        }
        Command::HmmerBuild {
            input,
            output,
            name,
            acc,
            desc,
            alphabet,
            window_length,
            window_beta,
            config: config_path,
            print_config: do_print,
        } => {
            use domainator_core::tools::hmmer_build as hb;
            let cli = config::HmmerBuildConfig {
                input: opt_path(&input)?,
                output: opt_path(&output)?,
                name,
                acc,
                desc,
                alphabet,
                window_length,
                window_beta,
            };
            let mut cfg = match &config_path {
                Some(p) => load_config::<config::HmmerBuildConfig>(p)?,
                None => config::HmmerBuildConfig::default(),
            };
            cfg.merge_cli(cli);
            config::reject_non_finite(&cfg)?;
            if do_print {
                print!("{}", print_config(&cfg)?);
                return Ok(());
            }
            let name = cfg.name.clone().context("missing required `--name`")?;
            let input = cfg.input.clone().context(
                "missing required `input` (-i / --input); stdin MSA is not yet supported, pass a file",
            )?;
            let opts = hb::HmmerBuildOptions {
                name,
                acc: cfg.acc.clone(),
                desc: cfg.desc.clone(),
                alphabet: cfg.alphabet.clone(),
                window_length: cfg.window_length,
                window_beta: cfg.window_beta,
            };
            match &cfg.output {
                Some(p) => write_hmm_output(p, |f| {
                    let hmm = hb::hmmer_build(&PathBuf::from(&input), &opts)?;
                    hb::write_hmm_out(&hmm, f)
                })
                .map_err(|e| anyhow::anyhow!(e))?,
                None => {
                    let hmm = hb::hmmer_build(&PathBuf::from(&input), &opts)
                        .map_err(|e| anyhow::anyhow!(e))?;
                    let stdout = std::io::stdout();
                    let mut h = stdout.lock();
                    hb::write_hmm_out(&hmm, &mut h).map_err(|e| anyhow::anyhow!(e))?;
                }
            }
        }
        Command::HmmerSearch {
            input,
            reference,
            output,
            score_cutoff,
            max_hits,
            cpu,
            max_output_gb,
            config: config_path,
            print_config: do_print,
        } => {
            use domainator_core::tools::hmmer_search as hsr;
            let cli = config::HmmerSearchConfig {
                input: paths_to_strings(&input)?,
                reference: paths_to_strings(&reference)?,
                output: opt_path(&output)?,
                score_cutoff,
                max_hits,
                cpu,
                max_output_gb,
            };
            let mut cfg = match &config_path {
                Some(p) => load_config::<config::HmmerSearchConfig>(p)?,
                None => config::HmmerSearchConfig::default(),
            };
            cfg.merge_cli(cli);
            config::reject_non_finite(&cfg)?;
            if do_print {
                print!("{}", print_config(&cfg)?);
                return Ok(());
            }
            if cfg.reference.is_empty() {
                anyhow::bail!("hmmer_search requires at least one -r/--reference .hmm file");
            }
            if cfg.input.is_empty() {
                anyhow::bail!("hmmer_search requires at least one -i/--input .hmm file");
            }
            // The default --score_cutoff is scaled to the REFERENCE alphabet: profile
            // scores are log-odds sums whose per-column ceiling is log(K), so a
            // protein-tuned threshold is about twice as strict on nucleotides and
            // silently returns nothing. An explicit value is always taken literally.
            let score_cutoff = match cfg.score_cutoff {
                Some(v) => v,
                None => {
                    let (resolved, abc) = hsr::default_score_cutoff_for(&cfg.reference)
                        .map_err(|e| anyhow::anyhow!(e))?;
                    eprintln!(
                        "Using --score_cutoff {} (the default for {abc} profiles).",
                        hsr::py_g(resolved)
                    );
                    resolved
                }
            };
            let opts = hsr::HmmerSearchOptions {
                score_cutoff,
                max_hits: cfg.max_hits,
                cpu: cfg.cpu.unwrap_or(0),
                max_output_gb: cfg.max_output_gb,
            };
            let found = match &cfg.output {
                Some(p) => write_hmm_output(p, |f| {
                    hsr::hmmer_search(&cfg.input, &cfg.reference, &opts, f)
                })
                .map_err(|e| anyhow::anyhow!(e))?,
                None => {
                    let stdout = std::io::stdout();
                    let mut h = stdout.lock();
                    hsr::hmmer_search(&cfg.input, &cfg.reference, &opts, &mut h)
                        .map_err(|e| anyhow::anyhow!(e))?
                }
            };
            eprintln!("Kept {found} query profiles.");
        }
        Command::FormatDb {
            input,
            shards,
            compress,
            index,
            output_dir,
            name,
            level,
            force,
            log,
            config: config_path,
            print_config: do_print,
        } => {
            use domainator_core::tools::format_db as fdb;
            let cli = config::FormatDbConfig {
                input: paths_to_strings(&input)?,
                shards,
                compress,
                index,
                output_dir,
                name,
                level,
                force,
                log: opt_path(&log)?,
            };
            let mut cfg = match &config_path {
                Some(p) => load_config::<config::FormatDbConfig>(p)?,
                None => config::FormatDbConfig::default(),
            };
            cfg.merge_cli(cli);
            config::reject_non_finite(&cfg)?;
            if do_print {
                print!("{}", print_config(&cfg)?);
                return Ok(());
            }
            if cfg.input.is_empty() {
                anyhow::bail!("format_db requires at least one -i/--input database file");
            }
            let opts = fdb::FormatDbOptions {
                shards: cfg.shards,
                compress: cfg.compress,
                index: cfg.index,
                output_dir: cfg.output_dir,
                name: cfg.name,
                level: cfg.level.unwrap_or(6),
                force: cfg.force,
            };
            let result = match &cfg.log {
                Some(p) => {
                    let mut f = std::io::BufWriter::new(std::fs::File::create(p)?);
                    let r = fdb::format_db(&cfg.input, &opts, &mut f);
                    f.flush()?;
                    r
                }
                None => {
                    let stderr = std::io::stderr();
                    let mut h = stderr.lock();
                    fdb::format_db(&cfg.input, &opts, &mut h)
                }
            };
            result.map_err(|e| anyhow::anyhow!(e))?;
        }
    }
    Ok(())
}
