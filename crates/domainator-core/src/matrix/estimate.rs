//! Output-size estimation for matrix writes, mirroring
//! `domainator.data_matrix.DataMatrix.estimate_write_size` (and its
//! `estimate_dense_hdf5_size` / `estimate_sparse_hdf5_size` /
//! `estimate_dense_text_size` helpers). Used by the output-size guardrails of
//! `seq_dist`, `compare_contigs`, and `transform_matrix` to fail *before* writing
//! a matrix that would exceed the configured `--max_output_gb` limit.
//!
//! Estimates match the byte layout of `hdf5_io`/`text`: dense HDF5 stores the
//! full `nrows*ncols` f64 buffer; sparse CSR stores f64 values (8 bytes),
//! int32 indices (4 bytes), and an int32 indptr (4 bytes); the dense-text writer
//! uses `text::py_float_str` (the NumPy `str(float64)` replica) for cell values.

use super::{DataMatrix, Storage};
use crate::matrix::text::py_float_str;

const FILE_OVERHEAD: u64 = 8 * 1024;
const DATASET_OVERHEAD: u64 = 4 * 1024;
const ATTR_OVERHEAD: u64 = 2 * 1024;

/// Estimated on-disk size of a variable-length UTF-8 label dataset.
fn label_dataset_size(labels: &[String]) -> u64 {
    if labels.is_empty() {
        return DATASET_OVERHEAD;
    }
    let encoded: u64 = labels.iter().map(|l| l.len() as u64).sum();
    DATASET_OVERHEAD + encoded + 16 * labels.len() as u64
}

/// Estimated on-disk size of an optional int64 lengths array (0 when absent).
fn optional_array_size(arr: Option<&Vec<i64>>) -> u64 {
    match arr {
        None => 0,
        Some(v) => DATASET_OVERHEAD + (v.len() as u64) * 8,
    }
}

/// CSR entry count (`nnz`) that the sparse writer would emit for `m`: for an
/// already-sparse matrix that is `values.len()`; for a dense matrix it is the
/// number of non-zero cells (the writer eliminates zeros).
fn csr_nnz(m: &DataMatrix) -> u64 {
    match &m.storage {
        Storage::Sparse { values, .. } => values.len() as u64,
        Storage::Dense(d) => d.iter().filter(|&&v| v != 0.0).count() as u64,
    }
}

/// Estimated size of the dense HDF5 output for `m`.
pub fn estimate_dense_hdf5_size(m: &DataMatrix) -> u64 {
    let data_bytes = (m.nrows() as u64) * (m.ncols() as u64) * 8;
    let mut size = FILE_OVERHEAD + ATTR_OVERHEAD + DATASET_OVERHEAD + data_bytes;
    size += label_dataset_size(&m.rows);
    size += optional_array_size(m.row_lengths.as_ref());
    if !m.symmetric_labels() {
        size += label_dataset_size(&m.cols);
        size += optional_array_size(m.col_lengths.as_ref());
    }
    size += m.data_type.len() as u64;
    size
}

/// Estimated size of the sparse (CSR) HDF5 output for `m`.
pub fn estimate_sparse_hdf5_size(m: &DataMatrix) -> u64 {
    let nnz = csr_nnz(m);
    let indptr_len = m.nrows() as u64 + 1;
    // values: f64 (8), indices: int32 (4), indptr: int32 (4).
    let mut size = FILE_OVERHEAD + ATTR_OVERHEAD + 3 * DATASET_OVERHEAD;
    size += nnz * 8 + nnz * 4 + indptr_len * 4;
    size += label_dataset_size(&m.rows);
    size += optional_array_size(m.row_lengths.as_ref());
    if !m.symmetric_labels() {
        size += label_dataset_size(&m.cols);
        size += optional_array_size(m.col_lengths.as_ref());
    }
    size += m.data_type.len() as u64;
    size
}

/// Estimated size of the dense-text (TSV) output for `m`. Matches
/// `text::write_dense_text`: a `"\t" + cols.join("\t") + "\n"` header, then per
/// row `row_label` followed by a tab and `py_float_str(cell)` for every column,
/// then a newline.
pub fn estimate_dense_text_size(m: &DataMatrix) -> u64 {
    let (nr, nc) = (m.nrows(), m.ncols());
    // Header: leading tab + tab-joined column labels + newline.
    let mut total: u64 = 1 + m.cols.join("\t").len() as u64 + 1;
    let dense = m.densify();
    for r in 0..nr {
        // row_label, then for each column a tab + formatted cell, then newline.
        total += m.rows[r].len() as u64 + nc as u64 + 1;
        for c in 0..nc {
            total += py_float_str(dense[r * nc + c]).len() as u64;
        }
    }
    total
}

/// Estimate the write size for a given output type (`"dense"`, `"sparse"`,
/// `"dense_text"`), mirroring `DataMatrix.estimate_write_size`.
pub fn estimate_write_size(output_type: &str, m: &DataMatrix) -> u64 {
    match output_type {
        "dense" => estimate_dense_hdf5_size(m),
        "sparse" => estimate_sparse_hdf5_size(m),
        "dense_text" => estimate_dense_text_size(m),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2x2 symmetric-labelled matrix `[[1,0],[0,2]]` with `data_type="score"`.
    fn tiny() -> DataMatrix {
        DataMatrix {
            rows: vec!["a".into(), "b".into()],
            cols: vec!["a".into(), "b".into()],
            row_lengths: None,
            col_lengths: None,
            data_type: "score".into(),
            storage: Storage::Dense(vec![1.0, 0.0, 0.0, 2.0]),
        }
    }

    #[test]
    fn dense_estimate_matches_hand_calc() {
        // 8192 + 2048 + 4096 + (2*2*8) data + label(4096+2+16*2) + 0 col (symmetric) + 5 data_type
        assert_eq!(estimate_dense_hdf5_size(&tiny()), 18503);
        assert_eq!(estimate_write_size("dense", &tiny()), 18503);
    }

    #[test]
    fn sparse_estimate_matches_hand_calc() {
        // nnz = 2 (zeros eliminated), indptr_len = 3.
        // 8192 + 2048 + 3*4096 + 2*8 + 2*4 + 3*4 + label(4130) + 5
        assert_eq!(estimate_sparse_hdf5_size(&tiny()), 26699);
        assert_eq!(estimate_write_size("sparse", &tiny()), 26699);
    }

    #[test]
    fn dense_text_estimate_matches_real_output() {
        // header "\ta\tb\n" = 5; each row "x\t1.0\t0.0\n" = 10; two rows -> 25.
        let est = estimate_dense_text_size(&tiny());
        assert_eq!(est, 25);
        assert_eq!(estimate_write_size("dense_text", &tiny()), 25);
        // Cross-check against the actual serialized bytes the writer produces.
        let dir = std::env::temp_dir();
        let path = dir.join("domainator_estimate_test.tsv");
        crate::matrix::text::write_dense_text(path.to_str().unwrap(), &tiny()).unwrap();
        let actual = std::fs::read(&path).unwrap().len() as u64;
        std::fs::remove_file(&path).ok();
        assert_eq!(est, actual);
    }

    #[test]
    fn asymmetric_counts_col_labels() {
        let mut m = tiny();
        m.cols = vec!["c".into(), "d".into()];
        // Now not symmetric: dense estimate gains a second label dataset (4130).
        assert_eq!(estimate_dense_hdf5_size(&m), 18503 + 4130);
    }
}
