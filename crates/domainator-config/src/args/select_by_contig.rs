use super::{CommonArgs, TaxonomyArgs};
use crate::types::PositiveFloat;

/// Selects whole contigs by their names, domain content, taxonomy and size (upstream
/// `select_by_contig.py`).
///
/// The criteria combine as: length bounds AND CDS-count bounds AND taxonomy AND
/// fragment status AND (any of: name in --contigs, name matches --contigs_regex,
/// definition matches --definition_regex, sequence matches --sequence_regex, a domain
/// in --domains, --domain_expr true, --unannotated and no domains, a search hit with
/// --has_search_hit, a search-hit score in the --min/max_search_score range), then at
/// most --max_records contigs. --invert inverts only the "any of" part.
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct SelectByContigArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// Names of input genbank files. Use - to read standard input.
    #[arg(short, long, required = true, num_args = 1..)]
    pub input: Vec<String>,
    /// Write the records to this GenBank file. Use - for standard output.
    #[arg(short = 'o', long)]
    pub genbank_out: Option<String>,
    /// Write the records to this FASTA file. Use - for standard output.
    #[arg(long)]
    pub fasta_out: Option<String>,
    /// Select records with these IDs, OR with other record selectors; affected by
    /// --invert. Additive with --contigs_file.
    #[arg(long, num_args = 1..)]
    pub contigs: Option<Vec<String>>,
    /// Select records with the IDs in this file: a plain-text file (one per line),
    /// FASTA/GenBank record IDs, or HMM profile names. Additive with --contigs.
    #[arg(long)]
    pub contigs_file: Option<String>,
    /// Only consider contigs with names matching this regex.
    #[arg(long)]
    pub contigs_regex: Option<String>,
    /// Only keep contigs with a definition that matches this regex.
    #[arg(long)]
    pub definition_regex: Option<String>,
    /// Only keep contigs with a sequence that matches this regex.
    #[arg(long)]
    pub sequence_regex: Option<String>,
    /// Select records containing these domain names, OR with other record selectors.
    /// Additive with --domains_file.
    #[arg(long, num_args = 1..)]
    pub domains: Option<Vec<String>>,
    /// Select records containing the domain names in this file: a plain-text file (one
    /// per line), FASTA/GenBank record IDs, or HMM profile names. Additive with
    /// --domains.
    #[arg(long)]
    pub domains_file: Option<String>,
    /// A boolean expression using operators & (AND), | (OR), and ~ (NOT), to specify a
    /// desired combination of domains.
    #[arg(long)]
    pub domain_expr: Option<String>,
    /// Select contigs by fragment status. include: all. exclude: drop fragments. only:
    /// keep only fragments. A protein record is a fragment if it has a UniProt
    /// ' (Fragment)' or MGnify 'FL=0' header, a 'Flags: Fragment' definition, NON_TER
    /// features, or a CDS/Protein feature with '<' or '>' spanning it. A nucleotide
    /// contig is a fragment if any of its CDSs has '<' or '>' in its location, or a
    /// /partial qualifier. Not affected by --invert.
    #[arg(long, default_value = "include", value_parser = ["include", "exclude", "only"])]
    pub partial: String,
    /// Which annotations to select contigs by. domainator: domain annotations (from
    /// domainate). domain_search: best-hit annotations (from domain_search). all: both.
    #[arg(long, default_value = "domainator",
          value_parser = ["domainator", "domain_search", "all"])]
    pub annotation_type: String,
    /// Also select existing domain_search hits, regardless of annotation_type, domain
    /// names, domain expressions, or databases. Hits must pass the e-value and score
    /// cutoffs where applicable. Participates in --invert where available.
    #[arg(long)]
    pub has_search_hit: bool,
    /// Consider only domains from these databases when filtering by domain. default:
    /// all databases.
    #[arg(long, num_args = 1..)]
    pub databases: Option<Vec<String>>,
    /// Select contigs with no domain annotations.
    #[arg(long)]
    pub unannotated: bool,
    /// The evalue cutoff (max) for domain annotations to be considered when selecting
    /// by domain. Default: no cutoff.
    #[arg(short, long)]
    pub evalue: Option<PositiveFloat>,
    #[command(flatten)]
    pub taxonomy: TaxonomyArgs,
    /// If set then stop after returning this many contigs.
    #[arg(long, allow_negative_numbers = true)]
    pub max_records: Option<i64>,
    /// Keep only input records with length >= this, in bp for nucleotide input or
    /// amino acids for protein input. Not affected by --invert.
    #[arg(long, allow_negative_numbers = true)]
    pub min_length: Option<i64>,
    /// Keep only input records with length <= this, in bp for nucleotide input or
    /// amino acids for protein input. Not affected by --invert.
    #[arg(long, allow_negative_numbers = true)]
    pub max_length: Option<i64>,
    /// Skip contigs with fewer CDSs than this. Pseudogenes are not counted.
    #[arg(long, allow_negative_numbers = true)]
    pub min_cds: Option<i64>,
    /// Skip contigs with more CDSs than this. Pseudogenes are not counted.
    #[arg(long, allow_negative_numbers = true)]
    pub max_cds: Option<i64>,
    /// Also select records with an existing Domain_Search annotation scoring >= this
    /// and passing e-value/database filters. Paired with --max_search_score as an
    /// inclusive range; OR with other selectors and affected by --invert. Also
    /// restricts --has_search_hit scoring, which ignores databases.
    #[arg(long, allow_negative_numbers = true)]
    pub min_search_score: Option<crate::types::Finite>,
    /// Upper bound (inclusive) of the search-score selector range. OR with other
    /// selectors, not a restriction on records selected by other criteria. Also
    /// restricts --has_search_hit scoring, which ignores databases.
    #[arg(long, allow_negative_numbers = true)]
    pub max_search_score: Option<crate::types::Finite>,
    /// Invert the selection criteria. i.e. return contigs that do not match the
    /// selection criteria.
    #[arg(long)]
    pub invert: bool,
}
