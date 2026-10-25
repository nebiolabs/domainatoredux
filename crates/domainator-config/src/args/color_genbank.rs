use super::CommonArgs;

/// Adds color qualifiers to CDS and domain features (upstream `color_genbank.py`).
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct ColorGenbankArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// Genbank filenames. Use - to read standard input.
    #[arg(short, long, required = true, num_args = 1..)]
    pub input: Vec<String>,
    /// Write the records to this GenBank file. Use - for standard output.
    #[arg(short = 'o', long)]
    pub genbank_out: Option<String>,
    /// Write the records to this FASTA file. Use - for standard output.
    #[arg(long)]
    pub fasta_out: Option<String>,
    /// Only color contigs with ids in this list (other contigs will be excluded from
    /// output). Additive with --contigs_file. default: all contigs.
    #[arg(long, num_args = 1..)]
    pub contigs: Vec<String>,
    /// Only color contigs with ids listed in this file (one per line). Additive with
    /// --contigs. default: all contigs.
    #[arg(long)]
    pub contigs_file: Option<String>,
    /// Tab separated file with two columns and no header, columns are: domain id, hex
    /// color. For example: CCDB   cc0000
    #[arg(long)]
    pub color_table: Option<String>,
    /// Tab separated file with two columns and no header, columns are: annotation, hex
    /// color. Written after the color table is updated with new colors, for example if
    /// using --color_by, but not supplying an external color table.
    #[arg(long)]
    pub color_table_out: Option<String>,
    /// Color to use for search hit annotations.
    #[arg(long)]
    pub search_hit_color: Option<String>,
    /// If set, then remove all color annotations from the genbank file before adding
    /// any new ones.
    #[arg(long)]
    pub clear: bool,
    /// Domain databases to use for coloring. default: all databases.
    #[arg(long, num_args = 1..)]
    pub databases: Option<Vec<String>>,
    /// What to color. domains: color only domain annotations. cdss: color only CDS
    /// annotations. domains_and_cdss: color both domains and CDS annotations.
    #[arg(long, default_value = "cdss", value_parser = ["domains", "cdss", "domains_and_cdss"])]
    pub target: String,
    /// If set, then color domains based on their database instead of their name. If
    /// used with --color_table, the first column must contain database ids.
    #[arg(long)]
    pub domain_database: bool,
}
