use super::CommonArgs;
use crate::types::{Finite, PositiveInt};

/// Builds a sequence similarity network from a similarity matrix, for Cytoscape or the
/// Domainator Viewer (upstream `build_ssn.py`).
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct BuildSsnArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// Pairwise similarity (or distance) scores. In the network, nodes will be derived
    /// from row and column names, and edges will be added between pairs of rows and
    /// columns meeting the threshold criteria. Format can be tab-separated text, or
    /// Domainator hdf5.
    #[arg(short, long, required = true)]
    pub input: String,
    /// Keep only edges with weights strictly above this value. This should be >= 0.
    #[arg(long, default_value = "0", allow_negative_numbers = true)]
    pub threshold: Finite,
    /// Tab separated files of sequence metadata.
    #[arg(long, num_args = 1..)]
    pub metadata: Option<Vec<String>>,
    /// Only consider matrix labels in this list. Additive with --subset_file.
    #[arg(long, num_args = 1..)]
    pub subset: Option<Vec<String>>,
    /// Text file containing matrix labels to retain.
    #[arg(long)]
    pub subset_file: Option<String>,
    /// Color the points in the output image based on this column of the metadata
    /// table.
    #[arg(long)]
    pub color_by: Option<String>,
    /// Tab separated file with two columns and no header, columns are: annotation, hex
    /// color. For example: CCDB   cc0000
    #[arg(long)]
    pub color_table: Option<String>,
    /// Give distinct colors only to this many of the largest --color_by groups, all
    /// remaining groups share a neutral gray. By default every group gets a color,
    /// cycling over a palette of 64 distinguishable colors.
    #[arg(long, allow_negative_numbers = true)]
    pub max_color_groups: Option<i64>,
    /// Skip this many colors at the start of the palette before assigning colors to
    /// --color_by groups. For example, if a network colored by one column used the
    /// first 5 colors, --palette_offset 5 colors another column without reusing them.
    /// Ignored with --color_table.
    #[arg(long, default_value = "0", allow_negative_numbers = true)]
    pub palette_offset: i64,
    /// Give every edge a boolean 'MST' property saying whether it belongs to the
    /// maximum spanning tree. Useful for styling or filtering the MST backbone within
    /// a larger network in Cytoscape.
    #[arg(long)]
    pub mark_mst: bool,
    /// Tab separated file with two columns and no header, columns are: annotation, hex
    /// color. Written after the color table is updated with new colors, for example if
    /// using --color_by, but not supplying an external color table.
    #[arg(long)]
    pub color_table_out: Option<String>,
    /// Write the network to this Cytoscape xgmml file.
    #[arg(short = 'o', long)]
    pub xgmml_out: Option<String>,
    /// Write the Domainator Viewer, showing this network, to this HTML file. The viewer
    /// shows the network's maximum spanning tree above --threshold, whatever --mst and
    /// --knn say.
    #[arg(long)]
    pub html_out: Option<String>,
    /// Write the gzip-compressed JSON Domainator Viewer bundle (.dvb) to this file. The
    /// viewer opens it (domainatoredux viewer), and saves sessions in the same format.
    #[arg(long)]
    pub dvb_out: Option<String>,
    /// How the viewer's split chart measures a merge: min_child (node count of the
    /// smaller piece) or product (the two piece sizes multiplied). Switchable in the
    /// viewer.
    #[arg(long, default_value = "min_child", value_parser = ["product", "min_child"])]
    pub merge_impact_metric: String,
    /// Label the nodes in the html viewer by this metadata column at first.
    #[arg(long)]
    pub label_by: Option<String>,
    /// Numeric metadata columns (for example integer cluster numbers) that the html
    /// viewer should color as discrete categories instead of a gradient.
    #[arg(long, num_args = 1..)]
    pub categorical: Option<Vec<String>>,
    /// The network's name, shown in the html viewer. Defaults to the html (or dvb)
    /// output's file name.
    #[arg(long)]
    pub name: Option<String>,
    /// Creates a new metadata column called 'SSN_cluster', replacing that column from
    /// the metadata file if already present. Finds clusters by following connectivity,
    /// assigns each cluster a distinct integer, based on size rank, bigger clusters get
    /// smaller numbers.
    #[arg(long)]
    pub cluster: bool,
    /// Path to write a tsv file mapping each sequence to its cluster. If set, then
    /// --cluster is automatically activated. If --no_cluster_header is not set, then
    /// the file will have the header: contig cluster.
    #[arg(long)]
    pub cluster_tsv_out: Option<String>,
    /// If set, then the tsv file will not have a header. Only relevant if
    /// --cluster_tsv_out is set.
    #[arg(long)]
    pub no_cluster_header: bool,
    /// Fail if a projected output file would exceed this many GB. Set to 0 to disable
    /// the guardrail.
    #[arg(long, default_value = "25.0", allow_negative_numbers = true)]
    pub max_output_gb: Finite,
    /// Keep only the edges of the maximum spanning tree (forest). The clusters
    /// (connected components) are the same as the full graph's, with the connections
    /// inside each cluster pruned to the minimum needed to keep it connected. Combine
    /// with --knn to also keep each node's nearest neighbors.
    #[arg(long)]
    pub mst: bool,
    /// Keep only OR-symmetric K-nearest-neighbor edges (K >= 1): an edge is kept if it
    /// is among the K best for either of its nodes. On its own this can split a
    /// cluster of the full graph into several; add --mst to keep the clusters
    /// connected.
    #[arg(long)]
    pub knn: Option<PositiveInt>,
}
