use super::CommonArgs;
use crate::types::{Finite, PositiveInt};

/// Computes a similarity matrix between input and reference sequences or profiles
/// (upstream `seq_dist.py`).
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct SeqDistArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// Input query file. Queries in the input file will be compared to the reference
    /// sequences/profiles. This can be a fasta, genbank, or hmm file.
    #[arg(short, long, required = true)]
    pub input: String,
    /// File format of the input, if it cannot be told from the file extension.
    #[arg(long, value_parser = ["fasta", "genbank", "hmm"])]
    pub input_format: Option<String>,
    /// Reference file. If not provided, the input file will be used as the reference,
    /// and the output will be a pairwise similarity matrix between the input
    /// sequences/profiles. This can be a fasta, genbank, or hmm file.
    #[arg(short, long)]
    pub reference: Option<String>,
    /// File format of the reference, if it cannot be told from the file extension.
    /// Ignored for some kinds of algorithms, for example, hmmer expects a .hmm
    /// reference file.
    #[arg(long, value_parser = ["fasta", "genbank", "hmm"])]
    pub reference_format: Option<String>,
    /// Write the similarity (or, for *_dist modes, distance) matrix to this
    /// Domainator hdf5 file. Values are stored sparse.
    #[arg(short = 'o', long)]
    pub hdf_out: Option<String>,
    /// Write the similarity (or, for *_dist modes, distance) matrix to this
    /// tab-separated text file, every cell included. Use - for standard output.
    #[arg(long)]
    pub dense_text_out: Option<String>,
    /// Round any scores lower than this down to zero (this applies to raw scores not
    /// processed/normalized).
    #[arg(long, default_value = "0", allow_negative_numbers = true)]
    pub threshold: Finite,
    /// Keep at most this many hits for each input sequence, the highest-scoring ones.
    /// This caps the search itself (diamond --max-target-seqs, or the hmmer hit list).
    /// Applied before --mst and --knn, so a cap can split clusters that --mst would
    /// otherwise keep connected. Default: keep all hits.
    #[arg(long, allow_negative_numbers = true)]
    pub max_hits_per_query: Option<i64>,
    /// Which distance metric to use, diamond_*; f: fast, d: default, mids:
    /// mid-sensitive, s: sensitive, mors: more-sensitive, vs: very-sensitive, us:
    /// ultra-sensitive. default: diamond_us (ultra sensitive). diamond_* needs the
    /// external diamond binary.
    #[arg(long, default_value = "diamond_us", value_parser = [
        "diamond_f", "diamond_d", "diamond_mids", "diamond_s", "diamond_mors",
        "diamond_vs", "diamond_us", "hmmer", "hmmer_compare",
    ])]
    pub algorithm: String,
    /// Extra options for diamond blastp, as one quoted string split like a shell
    /// command line. Use the = form: --diamond_args="-c 4 -b 0.4 --target-indexed".
    /// Options that seq_dist sets itself (query, database, --outfmt, -p,
    /// --max-target-seqs, --max-hsps, --min-score, --verbose, and the sensitivity
    /// flags) are rejected; use the corresponding seq_dist option instead. seq_dist
    /// defaults --algo to 0 and --tmpdir to its own managed temporary directory; both
    /// can be replaced here. Useful for large all-vs-all runs, where diamond's
    /// seed-search scratch can exceed available disk: a smaller --block-size (-b)
    /// and/or more --index-chunks (-c) bound its peak scratch usage.
    #[arg(long, allow_hyphen_values = true)]
    pub diamond_args: Option<String>,
    /// Path to the diamond executable, if it is not on the PATH.
    #[arg(long)]
    pub diamond_path: Option<String>,
    /// Keep only the edges of the maximum spanning tree (forest). The clusters
    /// (connected components) are the same as the full graph's, with the connections
    /// inside each cluster pruned to the minimum needed to keep it connected. Combine
    /// with --knn to also keep each node's nearest neighbors.
    ///
    /// Computed while streaming, so the full matrix is never held in memory. Requires
    /// the reference to be the same file as the input (a square, symmetric comparison)
    /// and --mode in {score, bool, efi_score}; bool selects on the scores and then
    /// keeps the selected cells as 1. Best paired with --hdf_out.
    #[arg(long)]
    pub mst: bool,
    /// Keep only OR-symmetric K-nearest-neighbor edges (K >= 1): an edge is kept if it
    /// is among the K best for either of its nodes. On its own this can split a
    /// cluster of the full graph into several; add --mst to keep the clusters
    /// connected.
    ///
    /// Computed while streaming, so the full matrix is never held in memory. Requires
    /// the reference to be the same file as the input (a square, symmetric comparison)
    /// and --mode in {score, bool, efi_score}; bool selects on the scores and then
    /// keeps the selected cells as 1. Best paired with --hdf_out.
    #[arg(long)]
    pub knn: Option<PositiveInt>,
    /// Reconcile the two directions of each pair of a square self-comparison, on the
    /// raw values before any --mode transform. none: leave the matrix as computed,
    /// which may be slightly asymmetric. max: the better of the two directions in both
    /// cells, the OR-symmetric convention used by --mst/--knn and by build_ssn. min:
    /// the worse of the two -- note a pair found in only one direction has a stored
    /// zero in the other, so min DELETES it, making this a mutual-hit filter. mean:
    /// the average of the two -- note a pair found in only one direction is HALVED,
    /// because the missing direction contributes zero. Applied to the raw scores,
    /// after --threshold and --max_hits_per_query. Requires the reference to be the
    /// same file as the input. Cannot be combined with --mst/--knn, whose output is
    /// always max-symmetric.
    #[arg(long, default_value = "none", value_parser = ["none", "min", "max", "mean"])]
    pub symmetrize: String,
    /// What kind of values should be in the matrix. score: raw score, bool: 1 if a hit
    /// otherwise 0, score_dist: 1 - (score / min(row_max, col_max)), norm_score:
    /// score/min(row_max, col_max), efi_score: -log10[2^(-score) *
    /// (input_seq_length * reference_seq_length)], efi_score_dist: 1 - (efi_score /
    /// min(row_max, col_max)).
    #[arg(long, default_value = "score", value_parser = [
        "score", "bool", "norm_score", "row_norm_score", "score_dist", "efi_score",
        "efi_score_dist",
    ])]
    pub mode: String,
    /// How many cpu threads to use. 0: all cores.
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
