use super::{CommonArgs, TaxonomyArgs};
use crate::types::{Finite, PositiveFloat};

/// Extracts the regions of proteins not covered by any domain annotation (upstream
/// `extract_unannotated.py`).
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct ExtractUnannotatedArgs {
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
    /// Only read input protein records with these IDs; restricts where extraction
    /// occurs. Additive with --contigs_file.
    #[arg(long, num_args = 1..)]
    pub contigs: Vec<String>,
    /// Only read input protein records with the IDs in this file: a plain-text file
    /// (one per line), FASTA/GenBank record IDs, or HMM profile names. Additive with
    /// --contigs.
    #[arg(long)]
    pub contigs_file: Option<String>,
    /// Count only domain/search-hit annotations with e-value <= this when finding
    /// unannotated gaps. Default: no cutoff.
    #[arg(short, long)]
    pub evalue: Option<PositiveFloat>,
    /// Count only domain/search-hit annotations with score >= this when finding
    /// unannotated gaps. Raising this can enlarge the gaps. Default: no cutoff.
    #[arg(long, allow_negative_numbers = true)]
    pub min_annotation_score: Option<Finite>,
    /// Only consider domains from these databases. default: all databases.
    #[arg(long, num_args = 1..)]
    pub databases: Option<Vec<String>>,
    /// Which annotations count as annotated. domainator: domain annotations (from
    /// domainate). domain_search: best-hit annotations (from domain_search). all: both.
    #[arg(long, default_value = "domainator",
          value_parser = ["domainator", "domain_search", "all"])]
    pub annotation_type: String,
    /// Has no effect: this tool supports only protein sequences and never
    /// reverse-complements them. Retained for compatibility with extract_domains.
    #[arg(long)]
    pub keep_direction: bool,
    /// Use the parent protein ID instead of a coordinate-based region ID. This can
    /// produce duplicate IDs for different extracted sequences.
    #[arg(long)]
    pub keep_parent_id: bool,
    /// Extract only the largest unannotated region from each protein, providing it
    /// passes the length cutoff specified by --min_region_length.
    #[arg(long)]
    pub largest: bool,
    /// Extract only unannotated gaps of at least this many amino acids, before choosing
    /// --largest. Not a cutoff on the input protein length. Default: no cutoff.
    #[arg(long, allow_negative_numbers = true)]
    pub min_region_length: Option<i64>,
}
