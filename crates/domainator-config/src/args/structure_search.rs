use super::{CommonArgs, StructureAlignmentArgs, StructureBackendArgs};
use crate::types::{Finite, NonNegFloat, PositiveFloat};

/// Selects matching structure-derived protein records and marks their best-hit regions
/// (upstream `structure_search.py`).
///
/// The structure counterpart of domain_search, for large structure databases (such as
/// a foldseek database built from AlphaFold DB) and relatively few references. Only
/// matching records are written, each with the target's full observed sequence and a
/// Domain_Search feature marking the region homologous to its best reference;
/// --add_annotations also writes Domainator features for the retained hits. --evalue is
/// computed against the database searched. --max_seqs bounds how many hits the backend
/// keeps per reference, and a warning is issued whenever a reference saturates it.
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct StructureSearchArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// The structure database to search: the prefix of a prebuilt foldseek database, or
    /// pdb/cif files (optionally gzipped), a directory to search recursively, or a glob.
    #[arg(short, long, required = true, num_args = 1..)]
    pub input: Vec<String>,
    /// Reference structures to search for in the --input database. Same accepted forms
    /// as --input.
    #[arg(short, long, required = true, num_args = 1..)]
    pub reference: Vec<String>,
    /// Write the records to this GenBank file. Use - for standard output.
    #[arg(short = 'o', long)]
    pub genbank_out: Option<String>,
    /// Write the records to this FASTA file. Use - for standard output.
    #[arg(long)]
    pub fasta_out: Option<String>,
    /// Threshold E value for a hit. E values are computed against the searched database.
    #[arg(short, long, default_value = "0.001")]
    pub evalue: PositiveFloat,
    /// Minimum E value for a hit. Useful for excluding a reference's match to its own
    /// copy in the database.
    #[arg(long, default_value = "0.0")]
    pub min_evalue: NonNegFloat,
    /// Return at most this many records, the best-scoring ones. Applies to the whole
    /// search, not per reference. When set, output is sorted best-first and written after
    /// the search completes.
    #[arg(long, allow_negative_numbers = true)]
    pub max_hits: Option<i64>,
    /// Maximum fraction of overlap allowed between annotations on one record. 1
    /// disables overlap filtering.
    #[arg(long, default_value = "1", allow_negative_numbers = true)]
    pub max_overlap: Finite,
    /// Filter overlapping annotations within each database independently, instead of
    /// across all databases.
    #[arg(long)]
    pub overlap_by_db: bool,
    /// Add domain annotations (Domainator features) for retained hits, in addition to
    /// the search best-hit annotations (Domain_Search features) already present.
    #[arg(long)]
    pub add_annotations: bool,
    /// Value for the /database qualifier on the annotations. Defaults to the reference
    /// database or directory name.
    #[arg(long)]
    pub database_name: Option<String>,
    /// When the input is structure files rather than a prebuilt database, also write the
    /// database built from them, using this path as its prefix.
    #[arg(long)]
    pub db_out: Option<String>,
    /// Write the aligner's own hit table here, for debugging or for cross-checking
    /// against a hand-run search. With reseek this is the table before the per-reference
    /// --max_seqs cap is applied, so it may hold more rows than the output.
    #[arg(long)]
    pub hits_tsv_out: Option<String>,
    #[command(flatten)]
    pub backend: StructureBackendArgs,
    #[command(flatten)]
    pub search: StructureAlignmentArgs,
    /// Fail if a projected output file would exceed this many GB. Set to 0 to disable
    /// the guardrail.
    #[arg(long, default_value = "25.0", allow_negative_numbers = true)]
    pub max_output_gb: Finite,
}
