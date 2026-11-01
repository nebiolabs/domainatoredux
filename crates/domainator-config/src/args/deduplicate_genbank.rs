use super::CommonArgs;

/// Reduces redundancy by clustering sequences with cd-hit, usearch or diamond, or by
/// exact hashes, keeping one representative per cluster (upstream
/// `deduplicate_genbank.py`).
#[derive(clap::Args, Clone, Debug, Default)]
#[command(
    rename_all = "snake_case",
    group(clap::ArgGroup::new("count").args(["prefix_count", "suffix_count"]).multiple(false))
)]
pub struct DeduplicateGenbankArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// GenBank or FASTA files.
    #[arg(short, long, required = true, num_args = 1..)]
    pub input: Vec<String>,
    /// Write the records to this GenBank file. Use - for standard output.
    #[arg(short = 'o', long)]
    pub genbank_out: Option<String>,
    /// Write the records to this FASTA file. Use - for standard output.
    #[arg(long)]
    pub fasta_out: Option<String>,
    /// Write the log to this file. If not supplied, writes it to stderr.
    #[arg(long)]
    pub log_out: Option<String>,
    /// Whether the sequences in FASTA inputs are protein or nucleotide. GenBank records
    /// declare their own molecule type.
    #[arg(long, default_value = "protein", value_parser = ["nucleotide", "protein"])]
    pub alphabet: String,
    /// Which clustering algorithm to use.
    #[arg(long, default_value = "cd-hit", value_parser = ["cd-hit", "usearch", "hash", "diamond"])]
    pub algorithm: String,
    /// Path to the cd-hit executable, if it is not on the PATH. Here that is the
    /// `cdhit` program (a Rust port of cd-hit), or a cd-hit binary itself.
    #[arg(long)]
    pub cdhit_path: Option<String>,
    /// Path to the cd-hit-est executable, if it is not on the PATH.
    #[arg(long)]
    pub cdhit_est_path: Option<String>,
    /// Path to the usearch executable, if it is not on the PATH.
    #[arg(long)]
    pub usearch_path: Option<String>,
    /// Path to the diamond executable, if it is not on the PATH.
    #[arg(long)]
    pub diamond_path: Option<String>,
    /// Identity threshold (between 0 and 1), passed to cd-hit as the -c parameter, to
    /// usearch as the -id parameter, or to diamond as --approx-id after converting to
    /// percent.
    #[arg(long, visible_alias = "id", required = true)]
    pub identity: crate::types::Finite,
    /// In the output genbank file, will prepend X- to the names of contigs, where X is
    /// the number of contigs from the original file collapsed into this centroid.
    #[arg(long)]
    pub prefix_count: bool,
    /// In the output genbank file, will append -X to the names of contigs, where X is
    /// the number of contigs from the original file collapsed into this centroid.
    #[arg(long)]
    pub suffix_count: bool,
    /// The number of threads to use for deduplication. 0: all cores.
    #[arg(long, default_value_t = 0, allow_negative_numbers = true)]
    pub cpu: i64,
    /// Extra options for cd-hit or cd-hit-est, as one quoted string split like a shell
    /// command line. Use the = form: --cdhit_args="-s 0.9". Options that domainatoredux
    /// sets itself are refused.
    #[arg(long, allow_hyphen_values = true)]
    pub cdhit_args: Option<String>,
    /// Extra options for usearch, as one quoted string split like a shell command line.
    /// Use the = form: --usearch_args="-s 0.9". Options that domainatoredux sets itself
    /// are refused.
    #[arg(long, allow_hyphen_values = true)]
    pub usearch_args: Option<String>,
    /// Extra options for diamond cluster, as one quoted string split like a shell
    /// command line. Use the = form: --diamond_args="-s 0.9". Options that
    /// domainatoredux sets itself are refused.
    #[arg(long, allow_hyphen_values = true)]
    pub diamond_args: Option<String>,
    /// When set for nucleotide inputs, consider both strands for clustering/hash
    /// deduplication.
    #[arg(long)]
    pub both_strands: bool,
    /// If supplied, then write a tab separated table with columns: representative,
    /// contigs.
    #[arg(long)]
    pub cluster_tsv_out: Option<String>,
    /// Separator to separate individual contig names in the second column of the
    /// cluster table.
    #[arg(long, default_value = " ; ", allow_hyphen_values = true)]
    pub cluster_sep: String,
}
