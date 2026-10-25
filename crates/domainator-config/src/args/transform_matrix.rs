use super::CommonArgs;
use crate::types::{Finite, PositiveInt};

/// Converts a matrix between value types, and prunes or reconciles it (upstream
/// `transform_matrix.py`).
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct TransformMatrixArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// Name of input matrix file.
    #[arg(short, long, required = true)]
    pub input: String,
    /// What the values of the input matrix are, if the file does not say. Overrides
    /// the type stored in the file.
    #[arg(long, value_parser = [
        "bool", "efi_score", "efi_score_dist", "norm_score", "row_norm_score", "score",
        "score_dist",
    ])]
    pub value_type: Option<String>,
    /// Write the transformed matrix to this Domainator hdf5 file. Values are stored
    /// sparse.
    #[arg(short = 'o', long)]
    pub hdf_out: Option<String>,
    /// Write the transformed matrix to this tab-separated text file, every cell
    /// included. Use - for standard output.
    #[arg(long)]
    pub dense_text_out: Option<String>,
    /// What kind of values should be in the output matrix. none: preserve the input
    /// values and data type, score: raw score, bool: 1 if a hit otherwise 0,
    /// score_dist: 1 - (score / min(row_max, col_max)), norm_score: score/min(row_max,
    /// col_max), row_norm_score: score/row_max, efi_score: -log10[2^(-score) *
    /// (input_seq_length * reference_seq_length)], efi_score_dist: 1 - (efi_score /
    /// min(row_max, col_max)).
    #[arg(long, default_value = "none", value_parser = [
        "none", "score", "bool", "norm_score", "row_norm_score", "score_dist", "efi_score",
        "efi_score_dist",
    ])]
    pub mode: String,
    /// Zero out all values less than or equal to this threshold after any mode
    /// transformation.
    #[arg(long, allow_negative_numbers = true)]
    pub threshold: Option<Finite>,
    /// Keep only the edges of the maximum spanning tree (forest). The clusters
    /// (connected components) are the same as the full graph's, with the connections
    /// inside each cluster pruned to the minimum needed to keep it connected. Combine
    /// with --knn to also keep each node's nearest neighbors. Uses the post-transform
    /// values, except that --mode bool selects on the scores and then keeps the
    /// selected cells as 1; requires a square, non-distance matrix.
    #[arg(long)]
    pub mst: bool,
    /// Keep only OR-symmetric K-nearest-neighbor edges (K >= 1): an edge is kept if it
    /// is among the K best for either of its nodes. On its own this can split a
    /// cluster of the full graph into several; add --mst to keep the clusters
    /// connected. Uses the post-transform values, except that --mode bool selects on
    /// the scores and then keeps the selected cells as 1; requires a square,
    /// non-distance matrix.
    #[arg(long)]
    pub knn: Option<PositiveInt>,
    /// Reconcile the two directions of each pair of a square self-comparison, on the
    /// raw values before any --mode transform. none: leave the matrix as computed,
    /// which may be slightly asymmetric. max: the better of the two directions in both
    /// cells, the OR-symmetric convention used by --mst/--knn and by build_ssn. min:
    /// the worse of the two -- note a pair found in only one direction has a stored
    /// zero in the other, so min DELETES it, making this a mutual-hit filter. mean:
    /// the average of the two -- note a pair found in only one direction is HALVED,
    /// because the missing direction contributes zero. Applied before --mode; requires
    /// a square matrix with the same row and column labels, holding similarity (not
    /// *_dist) values. Cannot be combined with --mst/--knn, whose output is always
    /// max-symmetric.
    #[arg(long, default_value = "none", value_parser = ["none", "min", "max", "mean"])]
    pub symmetrize: String,
    /// Fail if a projected output file would exceed this many GB. Set to 0 to disable
    /// the guardrail.
    #[arg(long, default_value = "25.0", allow_negative_numbers = true)]
    pub max_output_gb: Finite,
}
