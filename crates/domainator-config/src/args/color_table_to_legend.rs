use super::CommonArgs;

/// Draws a legend for color tables as an SVG (upstream `color_table_to_legend.py`).
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct ColorTableToLegendArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// names of color table files. Files are tab separated with two columns and no
    /// header, columns are: annotation, hex color. For example: CCDB   cc0000. Use - to
    /// read standard input.
    #[arg(short, long, required = true, num_args = 1..)]
    pub input: Vec<String>,
    /// Write the legend to this svg file.
    #[arg(short = 'o', long, required = true)]
    pub svg_out: String,
    /// Title of the legend.
    #[arg(long, default_value = "Legend")]
    pub title: String,
}
