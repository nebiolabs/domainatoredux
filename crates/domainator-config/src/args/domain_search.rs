use super::{CommonArgs, TaxonomyArgs};
use crate::types::{Finite, NonNegFloat, PositiveFloat};

/// Searches a large database with a few references and extracts the hits (upstream
/// `domain_search.py`).
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct DomainSearchArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// The genbank or fasta files to search. Genbank files can be nucleotide (with
    /// CDS annotations) or peptide. Fasta files can be protein or nucleotide; use
    /// --alphabet to specify which.
    #[arg(short, long, required = true, num_args = 1..)]
    pub input: Vec<String>,
    /// Whether the sequences in FASTA inputs are protein or nucleotide. GenBank records
    /// declare their own molecule type.
    #[arg(long, default_value = "protein", value_parser = ["nucleotide", "protein"])]
    pub alphabet: String,
    /// References to search for in the --input records. Supported types are protein HMMs, protein
    /// FASTA, nucleotide FASTA and nucleotide HMMs (not Infernal CM files).
    #[arg(short, long, required = true, num_args = 1..)]
    pub reference: Vec<String>,
    /// Write the hits to this GenBank file. A name ending in .gz/.bgz is written as
    /// BGZF, which a later step can read as a random-access, partitionable input. Use
    /// - for standard output.
    #[arg(short = 'o', long)]
    pub genbank_out: Option<String>,
    /// Write the hits to this FASTA file. Use - for standard output.
    #[arg(long)]
    pub fasta_out: Option<String>,
    /// Passed as the -Z parameter to hmmsearch: Assert that the total number of targets
    /// in your searches is `<x>`, for the purposes of per-sequence E-value calculations,
    /// rather than the actual number of targets seen. Set to 0 to use the number of
    /// target sequences in the input, counted before any taxonomy filter, so filtering
    /// does not change E-values. Supplying -Z is recommended for most use cases of
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
    /// Run hmmsearch, phmmer, or nhmmer in maximum sensitivity mode, which is much
    /// slower, but more sensitive.
    #[arg(long)]
    pub max_mode: bool,
    /// The maximum number of CDSs returned by the search, keeping the best-scoring
    /// ones by the bitscore of their best scoring profile. If not supplied, return all
    /// hits passing the evalue threshold.
    #[arg(long, allow_negative_numbers = true)]
    pub max_hits: Option<i64>,
    /// The maximum number of independent hits per contig. For nucleotide contig-level
    /// hits, each non-overlapping hit region is treated as an independent hit.
    /// [default: unlimited]
    #[arg(long, allow_negative_numbers = true)]
    pub max_hits_per_contig: Option<i64>,
    #[command(flatten)]
    pub taxonomy: TaxonomyArgs,
    /// Which CDSs/proteins to search, by fragment status. include: all. exclude: skip
    /// fragments. only: only fragments. A CDS is a fragment if its location has '<' or
    /// '>', or it has a /partial qualifier. A protein record is a fragment if it has a
    /// UniProt ' (Fragment)' or MGnify 'FL=0' header, a 'Flags: Fragment' definition or
    /// NON_TER features (UniProt text entries), or a CDS/Protein feature with '<' or
    /// '>' spanning it.
    #[arg(long, default_value = "include", value_parser = ["include", "exclude", "only"])]
    pub partial: String,
    /// Names of decoy domains. A decoy domain is a domain that is not expected to be
    /// found in the input sequences. If the best hit for a CDS/protein is a domain from
    /// the decoys list, it will be not be returned as a hit.
    #[arg(long, num_args = 1..)]
    pub decoys: Option<Vec<String>>,
    /// Text file containing names of decoy domains.
    #[arg(long)]
    pub decoys_file: Option<String>,
    /// The number of cores of the cpu which are used at a time to run the search (0:
    /// all cores).
    #[arg(long, default_value_t = 0, allow_negative_numbers = true)]
    pub cpu: i64,
    /// The maximum fractional of overlap between domains to be included in the
    /// annotated genbank. If >= 1, then no overlap filtering will be done.
    #[arg(long, default_value = "1", allow_negative_numbers = true)]
    pub max_overlap: Finite,
    /// Add domain annotations (Domainator features) for retained hits, in addition to
    /// the search best-hit annotations (Domain_Search features) already present.
    #[arg(long)]
    pub add_annotations: bool,
    /// Output uniqueness policy: none keeps all records; id drops repeated output IDs;
    /// sequence drops identical output sequences (ignoring case and terminal '*');
    /// sequence_rc also collapses nucleotide reverse complements. The same ID must
    /// identify the same sequence, so sequence uniqueness also ensures unique IDs.
    /// Deduplication happens before padding.
    #[arg(long, default_value = "none", value_parser = ["none", "id", "sequence", "sequence_rc"])]
    pub dedup: String,
    /// The maximum fractional of overlap between any two output regions. If >= 1, then
    /// no overlap filtering will be done. Regions are output in a greedy fashion based
    /// on CDS start site. New regions are output if less than this fraction of them
    /// overlaps with any previously output region.
    #[arg(long, default_value = "1.0", allow_negative_numbers = true)]
    pub max_region_overlap: Finite,
    /// By default, nucleotide inputs return nucleotide regions. When --translate is
    /// set, CDS hits are translated before writing. Whole-contig nucleic-acid hits are
    /// not translated. Note that --translate is incompatible with neighborhood
    /// extraction.
    #[arg(long)]
    pub translate: bool,
    /// How many kb upstream and downstream of the selected CDS to extract. Starting
    /// from the ends of the CDS.
    #[arg(long, allow_negative_numbers = true)]
    pub kb_range: Option<Finite>,
    /// How many kb upstream of the selected CDS to extract. Starting from the end of
    /// the CDS.
    #[arg(long, allow_negative_numbers = true)]
    pub kb_up: Option<Finite>,
    /// How many kb downstream of the selected CDS to extract. Starting from the end of
    /// the CDS.
    #[arg(long, allow_negative_numbers = true)]
    pub kb_down: Option<Finite>,
    /// How many CDSs upstream and downstream of the selected CDS to extract. Example,
    /// 1 would mean to return contigs including the selected CDSs and the immediate
    /// upstream and downstream CDSs. Only protein-coding CDSs are counted, never hits
    /// on a nucleotide reference.
    #[arg(long, allow_negative_numbers = true)]
    pub cds_range: Option<i64>,
    /// How many CDSs upstream of the selected CDS to extract. Example, 1 would mean to
    /// return contigs including the selected CDSs and the immediate upstream CDS.
    #[arg(long, allow_negative_numbers = true)]
    pub cds_up: Option<i64>,
    /// How many CDSs downstream of the selected CDS to extract. Example, 1 would mean
    /// to return contigs including the selected CDSs and the immediate downstream CDS.
    #[arg(long, allow_negative_numbers = true)]
    pub cds_down: Option<i64>,
    /// If set, then return the entire matching contigs. Circular contigs stay
    /// circular, and are rotated so that the hit is in the center.
    #[arg(long)]
    pub whole_contig: bool,
    /// Only extract regions around CDSs on the specified strand.
    #[arg(long, value_parser = ["f", "r"])]
    pub strand: Option<String>,
    /// If set, then ends of sequences will be padded so that the focus CDS aligns.
    /// See --anchor.
    #[arg(long)]
    pub pad: bool,
    /// Where to align the focus CDSs when --pad is set, relative to the focus CDS in
    /// its own orientation. start: align the first base of the focus CDSs. end: align
    /// their last base. center: align their midpoints.
    #[arg(long, default_value = "start", value_parser = ["start", "end", "center"])]
    pub anchor: String,
    /// By default extracted regions will be flipped so that the focus cds is on the
    /// forward strand. Setting this option will keep the focus cds on whatever strand
    /// it started on.
    #[arg(long)]
    pub keep_direction: bool,
    /// Approximately how many target sequences to search at one time in a batch. This
    /// also controls partition size: each parallel task processes about this many
    /// CDSs/sequences. Larger values mean fewer, bigger partitions (lower per-task
    /// overhead, but coarser load balancing); smaller values give finer parallelism at
    /// the cost of more tasks.
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
