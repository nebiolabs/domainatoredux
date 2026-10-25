use super::CommonArgs;
use crate::types::PositiveFloat;

/// Extracts translated CDSs as protein records, keeping their domain annotations
/// (upstream `extract_peptides.py`).
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
    /// Only look for peptides on contigs with ids in this list. Additive with
    /// --contigs_file. default: all contigs.
    #[arg(long, num_args = 1..)]
    pub contigs: Vec<String>,
    /// Only look for peptides on contigs with ids listed in this file (one per line).
    /// Additive with --contigs. default: all contigs.
    #[arg(long)]
    pub contigs_file: Option<String>,
    /// Only extract peptides containing these domains.
    #[arg(long, num_args = 1..)]
    pub domains: Option<Vec<String>>,
    /// Text file containing names of domains, where peptides containing these domains
    /// will be extracted.
    #[arg(long)]
    pub domains_file: Option<String>,
    /// Consider only domains from these databases when filtering by domain. default:
    /// all databases.
    #[arg(long, num_args = 1..)]
    pub databases: Option<Vec<String>>,
    /// Extract peptides from CDSs with no domain annotations.
    #[arg(long)]
    pub unannotated: bool,
    /// Which annotations to select peptides by. With domain_search, the search hits are
    /// extracted. domainator: domain annotations (from domainate). domain_search:
    /// best-hit annotations (from domain_search).
    #[arg(long, default_value = "domainator", value_parser = ["domainator", "domain_search"])]
    pub annotation_type: String,
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
    /// Invert the CDS selection criteria. i.e. return peptides from CDSs that don't
    /// match the CDS selection criteria. (Only applies to CDS selection, not contig
    /// selection)
    #[arg(long)]
    pub invert: bool,
    /// Only look for peptides on cdss with these names.
    #[arg(long, num_args = 1..)]
    pub cds: Option<Vec<String>>,
    /// Only look for peptides on cdss with names listed in this text file.
    #[arg(long)]
    pub cds_file: Option<String>,
    /// By default, extracted regions will be renamed to note their original
    /// coordinates and whether they were reverse complemented. Set this option to keep
    /// contigs with their original names. WARNING: if you aren't careful, this can
    /// easily result in duplicate names.
    #[arg(long)]
    pub keep_name: bool,
    /// The CDS qualifier to preferentially use as the name of the peptide. If not
    /// supplied, then qualifiers will be checked in order of: protein_id, gene_id,
    /// locus_tag, gene.
    #[arg(long)]
    pub name_field: Option<String>,
    /// The evalue cutoff (max) for domain annotations to be put onto the new genbank
    /// file. Default: no cutoff.
    #[arg(short, long)]
    pub evalue: Option<PositiveFloat>,
}
