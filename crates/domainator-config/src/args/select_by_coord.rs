use super::CommonArgs;

/// Extracts contig regions listed in a coordinate table (upstream
/// `select_by_coord.py`).
///
/// The table is tab- or comma-separated with a header: contig, start, end (1-based,
/// inclusive), and optionally strand and file. A file value is a basename (with
/// extension); a row without one matches any input file.
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct SelectByCoordArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// Names of input genbank files. Use - to read standard input.
    #[arg(short, long, required = true, num_args = 1..)]
    pub input: Vec<String>,
    /// Write the records to this GenBank file. Use - for standard output.
    #[arg(short = 'o', long)]
    pub genbank_out: Option<String>,
    /// Write the records to this FASTA file. Use - for standard output.
    #[arg(long)]
    pub fasta_out: Option<String>,
    /// Labeled coordinate table with columns contig, start, end, strand (optional),
    /// file (optional).
    #[arg(long, required = true)]
    pub coords: String,
}
