use super::CommonArgs;
use crate::types::Finite;

/// Builds a UPGMA tree from a distance or similarity matrix, or reads one from a
/// Newick file, and writes it as Newick, XGMML or the Domainator Viewer (upstream
/// `build_tree.py`).
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct BuildTreeArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// Square matrix of pairwise distances or similarity scores. Format can be
    /// tab-separated text or Domainator hdf5. Give this or --newick.
    #[arg(short, long)]
    pub input: Option<String>,
    /// Read the tree from this Newick file instead of building one from a matrix.
    /// Numeric internal node labels are read as support values.
    #[arg(long)]
    pub newick: Option<String>,
    /// How to turn matrix values into distances. precomputed: the values are distances.
    /// score_dist: the values are similarity scores, distance = 1 - (score /
    /// min(row_max, col_max)), so a pair with no score has distance 1. auto: precomputed
    /// if the matrix data type is a distance (ends in _dist) or not recorded (e.g.
    /// tab-separated text), otherwise score_dist.
    #[arg(long, default_value = "auto", value_parser = ["auto", "precomputed", "score_dist"])]
    pub distance: String,
    /// Which clustering algorithm to use.
    #[arg(long, default_value = "upgma", value_parser = ["upgma"])]
    pub algorithm: String,
    /// Tab separated files of sequence metadata, keyed by their first column. These data
    /// are added to the xgmml and html outputs.
    #[arg(long, num_args = 1..)]
    pub metadata: Option<Vec<String>>,
    /// Only include these leaves. Additive with --subset_file.
    #[arg(long, num_args = 1..)]
    pub subset: Option<Vec<String>>,
    /// Text file of leaf names to include.
    #[arg(long)]
    pub subset_file: Option<String>,
    /// An aligned FASTA file of the leaves' sequences, keyed by name, shown beside the
    /// tree in the html output.
    #[arg(long)]
    pub msa: Option<String>,
    /// Color the leaves in the xgmml, and at first in the html viewer, by this column of
    /// the metadata table.
    #[arg(long)]
    pub color_by: Option<String>,
    /// Tab separated file with two columns and no header, columns are: annotation, hex
    /// color. For example: CCDB   cc0000. Requires --color_by.
    #[arg(long)]
    pub color_table: Option<String>,
    /// Tab separated file with two columns and no header, columns are: annotation, hex
    /// color. Written after the color table is updated with new colors, for example if
    /// using --color_by, but not supplying an external color table.
    #[arg(long)]
    pub color_table_out: Option<String>,
    /// Skip this many colors at the start of the palette before assigning colors to
    /// --color_by values. Ignored with --color_table.
    #[arg(long, default_value = "0", allow_negative_numbers = true)]
    pub palette_offset: i64,
    /// Give distinct colors only to this many of the largest --color_by groups, all
    /// remaining groups share a neutral gray.
    #[arg(long, allow_negative_numbers = true)]
    pub max_color_groups: Option<i64>,
    /// Label the leaves in the html viewer by this metadata column at first.
    #[arg(long)]
    pub label_by: Option<String>,
    /// Numeric metadata columns (for example integer cluster numbers) that the html
    /// viewer should color as discrete categories instead of a gradient.
    #[arg(long, num_args = 1..)]
    pub categorical: Option<Vec<String>>,
    /// The tree's name, shown in the html viewer. Defaults to the html (or dvb) output's
    /// file name.
    #[arg(long)]
    pub name: Option<String>,
    /// Write the tree to this Newick file. Use - for standard output.
    #[arg(short = 'o', long)]
    pub newick_out: Option<String>,
    /// Write the tree to this Cytoscape xgmml file, laid out as a rectangular tree.
    #[arg(long)]
    pub xgmml_out: Option<String>,
    /// Write the Domainator Viewer, showing this tree, to this HTML file.
    #[arg(long)]
    pub html_out: Option<String>,
    /// Write the gzip-compressed JSON Domainator Viewer bundle (.dvb) to this file. The
    /// viewer opens it, and saves sessions in the same format.
    #[arg(long)]
    pub dvb_out: Option<String>,
    /// Fail if a projected output file would exceed this many GB. Set to 0 to disable
    /// the guardrail.
    #[arg(long, default_value = "25.0", allow_negative_numbers = true)]
    pub max_output_gb: Finite,
}
