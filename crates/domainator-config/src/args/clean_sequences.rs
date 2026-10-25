use super::CommonArgs;

/// Cleans sequence names and residues, and filters out sequences with non-canonical
/// residues (upstream `clean_sequences.py`).
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct CleanSequencesArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// Names of input genbank files. Use - to read standard input.
    #[arg(short, long, required = true, num_args = 1..)]
    pub input: Vec<String>,
    /// Whether the sequences in FASTA inputs are protein or nucleotide. GenBank records
    /// declare their own molecule type.
    #[arg(long, default_value = "protein", value_parser = ["nucleotide", "protein"])]
    pub alphabet: String,
    /// Write the records to this GenBank file. Use - for standard output.
    #[arg(short = 'o', long)]
    pub genbank_out: Option<String>,
    /// Write the records to this FASTA file. Use - for standard output.
    #[arg(long)]
    pub fasta_out: Option<String>,
    /// If true then make the name compatible with newick format.
    #[arg(long)]
    pub clean_name: bool,
    /// If true then rename sequences such that output sequences have unique names.
    #[arg(long)]
    pub deduplicate_names: bool,
    /// If true then strip non-canonical residues or nucleotides from the ends of
    /// sequences (depending on input type). Including * characters.
    #[arg(long)]
    pub strip: bool,
    /// If true then throw out sequences containing non-canonical residues or
    /// nucleotides (depending on input type). Including * characters. (applied after
    /// strip)
    #[arg(long)]
    pub filter_by_aa: bool,
    /// Write the cleaning statistics to this file. If not supplied, writes them to
    /// stderr.
    #[arg(long)]
    pub log_out: Option<String>,
}
