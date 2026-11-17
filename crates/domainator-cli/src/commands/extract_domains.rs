use anyhow::{anyhow, Result};
use domainator_config::args::ExtractDomainsArgs;
use domainator_core::tools::extract_domains::{
    check_extract_domains_options, extract_domains_from_record, ExtractDomainsOptions,
};
use domainator_core::tools::select_common::parse_name_list;

use crate::input::{flat_map_records, InputMode, SeqInputs};
use crate::output::write_seq_stream;

pub fn run(args: &ExtractDomainsArgs) -> Result<()> {
    // Only validated here: the run's pool is sized from it before dispatch.
    super::cpu_threads(args.cpu)?;
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
    check_extract_domains_options(&opts).map_err(|e| anyhow!(e))?;
    let records = SeqInputs::new(&args.input, InputMode::DETECT)
        .contigs(&args.contigs, args.contigs_file.as_deref())?;
    let regions = flat_map_records(records, |rec| {
        let mut out = Vec::new();
        extract_domains_from_record(&rec, &opts, &mut out).map_err(|e| anyhow!(e))?;
        Ok(out)
    });
    write_seq_stream(
        regions,
        args.genbank_out.as_deref(),
        args.fasta_out.as_deref(),
        None,
    )?;
    Ok(())
}
