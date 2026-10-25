use super::CommonArgs;
use crate::types::Finite;

/// Keeps the HMM profiles that align well to any reference profile (upstream
/// `hmmer_search.py`).
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct HmmerSearchArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// Input query files. One or more hmm text files with one or more hmmer3 profiles.
    /// If -i and -r are different, -i will typically be the bigger of the two, as it is
    /// what is being filtered. Use - to read standard input.
    #[arg(short, long, required = true, num_args = 1..)]
    pub input: Vec<String>,
    /// Reference files. One or more hmm text files with one or more hmmer3 profiles. If
    /// -i and -r are different, -r will typically be the smaller of the two, as it is
    /// used as a filtering criterion.
    #[arg(short, long, required = true, num_args = 1..)]
    pub reference: Vec<String>,
    /// Write the hit profiles to this hmm file. Writes gzip-compressed output if the
    /// path ends in '.gz'; '.bgz' is refused because HMMER cannot read BGZF. Output to
    /// stdout is never compressed.
    #[arg(short = 'o', long)]
    pub hmm_out: Option<String>,
    /// Report alignments with scores greater than or equal to this. Default: 10 for
    /// amino acid profiles, and the equivalent point on the nucleotide score scale for
    /// DNA and RNA profiles (about 4.63). A value supplied here is used as-is, without
    /// rescaling.
    #[arg(long, allow_negative_numbers = true)]
    pub min_score: Option<Finite>,
    /// The maximum number of hmms returned by the search, keeping the best-scoring ones
    /// by the bitscore of their best scoring profile. If not supplied, return all hits
    /// passing the score cutoff.
    #[arg(long, allow_negative_numbers = true)]
    pub max_hits: Option<i64>,
    /// how many cpu threads to use. 0: one per physical core.
    #[arg(long, default_value_t = 0, allow_negative_numbers = true)]
    pub cpu: i64,
    /// Fail if a projected output file would exceed this many GB. Set to 0 to disable
    /// the guardrail.
    #[arg(long, default_value = "25.0", allow_negative_numbers = true)]
    pub max_output_gb: Finite,
}
