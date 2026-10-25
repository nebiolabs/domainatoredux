//! HDF5 persistence for [`DataMatrix`] via `hdf5-pure`, in upstream's matrix file
//! format version 2.0 (`domainator.data_matrix.DataMatrix.write_hdf`), so the Python
//! suite (`DataMatrix.from_file`) reads our output unchanged.
//!
//! Attributes: `MATRIX_FILE_VERSION="2.0"`, `ARRAY_TYPE` (`SPARSE_CSR`, or
//! `SPARSE_CSR_UPPER_TRIANGLE` for an exactly symmetric matrix; legacy `DENSE` is
//! read but never written), `SYMMETRIC_LABELS` (written as an int 1/0, which h5py
//! reads truthily), and `DATA_TYPE`. Labels are UTF-8 variable-length strings;
//! `COL_*` datasets are omitted when labels are symmetric. Version 1.x files, and
//! those carrying the legacy `DOMAINATOR_MATRIX_FILE_VERSION` attribute, read too.

use std::io;
use std::path::Path;

use hdf5_pure::{AttrValue, File, FileBuilder};

use super::{DataMatrix, Storage};

const MATRIX_FILE_VERSION: &str = "2.0";
/// Major versions this reader accepts.
const SUPPORTED_MATRIX_FILE_MAJOR_VERSIONS: [&str; 2] = ["1", "2"];
const ARRAY_TYPE_ATTR: &str = "ARRAY_TYPE";
const SYMMETRIC_LABELS_ATTR: &str = "SYMMETRIC_LABELS";
const MATRIX_FILE_VERSION_ATTR: &str = "MATRIX_FILE_VERSION";
const LEGACY_MATRIX_FILE_VERSION_ATTR: &str = "DOMAINATOR_MATRIX_FILE_VERSION";
const DATA_TYPE_ATTR: &str = "DATA_TYPE";

/// Legacy dense storage: read only.
const DENSE: &str = "DENSE";
const SPARSE_CSR: &str = "SPARSE_CSR";
/// CSR holding only the upper triangle and diagonal of an exactly symmetric matrix.
/// A distinct `ARRAY_TYPE` makes readers that predate it fail instead of reading half
/// the matrix.
const SPARSE_CSR_UPPER_TRIANGLE: &str = "SPARSE_CSR_UPPER_TRIANGLE";

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
/// suite. Chooses the layout from the `ARRAY_TYPE` attribute, mirrors an upper
/// triangle into the full matrix, and reconstructs `COL_*` from `ROW_*` when the
/// labels are symmetric (those datasets are omitted on write). A file from an
/// unknown major format version is refused.
pub fn read(path: &str) -> io::Result<DataMatrix> {
    let reader = File::open(Path::new(path)).map_err(read_err)?;
    let attrs = reader.root().attrs().map_err(read_err)?;

    let version = attr_string(&attrs, MATRIX_FILE_VERSION_ATTR)
        .or_else(|| attr_string(&attrs, LEGACY_MATRIX_FILE_VERSION_ATTR))
        .ok_or_else(|| {
            read_err(format!(
                "Required attribute '{MATRIX_FILE_VERSION_ATTR}' not found in HDF5 file"
            ))
        })?;
    let major = version.split('.').next().unwrap_or_default();
    if !SUPPORTED_MATRIX_FILE_MAJOR_VERSIONS.contains(&major) {
        let supported: Vec<String> = SUPPORTED_MATRIX_FILE_MAJOR_VERSIONS
            .iter()
            .map(|m| format!("{m}.x"))
            .collect();
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "Unsupported matrix file version '{version}'. This version of domainatoredux \
                 reads version {} matrix files.",
                supported.join(", ")
            ),
        ));
    }

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
        SPARSE_CSR | SPARSE_CSR_UPPER_TRIANGLE => {
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
            super::validate_csr(nr, nc, values.len(), &indices, &indptr)
                .map_err(|e| read_err(format!("malformed sparse matrix: {e}")))?;
            if array_type == SPARSE_CSR_UPPER_TRIANGLE {
                if nr != nc {
                    return Err(read_err(format!(
                        "a {SPARSE_CSR_UPPER_TRIANGLE} matrix must be square, not {nr}x{nc}"
                    )));
                }
                mirror_upper_triangle(nr, &values, &indices, &indptr)
            } else {
                Storage::Sparse {
                    values,
                    indices,
                    indptr,
                }
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

/// The symmetric matrix whose upper triangle (diagonal included) is the stored CSR
/// (upstream `mirror_upper_triangle`), canonical: duplicates summed, zeros dropped.
fn mirror_upper_triangle(n: usize, values: &[f64], indices: &[i32], indptr: &[i32]) -> Storage {
    let mut rows: Vec<Vec<(i32, f64)>> = vec![Vec::new(); n];
    for r in 0..n {
        for k in super::csr_row(indptr, r) {
            let c = super::csr_index(indices[k]);
            rows[r].push((indices[k], values[k]));
            if c > r {
                rows[c].push((i32::try_from(r).expect("fits the i32 indices"), values[k]));
            }
        }
    }
    let mut out_values = Vec::new();
    let mut out_indices = Vec::new();
    let mut out_indptr = vec![0i32; n + 1];
    for (r, row) in rows.iter_mut().enumerate() {
        row.sort_by_key(|&(c, _)| c);
        let mut k = 0;
        while k < row.len() {
            let (c, mut v) = row[k];
            k += 1;
            while k < row.len() && row[k].0 == c {
                v += row[k].1;
                k += 1;
            }
            if v != 0.0 {
                out_values.push(v);
                out_indices.push(c);
            }
        }
        out_indptr[r + 1] = i32::try_from(out_values.len()).expect("fits the i32 indices");
    }
    Storage::Sparse {
        values: out_values,
        indices: out_indices,
        indptr: out_indptr,
    }
}

/// `m` as canonical CSR arrays (upstream `_to_canonical_csr`): explicit zeros dropped,
/// column indices sorted within each row. A NaN is a value, so it is kept.
fn canonical_csr(m: &DataMatrix) -> (Vec<f64>, Vec<i32>, Vec<i32>) {
    let (nr, nc) = (m.nrows(), m.ncols());
    let mut values = Vec::new();
    let mut indices = Vec::new();
    let mut indptr = vec![0i32; nr + 1];
    let as_index = |i: usize| i32::try_from(i).expect("fits the i32 indices");
    match &m.storage {
        Storage::Dense(d) => {
            for r in 0..nr {
                for c in 0..nc {
                    let v = d[r * nc + c];
                    if v != 0.0 {
                        values.push(v);
                        indices.push(as_index(c));
                    }
                }
                indptr[r + 1] = as_index(values.len());
            }
        }
        Storage::Sparse {
            values: sv,
            indices: si,
            indptr: sp,
        } => {
            for r in 0..nr {
                let mut row: Vec<(i32, f64)> = super::csr_row(sp, r)
                    .map(|k| (si[k], sv[k]))
                    .filter(|&(_, v)| v != 0.0)
                    .collect();
                row.sort_by_key(|&(c, _)| c);
                for (c, v) in row {
                    indices.push(c);
                    values.push(v);
                }
                indptr[r + 1] = as_index(values.len());
            }
        }
    }
    (values, indices, indptr)
}

/// True if square canonical CSR equals its transpose value for value (upstream
/// `_is_exactly_symmetric`). Exact equality is what makes triangle storage lossless.
fn is_exactly_symmetric(n: usize, values: &[f64], indices: &[i32], indptr: &[i32]) -> bool {
    // The transpose by counting sort, which leaves each of its rows sorted.
    let mut t_indptr = vec![0usize; n + 1];
    for &c in indices {
        t_indptr[super::csr_index(c) + 1] += 1;
    }
    for r in 0..n {
        t_indptr[r + 1] += t_indptr[r];
    }
    let mut next = t_indptr.clone();
    let mut t_indices = vec![0i32; indices.len()];
    let mut t_values = vec![0.0; values.len()];
    for r in 0..n {
        for k in super::csr_row(indptr, r) {
            let c = super::csr_index(indices[k]);
            t_indices[next[c]] = i32::try_from(r).expect("fits the i32 indices");
            t_values[next[c]] = values[k];
            next[c] += 1;
        }
    }
    indptr
        .iter()
        .zip(&t_indptr)
        .all(|(&a, &b)| super::csr_index(a) == b)
        && indices == t_indices.as_slice()
        && values.iter().zip(&t_values).all(|(a, b)| a == b)
}

/// Write `m` in matrix file format 2.0 (upstream `DataMatrix.write_hdf`): canonical
/// CSR, stored as its upper triangle (diagonal included) when the labels and lengths
/// are symmetric and every value equals its transpose exactly, in full otherwise.
pub fn write(path: &str, m: &DataMatrix) -> io::Result<()> {
    let symmetric = m.symmetric_labels();
    let (values, indices, indptr) = canonical_csr(m);
    let triangle = symmetric && is_exactly_symmetric(m.nrows(), &values, &indices, &indptr);
    let (array_type, values, indices, indptr) = if triangle {
        let mut upper_values = Vec::new();
        let mut upper_indices = Vec::new();
        let mut upper_indptr = vec![0i32; m.nrows() + 1];
        for r in 0..m.nrows() {
            for k in super::csr_row(&indptr, r) {
                if super::csr_index(indices[k]) >= r {
                    upper_values.push(values[k]);
                    upper_indices.push(indices[k]);
                }
            }
            upper_indptr[r + 1] = i32::try_from(upper_values.len()).expect("fits the i32 indices");
        }
        (
            SPARSE_CSR_UPPER_TRIANGLE,
            upper_values,
            upper_indices,
            upper_indptr,
        )
    } else {
        (SPARSE_CSR, values, indices, indptr)
    };

    let mut builder = FileBuilder::new();
    builder.set_attr(
        MATRIX_FILE_VERSION_ATTR,
        AttrValue::VarLenUtf8(MATRIX_FILE_VERSION.into()),
    );
    builder.set_attr(ARRAY_TYPE_ATTR, AttrValue::VarLenUtf8(array_type.into()));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> String {
        let dir = std::env::temp_dir().join(format!("domainator-hdf5-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name).to_string_lossy().into_owned()
    }

    fn matrix(cols: &[&str], dense: Vec<f64>) -> DataMatrix {
        DataMatrix {
            rows: vec!["a".into(), "b".into(), "c".into()],
            cols: cols.iter().map(|c| c.to_string()).collect(),
            row_lengths: None,
            col_lengths: None,
            data_type: "score".into(),
            storage: Storage::Dense(dense),
        }
    }

    fn array_type(path: &str) -> String {
        let file = File::open(Path::new(path)).unwrap();
        attr_string(&file.root().attrs().unwrap(), ARRAY_TYPE_ATTR).unwrap()
    }

    fn stored_values(path: &str) -> usize {
        let file = File::open(Path::new(path)).unwrap();
        file.dataset(SPARSE_VALUES)
            .unwrap()
            .read_f64()
            .unwrap()
            .len()
    }

    /// An exactly symmetric matrix is stored as its upper triangle and read back whole.
    #[test]
    fn a_symmetric_matrix_round_trips_through_its_upper_triangle() {
        let dense = vec![5.0, 1.0, 0.0, 1.0, 7.0, 2.5, 0.0, 2.5, 0.0];
        let path = scratch("sym.hdf5");
        write(&path, &matrix(&["a", "b", "c"], dense.clone())).unwrap();
        assert_eq!(array_type(&path), SPARSE_CSR_UPPER_TRIANGLE);
        assert_eq!(stored_values(&path), 4);
        let back = read(&path).unwrap();
        assert_eq!(back.densify(), dense);
        assert!(matches!(back.storage, Storage::Sparse { .. }));
    }

    /// Off by a hair, or with different column labels, the whole matrix is stored.
    #[test]
    fn other_matrices_are_stored_in_full() {
        let near = vec![5.0, 1.0, 0.0, 1.000_001, 7.0, 2.5, 0.0, 2.5, 0.0];
        let path = scratch("near.hdf5");
        write(&path, &matrix(&["a", "b", "c"], near.clone())).unwrap();
        assert_eq!(array_type(&path), SPARSE_CSR);
        assert_eq!(read(&path).unwrap().densify(), near);

        let sym = vec![5.0, 1.0, 0.0, 1.0, 7.0, 2.5, 0.0, 2.5, 0.0];
        let path = scratch("labels.hdf5");
        write(&path, &matrix(&["x", "y", "z"], sym.clone())).unwrap();
        assert_eq!(array_type(&path), SPARSE_CSR);
        let back = read(&path).unwrap();
        assert_eq!(back.cols, ["x", "y", "z"]);
        assert_eq!(back.densify(), sym);
    }

    /// Explicit zeros in sparse storage are not written.
    #[test]
    fn explicit_zeros_are_dropped() {
        let mut m = matrix(&["x", "y", "z"], vec![]);
        m.storage = Storage::Sparse {
            values: vec![0.0, 3.0, 0.0],
            indices: vec![0, 2, 1],
            indptr: vec![0, 2, 2, 3],
        };
        let path = scratch("zeros.hdf5");
        write(&path, &m).unwrap();
        assert_eq!(stored_values(&path), 1);
        assert_eq!(read(&path).unwrap().densify()[2], 3.0);
    }

    #[test]
    fn an_unknown_major_version_is_refused() {
        let mut builder = FileBuilder::new();
        builder.set_attr(
            MATRIX_FILE_VERSION_ATTR,
            AttrValue::VarLenUtf8("3.0".into()),
        );
        builder.set_attr(ARRAY_TYPE_ATTR, AttrValue::VarLenUtf8(SPARSE_CSR.into()));
        let path = scratch("v3.hdf5");
        builder.write(Path::new(&path)).unwrap();
        let Err(err) = read(&path) else {
            panic!("a version 3.0 file was read");
        };
        let err = err.to_string();
        assert!(
            err.contains("Unsupported matrix file version '3.0'"),
            "{err}"
        );
    }
}
