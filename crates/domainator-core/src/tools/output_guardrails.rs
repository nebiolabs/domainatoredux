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

/// `format_bytes`: a decimal byte count for people (`999 B`, `1.50 KB`, `2.00 GB`).
pub fn format_bytes(bytes: u64) -> String {
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    for unit in ["KB", "MB", "GB", "TB", "PB"] {
        value /= 1000.0;
        if value < 1000.0 || unit == "PB" {
            return format!("{value:.2} {unit}");
        }
    }
    unreachable!("PB ends the loop")
}

/// `build_output_limit_message`: why a run stopped at `--max_output_gb`, and which of
/// its options would shrink the output.
pub fn output_limit_message(
    description: &str,
    projected_bytes: u64,
    max_bytes: u64,
    mitigation: &[&str],
) -> String {
    let mut message = format!(
        "Projected {description} size is {}, which exceeds the configured limit of {}. \
         Reduce the projected output size or raise the limit with --max_output_gb.",
        format_bytes(projected_bytes),
        format_bytes(max_bytes)
    );
    if !mitigation.is_empty() {
        message.push_str(&format!(" Relevant options: {}.", mitigation.join(", ")));
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_read_like_upstreams() {
        assert_eq!(format_bytes(999), "999 B");
        assert_eq!(format_bytes(1500), "1.50 KB");
        assert_eq!(format_bytes(25_000_000_000), "25.00 GB");
        assert_eq!(
            output_limit_message("GenBank output 'a.gb'", 2000, 1000, &["--hits_only"]),
            "Projected GenBank output 'a.gb' size is 2.00 KB, which exceeds the configured \
             limit of 1.00 KB. Reduce the projected output size or raise the limit with \
             --max_output_gb. Relevant options: --hits_only."
        );
    }

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
