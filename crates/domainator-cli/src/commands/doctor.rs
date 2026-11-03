//! `domainatoredux doctor`: which optional external programs this installation can
//! run, and whether it can use a GPU (upstream `cli/doctor.py`, less its Python
//! packages, which have no counterpart; upstream's PyTorch line is this build's candle
//! backend here).

use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

use domainator_core::plm::{cuda_libraries, missing_libraries, CudaLibrary, Device};
use domainator_core::tools::external::EXTERNAL_PROGRAMS;
use domainator_core::version::LONG_VERSION;

const GPU_NEEDED_BY: &str = "find_features --device (TMbed); domainate --prostt5_device and \
                             --foldseek_device (a GPU build of foldseek)";

/// What `nvidia-smi` reports: the GPUs it lists, and the newest CUDA version their
/// driver supports.
struct NvidiaGpus {
    names: Vec<String>,
    driver_cuda: Option<String>,
}

/// The GPU backend this build was compiled with, and whether it can open a device.
enum Backend {
    Cpu,
    /// The CUDA libraries it looked for, and how many devices it can open (or why it
    /// can open none).
    Cuda {
        libraries: Vec<CudaLibrary>,
        devices: Result<usize, String>,
    },
    Metal(Result<(), String>),
}

/// `nvidia-smi`'s stdout, or `None` when it is not on PATH. A run that fails or
/// takes over 30 s, as upstream allows, counts as an empty listing.
fn nvidia_smi(args: &[&str]) -> Option<String> {
    let mut command = Command::new("nvidia-smi");
    command.args(args);
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = send.send(command.output());
    });
    match receive.recv_timeout(Duration::from_secs(30)) {
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => None,
        Ok(Ok(output)) => Some(String::from_utf8_lossy(&output.stdout).into_owned()),
        _ => Some(String::new()),
    }
}

fn nvidia_gpus() -> Option<NvidiaGpus> {
    let listing = nvidia_smi(&["-L"])?;
    let summary = nvidia_smi(&[]).unwrap_or_default();
    Some(parse_nvidia_smi(&listing, &summary))
}

/// The GPUs from `nvidia-smi -L` (each `GPU N: name`, less its UUID) and the driver's
/// CUDA version from `nvidia-smi`'s banner.
fn parse_nvidia_smi(listing: &str, summary: &str) -> NvidiaGpus {
    let names = listing
        .lines()
        .filter(|line| line.starts_with("GPU "))
        .map(|line| line.split(" (UUID").next().unwrap_or(line).to_string())
        .collect();
    let driver_cuda = summary.split_once("CUDA Version:").and_then(|(_, rest)| {
        let version: String = rest
            .trim_start()
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        (!version.is_empty()).then_some(version)
    });
    NvidiaGpus { names, driver_cuda }
}

fn backend() -> Backend {
    if cfg!(feature = "cuda") {
        let libraries = cuda_libraries();
        // cudarc panics on first use of a library it cannot open, so no device is
        // tried until every library is there.
        let devices = match missing_libraries(&libraries) {
            Some(why) => Err(why),
            None => count_cuda_devices(),
        };
        Backend::Cuda { libraries, devices }
    } else if cfg!(feature = "metal") {
        Backend::Metal(Device::new_metal(0).map(drop).map_err(|e| e.to_string()))
    } else {
        Backend::Cpu
    }
}

/// How many CUDA devices open, or why the first one does not.
fn count_cuda_devices() -> Result<usize, String> {
    let mut usable = 0;
    while usable < 64 {
        match Device::new_cuda(usable) {
            Ok(_) => usable += 1,
            Err(e) if usable == 0 => return Err(e.to_string()),
            Err(_) => break,
        }
    }
    Ok(usable)
}

fn backend_status(backend: &Backend) -> String {
    match backend {
        Backend::Cpu if cfg!(target_os = "macos") => {
            "CPU only; the macOS release builds use the Apple GPU (Metal)".to_string()
        }
        Backend::Cpu => "CPU only; for an NVIDIA GPU, install a CUDA build (the -cuda \
                         release downloads, for Linux and Windows), which uses the GPU when \
                         the CUDA 12 libraries cuBLAS and cuRAND (the CUDA Toolkit 12.x) \
                         are installed"
            .to_string(),
        Backend::Cuda { devices: Ok(0), .. } => "a CUDA build, cannot use a GPU".to_string(),
        Backend::Cuda {
            devices: Ok(usable),
            ..
        } => format!(
            "a CUDA build, can use {usable} GPU{}",
            if *usable == 1 { "" } else { "s" }
        ),
        // A CUDA build loads its libraries at run time, so it starts without them.
        Backend::Cuda {
            libraries,
            devices: Err(e),
        } if missing_libraries(libraries).is_some() => format!(
            "a CUDA build, cannot use a GPU because {e}, so it runs on the CPU; install \
             the NVIDIA driver and the CUDA Toolkit 12.x"
        ),
        // The release CUDA builds are compiled against CUDA 12.4 (build-release.yml).
        Backend::Cuda {
            devices: Err(e), ..
        } => format!(
            "a CUDA build, cannot use a GPU ({e}), so it runs on the CPU; the release CUDA \
             builds need an NVIDIA driver for CUDA 12.4 or newer"
        ),
        Backend::Metal(Ok(())) => "a Metal build, can use the Apple GPU".to_string(),
        Backend::Metal(Err(e)) => {
            format!("a Metal build, cannot use the Apple GPU ({e}), so it runs on the CPU")
        }
    }
}

/// The GPU section: what `nvidia-smi` finds, what this build can use, and which
/// options a GPU serves.
fn gpu_report(gpus: Option<NvidiaGpus>, backend: &Backend) -> Vec<String> {
    let found = match gpus {
        None => "none found (nvidia-smi is not on PATH)".to_string(),
        Some(gpus) if gpus.names.is_empty() => "none found (nvidia-smi lists no GPUs)".to_string(),
        Some(gpus) => {
            let mut found = gpus.names.join(", ");
            if let Some(cuda) = gpus.driver_cuda {
                found.push_str(&format!("; the driver supports CUDA {cuda} and older"));
            }
            found
        }
    };
    let mut lines = vec![
        String::new(),
        "GPU:".to_string(),
        format!("  NVIDIA GPU: {found}"),
        format!("  this build: {}", backend_status(backend)),
    ];
    if let Backend::Cuda { libraries, .. } = backend {
        let found: Vec<String> = libraries
            .iter()
            .map(|library| {
                let status = if library.present { "found" } else { "missing" };
                format!("{library} {status}")
            })
            .collect();
        lines.push(format!("  CUDA libs:  {}", found.join("; ")));
    }
    lines.push(format!("  needed by:  {GPU_NEEDED_BY}"));
    lines
}

/// The report, one line per entry: each program, whether a tool would find it (and
/// where), the commands that need it, and how to install it.
pub fn report() -> Vec<String> {
    let mut lines = vec![
        format!(
            "Domainatoredux {LONG_VERSION}, on {} {}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ),
        String::new(),
        "External programs:".to_string(),
    ];
    for program in EXTERNAL_PROGRAMS {
        let tried = program.find();
        let status = match program.locate() {
            Some(path) => format!("found ({})", path.display()),
            // An environment variable or bundled path that names no file.
            None if tried.components().count() > 1 => format!(
                "missing: {} does not exist; install: {}",
                tried.display(),
                program.install
            ),
            None => format!(
                "missing; install: {}; or set ${} to its path",
                program.install, program.env_vars[0]
            ),
        };
        lines.push(format!("  {:<9} {status}", program.name));
        lines.push(format!("  {:<9} needed by: {}", "", program.needed_by));
    }
    lines.push(String::new());
    lines.push(
        "The desktop app's External tools panel downloads all of them but cmbuild; see \
         docs/installation.md."
            .to_string(),
    );
    lines.extend(gpu_report(nvidia_gpus(), &backend()));
    lines
}

pub fn run() {
    println!("{}", report().join("\n"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_program_is_reported_with_its_users() {
        let report = report().join("\n");
        for program in EXTERNAL_PROGRAMS {
            assert!(
                report.contains(&format!("  {:<9} ", program.name)),
                "{}",
                program.name
            );
            assert!(report.contains(program.needed_by), "{}", program.name);
        }
        assert!(report.contains("\nGPU:\n"));
    }

    fn quadro() -> Option<NvidiaGpus> {
        Some(NvidiaGpus {
            names: vec!["GPU 0: Quadro RTX 6000".to_string()],
            driver_cuda: Some("12.4".to_string()),
        })
    }

    #[test]
    fn nvidia_smi_output_is_read_as_upstream_reads_it() {
        let gpus = parse_nvidia_smi(
            "GPU 0: Quadro RTX 6000 (UUID: GPU-90df31f2)\nGPU 1: Tesla T4 (UUID: GPU-1)\n",
            "| NVIDIA-SMI 550.54.14   Driver Version: 550.54.14   CUDA Version: 12.4     |\n",
        );
        assert_eq!(gpus.names, ["GPU 0: Quadro RTX 6000", "GPU 1: Tesla T4"]);
        assert_eq!(gpus.driver_cuda.as_deref(), Some("12.4"));
        let none = parse_nvidia_smi("No devices were found\n", "");
        assert!(none.names.is_empty() && none.driver_cuda.is_none());
    }

    fn library(name: &'static str, file: &'static str, present: bool) -> CudaLibrary {
        CudaLibrary {
            name,
            file,
            present,
        }
    }

    /// A CUDA build on a machine with every library.
    fn cuda(devices: Result<usize, String>) -> Backend {
        Backend::Cuda {
            libraries: vec![
                library("NVIDIA driver", "libcuda.so.1", true),
                library("cuBLAS", "libcublas.so.12", true),
            ],
            devices,
        }
    }

    #[test]
    fn the_gpu_section_says_what_was_found_and_what_this_build_can_use() {
        let libraries = vec![
            library("NVIDIA driver", "libcuda.so.1", true),
            library("cuBLAS", "libcublas.so.12", false),
        ];
        let missing = missing_libraries(&libraries).unwrap();
        let cases: [(Option<NvidiaGpus>, Backend, &[&str]); 7] = [
            (
                None,
                Backend::Cpu,
                &[
                    "NVIDIA GPU: none found (nvidia-smi is not on PATH)",
                    "this build: CPU only",
                ],
            ),
            (
                quadro(),
                cuda(Ok(1)),
                &[
                    "NVIDIA GPU: GPU 0: Quadro RTX 6000; the driver supports CUDA 12.4 and older",
                    "this build: a CUDA build, can use 1 GPU\n",
                    "CUDA libs:  NVIDIA driver (libcuda.so.1) found; cuBLAS (libcublas.so.12) \
                     found\n",
                ],
            ),
            (
                quadro(),
                Backend::Cuda {
                    libraries,
                    devices: Err(missing),
                },
                &[
                    "this build: a CUDA build, cannot use a GPU because a CUDA library is \
                     missing: cuBLAS (libcublas.so.12), so it runs on the CPU; \
                     install the NVIDIA driver and the CUDA Toolkit 12.x",
                    "CUDA libs:  NVIDIA driver (libcuda.so.1) found; cuBLAS (libcublas.so.12) \
                     missing\n",
                ],
            ),
            (
                quadro(),
                cuda(Ok(2)),
                &["this build: a CUDA build, can use 2 GPUs"],
            ),
            (
                quadro(),
                cuda(Err("DriverError(CUDA_ERROR_UNSUPPORTED_PTX_VERSION)".into())),
                &[
                    "cannot use a GPU (DriverError(CUDA_ERROR_UNSUPPORTED_PTX_VERSION)), so it \
                   runs on the CPU; the release CUDA builds need an NVIDIA driver for CUDA \
                   12.4 or newer",
                ],
            ),
            (
                Some(NvidiaGpus {
                    names: vec![],
                    driver_cuda: None,
                }),
                Backend::Cpu,
                &["NVIDIA GPU: none found (nvidia-smi lists no GPUs)"],
            ),
            (
                None,
                Backend::Metal(Ok(())),
                &["this build: a Metal build, can use the Apple GPU"],
            ),
        ];
        for (gpus, backend, expected) in cases {
            let report = gpu_report(gpus, &backend).join("\n") + "\n";
            for text in expected {
                assert!(report.contains(text), "{text:?} not in\n{report}");
            }
            assert!(report.contains(&format!("needed by:  {GPU_NEEDED_BY}")));
        }
    }
}
