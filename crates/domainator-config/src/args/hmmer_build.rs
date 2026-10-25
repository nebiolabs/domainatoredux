use super::CommonArgs;
use crate::types::Finite;

/// Builds an HMM profile from a multiple sequence alignment (upstream
/// `hmmer_build.py`).
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct HmmerBuildArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// Path of input msa. Acceptable formats are the same as for hmmbuild. Use - to
    /// read standard input.
    #[arg(short, long, required = true)]
    pub input: String,
    /// Write the profile to this hmm file. Writes gzip-compressed output if the path
    /// ends in '.gz'; '.bgz' is refused because HMMER cannot read BGZF. Output to
    /// stdout is never compressed.
    #[arg(short = 'o', long)]
    pub hmm_out: Option<String>,
    /// Name of the HMM profile.
    #[arg(long, required = true)]
    pub name: String,
    /// Accession of the HMM profile.
    #[arg(long)]
    pub acc: Option<String>,
    /// Description of the HMM profile.
    #[arg(long)]
    pub desc: Option<String>,
    /// Alphabet of the input MSA. If not supplied, inferred from the input, which can
    /// guess wrong on short or ambiguous alignments.
    #[arg(long, ignore_case = true, value_parser = ["amino", "dna", "rna"])]
    pub alphabet: Option<String>,
    /// Max expected hit length (the MAXL field of the profile), used by nhmmer for
    /// windowed E-values. Nucleotide alphabets only. If not supplied, hmmer picks one.
    /// Takes precedence over --window_beta, so only one of the two may be given.
    #[arg(long, allow_negative_numbers = true)]
    pub window_length: Option<i32>,
    /// Tail mass from which to derive the window length. Nucleotide alphabets only.
    /// Only one of --window_length and --window_beta may be given. Not supported by
    /// domainatoredux: giving it is an error.
    #[arg(long, allow_negative_numbers = true)]
    pub window_beta: Option<Finite>,
}
