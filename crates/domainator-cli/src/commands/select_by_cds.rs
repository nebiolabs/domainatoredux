use std::collections::HashSet;

use anyhow::{anyhow, Result};
use domainator_config::args::SelectByCdsArgs;
use domainator_core::tools::dedup::{Dedup, RecordDeduplicator};
use domainator_core::tools::neighborhood::validate_range;
use domainator_core::tools::select_by_cds::{pad_records, CdsSelector, SelectByCdsOptions};
use domainator_core::tools::select_common::parse_name_list;

use crate::input::{InputMode, SeqInputs};
use crate::output::SeqWriter;

pub fn run(args: &SelectByCdsArgs) -> Result<()> {
    // Only validated here: the run's pool is sized from it before dispatch.
    super::cpu_threads(args.cpu)?;
    let range = validate_range(
        args.whole_contig,
        args.kb_range.map(|v| v.0),
        args.kb_up.map(|v| v.0),
        args.kb_down.map(|v| v.0),
        args.cds_range,
        args.cds_up,
        args.cds_down,
    )
    .map_err(|e| anyhow!(e))?;
    let target_contigs = parse_name_list(
        args.contigs.as_deref().unwrap_or_default(),
        args.contigs_file.as_deref(),
    )?;
    let target_cdss = parse_name_list(
        args.cds.as_deref().unwrap_or_default(),
        args.cds_file.as_deref(),
    )?;
    let target_domains = parse_name_list(
        args.domains.as_deref().unwrap_or_default(),
        args.domains_file.as_deref(),
    )?;
    let anchor = args.anchor.parse().map_err(|e: String| anyhow!(e))?;
    let opts = SelectByCdsOptions {
        target_cdss,
        target_domains,
        domain_expr: args.domain_expr.clone(),
        range,
        normalize_direction: !args.keep_direction,
        evalue: args.evalue.map(|e| e.0),
        invert: args.invert,
        annotation_type: args
            .annotation_type
            .parse()
            .map_err(|e: String| anyhow!(e))?,
        has_search_hit: args.has_search_hit,
        name_field: args.name_field.clone(),
        taxonomy: crate::taxonomy_filter(&args.taxonomy)?,
        max_region_overlap: args.max_region_overlap.0,
        strand: match args.strand.as_deref() {
            Some("f") => Some(1),
            Some("r") => Some(-1),
            _ => None,
        },
        databases: args
            .databases
            .as_ref()
            .map(|d| d.iter().cloned().collect::<HashSet<_>>()),
        unannotated: args.unannotated,
        include_nucleic_acids: args.include_nucleic_acids,
        partial: args.partial.parse().map_err(|e: String| anyhow!(e))?,
    };

    let mut records = SeqInputs::new(&args.input, InputMode::DETECT);
    if let Some(set) = target_contigs {
        records = records.with_contigs(set);
    }
    let selector = CdsSelector::new(&opts).map_err(|e| anyhow!(e))?;
    // Upstream `deduplicate_and_pad`: the --dedup policy, before padding.
    let dedup: Dedup = args.dedup.parse().map_err(|e: String| anyhow!(e))?;
    let mut deduplicator = RecordDeduplicator::new(dedup);
    let mut out = SeqWriter::new(args.genbank_out.as_deref(), args.fasta_out.as_deref(), None)?;
    // `--pad` aligns the focus CDSs across every region, so only then are they held;
    // otherwise each region is written as it is found.
    let mut held = Vec::new();
    for rec in records {
        for region in selector.select(&rec?).map_err(|e| anyhow!(e))? {
            if deduplicator.keep(&region.record).map_err(|e| anyhow!(e))? {
                if args.pad {
                    held.push(region);
                } else {
                    out.write(&region.record)?;
                }
            }
        }
    }
    if args.pad {
        pad_records(&mut held, anchor);
        for region in &held {
            out.write(&region.record)?;
        }
    }
    out.finish()?;
    Ok(())
}
