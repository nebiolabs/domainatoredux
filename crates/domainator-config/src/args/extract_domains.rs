use super::CommonArgs;
use crate::types::{Finite, PositiveFloat};

/// Extracts annotated domains as their own records (upstream `extract_domains.py`).
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct ExtractDomainsArgs {
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
    /// Only extract domains from contigs with ids in this list. Additive with
    /// --contigs_file. default: all contigs.
    #[arg(long, num_args = 1..)]
    pub contigs: Vec<String>,
    /// Only extract domains from contigs with ids listed in this file (one per line).
    /// Additive with --contigs. default: all contigs.
    #[arg(long)]
    pub contigs_file: Option<String>,
    /// The evalue cutoff (max) for domain annotations to be extracted.
    #[arg(short, long)]
    pub evalue: Option<PositiveFloat>,
    /// The score cutoff (min) for domain annotations to be extracted.
    #[arg(long, allow_negative_numbers = true)]
    pub min_score: Option<Finite>,
    /// Include domains with these names.
    #[arg(long, num_args = 1..)]
    pub domains: Option<Vec<String>>,
    /// Text file containing names of domains to include.
    #[arg(long)]
    pub domains_file: Option<String>,
    /// Domain databases to use for domain extraction. default: all databases.
    #[arg(long, num_args = 1..)]
    pub databases: Option<Vec<String>>,
    /// Which annotations to extract. domainator: domain annotations (from domainate).
    /// domain_search: best-hit annotations (from domain_search).
    #[arg(long, default_value = "domainator", value_parser = ["domainator", "domain_search"])]
    pub annotation_type: String,
    /// Which domains to extract, by the CDS or protein they are on, by fragment status.
    /// include: all. exclude: skip fragments. only: only fragments. A CDS is a fragment
    /// if its location has '<' or '>', or it has a /partial qualifier. A protein record
    /// is a fragment if it has a UniProt ' (Fragment)' or MGnify 'FL=0' header, a
    /// 'Flags: Fragment' definition or NON_TER features (UniProt text entries), or a
    /// CDS/Protein feature with '<' or '>' spanning it. Contig-level nucleotide hits
    /// are never on fragments.
    #[arg(long, default_value = "include", value_parser = ["include", "exclude", "only"])]
    pub partial: String,
    /// Extract this many additional residues (or bases) on each side of the hit
    /// region. Sets the default of --extend_up and --extend_down.
    #[arg(long, allow_negative_numbers = true)]
    pub extend_range: Option<i64>,
    /// Extract this many additional residues (or bases) upstream of the hit region.
    /// Default: --extend_range, or 0.
    #[arg(long, allow_negative_numbers = true)]
    pub extend_up: Option<i64>,
    /// Extract this many additional residues (or bases) downstream of the hit region.
    /// Default: --extend_range, or 0.
    #[arg(long, allow_negative_numbers = true)]
    pub extend_down: Option<i64>,
    /// If set, then when multiple target domains occur in the same contig, an envelope
    /// containing all of the hit domains will be extracted. By default, each hit
    /// domain will be extracted separately. This option will raise an exception on
    /// circular contigs, because it would be ambiguous. Cannot be used with --splice.
    #[arg(long)]
    pub combine: bool,
    /// If set, then when multiple target domains occur in the same contig, the
    /// intermediate sequence will be deleted and the extracted domains from the contig
    /// glued together, in the order they are read on the strand of the domain with the
    /// lowest start coordinate. By default, each hit domain will be extracted
    /// separately. Cannot be used with --combine.
    #[arg(long)]
    pub splice: bool,
    /// By default extracted regions will be flipped so that the domain (or the domain
    /// with the lowest start coordinate when --combine or --splice is active) is on the
    /// forward strand. Set this option to not reverse complement anything. In the rare
    /// case of a split domain with parts on different strands, the strand of the first
    /// part will be considered.
    #[arg(long)]
    pub keep_direction: bool,
    /// By default, extracted regions will be renamed to note their original
    /// coordinates and whether they were reverse complemented. Set this option to keep
    /// contigs with their original names. WARNING: if you aren't careful, this can
    /// easily result in duplicate names.
    #[arg(long)]
    pub keep_name: bool,
}
