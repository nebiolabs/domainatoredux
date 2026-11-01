//! The aligner-backend options every structure tool shares (upstream
//! `structure_lib.add_backend_arguments`), flattened into each tool's args.

/// foldseek or reseek, and how to run it.
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct StructureBackendArgs {
    /// Structural alignment backend to use.
    #[arg(long, default_value = "foldseek", value_parser = ["foldseek", "reseek"])]
    pub algorithm: String,
    /// Path to the foldseek executable. If not supplied, foldseek is looked up on PATH.
    #[arg(long)]
    pub foldseek_path: Option<String>,
    /// Path to the reseek executable. If not supplied, reseek is looked up on PATH.
    #[arg(long)]
    pub reseek_path: Option<String>,
    /// Extra options for the foldseek search command, for options domainator does not
    /// wrap, as one quoted string split like a shell command line. Use the = form:
    /// --foldseek_args="--option value". Options that domainator sets itself are refused.
    #[arg(long, allow_hyphen_values = true)]
    pub foldseek_args: Option<String>,
    /// Extra options for the reseek search command, for options domainator does not
    /// wrap, as one quoted string split like a shell command line. Use the = form:
    /// --reseek_args="--option value". Options that domainator sets itself are refused.
    #[arg(long, allow_hyphen_values = true)]
    pub reseek_args: Option<String>,
    /// Number of threads to give the backend. 0 lets the backend use all available
    /// cores.
    #[arg(long, default_value_t = 0, allow_negative_numbers = true)]
    pub cpu: i64,
    /// Parent directory for temporary databases and alignment files. Structural
    /// searches can need a lot of scratch space, so point this at a large filesystem for
    /// big inputs.
    #[arg(long)]
    pub tmp_dir: Option<String>,
    /// reseek only: SCOP level that reseek's p-value model is calibrated against. This
    /// shifts p-values by orders of magnitude, so it changes which hits pass --evalue.
    /// Ignored by other backends.
    #[arg(long, default_value = "superfamily", value_parser = ["family", "superfamily", "fold"])]
    pub reseek_stats: String,
    /// reseek only: search sensitivity. Ignored by other backends.
    #[arg(long, default_value = "sensitive", value_parser = ["fast", "sensitive"])]
    pub reseek_sensitivity: String,
}

/// The backend options of the tools that search (all but `structure_to_genbank`).
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct StructureAlignmentArgs {
    /// foldseek only: how to compute the alignment: 0 = 3Di only, 1 = TM-align, 2 =
    /// 3Di+AA (the default when unset). Type 1 requires C-alpha coordinates in both
    /// databases. reseek has no alignment-type option and rejects this.
    #[arg(long, value_parser = alignment_type_parser())]
    pub alignment_type: Option<i64>,
    /// Additional per-hit structural metrics to compute and store as feature qualifiers.
    /// tmscore, lddt and rmsd require C-alpha coordinates in both databases.
    #[arg(long, num_args = 1.., value_parser = ["tmscore", "lddt", "rmsd", "prob"])]
    pub metrics: Option<Vec<String>>,
    /// Maximum number of hits to keep per reference, keeping the most significant ones.
    /// Defaults to the backend's own default: 1000 for foldseek, no cap for reseek. For
    /// foldseek this bounds the search itself, so raising it costs time but improves
    /// coverage; for reseek it bounds only the output, since reseek has no native cap and
    /// does the full search either way. A warning is printed whenever a reference
    /// saturates the cap.
    #[arg(long, allow_negative_numbers = true)]
    pub max_seqs: Option<i64>,
    /// Where to run the search. 'cpu' for a CPU-only search, or a CUDA device string
    /// ('cuda', 'cuda:0', 'cuda:1', ...) for a GPU-accelerated search. GPU search
    /// requires a recent foldseek.
    #[arg(long, default_value = "cpu")]
    pub device: String,
}

/// `0`, `1` or `2`, parsed as an integer (so the manifest types the choices as numbers).
fn alignment_type_parser() -> impl clap::builder::TypedValueParser<Value = i64> {
    use clap::builder::TypedValueParser;
    clap::builder::PossibleValuesParser::new(["0", "1", "2"])
        .map(|s| s.parse::<i64>().expect("a listed choice"))
}
