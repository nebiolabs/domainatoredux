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
}
