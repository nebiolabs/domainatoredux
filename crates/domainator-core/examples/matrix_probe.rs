//! E3 de-risk: write dense + sparse + TSV matrices and let Python read them back.
use domainator_core::matrix::{build_sparse_max, hdf5_io, text, DataMatrix, Storage};

fn main() {
    let rows = vec!["seqA".to_string(), "seqB".to_string(), "seqC".to_string()];
    // 3x3, symmetric-ish: A-A=10, A-B=3, B-B=20, C-C=5, B-C=1
    let r = vec![0, 0, 1, 1, 1, 2, 2];
    let c = vec![0, 1, 0, 1, 2, 1, 2];
    let v = vec![10.0, 3.0, 3.0, 20.0, 1.0, 1.0, 5.0];
    let storage = build_sparse_max(r, c, v, 3, 3);
    let m = DataMatrix {
        rows: rows.clone(),
        cols: rows.clone(),
        row_lengths: Some(vec![100, 200, 150]),
        col_lengths: Some(vec![100, 200, 150]),
        data_type: "score".to_string(),
        storage,
    };
    hdf5_io::write_sparse("/tmp/claude-1000/probe_sparse.hdf5", &m).unwrap();
    hdf5_io::write_dense("/tmp/claude-1000/probe_dense.hdf5", &m).unwrap();
    text::write_dense_text("/tmp/claude-1000/probe.tsv", &m).unwrap();

    // Asymmetric variant (different labels) to exercise COL_* datasets.
    let storage2 = match &m.storage {
        Storage::Sparse {
            values,
            indices,
            indptr,
        } => Storage::Sparse {
            values: values.clone(),
            indices: indices.clone(),
            indptr: indptr.clone(),
        },
        Storage::Dense(d) => Storage::Dense(d.clone()),
    };
    let m2 = DataMatrix {
        rows: rows.clone(),
        cols: vec!["hmm1".to_string(), "hmm2".to_string(), "hmm3".to_string()],
        row_lengths: Some(vec![100, 200, 150]),
        col_lengths: Some(vec![10, 20, 30]),
        data_type: "score".to_string(),
        storage: storage2,
    };
    hdf5_io::write_dense("/tmp/claude-1000/probe_dense_asym.hdf5", &m2).unwrap();
    println!("wrote probe matrices");
}
