//! Small numeric helpers shared by the reports and the GUI's summaries: numpy-style
//! percentiles, equal-width histograms, and the one float-to-index conversion they
//! both need.

/// `x.floor()` as an index into something with `len` items: clamped to `0..len`, so
/// a negative or NaN `x` gives 0 and anything past the end gives `len - 1`.
///
/// This is the one place a float becomes an index. `as usize` already saturates
/// (negatives and NaN become 0), but callers used to rely on that silently and clamp
/// the top by hand; here both ends are explicit.
///
/// # Panics
/// If `len` is 0, since no index exists.
#[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)] // clamped to 0..len first
pub fn floor_index(x: f64, len: usize) -> usize {
    assert!(len > 0, "floor_index into an empty collection");
    let f = x.floor();
    if f >= (len - 1) as f64 {
        len - 1
    } else if f > 0.0 {
        f as usize
    } else {
        0
    }
}

/// `np.percentile(sorted, q)` with numpy's default linear interpolation -- **not**
/// nearest-rank. `sorted` must be ascending; `q` is in percent. NaN when empty.
pub fn percentile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let pos = (sorted.len() - 1) as f64 * q / 100.0;
    let lo = floor_index(pos, sorted.len());
    if lo + 1 >= sorted.len() {
        return sorted[sorted.len() - 1];
    }
    let frac = pos - lo as f64;
    sorted[lo] + frac * (sorted[lo + 1] - sorted[lo])
}

/// Bin `values` (any order) into `bins` equal-width buckets spanning their
/// `[min, max]`, returning `(bin_low, count)` per bucket; the maximum lands in the
/// last bucket.
///
/// Empty when fewer than two values are finite, when they have no spread, or when
/// any value is infinite (there is no finite span to divide). NaN values are not
/// counted: saturating them to index 0 would report a missing score as the lowest.
pub fn histogram(values: &[f64], bins: usize) -> Vec<(f64, u64)> {
    if bins == 0 || values.len() < 2 {
        return Vec::new();
    }
    let not_nan = || values.iter().copied().filter(|v| !v.is_nan());
    let min = not_nan().fold(f64::INFINITY, f64::min);
    let max = not_nan().fold(f64::NEG_INFINITY, f64::max);
    if !(min.is_finite() && max.is_finite()) || max <= min {
        return Vec::new();
    }
    let width = (max - min) / bins as f64;
    let mut counts = vec![0u64; bins];
    for v in not_nan() {
        counts[floor_index((v - min) / width, bins)] += 1;
    }
    counts
        .into_iter()
        .enumerate()
        .map(|(i, c)| (min + i as f64 * width, c))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_index_clamps_both_ends() {
        assert_eq!(floor_index(-3.2, 5), 0);
        assert_eq!(floor_index(f64::NAN, 5), 0);
        assert_eq!(floor_index(0.0, 5), 0);
        assert_eq!(floor_index(2.9, 5), 2);
        assert_eq!(floor_index(4.0, 5), 4);
        assert_eq!(floor_index(99.0, 5), 4);
        assert_eq!(floor_index(f64::INFINITY, 5), 4);
    }

    #[test]
    fn percentile_matches_numpy_linear() {
        // np.percentile([1, 2, 3, 4], [0, 25, 50, 100]) == [1, 1.75, 2.5, 4]
        let v = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(percentile(&v, 0.0), 1.0);
        assert_eq!(percentile(&v, 25.0), 1.75);
        assert_eq!(percentile(&v, 50.0), 2.5);
        assert_eq!(percentile(&v, 100.0), 4.0);
        assert_eq!(percentile(&[7.0], 50.0), 7.0);
        assert!(percentile(&[], 50.0).is_nan());
    }

    #[test]
    fn histogram_bins_and_edge_cases() {
        assert!(histogram(&[], 4).is_empty());
        assert!(histogram(&[1.0], 4).is_empty());
        assert!(histogram(&[2.0, 2.0], 4).is_empty(), "no spread");
        assert!(histogram(&[f64::INFINITY, 1.0], 4).is_empty());
        assert!(
            histogram(&[f64::NAN, 1.0], 4).is_empty(),
            "one finite value"
        );
        let h = histogram(&[0.0, 1.0, 2.0, 3.0, 4.0], 4);
        assert_eq!(h, vec![(0.0, 1), (1.0, 1), (2.0, 1), (3.0, 2)]);
    }

    #[test]
    fn histogram_does_not_count_nan_in_the_lowest_bin() {
        let h = histogram(&[0.0, f64::NAN, 4.0], 2);
        assert_eq!(h, vec![(0.0, 1), (2.0, 1)]);
    }
}
