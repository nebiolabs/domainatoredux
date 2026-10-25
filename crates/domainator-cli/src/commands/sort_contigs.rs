use std::collections::HashSet;

use anyhow::Result;
use domainator_config::args::SortContigsArgs;
use domainator_core::tools::select_common::parse_name_list;
use domainator_core::tools::sort_contigs::{sort_contigs, SortContigsOptions};

use super::{load_seq_inputs, usage_error};
use crate::output::write_seq_outputs;

pub fn run(args: &SortContigsArgs) -> Result<()> {
    let max_hits = match args.max_hits {
        Some(n) if n < 1 => {
            return Err(usage_error(
                "--max_hits must be at least 1. Leave it out to return all contigs.",
            ))
        }
        n => n.map(|n| usize::try_from(n).unwrap_or(usize::MAX)),
    };
    let domains = parse_name_list(
        args.domains.as_deref().unwrap_or_default(),
        args.domains_file.as_deref(),
    )?;
    // Databases only narrow a domain sort.
    let databases: Option<HashSet<String>> = domains
        .as_ref()
        .and(args.databases.as_ref())
        .map(|d| d.iter().cloned().collect());
    let records = load_seq_inputs(&args.input)?;
    let opts = SortContigsOptions {
        max_hits,
        domains,
        databases,
    };
    let sorted = sort_contigs(records, &opts).map_err(|e| anyhow::anyhow!(e))?;
    write_seq_outputs(
        &sorted,
        args.genbank_out.as_deref(),
        args.fasta_out.as_deref(),
    )
}
