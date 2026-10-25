use super::CommonArgs;

/// Selects HMM profiles whose NAME, DESC or ACC fields match text criteria (upstream
/// `hmmer_select.py`).
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct HmmerSelectArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// names of input hmm files. Use - to read standard input.
    #[arg(short, long, required = true, num_args = 1..)]
    pub input: Vec<String>,
    /// Write the selected profiles to this hmm file. Writes gzip-compressed output if
    /// the path ends in '.gz'; '.bgz' is refused because HMMER cannot read BGZF.
    /// Output to stdout is never compressed.
    #[arg(short = 'o', long)]
    pub hmm_out: Option<String>,
    /// What regex(es) to search.
    #[arg(long, num_args = 1..)]
    pub regex: Option<Vec<String>>,
    /// What exact string(s) to search. Target must match exactly with no additions at
    /// either end.
    #[arg(long, num_args = 1..)]
    pub exact: Option<Vec<String>>,
    /// What string(s) to search. Target must contain this as a substring.
    #[arg(long, num_args = 1..)]
    pub contains: Option<Vec<String>>,
    /// whether to search the NAME field, the DESC field, the ACC field, or all three.
    #[arg(long, default_value = "all", ignore_case = true,
          value_parser = ["name", "desc", "acc", "all"])]
    pub field: String,
    /// If set, then the search is case sensitive. Otherwise it is case insensitive.
    #[arg(long)]
    pub case_sensitive: bool,
    /// Only read input files whose profiles use this alphabet, skipping the rest.
    /// Unlike the text criteria this narrows the selection rather than adding to it,
    /// and it can be used on its own to split a mixed set of input files by alphabet.
    /// Because an hmm file can only hold one alphabet, this selects whole files.
    #[arg(long, ignore_case = true, value_parser = ["amino", "dna", "rna"])]
    pub alphabet: Option<String>,
}
