use std::collections::HashSet;

use anyhow::{anyhow, Result};
use domainator_config::args::SelectByContigArgs;
use domainator_core::tools::select_by_contig::{ContigSelector, SelectByContigOptions};
use domainator_core::tools::select_common::parse_name_list;

use crate::input::{InputMode, SeqInputs};
use crate::output::write_seq_stream;

fn set<T: Clone + Eq + std::hash::Hash>(list: &Option<Vec<T>>) -> Option<HashSet<T>> {
    list.as_ref().map(|v| v.iter().cloned().collect())
}

pub fn run(args: &SelectByContigArgs) -> Result<()> {
    // Only validated here: the run's pool is sized from it before dispatch.
    super::cpu_threads(args.cpu)?;
    let target_contigs = parse_name_list(
        args.contigs.as_deref().unwrap_or_default(),
        args.contigs_file.as_deref(),
    )?;
    let target_domains = parse_name_list(
        args.domains.as_deref().unwrap_or_default(),
        args.domains_file.as_deref(),
    )?;
    let opts = SelectByContigOptions {
        target_domains,
        domain_expr: args.domain_expr.clone(),
        domain_evalue: args.evalue.map(|e| e.0),
        // Upstream stops when the count reaches max_records, which a count of 0 or
        // less never does.
        first: args
            .max_records
            .filter(|&n| n > 0)
            .map(|n| usize::try_from(n).unwrap_or(usize::MAX)),
        length_lb: args.min_length,
        length_ub: args.max_length,
        cds_count_lb: args.min_cds,
        cds_count_ub: args.max_cds,
        definition_regex: args.definition_regex.clone(),
        sequence_regex: args.sequence_regex.clone(),
        target_contigs,
        contigs_regex: args.contigs_regex.clone(),
        invert: args.invert,
        taxonomy: crate::taxonomy_filter(&args.taxonomy)?,
        databases: set(&args.databases),
        unannotated: args.unannotated,
        search_score_lb: args.min_search_score.map(|s| s.0),
        search_score_ub: args.max_search_score.map(|s| s.0),
        annotation_type: args
            .annotation_type
            .parse()
            .map_err(|e: String| anyhow!(e))?,
        has_search_hit: args.has_search_hit,
        partial: args.partial.parse().map_err(|e: String| anyhow!(e))?,
    };
    let selector = ContigSelector::new(&opts).map_err(|e| anyhow!(e))?;
    // Lazy, so with --max_records reading stops at the last record kept, as
    // upstream's generator does.
    let selected = SeqInputs::new(&args.input, InputMode::DETECT)
        .filter_map(|rec| {
            let rec = match rec {
                Ok(rec) => rec,
                Err(e) => return Some(Err(e)),
            };
            match selector.selects(&rec) {
                Ok(true) => Some(Ok(rec)),
                Ok(false) => None,
                Err(e) => Some(Err(anyhow!(e))),
            }
        })
        .take(opts.first.unwrap_or(usize::MAX));
    write_seq_stream(
        selected,
        args.genbank_out.as_deref(),
        args.fasta_out.as_deref(),
        None,
    )?;
    Ok(())
}
