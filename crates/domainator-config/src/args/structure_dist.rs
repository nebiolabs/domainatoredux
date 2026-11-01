use super::{CommonArgs, StructureAlignmentArgs, StructureBackendArgs};
use crate::types::{Finite, PositiveFloat, PositiveInt};

/// Calculates similarity or distance matrices between protein structures (upstream
/// `structure_dist.py`).
///
/// The structure counterpart of seq_dist. With only -i, an all-vs-all matrix of the
/// input structures; a separate -r gives inputs (rows) against references (columns).
/// Every chain of every input is a row, hit or not, so the labels are the record ids
/// structure_to_genbank writes. The input is the query and the reference the target, so
/// --evalue is computed against the reference set. The score-based modes behave as in
/// seq_dist; tmscore, lddt and fident are already between 0 and 1, and their *_dist forms are
/// one minus the value. tmscore and lddt require --algorithm foldseek and C-alpha
/// coordinates in both databases.
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct StructureDistArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// Structures to compare: pdb/cif files (optionally gzipped), a directory searched
    /// recursively, a glob, or the prefix of a prebuilt aligner database.
    #[arg(short, long, required = true, num_args = 1..)]
    pub input: Vec<String>,
    /// Structures to compare the input against. Same accepted forms as --input. If not
    /// supplied, the input is compared against itself, giving a square all-vs-all
    /// matrix. Rows are inputs and columns are references.
    #[arg(short, long, num_args = 1..)]
    pub reference: Option<Vec<String>>,
    /// Write the similarity (or, for *_dist modes, distance) matrix to this Domainator
    /// hdf5 file. Values are stored sparse; a matrix with exactly symmetric values is
    /// stored as its upper triangle.
    #[arg(short = 'o', long)]
    pub hdf_out: Option<String>,
    /// Write the similarity (or, for *_dist modes, distance) matrix to this
    /// tab-separated text file, every cell included. Use - for standard output.
    #[arg(long)]
    pub dense_text_out: Option<String>,
    /// Only pairs with an E-value strictly below this become non-zero cells. E-values
    /// are computed against the reference set, as they are in seq_dist.
    #[arg(short, long, default_value = "0.001")]
    pub evalue: PositiveFloat,
    /// What kind of values should be in the matrix. Score-based, from the aligner's
    /// alignment score, exactly as in seq_dist: score, bool, norm_score, row_norm_score,
    /// score_dist, efi_score, efi_score_dist. Structural, already between 0 and 1 so the
    /// distance forms are just 1 - value: tmscore, lddt, fident, tmscore_dist,
    /// lddt_dist, fident_dist. efi_score and efi_score_dist require a real bit score, so
    /// they are rejected with --algorithm reseek.
    #[arg(long, default_value = "score", value_parser = [
        "bool", "efi_score", "efi_score_dist", "fident", "fident_dist", "lddt", "lddt_dist",
        "norm_score", "row_norm_score", "score", "score_dist", "tmscore", "tmscore_dist",
    ])]
    pub mode: String,
    /// Reconcile the two directions of every pair before the --mode transform: max, min
    /// or mean of the two cells of each pair. Requires a square self-comparison (no -r). Cannot
    /// be combined with --mst/--knn, whose output is always max-symmetric.
    #[arg(long, default_value = "none", value_parser = ["none", "min", "max", "mean"])]
    pub symmetrize: String,
    /// Round any values lower than or equal to this down to zero. This applies to the
    /// raw per-hit value before the --mode transform: the aligner's alignment score for
    /// the score-based modes, and the metric itself (0-1) for the tmscore/lddt/fident
    /// modes.
    #[arg(long, default_value = "0", allow_negative_numbers = true)]
    pub threshold: Finite,
    /// Include at most this many non-zero entries in the matrix for each input
    /// structure, keeping the highest-valued ones. Applied after --symmetrize, and
    /// before --mst and --knn, so a cap can split clusters that --mst would otherwise
    /// keep connected. Use --max_seqs to bound the search itself.
    #[arg(long, allow_negative_numbers = true)]
    pub max_hits_per_query: Option<i64>,
    /// Keep only the edges of the maximum spanning tree (forest). Requires a square
    /// self-comparison and --mode in {score, bool, efi_score, tmscore, lddt, fident};
    /// bool selects on the alignment scores and then keeps the selected cells as 1.
    #[arg(long)]
    pub mst: bool,
    /// Keep only OR-symmetric K-nearest-neighbor edges (K >= 1): an edge is kept if it
    /// is among the K best for either of its nodes. Same requirements as --mst.
    #[arg(long)]
    pub knn: Option<PositiveInt>,
    /// When the input is structure files rather than a prebuilt database, also write the
    /// database built from them, using this path as its prefix.
    #[arg(long)]
    pub db_out: Option<String>,
    /// Write the aligner's own hit table here, for debugging or for cross-checking
    /// against a hand-run search.
    #[arg(long)]
    pub hits_tsv_out: Option<String>,
    /// Show a progress bar for long-running steps (on a terminal only).
    #[arg(long)]
    pub progress: bool,
    #[command(flatten)]
    pub backend: StructureBackendArgs,
    #[command(flatten)]
    pub search: StructureAlignmentArgs,
    /// Fail if a projected output file would exceed this many GB. Set to 0 to disable
    /// the guardrail.
    #[arg(long, default_value = "25.0", allow_negative_numbers = true)]
    pub max_output_gb: Finite,
}
