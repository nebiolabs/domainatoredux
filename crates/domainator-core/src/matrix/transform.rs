//! Matrix value transforms (`--mode`), mirroring `domainator.transform_matrix.MODES`.
//!
//! `score`/`bool` are no-ops here (the raw / 1.0 values are set during matrix
//! construction). The normalizing modes densify first, matching the Python
//! implementation which converts to a dense array for the row/column-max math.

use super::{DataMatrix, Storage};

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
        Storage::Sparse { values, indices, indptr } => {
            let nrows = indptr.len().saturating_sub(1);
            let mut new_values: Vec<f64> = Vec::with_capacity(values.len());
            let mut new_indices: Vec<i32> = Vec::with_capacity(indices.len());
            let mut new_indptr: Vec<i32> = vec![0; nrows + 1];
            for r in 0..nrows {
                let (s, e) = (indptr[r] as usize, indptr[r + 1] as usize);
                for k in s..e {
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

#[cfg(test)]
mod tests {
    use super::*;

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
