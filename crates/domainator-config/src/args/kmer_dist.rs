use super::CommonArgs;
use crate::types::{Finite, PositiveInt};

/// Calculates similarity scores between sequence records using exact k-mer set
/// comparisons (upstream `kmer_dist.py`).
///
/// Typically the input and reference will be the same sequence file, creating a
/// pairwise similarity matrix. Protein inputs use protein k-mers. Nucleotide inputs use
/// translated CDS protein k-mers by default, or strand-invariant nucleotide k-mers when
/// --nucleotide_kmers is supplied.
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct KmerDistArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// Input sequence file. This can be a fasta or genbank file.
    #[arg(short, long, required = true)]
    pub input: String,
    /// File format of the input, if it cannot be told from the file extension.
    #[arg(long, value_parser = ["fasta", "genbank"])]
    pub input_format: Option<String>,
    /// Reference file. If not provided, the input file will be used as the reference.
    #[arg(short, long)]
    pub reference: Option<String>,
    /// File format of the reference, if it cannot be told from the file extension.
    #[arg(long, value_parser = ["fasta", "genbank"])]
    pub reference_format: Option<String>,
    /// Whether the sequences in FASTA inputs are protein or nucleotide. GenBank records
    /// declare their own molecule type.
    #[arg(long, default_value = "protein", value_parser = ["nucleotide", "protein"])]
    pub alphabet: String,
    /// When activated, new CDS annotations will be added with Prodigal in Metagenomic
    /// mode before translated-CDS k-mer extraction. If 'all', then any existing CDS
    /// annotations will be deleted and all contigs will be re-annotated. If
    /// 'unannotated', then only contigs without CDS annotations will be annotated.
    #[arg(long, value_parser = ["all", "unannotated"])]
    pub gene_call: Option<String>,
    /// Use strand-invariant nucleotide k-mers instead of translated CDS protein k-mers
    /// for nucleotide inputs.
    #[arg(long)]
    pub nucleotide_kmers: bool,
    /// K-mer size. Defaults to 5 for protein/translated-CDS comparisons and 15 for
    /// nucleotide-kmer comparisons.
    #[arg(long, allow_negative_numbers = true)]
    pub ksize: Option<i64>,
    /// Similarity metric to compute from shared k-mer counts.
    #[arg(long, default_value = "max_containment", value_parser = [
        "max_containment", "min_containment", "jaccard",
    ])]
    pub metric: String,
    /// Output matrix values. score: raw similarity, bool: 1 if a similarity is
    /// non-zero, score_dist: 1 - (score / min(row_max, col_max)).
    #[arg(long, default_value = "score", value_parser = ["score", "bool", "score_dist"])]
    pub mode: String,
    /// Write the similarity (or, for score_dist, distance) matrix to this Domainator
    /// hdf5 file. Values are stored sparse; a matrix with exactly symmetric values is
    /// stored as its upper triangle.
    #[arg(short = 'o', long)]
    pub hdf_out: Option<String>,
    /// Write the similarity (or, for score_dist, distance) matrix to this tab-separated
    /// text file, every cell included. Use - for standard output.
    #[arg(long)]
    pub dense_text_out: Option<String>,
    /// Round any similarities lower than or equal to this down to zero before mode
    /// transforms.
    #[arg(long, default_value = "0", allow_negative_numbers = true)]
    pub threshold: Finite,
    /// Include at most this many non-zero entries in the matrix for each input
    /// sequence, keeping the highest-valued ones. Applied before --mst and --knn, so a
    /// cap can split clusters that --mst would otherwise keep connected.
    #[arg(long, allow_negative_numbers = true)]
    pub max_hits_per_query: Option<i64>,
    /// Keep only the edges of the maximum spanning tree (forest). The clusters
    /// (connected components) are the same as the full graph's, with the connections
    /// inside each cluster pruned to the minimum needed to keep it connected. Combine
    /// with --knn to also keep each node's nearest neighbors.
    ///
    /// Computed while streaming, so the full matrix is never held in memory. Selects on
    /// the k-mer similarity. Requires the reference to be the same file as the input (a
    /// square, symmetric comparison) and --mode in {score, bool}. Best paired with
    /// --hdf_out.
    #[arg(long)]
    pub mst: bool,
    /// Keep only OR-symmetric K-nearest-neighbor edges (K >= 1): an edge is kept if it
    /// is among the K best for either of its nodes. On its own this can split a
    /// cluster of the full graph into several; add --mst to keep the clusters
    /// connected.
    ///
    /// Computed while streaming, so the full matrix is never held in memory. Selects on
    /// the k-mer similarity. Requires the reference to be the same file as the input (a
    /// square, symmetric comparison) and --mode in {score, bool}. Best paired with
    /// --hdf_out.
    #[arg(long)]
    pub knn: Option<PositiveInt>,
    /// How many cpu threads to use. Default: use all physical cores.
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
