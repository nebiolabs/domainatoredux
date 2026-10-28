//! Plain-text plots and tables for the text reports (upstream `text_plot.py`).

use crate::search::results::py_g;
use crate::stats::floor_index;

/// `numpy.histogram(values, bins)`: the counts and the `bins + 1` edges.
///
/// The bins split `[min, max]` evenly (`[min - 0.5, max + 0.5]` when the two are equal,
/// `[0, 1]` when there are no values), each holding the values from its low edge up to,
/// not including, its high edge, except the last, which holds `max` too. A value's bin
/// is found the way numpy finds it, by scaling and then correcting by one against the
/// edges, so a value within a rounding error of an edge lands where numpy puts it.
fn numpy_histogram(values: &[f64], bins: usize) -> (Vec<u64>, Vec<f64>) {
    let (mut first, mut last) = if values.is_empty() {
        (0.0, 1.0)
    } else {
        values
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &v| {
                (lo.min(v), hi.max(v))
            })
    };
    if first == last {
        first -= 0.5;
        last += 0.5;
    }
    // numpy.linspace: i * step + start, the last edge set to stop.
    let step = (last - first) / bins as f64;
    let mut edges: Vec<f64> = (0..=bins).map(|i| i as f64 * step + first).collect();
    edges[bins] = last;

    let mut counts = vec![0u64; bins];
    let span = last - first;
    for &value in values {
        // numpy truncates (value >= first, so that is the floor) and puts max, at index
        // bins, in the last bin.
        let mut index = floor_index((value - first) / span * bins as f64, bins);
        if value < edges[index] {
            index -= 1;
        } else if value >= edges[index + 1] && index != bins - 1 {
            index += 1;
        }
        counts[index] += 1;
    }
    (counts, edges)
}

/// A histogram of `values` drawn with characters, one column per bin, `height` rows tall
/// (upstream `text_histogram`).
pub fn text_histogram(values: &[f64], title: &str, bins: usize, height: usize) -> String {
    let (counts, edges) = numpy_histogram(values, bins);
    let top = counts.iter().copied().max().unwrap_or(0);
    if top == 0 {
        return format!(" {title}\n (no values)\n");
    }
    let label_width = top.to_string().len();
    let mut lines = vec![format!(" {title}")];
    for row in (1..=height).rev() {
        let threshold = top as f64 * (row as f64 - 0.5) / height as f64;
        let label = if row == height || row == 1 || row % 5 == 0 {
            // Python's round(): halves to even.
            format!(
                "{}",
                (top as f64 * row as f64 / height as f64).round_ties_even()
            )
        } else {
            String::new()
        };
        let bars: String = counts
            .iter()
            .map(|&count| if count as f64 >= threshold { 'o' } else { ' ' })
            .collect();
        lines.push(format!(" {label:>label_width$} | {bars}"));
    }
    let pad = " ".repeat(label_width);
    lines.push(format!(" {pad} +-{}", "-".repeat(counts.len())));
    lines.push(format!(
        " {pad}   {} to {}, {} bins",
        py_g(edges[0]),
        py_g(edges[bins]),
        counts.len()
    ));
    lines.join("\n") + "\n"
}

/// Columns of text two spaces apart, each as wide as its widest cell, a header line first
/// (upstream `text_table`).
///
/// The cells are already formatted. A column is right-aligned (numbers) or left-aligned
/// (text), its header with it, and every line loses its trailing spaces. Widths count
/// characters, as Python's `len` does.
pub fn text_table(header: &[&str], rows: &[Vec<String>], right_aligned: &[bool]) -> String {
    let mut widths: Vec<usize> = header.iter().map(|name| name.chars().count()).collect();
    for row in rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let line = |cells: &mut dyn Iterator<Item = &str>| {
        let padded: Vec<String> = cells
            .zip(widths.iter().zip(right_aligned))
            .map(|(cell, (&width, &right))| {
                if right {
                    format!("{cell:>width$}")
                } else {
                    format!("{cell:<width$}")
                }
            })
            .collect();
        padded.join("  ").trim_end().to_string() + "\n"
    };
    let mut out = line(&mut header.iter().copied());
    for row in rows {
        out.push_str(&line(&mut row.iter().map(String::as_str)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_on_edges_land_in_numpys_bins() {
        // numpy.histogram([0, 1, 2, 3, 4], bins=4) -> [1, 1, 1, 2], edges 0, 1, 2, 3, 4.
        let (counts, edges) = numpy_histogram(&[0.0, 1.0, 2.0, 3.0, 4.0], 4);
        assert_eq!(counts, [1, 1, 1, 2]);
        assert_eq!(edges, [0.0, 1.0, 2.0, 3.0, 4.0]);
        // numpy.histogram([0.1, 0.2, 0.3], bins=3): 0.2 is not exactly an edge.
        assert_eq!(numpy_histogram(&[0.1, 0.2, 0.3], 3).0, [1, 1, 1]);
        // One distinct value: the range widens by a half each way.
        let (counts, edges) = numpy_histogram(&[5.0, 5.0], 2);
        assert_eq!((counts, edges), (vec![0, 2], vec![4.5, 5.0, 5.5]));
    }

    #[test]
    fn histogram_matches_upstreams_layout() {
        // text_histogram([1, 2, 2, 3, 3, 3], title="t", bins=3, height=5) in Python.
        let expected =
            " t\n 3 |   o\n   |   o\n   |  oo\n   | ooo\n 1 | ooo\n   +----\n     1 to 3, 3 bins\n";
        assert_eq!(
            text_histogram(&[1.0, 2.0, 2.0, 3.0, 3.0, 3.0], "t", 3, 5),
            expected
        );
        assert_eq!(text_histogram(&[], "t", 3, 5), " t\n (no values)\n");
    }

    #[test]
    fn text_table_aligns_columns() {
        let rows = vec![
            vec!["a".to_string(), "8".to_string(), "x".to_string()],
            vec!["longer".to_string(), "12".to_string(), String::new()],
        ];
        assert_eq!(
            text_table(&["name", "count", "note"], &rows, &[false, true, false]),
            "name    count  note\na           8  x\nlonger     12\n"
        );
        assert_eq!(text_table(&["name", "n"], &[], &[false, true]), "name  n\n");
        // Widths count characters, not bytes.
        let rows = vec![vec!["é".to_string(), "1".to_string()]];
        assert_eq!(
            text_table(&["ab", "n"], &rows, &[false, true]),
            "ab  n\né   1\n"
        );
    }
}
