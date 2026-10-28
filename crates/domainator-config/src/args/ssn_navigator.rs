use super::CommonArgs;

/// Answers navigation queries about the network in a Domainator Viewer bundle, as JSON
/// (upstream `ssn_navigator.py`).
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct SsnNavigatorArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// A Domainator Viewer bundle (.dvb) with a network, from build_ssn --dvb_out or the
    /// viewer's Save session. Older network bundles (.dsnv) work too.
    #[arg(short, long, required = true)]
    pub input: String,
    /// Write the JSON answer to this file.
    #[arg(short = 'o', long)]
    pub json_out: Option<String>,
    /// Which navigation query to run.
    #[arg(long, default_value = "overview", value_parser = [
        "overview", "thresholds", "clusters", "cluster", "node",
    ])]
    pub query: String,
    /// Similarity threshold to cut the network at (higher = finer clusters), matching
    /// the viewer's slider. Pass 'inf' for the slider's fine end, where every node is
    /// its own cluster. Omit it for the coarse end: one cluster per connected
    /// component.
    #[arg(long, allow_negative_numbers = true)]
    pub threshold: Option<String>,
    /// Cluster id to describe (see --query clusters). For --query cluster.
    #[arg(long, allow_negative_numbers = true)]
    pub cluster: Option<i64>,
    /// Node id/name to describe or locate. For --query cluster/node.
    #[arg(long)]
    pub node: Option<String>,
    /// Only report clusters with at least this many members (--query clusters).
    #[arg(long, default_value_t = 1, allow_negative_numbers = true)]
    pub min_size: i64,
    /// Bound list output: at most this many clusters, category values, etc.
    #[arg(long, default_value_t = 50, allow_negative_numbers = true)]
    pub top_n: i64,
    /// Include the full member id list in --query cluster output.
    #[arg(long)]
    pub members: bool,
    /// How many split events the --query thresholds/node cut-points are drawn from:
    /// the strongest this many by impact, plus a few so that no 5% band of the
    /// threshold range with an event to offer is left without one. 0 offers a
    /// cut-point at every split event. The series is derived from the bundle's merge
    /// order, so any value works on any bundle.
    #[arg(long, default_value_t = 500, allow_negative_numbers = true)]
    pub max_merge_events: i64,
}
