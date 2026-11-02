//! Where `find_features --tmbed` finds TMbed's ProtT5 encoder, and how it gets it.
//!
//! The directory is upstream's: `--tmbed_model_dir`, else `<--model_dir>/tmbed`, laid
//! out as TMbed's `--model-dir` is (what `save_pretrained` writes: `config.json`, the
//! tokenizer files and the weights), so one directory serves both implementations. With
//! neither option, TMbed keeps the model inside its Python package; the port keeps it in
//! the user's cache directory instead ([`default_model_dir`]). A directory without the
//! weights is filled from Hugging Face at a pinned revision, the weights checked
//! against their SHA-256.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use domainator_plm::tmbed::{model_weights, PROT_T5_MODEL};

/// The Hugging Face revision of [`PROT_T5_MODEL`] downloaded (its only one, from
/// 2023-01-31), so a later push cannot change the predictions under us.
pub const PROT_T5_REVISION: &str = "94a6abc029ae13029317b140b7424e012bf8dfbf";

/// The files downloaded, the weights last: TMbed's `save_pretrained` layout, so
/// upstream can read the directory too. `(name, size, sha256)` for the weights.
#[cfg(feature = "network")]
const SMALL_FILES: [&str; 4] = [
    "config.json",
    "special_tokens_map.json",
    "spiece.model",
    "tokenizer_config.json",
];
const WEIGHTS: (&str, u64, &str) = (
    "pytorch_model.bin",
    2_416_373_051,
    "7f51ba885541c7dc569d46b796af57cc7a2ba7945107dced4f19d1b5ec091157",
);

/// What a download fetches, in bytes: the weights (the other files are a few KB).
pub const DOWNLOAD_BYTES: u64 = WEIGHTS.1;

/// The model directory when neither `--model_dir` nor `--tmbed_model_dir` is given:
/// `domainatoredux/tmbed` in the user's cache directory (`$XDG_CACHE_HOME` or
/// `~/.cache` on Linux, `~/Library/Caches` on macOS, `%LOCALAPPDATA%` on Windows).
pub fn default_model_dir() -> PathBuf {
    let env = |key| {
        std::env::var_os(key)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let base = if cfg!(windows) {
        env("LOCALAPPDATA")
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library").join("Caches"))
    } else {
        env("XDG_CACHE_HOME").or_else(|| env("HOME").map(|h| h.join(".cache")))
    };
    base.unwrap_or_else(std::env::temp_dir)
        .join("domainatoredux")
        .join("tmbed")
}

/// Upstream's choice of directory: `tmbed_model_dir`, else `model_dir/tmbed` (created;
/// `model_dir` itself must exist), else [`default_model_dir`].
pub fn resolve_model_dir(
    model_dir: Option<&str>,
    tmbed_model_dir: Option<&str>,
) -> std::io::Result<PathBuf> {
    if let Some(dir) = model_dir {
        if !Path::new(dir).is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("Model directory {dir} does not exist or is not a directory."),
            ));
        }
    }
    match (tmbed_model_dir, model_dir) {
        (Some(dir), _) if !dir.is_empty() => Ok(PathBuf::from(dir)),
        (_, Some(dir)) => {
            let dir = Path::new(dir).join("tmbed");
            std::fs::create_dir_all(&dir)?;
            Ok(dir)
        }
        _ => Ok(default_model_dir()),
    }
}

/// Make sure `dir` holds ProtT5's weights, downloading the model there if it does not.
pub fn ensure_model(dir: &Path) -> std::io::Result<()> {
    if model_weights(dir).is_some() {
        return Ok(());
    }
    crate::log::info(format_args!(
        "Downloading TMbed's ProtT5 encoder ({PROT_T5_MODEL}, 2.4 GB) to {}",
        dir.display()
    ));
    download(dir, &AtomicBool::new(false), &|_, _| {})
}

/// Download the model into `dir`, each file landing beside its name and renamed
/// into place once complete. `progress(done, total)` follows the weights'
/// bytes; setting `cancel` stops the transfer at its next read, with an error.
#[cfg(feature = "network")]
pub fn download(
    dir: &Path,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64, u64),
) -> std::io::Result<()> {
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};
    use std::sync::atomic::Ordering;

    std::fs::create_dir_all(dir)?;
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(60))
        .timeout_read(std::time::Duration::from_secs(300))
        .build();
    let fetch = |name: &str, expected: Option<(u64, &str)>| -> std::io::Result<()> {
        let url =
            format!("https://huggingface.co/{PROT_T5_MODEL}/resolve/{PROT_T5_REVISION}/{name}");
        let dest = dir.join(name);
        let part = dir.join(format!("{name}.part"));
        let result = (|| {
            let mut reader = agent
                .get(&url)
                .call()
                .map_err(|e| std::io::Error::other(format!("downloading {url}: {e}")))?
                .into_reader();
            let mut out = std::fs::File::create(&part)?;
            let mut hasher = Sha256::new();
            let mut size = 0u64;
            let mut buf = vec![0u8; 1 << 20];
            loop {
                if cancel.load(Ordering::Relaxed) {
                    return Err(std::io::Error::other("canceled"));
                }
                let n = reader
                    .read(&mut buf)
                    .map_err(|e| std::io::Error::other(format!("downloading {url}: {e}")))?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
                out.write_all(&buf[..n])?;
                size += n as u64;
                if expected.is_some() {
                    progress(size, DOWNLOAD_BYTES);
                }
            }
            out.sync_all()?;
            if let Some((want_size, want_sha)) = expected {
                let got: String = hasher
                    .finalize()
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect();
                if size != want_size || got != want_sha {
                    return Err(std::io::Error::other(format!(
                        "downloaded {url}, but it is {size} bytes with SHA-256 {got}, not \
                         {want_size} bytes with {want_sha}"
                    )));
                }
            }
            std::fs::rename(&part, &dest)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&part);
        }
        result
    };
    for name in SMALL_FILES {
        fetch(name, None)?;
    }
    let (name, size, sha) = WEIGHTS;
    fetch(name, Some((size, sha)))
}

#[cfg(not(feature = "network"))]
pub fn download(
    dir: &Path,
    _cancel: &AtomicBool,
    _progress: &dyn Fn(u64, u64),
) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        format!(
            "no ProtT5 weights in {}: download {PROT_T5_MODEL} there (TMbed's `tmbed \
             download --model-dir {}` does)",
            dir.display(),
            dir.display()
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tmbed_model_dir_wins_then_model_dir_tmbed() {
        let scratch = std::env::temp_dir().join(format!("tmbed_model_{}", std::process::id()));
        std::fs::create_dir_all(&scratch).unwrap();
        let base = scratch.to_str().unwrap();
        assert_eq!(
            resolve_model_dir(Some(base), Some("/x/t5")).unwrap(),
            PathBuf::from("/x/t5")
        );
        let under = resolve_model_dir(Some(base), None).unwrap();
        assert_eq!(under, scratch.join("tmbed"));
        assert!(under.is_dir(), "created, as upstream creates it");
        assert_eq!(resolve_model_dir(None, None).unwrap(), default_model_dir());
        let missing = scratch.join("missing");
        let err = resolve_model_dir(missing.to_str(), Some("/x/t5")).unwrap_err();
        assert!(err.to_string().contains("does not exist"), "{err}");
        std::fs::remove_dir_all(&scratch).unwrap();
    }

    #[test]
    fn the_default_model_dir_ends_in_domainatoredux_tmbed() {
        assert!(default_model_dir().ends_with(Path::new("domainatoredux").join("tmbed")));
    }
}
