//! `DataMatrix`: the dense/sparse score-matrix model behind `seq_dist` and
//! `compare_contigs`, with HDF5 (`hdf5-pure`) and TSV persistence. Mirrors
//! `domainator.data_matrix.{DenseDataMatrix,SparseDataMatrix}` and the canonical
//! CSR layout (zeros eliminated, indices sorted, duplicate `(row,col)` max-merged).

pub mod cytoscape;
pub mod estimate;
pub mod hdf5_io;
pub mod hierarchy;
pub mod mst_knn;
pub mod ssn;
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

/// A stored CSR index -- an `indptr` or `indices` entry -- as a `usize`.
///
/// Never negative: every `Storage::Sparse` is either built by this module or loaded
/// through [`validate_csr`], which rejects a negative entry with a read error. The
/// check here only turns a broken invariant into a clear panic instead of a wild index.
pub(crate) fn csr_index(i: i32) -> usize {
    usize::try_from(i).expect("CSR indices are non-negative (checked by validate_csr)")
}

/// The stored-entry range of row `r` in a CSR matrix, into its `values`/`indices`.
pub(crate) fn csr_row(indptr: &[i32], r: usize) -> std::ops::Range<usize> {
    csr_index(indptr[r])..csr_index(indptr[r + 1])
}

/// Check that CSR arrays describe an `nrows x ncols` matrix: `indptr` has `nrows + 1`
/// entries, starts at 0, never decreases and ends at the entry count, and every
/// column index is in `0..ncols`. A file breaking any of these would otherwise fail
/// later, as an out-of-bounds panic inside whichever tool first walks the rows.
pub fn validate_csr(
    nrows: usize,
    ncols: usize,
    n_values: usize,
    indices: &[i32],
    indptr: &[i32],
) -> Result<(), String> {
    if indptr.len() != nrows + 1 {
        return Err(format!(
            "indptr has {} entries, expected nrows + 1 = {}",
            indptr.len(),
            nrows + 1
        ));
    }
    if indices.len() != n_values {
        return Err(format!(
            "{} column indices for {} stored values",
            indices.len(),
            n_values
        ));
    }
    if indptr[0] != 0 {
        return Err(format!("indptr starts at {}, not 0", indptr[0]));
    }
    if let Some(w) = indptr.windows(2).position(|w| w[1] < w[0]) {
        return Err(format!("indptr decreases at row {w}"));
    }
    let last = indptr[nrows];
    if usize::try_from(last).ok() != Some(n_values) {
        return Err(format!(
            "indptr ends at {last}, not the value count {n_values}"
        ));
    }
    if let Some(&c) = indices
        .iter()
        .find(|&&c| usize::try_from(c).map_or(true, |c| c >= ncols))
    {
        return Err(format!("column index {c} is outside 0..{ncols}"));
    }
    Ok(())
}

/// A labeled 2-D matrix of scores. Row/column labels, optional sequence lengths,
/// and a `data_type` string (`"score"`, `"bool"`, `"norm_score"`, ...).
pub struct DataMatrix {
    pub rows: Vec<String>,
    pub cols: Vec<String>,
    pub row_lengths: Option<Vec<i64>>,
    pub col_lengths: Option<Vec<i64>>,
    pub data_type: String,
    pub storage: Storage,
}

/// Read a [`DataMatrix`] from `path`, choosing the format by extension the same
/// way as `domainator.data_matrix.DataMatrix.from_file`: `.hdf5`/`.hdf`/`.h5` are
/// HDF5 (dense or sparse per `ARRAY_TYPE`), anything else is a dense TSV.
pub fn read_matrix(path: &str) -> std::io::Result<DataMatrix> {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "hdf5" | "hdf" | "h5" => hdf5_io::read(path),
        _ => text::read_dense_text(path),
    }
}

/// Refuse a distance matrix where 0 must mean "no edge" (upstream
/// `DataMatrix.require_similarity`).
pub fn require_similarity(data_type: &str, tool: &str) -> Result<(), String> {
    match data_type.strip_suffix("_dist") {
        Some(score_type) => Err(format!(
            "{tool} needs a similarity matrix (higher values mean more similar, 0 means no \
             edge), but the input is a '{data_type}' distance matrix. Use the '{score_type}' \
             matrix instead."
        )),
        None => Ok(()),
    }
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

    /// True for a `bool` matrix whose every value is 0 or 1: upstream holds those as
    /// int8 (`transform_matrix --mode bool`), so they are written as integers.
    pub fn is_integer_bool(&self) -> bool {
        let is_01 = |v: &f64| *v == 0.0 || *v == 1.0;
        self.data_type == "bool"
            && match &self.storage {
                Storage::Dense(d) => d.iter().all(is_01),
                Storage::Sparse { values, .. } => values.iter().all(is_01),
            }
    }

    /// Materialize the matrix as a row-major dense `Vec<f64>` (`nrows * ncols`).
    pub fn densify(&self) -> Vec<f64> {
        let (nr, nc) = (self.nrows(), self.ncols());
        match &self.storage {
            Storage::Dense(d) => d.clone(),
            Storage::Sparse {
                values,
                indices,
                indptr,
            } => {
                let mut out = vec![0.0; nr * nc];
                for r in 0..nr {
                    for k in csr_row(indptr, r) {
                        out[r * nc + csr_index(indices[k])] = values[k];
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
        return Storage::Sparse {
            values: Vec::new(),
            indices: Vec::new(),
            indptr: vec![0; nrows + 1],
        };
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
        let r = usize::try_from(r).expect("row indices are non-negative");
        indptr[r + 1] += 1;
    }
    for r in 0..nrows {
        indptr[r + 1] += indptr[r];
    }
    let indices: Vec<i32> = d_cols.iter().map(|&c| c as i32).collect();
    Storage::Sparse {
        values: d_vals,
        indices,
        indptr,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dm(storage: Storage) -> DataMatrix {
        DataMatrix {
            rows: vec!["a".into(), "b".into()],
            cols: vec!["a".into(), "b".into()],
            row_lengths: None,
            col_lengths: None,
            data_type: "score".into(),
            storage,
        }
    }

    #[test]
    fn build_sparse_max_dedups_by_max_and_densifies() {
        // (0,1) appears twice (3, 5) -> max 5; (1,0)=2.
        let s = build_sparse_max(vec![0, 0, 1], vec![1, 1, 0], vec![3.0, 5.0, 2.0], 2, 2);
        let m = dm(s);
        assert_eq!(m.densify(), vec![0.0, 5.0, 2.0, 0.0]);
    }

    #[test]
    fn build_sparse_max_empty() {
        let s = build_sparse_max(vec![], vec![], vec![], 2, 2);
        let m = dm(s);
        assert_eq!(m.densify(), vec![0.0; 4]);
    }

    #[test]
    fn validate_csr_accepts_canonical_and_rejects_malformed() {
        // 2x3: row 0 = {1}, row 1 = {0, 2}.
        let (idx, ptr) = (vec![1, 0, 2], vec![0, 1, 3]);
        assert!(validate_csr(2, 3, 3, &idx, &ptr).is_ok());
        assert!(validate_csr(0, 0, 0, &[], &[0]).is_ok());
        for (why, idx, ptr) in [
            ("indptr length", vec![1, 0, 2], vec![0, 3]),
            ("indptr start", vec![1, 0, 2], vec![1, 1, 3]),
            ("indptr decreases", vec![1, 0, 2], vec![0, 2, 1]),
            ("indptr end", vec![1, 0, 2], vec![0, 1, 2]),
            ("negative column", vec![1, -1, 2], vec![0, 1, 3]),
            ("column past ncols", vec![1, 0, 3], vec![0, 1, 3]),
        ] {
            assert!(validate_csr(2, 3, 3, &idx, &ptr).is_err(), "{why}");
        }
        assert!(
            validate_csr(2, 3, 2, &[1, 0, 2], &[0, 1, 3]).is_err(),
            "value count"
        );
    }

    #[test]
    fn symmetric_labels_detection() {
        let sym = dm(Storage::Dense(vec![0.0; 4]));
        assert!(sym.symmetric_labels());
        let mut asym = dm(Storage::Dense(vec![0.0; 4]));
        asym.cols = vec!["x".into(), "y".into()];
        assert!(!asym.symmetric_labels());
    }
}
