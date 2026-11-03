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

/// A shared library a `cuda` build opens at run time, and whether this machine has it.
///
/// candle is patched to have cudarc load these on first use instead of linking them
/// (`vendor/candle-core/PATCHES.md`), so a `cuda` build starts without them and runs on
/// the CPU. NVRTC, which cudarc can also load, is not listed: candle runs precompiled
/// PTX and never calls it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CudaLibrary {
    /// What it is called in NVIDIA's documentation, e.g. `cuBLAS`.
    pub name: &'static str,
    /// The file the loader looks for on this platform, e.g. `libcublas.so.12`.
    pub file: &'static str,
    pub present: bool,
}

impl std::fmt::Display for CudaLibrary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.name, self.file)
    }
}

/// The CUDA libraries a GPU device needs, each checked by trying to load it the way
/// cudarc will (every file name it tries, on the system library path, which includes
/// `PATH` on Windows). Empty in a build without the `cuda` feature.
#[must_use]
pub fn cuda_libraries() -> Vec<CudaLibrary> {
    #[cfg(feature = "cuda")]
    {
        use candle_core::cuda_backend::cudarc;
        let library = |name, linux, windows, check: unsafe fn() -> bool| CudaLibrary {
            name,
            file: if cfg!(windows) { windows } else { linux },
            // SAFETY: `is_culib_present` takes no arguments and only loads and unloads
            // the library by name, which cudarc does anyway on first use.
            present: unsafe { check() },
        };
        vec![
            library(
                "NVIDIA driver",
                "libcuda.so.1",
                "nvcuda.dll",
                cudarc::driver::sys::is_culib_present,
            ),
            library(
                "cuBLAS",
                "libcublas.so.12",
                "cublas64_12.dll",
                cudarc::cublas::sys::is_culib_present,
            ),
            library(
                "cuBLASLt",
                "libcublasLt.so.12",
                "cublasLt64_12.dll",
                cudarc::cublaslt::sys::is_culib_present,
            ),
            library(
                "cuRAND",
                "libcurand.so.10",
                "curand64_10.dll",
                cudarc::curand::sys::is_culib_present,
            ),
        ]
    }
    #[cfg(not(feature = "cuda"))]
    Vec::new()
}

/// Why a GPU device cannot be opened when some of `libraries` are absent, e.g. "this
/// CUDA library is missing: cuRAND (libcurand.so.10)"; `None` when all are present.
#[must_use]
pub fn missing_libraries(libraries: &[CudaLibrary]) -> Option<String> {
    let missing: Vec<String> = libraries
        .iter()
        .filter(|library| !library.present)
        .map(ToString::to_string)
        .collect();
    let what = if missing.len() == 1 {
        "a CUDA library is"
    } else {
        "CUDA libraries are"
    };
    (!missing.is_empty()).then(|| format!("{what} missing: {}", missing.join(", ")))
}

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
    // cudarc panics on first use of a library it cannot open, so check them all first.
    if let Some(why) = missing_libraries(&cuda_libraries()) {
        return fallback(why);
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
    fn a_missing_cuda_library_is_named_in_the_fallback_reason() {
        let library = |name, file, present| CudaLibrary {
            name,
            file,
            present,
        };
        let all = [
            library("NVIDIA driver", "libcuda.so.1", true),
            library("cuBLAS", "libcublas.so.12", true),
        ];
        assert_eq!(missing_libraries(&all), None);
        let one = [
            library("NVIDIA driver", "libcuda.so.1", true),
            library("cuRAND", "libcurand.so.10", false),
        ];
        assert_eq!(
            missing_libraries(&one).unwrap(),
            "a CUDA library is missing: cuRAND (libcurand.so.10)"
        );
        let two = [
            library("cuBLAS", "libcublas.so.12", false),
            library("cuRAND", "libcurand.so.10", false),
        ];
        assert_eq!(
            missing_libraries(&two).unwrap(),
            "CUDA libraries are missing: cuBLAS (libcublas.so.12), \
             cuRAND (libcurand.so.10)"
        );
    }

    #[cfg(not(feature = "cuda"))]
    #[test]
    fn a_build_without_cuda_lists_no_cuda_libraries() {
        assert!(cuda_libraries().is_empty());
    }

    #[test]
    fn an_unknown_device_falls_back_with_a_warning() {
        let (device, warning) = select_device(Some("tpu"));
        assert!(device.is_cpu());
        assert!(warning.unwrap().contains("tpu"));
    }
}
