use std::collections::HashSet;

use anyhow::{anyhow, bail, Result};
use domainator_config::args::SelectByContigArgs;
use domainator_core::seq::taxonomy::default_ncbi_taxonomy_path;
use domainator_core::tools::select_by_contig::{select_by_contig, SelectByContigOptions};
use domainator_core::tools::select_common::parse_name_list;

use super::load_seq_inputs;
use crate::output::write_seq_outputs;

fn set<T: Clone + Eq + std::hash::Hash>(list: &Option<Vec<T>>) -> Option<HashSet<T>> {
    list.as_ref().map(|v| v.iter().cloned().collect())
}

pub fn run(args: &SelectByContigArgs) -> Result<()> {
    let target_contigs = parse_name_list(
        args.contigs.as_deref().unwrap_or_default(),
        args.contigs_file.as_deref(),
    )?;
    let target_domains = parse_name_list(
        args.domains.as_deref().unwrap_or_default(),
        args.domains_file.as_deref(),
    )?;
    let has_taxids = args.include_taxids.is_some() || args.exclude_taxids.is_some();
    if args.taxonomy_expr.is_some() && has_taxids {
        bail!("--taxonomy_expr is mutually exclusive with --include_taxids/--exclude_taxids.");
    }
    let taxonomy = if has_taxids || args.taxonomy_expr.is_some() {
        let path = args
            .ncbi_taxonomy_path
            .clone()
            .unwrap_or_else(default_ncbi_taxonomy_path);
        Some(crate::load_taxonomy(&path)?)
    } else {
        None
    };
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
        include_taxids: set(&args.include_taxids),
        exclude_taxids: set(&args.exclude_taxids),
        taxonomy_expr: args.taxonomy_expr.clone(),
        databases: set(&args.databases),
        unannotated: args.unannotated,
        search_score_lb: args.min_search_score.map(|s| s.0),
        search_score_ub: args.max_search_score.map(|s| s.0),
        annotation_type: args
            .annotation_type
            .parse()
            .map_err(|e: String| anyhow!(e))?,
        partial: args.partial.parse().map_err(|e: String| anyhow!(e))?,
    };
    let records = load_seq_inputs(&args.input)?;
    let selected = select_by_contig(records, taxonomy.as_ref(), &opts).map_err(|e| anyhow!(e))?;
    write_seq_outputs(
        &selected,
        args.genbank_out.as_deref(),
        args.fasta_out.as_deref(),
    )
}
