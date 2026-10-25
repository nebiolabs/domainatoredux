use anyhow::{anyhow, Result};
use domainator_config::args::CompareContigsArgs;
use domainator_core::tools::compare_contigs::{compare_contigs, CompareContigsOptions};
use domainator_core::tools::select_common::parse_name_list;

use super::{cpu_threads, load_seq_inputs};
use crate::output::{check_hdf_out, write_matrix_outputs};

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
    let matrix = compare_contigs(records, &opts).map_err(|e| anyhow!(e))?;
    write_matrix_outputs(
        &matrix,
        args.hdf_out.as_deref(),
        args.dense_text_out.as_deref(),
        max_bytes,
        &["--max_hits_per_query", "--mst", "--knn"],
    )
}
