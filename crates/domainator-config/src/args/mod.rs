//! Each tool's interface, one clap `Args` struct per tool.
//!
//! The struct is the tool's single interface description: the command line parses
//! into it, a `--config` file is expanded into the same command line
//! ([`crate::config_file`]), `--print_config` and `describe` read it back through clap
//! ([`crate::manifest`]), and the GUI renders its dialog from that manifest. Field
//! names are upstream's parameter names, which are also the long flags (every struct
//! is `rename_all = "snake_case"`) and the config-file keys.

mod build_projection;
mod build_ssn;
mod build_tree;
mod clean_sequences;
mod color_genbank;
mod color_table_to_legend;
mod common;
mod compare_contigs;
mod db_download;
mod deduplicate_genbank;
mod domain_search;
mod domainate;
mod enum_report;
mod extract_domains;
mod extract_peptides;
mod extract_unannotated;
mod filter_domains;
mod find_features;
mod format_db;
mod genbank_to_fasta;
mod gene_call;
mod hmmer_build;
mod hmmer_compare;
mod hmmer_report;
mod hmmer_search;
mod hmmer_select;
mod infernal_build;
mod kmer_dist;
mod matrix_report;
mod plot_contigs;
mod rotate_contigs;
mod select_by_cds;
mod select_by_contig;
mod select_by_coord;
mod seq_dist;
mod sort_contigs;
mod split_by_cluster;
mod ssn_navigator;
mod structure_backend;
mod structure_dist;
mod structure_domainate;
mod structure_search;
mod structure_to_genbank;
mod summary_report;
mod transform_matrix;
mod trim_contigs;
mod viewer;

pub use build_projection::BuildProjectionArgs;
pub use build_ssn::BuildSsnArgs;
pub use build_tree::BuildTreeArgs;
pub use clean_sequences::CleanSequencesArgs;
pub use color_genbank::ColorGenbankArgs;
pub use color_table_to_legend::ColorTableToLegendArgs;
pub use common::{CommonArgs, TaxonomyArgs};
pub use compare_contigs::CompareContigsArgs;
pub use db_download::{DbDownloadArgs, DB_CHOICES};
pub use deduplicate_genbank::DeduplicateGenbankArgs;
pub use domain_search::DomainSearchArgs;
pub use domainate::DomainateArgs;
pub use enum_report::EnumReportArgs;
pub use extract_domains::ExtractDomainsArgs;
pub use extract_peptides::ExtractPeptidesArgs;
pub use extract_unannotated::ExtractUnannotatedArgs;
pub use filter_domains::FilterDomainsArgs;
pub use find_features::FindFeaturesArgs;
pub use format_db::FormatDbArgs;
pub use genbank_to_fasta::GenbankToFastaArgs;
pub use gene_call::GeneCallArgs;
pub use hmmer_build::HmmerBuildArgs;
pub use hmmer_compare::HmmerCompareArgs;
pub use hmmer_report::HmmerReportArgs;
pub use hmmer_search::HmmerSearchArgs;
pub use hmmer_select::HmmerSelectArgs;
pub use infernal_build::InfernalBuildArgs;
pub use kmer_dist::KmerDistArgs;
pub use matrix_report::MatrixReportArgs;
pub use plot_contigs::PlotContigsArgs;
pub use rotate_contigs::RotateContigsArgs;
pub use select_by_cds::SelectByCdsArgs;
pub use select_by_contig::SelectByContigArgs;
pub use select_by_coord::SelectByCoordArgs;
pub use seq_dist::SeqDistArgs;
pub use sort_contigs::SortContigsArgs;
pub use split_by_cluster::SplitByClusterArgs;
pub use ssn_navigator::SsnNavigatorArgs;
pub use structure_backend::{StructureAlignmentArgs, StructureBackendArgs};
pub use structure_dist::StructureDistArgs;
pub use structure_domainate::StructureDomainateArgs;
pub use structure_search::StructureSearchArgs;
pub use structure_to_genbank::StructureToGenbankArgs;
pub use summary_report::SummaryReportArgs;
pub use transform_matrix::TransformMatrixArgs;
pub use trim_contigs::TrimContigsArgs;
pub use viewer::ViewerArgs;

/// The `clap::Command` of a tool, built from its args struct alone, for
/// callers that have no CLI to ask (the GUI's tool dialogs).
pub fn tool_command(name: &str) -> Option<clap::Command> {
    use clap::Args;
    let cmd = clap::Command::new(crate::registry::tool_info(name)?.name);
    Some(match name {
        "sort_contigs" => SortContigsArgs::augment_args(cmd),
        "split_by_cluster" => SplitByClusterArgs::augment_args(cmd),
        "enum_report" => EnumReportArgs::augment_args(cmd),
        "hmmer_report" => HmmerReportArgs::augment_args(cmd),
        "select_by_contig" => SelectByContigArgs::augment_args(cmd),
        "select_by_cds" => SelectByCdsArgs::augment_args(cmd),
        "select_by_coord" => SelectByCoordArgs::augment_args(cmd),
        "filter_domains" => FilterDomainsArgs::augment_args(cmd),
        "find_features" => FindFeaturesArgs::augment_args(cmd),
        "extract_domains" => ExtractDomainsArgs::augment_args(cmd),
        "extract_peptides" => ExtractPeptidesArgs::augment_args(cmd),
        "extract_unannotated" => ExtractUnannotatedArgs::augment_args(cmd),
        "trim_contigs" => TrimContigsArgs::augment_args(cmd),
        "rotate_contigs" => RotateContigsArgs::augment_args(cmd),
        "color_genbank" => ColorGenbankArgs::augment_args(cmd),
        "clean_sequences" => CleanSequencesArgs::augment_args(cmd),
        "genbank_to_fasta" => GenbankToFastaArgs::augment_args(cmd),
        "deduplicate_genbank" => DeduplicateGenbankArgs::augment_args(cmd),
        "domainate" => DomainateArgs::augment_args(cmd),
        "domain_search" => DomainSearchArgs::augment_args(cmd),
        "seq_dist" => SeqDistArgs::augment_args(cmd),
        "kmer_dist" => KmerDistArgs::augment_args(cmd),
        "structure_dist" => StructureDistArgs::augment_args(cmd),
        "structure_domainate" => StructureDomainateArgs::augment_args(cmd),
        "structure_search" => StructureSearchArgs::augment_args(cmd),
        "structure_to_genbank" => StructureToGenbankArgs::augment_args(cmd),
        "db_download" => DbDownloadArgs::augment_args(cmd),
        "compare_contigs" => CompareContigsArgs::augment_args(cmd),
        "transform_matrix" => TransformMatrixArgs::augment_args(cmd),
        "build_ssn" => BuildSsnArgs::augment_args(cmd),
        "build_tree" => BuildTreeArgs::augment_args(cmd),
        "build_projection" => BuildProjectionArgs::augment_args(cmd),
        "viewer" => ViewerArgs::augment_args(cmd),
        "matrix_report" => MatrixReportArgs::augment_args(cmd),
        "ssn_navigator" => SsnNavigatorArgs::augment_args(cmd),
        "summary_report" => SummaryReportArgs::augment_args(cmd),
        "hmmer_compare" => HmmerCompareArgs::augment_args(cmd),
        "hmmer_select" => HmmerSelectArgs::augment_args(cmd),
        "hmmer_build" => HmmerBuildArgs::augment_args(cmd),
        "hmmer_search" => HmmerSearchArgs::augment_args(cmd),
        "infernal_build" => InfernalBuildArgs::augment_args(cmd),
        "format_db" => FormatDbArgs::augment_args(cmd),
        "color_table_to_legend" => ColorTableToLegendArgs::augment_args(cmd),
        "plot_contigs" => PlotContigsArgs::augment_args(cmd),
        "gene_call" => GeneCallArgs::augment_args(cmd),
        _ => return None,
    })
}
