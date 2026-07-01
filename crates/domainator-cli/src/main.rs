//! `domainator <tool> [opts]` — single-executable CLI for the Domainator Rust
//! rewrite. During M0 this exposes development subcommands (`inspect`, `convert`)
//! used to validate the parser/writer against the Python fixtures; tool
//! subcommands (domainate, domain_search, seq_dist, compare_contigs) land in M1+.

use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

mod config;
use config::{load_config, print_config, schema, ColumnSpec, EnumConfig};

use domainator_core::io::genbank::writer::write_record;
use domainator_core::io::genbank::{read_genbank, write_genbank};
use domainator_core::io::{detect_filetype, fasta, FileType};
use domainator_core::matrix::{hdf5_io, text};
use domainator_core::seq::taxonomy::NCBITaxonomy;
use domainator_core::seq::SeqRecord;
use domainator_core::tools::enum_report::{self, needs_taxonomy, Analysis, OutputFormat};
use domainator_core::tools::select_by_cds::{pad_records, select_by_cds, SelectByCdsOptions};
use domainator_core::tools::select_by_contig::{select_by_contig, SelectByContigOptions};
use domainator_core::tools::select_common::parse_name_list;
use domainator_core::tools::compare_contigs::{compare_contigs, CompareContigsOptions};
use domainator_core::tools::hmmer_compare::{hmmer_compare, HmmerCompareOptions};
use domainator_core::tools::seq_dist::{detect_kind, seq_dist, SeqDistOptions};
use domainator_core::tools::{domain_search, domainate};

#[derive(Parser)]
#[command(name = "domainator", version, about = "Domainator (Rust): protein-domain / SSN analysis")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
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
    Domainate {
        /// Input sequence file (GenBank or FASTA).
        #[arg(short, long)]
        input: Option<PathBuf>,
        /// Reference `.hmm` file(s).
        #[arg(short, long, num_args = 1..)]
        references: Vec<PathBuf>,
        /// Output GenBank file.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// E-value threshold (default: 0.001).
        #[arg(short = 'e', long)]
        evalue: Option<f64>,
        /// E-value search space size (-Z) (default: 1000).
        #[arg(short = 'Z', long = "z")]
        z: Option<f64>,
        /// Maximum fractional domain overlap before filtering (>=1 disables; default 1.0).
        #[arg(long = "max_overlap")]
        max_overlap: Option<f64>,
        /// Read parameters from a JSON/YAML config file (CLI flags override it).
        #[arg(long)]
        config: Option<String>,
        /// Print the resolved parameters as YAML and exit.
        #[arg(long = "print_config")]
        print_config: bool,
    },
    /// Search references against a (possibly sharded/BGZF/indexed) database.
    DomainSearch {
        /// Input database (GenBank or FASTA; logical name resolves shards).
        #[arg(short, long)]
        input: Option<PathBuf>,
        /// Reference `.hmm` file(s).
        #[arg(short, long, num_args = 1..)]
        references: Vec<PathBuf>,
        /// Output GenBank file.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// E-value threshold (default: 0.001).
        #[arg(short = 'e', long)]
        evalue: Option<f64>,
        /// E-value search space size (-Z) (default: 1000).
        #[arg(short = 'Z', long = "z")]
        z: Option<f64>,
        /// Add full Domainator annotations, not just the best-hit feature.
        #[arg(long = "add_annotations")]
        add_annotations: bool,
        /// Cap on total hit contigs returned (top-N by best domain score).
        #[arg(long = "max_hits")]
        max_hits: Option<usize>,
        /// Approximate CDSs per partition (default: 10000).
        #[arg(long = "batch_size")]
        batch_size: Option<u64>,
        /// Worker threads (0 = all cores).
        #[arg(long)]
        cpu: Option<usize>,
        /// Extract whole matching contigs (nucleotide).
        #[arg(long = "whole_contig")]
        whole_contig: bool,
        /// Extract this many CDSs up- and downstream of each hit CDS.
        #[arg(long = "cds_range")]
        cds_range: Option<i64>,
        /// CDSs upstream of each hit CDS.
        #[arg(long = "cds_up")]
        cds_up: Option<i64>,
        /// CDSs downstream of each hit CDS.
        #[arg(long = "cds_down")]
        cds_down: Option<i64>,
        /// Extract this many kb up- and downstream of each hit CDS.
        #[arg(long = "kb_range")]
        kb_range: Option<f64>,
        /// kb upstream of each hit CDS.
        #[arg(long = "kb_up")]
        kb_up: Option<f64>,
        /// kb downstream of each hit CDS.
        #[arg(long = "kb_down")]
        kb_down: Option<f64>,
        /// Only extract regions around hit CDSs on this strand (f/r).
        #[arg(long)]
        strand: Option<String>,
        /// Keep the hit CDS on its original strand (don't reverse-complement).
        #[arg(long = "keep_direction")]
        keep_direction: bool,
        /// Translate nucleotide CDS hits to peptides (no neighborhood extraction).
        #[arg(long)]
        translate: bool,
        /// Max fractional overlap between output regions (>=1 disables; default 1.0).
        #[arg(long = "max_region_overlap")]
        max_region_overlap: Option<f64>,
        /// Read parameters from a JSON/YAML config file (CLI flags override it).
        #[arg(long)]
        config: Option<String>,
        /// Print the resolved parameters as YAML and exit.
        #[arg(long = "print_config")]
        print_config: bool,
    },
    /// Compute a similarity matrix between input and reference sequences/profiles.
    SeqDist {
        /// Input query file (FASTA/GenBank; HMM input needs hmmer_compare, deferred).
        #[arg(short, long)]
        input: Option<PathBuf>,
        /// Reference file (FASTA/GenBank/HMM). Defaults to the input (all-vs-all).
        #[arg(short, long)]
        reference: Option<PathBuf>,
        /// Write a dense HDF5 matrix here.
        #[arg(long)]
        dense: Option<PathBuf>,
        /// Write a dense TSV matrix here.
        #[arg(long = "dense_text")]
        dense_text: Option<PathBuf>,
        /// Write a sparse (CSR) HDF5 matrix here.
        #[arg(long)]
        sparse: Option<PathBuf>,
        /// Distance metric (default: diamond_us). diamond_*: f=fast, d=default,
        /// mids=mid-sensitive, s=sensitive, mors=more-sensitive, vs=very-sensitive,
        /// us=ultra-sensitive (needs the external diamond binary). hmmer:
        /// phmmer/hmmsearch. hmmer_compare: HMM-vs-HMM (auto for two HMM files).
        #[arg(long, value_parser = [
            "diamond_f", "diamond_d", "diamond_mids", "diamond_s", "diamond_mors",
            "diamond_vs", "diamond_us", "hmmer", "hmmer_compare",
        ])]
        algorithm: Option<String>,
        /// Matrix value type (default: score).
        #[arg(long)]
        mode: Option<String>,
        /// Round raw scores <= this down to zero (default: 0).
        #[arg(long)]
        lb: Option<f64>,
        /// Keep at most this many non-zero entries per input row.
        #[arg(short = 'k', long)]
        k: Option<usize>,
        /// Prune to MST + OR-symmetric kNN edges (integer > 1; requires reference
        /// == input and --mode score/bool/efi_score). When set, -k is ignored.
        #[arg(long = "mst_knn", value_parser = parse_mst_knn)]
        mst_knn: Option<usize>,
        /// Worker threads (0 = all cores).
        #[arg(long)]
        cpu: Option<usize>,
        /// Read parameters from a JSON/YAML config file (CLI flags override it).
        #[arg(long)]
        config: Option<String>,
        /// Print the resolved parameters as YAML and exit.
        #[arg(long = "print_config")]
        print_config: bool,
    },
    /// Pairwise contig similarity (Jaccard + Adjacency) from annotated GenBank.
    CompareContigs {
        /// Annotated GenBank file(s).
        #[arg(short, long, num_args = 1..)]
        input: Vec<PathBuf>,
        /// Write a dense HDF5 matrix here.
        #[arg(long)]
        dense: Option<PathBuf>,
        /// Write a dense TSV matrix here.
        #[arg(long = "dense_text")]
        dense_text: Option<PathBuf>,
        /// Write a sparse (CSR) HDF5 matrix here.
        #[arg(long)]
        sparse: Option<PathBuf>,
        /// Jaccard-index weight (default: 0.5).
        #[arg(long)]
        ji: Option<f64>,
        /// Adjacency-index weight (default: 0.5).
        #[arg(long)]
        ai: Option<f64>,
        /// Restrict to domains from these databases.
        #[arg(long, num_args = 1..)]
        databases: Vec<String>,
        /// Keep at most this many non-zero entries per row (incl. the diagonal).
        #[arg(short = 'k', long)]
        k: Option<usize>,
        /// Zero out final scores <= this (default: 0).
        #[arg(long)]
        lb: Option<f64>,
        /// Prune to MST + OR-symmetric kNN edges (integer > 1). When set, -k is ignored.
        #[arg(long = "mst_knn", value_parser = parse_mst_knn)]
        mst_knn: Option<usize>,
        /// Worker threads (0 = all cores).
        #[arg(long)]
        cpu: Option<usize>,
        /// Read parameters from a JSON/YAML config file (CLI flags override it).
        #[arg(long)]
        config: Option<String>,
        /// Print the resolved parameters as YAML and exit.
        #[arg(long = "print_config")]
        print_config: bool,
    },
    /// Print a JSON Schema for a tool's `--config` file.
    Schema {
        /// Tool name: any of the tool subcommands (e.g. domainate, seq-dist, enum-report).
        tool: String,
    },
    /// Write a TSV table of per-contig/CDS/domain info (column order = flag order).
    EnumReport {
        /// All enum_report arguments (parsed in order; see the Python tool's flags).
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, num_args = 0..)]
        args: Vec<String>,
    },
    /// Extract contig regions around selected CDSs (by name/domain/expr/hits).
    SelectByCds {
        /// Input file(s) (GenBank or FASTA). Reads stdin GenBank if omitted.
        #[arg(short, long, num_args = 1..)]
        input: Vec<PathBuf>,
        /// Output GenBank file. Writes to stdout if omitted.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// kb up- and downstream of the selected CDS to extract.
        #[arg(long = "kb_range")]
        kb_range: Option<f64>,
        /// kb upstream of the selected CDS.
        #[arg(long = "kb_up")]
        kb_up: Option<f64>,
        /// kb downstream of the selected CDS.
        #[arg(long = "kb_down")]
        kb_down: Option<f64>,
        /// CDSs up- and downstream of the selected CDS.
        #[arg(long = "cds_range")]
        cds_range: Option<i64>,
        /// CDSs upstream of the selected CDS.
        #[arg(long = "cds_up")]
        cds_up: Option<i64>,
        /// CDSs downstream of the selected CDS.
        #[arg(long = "cds_down")]
        cds_down: Option<i64>,
        /// Extract the whole matching contig.
        #[arg(long = "whole_contig")]
        whole_contig: bool,
        /// Only extract from contigs with these names.
        #[arg(long, num_args = 1..)]
        contigs: Vec<String>,
        /// Text file of contig names.
        #[arg(long = "contigs_file")]
        contigs_file: Option<PathBuf>,
        /// Include CDSs with these names.
        #[arg(long, num_args = 1..)]
        cds: Vec<String>,
        /// Text file of CDS names.
        #[arg(long = "cds_file")]
        cds_file: Option<PathBuf>,
        /// Include CDSs containing domains with these names.
        #[arg(long, num_args = 1..)]
        domains: Vec<String>,
        /// Text file of domain names.
        #[arg(long = "domains_file")]
        domains_file: Option<PathBuf>,
        /// Boolean domain expression (& AND, | OR, ~ NOT).
        #[arg(long = "domain_expr")]
        domain_expr: Option<String>,
        /// Restrict domain consideration to these databases.
        #[arg(long, num_args = 1..)]
        databases: Vec<String>,
        /// Only extract regions around CDSs on this strand (f/r).
        #[arg(long)]
        strand: Option<String>,
        /// Select CDSs with no domain annotations.
        #[arg(long)]
        unannotated: bool,
        /// Select CDSs marked as search hits (Domain_Search).
        #[arg(long = "search_hits")]
        search_hits: bool,
        /// Pad sequence ends so the focus CDS aligns.
        #[arg(long)]
        pad: bool,
        /// Keep the focus CDS on its original strand (don't reverse-complement).
        #[arg(long = "keep_direction")]
        keep_direction: bool,
        /// Keep duplicate regions (don't deduplicate by name).
        #[arg(long = "skip_deduplicate")]
        skip_deduplicate: bool,
        /// Max fractional overlap between output regions (>=1 disables; default 1.0).
        #[arg(long = "max_region_overlap")]
        max_region_overlap: Option<f64>,
        /// E-value cutoff for domain annotations (default: 1e8).
        #[arg(short = 'e', long)]
        evalue: Option<f64>,
        /// Invert the CDS selection criteria.
        #[arg(long)]
        invert: bool,
        /// Include nucleic-acid annotations with no associated CDS.
        #[arg(long = "include_nucleic_acids")]
        include_nucleic_acids: bool,
        /// Read parameters from a JSON/YAML config file (CLI flags override it).
        #[arg(long)]
        config: Option<String>,
        /// Print the resolved parameters as YAML and exit.
        #[arg(long = "print_config")]
        print_config: bool,
    },
    /// Select a subset of contigs from a GenBank/FASTA file by various criteria.
    SelectByContig {
        /// Input file(s) (GenBank or FASTA). Reads stdin GenBank if omitted.
        #[arg(short, long, num_args = 1..)]
        input: Vec<PathBuf>,
        /// Output GenBank/FASTA file. Writes to stdout if omitted.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Only consider contigs with these names.
        #[arg(long, num_args = 1..)]
        contigs: Vec<String>,
        /// Text file of contig names to select.
        #[arg(long = "contigs_file")]
        contigs_file: Option<PathBuf>,
        /// Select contigs with names matching this regex.
        #[arg(long = "contigs_regex")]
        contigs_regex: Option<String>,
        /// Select contigs with a definition matching this regex.
        #[arg(long = "definition_regex")]
        definition_regex: Option<String>,
        /// Select contigs with a sequence matching this regex.
        #[arg(long = "sequence_regex")]
        sequence_regex: Option<String>,
        /// Select contigs containing domains with these names.
        #[arg(long, num_args = 1..)]
        domains: Vec<String>,
        /// Text file of domain names to select.
        #[arg(long = "domains_file")]
        domains_file: Option<PathBuf>,
        /// Boolean domain expression (& AND, | OR, ~ NOT).
        #[arg(long = "domain_expr")]
        domain_expr: Option<String>,
        /// Domain type to consider (default: domain).
        #[arg(long = "domain_type", value_parser = ["domain", "search", "both"])]
        domain_type: Option<String>,
        /// Restrict domain consideration to these databases.
        #[arg(long, num_args = 1..)]
        databases: Vec<String>,
        /// Select contigs with no domain annotations.
        #[arg(long)]
        unannotated: bool,
        /// E-value cutoff for domain annotations (default: 1e8).
        #[arg(short = 'e', long)]
        evalue: Option<f64>,
        /// Write FASTA output instead of GenBank.
        #[arg(long = "fasta_out")]
        fasta_out: bool,
        /// Only keep contigs with a taxid in this list (lineage match).
        #[arg(long = "include_taxids", num_args = 1..)]
        include_taxids: Vec<i64>,
        /// Exclude contigs with a taxid in this list (lineage match).
        #[arg(long = "exclude_taxids", num_args = 1..)]
        exclude_taxids: Vec<i64>,
        /// Boolean taxonomy expression over taxids.
        #[arg(long = "taxonomy_expr")]
        taxonomy_expr: Option<String>,
        /// Path to the NCBI taxonomy database directory (default: /tmp/ncbi_taxonomy).
        #[arg(long = "ncbi_taxonomy_path")]
        ncbi_taxonomy_path: Option<String>,
        /// Stop after returning this many contigs.
        #[arg(long)]
        first: Option<usize>,
        /// Skip contigs smaller than this.
        #[arg(long = "length_lb")]
        length_lb: Option<i64>,
        /// Skip contigs larger than this.
        #[arg(long = "length_ub")]
        length_ub: Option<i64>,
        /// Skip contigs with fewer CDSs than this.
        #[arg(long = "cds_count_lb")]
        cds_count_lb: Option<i64>,
        /// Skip contigs with more CDSs than this.
        #[arg(long = "cds_count_ub")]
        cds_count_ub: Option<i64>,
        /// Skip contigs with a domain_search score below this.
        #[arg(long = "search_score_lb")]
        search_score_lb: Option<f64>,
        /// Skip contigs with a domain_search score above this.
        #[arg(long = "search_score_ub")]
        search_score_ub: Option<f64>,
        /// Invert the (middle) selection criteria.
        #[arg(long)]
        invert: bool,
        /// Read parameters from a JSON/YAML config file (CLI flags override it).
        #[arg(long)]
        config: Option<String>,
        /// Print the resolved parameters as YAML and exit.
        #[arg(long = "print_config")]
        print_config: bool,
    },
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
        /// Read parameters from a JSON/YAML config file (CLI flags override it).
        #[arg(long)]
        config: Option<String>,
        /// Print the resolved parameters as YAML and exit.
        #[arg(long = "print_config")]
        print_config: bool,
    },
}

/// Parse `--mst_knn`: an integer greater than 1 (mirrors
/// `transform_matrix._mst_knn_arg`).
fn parse_mst_knn(value: &str) -> std::result::Result<usize, String> {
    let v: i64 = value.parse().map_err(|_| format!("`{value}` is not an integer"))?;
    if v <= 1 {
        return Err("--mst_knn must be an integer greater than 1".to_string());
    }
    Ok(v as usize)
}

fn load(input: &PathBuf) -> Result<Vec<SeqRecord>> {
    let path = input.to_str().context("non-UTF8 path")?;
    match detect_filetype(input) {
        Some(FileType::Genbank) => read_genbank(path).context("reading GenBank"),
        Some(FileType::Fasta) => fasta::read_fasta(path, Some("protein")).context("reading FASTA"),
        None => anyhow::bail!("could not detect file type from extension: {}", input.display()),
    }
}

/// Load and concatenate one or more sequence files; if `inputs` is empty, read
/// GenBank from stdin (matching the Python tools' stdin default).
fn load_inputs(inputs: &[PathBuf]) -> Result<Vec<SeqRecord>> {
    if inputs.is_empty() {
        use std::io::Read;
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf).context("reading stdin")?;
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

/// Like [`load_inputs`] but over string paths (config-file shape).
fn load_inputs_str(inputs: &[String]) -> Result<Vec<SeqRecord>> {
    let paths: Vec<PathBuf> = inputs.iter().map(PathBuf::from).collect();
    load_inputs(&paths)
}

/// Convert paths to UTF-8 strings.
fn paths_to_strings(paths: &[PathBuf]) -> Result<Vec<String>> {
    paths.iter().map(|p| p.to_str().map(str::to_string).context("non-UTF8 path")).collect()
}

/// Convert an optional path to an optional UTF-8 string.
fn opt_path(path: &Option<PathBuf>) -> Result<Option<String>> {
    match path {
        Some(p) => Ok(Some(p.to_str().context("non-UTF8 path")?.to_string())),
        None => Ok(None),
    }
}

/// Write records as GenBank to a file, or to stdout when `output` is None.
fn write_genbank_out(records: &[SeqRecord], output: Option<&PathBuf>) -> Result<()> {
    match output {
        Some(p) => write_genbank(p.to_str().context("non-UTF8 path")?, records).context("writing GenBank"),
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

/// Write records as FASTA to a file, or to stdout when `output` is None.
fn write_fasta_out(records: &[SeqRecord], output: Option<&PathBuf>) -> Result<()> {
    match output {
        Some(p) => fasta::write_fasta(p.to_str().context("non-UTF8 path")?, records).context("writing FASTA"),
        None => {
            let stdout = std::io::stdout();
            let mut h = std::io::BufWriter::new(stdout.lock());
            for rec in records {
                let header = if rec.description.is_empty() { &rec.id } else { &rec.description };
                writeln!(h, ">{header}")?;
                for chunk in rec.seq.chunks(60) {
                    writeln!(h, "{}", String::from_utf8_lossy(chunk))?;
                }
            }
            h.flush()?;
            Ok(())
        }
    }
}

/// Load an `NCBITaxonomy` from `--ncbi_taxonomy_path` (only when a taxonomy
/// filter/column is in use).
fn load_taxonomy(path: &str) -> Result<NCBITaxonomy> {
    NCBITaxonomy::load(std::path::Path::new(path)).context("loading NCBI taxonomy")
}

/// CLI-provided `enum_report` params (column order preserved), plus the
/// `--config`/`--print_config` control flags.
struct EnumCli {
    config: EnumConfig,
    config_path: Option<String>,
    print_config: bool,
}

/// Parse `enum_report` args by hand so the relative order of column flags is
/// preserved (mirrors the Python tool's ordered argparse actions), building a
/// declarative [`EnumConfig`].
fn parse_enum_args(args: &[String]) -> Result<EnumCli> {
    let is_flag = |t: &str| t == "-i" || t == "-o" || t == "-e" || t.starts_with("--");
    let mut cfg = EnumConfig::default();
    let mut config_path = None;
    let mut print_config = false;

    let mut i = 0;
    let next = |i: &mut usize, what: &str| -> Result<String> {
        *i += 1;
        args.get(*i).cloned().with_context(|| format!("{what} requires a value"))
    };
    let consume_multi = |i: &mut usize| -> Vec<String> {
        let mut out = Vec::new();
        while *i + 1 < args.len() && !is_flag(&args[*i + 1]) {
            *i += 1;
            out.push(args[*i].clone());
        }
        out
    };

    while i < args.len() {
        let a = args[i].clone();
        match a.as_str() {
            "-i" | "--input" => cfg.input.extend(consume_multi(&mut i)),
            "-o" | "--output" => cfg.output = Some(next(&mut i, "--output")?),
            "--by" => cfg.by = Some(next(&mut i, "--by")?.to_lowercase()),
            "--databases" => cfg.databases.extend(consume_multi(&mut i)),
            "--contigs" => cfg.contigs.extend(consume_multi(&mut i)),
            "--contigs_file" => cfg.contigs_file = Some(next(&mut i, "--contigs_file")?),
            "--column_names" => cfg.column_names = Some(consume_multi(&mut i)),
            "--ncbi_taxonomy_path" => cfg.ncbi_taxonomy_path = Some(next(&mut i, "--ncbi_taxonomy_path")?),
            "--json" => cfg.json = true,
            "--config" => config_path = Some(next(&mut i, "--config")?),
            "--print_config" => print_config = true,
            "--taxonomy_update" => {}
            "--html" | "--html_max_height" => {
                anyhow::bail!("enum_report HTML output is not supported in this port; use TSV or --json")
            }
            "--taxid" => cfg.columns.push(ColumnSpec::Taxid { ranks: consume_multi(&mut i) }),
            "--taxname" => cfg.columns.push(ColumnSpec::Taxname { ranks: consume_multi(&mut i) }),
            "--qualifier" => {
                let feature = next(&mut i, "--qualifier")?;
                let qualifier = next(&mut i, "--qualifier")?;
                cfg.columns.push(ColumnSpec::Qualifier { feature, qualifier });
            }
            "--feature_count" => {
                let feature = next(&mut i, "--feature_count")?;
                cfg.columns.push(ColumnSpec::FeatureCount { feature });
            }
            "--motif_count" => {
                let pattern = next(&mut i, "--motif_count")?;
                cfg.columns.push(ColumnSpec::MotifCount { pattern });
            }
            "--net_charge" => {
                let ph = next(&mut i, "--net_charge")?;
                cfg.columns.push(ColumnSpec::NetCharge { ph });
            }
            "--repeat_count" => {
                let min_length = next(&mut i, "--repeat_count")?;
                cfg.columns.push(ColumnSpec::RepeatCount { min_length });
            }
            "--named_domain_count" => {
                let name = next(&mut i, "--named_domain_count")?;
                cfg.columns.push(ColumnSpec::NamedDomainCount { name });
            }
            "--append" => {
                let name = next(&mut i, "--append")?;
                let value_type = next(&mut i, "--append")?;
                let value = next(&mut i, "--append")?;
                cfg.columns.push(ColumnSpec::Append { name, value_type, value });
            }
            other if other.starts_with("--") => {
                let name = &other[2..];
                match ColumnSpec::from_static_flag(name) {
                    Some(spec) => cfg.columns.push(spec),
                    None => anyhow::bail!("unknown enum_report option: {other}"),
                }
            }
            other => anyhow::bail!("unexpected enum_report argument: {other}"),
        }
        i += 1;
    }
    Ok(EnumCli { config: cfg, config_path, print_config })
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Inspect { input, verbose } => {
            let records = load(&input)?;
            println!("{} records", records.len());
            if verbose {
                for r in &records {
                    println!(
                        "  id={} name={} mol={:?} len={} features={}",
                        r.id, r.name, r.molecule_type, r.len(), r.features.len()
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
        Command::Domainate { input, references, output, evalue, z, max_overlap, config: config_path, print_config: do_print } => {
            let cli = config::DomainateConfig {
                input: opt_path(&input)?,
                references: paths_to_strings(&references)?,
                output: opt_path(&output)?,
                evalue,
                z,
                max_overlap,
            };
            let mut cfg = match &config_path {
                Some(p) => load_config::<config::DomainateConfig>(p)?,
                None => config::DomainateConfig::default(),
            };
            cfg.merge_cli(cli);
            if do_print {
                print!("{}", print_config(&cfg)?);
                return Ok(());
            }
            let input = cfg.input.context("missing required `input` (-i / --input or config)")?;
            let output = cfg.output.context("missing required `output` (-o / --output or config)")?;
            let records = load(&PathBuf::from(&input))?;
            let dbs = domainate::read_hmm_references(&cfg.references).context("reading HMM references")?;
            let opts = domainate::DomainateOptions {
                z: cfg.z.unwrap_or(1000.0),
                evalue: cfg.evalue.unwrap_or(0.001),
                max_overlap: cfg.max_overlap.unwrap_or(1.0),
                ..Default::default()
            };
            let annotated = domainate::domainate(records, &dbs, &opts);
            write_genbank(&output, &annotated).context("writing GenBank")?;
            println!("wrote {} records to {}", annotated.len(), output);
        }
        Command::DomainSearch { input, references, output, evalue, z, add_annotations, max_hits, batch_size, cpu, whole_contig, cds_range, cds_up, cds_down, kb_range, kb_up, kb_down, strand, keep_direction, max_region_overlap, translate, config: config_path, print_config: do_print } => {
            let cli = config::DomainSearchConfig {
                input: opt_path(&input)?,
                references: paths_to_strings(&references)?,
                output: opt_path(&output)?,
                evalue, z, add_annotations, max_hits, batch_size, cpu,
                whole_contig, cds_range, cds_up, cds_down, kb_range, kb_up, kb_down,
                strand, keep_direction, translate, max_region_overlap,
            };
            let mut cfg = match &config_path {
                Some(p) => load_config::<config::DomainSearchConfig>(p)?,
                None => config::DomainSearchConfig::default(),
            };
            cfg.merge_cli(cli);
            if do_print {
                print!("{}", print_config(&cfg)?);
                return Ok(());
            }
            let input = cfg.input.context("missing required `input` (-i / --input or config)")?;
            let output = cfg.output.context("missing required `output` (-o / --output or config)")?;
            let dbs = domainate::read_hmm_references(&cfg.references).context("reading HMM references")?;
            let range = domainator_core::tools::neighborhood::validate_range(
                cfg.whole_contig, cfg.kb_range, cfg.kb_up, cfg.kb_down, cfg.cds_range, cfg.cds_up, cfg.cds_down,
            )
            .map_err(|e| anyhow::anyhow!(e))?;
            if cfg.translate && !matches!(range, domainator_core::tools::neighborhood::Range::None) {
                anyhow::bail!("--translate is incompatible with neighborhood extraction (--cds_range/--kb_range/--whole_contig)");
            }
            let strand = match cfg.strand.as_deref() {
                Some("f") => Some(1),
                Some("r") => Some(-1),
                Some(other) => anyhow::bail!("strand must be 'f' or 'r', got {other:?}"),
                None => None,
            };
            let opts = domain_search::DomainSearchOptions {
                z: cfg.z.unwrap_or(1000.0),
                evalue: cfg.evalue.unwrap_or(0.001),
                add_annotations: cfg.add_annotations,
                max_hits: cfg.max_hits,
                batch_size: cfg.batch_size.unwrap_or(10000),
                cpu: cfg.cpu.unwrap_or(0),
                range,
                normalize_direction: !cfg.keep_direction,
                strand,
                max_region_overlap: cfg.max_region_overlap.unwrap_or(1.0),
                translate: cfg.translate,
                ..Default::default()
            };
            let hits = domain_search::domain_search(&[input], &dbs, &opts)
                .context("running domain_search")?;
            write_genbank(&output, &hits).context("writing GenBank")?;
            println!("wrote {} hit contigs to {}", hits.len(), output);
        }
        Command::SeqDist { input, reference, dense, dense_text, sparse, algorithm, mode, lb, k, mst_knn, cpu, config: config_path, print_config: do_print } => {
            let cli = config::SeqDistConfig {
                input: opt_path(&input)?,
                reference: opt_path(&reference)?,
                dense: opt_path(&dense)?,
                dense_text: opt_path(&dense_text)?,
                sparse: opt_path(&sparse)?,
                algorithm,
                mode,
                lb,
                k,
                mst_knn,
                cpu,
            };
            let mut cfg = match &config_path {
                Some(p) => load_config::<config::SeqDistConfig>(p)?,
                None => config::SeqDistConfig::default(),
            };
            cfg.merge_cli(cli);
            if do_print {
                print!("{}", print_config(&cfg)?);
                return Ok(());
            }
            if cfg.dense.is_none() && cfg.dense_text.is_none() && cfg.sparse.is_none() {
                anyhow::bail!("no output specified: pass at least one of --dense, --dense_text, --sparse");
            }
            let input = cfg.input.context("missing required `input` (-i / --input or config)")?;
            let input_pb = PathBuf::from(&input);
            let input_kind = detect_kind(&input_pb)
                .with_context(|| format!("could not detect input kind: {input}"))?;
            let ref_pb = cfg.reference.as_ref().map(PathBuf::from);
            let ref_kind = match &ref_pb {
                Some(p) => Some(
                    detect_kind(p).with_context(|| format!("could not detect reference kind: {}", p.display()))?,
                ),
                None => None,
            };
            let opts = SeqDistOptions {
                algorithm: cfg.algorithm.unwrap_or_else(|| "diamond_us".into()),
                mode: cfg.mode.unwrap_or_else(|| "score".into()),
                lb: cfg.lb.unwrap_or(0.0),
                k: cfg.k,
                mst_knn: cfg.mst_knn,
                cpu: cfg.cpu.unwrap_or(0),
            };
            let matrix = seq_dist(&input, input_kind, cfg.reference.as_deref(), ref_kind, &opts)
                .context("running seq_dist")?;
            if let Some(p) = &cfg.dense {
                hdf5_io::write_dense(p, &matrix)?;
            }
            if let Some(p) = &cfg.dense_text {
                text::write_dense_text(p, &matrix)?;
            }
            if let Some(p) = &cfg.sparse {
                hdf5_io::write_sparse(p, &matrix)?;
            }
            println!("wrote {}x{} matrix", matrix.nrows(), matrix.ncols());
        }
        Command::CompareContigs { input, dense, dense_text, sparse, ji, ai, databases, k, lb, mst_knn, cpu, config: config_path, print_config: do_print } => {
            let cli = config::CompareContigsConfig {
                input: paths_to_strings(&input)?,
                dense: opt_path(&dense)?,
                dense_text: opt_path(&dense_text)?,
                sparse: opt_path(&sparse)?,
                ji,
                ai,
                databases,
                k,
                lb,
                mst_knn,
                cpu,
            };
            let mut cfg = match &config_path {
                Some(p) => load_config::<config::CompareContigsConfig>(p)?,
                None => config::CompareContigsConfig::default(),
            };
            cfg.merge_cli(cli);
            if do_print {
                print!("{}", print_config(&cfg)?);
                return Ok(());
            }
            if cfg.dense.is_none() && cfg.dense_text.is_none() && cfg.sparse.is_none() {
                anyhow::bail!("no output specified: pass at least one of --dense, --dense_text, --sparse");
            }
            let ji = cfg.ji.unwrap_or(0.5);
            let ai = cfg.ai.unwrap_or(0.5);
            if ji == 0.0 && ai == 0.0 {
                anyhow::bail!("specify a non-zero weight for at least one metric (--ji / --ai)");
            }
            let opts = CompareContigsOptions {
                ji_weight: ji,
                ai_weight: ai,
                databases: if cfg.databases.is_empty() { None } else { Some(cfg.databases.into_iter().collect()) },
                k: cfg.k,
                lb: cfg.lb.unwrap_or(0.0),
                mst_knn: cfg.mst_knn,
                cpu: cfg.cpu.unwrap_or(0),
            };
            let matrix = compare_contigs(&cfg.input, &opts).context("running compare_contigs")?;
            if let Some(p) = &cfg.dense {
                hdf5_io::write_dense(p, &matrix)?;
            }
            if let Some(p) = &cfg.dense_text {
                text::write_dense_text(p, &matrix)?;
            }
            if let Some(p) = &cfg.sparse {
                hdf5_io::write_sparse(p, &matrix)?;
            }
            println!("wrote {}x{} matrix", matrix.nrows(), matrix.ncols());
        }
        Command::Schema { tool } => {
            println!("{}", schema(&tool)?);
        }
        Command::EnumReport { args } => {
            let EnumCli { config: cli_cfg, config_path, print_config: do_print } = parse_enum_args(&args)?;
            // Config file (if any) provides the base; CLI values override/extend.
            let mut cfg = match &config_path {
                Some(p) => load_config::<EnumConfig>(p)?,
                None => EnumConfig::default(),
            };
            cfg.merge_cli(cli_cfg);
            if do_print {
                print!("{}", print_config(&cfg)?);
                return Ok(());
            }
            let by = cfg.by_enum()?;
            let analyses: Vec<Analysis> =
                cfg.columns.iter().map(|c| c.to_analysis()).collect::<Result<_>>()?;
            let format = if cfg.json { OutputFormat::Json } else { OutputFormat::Tsv };
            let ncbi_path = cfg.ncbi_taxonomy_path.clone().unwrap_or_else(|| "/tmp/ncbi_taxonomy".into());

            let target_contigs = parse_name_list(&cfg.contigs, cfg.contigs_file.as_deref())?;
            let mut records: Vec<(SeqRecord, String)> = Vec::new();
            if cfg.input.is_empty() {
                for r in load_inputs(&[])? {
                    records.push((r, "<stdin>".to_string()));
                }
            } else {
                for path in &cfg.input {
                    for r in load(&PathBuf::from(path))? {
                        records.push((r, path.clone()));
                    }
                }
            }
            if let Some(set) = &target_contigs {
                records.retain(|(r, _)| set.contains(&r.name));
            }
            let tax = if needs_taxonomy(&analyses) { Some(load_taxonomy(&ncbi_path)?) } else { None };
            let databases: Option<std::collections::HashSet<String>> =
                if cfg.databases.is_empty() { None } else { Some(cfg.databases.iter().cloned().collect()) };
            let col_names = cfg.column_names.as_deref();
            let result = match &cfg.output {
                Some(p) => {
                    let mut f = std::io::BufWriter::new(std::fs::File::create(p)?);
                    let r = enum_report::enum_report(
                        records, by, &analyses, col_names, tax.as_ref(), databases.as_ref(), format, &mut f,
                    );
                    f.flush()?;
                    r
                }
                None => {
                    let stdout = std::io::stdout();
                    let mut h = stdout.lock();
                    enum_report::enum_report(
                        records, by, &analyses, col_names, tax.as_ref(), databases.as_ref(), format, &mut h,
                    )
                }
            };
            result.map_err(|e| anyhow::anyhow!(e))?;
        }
        Command::SelectByCds {
            input, output, kb_range, kb_up, kb_down, cds_range, cds_up, cds_down, whole_contig,
            contigs, contigs_file, cds, cds_file, domains, domains_file, domain_expr, databases,
            strand, unannotated, search_hits, pad, keep_direction, skip_deduplicate,
            max_region_overlap, evalue, invert, include_nucleic_acids,
            config: config_path, print_config: do_print,
        } => {
            let cli = config::SelectByCdsConfig {
                input: paths_to_strings(&input)?,
                output: opt_path(&output)?,
                kb_range,
                kb_up,
                kb_down,
                cds_range,
                cds_up,
                cds_down,
                whole_contig,
                contigs,
                contigs_file: opt_path(&contigs_file)?,
                cds,
                cds_file: opt_path(&cds_file)?,
                domains,
                domains_file: opt_path(&domains_file)?,
                domain_expr,
                databases,
                strand,
                unannotated,
                search_hits,
                pad,
                keep_direction,
                skip_deduplicate,
                max_region_overlap,
                evalue,
                invert,
                include_nucleic_acids,
            };
            let mut cfg = match &config_path {
                Some(p) => load_config::<config::SelectByCdsConfig>(p)?,
                None => config::SelectByCdsConfig::default(),
            };
            cfg.merge_cli(cli);
            if do_print {
                print!("{}", print_config(&cfg)?);
                return Ok(());
            }
            let mut records = load_inputs_str(&cfg.input)?;
            let target_contigs = parse_name_list(&cfg.contigs, cfg.contigs_file.as_deref())?;
            if let Some(set) = &target_contigs {
                records.retain(|r| set.contains(&r.name));
            }
            let target_cdss = parse_name_list(&cfg.cds, cfg.cds_file.as_deref())?;
            let target_domains = parse_name_list(&cfg.domains, cfg.domains_file.as_deref())?;
            let range = domainator_core::tools::neighborhood::validate_range(
                cfg.whole_contig, cfg.kb_range, cfg.kb_up, cfg.kb_down, cfg.cds_range, cfg.cds_up, cfg.cds_down,
            )
            .map_err(|e| anyhow::anyhow!(e))?;
            let strand = match cfg.strand.as_deref() {
                Some("f") => Some(1),
                Some("r") => Some(-1),
                Some(other) => anyhow::bail!("strand must be 'f' or 'r', got {other:?}"),
                None => None,
            };
            let opts = SelectByCdsOptions {
                target_cdss,
                target_domains,
                domain_expr: cfg.domain_expr,
                range,
                normalize_direction: !cfg.keep_direction,
                evalue: cfg.evalue.unwrap_or(100000000.0),
                invert: cfg.invert,
                search_hits: cfg.search_hits,
                max_region_overlap: cfg.max_region_overlap.unwrap_or(1.0),
                strand,
                databases: if cfg.databases.is_empty() { None } else { Some(cfg.databases.into_iter().collect()) },
                unannotated: cfg.unannotated,
                include_nucleic_acids: cfg.include_nucleic_acids,
            };
            let mut regions = select_by_cds(&records, &opts);
            // Deduplicate by record name (LOCUS), keeping the first occurrence.
            if !cfg.skip_deduplicate {
                let mut seen = std::collections::HashSet::new();
                regions.retain(|r| seen.insert(r.record.name.clone()));
            }
            if cfg.pad {
                pad_records(&mut regions);
            }
            let out_records: Vec<SeqRecord> = regions.into_iter().map(|r| r.record).collect();
            write_genbank_out(&out_records, cfg.output.map(PathBuf::from).as_ref())?;
        }
        Command::SelectByContig {
            input, output, contigs, contigs_file, contigs_regex, definition_regex, sequence_regex,
            domains, domains_file, domain_expr, domain_type, databases, unannotated, evalue,
            fasta_out, include_taxids, exclude_taxids, taxonomy_expr, ncbi_taxonomy_path, first,
            length_lb, length_ub, cds_count_lb, cds_count_ub, search_score_lb, search_score_ub, invert,
            config: config_path, print_config: do_print,
        } => {
            let cli = config::SelectByContigConfig {
                input: paths_to_strings(&input)?,
                output: opt_path(&output)?,
                contigs,
                contigs_file: opt_path(&contigs_file)?,
                contigs_regex,
                definition_regex,
                sequence_regex,
                domains,
                domains_file: opt_path(&domains_file)?,
                domain_expr,
                domain_type,
                databases,
                unannotated,
                evalue,
                fasta_out,
                include_taxids,
                exclude_taxids,
                taxonomy_expr,
                ncbi_taxonomy_path,
                first,
                length_lb,
                length_ub,
                cds_count_lb,
                cds_count_ub,
                search_score_lb,
                search_score_ub,
                invert,
            };
            let mut cfg = match &config_path {
                Some(p) => load_config::<config::SelectByContigConfig>(p)?,
                None => config::SelectByContigConfig::default(),
            };
            cfg.merge_cli(cli);
            if do_print {
                print!("{}", print_config(&cfg)?);
                return Ok(());
            }
            let records = load_inputs_str(&cfg.input)?;
            let target_contigs = parse_name_list(&cfg.contigs, cfg.contigs_file.as_deref())?;
            let target_domains = parse_name_list(&cfg.domains, cfg.domains_file.as_deref())?;
            let needs_tax = !cfg.include_taxids.is_empty()
                || !cfg.exclude_taxids.is_empty()
                || cfg.taxonomy_expr.as_deref().is_some_and(|e| !e.trim().is_empty());
            let tax = if needs_tax {
                Some(load_taxonomy(cfg.ncbi_taxonomy_path.as_deref().unwrap_or("/tmp/ncbi_taxonomy"))?)
            } else {
                None
            };
            let opts = SelectByContigOptions {
                target_domains,
                domain_expr: cfg.domain_expr,
                domain_evalue: cfg.evalue.unwrap_or(100000000.0),
                first: cfg.first,
                length_lb: cfg.length_lb,
                length_ub: cfg.length_ub,
                cds_count_lb: cfg.cds_count_lb,
                cds_count_ub: cfg.cds_count_ub,
                definition_regex: cfg.definition_regex,
                sequence_regex: cfg.sequence_regex,
                target_contigs,
                contigs_regex: cfg.contigs_regex,
                invert: cfg.invert,
                include_taxids: if cfg.include_taxids.is_empty() { None } else { Some(cfg.include_taxids.into_iter().collect()) },
                exclude_taxids: if cfg.exclude_taxids.is_empty() { None } else { Some(cfg.exclude_taxids.into_iter().collect()) },
                taxonomy_expr: cfg.taxonomy_expr,
                databases: if cfg.databases.is_empty() { None } else { Some(cfg.databases.into_iter().collect()) },
                unannotated: cfg.unannotated,
                search_score_lb: cfg.search_score_lb,
                search_score_ub: cfg.search_score_ub,
                domain_type: cfg.domain_type.unwrap_or_else(|| "domain".into()),
            };
            let out_path = cfg.output.map(PathBuf::from);
            let selected = select_by_contig(records, tax.as_ref(), &opts).map_err(|e| anyhow::anyhow!(e))?;
            if cfg.fasta_out {
                write_fasta_out(&selected, out_path.as_ref())?;
            } else {
                write_genbank_out(&selected, out_path.as_ref())?;
            }
        }
        Command::HmmerCompare { input, reference, output, score_cutoff, k, alignments, cpu, config: config_path, print_config: do_print } => {
            let cli = config::HmmerCompareConfig {
                input: paths_to_strings(&input)?,
                reference: paths_to_strings(&reference)?,
                output: opt_path(&output)?,
                score_cutoff,
                k,
                alignments,
                cpu,
            };
            let mut cfg = match &config_path {
                Some(p) => load_config::<config::HmmerCompareConfig>(p)?,
                None => config::HmmerCompareConfig::default(),
            };
            cfg.merge_cli(cli);
            if do_print {
                print!("{}", print_config(&cfg)?);
                return Ok(());
            }
            let opts = HmmerCompareOptions {
                score_cutoff: cfg.score_cutoff.unwrap_or(0.0),
                alignments: cfg.alignments,
                k: cfg.k,
                cpu: cfg.cpu.unwrap_or(0),
            };
            match &cfg.output {
                Some(p) => {
                    let mut f = std::io::BufWriter::new(std::fs::File::create(p)?);
                    hmmer_compare(&cfg.input, &cfg.reference, &opts, &mut f).context("running hmmer_compare")?;
                    f.flush()?;
                }
                None => {
                    let stdout = std::io::stdout();
                    let mut h = stdout.lock();
                    hmmer_compare(&cfg.input, &cfg.reference, &opts, &mut h).context("running hmmer_compare")?;
                }
            }
        }
    }
    Ok(())
}
