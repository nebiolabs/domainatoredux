//! Protein language model inference for domainatoredux, on [candle] (pure Rust on the
//! CPU; CUDA and Metal behind the `cuda`/`metal` features).
//!
//! * [`t5`]: the T5 encoder (ProtT5's embeddings).
//! * [`tmbed`]: TMbed's transmembrane and signal peptide prediction (upstream
//!   `find_features --tmbed`): ProtT5 embeddings, a CNN ensemble and a Viterbi decoder.
//!
//! This crate only runs models from files on disk; finding or downloading them is the
//! caller's business.
//!
//! [candle]: https://github.com/huggingface/candle

pub mod t5;
pub mod tmbed;

pub use candle_core::Device;

/// The device for an upstream-style device string, and a warning when it falls back
/// to the CPU. `None`, `"None"`, `"none"` and `"cpu"` are the CPU; `"cuda"`, `"cuda:N"`
/// or a bare `"N"` a CUDA GPU; `"metal"` or `"mps"` Apple's GPU. A GPU this build or
/// machine lacks falls back to the CPU with a warning, as upstream does when CUDA is not
/// available.
pub fn select_device(device: Option<&str>) -> (Device, Option<String>) {
    let Some(name) = device.map(str::trim) else {
        return (Device::Cpu, None);
    };
    if name.is_empty() || name.eq_ignore_ascii_case("none") || name == "cpu" {
        return (Device::Cpu, None);
    }
    let fallback = |why: String| {
        (
            Device::Cpu,
            Some(format!("GPU device {name} requested but {why}. Using CPU.")),
        )
    };
    if name == "metal" || name == "mps" {
        return match Device::new_metal(0) {
            Ok(d) => (d, None),
            Err(e) => fallback(format!("Metal is not available ({e})")),
        };
    }
    let index = if let Some(index) = name.strip_prefix("cuda:") {
        index.parse::<usize>().ok()
    } else if name == "cuda" {
        Some(0)
    } else {
        name.parse::<usize>().ok()
    };
    let Some(index) = index else {
        return fallback("it is not a device this build knows".to_string());
    };
    if !candle_core::utils::cuda_is_available() {
        return fallback("this build has no CUDA support (the `cuda` feature)".to_string());
    }
    match Device::new_cuda(index) {
        Ok(d) => (d, None),
        Err(e) => fallback(format!("CUDA is not available ({e})")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_spellings_select_the_cpu_silently() {
        for name in [None, Some("None"), Some("none"), Some("cpu"), Some("")] {
            let (device, warning) = select_device(name);
            assert!(device.is_cpu(), "{name:?}");
            assert!(warning.is_none(), "{name:?}");
        }
    }

    #[test]
    fn an_unknown_device_falls_back_with_a_warning() {
        let (device, warning) = select_device(Some("tpu"));
        assert!(device.is_cpu());
        assert!(warning.unwrap().contains("tpu"));
    }
}
