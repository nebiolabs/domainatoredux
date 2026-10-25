use super::CommonArgs;

/// Sorts contigs by their best Domain_Search score, or by their best Domainator score
/// among chosen domains (upstream `sort_contigs.py`).
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct SortContigsArgs {
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
    /// Maximum number of contigs to return, keeping the best-scoring ones. If not
    /// supplied, return all contigs.
    #[arg(long, allow_negative_numbers = true)]
    pub max_hits: Option<i64>,
    /// If supplied, sort by the best Domainator annotation score from domains with
    /// these names instead of Domain_Search annotation scores. Additive with
    /// --domains_file.
    #[arg(long, num_args = 1..)]
    pub domains: Option<Vec<String>>,
    /// Like --domains, but read the domain names from this file, one per line.
    /// Additive with --domains.
    #[arg(long)]
    pub domains_file: Option<String>,
    /// When used with --domains, only consider Domainator annotations from these
    /// databases.
    #[arg(long, num_args = 1..)]
    pub databases: Option<Vec<String>>,
}
