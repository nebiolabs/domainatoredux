//! `transform-matrix`: normalize / convert / sparsify a score matrix. Mirrors
//! `domainator.transform_matrix`: apply a `--mode` value transform (only from a
//! `score` matrix), optionally `--mst_knn` sparsify and `--lb` threshold, then
//! the CLI writes the result as dense HDF5 / dense TSV / sparse HDF5.

use std::io;

use crate::matrix::mst_knn::apply_mst_knn;
use crate::matrix::transform::{apply_lower_bound, transform_values};
use crate::matrix::DataMatrix;

/// Pass-through mode name (`--mode none`): keep values and `data_type` as-is.
pub const PASS_THROUGH_MODE: &str = "none";

/// The selectable `--mode` values (matches Python's `choices`; note `rank` is not
/// exposed by the Python CLI either, so it is intentionally omitted).
pub const MODES: [&str; 8] = [
    "none",
    "score",
    "bool",
    "norm_score",
    "row_norm_score",
    "score_dist",
    "efi_score",
    "efi_score_dist",
];

/// Options for [`transform_matrix`].
pub struct TransformMatrixOptions {
    /// Output value type / transform. `"none"` preserves the input.
    pub mode: String,
    /// Optional override of the input matrix's `data_type`.
    pub input_type: Option<String>,
    /// Zero out values `<= lb` after the mode transform.
    pub lb: Option<f64>,
    /// Keep only MST + OR-symmetric kNN edges (post-transform values).
    pub mst_knn: Option<usize>,
}

/// Transform `m` per `opts` and return the result. Errors mirror the Python
/// tool: transforms are only supported starting from a `score` matrix, and
/// `--mst_knn` requires a square matrix.
pub fn transform_matrix(mut m: DataMatrix, opts: &TransformMatrixOptions) -> io::Result<DataMatrix> {
    // --input_type override of the declared data type.
    if let Some(it) = &opts.input_type {
        m.data_type = it.clone();
    }

    let requested_mode: Option<&str> =
        if opts.mode == PASS_THROUGH_MODE { None } else { Some(opts.mode.as_str()) };

    if let Some(mode) = requested_mode {
        if m.data_type.is_empty() {
            // Python warns and assumes 'score'.
            m.data_type = "score".to_string();
        }
        if m.data_type != "score" && m.data_type != mode {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "Input matrix format is '{}', but --mode is '{}'. Transforming between data \
                     types is only supported from a 'score' matrix.",
                    m.data_type, mode
                ),
            ));
        }
        if m.data_type == "score" && m.data_type != mode {
            transform_values(&mut m, mode);
        }
        m.data_type = mode.to_string();
    }

    if let Some(k) = opts.mst_knn {
        if m.nrows() != m.ncols() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "--mst_knn requires a square matrix."));
        }
        m = apply_mst_knn(&m, k, opts.lb.unwrap_or(0.0));
    }

    if let Some(lb) = opts.lb {
        apply_lower_bound(&mut m, lb);
    }

    Ok(m)
}
