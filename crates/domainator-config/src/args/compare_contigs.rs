use super::CommonArgs;
use crate::types::{Finite, PositiveInt};

/// Scores every pair of contigs by their shared domain content (upstream
/// `compare_contigs.py`).
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct CompareContigsArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// Genbank filenames. Use - to read standard input.
    #[arg(short, long, required = true, num_args = 1..)]
    pub input: Vec<String>,
    /// Write the contigs to this GenBank file, sorted hierarchically by similarity.
    /// Use - for standard output.
    #[arg(long)]
    pub genbank_out: Option<String>,
    /// Will rename contigs in the genbank file such that they have a prefix that can be
    /// sorted to restore the sorted order.
    #[arg(long)]
    pub name_by_order: bool,
    /// Hierarchical clustering linkage method used to sort contigs for --genbank_out.
    /// Distances are 1 - score/max(score); pairs with no score have distance 1.
    #[arg(long, default_value = "average",
          value_parser = ["average", "ward", "complete", "single", "weighted"])]
    pub linkage: String,
    /// How to order the leaves of the clustering tree for --genbank_out.
    /// greedy: flip subtrees so that the most similar contigs meet at each junction;
    /// fast and usually about as good as optimal. optimal: scipy's optimal leaf
    /// ordering; very slow (minutes to hours) beyond a few thousand contigs. none: plain
    /// dendrogram order.
    #[arg(long, default_value = "greedy", value_parser = ["greedy", "optimal", "none"])]
    pub leaf_ordering: String,
    /// Don't fail early when sorting the contigs for --genbank_out is estimated to need
    /// more memory than is available.
    #[arg(long)]
    pub skip_sort_memory_check: bool,
    /// Only annotate contigs with ids in this list. Additive with --contigs_file.
    /// default: all contigs.
    #[arg(long, num_args = 1..)]
    pub contigs: Option<Vec<String>>,
    /// Only annotate contigs with ids listed in this file (one per line). Additive
    /// with --contigs. default: all contigs.
    #[arg(long)]
    pub contigs_file: Option<String>,
    /// After combining all metric scores, keep this many of the top hits for each
    /// query and consider all other hits to be zero. Applied before --mst and --knn,
    /// so a cap can split clusters that --mst would otherwise keep connected.
    /// Default: keep all hits.
    #[arg(long, allow_negative_numbers = true)]
    pub max_hits_per_query: Option<i64>,
    /// Round any final scores less than or equal to this down to zero. This is
    /// primarily useful for making sparse outputs smaller.
    #[arg(long, default_value = "0", allow_negative_numbers = true)]
    pub threshold: Finite,
    /// Keep only the edges of the maximum spanning tree (forest). The clusters
    /// (connected components) are the same as the full graph's, with the connections
    /// inside each cluster pruned to the minimum needed to keep it connected. Combine
    /// with --knn to also keep each node's nearest neighbors. Best paired with
    /// --hdf_out.
    #[arg(long)]
    pub mst: bool,
    /// Keep only OR-symmetric K-nearest-neighbor edges (K >= 1): an edge is kept if it
    /// is among the K best for either of its nodes. On its own this can split a
    /// cluster of the full graph into several; add --mst to keep the clusters
    /// connected. Best paired with --hdf_out.
    #[arg(long)]
    pub knn: Option<PositiveInt>,
    /// Weighting for jaccard index metric, a comparison of all distinct types of
    /// domains in the contigs. When two contigs have no domains, their ji is 0.0.
    #[arg(long, default_value = "0.5", allow_negative_numbers = true)]
    pub ji: Finite,
    /// Weighting for adjacency index metric, a comparison of shared domain pairs in
    /// the contigs. When two contigs have 0 or 1 domains, their ai is 0.0.
    #[arg(long, default_value = "0.5", allow_negative_numbers = true)]
    pub ai: Finite,
    /// Domain databases to use for domain content comparison. default: all databases.
    #[arg(long, num_args = 1..)]
    pub databases: Option<Vec<String>>,
    /// Write the contig similarity score matrix to this Domainator hdf5 file. Values
    /// are stored sparse.
    #[arg(short = 'o', long)]
    pub hdf_out: Option<String>,
    /// Write the contig similarity score matrix to this tab-separated text file, every
    /// cell included. Use - for standard output.
    #[arg(long)]
    pub dense_text_out: Option<String>,
    /// How many cpu workers to use. 0: all cores.
    #[arg(long, default_value_t = 0, allow_negative_numbers = true)]
    pub cpu: i64,
    /// Show a progress bar for long-running steps (on a terminal only).
    #[arg(long)]
    pub progress: bool,
    /// Fail if a projected output file would exceed this many GB. Set to 0 to disable
    /// the guardrail.
    #[arg(long, default_value = "25.0", allow_negative_numbers = true)]
    pub max_output_gb: Finite,
}
