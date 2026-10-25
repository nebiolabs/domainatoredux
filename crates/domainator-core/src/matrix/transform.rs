//! Matrix value transforms (`--mode`), mirroring `domainator.transform_matrix.MODES`.
//!
//! `score`/`bool` are no-ops here (the raw / 1.0 values are set during matrix
//! construction). The normalizing modes densify first, matching the Python
//! implementation which converts to a dense array for the row/column-max math.

use super::{csr_row, DataMatrix, Storage};

/// `log10(2)`, the EFI score's per-bit factor.
pub const LOG10_2: f64 = std::f64::consts::LOG10_2;

/// Apply `--mode` to `m` in place and set `data_type = mode`. Used by `seq_dist`
/// (which already materializes `bool`/`score` values during construction, so those
/// are effectively no-ops there).
pub fn apply_mode(m: &mut DataMatrix, mode: &str) {
    transform_values(m, mode);
    m.data_type = mode.to_string();
}

/// Apply the value transform for `mode` to `m` in place **without** touching
/// `data_type`. `score` is a pass-through; `bool` maps every value to `1.0`/`0.0`
/// (`v > 0`); the rest densify and normalize as in `transform_matrix.MODES`.
pub fn transform_values(m: &mut DataMatrix, mode: &str) {
    match mode {
        "score" => {}
        "bool" => {
            let mut d = m.densify();
            for v in d.iter_mut() {
                *v = if *v > 0.0 { 1.0 } else { 0.0 };
            }
            m.storage = Storage::Dense(d);
        }
        "norm_score" | "row_norm_score" | "score_dist" | "efi_score" | "efi_score_dist" => {
            dense_transform(m, mode)
        }
        _ => {}
    }
}

/// Zero out every value `<= lb` (mirrors `transform_matrix.apply_lower_bound`).
pub fn apply_lower_bound(m: &mut DataMatrix, lb: f64) {
    match &mut m.storage {
        Storage::Dense(d) => {
            for v in d.iter_mut() {
                if *v <= lb {
                    *v = 0.0;
                }
            }
        }
        Storage::Sparse {
            values,
            indices,
            indptr,
        } => {
            let nrows = indptr.len().saturating_sub(1);
            let mut new_values: Vec<f64> = Vec::with_capacity(values.len());
            let mut new_indices: Vec<i32> = Vec::with_capacity(indices.len());
            let mut new_indptr: Vec<i32> = vec![0; nrows + 1];
            for r in 0..nrows {
                for k in csr_row(indptr, r) {
                    if values[k] > lb {
                        new_values.push(values[k]);
                        new_indices.push(indices[k]);
                    }
                }
                new_indptr[r + 1] = new_values.len() as i32;
            }
            *values = new_values;
            *indices = new_indices;
            *indptr = new_indptr;
        }
    }
}

/// Column-wise max over a row-major dense buffer.
fn col_maxes(d: &[f64], nr: usize, nc: usize) -> Vec<f64> {
    let mut out = vec![f64::NEG_INFINITY; nc];
    for r in 0..nr {
        for c in 0..nc {
            let v = d[r * nc + c];
            if v > out[c] {
                out[c] = v;
            }
        }
    }
    out
}

/// Row-wise max over a row-major dense buffer.
fn row_maxes(d: &[f64], nr: usize, nc: usize) -> Vec<f64> {
    let mut out = vec![f64::NEG_INFINITY; nr];
    for r in 0..nr {
        for c in 0..nc {
            let v = d[r * nc + c];
            if v > out[r] {
                out[r] = v;
            }
        }
    }
    out
}

fn dense_transform(m: &mut DataMatrix, mode: &str) {
    let (nr, nc) = (m.nrows(), m.ncols());
    let mut d = m.densify();

    match mode {
        // score / min(row_max, col_max)
        "norm_score" => {
            let rmax = row_maxes(&d, nr, nc);
            let cmax = col_maxes(&d, nr, nc);
            for r in 0..nr {
                for c in 0..nc {
                    d[r * nc + c] /= rmax[r].min(cmax[c]);
                }
            }
        }
        // score / row_max
        "row_norm_score" => {
            let rmax = row_maxes(&d, nr, nc);
            for r in 0..nr {
                for c in 0..nc {
                    d[r * nc + c] /= rmax[r];
                }
            }
        }
        // 1 - (score / min(row_max, col_max))
        "score_dist" => {
            let rmax = row_maxes(&d, nr, nc);
            let cmax = col_maxes(&d, nr, nc);
            for r in 0..nr {
                for c in 0..nc {
                    d[r * nc + c] = 1.0 - d[r * nc + c] / rmax[r].min(cmax[c]);
                }
            }
        }
        // -log10[2^(-score) * (row_len * col_len)] = score*log10(2) - log10(row_len) - log10(col_len)
        // values <= 0 set to 0
        "efi_score" => {
            let (rl, cl) = efi_logs(m, nr, nc);
            for r in 0..nr {
                for c in 0..nc {
                    let v = d[r * nc + c] * LOG10_2 - rl[r] - cl[c];
                    d[r * nc + c] = if v > 0.0 { v } else { 0.0 };
                }
            }
        }
        // 1 - (efi_score / min(row_max, col_max))
        "efi_score_dist" => {
            let (rl, cl) = efi_logs(m, nr, nc);
            for r in 0..nr {
                for c in 0..nc {
                    let v = d[r * nc + c] * LOG10_2 - rl[r] - cl[c];
                    d[r * nc + c] = if v > 0.0 { v } else { 0.0 };
                }
            }
            let rmax = row_maxes(&d, nr, nc);
            let cmax = col_maxes(&d, nr, nc);
            for r in 0..nr {
                for c in 0..nc {
                    d[r * nc + c] = 1.0 - d[r * nc + c] / rmax[r].min(cmax[c]);
                }
            }
        }
        _ => {}
    }

    m.storage = Storage::Dense(d);
}

/// `log10` of row and column lengths for the EFI modes (panics-free: callers
/// validate lengths exist, matching the Python `ValueError`).
fn efi_logs(m: &DataMatrix, nr: usize, nc: usize) -> (Vec<f64>, Vec<f64>) {
    let rl = m
        .row_lengths
        .as_ref()
        .map(|v| v.iter().map(|&x| (x as f64).log10()).collect())
        .unwrap_or_else(|| vec![0.0; nr]);
    let cl = m
        .col_lengths
        .as_ref()
        .map(|v| v.iter().map(|&x| (x as f64).log10()).collect())
        .unwrap_or_else(|| vec![0.0; nc]);
    (rl, cl)
}

/// The `--symmetrize` choices (upstream `SYMMETRIZE_CHOICES`).
pub const SYMMETRIZE_CHOICES: [&str; 4] = ["none", "min", "max", "mean"];

/// Reconcile the two directions of every pair of a square matrix (upstream
/// `symmetrize_matrix`): both cells of a pair get the `max`, `min` or `mean` of the
/// two; `none` leaves `m` alone. Apply it to raw values, before any `--mode`
/// transform. A pair stored in one direction only has a zero in the other, so `min`
/// deletes it and `mean` halves it. A sparse matrix stays sparse, zeros dropped.
pub fn symmetrize(m: &mut DataMatrix, how: &str) -> Result<(), String> {
    let combine: fn(f64, f64) -> f64 = match how {
        "none" => return Ok(()),
        "max" => f64::max,
        "min" => f64::min,
        "mean" => |a, b| (a + b) / 2.0,
        other => {
            return Err(format!(
                "Unknown symmetrize method: {other:?}, expected one of {SYMMETRIZE_CHOICES:?}"
            ))
        }
    };
    let n = m.nrows();
    if n != m.ncols() {
        return Err("Only a square matrix can be symmetrized.".into());
    }
    match &mut m.storage {
        Storage::Dense(d) => {
            for r in 0..n {
                for c in r..n {
                    let v = combine(d[r * n + c], d[c * n + r]);
                    d[r * n + c] = v;
                    d[c * n + r] = v;
                }
            }
        }
        Storage::Sparse {
            values,
            indices,
            indptr,
        } => {
            let mut cells = std::collections::BTreeMap::new();
            for r in 0..n {
                for k in csr_row(indptr, r) {
                    cells.insert((r, super::csr_index(indices[k])), values[k]);
                }
            }
            let mut out = std::collections::BTreeMap::new();
            for (&(r, c), &v) in &cells {
                let t = cells.get(&(c, r)).copied().unwrap_or(0.0);
                out.insert((r, c), combine(v, t));
                out.insert((c, r), combine(t, v));
            }
            let (mut vals, mut idx, mut ptr) = (Vec::new(), Vec::new(), vec![0i32; n + 1]);
            for ((r, c), v) in out {
                if v != 0.0 {
                    vals.push(v);
                    idx.push(i32::try_from(c).map_err(|e| e.to_string())?);
                    ptr[r + 1] += 1;
                }
            }
            for r in 0..n {
                ptr[r + 1] += ptr[r];
            }
            (*values, *indices, *indptr) = (vals, idx, ptr);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sparse(n: usize, cells: &[(usize, usize, f64)]) -> DataMatrix {
        let mut m = mat(vec![0.0; n * n]);
        m.rows = (0..n).map(|i| i.to_string()).collect();
        m.cols = m.rows.clone();
        let mut d = vec![0.0; n * n];
        for &(r, c, v) in cells {
            d[r * n + c] = v;
        }
        let (mut values, mut indices, mut indptr) = (Vec::new(), Vec::new(), vec![0i32]);
        for r in 0..n {
            for c in 0..n {
                if d[r * n + c] != 0.0 {
                    values.push(d[r * n + c]);
                    indices.push(c as i32);
                }
            }
            indptr.push(values.len() as i32);
        }
        m.storage = Storage::Sparse {
            values,
            indices,
            indptr,
        };
        m
    }

    /// A pair found one way only is kept by max, deleted by min and halved by mean
    /// (upstream's documented `--symmetrize` behavior), sparse and dense alike.
    #[test]
    fn symmetrize_reconciles_both_directions() {
        let cells = [(0, 1, 4.0), (1, 0, 2.0), (0, 2, 6.0), (2, 2, 5.0)];
        for (how, want) in [
            ("max", [0.0, 4.0, 6.0, 4.0, 0.0, 0.0, 6.0, 0.0, 5.0]),
            ("min", [0.0, 2.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, 5.0]),
            ("mean", [0.0, 3.0, 3.0, 3.0, 0.0, 0.0, 3.0, 0.0, 5.0]),
        ] {
            let mut m = sparse(3, &cells);
            symmetrize(&mut m, how).unwrap();
            assert_eq!(m.densify(), want, "{how} (sparse)");
            if let Storage::Sparse { values, .. } = &m.storage {
                assert!(values.iter().all(|&v| v != 0.0), "{how}: zeros stored");
            }
            let mut d = sparse(3, &cells).to_dense();
            symmetrize(&mut d, how).unwrap();
            assert_eq!(d.densify(), want, "{how} (dense)");
        }
        let mut m = sparse(3, &cells);
        symmetrize(&mut m, "none").unwrap();
        assert_eq!(m.densify()[1], 4.0);
        assert_eq!(m.densify()[3], 2.0);
    }

    fn mat(d: Vec<f64>) -> DataMatrix {
        DataMatrix {
            rows: vec!["a".into(), "b".into()],
            cols: vec!["a".into(), "b".into()],
            row_lengths: None,
            col_lengths: None,
            data_type: "score".into(),
            storage: Storage::Dense(d),
        }
    }

    fn approx(a: &[f64], b: &[f64]) {
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b) {
            assert!((x - y).abs() < 1e-9, "{a:?} != {b:?}");
        }
    }

    #[test]
    fn score_and_bool_are_noops() {
        let mut m = mat(vec![2.0, 4.0, 1.0, 8.0]);
        apply_mode(&mut m, "score");
        approx(&m.densify(), &[2.0, 4.0, 1.0, 8.0]);
        assert_eq!(m.data_type, "score");
    }

    #[test]
    fn norm_score_divides_by_min_row_col_max() {
        // d=[[2,4],[1,8]]; row_max=[4,8], col_max=[2,8].
        let mut m = mat(vec![2.0, 4.0, 1.0, 8.0]);
        apply_mode(&mut m, "norm_score");
        approx(&m.densify(), &[1.0, 1.0, 0.5, 1.0]);
        assert_eq!(m.data_type, "norm_score");
    }

    #[test]
    fn row_norm_score_divides_by_row_max() {
        let mut m = mat(vec![2.0, 4.0, 1.0, 8.0]);
        apply_mode(&mut m, "row_norm_score");
        approx(&m.densify(), &[0.5, 1.0, 0.125, 1.0]);
    }

    #[test]
    fn score_dist_is_one_minus_norm() {
        let mut m = mat(vec![2.0, 4.0, 1.0, 8.0]);
        apply_mode(&mut m, "score_dist");
        approx(&m.densify(), &[0.0, 0.0, 0.5, 0.0]);
    }
}
