use super::CommonArgs;
use crate::types::Finite;

/// Builds a covariance model (CM) from a multiple sequence alignment with Infernal's
/// cmbuild (upstream `infernal_build.py`).
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct InfernalBuildArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// Path of input aligned MSA. Any format accepted by cmbuild can be used. Formats
    /// without structure annotation, such as aligned FASTA, generally require --noss.
    /// Use - to read standard input.
    #[arg(short, long, required = true)]
    pub input: String,
    /// Write the CM to this file.
    #[arg(short = 'o', long)]
    pub cm_out: Option<String>,
    /// Name of the CM profile.
    #[arg(long, required = true)]
    pub name: String,
    /// Accession of the CM profile.
    #[arg(long)]
    pub acc: Option<String>,
    /// Description of the CM profile.
    #[arg(long)]
    pub desc: Option<String>,
    /// Optional alignment format to pass through to cmbuild, such as stockholm, selex,
    /// or afa.
    #[arg(long)]
    pub input_format: Option<String>,
    /// Path to the cmbuild executable. Default: $DOMAINATOREDUX_CMBUILD, then a bundled
    /// copy, then cmbuild on the PATH. infernox's infernox-cmbuild works too, for
    /// Stockholm, Pfam or SELEX input.
    #[arg(long)]
    pub cmbuild_path: Option<String>,
    /// Ignore secondary-structure annotation and build a zero-basepair model.
    #[arg(long)]
    pub noss: bool,
    /// Use Stockholm RF annotation to define consensus columns.
    #[arg(long)]
    pub hand: bool,
    /// Consensus-column threshold to pass through to cmbuild.
    #[arg(long, allow_negative_numbers = true)]
    pub symfrac: Option<Finite>,
}
