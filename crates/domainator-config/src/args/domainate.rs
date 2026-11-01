use super::CommonArgs;
use crate::types::{Finite, NonNegFloat, PositiveFloat};

/// Annotates contigs or proteins with hits to HMM or sequence references (upstream
/// `domainate.py`). Every input record is kept, with or without hits; domain_search
/// selects the matching records or hit neighborhoods instead.
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct DomainateArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// The genbank or fasta files to annotate. Genbank files can be nucleotide (with
    /// CDS annotations) or peptide. Fasta files can be protein or nucleotide; use
    /// --alphabet to specify which.
    #[arg(short, long, required = true, num_args = 1..)]
    pub input: Vec<String>,
    /// Whether the sequences in FASTA inputs are protein or nucleotide. GenBank records
    /// declare their own molecule type.
    #[arg(long, default_value = "protein", value_parser = ["nucleotide", "protein"])]
    pub alphabet: String,
    /// References to search for in the --input records. Supported types are protein HMMs,
    /// protein FASTA, nucleotide FASTA and nucleotide HMMs (not Infernal CM files).
    #[arg(short, long, num_args = 1..)]
    pub reference: Option<Vec<String>>,
    /// Write the records to this GenBank file. Use - for standard output.
    #[arg(short = 'o', long)]
    pub genbank_out: Option<String>,
    /// Write the records to this FASTA file. Use - for standard output.
    #[arg(long)]
    pub fasta_out: Option<String>,
    /// Passed as the -Z parameter to hmmsearch: Assert that the total number of targets
    /// in your searches is `<x>`, for the purposes of per-sequence E-value calculations,
    /// rather than the actual number of targets seen. Set to 0 to use the number of
    /// target sequences in the input. Supplying -Z is recommended for most use cases of
    /// domainate, as it will speed up the search and make comparisons between searches
    /// more meaningful.
    #[arg(
        short = 'Z',
        id = "Z",
        value_name = "Z",
        default_value_t = 1000,
        allow_negative_numbers = true
    )]
    pub z: i64,
    /// Threshold E value for the domain hit. Must be <=10.
    #[arg(short, long, default_value = "0.001")]
    pub evalue: PositiveFloat,
    /// Hits with E value LOWER (as in BETTER) than this will be filtered out. NOT
    /// FREQUENTLY USED. Use only if you want to eliminate close matches. Must be >=0.
    #[arg(long, default_value = "0.0")]
    pub min_evalue: NonNegFloat,
    /// The maximum number of domains to be reported per CDS. If not specified, then all
    /// hits are reported.
    #[arg(long, allow_negative_numbers = true)]
    pub max_hits_per_cds: Option<i64>,
    /// Run hmmsearch, phmmer, or nhmmer in maximum sensitivity mode, which is much
    /// slower, but more sensitive.
    #[arg(long)]
    pub max_mode: bool,
    /// Which CDSs/proteins to search, by fragment status. include: all. exclude: skip
    /// fragments. only: only fragments. A CDS is a fragment if its location has '<' or
    /// '>', or it has a /partial qualifier. A protein record is a fragment if it has a
    /// UniProt ' (Fragment)' or MGnify 'FL=0' header, a 'Flags: Fragment' definition or
    /// NON_TER features (UniProt text entries), or a CDS/Protein feature with '<' or
    /// '>' spanning it.
    #[arg(long, default_value = "include", value_parser = ["include", "exclude", "only"])]
    pub partial: String,
    /// The number of cores of the cpu which are used at a time to run the search (0:
    /// all cores).
    #[arg(long, default_value_t = 0, allow_negative_numbers = true)]
    pub cpu: i64,
    /// The maximum fractional of overlap between domains to be included in the
    /// annotated genbank. If >= 1, then no overlap filtering will be done.
    #[arg(long, default_value = "1", allow_negative_numbers = true)]
    pub max_overlap: Finite,
    /// If activated, then overlap filtering will be done by database, rather than all
    /// together.
    #[arg(long)]
    pub overlap_by_db: bool,
    /// How many target sequences to search at one time in a batch. Accepted for
    /// compatibility with upstream; domainatoredux searches all of them at once.
    #[arg(long, default_value_t = 10000, allow_negative_numbers = true)]
    pub batch_size: i64,
    /// When activated, new CDS annotations will be added with Orphos (Prodigal) in
    /// metagenomic mode. If 'all', then any existing CDS annotations will be deleted
    /// and all contigs will be re-annotated. If 'unannotated', then only contigs
    /// without CDS annotations will be annotated. If you supply a nucleotide fasta
    /// file, be sure to also specify --alphabet nucleotide. -Z 0 can't be used with
    /// gene calling, because the number of target sequences isn't known until the
    /// genes are called.
    #[arg(long, value_parser = ["all", "unannotated"])]
    pub gene_call: Option<String>,
    /// Fail if a projected output file would exceed this many GB. Set to 0 to disable
    /// the guardrail.
    #[arg(long, default_value = "25.0", allow_negative_numbers = true)]
    pub max_output_gb: Finite,
}
