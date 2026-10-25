//! Each migrated tool's interface, one clap `Args` struct per tool.
//!
//! The struct is the tool's single interface description: the command line parses
//! into it, a `--config` file is expanded into the same command line
//! ([`crate::config_file`]), `--print_config` and `describe` read it back through clap
//! ([`crate::manifest`]), and the GUI renders its dialog from that manifest. Field
//! names are upstream's parameter names, which are also the long flags (every struct
//! is `rename_all = "snake_case"`) and the config-file keys.

mod common;
mod sort_contigs;

pub use common::CommonArgs;
pub use sort_contigs::SortContigsArgs;

/// The `clap::Command` of a migrated tool, built from its args struct alone, for
/// callers that have no CLI to ask (the config schema the GUI renders from).
pub fn tool_command(name: &str) -> Option<clap::Command> {
    use clap::Args;
    let cmd = clap::Command::new(crate::registry::tool_info(name)?.name);
    Some(match name {
        "sort_contigs" => SortContigsArgs::augment_args(cmd),
        _ => return None,
    })
}
