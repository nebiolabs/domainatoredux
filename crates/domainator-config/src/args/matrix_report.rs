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
    /// Write the report as html to this file.
    #[arg(long)]
    pub html_out: Option<String>,
    /// Write a compact JSON summary (edge stats, connected components, split-event
    /// series) to this file.
    #[arg(long)]
    pub json_out: Option<String>,
    /// Project how many edges 'build_ssn --threshold T --mst --knn K' would keep, for every
    /// threshold T and K from 1 to 25, and add a K slider to the HTML report. Disabled by
    /// default to reduce memory use.
    #[arg(long)]
    pub knn_projection: bool,
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
    /// Print live progress updates to stderr during long-running matrix_report stages.
    #[arg(long)]
    pub progress: bool,
    /// Print per-stage runtime timings to stderr for profiling large matrix_report runs.
    #[arg(long)]
    pub profile_stages: bool,
}
