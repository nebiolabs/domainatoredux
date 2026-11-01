use super::CommonArgs;
use crate::types::{Finite, PositiveInt};

/// Splits a sequence or profile file into one file per cluster of a similarity matrix
/// (upstream `split_by_cluster.py`).
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct SplitByClusterArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// Input genbank, fasta, hmm, or cm file to split by cluster.
    #[arg(short, long, required = true)]
    pub input: String,
    /// Matrix input in dense HDF5, sparse HDF5, or dense text format.
    #[arg(long, required = true)]
    pub matrix: String,
    /// Directory to write cluster files into. Existing target files cause an error.
    #[arg(long, required = true)]
    pub output_dir: String,
    /// Keep only edges with weights strictly above this value when finding connected
    /// components.
    #[arg(long, default_value = "0", allow_negative_numbers = true)]
    pub threshold: Finite,
    /// Minimum cluster size required to write a dedicated cluster file.
    #[arg(long, default_value = "1")]
    pub min_cluster_size: PositiveInt,
    /// If set, write clusters smaller than --min_cluster_size into one small_clusters
    /// file (small_clusters.gb for a .gb input). Otherwise those records or models are
    /// skipped.
    #[arg(long)]
    pub write_small_clusters: bool,
    /// For GenBank inputs, center each contig on its first Domain_Search annotation and
    /// pad records within each output file so the search hits align. Raises an error
    /// if any record lacks Domain_Search.
    #[arg(long)]
    pub pad_on_search_hits: bool,
}
