//! With the `cuda` feature: tell the linker where the CUDA toolkit's libraries are.
//!
//! candle-kernels links its statically compiled kernels against `cudart` and emits
//! `-lcudart` with no search path. cudarc's `dynamic-linking` used to supply one, but the
//! vendored candle-core switches cudarc to `dynamic-loading`
//! (`vendor/candle-core/PATCHES.md`), which supplies none. The finished binary does not
//! depend on cudart at load time (LTO drops the unused kernels and `--as-needed` the
//! library; the release workflow checks this), but the linker still has to find it. The
//! roots and subdirectories are cudarc's (`build.rs`, `link_searches`).

use std::path::PathBuf;

const ROOT_VARS: [&str; 4] = [
    "CUDA_HOME",
    "CUDA_PATH",
    "CUDA_ROOT",
    "CUDA_TOOLKIT_ROOT_DIR",
];
// Windows has no fixed root: the CUDA installer sets `CUDA_PATH` (`...\CUDA\v12.4`).
const TYPICAL_ROOTS: [&str; 3] = ["/usr/local/cuda", "/opt/cuda", "/usr/lib/cuda"];
const LIB_DIRS: [&str; 5] = [
    "lib64",
    "lib",
    "lib/x64",
    "targets/x86_64-linux/lib",
    "targets/sbsa-linux/lib",
];

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    for var in ROOT_VARS {
        println!("cargo::rerun-if-env-changed={var}");
    }
    if std::env::var_os("CARGO_FEATURE_CUDA").is_none() {
        return;
    }
    let from_env: Vec<PathBuf> = ROOT_VARS
        .iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .collect();
    let roots = if from_env.is_empty() {
        TYPICAL_ROOTS.iter().map(PathBuf::from).collect()
    } else {
        from_env
    };
    for root in roots {
        for dir in LIB_DIRS {
            let path = root.join(dir);
            if path.is_dir() {
                println!("cargo::rustc-link-search=native={}", path.display());
            }
        }
    }
}
