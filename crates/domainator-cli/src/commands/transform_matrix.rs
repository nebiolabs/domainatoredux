use anyhow::{Context, Result};
use domainator_config::args::TransformMatrixArgs;
use domainator_core::matrix::read_matrix;
use domainator_core::tools::transform_matrix::{transform_matrix, TransformMatrixOptions};

use crate::output::{check_hdf_out, write_matrix_outputs};

pub fn run(args: &TransformMatrixArgs) -> Result<()> {
    check_hdf_out(args.hdf_out.as_deref())?;
    let max_bytes = crate::max_output_gb_to_bytes(args.max_output_gb.0)?;
    let matrix =
        read_matrix(&args.input).with_context(|| format!("reading matrix {}", args.input))?;
    let opts = TransformMatrixOptions {
        mode: args.mode.clone(),
        value_type: args.value_type.clone(),
        threshold: args.threshold.map(|t| t.0),
        mst: args.mst,
        knn: args.knn.map(|k| k.0 as usize),
        symmetrize: args.symmetrize.clone(),
    };
    let matrix = transform_matrix(matrix, &opts)?;
    write_matrix_outputs(
        &matrix,
        args.hdf_out.as_deref(),
        args.dense_text_out.as_deref(),
        max_bytes,
        &["--threshold", "--mst", "--knn"],
    )
}
