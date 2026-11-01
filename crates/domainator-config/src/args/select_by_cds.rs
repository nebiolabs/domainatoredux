use super::{CommonArgs, TaxonomyArgs};
use crate::types::{Finite, PositiveFloat};

/// Extracts contigs, or the neighborhoods around CDSs selected by name or domain
/// content (upstream `select_by_cds.py`).
///
/// CDS names, domain names, domain expressions, unannotated, and search-hit presence
/// are OR selectors; with none, every eligible CDS is selected. Contig IDs, strand,
/// fragment status, and taxonomy are independent AND restrictions, applied to the
/// focus CDS. --invert negates only the combined selectors.
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct SelectByCdsArgs {
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
    #[command(flatten)]
    pub taxonomy: TaxonomyArgs,
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
    /// upstream and downstream CDSs.
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
    /// If set, then return the entire matching contigs.
    #[arg(long)]
    pub whole_contig: bool,
    /// Only read input records with these IDs; not inverted by --invert. Additive with
    /// --contigs_file.
    #[arg(long, num_args = 1..)]
    pub contigs: Option<Vec<String>>,
    /// Only read input records with the IDs in this file: a plain-text file (one per
    /// line), FASTA/GenBank record IDs, or HMM profile names. Additive with --contigs.
    #[arg(long)]
    pub contigs_file: Option<String>,
    /// Include CDSs with these names (additive with --domains).
    #[arg(long, num_args = 1..)]
    pub cds: Option<Vec<String>>,
    /// Text file containing names of CDSs to include.
    #[arg(long)]
    pub cds_file: Option<String>,
    /// CDS qualifier to prefer for name matching, falling back to gene_id, locus_tag,
    /// gene, then protein_id. Does not change coordinate-based region IDs.
    #[arg(long)]
    pub name_field: Option<String>,
    /// Select CDSs containing these domain names, OR with other CDS selectors.
    /// Additive with --domains_file.
    #[arg(long, num_args = 1..)]
    pub domains: Option<Vec<String>>,
    /// Select CDSs containing the domain names in this file: a plain-text file (one per
    /// line), FASTA/GenBank record IDs, or HMM profile names. Additive with --domains.
    #[arg(long)]
    pub domains_file: Option<String>,
    /// A boolean expression using operators & (AND), | (OR), and ~ (NOT), to specify a
    /// desired combination of domains.
    #[arg(long)]
    pub domain_expr: Option<String>,
    /// Consider only domains from these databases when filtering by domain. default:
    /// all databases.
    #[arg(long, num_args = 1..)]
    pub databases: Option<Vec<String>>,
    /// Only extract regions around CDSs on the specified strand.
    #[arg(long, value_parser = ["f", "r"])]
    pub strand: Option<String>,
    /// Which CDSs to extract regions around, by fragment status. include: all.
    /// exclude: skip fragments. only: only fragments. A CDS is a fragment if its
    /// location has '<' or '>', or it has a /partial qualifier. A protein record is a
    /// fragment if it has a UniProt ' (Fragment)' or MGnify 'FL=0' header, a 'Flags:
    /// Fragment' definition or NON_TER features (UniProt text entries), or a
    /// CDS/Protein feature with '<' or '>' spanning it. Not affected by --invert.
    #[arg(long, default_value = "include", value_parser = ["include", "exclude", "only"])]
    pub partial: String,
    /// Select CDSs with no domain annotations.
    #[arg(long)]
    pub unannotated: bool,
    /// Which annotations to select CDSs by. domainator: domain annotations (from
    /// domainate). domain_search: best-hit annotations (from domain_search). all: both.
    #[arg(long, default_value = "domainator",
          value_parser = ["domainator", "domain_search", "all"])]
    pub annotation_type: String,
    /// Also select existing domain_search hits, regardless of annotation_type, domain
    /// names, domain expressions, or databases. Hits must pass the e-value and score
    /// cutoffs where applicable. Participates in --invert where available.
    #[arg(long)]
    pub has_search_hit: bool,
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
    /// Output uniqueness policy: none keeps all records; id drops repeated output IDs;
    /// sequence drops identical output sequences (ignoring case and terminal '*');
    /// sequence_rc also collapses nucleotide reverse complements. The same ID must
    /// identify the same sequence, so sequence uniqueness also ensures unique IDs.
    /// Deduplication happens before padding.
    #[arg(long, default_value = "id", value_parser = ["none", "id", "sequence", "sequence_rc"])]
    pub dedup: String,
    /// The maximum fractional of overlap between any two output regions. If >= 1, then
    /// no overlap filtering will be done. Regions are output in a greedy fashion based
    /// on CDS start site. New regions are output if less than this fraction of them
    /// overlaps with any previously output region.
    #[arg(long, default_value = "1.0", allow_negative_numbers = true)]
    pub max_region_overlap: Finite,
    /// The evalue cutoff (max) for domain annotations to be considered when selecting
    /// CDSs. Default: no cutoff.
    #[arg(short, long)]
    pub evalue: Option<PositiveFloat>,
    /// Invert the combined CDS selectors, not contigs, strand, partial, or taxonomy.
    /// With no selectors, warn and ignore.
    #[arg(long)]
    pub invert: bool,
    /// Include nucleic-acid annotations with no associated CDS (those with cds_id='.')
    /// when applying CDS selection criteria.
    #[arg(long)]
    pub include_nucleic_acids: bool,
}
