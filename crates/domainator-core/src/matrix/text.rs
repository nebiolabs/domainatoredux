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
        return if x < 0.0 {
            "-inf".to_string()
        } else {
            "inf".to_string()
        };
    }
    let s = format!("{x}");
    if s.contains('.') || s.contains('e') || s.contains('E') {
        s
    } else {
        format!("{s}.0")
    }
}

/// Read a dense matrix from a tab-separated file, matching
/// `domainator.data_matrix.DenseDataMatrix._from_text`: first row is an empty
/// cell followed by column labels; each subsequent row is a label followed by
/// numeric values. `data_type` is empty and lengths are absent (as in Python).
pub fn read_dense_text(path: &str) -> io::Result<DataMatrix> {
    use super::Storage;
    let text = std::fs::read_to_string(path)?;
    let mut lines = text.lines();
    let header = lines.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "Dense text matrix file is empty.",
        )
    })?;
    let cols: Vec<String> = header.split('\t').skip(1).map(|s| s.to_string()).collect();

    let mut rows: Vec<String> = Vec::new();
    let mut data: Vec<f64> = Vec::new();
    for (line_number, line) in lines.enumerate() {
        if line.is_empty() {
            continue;
        }
        let mut fields = line.split('\t');
        let label = fields.next().unwrap_or_default().to_string();
        let mut count = 0;
        for f in fields {
            let v: f64 = f.trim().parse().map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "Dense text matrix values must be numeric (row {})",
                        line_number + 2
                    ),
                )
            })?;
            data.push(v);
            count += 1;
        }
        if count != cols.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Dense text matrix row {} has {} values; expected {}.",
                    line_number + 2,
                    count,
                    cols.len()
                ),
            ));
        }
        rows.push(label);
    }

    Ok(DataMatrix {
        rows,
        cols,
        row_lengths: None,
        col_lengths: None,
        data_type: String::new(),
        storage: Storage::Dense(data),
    })
}

/// Write `m` as a dense tab-separated matrix to `path`.
pub fn write_dense_text(path: &str, m: &DataMatrix) -> io::Result<()> {
    let file = std::fs::File::create(path)?;
    write_dense_text_to(&mut io::BufWriter::new(file), m)
}

/// [`write_dense_text`] to any writer.
pub fn write_dense_text_to(w: &mut impl Write, m: &DataMatrix) -> io::Result<()> {
    let (nr, nc) = (m.nrows(), m.ncols());
    let dense = m.densify();

    // Header: empty first cell, then column labels.
    w.write_all(b"\t")?;
    w.write_all(m.cols.join("\t").as_bytes())?;
    w.write_all(b"\n")?;

    // Upstream's bool matrices are int8, which numpy prints without a decimal point.
    let integers = m.is_integer_bool();
    let mut line = String::new();
    for r in 0..nr {
        line.clear();
        line.push_str(&m.rows[r]);
        for c in 0..nc {
            line.push('\t');
            let v = dense[r * nc + c];
            if integers {
                line.push(if v == 1.0 { '1' } else { '0' });
            } else {
                line.push_str(&py_float_str(v));
            }
        }
        line.push('\n');
        w.write_all(line.as_bytes())?;
    }
    w.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn py_float_str_formatting() {
        assert_eq!(py_float_str(410.0), "410.0");
        assert_eq!(py_float_str(0.0), "0.0");
        assert_eq!(py_float_str(-2.0), "-2.0");
        assert_eq!(py_float_str(0.5), "0.5");
        assert_eq!(py_float_str(f64::NAN), "nan");
        assert_eq!(py_float_str(f64::INFINITY), "inf");
        assert_eq!(py_float_str(f64::NEG_INFINITY), "-inf");
    }
}
