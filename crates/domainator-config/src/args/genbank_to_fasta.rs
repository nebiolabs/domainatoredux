use super::CommonArgs;

/// Converts GenBank files to FASTA (upstream `genbank_to_fasta.py`).
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct GenbankToFastaArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// Genbank filenames. Use - to read standard input.
    #[arg(short, long, required = true, num_args = 1..)]
    pub input: Vec<String>,
    /// Write the records to this FASTA file. Use - for standard output.
    #[arg(short = 'o', long)]
    pub fasta_out: Option<String>,
    // Port-only: upstream's tool runs on one thread. Here `.gz`/`.bgz` outputs are
    // compressed on this many.
    /// Threads for compressing a .gz or .bgz output (0: one thread per physical core).
    #[arg(long, default_value_t = 0, allow_negative_numbers = true)]
    pub cpu: i64,
}
