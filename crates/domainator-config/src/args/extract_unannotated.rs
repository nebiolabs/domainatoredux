use super::CommonArgs;
use crate::types::{Finite, PositiveFloat};

/// Extracts the regions of proteins not covered by any domain annotation (upstream
/// `extract_unannotated.py`).
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct ExtractUnannotatedArgs {
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
    /// Only extract regions from contigs with ids in this list. Additive with
    /// --contigs_file. default: all contigs.
    #[arg(long, num_args = 1..)]
    pub contigs: Vec<String>,
    /// Only extract regions from contigs with ids listed in this file (one per line).
    /// Additive with --contigs. default: all contigs.
    #[arg(long)]
    pub contigs_file: Option<String>,
    /// The evalue cutoff (max) for domain annotations to be considered. default: no
    /// cutoff.
    #[arg(short, long)]
    pub evalue: Option<PositiveFloat>,
    /// The score cutoff (min) for domain annotations to be considered. default: no
    /// cutoff.
    #[arg(long, allow_negative_numbers = true)]
    pub min_score: Option<Finite>,
    /// Only consider domains from these databases. default: all databases.
    #[arg(long, num_args = 1..)]
    pub databases: Option<Vec<String>>,
    /// Which annotations count as annotated. domainator: domain annotations (from
    /// domainate). domain_search: best-hit annotations (from domain_search).
    #[arg(long, default_value = "domainator", value_parser = ["domainator", "domain_search"])]
    pub annotation_type: String,
    /// By default extracted regions will be flipped so that the source CDS will be on
    /// the forward strand. Set this option to not reverse complement anything. In the
    /// rare case of a split CDS with parts on different strands, the strand of the
    /// first part will be considered.
    #[arg(long)]
    pub keep_direction: bool,
    /// By default, extracted regions will be renamed to note their original
    /// coordinates and whether they were reverse complemented. Set this option to keep
    /// contigs with their original names. WARNING: if you aren't careful, this can
    /// easily result in duplicate names.
    #[arg(long)]
    pub keep_name: bool,
    /// Extract only the largest unannotated region from each protein, providing it
    /// passes the length cutoff specified by --min_length.
    #[arg(long)]
    pub largest: bool,
    /// Extract unannotated regions of at least this length from each protein. default:
    /// no cutoff.
    #[arg(long, allow_negative_numbers = true)]
    pub min_length: Option<i64>,
}
