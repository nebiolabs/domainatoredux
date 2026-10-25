//! Each migrated tool's interface, one clap `Args` struct per tool.
//!
//! The struct is the tool's single interface description: the command line parses
//! into it, a `--config` file is expanded into the same command line
//! ([`crate::config_file`]), `--print_config` and `describe` read it back through clap
//! ([`crate::manifest`]), and the GUI renders its dialog from that manifest. Field
//! names are upstream's parameter names, which are also the long flags (every struct
//! is `rename_all = "snake_case"`) and the config-file keys.

mod clean_sequences;
mod color_genbank;
mod common;
mod enum_report;
mod extract_domains;
mod extract_peptides;
mod extract_unannotated;
mod filter_domains;
mod genbank_to_fasta;
mod hmmer_report;
mod select_by_cds;
mod select_by_contig;
mod select_by_coord;
mod sort_contigs;
mod trim_contigs;

pub use clean_sequences::CleanSequencesArgs;
pub use color_genbank::ColorGenbankArgs;
pub use common::CommonArgs;
pub use enum_report::EnumReportArgs;
pub use extract_domains::ExtractDomainsArgs;
pub use extract_peptides::ExtractPeptidesArgs;
pub use extract_unannotated::ExtractUnannotatedArgs;
pub use filter_domains::FilterDomainsArgs;
pub use genbank_to_fasta::GenbankToFastaArgs;
pub use hmmer_report::HmmerReportArgs;
pub use select_by_cds::SelectByCdsArgs;
pub use select_by_contig::SelectByContigArgs;
pub use select_by_coord::SelectByCoordArgs;
pub use sort_contigs::SortContigsArgs;
pub use trim_contigs::TrimContigsArgs;

/// The `clap::Command` of a migrated tool, built from its args struct alone, for
/// callers that have no CLI to ask (the GUI's tool dialogs).
pub fn tool_command(name: &str) -> Option<clap::Command> {
    use clap::Args;
    let cmd = clap::Command::new(crate::registry::tool_info(name)?.name);
    Some(match name {
        "sort_contigs" => SortContigsArgs::augment_args(cmd),
        "enum_report" => EnumReportArgs::augment_args(cmd),
        "hmmer_report" => HmmerReportArgs::augment_args(cmd),
        "select_by_contig" => SelectByContigArgs::augment_args(cmd),
        "select_by_cds" => SelectByCdsArgs::augment_args(cmd),
        "select_by_coord" => SelectByCoordArgs::augment_args(cmd),
        "filter_domains" => FilterDomainsArgs::augment_args(cmd),
        "extract_domains" => ExtractDomainsArgs::augment_args(cmd),
        "extract_peptides" => ExtractPeptidesArgs::augment_args(cmd),
        "extract_unannotated" => ExtractUnannotatedArgs::augment_args(cmd),
        "trim_contigs" => TrimContigsArgs::augment_args(cmd),
        "color_genbank" => ColorGenbankArgs::augment_args(cmd),
        "clean_sequences" => CleanSequencesArgs::augment_args(cmd),
        "genbank_to_fasta" => GenbankToFastaArgs::augment_args(cmd),
        _ => return None,
    })
}
