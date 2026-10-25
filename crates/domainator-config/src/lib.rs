//! The tools' interface: each tool's clap arguments (`args`), the registry of what
//! clap cannot express (`registry`), the `describe` manifest (`manifest`), `--config`
//! files and `--print_config` (`config_file`), and the report column catalogs
//! (`columns`).

use anyhow::{Context, Result};

pub mod args;
pub mod columns;
pub mod config_file;
pub mod manifest;
pub mod registry;
pub mod types;

/// A tool's manifest entry, as `domainatoredux describe` gives it: what the GUI renders
/// its dialogs from.
pub fn tool_manifest(tool: &str) -> Result<serde_json::Value> {
    let info = registry::tool_info(tool)
        .with_context(|| format!("unknown tool {tool:?} (`domainatoredux --help` lists them)"))?;
    let cmd =
        args::tool_command(tool).with_context(|| format!("{tool} has no options to describe"))?;
    Ok(manifest::describe_tool(&cmd, info))
}
