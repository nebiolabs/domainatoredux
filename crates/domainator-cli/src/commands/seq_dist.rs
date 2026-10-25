use std::path::Path;

use anyhow::{anyhow, Context, Result};
use domainator_config::args::SeqDistArgs;
use domainator_core::tools::seq_dist::{
    detect_kind, parse_diamond_args, seq_dist, InputKind, SeqDistOptions,
};

use super::cpu_threads;
use crate::output::{check_hdf_out, write_matrix_outputs};

pub fn run(args: &SeqDistArgs) -> Result<()> {
    check_hdf_out(args.hdf_out.as_deref())?;
    let max_bytes = crate::max_output_gb_to_bytes(args.max_output_gb.0)?;
    let tokens = match &args.diamond_args {
        Some(s) => shlex::split(s)
            .ok_or_else(|| anyhow!("Could not split --diamond_args into options: {s:?}"))?,
        None => Vec::new(),
    };
    let diamond_args = parse_diamond_args(&tokens).map_err(|e| anyhow!(e))?;
    if !diamond_args.is_empty() && !args.algorithm.starts_with("diamond") {
        anyhow::bail!(
            "--diamond_args only applies to the diamond_* algorithms, but --algorithm is '{}'.",
            args.algorithm
        );
    }

    let input_kind = kind(&args.input, args.input_format.as_deref())?;
    let reference_kind = args
        .reference
        .as_deref()
        .map(|r| kind(r, args.reference_format.as_deref()))
        .transpose()?;
    let opts = SeqDistOptions {
        algorithm: args.algorithm.clone(),
        mode: args.mode.clone(),
        threshold: args.threshold.0,
        // A cap below 1 keeps every hit, as diamond's `--max-target-seqs 0` does.
        max_hits_per_query: args
            .max_hits_per_query
            .and_then(|k| usize::try_from(k).ok())
            .filter(|&k| k > 0),
        mst: args.mst,
        knn: args.knn.map(|k| k.0 as usize),
        symmetrize: args.symmetrize.clone(),
        diamond_args,
        diamond_path: args.diamond_path.clone(),
        cpu: cpu_threads(args.cpu)?,
    };
    let matrix = seq_dist(
        &args.input,
        input_kind,
        args.reference.as_deref(),
        reference_kind,
        &opts,
    )?;
    write_matrix_outputs(
        &matrix,
        args.hdf_out.as_deref(),
        args.dense_text_out.as_deref(),
        max_bytes,
        &["--max_hits_per_query", "--threshold", "--mst", "--knn"],
    )
}

/// The kind of a `seq_dist` input: its `--*_format`, else read off the file name.
fn kind(path: &str, format: Option<&str>) -> Result<InputKind> {
    match format {
        Some("fasta") => Ok(InputKind::Fasta),
        Some("genbank") => Ok(InputKind::Genbank),
        Some("hmm") => Ok(InputKind::Hmm),
        _ => detect_kind(Path::new(path)).with_context(|| {
            format!("could not tell the file type of {path}; give its format (--input_format or --reference_format)")
        }),
    }
}
