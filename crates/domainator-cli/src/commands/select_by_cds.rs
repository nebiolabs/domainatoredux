use std::collections::HashSet;

use anyhow::{anyhow, Result};
use domainator_config::args::SelectByCdsArgs;
use domainator_core::seq::SeqRecord;
use domainator_core::tools::neighborhood::validate_range;
use domainator_core::tools::select_by_cds::{pad_records, select_by_cds, SelectByCdsOptions};
use domainator_core::tools::select_common::parse_name_list;

use super::load_seq_inputs;
use crate::output::write_seq_outputs;

pub fn run(args: &SelectByCdsArgs) -> Result<()> {
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
        search_hits: args.annotation_type == "domain_search",
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

    let mut records = load_seq_inputs(&args.input)?;
    if let Some(set) = &target_contigs {
        records.retain(|r| set.contains(&r.id));
    }
    let mut regions = select_by_cds(&records, &opts).map_err(|e| anyhow!(e))?;
    if !args.no_deduplicate {
        let mut seen = HashSet::new();
        regions.retain(|r| seen.insert(r.record.id.clone()));
    }
    if args.pad {
        pad_records(&mut regions, anchor);
    }
    let out: Vec<SeqRecord> = regions.into_iter().map(|r| r.record).collect();
    write_seq_outputs(&out, args.genbank_out.as_deref(), args.fasta_out.as_deref())
}
