use super::{CommonArgs, StructureBackendArgs};
use crate::types::Finite;

/// Converts protein structures into a Domainator protein genbank file (upstream
/// `structure_to_genbank.py`).
///
/// Reads structure files (pdb/cif, optionally gzipped, given as files, globs, or a
/// directory searched recursively) or a prebuilt foldseek database, and writes one
/// protein genbank record per structure chain, so the sequence-based tools work on
/// structures with no further conversion. Chain splitting, parsing and naming are done
/// by the structural aligner, so records are named exactly as they are in a search
/// against the same structures. Use --db_out to also keep the aligner database, which
/// can be reused as the -i or -r of structure_domainate / structure_search.
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct StructureToGenbankArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// Structure files (pdb/cif, optionally gzipped), a directory to search recursively,
    /// a glob, or the prefix of a prebuilt foldseek database.
    #[arg(short, long, required = true, num_args = 1..)]
    pub input: Vec<String>,
    /// Write the protein records to this GenBank file. Use - for standard output.
    #[arg(short = 'o', long)]
    pub genbank_out: Option<String>,
    /// Write the protein sequences to this FASTA file. Use - for standard output.
    #[arg(long)]
    pub fasta_out: Option<String>,
    /// Also write the aligner database built from the input structures, using this path
    /// as its prefix. The database keeps C-alpha coordinates, so it can be reused as an
    /// input or reference for structure_domainate and structure_search.
    #[arg(long)]
    pub db_out: Option<String>,
    /// Store each chain's 3Di structural alphabet string in a /threedi qualifier on a
    /// full-length misc_feature. The qualifier is a single opaque string, so it does not
    /// survive tools that slice records (extract_domains, select_by_coord).
    #[arg(long)]
    pub store_3di: bool,
    #[command(flatten)]
    pub backend: StructureBackendArgs,
    /// Fail if a projected output file would exceed this many GB. Set to 0 to disable
    /// the guardrail.
    #[arg(long, default_value = "25.0", allow_negative_numbers = true)]
    pub max_output_gb: Finite,
}
