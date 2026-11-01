use anyhow::{anyhow, Result};
use domainator_config::args::ExtractPeptidesArgs;
use domainator_core::tools::extract_peptides::{extract_peptides, ExtractPeptidesOptions};
use domainator_core::tools::select_common::parse_name_list;

use super::load_seq_inputs;
use crate::output::write_seq_outputs;

pub fn run(args: &ExtractPeptidesArgs) -> Result<()> {
    let target_domains = parse_name_list(
        args.domains.as_deref().unwrap_or_default(),
        args.domains_file.as_deref(),
    )?;
    let target_cdss = parse_name_list(
        args.cds.as_deref().unwrap_or_default(),
        args.cds_file.as_deref(),
    )?;
    // With no selectors every peptide is extracted, and --invert is ignored with a
    // warning (`extract_peptides` decides both).
    let opts = ExtractPeptidesOptions {
        evalue: args.evalue.map(|e| e.0),
        target_domains,
        target_cdss,
        domain_expr: args.domain_expr.clone(),
        annotation_type: args
            .annotation_type
            .parse()
            .map_err(|e: String| anyhow!(e))?,
        has_search_hit: args.has_search_hit,
        strand: args.strand.clone(),
        unannotated: args.unannotated,
        invert: args.invert,
        databases: args.databases.as_ref().map(|d| d.iter().cloned().collect()),
        keep_parent_id: args.keep_parent_id,
        name_field: args.name_field.clone(),
        extract_all: false,
        partial: args.partial.parse().map_err(|e: String| anyhow!(e))?,
        taxonomy: crate::taxonomy_filter(&args.taxonomy)?,
    };
    let mut records = load_seq_inputs(&args.input)?;
    crate::filter_contigs(&mut records, &args.contigs, args.contigs_file.as_deref())?;
    let out = extract_peptides(&records, &opts).map_err(|e| anyhow!(e))?;
    write_seq_outputs(&out, args.genbank_out.as_deref(), args.fasta_out.as_deref())
}
