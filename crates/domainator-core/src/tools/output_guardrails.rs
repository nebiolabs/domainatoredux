//! Port of `domainator.output_guardrails`: the `--max_output_gb` limit shared by the
//! tools that can write very large outputs.

/// Bytes in one `--max_output_gb` gigabyte (decimal, as upstream).
pub const BYTES_PER_GB: f64 = 1_000_000_000.0;

/// `max_output_gb_to_bytes`: a GB limit as a byte budget, or `None` when `0`
/// disables the guardrail.
///
/// Negative is an error, as upstream. So are NaN and infinity: upstream's
/// `int(max_output_gb * BYTES_PER_GB)` raises on both, while a bare `as u64` would
/// silently turn NaN into a zero-byte budget that every output exceeds.
pub fn max_output_gb_to_bytes(max_gb: f64) -> Result<Option<u64>, String> {
    if max_gb < 0.0 {
        return Err("--max_output_gb must be >= 0.".into());
    }
    if !max_gb.is_finite() {
        return Err(format!(
            "--max_output_gb must be a finite number, not {max_gb}."
        ));
    }
    if max_gb == 0.0 {
        return Ok(None);
    }
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)] // finite and > 0 here
    Ok(Some((max_gb * BYTES_PER_GB) as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_disables_and_rejects() {
        assert_eq!(max_output_gb_to_bytes(0.0), Ok(None));
        assert_eq!(max_output_gb_to_bytes(1.5), Ok(Some(1_500_000_000)));
        assert_eq!(max_output_gb_to_bytes(25.0), Ok(Some(25_000_000_000)));
        for bad in [-1.0, f64::NAN, f64::INFINITY] {
            assert!(max_output_gb_to_bytes(bad).is_err(), "{bad}");
        }
    }
}
