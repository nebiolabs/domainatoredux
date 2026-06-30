//! `DataMatrix`: the dense/sparse score-matrix model behind `seq_dist` and
//! `compare_contigs`, with HDF5 (`hdf5-pure`) and TSV persistence. Mirrors
//! `domainator.data_matrix.{DenseDataMatrix,SparseDataMatrix}` and the canonical
//! CSR layout (zeros eliminated, indices sorted, duplicate `(row,col)` max-merged).

pub mod hdf5_io;
pub mod mst_knn;
pub mod text;
pub mod transform;

/// Backing storage for a [`DataMatrix`].
pub enum Storage {
    /// Row-major dense buffer of length `nrows * ncols`.
    Dense(Vec<f64>),
    /// Canonical CSR: `values`/`indices` per stored entry, `indptr` of length
    /// `nrows + 1`. Zeros eliminated, column indices sorted within each row.
    Sparse {
        values: Vec<f64>,
        indices: Vec<i32>,
        indptr: Vec<i32>,
    },
}

/// A labelled 2-D matrix of scores. Row/column labels, optional sequence lengths,
/// and a `data_type` string (`"score"`, `"bool"`, `"norm_score"`, ...).
pub struct DataMatrix {
    pub rows: Vec<String>,
    pub cols: Vec<String>,
    pub row_lengths: Option<Vec<i64>>,
    pub col_lengths: Option<Vec<i64>>,
    pub data_type: String,
    pub storage: Storage,
}

impl DataMatrix {
    pub fn nrows(&self) -> usize {
        self.rows.len()
    }
    pub fn ncols(&self) -> usize {
        self.cols.len()
    }

    /// True when row and column labels are identical (the symmetric-labels case
    /// that lets the writer omit the `COL_*` datasets).
    pub fn symmetric_labels(&self) -> bool {
        self.rows == self.cols && self.row_lengths == self.col_lengths
    }

    /// Materialize the matrix as a row-major dense `Vec<f64>` (`nrows * ncols`).
    pub fn densify(&self) -> Vec<f64> {
        let (nr, nc) = (self.nrows(), self.ncols());
        match &self.storage {
            Storage::Dense(d) => d.clone(),
            Storage::Sparse { values, indices, indptr } => {
                let mut out = vec![0.0; nr * nc];
                for r in 0..nr {
                    let (s, e) = (indptr[r] as usize, indptr[r + 1] as usize);
                    for k in s..e {
                        out[r * nc + indices[k] as usize] = values[k];
                    }
                }
                out
            }
        }
    }

    /// Return a dense copy of this matrix (same labels/lengths/data_type).
    pub fn to_dense(&self) -> DataMatrix {
        DataMatrix {
            rows: self.rows.clone(),
            cols: self.cols.clone(),
            row_lengths: self.row_lengths.clone(),
            col_lengths: self.col_lengths.clone(),
            data_type: self.data_type.clone(),
            storage: Storage::Dense(self.densify()),
        }
    }
}

/// Build a canonical max-merged CSR matrix from `(row, col, value)` triples,
/// mirroring `seq_dist._build_sparse_max_matrix`: sort by `(row, col)`, collapse
/// duplicate cells by `max`, drop nothing else (callers pre-filter zeros).
pub fn build_sparse_max(
    mut rows: Vec<i64>,
    mut cols: Vec<i64>,
    mut vals: Vec<f64>,
    nrows: usize,
    ncols: usize,
) -> Storage {
    let _ = ncols;
    if vals.is_empty() {
        return Storage::Sparse { values: Vec::new(), indices: Vec::new(), indptr: vec![0; nrows + 1] };
    }

    // Sort entries by (row, col) — np.lexsort((col, row)).
    let mut order: Vec<usize> = (0..vals.len()).collect();
    order.sort_by(|&a, &b| rows[a].cmp(&rows[b]).then(cols[a].cmp(&cols[b])));
    rows = order.iter().map(|&i| rows[i]).collect();
    cols = order.iter().map(|&i| cols[i]).collect();
    vals = order.iter().map(|&i| vals[i]).collect();

    // Collapse consecutive duplicate (row, col) by max.
    let mut d_rows: Vec<i64> = Vec::new();
    let mut d_cols: Vec<i64> = Vec::new();
    let mut d_vals: Vec<f64> = Vec::new();
    for i in 0..vals.len() {
        if let Some(last) = d_rows.len().checked_sub(1) {
            if d_rows[last] == rows[i] && d_cols[last] == cols[i] {
                if vals[i] > d_vals[last] {
                    d_vals[last] = vals[i];
                }
                continue;
            }
        }
        d_rows.push(rows[i]);
        d_cols.push(cols[i]);
        d_vals.push(vals[i]);
    }

    // Emit CSR (entries already sorted by row then col).
    let mut indptr = vec![0i32; nrows + 1];
    for &r in &d_rows {
        indptr[r as usize + 1] += 1;
    }
    for r in 0..nrows {
        indptr[r + 1] += indptr[r];
    }
    let indices: Vec<i32> = d_cols.iter().map(|&c| c as i32).collect();
    Storage::Sparse { values: d_vals, indices, indptr }
}
