//! `build_projection`: project a matrix's rows into 2D or 3D. Mirrors
//! `domainator.build_projection`, which treats each row as a point whose features are
//! its columns.
//!
//! This port computes `pca` and `coords`. Upstream projects a mostly-filled matrix with
//! scikit-learn's `PCA` (centered) and a mostly-empty one with `TruncatedSVD`
//! (uncentered); both are an SVD whose signs `svd_flip(u_based_decision=False)` fixes,
//! which is what [`pca`] and [`truncated_svd`] compute, exactly rather than with
//! scikit-learn's randomized solver for large inputs. `tsne` and `umap` are stochastic
//! and are not ported.

use std::collections::BTreeMap;
use std::io;

use faer::Mat;
use serde_json::{json, Value};

use crate::matrix::DataMatrix;

pub const XCOLNAME: &str = "projection_X";
pub const YCOLNAME: &str = "projection_Y";
pub const ZCOLNAME: &str = "projection_Z";

/// Matrices with at least this fraction of non-zero cells are projected as dense
/// arrays.
pub const DENSE_PROJECTION_MIN_FILL: f64 = 0.5;

/// The algorithms upstream offers, and the ones this port computes.
pub const ALGORITHMS: [&str; 4] = ["pca", "tsne", "umap", "coords"];
pub const SUPPORTED_ALGORITHMS: [&str; 2] = ["pca", "coords"];

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, msg.into())
}

/// A keyword argument's value: a Python literal, or failing that the text itself
/// (`parse_keyword_args`).
#[derive(Clone, Debug, PartialEq)]
pub enum KeywordValue {
    Bool(bool),
    None,
    Int(i64),
    Float(f64),
    Str(String),
}

impl KeywordValue {
    fn literal(text: &str) -> KeywordValue {
        match text.trim() {
            "True" => return KeywordValue::Bool(true),
            "False" => return KeywordValue::Bool(false),
            "None" => return KeywordValue::None,
            _ => {}
        }
        let t = text.trim();
        let digits = t.strip_prefix(['-', '+']).unwrap_or(t);
        if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit() || c == '_') {
            if let Ok(v) = t.replace('_', "").parse::<i64>() {
                return KeywordValue::Int(v);
            }
        }
        let looks_numeric = digits
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit() || c == '.');
        if looks_numeric {
            if let Ok(v) = t.parse::<f64>() {
                return KeywordValue::Float(v);
            }
        }
        for quote in ['\'', '"'] {
            if t.len() >= 2 && t.starts_with(quote) && t.ends_with(quote) {
                return KeywordValue::Str(t[1..t.len() - 1].to_string());
            }
        }
        KeywordValue::Str(text.to_string())
    }

    fn truthy(&self) -> bool {
        match self {
            KeywordValue::Bool(b) => *b,
            KeywordValue::None => false,
            KeywordValue::Int(i) => *i != 0,
            KeywordValue::Float(f) => *f != 0.0,
            KeywordValue::Str(s) => !s.is_empty(),
        }
    }
}

/// Parse `"key=value key2=value2"` (split like a shell command line) into keyword
/// arguments (`parse_keyword_args`). `option` names the command-line option in error
/// messages.
pub fn parse_keyword_args(
    text: Option<&str>,
    option: &str,
) -> io::Result<BTreeMap<String, KeywordValue>> {
    let mut out = BTreeMap::new();
    let tokens = shlex::split(text.unwrap_or(""))
        .ok_or_else(|| invalid(format!("{option}: No closing quotation")))?;
    for token in tokens {
        let (key, value) = token.split_once('=').unwrap_or((token.as_str(), ""));
        let identifier = key
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
            && key.chars().all(|c| c.is_alphanumeric() || c == '_');
        if !token.contains('=') || !identifier {
            return Err(invalid(format!(
                "{option} expects key=value pairs, such as {option}=\"n_neighbors=30 \
                 min_dist=0.1\", but got '{token}'."
            )));
        }
        out.insert(key.to_string(), KeywordValue::literal(value));
    }
    Ok(out)
}

/// Whether upstream projects `matrix` as a sparse matrix (`TruncatedSVD`) rather than a
/// dense one (`PCA`): fewer than half its cells hold a value.
pub fn is_sparse(matrix: &DataMatrix) -> bool {
    let size = matrix.nrows() * matrix.ncols();
    let nnz = matrix.densify().iter().filter(|&&v| v != 0.0).count();
    (nnz as f64) < DENSE_PROJECTION_MIN_FILL * size as f64
}

/// The top `k` singular triplets of the `rows x cols` row-major `data`, with signs
/// fixed as `svd_flip(u_based_decision=False)` fixes them: each right singular vector's
/// largest-magnitude entry (the first, on a tie) is positive. Returns `U[:, :k] * S`,
/// or `U[:, :k]` scaled by `scale_u` when it is given (PCA's whitening).
fn flipped_svd_scores(
    data: &[f64],
    rows: usize,
    cols: usize,
    k: usize,
    scale_u: Option<f64>,
) -> io::Result<Vec<Vec<f64>>> {
    let m = Mat::<f64>::from_fn(rows, cols, |i, j| data[i * cols + j]);
    let svd = m
        .thin_svd()
        .map_err(|e| invalid(format!("The SVD did not converge: {e:?}")))?;
    let (u, v) = (svd.U(), svd.V());
    let s = svd.S().column_vector();
    // Singular values in decreasing order (stable, so equal ones keep faer's order).
    let mut order: Vec<usize> = (0..s.nrows()).collect();
    order.sort_by(|&a, &b| s[b].total_cmp(&s[a]));
    let mut scores = vec![vec![0.0; rows]; k];
    for (axis, &c) in order.iter().take(k).enumerate() {
        let mut best = 0usize;
        for j in 0..cols {
            if v[(j, c)].abs() > v[(best, c)].abs() {
                best = j;
            }
        }
        let sign = if v[(best, c)] > 0.0 {
            1.0
        } else if v[(best, c)] < 0.0 {
            -1.0
        } else {
            0.0
        };
        let factor = scale_u.unwrap_or(s[c]);
        for i in 0..rows {
            scores[axis][i] = u[(i, c)] * sign * factor;
        }
    }
    Ok(scores)
}

/// scikit-learn's `PCA(n_components=k).fit_transform` of the rows of `data`: the
/// centered data's top `k` principal component scores, `scores[axis][row]`. `whiten`
/// scales each to unit variance.
pub fn pca(
    data: &[f64],
    rows: usize,
    cols: usize,
    k: usize,
    whiten: bool,
) -> io::Result<Vec<Vec<f64>>> {
    if k > rows.min(cols) {
        return Err(invalid(format!(
            "n_components={k} must be between 0 and min(n_samples, n_features)={} with \
             svd_solver='full'",
            rows.min(cols)
        )));
    }
    let mut centered = data.to_vec();
    for j in 0..cols {
        let mean = (0..rows).map(|i| data[i * cols + j]).sum::<f64>() / rows as f64;
        for i in 0..rows {
            centered[i * cols + j] -= mean;
        }
    }
    let whiten = whiten.then(|| ((rows as f64) - 1.0).sqrt());
    flipped_svd_scores(&centered, rows, cols, k, whiten)
}

/// scikit-learn's `TruncatedSVD(n_components=k).fit_transform` of the rows of `data`:
/// the uncentered data's top `k` singular-vector scores, `scores[axis][row]`.
pub fn truncated_svd(
    data: &[f64],
    rows: usize,
    cols: usize,
    k: usize,
) -> io::Result<Vec<Vec<f64>>> {
    if k > cols {
        return Err(invalid(format!(
            "n_components({k}) must be <= n_features({cols})."
        )));
    }
    flipped_svd_scores(data, rows, cols, k, None)
}

/// The keyword arguments of scikit-learn's `PCA` this port accepts. Of these only
/// `whiten` changes the result: the solver is always the exact one, and its random
/// state is unused.
const PCA_ARGS: [&str; 5] = [
    "whiten",
    "svd_solver",
    "random_state",
    "tol",
    "iterated_power",
];
/// The keyword arguments scikit-learn's `TruncatedSVD` takes, none of which changes an
/// exact SVD.
const TRUNCATED_SVD_ARGS: [&str; 6] = [
    "n_components",
    "algorithm",
    "n_iter",
    "n_oversamples",
    "power_iteration_normalizer",
    "random_state",
];
/// The `PCA` keyword arguments upstream would accept but this port does not compute.
const PCA_UNSUPPORTED: [&str; 3] = ["n_components", "copy", "n_oversamples"];

/// Project `matrix`'s rows with `algorithm` into `n_components` dimensions
/// (`scores[axis][row]`), before centering and scaling.
pub fn project(
    matrix: &DataMatrix,
    algorithm: &str,
    n_components: usize,
    parameters: &BTreeMap<String, KeywordValue>,
) -> io::Result<Vec<Vec<f64>>> {
    let (rows, cols) = (matrix.nrows(), matrix.ncols());
    match algorithm {
        "pca" if is_sparse(matrix) => {
            let unsupported: Vec<&str> = parameters
                .keys()
                .map(String::as_str)
                .filter(|k| !TRUNCATED_SVD_ARGS.contains(k))
                .collect();
            if !unsupported.is_empty() {
                return Err(invalid(format!(
                    "A sparse matrix is projected with TruncatedSVD instead of PCA, which does \
                     not take {}.",
                    unsupported.join(", ")
                )));
            }
            truncated_svd(&matrix.densify(), rows, cols, n_components)
        }
        "pca" => {
            let unsupported: Vec<&str> = parameters
                .keys()
                .map(String::as_str)
                .filter(|k| !PCA_ARGS.contains(k))
                .collect();
            if let Some(first) = unsupported.first() {
                let reason = if PCA_UNSUPPORTED.contains(first) {
                    "is not supported by domainatoredux"
                } else {
                    "is not a PCA parameter"
                };
                return Err(invalid(format!("--pca_args {first} {reason}.")));
            }
            let whiten = parameters.get("whiten").is_some_and(KeywordValue::truthy);
            pca(&matrix.densify(), rows, cols, n_components, whiten)
        }
        "coords" => {
            if !parameters.is_empty() {
                return Err(invalid(format!(
                    "algorithm parameters do not apply to --algorithm coords, which reads the \
                     coordinates as given: {:?}",
                    parameters.keys().collect::<Vec<_>>()
                )));
            }
            check_coords(matrix, n_components)
        }
        "tsne" | "umap" => Err(invalid(format!(
            "--algorithm {algorithm} is not supported by domainatoredux yet; use pca, or \
             project with upstream domainator and read the result with --algorithm coords."
        ))),
        other => Err(invalid(format!(
            "algorithm must be one of {ALGORITHMS:?}, not '{other}'"
        ))),
    }
}

/// The coordinates a matrix already holds (`check_coords`): its columns must be exactly
/// `projection_X`, `projection_Y` (and `projection_Z` in 3D).
pub fn check_coords(matrix: &DataMatrix, n_components: usize) -> io::Result<Vec<Vec<f64>>> {
    let expected: &[&str] = if n_components == 3 {
        &[XCOLNAME, YCOLNAME, ZCOLNAME]
    } else {
        &[XCOLNAME, YCOLNAME]
    };
    let words = if n_components == 3 { "three" } else { "two" };
    if matrix.ncols() != expected.len() {
        return Err(invalid(format!(
            "Input matrix should have {words} columns, not: {:?}",
            matrix.cols
        )));
    }
    if matrix.cols.iter().zip(expected).any(|(c, e)| c != e) {
        let order = if n_components == 3 { " (in order)" } else { "" };
        return Err(invalid(format!(
            "Input matrix should have columns should be{order} {expected:?}, not: {:?}",
            matrix.cols
        )));
    }
    let dense = matrix.densify();
    let cols = matrix.ncols();
    Ok((0..cols)
        .map(|c| (0..matrix.nrows()).map(|r| dense[r * cols + c]).collect())
        .collect())
}

/// Move the smallest value along each axis to zero, then scale every axis by the same
/// factor so that the largest value is `max_value` (`center_and_scale`).
pub fn center_and_scale(scores: &mut [Vec<f64>], max_value: f64) {
    for axis in scores.iter_mut() {
        let min = axis.iter().cloned().fold(f64::INFINITY, f64::min);
        for v in axis.iter_mut() {
            *v -= min;
        }
    }
    let max = scores
        .iter()
        .flatten()
        .cloned()
        .fold(f64::NEG_INFINITY, f64::max);
    let factor = max_value / max;
    for v in scores.iter_mut().flatten() {
        *v *= factor;
    }
}

/// The axes' column names for `n_components` dimensions.
pub fn axes(n_components: usize) -> Vec<&'static str> {
    [XCOLNAME, YCOLNAME, ZCOLNAME][..n_components].to_vec()
}

/// The bundle's `views.projection`: `{algorithm, axes, coords}`, `coords[a][i]` being
/// node `i`'s coordinate on axis `a`.
pub fn projection_view(algorithm: &str, coords: &[Vec<f64>]) -> Value {
    let coords: Vec<Vec<Value>> = coords
        .iter()
        .map(|axis| {
            axis.iter()
                .map(|&v| crate::tools::viewer_bundle::num_or_null(v))
                .collect()
        })
        .collect();
    json!({
        "algorithm": algorithm,
        "axes": axes(coords.len()),
        "coords": coords,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyword_args_are_python_literals() {
        let args = parse_keyword_args(
            Some("whiten=True n=30 x=0.1 s='a b' raw=abc none=None"),
            "--pca_args",
        )
        .unwrap();
        assert_eq!(args["whiten"], KeywordValue::Bool(true));
        assert_eq!(args["n"], KeywordValue::Int(30));
        assert_eq!(args["x"], KeywordValue::Float(0.1));
        assert_eq!(args["s"], KeywordValue::Str("a b".into()));
        assert_eq!(args["raw"], KeywordValue::Str("abc".into()));
        assert_eq!(args["none"], KeywordValue::None);
        let err = parse_keyword_args(Some("novalue"), "--pca_args").unwrap_err();
        assert!(err.to_string().contains("expects key=value pairs"));
        assert!(parse_keyword_args(None, "--pca_args").unwrap().is_empty());
    }

    #[test]
    fn pca_matches_scikit_learn() {
        // sklearn.decomposition.PCA(n_components=2).fit_transform(X) for this X, from
        // scikit-learn 1.9.1: signs fixed by the right singular vectors.
        let x = [
            1.0, 2.0, 0.5, //
            3.0, 1.0, 2.0, //
            0.0, 4.0, 1.0, //
            2.0, 2.0, 3.0,
        ];
        let scores = pca(&x, 4, 3, 2, false).unwrap();
        let expected = [
            [
                -0.6391305206771191,
                1.9354687056314472,
                -2.344753470673897,
                1.0484152857195694,
            ],
            [
                -1.0632725149375335,
                -0.39664681859790324,
                0.42511123385399724,
                1.0348080996814393,
            ],
        ];
        for axis in 0..2 {
            for row in 0..4 {
                assert!(
                    (scores[axis][row] - expected[axis][row]).abs() < 1e-12,
                    "axis {axis} row {row}: {} vs {}",
                    scores[axis][row],
                    expected[axis][row]
                );
            }
        }
        assert!(pca(&x, 4, 3, 4, false).is_err());
    }

    #[test]
    fn centering_and_scaling() {
        let mut scores = vec![vec![-1.0, 1.0, 3.0], vec![2.0, 2.5, 4.0]];
        center_and_scale(&mut scores, 1000.0);
        assert_eq!(scores, [vec![0.0, 500.0, 1000.0], vec![0.0, 125.0, 500.0]]);
    }

    #[test]
    fn coords_are_checked_and_unported_algorithms_refused() {
        let m = DataMatrix {
            rows: vec!["a".into()],
            cols: vec![XCOLNAME.into(), YCOLNAME.into()],
            row_lengths: None,
            col_lengths: None,
            data_type: String::new(),
            storage: crate::matrix::Storage::Dense(vec![1.0, 2.0]),
        };
        assert_eq!(check_coords(&m, 2).unwrap(), [vec![1.0], vec![2.0]]);
        assert!(check_coords(&m, 3)
            .unwrap_err()
            .to_string()
            .contains("three columns"));
        let err = project(&m, "umap", 2, &BTreeMap::new()).unwrap_err();
        assert!(err.to_string().contains("not supported by domainatoredux"));
    }
}
