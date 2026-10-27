use super::CommonArgs;
use crate::columns::HmmerColumn;

/// Tabulates each profile of HMM files (upstream `hmmer_report.py`).
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct HmmerReportArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// hmm filenames. Use - to read standard input.
    #[arg(short, long, required = true, num_args = 1..)]
    pub input: Vec<String>,
    /// Write the table to this tab-separated file. Use - for standard output.
    #[arg(short = 'o', long)]
    pub tsv_out: Option<String>,
    /// Write the table in html format to this file. Use - for standard output.
    #[arg(long)]
    pub html_out: Option<String>,
    /// Write streaming NDJSON (one compact JSON object per profile) to this file. Use -
    /// for standard output.
    #[arg(long)]
    pub json_out: Option<String>,
    /// Max height in pixels to set the html output to.
    #[arg(long)]
    pub html_max_height: Option<i64>,
    /// If supplied, then this list will be used instead of the default column names.
    #[arg(long, num_args = 1..)]
    pub column_names: Option<Vec<String>>,
    /// The columns to report, in order, as name:arg:arg tokens (--help lists them).
    #[arg(long, num_args = 1.., value_name = "COLUMN",
          long_help = crate::columns::columns_help(crate::columns::HMMER_COLUMNS))]
    pub columns: Vec<HmmerColumn>,
}
