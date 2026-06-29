//! Dense TSV output for [`DataMatrix`], matching
//! `domainator.data_matrix.DataMatrix.write_dense_text`: a leading empty header
//! cell, tab-separated column labels, then one row per record prefixed by its
//! label. Values use NumPy's `str(float64)` formatting.

use std::io::{self, Write};

use super::DataMatrix;

/// Format `x` like NumPy/Python `str(np.float64(x))`: Rust's shortest round-trip
/// Display, but always with a decimal point for finite values (`410` -> `410.0`),
/// and `nan`/`inf`/`-inf` spelled the Python way.
pub fn py_float_str(x: f64) -> String {
    if x.is_nan() {
        return "nan".to_string();
    }
    if x.is_infinite() {
        return if x < 0.0 { "-inf".to_string() } else { "inf".to_string() };
    }
    let s = format!("{x}");
    if s.contains('.') || s.contains('e') || s.contains('E') {
        s
    } else {
        format!("{s}.0")
    }
}

/// Write `m` as a dense tab-separated matrix to `path`.
pub fn write_dense_text(path: &str, m: &DataMatrix) -> io::Result<()> {
    let (nr, nc) = (m.nrows(), m.ncols());
    let dense = m.densify();
    let file = std::fs::File::create(path)?;
    let mut w = io::BufWriter::new(file);

    // Header: empty first cell, then column labels.
    w.write_all(b"\t")?;
    w.write_all(m.cols.join("\t").as_bytes())?;
    w.write_all(b"\n")?;

    let mut line = String::new();
    for r in 0..nr {
        line.clear();
        line.push_str(&m.rows[r]);
        for c in 0..nc {
            line.push('\t');
            line.push_str(&py_float_str(dense[r * nc + c]));
        }
        line.push('\n');
        w.write_all(line.as_bytes())?;
    }
    w.flush()
}
