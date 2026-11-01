use anyhow::{anyhow, Result};
use domainator_config::args::ExtractDomainsArgs;
use domainator_core::tools::extract_domains::{extract_domains, ExtractDomainsOptions};
use domainator_core::tools::select_common::parse_name_list;

use super::load_seq_inputs;
use crate::output::write_seq_outputs;

pub fn run(args: &ExtractDomainsArgs) -> Result<()> {
    let extend = args.extend_range.unwrap_or(0);
    let opts = ExtractDomainsOptions {
        evalue: args.evalue.map(|e| e.0),
        score: args.min_score.map(|s| s.0),
        domains: parse_name_list(
            args.domains.as_deref().unwrap_or_default(),
            args.domains_file.as_deref(),
        )?,
        pad_up: args.extend_up.unwrap_or(extend),
        pad_down: args.extend_down.unwrap_or(extend),
        combine: args.combine,
        keep_direction: args.keep_direction,
        keep_parent_id: args.keep_parent_id,
        annotation_type: args
            .annotation_type
            .parse()
            .map_err(|e: String| anyhow!(e))?,
        has_search_hit: args.has_search_hit,
        databases: args.databases.as_ref().map(|d| d.iter().cloned().collect()),
        splice: args.splice,
        partial: args.partial.parse().map_err(|e: String| anyhow!(e))?,
        taxonomy: crate::taxonomy_filter(&args.taxonomy)?,
    };
    let mut records = load_seq_inputs(&args.input)?;
    crate::filter_contigs(&mut records, &args.contigs, args.contigs_file.as_deref())?;
    let out = extract_domains(&records, &opts).map_err(|e| anyhow!(e))?;
    write_seq_outputs(&out, args.genbank_out.as_deref(), args.fasta_out.as_deref())
}
