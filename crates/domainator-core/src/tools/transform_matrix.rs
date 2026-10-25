//! `transform_matrix`: normalize / convert / sparsify a score matrix. Mirrors
//! `domainator.transform_matrix`: optionally `--symmetrize`, apply a `--mode` value
//! transform (only from a `score` matrix), optionally `--mst`/`--knn` sparsify and
//! `--threshold`; the CLI writes the result.

use std::io;

use crate::matrix::mst_knn::apply_mst_knn;
use crate::matrix::transform::{apply_lower_bound, symmetrize, transform_values};
use crate::matrix::DataMatrix;
use crate::tools::seq_dist::sparsify_option_label;

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
#[derive(Default)]
pub struct TransformMatrixOptions {
    /// Output value type / transform. `"none"` preserves the input.
    pub mode: String,
    /// `--value_type`: what the input's values are, overriding the file's type.
    pub value_type: Option<String>,
    /// `--threshold`: zero out values `<= threshold` after the mode transform.
    pub threshold: Option<f64>,
    /// `--mst`: keep the maximum spanning forest's edges (with `knn`, the union).
    pub mst: bool,
    /// `--knn`: keep the OR-symmetric k-nearest-neighbor edges.
    pub knn: Option<usize>,
    /// `--symmetrize`: `none`, `max`, `min` or `mean`, applied before `mode`.
    pub symmetrize: String,
}

fn invalid(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, msg)
}

/// Whether `data_type` holds distances, where 0 means identical (upstream
/// `is_distance_data_type`).
fn is_distance(data_type: &str) -> bool {
    data_type.ends_with("_dist")
}

/// Keep `m`'s maximum spanning forest and/or kNN edges (upstream `sparsify_matrix`).
fn sparsify(
    m: &DataMatrix,
    opts: &TransformMatrixOptions,
    lower_bound: f64,
) -> io::Result<DataMatrix> {
    if m.nrows() != m.ncols() {
        return Err(invalid(format!(
            "{} requires a square matrix.",
            sparsify_option_label(opts.mst, opts.knn)
        )));
    }
    Ok(apply_mst_knn(
        m,
        opts.knn.unwrap_or(0),
        lower_bound,
        opts.mst,
    ))
}

/// Transform `m` per `opts` and return the result, as upstream's `main` does:
/// `--symmetrize`, then the `--mode` transform (only from a `score` matrix), then
/// `--mst`/`--knn`, then `--threshold`.
pub fn transform_matrix(
    mut m: DataMatrix,
    opts: &TransformMatrixOptions,
) -> io::Result<DataMatrix> {
    if let Some(value_type) = &opts.value_type {
        if !m.data_type.is_empty() && m.data_type != *value_type {
            eprintln!(
                "Warning: Input matrix format is '{}', but --value_type is '{value_type}'. The \
                 input matrix format is overridden.",
                m.data_type
            );
        }
        m.data_type = value_type.clone();
    }
    let requested_mode = (opts.mode != PASS_THROUGH_MODE).then_some(opts.mode.as_str());
    let mut sparsifying = opts.mst || opts.knn.is_some();
    let label = sparsify_option_label(opts.mst, opts.knn);

    if opts.symmetrize != "none" {
        if sparsifying {
            return Err(invalid(format!(
                "{label} output is always max-symmetric, so --symmetrize {} cannot be applied \
                 on top of it.",
                opts.symmetrize
            )));
        }
        if m.rows != m.cols {
            return Err(invalid(
                "--symmetrize requires a square matrix with the same row and column labels.".into(),
            ));
        }
        if is_distance(&m.data_type) {
            return Err(invalid(format!(
                "--symmetrize reconciles similarity values; the input is a '{}' distance matrix. \
                 Symmetrize the score matrix, then convert it.",
                m.data_type
            )));
        }
        symmetrize(&mut m, &opts.symmetrize).map_err(invalid)?;
    }

    if let Some(mode) = requested_mode {
        if m.data_type.is_empty() {
            eprintln!("Warning: Input matrix format not specified, assuming 'score'");
            m.data_type = "score".to_string();
        }
        if m.data_type != "score" && m.data_type != mode {
            return Err(invalid(format!(
                "Input matrix format is '{}', but --mode is '{mode}'. Transforming between data \
                 types is only supported from a 'score' matrix.",
                m.data_type
            )));
        }
        // Every bool cell is 1, which would leave --mst/--knn nothing to rank, so select
        // on the scores and keep the mask afterward.
        if sparsifying && mode == "bool" && m.data_type == "score" {
            m = sparsify(&m, opts, 0.0)?;
            sparsifying = false;
        }
        if m.data_type == "score" && m.data_type != mode {
            transform_values(&mut m, mode);
        }
        m.data_type = mode.to_string();
    }

    if sparsifying {
        if let Some(score_type) = m.data_type.strip_suffix("_dist") {
            return Err(invalid(format!(
                "{label} keeps the highest-valued edges, so it cannot be applied to a '{}' \
                 (distance) matrix. Sparsify the '{score_type}' matrix instead.",
                m.data_type
            )));
        }
        m = sparsify(&m, opts, opts.threshold.unwrap_or(0.0))?;
    }

    if let Some(threshold) = opts.threshold {
        if is_distance(&m.data_type) {
            return Err(invalid(format!(
                "--threshold zeroes out low values, which in a '{}' (distance) matrix are the \
                 closest pairs (0 means identical). Apply --threshold to the score matrix \
                 instead.",
                m.data_type
            )));
        }
        apply_lower_bound(&mut m, threshold);
    }

    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matrix::Storage;

    fn score_matrix(d: Vec<f64>) -> DataMatrix {
        let names: Vec<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
        DataMatrix {
            rows: names.clone(),
            cols: names,
            row_lengths: None,
            col_lengths: None,
            data_type: "score".into(),
            storage: Storage::Dense(d),
        }
    }

    /// `--mode bool --knn 1` keeps each node's best-*scoring* neighbor and then marks
    /// it 1; ranking the 1s instead would keep an arbitrary one (upstream's order).
    #[test]
    fn bool_pruning_selects_on_the_scores() {
        #[rustfmt::skip]
        let m = score_matrix(vec![
            0.0, 1.0, 9.0,
            1.0, 0.0, 2.0,
            9.0, 2.0, 0.0,
        ]);
        let opts = TransformMatrixOptions {
            mode: "bool".into(),
            knn: Some(1),
            symmetrize: "none".into(),
            ..Default::default()
        };
        let out = transform_matrix(m, &opts).unwrap();
        assert_eq!(out.data_type, "bool");
        // a-c (9) is a's and c's best edge; b-c (2) is b's. a-b (1) is nobody's.
        assert_eq!(out.densify(), [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0]);
    }
}
