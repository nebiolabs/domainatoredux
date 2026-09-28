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

use hdf5_pure::{AttrValue, File, FileBuilder};

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
    io::Error::other(format!("hdf5 write: {e:?}"))
}

fn read_err<E: std::fmt::Debug>(e: E) -> io::Error {
    io::Error::other(format!("hdf5 read: {e:?}"))
}

fn attr_string(attrs: &std::collections::HashMap<String, AttrValue>, key: &str) -> Option<String> {
    match attrs.get(key)? {
        AttrValue::VarLenUtf8(s) | AttrValue::String(s) | AttrValue::AsciiString(s) => {
            Some(s.clone())
        }
        _ => None,
    }
}

/// Read a [`DataMatrix`] from an HDF5 file written by this module or the Python
/// suite (`DataMatrix.{write_dense,write_sparse}`). Chooses dense vs sparse from
/// the `ARRAY_TYPE` attribute and reconstructs `COL_*` from `ROW_*` when the
/// matrix is symmetric (those datasets are omitted on write).
pub fn read(path: &str) -> io::Result<DataMatrix> {
    let reader = File::open(Path::new(path)).map_err(read_err)?;
    let attrs = reader.root().attrs().map_err(read_err)?;

    let array_type = attr_string(&attrs, ARRAY_TYPE_ATTR)
        .ok_or_else(|| read_err(format!("missing required attribute '{ARRAY_TYPE_ATTR}'")))?;
    let data_type = attr_string(&attrs, DATA_TYPE_ATTR).unwrap_or_default();

    let rows = reader
        .dataset(ROW_LABELS)
        .map_err(read_err)?
        .read_string()
        .map_err(read_err)?;
    let row_lengths = match reader.dataset(ROW_LENGTHS) {
        Ok(ds) => Some(ds.read_i64().map_err(read_err)?),
        Err(_) => None,
    };
    // Symmetry is inferred from the *presence* of COL_LABELS (both the Python and
    // Rust writers omit COL_* exactly when the matrix is symmetric). The
    // `SYMMETRIC_LABELS` attribute itself is unreliable here: h5py stores it as a
    // numpy bool, which the pure-Rust HDF5 reader does not surface.
    let (cols, col_lengths) = match reader.dataset(COL_LABELS) {
        Ok(ds) => {
            let cols = ds.read_string().map_err(read_err)?;
            let col_lengths = match reader.dataset(COL_LENGTHS) {
                Ok(ds) => Some(ds.read_i64().map_err(read_err)?),
                Err(_) => None,
            };
            (cols, col_lengths)
        }
        Err(_) => (rows.clone(), row_lengths.clone()),
    };

    let (nr, nc) = (rows.len(), cols.len());
    let storage = match array_type.as_str() {
        DENSE => {
            let flat = reader
                .dataset(DENSE_DATA)
                .map_err(read_err)?
                .read_f64()
                .map_err(read_err)?;
            if flat.len() != nr * nc {
                return Err(read_err(format!(
                    "DENSE_DATA length {} != nrows*ncols {}",
                    flat.len(),
                    nr * nc
                )));
            }
            Storage::Dense(flat)
        }
        SPARSE_CSR => {
            let values = reader
                .dataset(SPARSE_VALUES)
                .map_err(read_err)?
                .read_f64()
                .map_err(read_err)?;
            // Index width is not fixed by the format: upstream narrows to int32 on write
            // when the matrix can be addressed by it, but a file derived from a large
            // int64-indexed matrix keeps int64. Readers must accept either
            // (docs/file_formats.md), so read both as i64 and narrow here.
            let indices = read_index_array(&reader, SPARSE_CSR_INDICES)?;
            let indptr = read_index_array(&reader, SPARSE_CSR_INDPTR)?;
            Storage::Sparse {
                values,
                indices,
                indptr,
            }
        }
        other => return Err(read_err(format!("unknown ARRAY_TYPE '{other}'"))),
    };

    Ok(DataMatrix {
        rows,
        cols,
        row_lengths,
        col_lengths,
        data_type,
        storage,
    })
}

/// Read a CSR index dataset written at either int32 or int64 width.
///
/// `read_i64` widens an int32 dataset for free, so one call covers both. The values are
/// then narrowed to the `i32` this port's `Storage::Sparse` uses — a matrix whose
/// indices genuinely exceed i32 is larger than anything this representation can hold,
/// so that is reported rather than silently truncated.
fn read_index_array(reader: &File, name: &str) -> io::Result<Vec<i32>> {
    let wide = reader
        .dataset(name)
        .map_err(read_err)?
        .read_i64()
        .map_err(read_err)?;
    wide.into_iter()
        .map(|v| {
            i32::try_from(v).map_err(|_| {
                read_err(format!(
                    "{name} contains index {v}, which exceeds this build's 32-bit \
                     sparse index limit"
                ))
            })
        })
        .collect()
}

fn add_labels_and_lengths(builder: &mut FileBuilder, m: &DataMatrix, symmetric: bool) {
    let row_refs: Vec<&str> = m.rows.iter().map(String::as_str).collect();
    builder
        .create_dataset(ROW_LABELS)
        .with_vlen_strings(&row_refs);
    if let Some(rl) = &m.row_lengths {
        builder.create_dataset(ROW_LENGTHS).with_i64_data(rl);
    }
    if !symmetric {
        let col_refs: Vec<&str> = m.cols.iter().map(String::as_str).collect();
        builder
            .create_dataset(COL_LABELS)
            .with_vlen_strings(&col_refs);
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
    builder.set_attr(
        MATRIX_FILE_VERSION_ATTR,
        AttrValue::VarLenUtf8(MATRIX_FILE_VERSION.into()),
    );
    builder.set_attr(ARRAY_TYPE_ATTR, AttrValue::VarLenUtf8(DENSE.into()));
    builder
        .create_dataset(DENSE_DATA)
        .with_f64_data(&flat)
        .with_shape(&[nr as u64, nc as u64]);
    add_labels_and_lengths(&mut builder, m, symmetric);
    builder.set_attr(SYMMETRIC_LABELS_ATTR, AttrValue::I64(symmetric as i64));
    builder.set_attr(DATA_TYPE_ATTR, AttrValue::VarLenUtf8(m.data_type.clone()));
    builder.write(Path::new(path)).map_err(write_err)
}

/// Write `m` as a CSR sparse HDF5 matrix (`ARRAY_TYPE=SPARSE_CSR`). The matrix is
/// stored canonically (zeros eliminated, indices sorted) by construction.
pub fn write_sparse(path: &str, m: &DataMatrix) -> io::Result<()> {
    let symmetric = m.symmetric_labels();
    // Obtain canonical CSR arrays (densify->rebuild if currently dense).
    let (values, indices, indptr) = match &m.storage {
        Storage::Sparse {
            values,
            indices,
            indptr,
        } => (values.clone(), indices.clone(), indptr.clone()),
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
    builder.set_attr(
        MATRIX_FILE_VERSION_ATTR,
        AttrValue::VarLenUtf8(MATRIX_FILE_VERSION.into()),
    );
    builder.set_attr(ARRAY_TYPE_ATTR, AttrValue::VarLenUtf8(SPARSE_CSR.into()));
    builder.create_dataset(SPARSE_VALUES).with_f64_data(&values);
    builder
        .create_dataset(SPARSE_CSR_INDICES)
        .with_i32_data(&indices);
    builder
        .create_dataset(SPARSE_CSR_INDPTR)
        .with_i32_data(&indptr);
    add_labels_and_lengths(&mut builder, m, symmetric);
    builder.set_attr(SYMMETRIC_LABELS_ATTR, AttrValue::I64(symmetric as i64));
    builder.set_attr(DATA_TYPE_ATTR, AttrValue::VarLenUtf8(m.data_type.clone()));
    builder.write(Path::new(path)).map_err(write_err)
}
