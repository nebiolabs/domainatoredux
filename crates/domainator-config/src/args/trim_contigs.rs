use super::CommonArgs;
use crate::types::{Finite, PositiveFloat};

/// Trims the ends of contigs by kb, by CDS count, or past end CDSs with (or without)
/// chosen domains (upstream `trim_contigs.py`). Circular contigs are linearized
/// first. CDS and domain options do not apply to protein records.
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct TrimContigsArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// GenBank files. Use - to read standard input.
    #[arg(short, long, required = true, num_args = 1..)]
    pub input: Vec<String>,
    /// Write the records to this GenBank file. Use - for standard output.
    #[arg(short = 'o', long)]
    pub genbank_out: Option<String>,
    /// Write the records to this FASTA file. Use - for standard output.
    #[arg(long)]
    pub fasta_out: Option<String>,
    /// How many kb to trim from both sides of each contig.
    #[arg(long, allow_negative_numbers = true)]
    pub kb_range: Option<Finite>,
    /// How many kb to trim from the left side of each contig.
    #[arg(long, allow_negative_numbers = true)]
    pub kb_left: Option<Finite>,
    /// How many kb to trim from the right side of each contig.
    #[arg(long, allow_negative_numbers = true)]
    pub kb_right: Option<Finite>,
    /// How many CDSs to trim from both sides of each contig.
    #[arg(long, allow_negative_numbers = true)]
    pub cds_range: Option<i64>,
    /// How many CDSs to trim from the left side of each contig.
    #[arg(long, allow_negative_numbers = true)]
    pub cds_left: Option<i64>,
    /// How many CDSs to trim from the right side of each contig.
    #[arg(long, allow_negative_numbers = true)]
    pub cds_right: Option<i64>,
    /// Only return contigs with these names.
    #[arg(long, num_args = 1..)]
    pub contigs: Option<Vec<String>>,
    /// Text file containing names of contigs to return.
    #[arg(long)]
    pub contigs_file: Option<String>,
    /// Trim CDSs containing domains with these names from the ends of each contig.
    #[arg(long, num_args = 1..)]
    pub domains: Option<Vec<String>>,
    /// Trim CDSs containing no domain annotations from the ends of each contig.
    #[arg(long)]
    pub unannotated: bool,
    /// Text file containing names of domains to trim contigs containing.
    #[arg(long)]
    pub domains_file: Option<String>,
    /// A boolean expression using operators & (AND), | (OR), and ~ (NOT), to specify a
    /// desired combination of domains for CDSs to trim.
    #[arg(long)]
    pub domain_expr: Option<String>,
    /// The evalue cutoff (max) for domain annotations to be considered when trimming by
    /// domain. Default: no cutoff.
    #[arg(short, long)]
    pub evalue: Option<PositiveFloat>,
    /// Only consider domain annotations from these databases when trimming by domain.
    #[arg(long, num_args = 1..)]
    pub databases: Option<Vec<String>>,
}
