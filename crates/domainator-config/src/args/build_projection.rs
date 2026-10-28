use super::CommonArgs;
use crate::types::Finite;

/// Projects a matrix's rows into 2D or 3D, and writes the coordinates, XGMML or the
/// Domainator Viewer (upstream `build_projection.py`). This port computes `pca` and
/// `coords`; `tsne` and `umap` are refused at run time.
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct BuildProjectionArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// The matrix that you want to project into 2D or 3D space, rows are observations
    /// (typically gene names), columns are features (typically similarity to other
    /// genes, or hmm profile scores). Format can be tab-separated text or Domainator
    /// hdf5.
    #[arg(short, long, required = true)]
    pub input: String,
    /// Which projection algorithm to use. tsne and umap are not supported by
    /// domainatoredux yet.
    #[arg(long, default_value = "pca", value_parser = ["pca", "tsne", "umap", "coords"])]
    pub algorithm: String,
    /// Tab separated files of sequence metadata.
    #[arg(long, num_args = 1..)]
    pub metadata: Option<Vec<String>>,
    /// Color the points in the HTML plot and the xgmml by this column of the metadata
    /// table.
    #[arg(long)]
    pub color_by: Option<String>,
    /// Tab separated file with two columns and no header, columns are: annotation, hex
    /// color. For example: CCDB   cc0000
    #[arg(long)]
    pub color_table: Option<String>,
    /// Tab separated file with two columns and no header, columns are: annotation, hex
    /// color. Written after the color table is updated with new colors, for example if
    /// using --color_by, but not supplying an external color table.
    #[arg(long)]
    pub color_table_out: Option<String>,
    /// Skip this many colors at the start of the palette before assigning colors to
    /// --color_by values. For example, if a plot colored by one column used the first 5
    /// colors, --palette_offset 5 colors another column without reusing them. Ignored
    /// with --color_table.
    #[arg(long, default_value = "0", allow_negative_numbers = true)]
    pub palette_offset: i64,
    /// Label the points in the html viewer by this metadata column at first.
    #[arg(long)]
    pub label_by: Option<String>,
    /// Numeric metadata columns (for example integer cluster numbers) that the html
    /// viewer should color as discrete categories instead of a gradient.
    #[arg(long, num_args = 1..)]
    pub categorical: Option<Vec<String>>,
    /// The projection's name, shown in the html viewer. Defaults to the html (or dvb)
    /// output's file name.
    #[arg(long)]
    pub name: Option<String>,
    /// Write the projection to this Cytoscape xgmml file.
    #[arg(short = 'o', long)]
    pub xgmml_out: Option<String>,
    /// Write the Domainator Viewer, showing the projection, to this HTML file. It
    /// selects points into a metadata table and saves pictures as SVG or PNG.
    #[arg(long)]
    pub html_out: Option<String>,
    /// Write the gzip-compressed JSON Domainator Viewer bundle (.dvb) to this file. The
    /// viewer opens it, and saves sessions in the same format.
    #[arg(long)]
    pub dvb_out: Option<String>,
    /// Write the projected coordinates to this tab-separated file.
    #[arg(long)]
    pub coords_out: Option<String>,
    /// Keyword arguments for the pca implementation, as key=value pairs in one quoted
    /// string, for example --pca_args="whiten=True". Values are read as Python literals
    /// where possible.
    #[arg(long)]
    pub pca_args: Option<String>,
    /// Keyword arguments for the tsne implementation, as key=value pairs in one quoted
    /// string. Values are read as Python literals where possible.
    #[arg(long)]
    pub tsne_args: Option<String>,
    /// Keyword arguments for the umap implementation, as key=value pairs in one quoted
    /// string. Values are read as Python literals where possible.
    #[arg(long)]
    pub umap_args: Option<String>,
    /// Scale the largest axis so that the difference between the largest and smallest
    /// value is this.
    #[arg(long, default_value = "1000", allow_negative_numbers = true)]
    pub scale: Finite,
    /// If set, then makes 3d projections. default: 2d projections.
    #[arg(long)]
    pub three_d: bool,
}
