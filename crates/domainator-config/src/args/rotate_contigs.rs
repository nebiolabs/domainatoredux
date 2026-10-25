use super::CommonArgs;
use crate::types::PositiveFloat;

/// Rotates circular contigs so that their origin is set relative to a domain
/// annotation (upstream `rotate_contigs.py`). The domain is the best of the new hits of
/// a reference search and the pre-existing annotations named by --domains or
/// --annotation_type domain_search: the highest score, then the lowest E-value, then
/// the lowest position.
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct RotateContigsArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// Names of input genbank or fasta files. Must be nucleotide sequences.
    #[arg(short, long, required = true, num_args = 1..)]
    pub input: Vec<String>,
    /// Write the records to this GenBank file. Use - for standard output.
    #[arg(short = 'o', long)]
    pub genbank_out: Option<String>,
    /// Write the records to this FASTA file. Use - for standard output.
    #[arg(long)]
    pub fasta_out: Option<String>,
    /// Reference files to search against the input, to find new domains to rotate on.
    /// Supported types are protein HMMs, protein FASTA, nucleotide FASTA and nucleotide
    /// HMMs (not Infernal CM files).
    #[arg(short, long, num_args = 1..)]
    pub reference: Option<Vec<String>>,
    /// Passed as the -Z parameter to hmmsearch: Assert that the total number of targets
    /// in your searches is `<x>`, for the purposes of per-sequence E-value calculations,
    /// rather than the actual number of targets seen. Set to 0 to use the number of
    /// actual target sequences in the input files (not available when reading from
    /// stdin).
    #[arg(
        short = 'Z',
        id = "Z",
        value_name = "Z",
        default_value_t = 1000,
        allow_negative_numbers = true
    )]
    pub z: i64,
    /// Threshold E value for domains to rotate on. Both new and pre-existing
    /// annotations must pass this threshold. Must be <=10.
    #[arg(short, long, default_value = "0.001")]
    pub evalue: PositiveFloat,
    /// Rotate on pre-existing domainator annotations with these names.
    #[arg(long, num_args = 1..)]
    pub domains: Option<Vec<String>>,
    /// Text file containing names of pre-existing domainator annotations to rotate on.
    #[arg(long)]
    pub domains_file: Option<String>,
    /// Consider only pre-existing annotations from these databases. default: all
    /// databases.
    #[arg(long, num_args = 1..)]
    pub databases: Option<Vec<String>>,
    /// Which pre-existing annotations to rotate on. domainator: domain annotations
    /// (from domainate). domain_search: best-hit annotations (from domain_search).
    #[arg(long, default_value = "domainator", value_parser = ["domainator", "domain_search"])]
    pub annotation_type: String,
    /// Where to put the origin, relative to the chosen domain in its own orientation
    /// (not relative to the original orientation of the contig). start: the first base
    /// of the domain becomes the first base of the contig. end: the last base of the
    /// domain becomes the last base of the contig. center: the origin is placed in the
    /// middle of the domain.
    #[arg(long, default_value = "start", value_parser = ["start", "end", "center"])]
    pub anchor: String,
    /// By default, contigs are reverse complemented if necessary to put the chosen
    /// domain on the forward strand. Setting this option will keep the original
    /// orientation of the contig.
    #[arg(long)]
    pub keep_direction: bool,
    /// Treat fasta records and linear genbank records as circular, before searching and
    /// rotating. By default, they are skipped.
    #[arg(long)]
    pub circularize: bool,
    /// By default, rotated contigs will be renamed to note the original coordinates of
    /// their first and last bases, and whether they were reverse complemented. Set this
    /// option to keep contigs with their original names.
    #[arg(long)]
    pub keep_name: bool,
    /// By default, new annotations from the reference search are not written to the
    /// output. Setting this option will keep the new annotation of the domain that the
    /// contig was rotated on. Has no effect if the contig was rotated on a pre-existing
    /// annotation.
    #[arg(long)]
    pub keep_annotation: bool,
    /// When activated, new CDS annotations will be added with Orphos (Prodigal) in
    /// metagenomic mode before the search. If 'all', then any existing CDS and
    /// domainator annotations will be deleted and all contigs will be re-annotated. If
    /// 'unannotated', then only contigs without CDS annotations will be annotated.
    /// Needed to search protein references against contigs without CDS annotations,
    /// such as fasta records.
    #[arg(long, value_parser = ["all", "unannotated"])]
    pub gene_call: Option<String>,
    /// The number of cores of the cpu which are used at a time to run the search (0:
    /// one thread per physical core).
    #[arg(long, default_value_t = 0, allow_negative_numbers = true)]
    pub cpu: i64,
    /// Run hmmsearch, phmmer, or nhmmer in maximum sensitivity mode, which is much
    /// slower, but more sensitive.
    #[arg(long)]
    pub max_mode: bool,
    /// How many contigs to search at one time in a batch. Increasing this number might
    /// improve speeds at the cost of memory.
    #[arg(long, default_value_t = 1000, allow_negative_numbers = true)]
    pub batch_size: i64,
}
