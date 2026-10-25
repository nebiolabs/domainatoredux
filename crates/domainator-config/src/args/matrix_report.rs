use super::CommonArgs;

/// Summarizes a similarity matrix: its size, score distribution, and how its clusters
/// merge as the threshold falls (upstream `matrix_report.py`).
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct MatrixReportArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// name of input matrix file.
    #[arg(short, long, required = true)]
    pub input: String,
    /// Write the report as text to this file.
    #[arg(short = 'o', long)]
    pub text_out: Option<String>,
    /// Write a compact JSON summary (edge stats, connected components, split-event
    /// series) to this file.
    #[arg(long)]
    pub json_out: Option<String>,
    /// Metric used for split-event plots and tables: 'min_child' emphasizes the
    /// smaller cluster breaking away, while 'product' emphasizes balanced large splits.
    #[arg(long, default_value = "min_child", value_parser = ["min_child", "product"])]
    pub merge_impact_metric: String,
    /// Maximum number of strongest merge events to report. A few more may be added on
    /// top: after the strongest are taken, the strongest event in each otherwise-empty
    /// 5% band of the threshold axis is added back, so the series never leaves a
    /// stretch of its own axis blank. Use 0 to include all merge events.
    #[arg(long, default_value_t = 500, allow_negative_numbers = true)]
    pub max_merge_events: i64,
    /// Accepted for compatibility with upstream; domainatoredux prints no progress
    /// updates.
    #[arg(long)]
    pub progress: bool,
    /// Accepted for compatibility with upstream; domainatoredux prints no stage
    /// timings.
    #[arg(long)]
    pub profile_stages: bool,
}
