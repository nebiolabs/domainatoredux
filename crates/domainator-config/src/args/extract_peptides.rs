use super::{CommonArgs, TaxonomyArgs};
use crate::types::PositiveFloat;

/// Extracts translated CDSs as protein records, keeping their domain annotations
/// (upstream `extract_peptides.py`).
///
/// CDS names, domain names, domain expressions, unannotated, and search-hit presence
/// are OR selectors; with none, every eligible CDS is selected. Contig IDs, strand,
/// fragment status, and taxonomy are independent AND restrictions. --invert negates
/// only the combined selectors.
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct ExtractPeptidesArgs {
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
    /// Only read input records with these IDs; not inverted by --invert. Additive with
    /// --contigs_file.
    #[arg(long, num_args = 1..)]
    pub contigs: Vec<String>,
    /// Only read input records with the IDs in this file: a plain-text file (one per
    /// line), FASTA/GenBank record IDs, or HMM profile names. Additive with --contigs.
    #[arg(long)]
    pub contigs_file: Option<String>,
    /// Select CDSs containing these domain names, OR with other CDS selectors.
    /// Additive with --domains_file.
    #[arg(long, num_args = 1..)]
    pub domains: Option<Vec<String>>,
    /// Select CDSs containing the domain names in this file: a plain-text file (one per
    /// line), FASTA/GenBank record IDs, or HMM profile names. Additive with --domains.
    #[arg(long)]
    pub domains_file: Option<String>,
    /// Boolean domain expression using &, |, ~, and parentheses. Additive with other
    /// CDS selectors.
    #[arg(long)]
    pub domain_expr: Option<String>,
    /// Consider only domains from these databases when filtering by domain. default:
    /// all databases.
    #[arg(long, num_args = 1..)]
    pub databases: Option<Vec<String>>,
    /// Extract peptides from CDSs with no domain annotations.
    #[arg(long)]
    pub unannotated: bool,
    /// Which annotations to select peptides by. domainator: domain annotations (from
    /// domainate). domain_search: best-hit annotations (from domain_search). all: both.
    #[arg(long, default_value = "domainator",
          value_parser = ["domainator", "domain_search", "all"])]
    pub annotation_type: String,
    /// Also select existing domain_search hits, regardless of annotation_type, domain
    /// names, domain expressions, or databases. Hits must pass the e-value and score
    /// cutoffs where applicable. Participates in --invert where available.
    #[arg(long)]
    pub has_search_hit: bool,
    /// Only extract peptides from CDSs on the specified strand.
    #[arg(long, value_parser = ["f", "r"])]
    pub strand: Option<String>,
    /// Which CDSs to extract peptides from, by fragment status. include: all. exclude:
    /// skip fragments. only: only fragments. A CDS is a fragment if its location has
    /// '<' or '>', or it has a /partial qualifier. A protein record is a fragment if it
    /// has a UniProt ' (Fragment)' or MGnify 'FL=0' header, a 'Flags: Fragment'
    /// definition or NON_TER features (UniProt text entries), or a CDS/Protein feature
    /// with '<' or '>' spanning it. Not affected by --invert.
    #[arg(long, default_value = "include", value_parser = ["include", "exclude", "only"])]
    pub partial: String,
    /// Invert the combined CDS selectors, not contigs, strand, partial, or taxonomy.
    /// With no selectors, warn and ignore.
    #[arg(long)]
    pub invert: bool,
    /// Only look for peptides on cdss with these names.
    #[arg(long, num_args = 1..)]
    pub cds: Option<Vec<String>>,
    /// Only look for peptides on cdss with names listed in this text file.
    #[arg(long)]
    pub cds_file: Option<String>,
    /// Use the original contig ID instead of the resolved CDS name for peptide IDs.
    /// This can produce duplicate IDs.
    #[arg(long)]
    pub keep_parent_id: bool,
    /// The CDS qualifier to prefer for CDS name matching and peptide IDs. If not
    /// supplied, then qualifiers will be checked in order of: protein_id, gene_id,
    /// locus_tag, gene.
    #[arg(long)]
    pub name_field: Option<String>,
    /// Maximum e-value of domain/search-hit annotations used for CDS selection and
    /// carried onto peptide records. Default: no cutoff.
    #[arg(short, long)]
    pub evalue: Option<PositiveFloat>,
}
