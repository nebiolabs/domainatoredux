//! HDF5 persistence for [`DataMatrix`] via `hdf5-pure`, byte-compatible with
//! `domainator.data_matrix.DataMatrix.{write_dense,write_sparse}` so the Python
//! suite (`DataMatrix.from_file`) reads our output unchanged.
//!
//! Attributes: `MATRIX_FILE_VERSION="1.0"`, `ARRAY_TYPE` in {`DENSE`,`SPARSE_CSR`},
//! `SYMMETRIC_LABELS` (written as an int 1/0 — h5py reads it truthily), and
//! `DATA_TYPE`. Labels are UTF-8 variable-length strings; `COL_*` datasets are
//! omitted when labels are symmetric.

use std::io;
use std::path::Path;

use hdf5_pure::{AttrValue, FileBuilder};

use super::{DataMatrix, Storage};

const MATRIX_FILE_VERSION: &str = "1.0";
const ARRAY_TYPE_ATTR: &str = "ARRAY_TYPE";
const SYMMETRIC_LABELS_ATTR: &str = "SYMMETRIC_LABELS";
const MATRIX_FILE_VERSION_ATTR: &str = "MATRIX_FILE_VERSION";
const DATA_TYPE_ATTR: &str = "DATA_TYPE";

const DENSE: &str = "DENSE";
const SPARSE_CSR: &str = "SPARSE_CSR";

const ROW_LABELS: &str = "ROW_LABELS";
const COL_LABELS: &str = "COL_LABELS";
const ROW_LENGTHS: &str = "ROW_LENGTHS";
const COL_LENGTHS: &str = "COL_LENGTHS";
const DENSE_DATA: &str = "DENSE_DATA";
const SPARSE_VALUES: &str = "SPARSE_VALUES";
const SPARSE_CSR_INDICES: &str = "SPARSE_CSR_INDICES";
const SPARSE_CSR_INDPTR: &str = "SPARSE_CSR_INDPTR";

fn write_err(e: hdf5_pure::Error) -> io::Error {
    io::Error::new(io::ErrorKind::Other, format!("hdf5 write: {e:?}"))
}

fn add_labels_and_lengths(builder: &mut FileBuilder, m: &DataMatrix, symmetric: bool) {
    let row_refs: Vec<&str> = m.rows.iter().map(String::as_str).collect();
    builder.create_dataset(ROW_LABELS).with_vlen_strings(&row_refs);
    if let Some(rl) = &m.row_lengths {
        builder.create_dataset(ROW_LENGTHS).with_i64_data(rl);
    }
    if !symmetric {
        let col_refs: Vec<&str> = m.cols.iter().map(String::as_str).collect();
        builder.create_dataset(COL_LABELS).with_vlen_strings(&col_refs);
        if let Some(cl) = &m.col_lengths {
            builder.create_dataset(COL_LENGTHS).with_i64_data(cl);
        }
    }
}

/// Write `m` as a dense HDF5 matrix (`ARRAY_TYPE=DENSE`, `DENSE_DATA` is a
/// row-major `[nrows, ncols]` f64 dataset).
pub fn write_dense(path: &str, m: &DataMatrix) -> io::Result<()> {
    let (nr, nc) = (m.nrows(), m.ncols());
    let flat = m.densify();
    let symmetric = m.symmetric_labels();

    let mut builder = FileBuilder::new();
    builder.set_attr(MATRIX_FILE_VERSION_ATTR, AttrValue::String(MATRIX_FILE_VERSION.into()));
    builder.set_attr(ARRAY_TYPE_ATTR, AttrValue::String(DENSE.into()));
    builder
        .create_dataset(DENSE_DATA)
        .with_f64_data(&flat)
        .with_shape(&[nr as u64, nc as u64]);
    add_labels_and_lengths(&mut builder, m, symmetric);
    builder.set_attr(SYMMETRIC_LABELS_ATTR, AttrValue::I64(symmetric as i64));
    builder.set_attr(DATA_TYPE_ATTR, AttrValue::String(m.data_type.clone()));
    builder.write(Path::new(path)).map_err(write_err)
}

/// Write `m` as a CSR sparse HDF5 matrix (`ARRAY_TYPE=SPARSE_CSR`). The matrix is
/// stored canonically (zeros eliminated, indices sorted) by construction.
pub fn write_sparse(path: &str, m: &DataMatrix) -> io::Result<()> {
    let symmetric = m.symmetric_labels();
    // Obtain canonical CSR arrays (densify->rebuild if currently dense).
    let (values, indices, indptr) = match &m.storage {
        Storage::Sparse { values, indices, indptr } => (values.clone(), indices.clone(), indptr.clone()),
        Storage::Dense(d) => {
            let (nr, nc) = (m.nrows(), m.ncols());
            let mut values = Vec::new();
            let mut indices = Vec::new();
            let mut indptr = vec![0i32; nr + 1];
            for r in 0..nr {
                for c in 0..nc {
                    let v = d[r * nc + c];
                    if v != 0.0 {
                        values.push(v);
                        indices.push(c as i32);
                    }
                }
                indptr[r + 1] = values.len() as i32;
            }
            (values, indices, indptr)
        }
    };

    let mut builder = FileBuilder::new();
    builder.set_attr(MATRIX_FILE_VERSION_ATTR, AttrValue::String(MATRIX_FILE_VERSION.into()));
    builder.set_attr(ARRAY_TYPE_ATTR, AttrValue::String(SPARSE_CSR.into()));
    builder.create_dataset(SPARSE_VALUES).with_f64_data(&values);
    builder.create_dataset(SPARSE_CSR_INDICES).with_i32_data(&indices);
    builder.create_dataset(SPARSE_CSR_INDPTR).with_i32_data(&indptr);
    add_labels_and_lengths(&mut builder, m, symmetric);
    builder.set_attr(SYMMETRIC_LABELS_ATTR, AttrValue::I64(symmetric as i64));
    builder.set_attr(DATA_TYPE_ATTR, AttrValue::String(m.data_type.clone()));
    builder.write(Path::new(path)).map_err(write_err)
}
