use super::{CommonArgs, TaxonomyArgs};
use crate::types::{Finite, PositiveFloat};

/// Extracts annotated domains as their own records (upstream `extract_domains.py`).
/// Output keeps the input's sequence type: nucleotide input gives nucleotide regions,
/// not peptides. --combine and --splice work across the selected annotations of each
/// record, not per CDS.
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct ExtractDomainsArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// Genbank filenames. Use - to read standard input.
    #[arg(short, long, required = true, num_args = 1..)]
    pub input: Vec<String>,
    /// Write the records to this GenBank file. Use - for standard output.
    #[arg(short = 'o', long)]
    pub genbank_out: Option<String>,
    /// Write the records to this FASTA file. Use - for standard output.
    #[arg(long)]
    pub fasta_out: Option<String>,
    #[command(flatten)]
    pub taxonomy: TaxonomyArgs,
    /// Only read input records with these IDs; restricts where extraction occurs.
    /// Additive with --contigs_file.
    #[arg(long, num_args = 1..)]
    pub contigs: Vec<String>,
    /// Only read input records with the IDs in this file: a plain-text file (one per
    /// line), FASTA/GenBank record IDs, or HMM profile names. Additive with --contigs.
    #[arg(long)]
    pub contigs_file: Option<String>,
    /// Extract only domain/search-hit annotations with e-value <= this, before
    /// extension, combination, or splicing. Default: no cutoff.
    #[arg(short, long)]
    pub evalue: Option<PositiveFloat>,
    /// Extract only domain/search-hit annotations with score >= this, before extension,
    /// combination, or splicing. Default: no cutoff.
    #[arg(long, allow_negative_numbers = true)]
    pub min_score: Option<Finite>,
    /// Extract annotations with these domain names; search hits can also be selected by
    /// --has_search_hit. Additive with --domains_file.
    #[arg(long, num_args = 1..)]
    pub domains: Option<Vec<String>>,
    /// Extract annotations with the domain names in this file: a plain-text file (one
    /// per line), FASTA/GenBank record IDs, or HMM profile names. Additive with
    /// --domains.
    #[arg(long)]
    pub domains_file: Option<String>,
    /// Domain databases to use for domain extraction. default: all databases.
    #[arg(long, num_args = 1..)]
    pub databases: Option<Vec<String>>,
    /// Which annotations to extract. domainator: domain annotations (from
    /// domainate). domain_search: best-hit annotations (from domain_search). all: both.
    #[arg(long, default_value = "domainator",
          value_parser = ["domainator", "domain_search", "all"])]
    pub annotation_type: String,
    /// Also select existing domain_search hits, regardless of annotation_type, domain
    /// names, domain expressions, or databases. Hits must pass the e-value and score
    /// cutoffs where applicable. Participates in --invert where available.
    #[arg(long)]
    pub has_search_hit: bool,
    /// Which domains to extract, by the CDS or protein they are on, by fragment status.
    /// include: all. exclude: skip fragments. only: only fragments. A CDS is a fragment
    /// if its location has '<' or '>', or it has a /partial qualifier. A protein record
    /// is a fragment if it has a UniProt ' (Fragment)' or MGnify 'FL=0' header, a
    /// 'Flags: Fragment' definition or NON_TER features (UniProt text entries), or a
    /// CDS/Protein feature with '<' or '>' spanning it. Contig-level nucleotide hits
    /// are never on fragments.
    #[arg(long, default_value = "include", value_parser = ["include", "exclude", "only"])]
    pub partial: String,
    /// Extend each side by this many amino acids for protein input or bases for
    /// nucleotide input. Not rounded to codons or restricted to CDS boundaries. Sets
    /// the default of --extend_up and --extend_down.
    #[arg(long, allow_negative_numbers = true)]
    pub extend_range: Option<i64>,
    /// Extend upstream by this many amino acids for protein input or bases for
    /// nucleotide input. With --combine, extends the enclosing span. Default:
    /// --extend_range, or 0.
    #[arg(long, allow_negative_numbers = true)]
    pub extend_up: Option<i64>,
    /// Extend downstream by this many amino acids for protein input or bases for
    /// nucleotide input. With --combine, extends the enclosing span. Default:
    /// --extend_range, or 0.
    #[arg(long, allow_negative_numbers = true)]
    pub extend_down: Option<i64>,
    /// Extract one enclosing span of all selected annotations per input record,
    /// including intervening sequence. Not grouped by CDS; may span different genes or
    /// strands. Does not guarantee a coding sequence. Rejects circular records. By
    /// default, extracts each annotation separately. Cannot be used with --splice.
    #[arg(long)]
    pub combine: bool,
    /// If set, then when multiple target domains occur in the same contig, the
    /// intermediate sequence will be deleted and the extracted domains from the contig
    /// glued together, in the order they are read on the strand of the domain with the
    /// lowest start coordinate. Applies across selected annotations per input record,
    /// not per CDS; does not guarantee biological exon splicing or a coding sequence.
    /// By default, each hit domain will be extracted separately. Cannot be used with
    /// --combine.
    #[arg(long)]
    pub splice: bool,
    /// By default extracted regions will be flipped so that the domain (or the domain
    /// with the lowest start coordinate when --combine or --splice is active) is on the
    /// forward strand. Set this option to not reverse complement anything. In the rare
    /// case of a split domain with parts on different strands, the strand of the first
    /// part will be considered.
    #[arg(long)]
    pub keep_direction: bool,
    /// Use the parent nucleotide or protein record ID instead of a coordinate-based
    /// region ID. This can produce duplicate IDs for different extracted sequences.
    #[arg(long)]
    pub keep_parent_id: bool,
}
