//! `domainator <tool> [opts]` — the CLI for the Domainator Rust rewrite, as a
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
use clap::{Parser, Subcommand};

use config::{load_config, print_config, schema, ColumnSpec, EnumConfig, HmmerReportColumn};
use domainator_config as config;

use domainator_core::io::genbank::writer::write_record;
use domainator_core::io::genbank::{read_genbank, write_genbank};
use domainator_core::io::{detect_filetype, fasta, FileType};
use domainator_core::matrix::{hdf5_io, read_matrix, text};
use domainator_core::search::hmm_write::write_hmm_output;
use domainator_core::seq::taxonomy::{default_ncbi_taxonomy_path, NCBITaxonomy};
use domainator_core::seq::SeqRecord;
use domainator_core::tools;
use domainator_core::tools::build_ssn::build_ssn;
use domainator_core::tools::build_ssn_viewer::{
    build_ssn_viewer_bundle, gzip, render_html, serialize_bundle,
};
use domainator_core::tools::clean_sequences;
use domainator_core::tools::compare_contigs::{compare_contigs, CompareContigsOptions};
use domainator_core::tools::deduplicate_genbank;
use domainator_core::tools::enum_report::{self, needs_taxonomy, Analysis, OutputFormat};
use domainator_core::tools::hmmer_compare::{hmmer_compare, HmmerCompareOptions};
use domainator_core::tools::matrix_report::{matrix_report, render_text, to_json};
use domainator_core::tools::select_by_cds::{pad_records, select_by_cds, SelectByCdsOptions};
use domainator_core::tools::select_by_contig::{select_by_contig, SelectByContigOptions};
use domainator_core::tools::select_common::parse_name_list;
use domainator_core::tools::seq_dist::{detect_kind, seq_dist, SeqDistOptions};
use domainator_core::tools::sort_contigs;
use domainator_core::tools::ssn_bundle;
use domainator_core::tools::ssn_navigator::ssn_navigator;
use domainator_core::tools::transform_matrix::transform_matrix;
use domainator_core::tools::{domain_search, domainate};

#[derive(Parser)]
#[command(name = "domainator", version, long_version = domainator_core::version::LONG_VERSION, about = "Domainator (Rust): protein-domain / SSN analysis")]
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
    Convert { input: PathBuf, output: PathBuf },
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
        /// Drop hits with i-evalue below this (default: 0.0).
        #[arg(long = "min_evalue")]
        min_evalue: Option<f64>,
        /// E-value search space size (-Z) (default: 1000).
        #[arg(short = 'Z', long = "z")]
        z: Option<f64>,
        /// Maximum fractional domain overlap before filtering (>=1 disables; default 1.0).
        #[arg(long = "max_overlap")]
        max_overlap: Option<f64>,
        /// Per-CDS domain cap (0 = unlimited).
        #[arg(long = "max_domains")]
        max_domains: Option<usize>,
        /// hmmsearch --max (skip heuristic filters for maximum sensitivity).
        #[arg(long = "max_mode")]
        max_mode: bool,
        /// Apply the overlap filter independently per database.
        #[arg(long = "overlap_by_db")]
        overlap_by_db: bool,
        /// Output only contigs that had at least one hit.
        #[arg(long = "hits_only")]
        hits_only: bool,
        /// Do not add Domainator annotations (keep only inputs).
        #[arg(long = "no_annotations")]
        no_annotations: bool,
        /// Worker threads (0 = all cores).
        #[arg(long)]
        cpu: Option<usize>,
        /// Treat FASTA input as this molecule type (protein/nucleotide).
        #[arg(long = "fasta_type")]
        fasta_type: Option<String>,
        /// Only keep contigs with a taxid in this list (lineage-aware).
        #[arg(long = "include_taxids", num_args = 1..)]
        include_taxids: Vec<i64>,
        /// Drop contigs with a taxid in this list (lineage-aware).
        #[arg(long = "exclude_taxids", num_args = 1..)]
        exclude_taxids: Vec<i64>,
        /// Boolean taxid expression (& | ~), e.g. "2 & ~1224".
        #[arg(long = "taxonomy_expr")]
        taxonomy_expr: Option<String>,
        /// Path to the NCBI taxonomy directory (default: /tmp/ncbi_taxonomy_$USER).
        #[arg(long = "ncbi_taxonomy_path")]
        ncbi_taxonomy_path: Option<String>,
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
        /// Drop hits with i-evalue below this (default: 0.0).
        #[arg(long = "min_evalue")]
        min_evalue: Option<f64>,
        /// E-value search space size (-Z) (default: 1000).
        #[arg(short = 'Z', long = "z")]
        z: Option<f64>,
        /// Add full Domainator annotations, not just the best-hit feature.
        #[arg(long = "add_annotations")]
        add_annotations: bool,
        /// Cap on total hit contigs returned (top-N by best domain score).
        #[arg(long = "max_hits")]
        max_hits: Option<usize>,
        /// Cap on independent hits emitted per contig.
        #[arg(long = "max_hits_per_contig")]
        max_hits_per_contig: Option<usize>,
        /// Maximum fractional domain overlap on a CDS before filtering (>=1 disables).
        #[arg(long = "max_overlap")]
        max_overlap: Option<f64>,
        /// hmmsearch --max (skip heuristic filters for maximum sensitivity).
        #[arg(long = "max_mode")]
        max_mode: bool,
        /// Approximate CDSs per partition (default: 10000).
        #[arg(long = "batch_size")]
        batch_size: Option<u64>,
        /// Worker threads (0 = all cores).
        #[arg(long)]
        cpu: Option<usize>,
        /// Treat FASTA input as this molecule type (protein/nucleotide).
        #[arg(long = "fasta_type")]
        fasta_type: Option<String>,
        /// Decoy domain names: drop a hit if its best hit is a decoy.
        #[arg(long, num_args = 1..)]
        decoys: Vec<String>,
        /// File of decoy domain names (one per line).
        #[arg(long = "decoys_file")]
        decoys_file: Option<String>,
        /// Only keep contigs with a taxid in this list (lineage-aware).
        #[arg(long = "include_taxids", num_args = 1..)]
        include_taxids: Vec<i64>,
        /// Drop contigs with a taxid in this list (lineage-aware).
        #[arg(long = "exclude_taxids", num_args = 1..)]
        exclude_taxids: Vec<i64>,
        /// Boolean taxid expression (& | ~), e.g. "2 & ~1224".
        #[arg(long = "taxonomy_expr")]
        taxonomy_expr: Option<String>,
        /// Path to the NCBI taxonomy directory (default: /tmp/ncbi_taxonomy_$USER).
        #[arg(long = "ncbi_taxonomy_path")]
        ncbi_taxonomy_path: Option<String>,
        /// Fail if projected uncompressed GenBank output exceeds this many GB (0 disables; default 25).
        #[arg(long = "max_output_gb")]
        max_output_gb: Option<f64>,
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
        /// Pad the ends of extracted regions so the hit CDSs line up.
        #[arg(long)]
        pad: bool,
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
    /// Remove redundant sequences (hash exact-dedup, or cd-hit/diamond/usearch clustering).
    Deduplicate {
        /// Input GenBank/FASTA file(s).
        #[arg(short, long, num_args = 1..)]
        input: Vec<PathBuf>,
        /// Output file. Writes to stdout if omitted.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Log file (default: stderr).
        #[arg(long)]
        log: Option<String>,
        /// Molecule type for FASTA input (protein/nucleotide).
        #[arg(long = "fasta_type")]
        fasta_type: Option<String>,
        /// Clustering algorithm: cd-hit | hash | diamond | usearch [default: cd-hit].
        #[arg(long)]
        algorithm: Option<String>,
        /// Path to the external clusterer binary (else $DOMAINATOR_CDHIT / $DOMAINATOR_DIAMOND /
        /// $DOMAINATOR_USEARCH, then PATH). For cd-hit this is the `cdhit` multiplexer, or a
        /// `cd-hit`/`cd-hit-est` executable, which is run directly.
        #[arg(long = "bin_path")]
        bin_path: Option<String>,
        /// Identity threshold in [0,1] (must be 1 for hash).
        #[arg(long)]
        id: Option<f64>,
        /// Prepend "<cluster_size>-" to representative names.
        #[arg(long = "prefix_count")]
        prefix_count: bool,
        /// Append "-<cluster_size>" to representative names.
        #[arg(long = "suffix_count")]
        suffix_count: bool,
        /// Threads (0 = all cores).
        #[arg(long)]
        cpu: Option<usize>,
        /// Extra clusterer args as an inner JSON dict, e.g. '"-s":0.9'.
        #[arg(long)]
        params: Option<String>,
        /// Write output as FASTA instead of GenBank.
        #[arg(long = "fasta_out")]
        fasta_out: bool,
        /// Consider both strands (nucleotide only).
        #[arg(long = "both_strands")]
        both_strands: bool,
        /// Write a representative/contigs cluster-membership TSV to this path.
        #[arg(long = "cluster_table")]
        cluster_table: Option<String>,
        /// Separator between contig names in the cluster table (default ' ; ').
        #[arg(long = "cluster_sep")]
        cluster_sep: Option<String>,
        /// Read parameters from a JSON/YAML config file (CLI flags override it).
        #[arg(long)]
        config: Option<String>,
        /// Print the resolved parameters as YAML and exit.
        #[arg(long = "print_config")]
        print_config: bool,
    },
    /// Convert GenBank record(s) to FASTA.
    GenbankToFasta {
        /// Input GenBank file(s) (stdin if omitted).
        #[arg(short, long, num_args = 1..)]
        input: Vec<PathBuf>,
        /// Output FASTA file (stdout if omitted).
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long)]
        config: Option<String>,
        #[arg(long = "print_config")]
        print_config: bool,
    },
    /// Clean/filter sequences: newick-safe names, dedup names, strip/filter residues.
    CleanSequences {
        /// Input GenBank/FASTA file(s) (stdin if omitted).
        #[arg(short, long, num_args = 1..)]
        input: Vec<PathBuf>,
        /// Output file (stdout if omitted).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Log file (default: stderr).
        #[arg(long)]
        log: Option<String>,
        /// Molecule type for FASTA input (protein/nucleotide).
        #[arg(long = "fasta_type")]
        fasta_type: Option<String>,
        /// Write output as FASTA instead of GenBank.
        #[arg(long = "fasta_out")]
        fasta_out: bool,
        /// Make names newick-compatible (bad chars -> '_').
        #[arg(long = "clean_name")]
        clean_name: bool,
        /// Ensure unique sequence names.
        #[arg(long = "deduplicate_names")]
        deduplicate_names: bool,
        /// Strip non-canonical residues (and '*') from sequence ends.
        #[arg(long)]
        strip: bool,
        /// Drop sequences containing non-canonical residues (after strip).
        #[arg(long = "filter_by_aa")]
        filter_by_aa: bool,
        #[arg(long)]
        config: Option<String>,
        #[arg(long = "print_config")]
        print_config: bool,
    },
    /// Sort contigs by best Domain_Search (or selected Domainator) score.
    SortContigs {
        /// Input GenBank file(s) (stdin if omitted).
        #[arg(short, long, num_args = 1..)]
        input: Vec<PathBuf>,
        /// Output GenBank file (stdout if omitted).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Keep only the top-N contigs by score.
        #[arg(long = "max_hits")]
        max_hits: Option<usize>,
        /// Sort by best Domainator score among these domain names.
        #[arg(long, num_args = 1..)]
        domains: Vec<String>,
        /// With --domains, restrict to these databases.
        #[arg(long, num_args = 1..)]
        databases: Vec<String>,
        #[arg(long)]
        config: Option<String>,
        #[arg(long = "print_config")]
        print_config: bool,
    },
    /// Extract contig regions using explicit 1-based coordinate ranges from a table.
    SelectByCoord {
        /// Input GenBank file(s) (stdin if omitted).
        #[arg(short, long, num_args = 1..)]
        input: Vec<PathBuf>,
        /// Output GenBank file (stdout if omitted).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Labeled coordinate table (columns: contig, start, end, strand?, file?).
        #[arg(short = 'c', long)]
        coords: Option<PathBuf>,
        #[arg(long)]
        config: Option<String>,
        #[arg(long = "print_config")]
        print_config: bool,
    },
    /// Filter Domainator/Domain_Search annotations by evalue, overlap, database, name.
    #[command(allow_negative_numbers = true)]
    FilterDomains {
        /// Input GenBank file(s) (stdin if omitted).
        #[arg(short, long, num_args = 1..)]
        input: Vec<PathBuf>,
        /// Output GenBank file (stdout if omitted).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Threshold E-value (max) for a domain hit.
        #[arg(short, long)]
        evalue: Option<f64>,
        /// Maximum fraction of overlap between kept domains.
        #[arg(short = 'm', long = "max_overlap")]
        max_overlap: Option<f64>,
        /// Only keep domains with these names.
        #[arg(long, num_args = 1..)]
        domains: Vec<String>,
        /// File of domain names to keep (one per line).
        #[arg(long = "domains_file")]
        domains_file: Option<String>,
        /// Remove annotations from all databases except these (applied first).
        #[arg(long = "databases_keep", num_args = 1..)]
        databases_keep: Vec<String>,
        /// Only apply filters to these databases (others kept as-is).
        #[arg(long = "databases_filter", num_args = 1..)]
        databases_filter: Vec<String>,
        /// Which annotations to filter: domainator | domain_search | all.
        #[arg(long = "annotation_type")]
        annotation_type: Option<String>,
        /// Only process contigs with these names.
        #[arg(long, num_args = 1..)]
        contigs: Vec<String>,
        /// File of contig names to process.
        #[arg(long = "contigs_file")]
        contigs_file: Option<String>,
        #[arg(long)]
        config: Option<String>,
        #[arg(long = "print_config")]
        print_config: bool,
    },
    /// Assign hex colors to domain and/or CDS features (Color qualifier).
    ColorGenbank {
        /// Input GenBank file(s) (stdin if omitted).
        #[arg(short, long, num_args = 1..)]
        input: Vec<PathBuf>,
        /// Output GenBank file (stdout if omitted).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Only color contigs with these names.
        #[arg(long, num_args = 1..)]
        contigs: Vec<String>,
        /// File of contig names to color (one per line).
        #[arg(long = "contigs_file")]
        contigs_file: Option<String>,
        /// TSV (no header): key, hex color. If omitted, colors are auto-generated.
        #[arg(long = "color_table")]
        color_table: Option<String>,
        /// Write the resolved color table (key, hex) to this file.
        #[arg(long = "color_table_out")]
        color_table_out: Option<String>,
        /// Color to use for Domain_Search best-hit features.
        #[arg(long = "search_hit_color")]
        search_hit_color: Option<String>,
        /// Remove all existing Color qualifiers before coloring.
        #[arg(long)]
        clear: bool,
        /// Only use domains from these databases when determining color.
        #[arg(long, num_args = 1..)]
        databases: Vec<String>,
        /// What to color: domains | cdss | domains_and_cdss (default: cdss).
        #[arg(long)]
        mode: Option<String>,
        /// Color domains by their database id instead of their name.
        #[arg(long = "domain_database")]
        domain_database: bool,
        #[arg(long)]
        config: Option<String>,
        #[arg(long = "print_config")]
        print_config: bool,
    },
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
    TrimContigs {
        /// Input GenBank file(s) (stdin if omitted).
        #[arg(short, long, num_args = 1..)]
        input: Vec<PathBuf>,
        /// Output GenBank file (stdout if omitted).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// kb to trim from both sides.
        #[arg(long = "kb_both")]
        kb_both: Option<f64>,
        /// kb to trim from the left side.
        #[arg(long = "kb_left")]
        kb_left: Option<f64>,
        /// kb to trim from the right side.
        #[arg(long = "kb_right")]
        kb_right: Option<f64>,
        /// CDSs to trim from both sides.
        #[arg(long = "cds_both")]
        cds_both: Option<i64>,
        /// CDSs to trim from the left side.
        #[arg(long = "cds_left")]
        cds_left: Option<i64>,
        /// CDSs to trim from the right side.
        #[arg(long = "cds_right")]
        cds_right: Option<i64>,
        /// Trim end CDSs containing domains with these names.
        #[arg(long, num_args = 1..)]
        domains: Vec<String>,
        /// File of domain names to trim.
        #[arg(long = "domains_file")]
        domains_file: Option<String>,
        /// Trim end CDSs matching this boolean domain expression.
        #[arg(long = "domain_expr")]
        domain_expr: Option<String>,
        /// Trim end CDSs with no domain annotations.
        #[arg(long = "no_domain")]
        no_domain: bool,
        /// Only process contigs with these names.
        #[arg(long, num_args = 1..)]
        contigs: Vec<String>,
        /// File of contig names to process.
        #[arg(long = "contigs_file")]
        contigs_file: Option<String>,
        #[arg(long)]
        config: Option<String>,
        #[arg(long = "print_config")]
        print_config: bool,
    },
    /// Extract domain-annotated regions from annotated contigs.
    #[command(allow_negative_numbers = true)]
    ExtractDomains {
        /// Input GenBank file(s) (stdin if omitted).
        #[arg(short, long, num_args = 1..)]
        input: Vec<PathBuf>,
        /// Output file (stdout if omitted).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// E-value cutoff (max) for domains to extract.
        #[arg(short, long)]
        evalue: Option<f64>,
        /// Score cutoff (min) for domains to extract.
        #[arg(long)]
        score: Option<f64>,
        /// Include domains with these names.
        #[arg(long, num_args = 1..)]
        domains: Vec<String>,
        /// File of domain names to include.
        #[arg(long = "domains_file")]
        domains_file: Option<String>,
        /// Only these databases.
        #[arg(long, num_args = 1..)]
        databases: Vec<String>,
        /// Select domains marked as search hits.
        #[arg(long = "search_hits")]
        search_hits: bool,
        /// Extract this many extra residues/bases upstream of the hit.
        #[arg(long = "pad_up")]
        pad_up: Option<i64>,
        /// Extract this many extra residues/bases downstream of the hit.
        #[arg(long = "pad_down")]
        pad_down: Option<i64>,
        /// Extract one envelope covering all target domains per contig.
        #[arg(long)]
        combine: bool,
        /// Splice target domains together, deleting intervening sequence.
        #[arg(long)]
        splice: bool,
        /// Don't reverse-complement reverse-strand hits.
        #[arg(long = "keep_direction")]
        keep_direction: bool,
        /// Keep original contig names (don't add coordinate suffixes).
        #[arg(long = "keep_name")]
        keep_name: bool,
        /// Write FASTA instead of GenBank.
        #[arg(long = "fasta_out")]
        fasta_out: bool,
        /// Only process contigs with these names.
        #[arg(long, num_args = 1..)]
        contigs: Vec<String>,
        /// File of contig names to process.
        #[arg(long = "contigs_file")]
        contigs_file: Option<String>,
        #[arg(long)]
        config: Option<String>,
        #[arg(long = "print_config")]
        print_config: bool,
    },
    /// Extract peptide sequences from selected CDSs.
    ExtractPeptides {
        /// Input GenBank file(s) (stdin if omitted).
        #[arg(short, long, num_args = 1..)]
        input: Vec<PathBuf>,
        /// Output file (stdout if omitted).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Only peptides containing these domains.
        #[arg(long, num_args = 1..)]
        domains: Vec<String>,
        /// File of domain names.
        #[arg(long = "domains_file")]
        domains_file: Option<String>,
        /// Only consider domains from these databases.
        #[arg(long, num_args = 1..)]
        databases: Vec<String>,
        /// Extract peptides from CDSs with no domain annotations.
        #[arg(long)]
        unannotated: bool,
        /// Extract peptides from search hits.
        #[arg(long = "search_hits")]
        search_hits: bool,
        /// Only CDSs on this strand: f | r.
        #[arg(long)]
        strand: Option<String>,
        /// Invert the CDS selection criteria.
        #[arg(long)]
        invert: bool,
        /// Only CDSs with these names.
        #[arg(long, num_args = 1..)]
        cdss: Vec<String>,
        /// File of CDS names.
        #[arg(long = "cdss_file")]
        cdss_file: Option<String>,
        /// Keep the original contig name instead of the CDS name.
        #[arg(long = "keep_name")]
        keep_name: bool,
        /// CDS qualifier to prefer as the peptide name.
        #[arg(long = "name_field")]
        name_field: Option<String>,
        /// E-value cutoff for domain annotations kept on the output.
        #[arg(short, long)]
        evalue: Option<f64>,
        /// Write FASTA instead of GenBank.
        #[arg(long = "fasta_out")]
        fasta_out: bool,
        /// Only process contigs with these names.
        #[arg(long, num_args = 1..)]
        contigs: Vec<String>,
        /// File of contig names to process.
        #[arg(long = "contigs_file")]
        contigs_file: Option<String>,
        #[arg(long)]
        config: Option<String>,
        #[arg(long = "print_config")]
        print_config: bool,
    },
    /// Extract regions of proteins not containing domain annotations.
    #[command(allow_negative_numbers = true)]
    ExtractUnannotated {
        /// Input GenBank file(s) (stdin if omitted).
        #[arg(short, long, num_args = 1..)]
        input: Vec<PathBuf>,
        /// Output file (stdout if omitted).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// E-value cutoff (max) for domains to consider.
        #[arg(short, long)]
        evalue: Option<f64>,
        /// Score cutoff (min) for domains to consider.
        #[arg(long)]
        score: Option<f64>,
        /// Only consider domains from these databases.
        #[arg(long, num_args = 1..)]
        databases: Vec<String>,
        /// Consider domains marked as search hits.
        #[arg(long = "search_hits")]
        search_hits: bool,
        /// Don't reverse-complement reverse-strand regions.
        #[arg(long = "keep_direction")]
        keep_direction: bool,
        /// Keep original names (no coordinate suffixes).
        #[arg(long = "keep_name")]
        keep_name: bool,
        /// Only extract the largest unannotated region per protein.
        #[arg(long)]
        largest: bool,
        /// Minimum length of unannotated regions to extract.
        #[arg(long)]
        lb: Option<i64>,
        /// Write FASTA instead of GenBank.
        #[arg(long = "fasta_out")]
        fasta_out: bool,
        /// Only process contigs with these names.
        #[arg(long, num_args = 1..)]
        contigs: Vec<String>,
        /// File of contig names to process.
        #[arg(long = "contigs_file")]
        contigs_file: Option<String>,
        #[arg(long)]
        config: Option<String>,
        #[arg(long = "print_config")]
        print_config: bool,
    },
    /// Compute a similarity matrix between input and reference sequences/profiles.
    SeqDist {
        /// Input query file (FASTA/GenBank/HMM; two HMM files select --algorithm hmmer_compare).
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
        /// Extra options forwarded to `diamond blastp`, as a JSON dict body without the
        /// outer braces, e.g. --params '"-c":4, "-b":0.4'. A null value is a bare flag.
        #[arg(long)]
        params: Option<String>,
        /// Prune to MST + OR-symmetric kNN edges (integer >= 0, 0 = spanning tree only;
        /// requires reference == input and --mode score/bool/efi_score). When set, -k
        /// is ignored.
        #[arg(long = "mst_knn", value_parser = parse_mst_knn)]
        mst_knn: Option<usize>,
        /// Keep OR-symmetric kNN edges without the spanning tree. Unlike --mst_knn this
        /// does not preserve connected components.
        #[arg(long = "knn", value_parser = parse_knn)]
        knn: Option<usize>,
        /// Worker threads (0 = all cores).
        #[arg(long)]
        cpu: Option<usize>,
        /// Fail if a projected matrix output exceeds this many GB (default 25; 0 disables).
        #[arg(long = "max_output_gb")]
        max_output_gb: Option<f64>,
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
        /// Prune to MST + OR-symmetric kNN edges (integer >= 0, 0 = spanning tree only).
        /// When set, -k is ignored.
        #[arg(long = "mst_knn", value_parser = parse_mst_knn)]
        mst_knn: Option<usize>,
        /// Keep OR-symmetric kNN edges without the spanning tree. Unlike --mst_knn this
        /// does not preserve connected components.
        #[arg(long = "knn", value_parser = parse_knn)]
        knn: Option<usize>,
        /// Worker threads (0 = all cores).
        #[arg(long)]
        cpu: Option<usize>,
        /// Fail if a projected matrix output exceeds this many GB (default 25; 0 disables).
        #[arg(long = "max_output_gb")]
        max_output_gb: Option<f64>,
        /// Read parameters from a JSON/YAML config file (CLI flags override it).
        #[arg(long)]
        config: Option<String>,
        /// Print the resolved parameters as YAML and exit.
        #[arg(long = "print_config")]
        print_config: bool,
    },
    /// Normalize / convert / sparsify a score matrix (dense<->sparse, --mode, --mst_knn).
    TransformMatrix {
        /// Input matrix file (HDF5 or dense TSV).
        #[arg(short, long)]
        input: Option<PathBuf>,
        /// Write a dense HDF5 matrix here.
        #[arg(long)]
        dense: Option<PathBuf>,
        /// Write a dense TSV matrix here.
        #[arg(long = "dense_text")]
        dense_text: Option<PathBuf>,
        /// Write a sparse (CSR) HDF5 matrix here.
        #[arg(long)]
        sparse: Option<PathBuf>,
        /// Output value type (default: none = pass-through). Transforms only apply
        /// to a `score` input: score, bool, norm_score, row_norm_score, score_dist,
        /// efi_score, efi_score_dist.
        #[arg(long)]
        mode: Option<String>,
        /// Override the input matrix's declared data type.
        #[arg(long = "input_type")]
        input_type: Option<String>,
        /// Zero out all values <= this after the mode transform.
        #[arg(long)]
        lb: Option<f64>,
        /// Keep only MST + OR-symmetric kNN edges (integer >= 0, 0 = spanning tree only).
        #[arg(long = "mst_knn", value_parser = parse_mst_knn)]
        mst_knn: Option<usize>,
        /// Keep OR-symmetric kNN edges without the spanning tree. Unlike --mst_knn this
        /// does not preserve connected components.
        #[arg(long = "knn", value_parser = parse_knn)]
        knn: Option<usize>,
        /// Fail if a projected matrix output exceeds this many GB (default 25; 0 disables).
        #[arg(long = "max_output_gb")]
        max_output_gb: Option<f64>,
        /// Read parameters from a JSON/YAML config file (CLI flags override it).
        #[arg(long)]
        config: Option<String>,
        /// Print the resolved parameters as YAML and exit.
        #[arg(long = "print_config")]
        print_config: bool,
    },
    /// Build a Cytoscape XGMML sequence-similarity network from a score matrix.
    BuildSsn {
        /// Input matrix file (HDF5 or dense TSV).
        #[arg(short, long)]
        input: Option<PathBuf>,
        /// Write the Cytoscape XGMML network here (required).
        #[arg(long)]
        xgmml: Option<PathBuf>,
        /// Exclude edges with weight <= this (default: 0).
        #[arg(long)]
        lb: Option<f64>,
        /// TSV node metadata files keyed by node id.
        #[arg(long, num_args = 1..)]
        metadata: Vec<String>,
        /// Only consider labels in this list (additive with --subset_file).
        #[arg(long, num_args = 1..)]
        subset: Vec<String>,
        /// Text file of labels to retain.
        #[arg(long = "subset_file")]
        subset_file: Option<String>,
        /// Color nodes by this metadata column.
        #[arg(long = "color_by")]
        color_by: Option<String>,
        /// TSV (no header) mapping metadata values to hex colors.
        #[arg(long = "color_table")]
        color_table: Option<String>,
        /// Write the resolved color table here.
        #[arg(long = "color_table_out")]
        color_table_out: Option<String>,
        /// Give distinct colors only to this many of the largest --color_by groups;
        /// every remaining group shares a neutral gray.
        #[arg(long = "max_color_groups")]
        max_color_groups: Option<usize>,
        /// Add an SSN_cluster column of connected-component ids.
        #[arg(long)]
        cluster: bool,
        /// Write a node->cluster TSV here (implies --cluster).
        #[arg(long = "cluster_tsv")]
        cluster_tsv: Option<String>,
        /// Omit the header row from the cluster TSV.
        #[arg(long = "no_cluster_header")]
        no_cluster_header: bool,
        /// Keep only maximum-spanning-tree edges.
        #[arg(long)]
        mst: bool,
        /// Keep MST + OR-symmetric kNN edges (integer >= 0, 0 = spanning tree only).
        #[arg(long = "mst_knn", value_parser = parse_mst_knn)]
        mst_knn: Option<usize>,
        /// Keep OR-symmetric kNN edges without the spanning tree. Unlike --mst_knn this
        /// does not preserve connected components.
        #[arg(long = "knn", value_parser = parse_knn)]
        knn: Option<usize>,
        /// Fail if the projected XGMML output exceeds this many GB (default 25; 0 disables).
        #[arg(long = "max_output_gb")]
        max_output_gb: Option<f64>,
        /// Read parameters from a JSON/YAML config file (CLI flags override it).
        #[arg(long)]
        config: Option<String>,
        /// Print the resolved parameters as YAML and exit.
        #[arg(long = "print_config")]
        print_config: bool,
    },
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
        /// Path to the NCBI taxonomy database directory (default: /tmp/ncbi_taxonomy_$USER).
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
    /// Tabulate HMM-profile metadata to TSV (column order = flag order; or --json).
    HmmerReport {
        /// All hmmer_report arguments (parsed in order; see the Python tool's flags).
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, num_args = 0..)]
        args: Vec<String>,
    },
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

/// Parse `--mst_knn`: an integer >= 1 (mirrors `transform_matrix._mst_knn_arg`).
/// `k == 1` reduces to the MST plus each node's single nearest neighbor.
/// `--mst_knn k` is exact shorthand for `--mst --knn k`, so `0` is legal and means
/// the spanning tree alone.
fn parse_mst_knn(value: &str) -> std::result::Result<usize, String> {
    let v: i64 = value
        .parse()
        .map_err(|_| format!("`{value}` is not an integer"))?;
    if v < 0 {
        return Err("--mst_knn must be an integer >= 0 (0 reduces to --mst)".to_string());
    }
    Ok(v as usize)
}

/// A pure kNN graph with `k = 0` would have no edges at all, so `--knn` starts at 1.
fn parse_knn(value: &str) -> std::result::Result<usize, String> {
    let v: i64 = value
        .parse()
        .map_err(|_| format!("`{value}` is not an integer"))?;
    if v < 1 {
        return Err("--knn must be an integer >= 1".to_string());
    }
    Ok(v as usize)
}

fn load(input: &Path) -> Result<Vec<SeqRecord>> {
    let path = input.to_str().context("non-UTF8 path")?;
    match detect_filetype(input) {
        Some(FileType::Genbank) => read_genbank(path).context("reading GenBank"),
        Some(FileType::Fasta) => fasta::read_fasta(path, Some("protein")).context("reading FASTA"),
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
fn filter_contigs(
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

/// Like [`load_inputs`] but over string paths (config-file shape).
fn load_inputs_str(inputs: &[String]) -> Result<Vec<SeqRecord>> {
    let paths: Vec<PathBuf> = inputs.iter().map(PathBuf::from).collect();
    load_inputs(&paths)
}

/// Convert paths to UTF-8 strings.
fn paths_to_strings(paths: &[PathBuf]) -> Result<Vec<String>> {
    paths
        .iter()
        .map(|p| p.to_str().map(str::to_string).context("non-UTF8 path"))
        .collect()
}

/// Read a color table: a TSV with no header mapping a value to a hex color.
fn read_color_table(path: &str) -> Result<std::collections::HashMap<String, String>> {
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

/// Enforce a projected-output-size guardrail (mirrors `output_guardrails`): fail
/// if the cumulative uncompressed GenBank size of `records` exceeds `max_gb` GB.
/// `max_gb == 0` disables the check; `max_gb < 0` is an error.
fn enforce_output_limit_gb(records: &[SeqRecord], max_gb: f64) -> Result<()> {
    if max_gb < 0.0 {
        anyhow::bail!("--max_output_gb must be >= 0.");
    }
    if max_gb == 0.0 {
        return Ok(());
    }
    let max_bytes = (max_gb * 1_000_000_000.0) as u64;
    let mut total: u64 = 0;
    let mut buf: Vec<u8> = Vec::new();
    for rec in records {
        buf.clear();
        write_record(&mut buf, rec)?;
        total += buf.len() as u64;
        if total > max_bytes {
            anyhow::bail!(
                "Projected domain_search GenBank output size is {:.2} GB, which exceeds the \
                 configured limit of {max_gb} GB. Reduce the projected output size or raise the \
                 limit with --max_output_gb. Relevant options: --max_hits, --max_hits_per_contig, \
                 --cds_range, --kb_range.",
                total as f64 / 1_000_000_000.0
            );
        }
    }
    Ok(())
}

/// Convert a `--max_output_gb` value (GB) to a byte budget, mirroring
/// `output_guardrails.max_output_gb_to_bytes`: negative is an error, `0` disables
/// (returns `None`).
fn max_output_gb_to_bytes(max_gb: f64) -> Result<Option<u64>> {
    if max_gb < 0.0 {
        anyhow::bail!("--max_output_gb must be >= 0.");
    }
    if max_gb == 0.0 {
        return Ok(None);
    }
    Ok(Some((max_gb * 1_000_000_000.0) as u64))
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

/// Fail when the estimated size of writing `matrix` as `output_type` (`"dense"`,
/// `"sparse"`, `"dense_text"`) to `path` exceeds the byte budget for `max_gb`.
/// Uses the core matrix estimator (mirrors `DataMatrix.estimate_write_size`).
fn enforce_matrix_gb(
    matrix: &domainator_core::matrix::DataMatrix,
    output_type: &str,
    path: &str,
    max_gb: f64,
    mitigation: &str,
) -> Result<()> {
    let projected = domainator_core::matrix::estimate::estimate_write_size(output_type, matrix);
    let description = format!("{output_type} matrix output '{path}'");
    enforce_bytes_gb(projected, max_gb, &description, mitigation)
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

/// Write records as FASTA (Biopython header convention) to a file or stdout.
fn write_fasta_bp_out(records: &[SeqRecord], output: Option<&PathBuf>) -> Result<()> {
    match output {
        Some(p) => {
            let file = std::fs::File::create(p).context("creating FASTA output")?;
            let mut w = std::io::BufWriter::new(file);
            fasta::write_fasta_bp(&mut w, records)?;
            w.flush()?;
            Ok(())
        }
        None => {
            let stdout = std::io::stdout();
            let mut h = std::io::BufWriter::new(stdout.lock());
            fasta::write_fasta_bp(&mut h, records)?;
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
fn load_typed(input: &Path, fasta_type: &str) -> Result<Vec<SeqRecord>> {
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

/// Parse `deduplicate`'s `--params` (an inner JSON dict like `"-s":0.9`) into an
/// ordered list of `(flag, value)` args. Numbers/bools are stringified; a null
/// value means a bare flag.
fn parse_dedup_params(s: Option<&str>) -> Result<Vec<(String, Option<String>)>> {
    let Some(s) = s.filter(|s| !s.trim().is_empty()) else {
        return Ok(Vec::new());
    };
    let wrapped = format!("{{{s}}}");
    let val: serde_json::Value =
        serde_json::from_str(&wrapped).context("parsing --params as a JSON dict")?;
    let obj = val
        .as_object()
        .context("--params must be a JSON object body")?;
    let mut out = Vec::new();
    for (k, v) in obj {
        let sv = match v {
            serde_json::Value::Null => None,
            serde_json::Value::String(s) => Some(s.clone()),
            other => Some(other.to_string()),
        };
        out.push((k.clone(), sv));
    }
    Ok(out)
}

/// Compile `--include_taxids`/`--exclude_taxids`/`--taxonomy_expr` into an
/// allowed-taxid set, loading the taxonomy from `path` when a filter is active.
/// Returns `None` when no taxonomy filtering was requested.
fn compile_allowed_taxids(
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
        args.get(*i)
            .cloned()
            .with_context(|| format!("{what} requires a value"))
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
            "--ncbi_taxonomy_path" => {
                cfg.ncbi_taxonomy_path = Some(next(&mut i, "--ncbi_taxonomy_path")?)
            }
            "--json" => cfg.json = Some(next(&mut i, "--json")?),
            "--config" => config_path = Some(next(&mut i, "--config")?),
            "--print_config" => print_config = true,
            "--taxonomy_update" => {}
            "--html" | "--html_max_height" => {
                anyhow::bail!(
                    "enum_report HTML output is not supported in this port; use TSV or --json"
                )
            }
            "--taxid" => cfg.columns.push(ColumnSpec::Taxid {
                ranks: consume_multi(&mut i),
            }),
            "--taxname" => cfg.columns.push(ColumnSpec::Taxname {
                ranks: consume_multi(&mut i),
            }),
            "--qualifier" => {
                let feature = next(&mut i, "--qualifier")?;
                let qualifier = next(&mut i, "--qualifier")?;
                cfg.columns
                    .push(ColumnSpec::Qualifier { feature, qualifier });
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
                cfg.columns.push(ColumnSpec::Append {
                    name,
                    value_type,
                    value,
                });
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
    Ok(EnumCli {
        config: cfg,
        config_path,
        print_config,
    })
}

/// CLI-provided `hmmer_report` params (column order preserved) + control flags.
struct HmmerReportCli {
    config: config::HmmerReportConfig,
    config_path: Option<String>,
    print_config: bool,
}

/// Parse `hmmer_report` args by hand so the relative order of column flags is
/// preserved (mirrors the Python tool's ordered argparse actions).
fn parse_hmmer_report_args(args: &[String]) -> Result<HmmerReportCli> {
    let is_flag = |t: &str| t == "-i" || t == "-o" || t.starts_with("--");
    let mut cfg = config::HmmerReportConfig::default();
    let mut config_path = None;
    let mut print_config = false;

    let mut i = 0;
    let next = |i: &mut usize, what: &str| -> Result<String> {
        *i += 1;
        args.get(*i)
            .cloned()
            .with_context(|| format!("{what} requires a value"))
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
            "--column_names" => cfg.column_names = Some(consume_multi(&mut i)),
            "--json" => cfg.json = Some(next(&mut i, "--json")?),
            "--config" => config_path = Some(next(&mut i, "--config")?),
            "--print_config" => print_config = true,
            "--html" | "--html_max_height" => {
                anyhow::bail!(
                    "hmmer_report HTML output is not supported in this port; use TSV or --json"
                )
            }
            "--append" => {
                let name = next(&mut i, "--append")?;
                let value_type = next(&mut i, "--append")?;
                let value = next(&mut i, "--append")?;
                cfg.columns.push(HmmerReportColumn::Append {
                    name,
                    value_type,
                    value,
                });
            }
            other if other.starts_with("--") => {
                let name = &other[2..];
                match HmmerReportColumn::from_static_flag(name) {
                    Some(spec) => cfg.columns.push(spec),
                    None => anyhow::bail!("unknown hmmer_report option: {other}"),
                }
            }
            other => anyhow::bail!("unexpected hmmer_report argument: {other}"),
        }
        i += 1;
    }
    Ok(HmmerReportCli {
        config: cfg,
        config_path,
        print_config,
    })
}

/// Run one `domainator` command line. `args` includes the program name first, as
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
    let cli = Cli::try_parse_from(args)?;
    dispatch(cli)
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
        Command::Domainate {
            input,
            references,
            output,
            evalue,
            min_evalue,
            z,
            max_overlap,
            max_domains,
            max_mode,
            overlap_by_db,
            hits_only,
            no_annotations,
            cpu,
            fasta_type,
            include_taxids,
            exclude_taxids,
            taxonomy_expr,
            ncbi_taxonomy_path,
            config: config_path,
            print_config: do_print,
        } => {
            let cli = config::DomainateConfig {
                input: opt_path(&input)?,
                references: paths_to_strings(&references)?,
                output: opt_path(&output)?,
                evalue,
                min_evalue,
                z,
                max_overlap,
                max_domains,
                max_mode,
                overlap_by_db,
                hits_only,
                no_annotations,
                cpu,
                fasta_type,
                include_taxids,
                exclude_taxids,
                taxonomy_expr,
                ncbi_taxonomy_path,
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
            let input = cfg
                .input
                .context("missing required `input` (-i / --input or config)")?;
            let output = cfg
                .output
                .context("missing required `output` (-o / --output or config)")?;
            let fasta_type = cfg.fasta_type.as_deref().unwrap_or("protein");
            let records = load_typed(&PathBuf::from(&input), fasta_type)?;
            let allowed_taxids = compile_allowed_taxids(
                &cfg.include_taxids,
                &cfg.exclude_taxids,
                cfg.taxonomy_expr.as_deref(),
                cfg.ncbi_taxonomy_path.as_deref(),
            )?;
            let dbs =
                domainate::read_hmm_references(&cfg.references).context("reading references")?;
            // Protein input cannot be searched with nucleotide references
            // (mirrors validate_input_molecule_types).
            if dbs.has_nucleotide() && records.iter().any(|r| r.is_protein()) {
                anyhow::bail!("Cannot search protein inputs with a nucleotide reference.");
            }
            // Python `--max_domains 0` means unlimited.
            let max_domains = match cfg.max_domains {
                Some(0) | None => usize::MAX,
                Some(n) => n,
            };
            let opts = domainate::DomainateOptions {
                z: cfg.z.unwrap_or(1000.0),
                evalue: cfg.evalue.unwrap_or(0.001),
                min_evalue: cfg.min_evalue.unwrap_or(0.0),
                max_hits: max_domains,
                max_overlap: cfg.max_overlap.unwrap_or(1.0),
                no_annotations: cfg.no_annotations,
                hits_only: cfg.hits_only,
                overlap_by_db: cfg.overlap_by_db,
                max_mode: cfg.max_mode,
                allowed_taxids,
                cpu: cfg.cpu.unwrap_or(0),
                ..Default::default()
            };
            let annotated = domainate::domainate(records, &dbs, &opts);
            write_genbank(&output, &annotated).context("writing GenBank")?;
            println!("wrote {} records to {}", annotated.len(), output);
        }
        Command::DomainSearch {
            input,
            references,
            output,
            evalue,
            min_evalue,
            z,
            add_annotations,
            max_hits,
            max_hits_per_contig,
            max_overlap,
            max_mode,
            batch_size,
            cpu,
            fasta_type,
            decoys,
            decoys_file,
            whole_contig,
            cds_range,
            cds_up,
            cds_down,
            kb_range,
            kb_up,
            kb_down,
            strand,
            keep_direction,
            max_region_overlap,
            translate,
            pad,
            include_taxids,
            exclude_taxids,
            taxonomy_expr,
            ncbi_taxonomy_path,
            max_output_gb,
            config: config_path,
            print_config: do_print,
        } => {
            let cli = config::DomainSearchConfig {
                input: opt_path(&input)?,
                references: paths_to_strings(&references)?,
                output: opt_path(&output)?,
                evalue,
                min_evalue,
                z,
                add_annotations,
                max_hits,
                max_hits_per_contig,
                max_overlap,
                max_mode,
                batch_size,
                cpu,
                fasta_type,
                whole_contig,
                cds_range,
                cds_up,
                cds_down,
                kb_range,
                kb_up,
                kb_down,
                strand,
                keep_direction,
                translate,
                pad,
                max_region_overlap,
                decoys,
                decoys_file,
                include_taxids,
                exclude_taxids,
                taxonomy_expr,
                ncbi_taxonomy_path,
                max_output_gb,
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
            let input = cfg
                .input
                .context("missing required `input` (-i / --input or config)")?;
            let output = cfg
                .output
                .context("missing required `output` (-o / --output or config)")?;
            let dbs =
                domainate::read_hmm_references(&cfg.references).context("reading references")?;
            let range = tools::neighborhood::validate_range(
                cfg.whole_contig,
                cfg.kb_range,
                cfg.kb_up,
                cfg.kb_down,
                cfg.cds_range,
                cfg.cds_up,
                cfg.cds_down,
            )
            .map_err(|e| anyhow::anyhow!(e))?;
            if cfg.translate && !matches!(range, tools::neighborhood::Range::None) {
                anyhow::bail!("--translate is incompatible with neighborhood extraction (--cds_range/--kb_range/--whole_contig)");
            }
            // Mirrors Python's "Protein sequence output is not compatible with
            // neighborhood extraction or padding." A peptide has no neighborhood to
            // align, so padding it would only right-pad to a common length.
            if cfg.pad && cfg.translate {
                anyhow::bail!("--pad is incompatible with --translate (protein output has no neighborhood to align)");
            }
            let strand = match cfg.strand.as_deref() {
                Some("f") => Some(1),
                Some("r") => Some(-1),
                Some(other) => anyhow::bail!("strand must be 'f' or 'r', got {other:?}"),
                None => None,
            };
            let allowed_taxids = compile_allowed_taxids(
                &cfg.include_taxids,
                &cfg.exclude_taxids,
                cfg.taxonomy_expr.as_deref(),
                cfg.ncbi_taxonomy_path.as_deref(),
            )?;
            let decoys = parse_name_list(&cfg.decoys, cfg.decoys_file.as_deref())?;
            let opts = domain_search::DomainSearchOptions {
                z: cfg.z.unwrap_or(1000.0),
                evalue: cfg.evalue.unwrap_or(0.001),
                min_evalue: cfg.min_evalue.unwrap_or(0.0),
                max_overlap: cfg.max_overlap.unwrap_or(1.0),
                add_annotations: cfg.add_annotations,
                max_hits: cfg.max_hits,
                max_hits_per_contig: cfg.max_hits_per_contig,
                pad: cfg.pad,
                batch_size: cfg.batch_size.unwrap_or(10000),
                fasta_type: cfg.fasta_type.clone().unwrap_or_else(|| "protein".into()),
                cpu: cfg.cpu.unwrap_or(0),
                range,
                normalize_direction: !cfg.keep_direction,
                strand,
                max_region_overlap: cfg.max_region_overlap.unwrap_or(1.0),
                translate: cfg.translate,
                max_mode: cfg.max_mode,
                allowed_taxids,
                decoys,
            };
            let hits = domain_search::domain_search(&[input], &dbs, &opts)
                .context("running domain_search")?;
            // Output-size guardrail (uncompressed GenBank bytes; default 25 GB, 0 disables).
            enforce_output_limit_gb(&hits, cfg.max_output_gb.unwrap_or(25.0))?;
            write_genbank(&output, &hits).context("writing GenBank")?;
            println!("wrote {} hit contigs to {}", hits.len(), output);
        }
        Command::Deduplicate {
            input,
            output,
            log,
            fasta_type,
            algorithm,
            bin_path,
            id,
            prefix_count,
            suffix_count,
            cpu,
            params,
            fasta_out,
            both_strands,
            cluster_table,
            cluster_sep,
            config: config_path,
            print_config: do_print,
        } => {
            let cli = config::DeduplicateConfig {
                input: paths_to_strings(&input)?,
                output: opt_path(&output)?,
                log,
                fasta_type,
                algorithm,
                bin_path,
                id,
                prefix_count,
                suffix_count,
                cpu,
                params,
                fasta_out,
                both_strands,
                cluster_table,
                cluster_sep,
            };
            let mut cfg = match &config_path {
                Some(p) => load_config::<config::DeduplicateConfig>(p)?,
                None => config::DeduplicateConfig::default(),
            };
            cfg.merge_cli(cli);
            if do_print {
                print!("{}", print_config(&cfg)?);
                return Ok(());
            }
            if cfg.input.is_empty() {
                anyhow::bail!("missing required `input` (-i / --input or config)");
            }
            if cfg.prefix_count && cfg.suffix_count {
                anyhow::bail!("--prefix_count and --suffix_count are mutually exclusive");
            }
            let id = cfg.id.context("missing required `--id`")?;
            let fasta_type = cfg.fasta_type.clone().unwrap_or_else(|| "protein".into());
            let params = parse_dedup_params(cfg.params.as_deref())?;
            // Read all inputs once (used for both clustering and output).
            let mut records: Vec<SeqRecord> = Vec::new();
            for p in &cfg.input {
                records.extend(load_typed(&PathBuf::from(p), &fasta_type)?);
            }
            let opts = deduplicate_genbank::DeduplicateOptions {
                algorithm: cfg.algorithm.clone().unwrap_or_else(|| "cd-hit".into()),
                id,
                both_strands: cfg.both_strands,
                fasta_type: fasta_type.clone(),
                cpu: cfg.cpu.unwrap_or(0),
                bin_path: cfg.bin_path.clone(),
                params,
            };
            let result = deduplicate_genbank::deduplicate(&records, &opts)
                .map_err(|e| anyhow::anyhow!(e))?;

            // Build output records (representatives, input order) with optional counts.
            let count_by_rep: std::collections::HashMap<usize, usize> =
                result.clusters.iter().map(|(r, m)| (*r, m.len())).collect();
            let mut final_name: std::collections::HashMap<usize, String> =
                std::collections::HashMap::new();
            let mut out_records: Vec<SeqRecord> = Vec::new();
            for &i in &result.representatives {
                let mut rec = records[i].clone();
                let base = rec.id.clone();
                let count = count_by_rep.get(&i).copied().unwrap_or(1);
                let newname = if cfg.prefix_count {
                    format!("{count}-{base}")
                } else if cfg.suffix_count {
                    format!("{base}-{count}")
                } else {
                    base
                };
                rec.id = newname.clone();
                final_name.insert(i, newname);
                out_records.push(rec);
            }

            if cfg.fasta_out {
                write_fasta_bp_out(
                    &out_records,
                    cfg.output.as_ref().map(PathBuf::from).as_ref(),
                )?;
            } else {
                write_genbank_out(
                    &out_records,
                    cfg.output.as_ref().map(PathBuf::from).as_ref(),
                )?;
            }

            if let Some(table_path) = &cfg.cluster_table {
                let sep = cfg.cluster_sep.clone().unwrap_or_else(|| " ; ".into());
                let mut t = String::from("representative\tcontigs\n");
                for (rep, members) in &result.clusters {
                    let rep_name = final_name.get(rep).cloned().unwrap_or_default();
                    let names: Vec<String> =
                        members.iter().map(|&m| records[m].id.clone()).collect();
                    t.push_str(&format!("{rep_name}\t{}\n", names.join(&sep)));
                }
                std::fs::write(table_path, t).context("writing cluster table")?;
            }

            let log_msg = format!(
                "Input  size: {}\nOutput size: {}\n",
                records.len(),
                out_records.len()
            );
            match &cfg.log {
                Some(p) => std::fs::write(p, log_msg).context("writing log")?,
                None => eprint!("{log_msg}"),
            }
        }
        Command::GenbankToFasta {
            input,
            output,
            config: config_path,
            print_config: do_print,
        } => {
            let cli = config::GenbankToFastaConfig {
                input: paths_to_strings(&input)?,
                output: opt_path(&output)?,
            };
            let mut cfg = match &config_path {
                Some(p) => load_config::<config::GenbankToFastaConfig>(p)?,
                None => config::GenbankToFastaConfig::default(),
            };
            cfg.merge_cli(cli);
            if do_print {
                print!("{}", print_config(&cfg)?);
                return Ok(());
            }
            let records = load_inputs_str(&cfg.input)?;
            write_fasta_bp_out(&records, cfg.output.as_ref().map(PathBuf::from).as_ref())?;
        }
        Command::CleanSequences {
            input,
            output,
            log,
            fasta_type,
            fasta_out,
            clean_name,
            deduplicate_names,
            strip,
            filter_by_aa,
            config: config_path,
            print_config: do_print,
        } => {
            let cli = config::CleanSequencesConfig {
                input: paths_to_strings(&input)?,
                output: opt_path(&output)?,
                log,
                fasta_type,
                fasta_out,
                clean_name,
                deduplicate_names,
                strip,
                filter_by_aa,
            };
            let mut cfg = match &config_path {
                Some(p) => load_config::<config::CleanSequencesConfig>(p)?,
                None => config::CleanSequencesConfig::default(),
            };
            cfg.merge_cli(cli);
            if do_print {
                print!("{}", print_config(&cfg)?);
                return Ok(());
            }
            let fasta_type = cfg.fasta_type.clone().unwrap_or_else(|| "protein".into());
            let mut records: Vec<SeqRecord> = Vec::new();
            for p in &cfg.input {
                records.extend(load_typed(&PathBuf::from(p), &fasta_type)?);
            }
            let opts = clean_sequences::CleanSequencesOptions {
                fasta_type: fasta_type.clone(),
                clean_name: cfg.clean_name,
                deduplicate_names: cfg.deduplicate_names,
                strip: cfg.strip,
                filter_by_aa: cfg.filter_by_aa,
            };
            let mut stats = clean_sequences::CleaningStats::default();
            let out = clean_sequences::clean_sequences(records, &opts, &mut stats);
            if cfg.fasta_out {
                write_fasta_bp_out(&out, cfg.output.as_ref().map(PathBuf::from).as_ref())?;
            } else {
                write_genbank_out(&out, cfg.output.as_ref().map(PathBuf::from).as_ref())?;
            }
            // Six lines, spelled exactly as `clean_sequences.py` prints them -- the
            // log is the tool's report, so its wording is part of the output.
            let msg = format!(
                "Input sequences:            {}\n\
                 Output sequences:           {}\n\
                 Filtered (non-canonical):   {}\n\
                 Filtered (empty after strip): {}\n\
                 Names changed:              {}\n\
                 Sequences changed:          {}\n",
                stats.input_count,
                stats.output_count,
                stats.filtered_count,
                stats.empty_after_strip_count,
                stats.names_changed_count,
                stats.sequences_changed_count,
            );
            match &cfg.log {
                Some(p) => std::fs::write(p, msg).context("writing log")?,
                None => eprint!("{msg}"),
            }
        }
        Command::SortContigs {
            input,
            output,
            max_hits,
            domains,
            databases,
            config: config_path,
            print_config: do_print,
        } => {
            let cli = config::SortContigsConfig {
                input: paths_to_strings(&input)?,
                output: opt_path(&output)?,
                max_hits,
                domains,
                databases,
            };
            let mut cfg = match &config_path {
                Some(p) => load_config::<config::SortContigsConfig>(p)?,
                None => config::SortContigsConfig::default(),
            };
            cfg.merge_cli(cli);
            if do_print {
                print!("{}", print_config(&cfg)?);
                return Ok(());
            }
            let records = load_inputs_str(&cfg.input)?;
            // Python: --max_hits 0 means "return all"; databases only apply with domains.
            let max_hits = match cfg.max_hits {
                Some(0) | None => None,
                Some(n) => Some(n),
            };
            let domains = if cfg.domains.is_empty() {
                None
            } else {
                Some(cfg.domains.iter().cloned().collect())
            };
            let databases = if domains.is_none() || cfg.databases.is_empty() {
                None
            } else {
                Some(cfg.databases.iter().cloned().collect())
            };
            let opts = sort_contigs::SortContigsOptions {
                max_hits,
                domains,
                databases,
            };
            let out = sort_contigs::sort_contigs(records, &opts).map_err(|e| anyhow::anyhow!(e))?;
            write_genbank_out(&out, cfg.output.as_ref().map(PathBuf::from).as_ref())?;
        }
        Command::SelectByCoord {
            input,
            output,
            coords,
            config: config_path,
            print_config: do_print,
        } => {
            let cli = config::SelectByCoordConfig {
                input: paths_to_strings(&input)?,
                output: opt_path(&output)?,
                coords: opt_path(&coords)?,
            };
            let mut cfg = match &config_path {
                Some(p) => load_config::<config::SelectByCoordConfig>(p)?,
                None => config::SelectByCoordConfig::default(),
            };
            cfg.merge_cli(cli);
            if do_print {
                print!("{}", print_config(&cfg)?);
                return Ok(());
            }
            let coords_path = cfg
                .coords
                .clone()
                .context("missing required `--coords` (-c)")?;
            let table =
                std::fs::read_to_string(&coords_path).context("reading coordinate table")?;
            let selections = tools::select_by_coord::parse_coordinate_table(&table)
                .map_err(|e| anyhow::anyhow!(e))?;
            // Load each input file keeping its basename for the file-column filter.
            let mut records: Vec<(String, SeqRecord)> = Vec::new();
            for p in &cfg.input {
                let path = PathBuf::from(p);
                let base = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or(p)
                    .to_string();
                for rec in load_inputs(std::slice::from_ref(&path))? {
                    records.push((base.clone(), rec));
                }
            }
            let out = tools::select_by_coord::select_by_coord(&records, &selections)
                .map_err(|e| anyhow::anyhow!(e))?;
            write_genbank_out(&out, cfg.output.as_ref().map(PathBuf::from).as_ref())?;
        }
        Command::FilterDomains {
            input,
            output,
            evalue,
            max_overlap,
            domains,
            domains_file,
            databases_keep,
            databases_filter,
            annotation_type,
            contigs,
            contigs_file,
            config: config_path,
            print_config: do_print,
        } => {
            let cli = config::FilterDomainsConfig {
                input: paths_to_strings(&input)?,
                output: opt_path(&output)?,
                evalue,
                max_overlap,
                domains,
                domains_file,
                databases_keep,
                databases_filter,
                annotation_type,
                contigs,
                contigs_file,
            };
            let mut cfg = match &config_path {
                Some(p) => load_config::<config::FilterDomainsConfig>(p)?,
                None => config::FilterDomainsConfig::default(),
            };
            cfg.merge_cli(cli);
            if do_print {
                print!("{}", print_config(&cfg)?);
                return Ok(());
            }
            let mut records = load_inputs_str(&cfg.input)?;
            filter_contigs(&mut records, &cfg.contigs, cfg.contigs_file.as_deref())?;
            let selected_domains = parse_name_list(&cfg.domains, cfg.domains_file.as_deref())?;
            let databases_keep = if cfg.databases_keep.is_empty() {
                None
            } else {
                Some(cfg.databases_keep.iter().cloned().collect())
            };
            let databases_filter = if cfg.databases_filter.is_empty() {
                None
            } else {
                Some(cfg.databases_filter.iter().cloned().collect())
            };
            let annotation_type = tools::filter_domains::AnnotationType::parse(
                cfg.annotation_type.as_deref().unwrap_or("all"),
            )
            .map_err(|e| anyhow::anyhow!(e))?;
            let opts = tools::filter_domains::FilterDomainsOptions {
                evalue: cfg.evalue.unwrap_or(100000000.0),
                max_overlap: cfg.max_overlap.unwrap_or(1.0),
                databases_keep,
                databases_filter,
                selected_domains,
                annotation_type,
            };
            let out = tools::filter_domains::filter_domains(records, &opts);
            write_genbank_out(&out, cfg.output.as_ref().map(PathBuf::from).as_ref())?;
        }
        Command::ColorGenbank {
            input,
            output,
            contigs,
            contigs_file,
            color_table,
            color_table_out,
            search_hit_color,
            clear,
            databases,
            mode,
            domain_database,
            config: config_path,
            print_config: do_print,
        } => {
            let cli = config::ColorGenbankConfig {
                input: paths_to_strings(&input)?,
                output: opt_path(&output)?,
                contigs,
                contigs_file,
                color_table,
                color_table_out,
                search_hit_color,
                clear,
                databases,
                mode,
                domain_database,
            };
            let mut cfg = match &config_path {
                Some(p) => load_config::<config::ColorGenbankConfig>(p)?,
                None => config::ColorGenbankConfig::default(),
            };
            cfg.merge_cli(cli);
            if do_print {
                print!("{}", print_config(&cfg)?);
                return Ok(());
            }
            let mut records = load_inputs_str(&cfg.input)?;
            filter_contigs(&mut records, &cfg.contigs, cfg.contigs_file.as_deref())?;

            let targets =
                tools::color_genbank::ColorTargets::parse(cfg.mode.as_deref().unwrap_or("cdss"))
                    .map_err(|e| anyhow::anyhow!(e))?;
            let color_feature = if cfg.domain_database {
                "database"
            } else {
                "name"
            }
            .to_string();

            let mut color_map = match &cfg.color_table {
                Some(path) => {
                    let text = std::fs::read_to_string(path)
                        .with_context(|| format!("reading color table {path}"))?;
                    let table = tools::color_genbank::read_color_table(&text)
                        .map_err(|e| anyhow::anyhow!(e))?;
                    tools::color_genbank::ColorMap::from_table(table)
                }
                None => tools::color_genbank::ColorMap::auto(),
            };

            let search_hit_color = match &cfg.search_hit_color {
                Some(c) => Some(
                    tools::color_genbank::normalize_color_hex(c).map_err(|e| anyhow::anyhow!(e))?,
                ),
                None => None,
            };
            let databases = if cfg.databases.is_empty() {
                None
            } else {
                Some(cfg.databases.iter().cloned().collect())
            };

            let opts = tools::color_genbank::ColorGenbankOptions {
                targets,
                search_hit_color,
                clear: cfg.clear,
                databases,
                color_feature,
            };
            let out = tools::color_genbank::color_genbank(records, &mut color_map, &opts);
            write_genbank_out(&out, cfg.output.as_ref().map(PathBuf::from).as_ref())?;

            if let Some(path) = &cfg.color_table_out {
                use std::io::Write;
                let mut f = std::fs::File::create(path)
                    .with_context(|| format!("writing color table {path}"))?;
                for (annotation, color) in color_map.items() {
                    writeln!(f, "{annotation}\t{color}")?;
                }
            }
        }
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
        Command::TrimContigs {
            input,
            output,
            kb_both,
            kb_left,
            kb_right,
            cds_both,
            cds_left,
            cds_right,
            domains,
            domains_file,
            domain_expr,
            no_domain,
            contigs,
            contigs_file,
            config: config_path,
            print_config: do_print,
        } => {
            let cli = config::TrimContigsConfig {
                input: paths_to_strings(&input)?,
                output: opt_path(&output)?,
                kb_both,
                kb_left,
                kb_right,
                cds_both,
                cds_left,
                cds_right,
                domains,
                domains_file,
                domain_expr,
                no_domain,
                contigs,
                contigs_file,
            };
            let mut cfg = match &config_path {
                Some(p) => load_config::<config::TrimContigsConfig>(p)?,
                None => config::TrimContigsConfig::default(),
            };
            cfg.merge_cli(cli);
            if do_print {
                print!("{}", print_config(&cfg)?);
                return Ok(());
            }
            let mut records = load_inputs_str(&cfg.input)?;
            filter_contigs(&mut records, &cfg.contigs, cfg.contigs_file.as_deref())?;
            let (kb_truncation, cds_truncation) = tools::trim_contigs::validate_range(
                cfg.kb_both,
                cfg.kb_left,
                cfg.kb_right,
                cfg.cds_both,
                cfg.cds_left,
                cfg.cds_right,
            )
            .map_err(|e| anyhow::anyhow!(e))?;
            let strip_domains = parse_name_list(&cfg.domains, cfg.domains_file.as_deref())?;
            let opts = tools::trim_contigs::TrimContigsOptions {
                kb_truncation,
                cds_truncation,
                strip_domains,
                domain_expr: cfg.domain_expr.clone(),
                no_domain: cfg.no_domain,
            };
            let out = tools::trim_contigs::trim_contigs(records, &opts)
                .map_err(|e| anyhow::anyhow!(e))?;
            write_genbank_out(&out, cfg.output.as_ref().map(PathBuf::from).as_ref())?;
        }
        Command::ExtractDomains {
            input,
            output,
            evalue,
            score,
            domains,
            domains_file,
            databases,
            search_hits,
            pad_up,
            pad_down,
            combine,
            splice,
            keep_direction,
            keep_name,
            fasta_out,
            contigs,
            contigs_file,
            config: config_path,
            print_config: do_print,
        } => {
            let cli = config::ExtractDomainsConfig {
                input: paths_to_strings(&input)?,
                output: opt_path(&output)?,
                evalue,
                score,
                domains,
                domains_file,
                databases,
                search_hits,
                pad_up,
                pad_down,
                combine,
                splice,
                keep_direction,
                keep_name,
                fasta_out,
                contigs,
                contigs_file,
            };
            let mut cfg = match &config_path {
                Some(p) => load_config::<config::ExtractDomainsConfig>(p)?,
                None => config::ExtractDomainsConfig::default(),
            };
            cfg.merge_cli(cli);
            if do_print {
                print!("{}", print_config(&cfg)?);
                return Ok(());
            }
            let mut records = load_inputs_str(&cfg.input)?;
            filter_contigs(&mut records, &cfg.contigs, cfg.contigs_file.as_deref())?;
            let selected_domains = parse_name_list(&cfg.domains, cfg.domains_file.as_deref())?;
            let databases = if cfg.databases.is_empty() {
                None
            } else {
                Some(cfg.databases.iter().cloned().collect())
            };
            let opts = tools::extract_domains::ExtractDomainsOptions {
                evalue: cfg.evalue,
                score: cfg.score,
                domains: selected_domains,
                pad_up: cfg.pad_up.unwrap_or(0),
                pad_down: cfg.pad_down.unwrap_or(0),
                combine: cfg.combine,
                keep_direction: cfg.keep_direction,
                keep_name: cfg.keep_name,
                search_hits: cfg.search_hits,
                databases,
                splice: cfg.splice,
            };
            let out = tools::extract_domains::extract_domains(&records, &opts)
                .map_err(|e| anyhow::anyhow!(e))?;
            if cfg.fasta_out {
                write_fasta_bp_out(&out, cfg.output.as_ref().map(PathBuf::from).as_ref())?;
            } else {
                write_genbank_out(&out, cfg.output.as_ref().map(PathBuf::from).as_ref())?;
            }
        }
        Command::ExtractPeptides {
            input,
            output,
            domains,
            domains_file,
            databases,
            unannotated,
            search_hits,
            strand,
            invert,
            cdss,
            cdss_file,
            keep_name,
            name_field,
            evalue,
            fasta_out,
            contigs,
            contigs_file,
            config: config_path,
            print_config: do_print,
        } => {
            let cli = config::ExtractPeptidesConfig {
                input: paths_to_strings(&input)?,
                output: opt_path(&output)?,
                domains,
                domains_file,
                databases,
                unannotated,
                search_hits,
                strand,
                invert,
                cdss,
                cdss_file,
                keep_name,
                name_field,
                evalue,
                fasta_out,
                contigs,
                contigs_file,
            };
            let mut cfg = match &config_path {
                Some(p) => load_config::<config::ExtractPeptidesConfig>(p)?,
                None => config::ExtractPeptidesConfig::default(),
            };
            cfg.merge_cli(cli);
            if do_print {
                print!("{}", print_config(&cfg)?);
                return Ok(());
            }
            let mut records = load_inputs_str(&cfg.input)?;
            filter_contigs(&mut records, &cfg.contigs, cfg.contigs_file.as_deref())?;
            let target_domains = parse_name_list(&cfg.domains, cfg.domains_file.as_deref())?;
            let target_cdss = parse_name_list(&cfg.cdss, cfg.cdss_file.as_deref())?;
            let databases = if cfg.databases.is_empty() {
                None
            } else {
                Some(cfg.databases.iter().cloned().collect())
            };
            let mut invert = cfg.invert;
            // extract_all when no selection criteria; invert then ignored.
            let extract_all = target_domains.is_none()
                && target_cdss.is_none()
                && !cfg.unannotated
                && !cfg.search_hits;
            if extract_all && invert {
                eprintln!("Invert supplied with no selection criteria. Invert will be ignored.");
                invert = false;
            }
            let opts = tools::extract_peptides::ExtractPeptidesOptions {
                evalue: cfg.evalue.unwrap_or(100000000.0),
                target_domains,
                target_cdss,
                search_hits: cfg.search_hits,
                strand: cfg.strand.clone(),
                unannotated: cfg.unannotated,
                invert,
                databases,
                keep_name: cfg.keep_name,
                name_field: cfg.name_field.clone(),
                extract_all,
            };
            let out = tools::extract_peptides::extract_peptides(&records, &opts);
            if cfg.fasta_out {
                write_fasta_bp_out(&out, cfg.output.as_ref().map(PathBuf::from).as_ref())?;
            } else {
                write_genbank_out(&out, cfg.output.as_ref().map(PathBuf::from).as_ref())?;
            }
        }
        Command::ExtractUnannotated {
            input,
            output,
            evalue,
            score,
            databases,
            search_hits,
            keep_direction,
            keep_name,
            largest,
            lb,
            fasta_out,
            contigs,
            contigs_file,
            config: config_path,
            print_config: do_print,
        } => {
            let cli = config::ExtractUnannotatedConfig {
                input: paths_to_strings(&input)?,
                output: opt_path(&output)?,
                evalue,
                score,
                databases,
                search_hits,
                keep_direction,
                keep_name,
                largest,
                lb,
                fasta_out,
                contigs,
                contigs_file,
            };
            let mut cfg = match &config_path {
                Some(p) => load_config::<config::ExtractUnannotatedConfig>(p)?,
                None => config::ExtractUnannotatedConfig::default(),
            };
            cfg.merge_cli(cli);
            if do_print {
                print!("{}", print_config(&cfg)?);
                return Ok(());
            }
            let mut records = load_inputs_str(&cfg.input)?;
            filter_contigs(&mut records, &cfg.contigs, cfg.contigs_file.as_deref())?;
            let databases = if cfg.databases.is_empty() {
                None
            } else {
                Some(cfg.databases.iter().cloned().collect())
            };
            let opts = tools::extract_unannotated::ExtractUnannotatedOptions {
                evalue: cfg.evalue,
                score: cfg.score,
                keep_direction: cfg.keep_direction,
                keep_name: cfg.keep_name,
                search_hits: cfg.search_hits,
                databases,
                largest: cfg.largest,
                lb: cfg.lb,
            };
            let out = tools::extract_unannotated::extract_unannotated(&records, &opts)
                .map_err(|e| anyhow::anyhow!(e))?;
            if cfg.fasta_out {
                write_fasta_bp_out(&out, cfg.output.as_ref().map(PathBuf::from).as_ref())?;
            } else {
                write_genbank_out(&out, cfg.output.as_ref().map(PathBuf::from).as_ref())?;
            }
        }
        Command::SeqDist {
            input,
            reference,
            dense,
            dense_text,
            sparse,
            algorithm,
            mode,
            lb,
            k,
            params,
            mst_knn,
            knn,
            cpu,
            max_output_gb,
            config: config_path,
            print_config: do_print,
        } => {
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
                params,
                mst_knn,
                knn,
                cpu,
                max_output_gb,
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
                anyhow::bail!(
                    "no output specified: pass at least one of --dense, --dense_text, --sparse"
                );
            }
            let input = cfg
                .input
                .context("missing required `input` (-i / --input or config)")?;
            let input_pb = PathBuf::from(&input);
            let input_kind = detect_kind(&input_pb)
                .with_context(|| format!("could not detect input kind: {input}"))?;
            let ref_pb = cfg.reference.as_ref().map(PathBuf::from);
            let ref_kind = match &ref_pb {
                Some(p) => Some(detect_kind(p).with_context(|| {
                    format!("could not detect reference kind: {}", p.display())
                })?),
                None => None,
            };
            let opts = SeqDistOptions {
                algorithm: cfg.algorithm.unwrap_or_else(|| "diamond_us".into()),
                mode: cfg.mode.unwrap_or_else(|| "score".into()),
                lb: cfg.lb.unwrap_or(0.0),
                k: cfg.k,
                mst_knn: cfg.mst_knn,
                knn: cfg.knn,
                params: tools::seq_dist::parse_diamond_params(cfg.params.as_deref().unwrap_or(""))
                    .map_err(|e| anyhow::anyhow!(e))?,
                cpu: cfg.cpu.unwrap_or(0),
            };
            let matrix = seq_dist(
                &input,
                input_kind,
                cfg.reference.as_deref(),
                ref_kind,
                &opts,
            )
            .context("running seq_dist")?;
            // Output-size guardrails: estimate each requested output before writing.
            let max_gb = cfg.max_output_gb.unwrap_or(25.0);
            if let Some(p) = &cfg.dense {
                enforce_matrix_gb(&matrix, "dense", p, max_gb, "--sparse, -k, --lb")?;
            }
            if let Some(p) = &cfg.dense_text {
                enforce_matrix_gb(&matrix, "dense_text", p, max_gb, "--sparse, -k, --lb")?;
            }
            if let Some(p) = &cfg.sparse {
                enforce_matrix_gb(&matrix, "sparse", p, max_gb, "-k, --lb")?;
            }
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
        Command::CompareContigs {
            input,
            dense,
            dense_text,
            sparse,
            ji,
            ai,
            databases,
            k,
            lb,
            mst_knn,
            knn,
            cpu,
            max_output_gb,
            config: config_path,
            print_config: do_print,
        } => {
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
                knn,
                cpu,
                max_output_gb,
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
                anyhow::bail!(
                    "no output specified: pass at least one of --dense, --dense_text, --sparse"
                );
            }
            let ji = cfg.ji.unwrap_or(0.5);
            let ai = cfg.ai.unwrap_or(0.5);
            if ji == 0.0 && ai == 0.0 {
                anyhow::bail!("specify a non-zero weight for at least one metric (--ji / --ai)");
            }
            let opts = CompareContigsOptions {
                ji_weight: ji,
                ai_weight: ai,
                databases: if cfg.databases.is_empty() {
                    None
                } else {
                    Some(cfg.databases.into_iter().collect())
                },
                k: cfg.k,
                lb: cfg.lb.unwrap_or(0.0),
                mst_knn: cfg.mst_knn,
                knn: cfg.knn,
                cpu: cfg.cpu.unwrap_or(0),
            };
            let matrix = compare_contigs(&cfg.input, &opts).context("running compare_contigs")?;
            // Output-size guardrails: estimate each requested output before writing.
            let max_gb = cfg.max_output_gb.unwrap_or(25.0);
            if let Some(p) = &cfg.dense {
                enforce_matrix_gb(&matrix, "dense", p, max_gb, "--sparse, -k")?;
            }
            if let Some(p) = &cfg.dense_text {
                enforce_matrix_gb(&matrix, "dense_text", p, max_gb, "--sparse, -k")?;
            }
            if let Some(p) = &cfg.sparse {
                enforce_matrix_gb(&matrix, "sparse", p, max_gb, "--sparse, -k")?;
            }
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
        Command::TransformMatrix {
            input,
            dense,
            dense_text,
            sparse,
            mode,
            input_type,
            lb,
            mst_knn,
            knn,
            max_output_gb,
            config: config_path,
            print_config: do_print,
        } => {
            let cli = config::TransformMatrixConfig {
                input: opt_path(&input)?,
                dense: opt_path(&dense)?,
                dense_text: opt_path(&dense_text)?,
                sparse: opt_path(&sparse)?,
                mode,
                input_type,
                lb,
                mst_knn,
                knn,
                max_output_gb,
            };
            let mut cfg = match &config_path {
                Some(p) => load_config::<config::TransformMatrixConfig>(p)?,
                None => config::TransformMatrixConfig::default(),
            };
            cfg.merge_cli(cli);
            if do_print {
                print!("{}", print_config(&cfg)?);
                return Ok(());
            }
            let input = cfg
                .input
                .clone()
                .context("missing required `input` (-i / --input or config)")?;
            // Shared config->Options resolution (used by the GUI too, so they can't drift).
            let opts = config::resolve::resolve_transform_matrix(&cfg)?;
            let mode = opts.mode.clone();
            if cfg.dense.is_none() && cfg.dense_text.is_none() && cfg.sparse.is_none() {
                anyhow::bail!(
                    "no output specified: pass at least one of --dense, --dense_text, --sparse"
                );
            }
            if cfg.sparse.is_some() && mode == "score_dist" {
                anyhow::bail!(
                    "sparse distance matrices not implemented (--mode score_dist with --sparse)"
                );
            }
            let matrix = read_matrix(&input).with_context(|| format!("reading matrix {input}"))?;
            let matrix = transform_matrix(matrix, &opts).context("running transform_matrix")?;
            // Output-size guardrails: estimate each requested output before writing.
            let max_gb = cfg.max_output_gb.unwrap_or(25.0);
            if let Some(p) = &cfg.dense {
                enforce_matrix_gb(&matrix, "dense", p, max_gb, "--sparse, --lb, --mst_knn")?;
            }
            if let Some(p) = &cfg.dense_text {
                enforce_matrix_gb(
                    &matrix,
                    "dense_text",
                    p,
                    max_gb,
                    "--sparse, --lb, --mst_knn",
                )?;
            }
            if let Some(p) = &cfg.sparse {
                enforce_matrix_gb(&matrix, "sparse", p, max_gb, "--lb, --mst_knn")?;
            }
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
        Command::BuildSsn {
            input,
            xgmml,
            lb,
            metadata,
            subset,
            subset_file,
            color_by,
            color_table,
            color_table_out,
            max_color_groups,
            cluster,
            cluster_tsv,
            no_cluster_header,
            mst,
            mst_knn,
            knn,
            max_output_gb,
            config: config_path,
            print_config: do_print,
        } => {
            let cli = config::BuildSsnConfig {
                input: opt_path(&input)?,
                xgmml: opt_path(&xgmml)?,
                lb,
                metadata,
                subset,
                subset_file,
                color_by,
                color_table,
                color_table_out,
                max_color_groups,
                cluster,
                cluster_tsv,
                no_cluster_header,
                mst,
                mst_knn,
                knn,
                max_output_gb,
            };
            let mut cfg = match &config_path {
                Some(p) => load_config::<config::BuildSsnConfig>(p)?,
                None => config::BuildSsnConfig::default(),
            };
            cfg.merge_cli(cli);
            if do_print {
                print!("{}", print_config(&cfg)?);
                return Ok(());
            }
            let input = cfg
                .input
                .clone()
                .context("missing required `input` (-i / --input or config)")?;
            let xgmml = cfg
                .xgmml
                .clone()
                .context("missing required `xgmml` (--xgmml or config)")?;
            if cfg.mst && cfg.mst_knn.is_some() {
                anyhow::bail!("--mst and --mst_knn are mutually exclusive");
            }
            let matrix = read_matrix(&input).with_context(|| format!("reading matrix {input}"))?;

            // Resolve subset labels and the optional color table.
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
            let color_table = match &cfg.color_table {
                Some(f) => Some(read_color_table(f)?),
                None => None,
            };
            let opts = config::resolve::resolve_build_ssn(&cfg, subset_labels, color_table);
            let result = build_ssn(&matrix, &opts).context("running build_ssn")?;

            let name = Path::new(&xgmml)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            // Serialize the XGMML to a buffer first so the output-size guardrail
            // (default 25 GB, 0 disables) can reject it before touching disk.
            let mut xgmml_buf: Vec<u8> = Vec::new();
            domainator_core::matrix::cytoscape::write_xgmml_to(
                &mut xgmml_buf,
                &name,
                &result.table,
                &result.edges,
                result.node_fills.as_deref(),
            )?;
            enforce_bytes_gb(
                xgmml_buf.len() as u64,
                cfg.max_output_gb.unwrap_or(25.0),
                &format!("XGMML network output '{xgmml}'"),
                "--lb, --mst, --mst_knn, --subset",
            )?;
            std::fs::write(&xgmml, &xgmml_buf)?;

            if let Some(path) = &cfg.cluster_tsv {
                let labels = result
                    .cluster_labels
                    .as_ref()
                    .expect("cluster labels present");
                let mut out = String::new();
                if !cfg.no_cluster_header {
                    out.push_str("contig\tcluster\n");
                }
                for (name, id) in result.table.names.iter().zip(labels) {
                    out.push_str(name);
                    out.push('\t');
                    out.push_str(&id.to_string());
                    out.push('\n');
                }
                std::fs::write(path, out)?;
            }

            if let Some(path) = &cfg.color_table_out {
                if let Some(ct) = &result.resolved_color_table {
                    let mut items: Vec<(&String, &String)> = ct.iter().collect();
                    items.sort_by(|a, b| a.0.cmp(b.0));
                    let mut out = String::new();
                    for (k, v) in items {
                        out.push_str(k);
                        out.push('\t');
                        out.push_str(v);
                        out.push('\n');
                    }
                    std::fs::write(path, out)?;
                }
            }

            println!(
                "wrote {} nodes, {} edges to {}",
                result.table.names.len(),
                result.edges.len(),
                xgmml
            );
        }
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
                std::fs::write(p, serde_json::to_string_pretty(&to_json(&report))?)?;
            }
            println!(
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
            };
            let mut cfg = match &config_path {
                Some(p) => load_config::<config::SummaryReportConfig>(p)?,
                None => config::SummaryReportConfig::default(),
            };
            cfg.merge_cli(cli);
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
        Command::Schema { tool } => {
            println!("{}", schema(&tool)?);
        }
        Command::EnumReport { args } => {
            let EnumCli {
                config: cli_cfg,
                config_path,
                print_config: do_print,
            } = parse_enum_args(&args)?;
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
            let analyses: Vec<Analysis> = cfg
                .columns
                .iter()
                .map(|c| c.to_analysis())
                .collect::<Result<_>>()?;
            let ncbi_path = cfg
                .ncbi_taxonomy_path
                .clone()
                .unwrap_or_else(default_ncbi_taxonomy_path);

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
                records.retain(|(r, _)| set.contains(&r.id));
            }
            let tax = if needs_taxonomy(&analyses) {
                Some(load_taxonomy(&ncbi_path)?)
            } else {
                None
            };
            let databases: Option<std::collections::HashSet<String>> = if cfg.databases.is_empty() {
                None
            } else {
                Some(cfg.databases.iter().cloned().collect())
            };
            let col_names = cfg.column_names.as_deref();
            // `-o` is TSV and `--json` is NDJSON; they are independent outputs, so a
            // run can produce both. `--json -` writes to stdout, like Python.
            // `-o` defaults to stdout only when no output at all was requested.
            let want_tsv = cfg.output.is_some() || cfg.json.is_none();
            for (target, format) in [
                (want_tsv.then(|| cfg.output.clone()), OutputFormat::Tsv),
                (cfg.json.clone().map(Some), OutputFormat::Json),
            ] {
                let Some(target) = target else { continue };
                let result = match target.as_deref() {
                    Some(p) if p != "-" => {
                        let mut f = std::io::BufWriter::new(std::fs::File::create(p)?);
                        let r = enum_report::enum_report(
                            records.clone(),
                            by,
                            &analyses,
                            col_names,
                            tax.as_ref(),
                            databases.as_ref(),
                            format,
                            &mut f,
                        );
                        f.flush()?;
                        r
                    }
                    _ => {
                        let stdout = std::io::stdout();
                        let mut h = stdout.lock();
                        enum_report::enum_report(
                            records.clone(),
                            by,
                            &analyses,
                            col_names,
                            tax.as_ref(),
                            databases.as_ref(),
                            format,
                            &mut h,
                        )
                    }
                };
                result.map_err(|e| anyhow::anyhow!(e))?;
            }
        }
        Command::SelectByCds {
            input,
            output,
            kb_range,
            kb_up,
            kb_down,
            cds_range,
            cds_up,
            cds_down,
            whole_contig,
            contigs,
            contigs_file,
            cds,
            cds_file,
            domains,
            domains_file,
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
            config: config_path,
            print_config: do_print,
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
                records.retain(|r| set.contains(&r.id));
            }
            let target_cdss = parse_name_list(&cfg.cds, cfg.cds_file.as_deref())?;
            let target_domains = parse_name_list(&cfg.domains, cfg.domains_file.as_deref())?;
            let range = tools::neighborhood::validate_range(
                cfg.whole_contig,
                cfg.kb_range,
                cfg.kb_up,
                cfg.kb_down,
                cfg.cds_range,
                cfg.cds_up,
                cfg.cds_down,
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
                databases: if cfg.databases.is_empty() {
                    None
                } else {
                    Some(cfg.databases.into_iter().collect())
                },
                unannotated: cfg.unannotated,
                include_nucleic_acids: cfg.include_nucleic_acids,
            };
            let mut regions = select_by_cds(&records, &opts);
            // Deduplicate by record name (LOCUS), keeping the first occurrence.
            if !cfg.skip_deduplicate {
                let mut seen = std::collections::HashSet::new();
                regions.retain(|r| seen.insert(r.record.id.clone()));
            }
            if cfg.pad {
                pad_records(&mut regions);
            }
            let out_records: Vec<SeqRecord> = regions.into_iter().map(|r| r.record).collect();
            write_genbank_out(&out_records, cfg.output.map(PathBuf::from).as_ref())?;
        }
        Command::SelectByContig {
            input,
            output,
            contigs,
            contigs_file,
            contigs_regex,
            definition_regex,
            sequence_regex,
            domains,
            domains_file,
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
            config: config_path,
            print_config: do_print,
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
                || cfg
                    .taxonomy_expr
                    .as_deref()
                    .is_some_and(|e| !e.trim().is_empty());
            let tax = if needs_tax {
                Some(load_taxonomy(
                    &cfg.ncbi_taxonomy_path
                        .clone()
                        .unwrap_or_else(default_ncbi_taxonomy_path),
                )?)
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
                include_taxids: if cfg.include_taxids.is_empty() {
                    None
                } else {
                    Some(cfg.include_taxids.into_iter().collect())
                },
                exclude_taxids: if cfg.exclude_taxids.is_empty() {
                    None
                } else {
                    Some(cfg.exclude_taxids.into_iter().collect())
                },
                taxonomy_expr: cfg.taxonomy_expr,
                databases: if cfg.databases.is_empty() {
                    None
                } else {
                    Some(cfg.databases.into_iter().collect())
                },
                unannotated: cfg.unannotated,
                search_score_lb: cfg.search_score_lb,
                search_score_ub: cfg.search_score_ub,
                domain_type: cfg.domain_type.unwrap_or_else(|| "domain".into()),
            };
            let out_path = cfg.output.map(PathBuf::from);
            let selected =
                select_by_contig(records, tax.as_ref(), &opts).map_err(|e| anyhow::anyhow!(e))?;
            if cfg.fasta_out {
                write_fasta_bp_out(&selected, out_path.as_ref())?;
            } else {
                write_genbank_out(&selected, out_path.as_ref())?;
            }
        }
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
        Command::HmmerReport { args } => {
            use domainator_core::tools::hmmer_report as hr;
            let HmmerReportCli {
                config: cli_cfg,
                config_path,
                print_config: do_print,
            } = parse_hmmer_report_args(&args)?;
            let mut cfg = match &config_path {
                Some(p) => load_config::<config::HmmerReportConfig>(p)?,
                None => config::HmmerReportConfig::default(),
            };
            cfg.merge_cli(cli_cfg);
            if do_print {
                print!("{}", print_config(&cfg)?);
                return Ok(());
            }
            let analyses: Vec<hr::Analysis> = cfg
                .columns
                .iter()
                .map(|c| c.to_analysis())
                .collect::<Result<_>>()?;
            if cfg.input.is_empty() {
                anyhow::bail!("hmmer_report requires at least one -i/--input .hmm file");
            }
            let records = hr::read_hmms_to_iterators(&cfg.input).map_err(|e| anyhow::anyhow!(e))?;
            let col_names = cfg.column_names.as_deref();
            // `-o` is TSV and `--json` is NDJSON; independent outputs, so one run can
            // produce both. `--json -` writes to stdout, like Python.
            let want_tsv = cfg.output.is_some() || cfg.json.is_none();
            for (target, format) in [
                (want_tsv.then(|| cfg.output.clone()), hr::OutputFormat::Tsv),
                (cfg.json.clone().map(Some), hr::OutputFormat::Json),
            ] {
                let Some(target) = target else { continue };
                match target.as_deref() {
                    Some(p) if p != "-" => {
                        let mut f = std::io::BufWriter::new(std::fs::File::create(p)?);
                        hr::hmmer_report(&records, &analyses, col_names, format, &mut f)
                            .map_err(|e| anyhow::anyhow!(e))?;
                        f.flush()?;
                    }
                    _ => {
                        let stdout = std::io::stdout();
                        let mut h = stdout.lock();
                        hr::hmmer_report(&records, &analyses, col_names, format, &mut h)
                            .map_err(|e| anyhow::anyhow!(e))?;
                    }
                }
            }
        }
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
