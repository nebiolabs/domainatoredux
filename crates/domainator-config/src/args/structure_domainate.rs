use super::{CommonArgs, StructureAlignmentArgs, StructureBackendArgs};
use crate::types::{Finite, NonNegFloat, PositiveFloat};

/// Annotates protein structures with hits to reference structures (upstream
/// `structure_domainate.py`).
///
/// The structure counterpart of domainate: every input chain is one protein record,
/// carrying Domainator features for the references that hit it; records without hits
/// are kept. References are aligned as the query and the input database as the target,
/// so --evalue is computed against the database being annotated. Annotated coordinates
/// are positions on the input record; the matching reference positions go in the
/// /rstart, /rend and /rlen qualifiers.
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct StructureDomainateArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// Structures to annotate: pdb/cif files (optionally gzipped), a directory to search
    /// recursively, a glob, or the prefix of a prebuilt foldseek database.
    #[arg(short, long, required = true, num_args = 1..)]
    pub input: Vec<String>,
    /// Reference structures to search for in the --input records being annotated. Same
    /// accepted forms as --input.
    #[arg(short, long, required = true, num_args = 1..)]
    pub reference: Vec<String>,
    /// Write the records to this GenBank file. Use - for standard output.
    #[arg(short = 'o', long)]
    pub genbank_out: Option<String>,
    /// Write the records to this FASTA file. Use - for standard output.
    #[arg(long)]
    pub fasta_out: Option<String>,
    /// Threshold E value for a hit. E values are computed against the input database.
    #[arg(short, long, default_value = "0.001")]
    pub evalue: PositiveFloat,
    /// Minimum E value for a hit. Useful for excluding near-identical matches, such as a
    /// structure's hit to itself.
    #[arg(long, default_value = "0.0")]
    pub min_evalue: NonNegFloat,
    /// Maximum number of annotations to add per record. If not supplied, there is no
    /// limit.
    #[arg(long, allow_negative_numbers = true)]
    pub max_hits_per_cds: Option<i64>,
    /// Maximum fraction of overlap allowed between annotations on one record. 1
    /// disables overlap filtering.
    #[arg(long, default_value = "1", allow_negative_numbers = true)]
    pub max_overlap: Finite,
    /// Filter overlapping annotations within each database independently, instead of
    /// across all databases.
    #[arg(long)]
    pub overlap_by_db: bool,
    /// Value for the /database qualifier on the annotations. Defaults to the reference
    /// database or directory name.
    #[arg(long)]
    pub database_name: Option<String>,
    /// Also write the aligner database built from the input structures, using this path
    /// as its prefix, so it can be reused as --input later.
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
