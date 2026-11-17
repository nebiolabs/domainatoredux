use anyhow::{anyhow, Result};
use domainator_config::args::ExtractPeptidesArgs;
use domainator_core::tools::extract_peptides::{ExtractPeptidesOptions, PeptideExtractor};
use domainator_core::tools::select_common::parse_name_list;

use crate::input::{flat_map_records, InputMode, SeqInputs};
use crate::output::write_seq_stream;

pub fn run(args: &ExtractPeptidesArgs) -> Result<()> {
    // Only validated here: the run's pool is sized from it before dispatch.
    super::cpu_threads(args.cpu)?;
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
    let extractor = PeptideExtractor::new(&opts).map_err(|e| anyhow!(e))?;
    let records = SeqInputs::new(&args.input, InputMode::DETECT)
        .contigs(&args.contigs, args.contigs_file.as_deref())?;
    let peptides = flat_map_records(records, |contig| {
        let mut out = Vec::new();
        extractor
            .extract(&contig, &mut out)
            .map_err(|e| anyhow!(e))?;
        Ok(out)
    });
    write_seq_stream(
        peptides,
        args.genbank_out.as_deref(),
        args.fasta_out.as_deref(),
        None,
    )?;
    Ok(())
}
