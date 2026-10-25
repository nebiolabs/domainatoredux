use super::CommonArgs;
use crate::types::{Finite, PositiveFloat};

/// Filters domain annotations by e-value, overlap, database and name (upstream
/// `filter_domains.py`).
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct FilterDomainsArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// The genbank filenames to be filtered. Use - to read standard input.
    #[arg(short, long, required = true, num_args = 1..)]
    pub input: Vec<String>,
    /// Write the records to this GenBank file. Use - for standard output.
    #[arg(short = 'o', long)]
    pub genbank_out: Option<String>,
    /// Write the records to this FASTA file. Use - for standard output.
    #[arg(long)]
    pub fasta_out: Option<String>,
    /// Threshold (max) E value for the domain hit. Default: no cutoff.
    #[arg(short, long)]
    pub evalue: Option<PositiveFloat>,
    /// The maximum fraction of overlap between domains to be included in the annotated
    /// genbank.
    #[arg(long, default_value = "1.0", allow_negative_numbers = true)]
    pub max_overlap: Finite,
    /// Only annotate contigs with ids in this list. Additive with --contigs_file.
    /// default: annotate all contigs.
    #[arg(long, num_args = 1..)]
    pub contigs: Vec<String>,
    /// Only annotate contigs with ids listed in this file (one per line). Additive
    /// with --contigs. default: annotate all contigs.
    #[arg(long)]
    pub contigs_file: Option<String>,
    /// Remove all domains NOT matching these names.
    #[arg(long, num_args = 1..)]
    pub domains: Option<Vec<String>>,
    /// Text file containing names of domains to include. (all domains not matching
    /// this list will be removed).
    #[arg(long)]
    pub domains_file: Option<String>,
    /// Remove annotations from all databases except these. Applied before other
    /// filters, such as domains, evalue, and max_overlap. By default will keep all
    /// annotations from all databases.
    #[arg(long, num_args = 1..)]
    pub keep_databases: Option<Vec<String>>,
    /// Only filter annotations from these databases, according to evalue, domains, and
    /// max_overlap; annotations from other databases are left as they are. By default,
    /// annotations from all databases are filtered.
    #[arg(long, num_args = 1..)]
    pub databases: Option<Vec<String>>,
    /// Which type of annotations to filter (domainator, domain_search, all).
    #[arg(long, default_value = "all", value_parser = ["domainator", "domain_search", "all"])]
    pub annotation_type: String,
}
