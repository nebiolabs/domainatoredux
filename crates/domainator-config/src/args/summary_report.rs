use super::CommonArgs;

/// From a domainator-annotated genbank file, calculates contig stats and domain
/// frequency stats (upstream `summary_report.py`).
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct SummaryReportArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// name of input genbank files. Use - to read standard input.
    #[arg(short, long, required = true, num_args = 1..)]
    pub input: Vec<String>,
    /// Write the report as text to this file.
    #[arg(short = 'o', long)]
    pub text_out: Option<String>,
    /// Write the report as html to this file.
    #[arg(long)]
    pub html_out: Option<String>,
    /// Write the full summary as a single compact JSON object to this file.
    #[arg(long)]
    pub json_out: Option<String>,
    /// Write a tab-separated table of domain frequencies to this file. Headers are:
    /// domain, database, count, description, avg score.
    #[arg(long)]
    pub domains_tsv_out: Option<String>,
    /// Write a dense text matrix of domain co-occurrence frequencies to this file.
    /// Every row is a domain, the values are the fraction of contigs containing the
    /// row-domain that also include the column-domain (so the matrix is asymmetric).
    #[arg(long)]
    pub co_occurrence_fraction_out: Option<String>,
    /// Write a dense text matrix of domain co-occurrence frequencies to this file.
    /// Every row is a domain, the values are the number of contigs containing both the
    /// row-domain and the column-domain (so the matrix is symmetric).
    #[arg(long)]
    pub co_occurrence_count_out: Option<String>,
    /// Write a summary of the taxonomic origins of the contigs.
    #[arg(long)]
    pub taxonomy: bool,
    /// Path to an NCBI taxonomy database: a directory of extracted .dmp files, or a
    /// taxdump.tar.gz. Default: /tmp/ncbi_taxonomy_USER, USER being your user name.
    /// Unlike upstream, it is never downloaded.
    #[arg(long)]
    pub ncbi_taxonomy_path: Option<String>,
    /// Only consider contigs with these names.
    #[arg(long, num_args = 1..)]
    pub contigs: Option<Vec<String>>,
    /// text file containing names of contigs to select.
    #[arg(long)]
    pub contigs_file: Option<String>,
    /// Report how often each other domain occurs on the same contigs as these domains.
    #[arg(long, num_args = 1..)]
    pub domains: Option<Vec<String>>,
    /// text file containing names of domains to report co-occurrence for.
    #[arg(long)]
    pub domains_file: Option<String>,
    /// Ignore domain annotations not from these databases. default: consider all
    /// databases.
    #[arg(long, num_args = 1..)]
    pub databases: Option<Vec<String>>,
    /// Which CDSs/proteins to count, by fragment status. include: all. exclude: skip
    /// fragments. only: only fragments. A CDS is a fragment if its location has '<' or
    /// '>', or it has a /partial qualifier. A protein record is a fragment if it has a
    /// UniProt ' (Fragment)' or MGnify 'FL=0' header, a 'Flags: Fragment' definition
    /// or NON_TER features (UniProt text entries), or a CDS/Protein feature with '<' or
    /// '>' spanning it. Protein records that are filtered out are skipped; on
    /// nucleotide contigs, filtered CDSs and their domains are not counted, but the
    /// contig is.
    #[arg(long, default_value = "include", value_parser = ["include", "exclude", "only"])]
    pub partial: String,
}
