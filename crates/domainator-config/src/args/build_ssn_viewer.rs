use super::CommonArgs;
use crate::types::Finite;

/// Builds a compact MST-based bundle for the standalone Domainator Similarity Network
/// Viewer (upstream `build_ssn_viewer.py`).
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct BuildSsnViewerArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// Symmetric similarity matrix to convert into a Domainator Similarity Network
    /// Viewer bundle. Format can be tab-separated text or Domainator hdf5.
    #[arg(short, long)]
    pub input: Option<String>,
    /// Write the gzip-compressed JSON Domainator Similarity Network Viewer bundle
    /// (.dsnv) to this file.
    #[arg(short = 'o', long)]
    pub dsnv_out: Option<String>,
    /// Write a standalone static HTML viewer page to this file. Without --embed_data,
    /// the page loads local bundle files.
    #[arg(long)]
    pub html_out: Option<String>,
    /// Embed the input bundle JSON into --html_out so the viewer autoloads it on open.
    #[arg(long)]
    pub embed_data: bool,
    /// Optional display name recorded in the bundle. Defaults to the output file stem.
    #[arg(long)]
    pub name: Option<String>,
    /// Tab-separated metadata tables keyed by node id.
    #[arg(long, num_args = 1..)]
    pub metadata: Option<Vec<String>>,
    /// Only consider matrix labels in this list. Additive with --subset_file.
    #[arg(long, num_args = 1..)]
    pub subset: Option<Vec<String>>,
    /// Text file containing matrix labels to retain.
    #[arg(long)]
    pub subset_file: Option<String>,
    /// Record a default metadata field for node coloring in the viewer bundle.
    #[arg(long)]
    pub color_by: Option<String>,
    /// Tab separated file with two columns and no header, columns are: annotation, hex
    /// color. For example: CCDB   cc0000. Sets the starting colors of the --color_by
    /// column in the viewer; values not in the table are gray. Requires --color_by.
    #[arg(long)]
    pub color_table: Option<String>,
    /// Record a default metadata field for node labels in the viewer bundle.
    #[arg(long)]
    pub label_by: Option<String>,
    /// Numeric metadata columns (for example integer cluster numbers) that the viewer
    /// should color as discrete categories instead of a gradient. Can be toggled per
    /// column in the viewer's color picker.
    #[arg(long, num_args = 1..)]
    pub categorical: Option<Vec<String>>,
    /// Metric the viewer uses for split events: min_child (node count of the smaller
    /// piece) or product (the two piece sizes multiplied). Recorded in the bundle as
    /// the default; switchable in the viewer.
    #[arg(long, default_value = "min_child", value_parser = ["min_child", "product"])]
    pub merge_impact_metric: String,
    /// Fail if a projected output file would exceed this many GB. Set to 0 to disable
    /// the guardrail.
    #[arg(long, default_value = "25.0", allow_negative_numbers = true)]
    pub max_output_gb: Finite,
}
