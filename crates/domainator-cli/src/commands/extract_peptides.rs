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
    let search_hits = args.annotation_type == "domain_search";
    // With no selection criteria every peptide is extracted, and --invert is ignored.
    let extract_all =
        target_domains.is_none() && target_cdss.is_none() && !args.unannotated && !search_hits;
    let mut invert = args.invert;
    if extract_all && invert {
        eprintln!("Invert supplied with no selection criteria. Invert will be ignored.");
        invert = false;
    }
    let opts = ExtractPeptidesOptions {
        evalue: args.evalue.map(|e| e.0),
        target_domains,
        target_cdss,
        search_hits,
        strand: args.strand.clone(),
        unannotated: args.unannotated,
        invert,
        databases: args.databases.as_ref().map(|d| d.iter().cloned().collect()),
        keep_name: args.keep_name,
        name_field: args.name_field.clone(),
        extract_all,
        partial: args.partial.parse().map_err(|e: String| anyhow!(e))?,
    };
    let mut records = load_seq_inputs(&args.input)?;
    crate::filter_contigs(&mut records, &args.contigs, args.contigs_file.as_deref())?;
    let out = extract_peptides(&records, &opts);
    write_seq_outputs(&out, args.genbank_out.as_deref(), args.fasta_out.as_deref())
}
