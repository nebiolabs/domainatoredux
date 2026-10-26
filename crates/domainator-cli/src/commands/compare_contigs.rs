use anyhow::{anyhow, Result};
use domainator_config::args::CompareContigsArgs;
use domainator_core::tools::compare_contigs::{
    check_sort_feasible, compare_contigs, hierarchical_order, sorted_records, unique_records,
    CompareContigsOptions, LeafOrdering,
};
use domainator_core::tools::select_common::parse_name_list;

use super::{cpu_threads, load_seq_inputs};
use crate::output::{check_hdf_out, write_matrix_outputs, write_seq_outputs};

pub fn run(args: &CompareContigsArgs) -> Result<()> {
    let max_bytes = crate::max_output_gb_to_bytes(args.max_output_gb.0)?;
    check_hdf_out(args.hdf_out.as_deref())?;
    let opts = CompareContigsOptions {
        ji_weight: args.ji.0,
        ai_weight: args.ai.0,
        databases: args.databases.as_ref().map(|d| d.iter().cloned().collect()),
        // A negative cap keeps every hit, as no cap does.
        max_hits_per_query: args
            .max_hits_per_query
            .and_then(|k| usize::try_from(k).ok()),
        threshold: args.threshold.0,
        mst: args.mst,
        knn: args.knn.map(|k| k.0 as usize),
        cpu: cpu_threads(args.cpu)?,
    };
    let contigs = parse_name_list(
        args.contigs.as_deref().unwrap_or_default(),
        args.contigs_file.as_deref(),
    )?;
    let mut records = load_seq_inputs(&args.input)?;
    if let Some(contigs) = &contigs {
        records.retain(|r| contigs.contains(&r.id));
    }
    let records = unique_records(records);
    let leaf_ordering: LeafOrdering = args.leaf_ordering.parse().map_err(|e: String| anyhow!(e))?;
    if args.genbank_out.is_some() {
        // Fail before scoring if the GenBank output can't be sorted.
        check_sort_feasible(records.len(), leaf_ordering, !args.skip_sort_memory_check)
            .map_err(|e| anyhow!(e))?;
    }
    let matrix = compare_contigs(&records, &opts).map_err(|e| anyhow!(e))?;
    write_matrix_outputs(
        &matrix,
        args.hdf_out.as_deref(),
        args.dense_text_out.as_deref(),
        max_bytes,
        &["--max_hits_per_query", "--mst", "--knn"],
    )?;
    if let Some(path) = &args.genbank_out {
        let linkage = args.linkage.parse().map_err(|e: String| anyhow!(e))?;
        let order = hierarchical_order(&matrix, linkage, leaf_ordering).map_err(|e| anyhow!(e))?;
        let sorted = sorted_records(&records, &order, args.name_by_order);
        write_seq_outputs(&sorted, Some(path), None)?;
    }
    Ok(())
}
