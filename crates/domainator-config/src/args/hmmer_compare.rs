use super::CommonArgs;
use crate::types::Finite;

/// Aligns and calculates alignment scores between hmmer3 profiles (upstream
/// `hmmer_compare.py`).
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct HmmerCompareArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// Input query files. One or more hmm text files with one or more hmmer3 profiles.
    /// Use - to read standard input.
    #[arg(short, long, required = true, num_args = 1..)]
    pub input: Vec<String>,
    /// Reference files. One or more hmm text files with one or more hmmer3 profiles.
    #[arg(short, long, required = true, num_args = 1..)]
    pub reference: Vec<String>,
    /// Report alignments with scores greater than or equal to this. Scores are on the
    /// score scale of the input alphabet: DNA and RNA scores run at roughly half the
    /// amino acid scale, so a threshold tuned on proteins is about twice as strict on
    /// nucleotide profiles.
    #[arg(long, default_value = "0", allow_negative_numbers = true)]
    pub min_score: Finite,
    /// Include at most this many of the top hits for each query. Default: Include all
    /// hits.
    #[arg(long, allow_negative_numbers = true)]
    pub max_hits_per_query: Option<i64>,
    /// Write the scores to this tab-separated file.
    #[arg(short = 'o', long)]
    pub tsv_out: Option<String>,
    /// Also write each hit's alignment to this text file.
    #[arg(long)]
    pub alignments_out: Option<String>,
    /// how many cpu threads to use. 0: one per physical core.
    #[arg(long, default_value_t = 0, allow_negative_numbers = true)]
    pub cpu: i64,
    /// Fail if a projected output file would exceed this many GB. Set to 0 to disable
    /// the guardrail.
    #[arg(long, default_value = "25.0", allow_negative_numbers = true)]
    pub max_output_gb: Finite,
}
