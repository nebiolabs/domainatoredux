use anyhow::{bail, Result};
use domainator_config::args::StructureDistArgs;
use domainator_core::matrix::mst_knn::sparsify_option_label;
use domainator_core::tools::structure_dist::{
    is_structural_mode, mode_metrics, reject_mode_on_backend, structure_dist, StructureDistOptions,
    SPARSIFY_STREAMABLE_MODES,
};

use super::structure_to_genbank::{aligner, check_backend, max_seqs};
use crate::output::{check_hdf_out, write_matrix_outputs};

pub fn run(args: &StructureDistArgs) -> Result<()> {
    if args.search.metrics.is_some() {
        bail!(
            "structure_dist derives the structural metric from --mode (tmscore, lddt, fident), \
             so --metrics is rejected here rather than looking as though it changes the matrix."
        );
    }
    let max_hits_per_query = args
        .max_hits_per_query
        .map(|k| {
            usize::try_from(k).map_err(|_| anyhow::anyhow!("--max_hits_per_query must be >= 0"))
        })
        .transpose()?;
    check_hdf_out(args.hdf_out.as_deref(), None)?;
    let mode = args.mode.as_str();
    if is_structural_mode(mode) && !(0.0..=1.0).contains(&args.threshold.0) {
        bail!("--threshold must be between 0 and 1 for --mode {mode}.");
    }
    // A reference equal to the input is a self-comparison, which builds one database.
    let references = args.reference.as_ref().filter(|r| **r != args.input);
    let self_comparison = references.is_none();

    let knn = args.knn.map(|k| k.0 as usize);
    if args.mst || knn.is_some() {
        let option = sparsify_option_label(args.mst, knn);
        if !self_comparison {
            bail!(
                "{option} requires a square, symmetric comparison: omit -r, or pass the same \
                 value for -i and -r."
            );
        }
        if !SPARSIFY_STREAMABLE_MODES.contains(&mode) {
            bail!(
                "{option} is only supported with --mode [{}] (modes that normalize against \
                 global matrix maxima cannot be streamed). For '{mode}', run 'structure_dist \
                 --mode score --hdf_out out.hdf5' then 'transform_matrix -i out.hdf5 --mode \
                 {mode} {option} --hdf_out pruned.hdf5'.",
                SPARSIFY_STREAMABLE_MODES
                    .map(|m| format!("'{m}'"))
                    .join(", ")
            );
        }
        if args.symmetrize != "none" {
            bail!(
                "{option} output is always max-symmetric -- the accumulator selects edges on \
                 max(M[i,j], M[j,i]) and writes that value to both cells -- so --symmetrize {} \
                 cannot be applied on top of it. Drop --symmetrize, or run 'structure_dist \
                 --mode score --hdf_out out.hdf5' and prune with transform_matrix afterwards.",
                args.symmetrize
            );
        }
    } else if args.symmetrize != "none" && !self_comparison {
        bail!(
            "--symmetrize requires a square, symmetric comparison: omit -r, or pass the same \
             value for -i and -r."
        );
    }

    let aligner = aligner(&args.backend, Some(&args.search))?;
    reject_mode_on_backend(&aligner, mode)?;
    check_backend(&aligner, &args.search, &mode_metrics(mode))?;
    let max_bytes = crate::max_output_gb_to_bytes(args.max_output_gb.0)?;

    let opts = StructureDistOptions {
        mode: args.mode.clone(),
        evalue: args.evalue.0,
        max_hits_per_query,
        threshold: args.threshold.0,
        symmetrize: args.symmetrize.clone(),
        mst: args.mst,
        knn,
        alignment_type: args.search.alignment_type,
        max_seqs: max_seqs(&args.search)?,
        db_out: args.db_out.clone(),
        hits_tsv_out: args.hits_tsv_out.clone(),
        progress: args.progress,
    };
    let matrix = structure_dist(&args.input, references.map(Vec::as_slice), &aligner, &opts)?;
    write_matrix_outputs(
        &matrix,
        args.hdf_out.as_deref(),
        args.dense_text_out.as_deref(),
        max_bytes,
        &[
            "--max_hits_per_query",
            "--threshold",
            "-e/--evalue",
            "--mst",
            "--knn",
        ],
    )
}
