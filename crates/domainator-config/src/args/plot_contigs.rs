use super::CommonArgs;

/// Create graphical representations of the contigs and their annotations (upstream
/// `plot_contigs.py`). It works best for relatively small datasets, of a few megabases or a
/// few hundred contigs.
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct PlotContigsArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// names of input genbank files. Use - to read standard input.
    #[arg(short, long, required = true, num_args = 1..)]
    pub input: Vec<String>,
    /// Write the plot to this html file.
    #[arg(short = 'o', long, required = true)]
    pub html_out: String,
    /// height of the contig plot, in pixels
    #[arg(long, default_value = "2400", allow_negative_numbers = true)]
    pub height: i64,
    /// width of the contig plot, in pixels
    #[arg(long, default_value = "1200", allow_negative_numbers = true)]
    pub width: i64,
    /// Feature types to include in the plot. Default is 'CDS exon repeat_region ncRNA rRNA
    /// tRNA Domainator Domain_Search'.
    #[arg(long, num_args = 1..)]
    pub types: Option<Vec<String>>,
    /// height of the feature arrows, in pixels.
    #[arg(long, default_value = "50", allow_negative_numbers = true)]
    pub feature_height: i64,
    /// font size of the feature labels, in pixels.
    #[arg(long, default_value = "26", allow_negative_numbers = true)]
    pub feature_font_size: i64,
    /// tab separated files of additional contig metadata.
    #[arg(long, num_args = 1..)]
    pub metadata: Option<Vec<String>>,
    /// Color the contig lines in the plot based on this column of the metadata table.
    #[arg(long)]
    pub color_by: Option<String>,
    /// tab separated file with two columns and no header: a --color_by value and a hex
    /// color, for example: CCDB   cc0000. Without it, colors are assigned automatically.
    #[arg(long)]
    pub color_table: Option<String>,
    /// Skip this many colors at the start of the palette before assigning colors to
    /// --color_by values. For example, if a plot colored by one column used the first 5
    /// colors, --palette_offset 5 colors another column without reusing them. Ignored with
    /// --color_table.
    #[arg(long, default_value = "0", allow_negative_numbers = true)]
    pub palette_offset: i64,
}
