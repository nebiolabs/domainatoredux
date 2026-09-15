//! Flexible registry + downloader for external executable dependencies.
//!
//! Each [`ToolSpec`] describes a binary the app can fetch on demand for the
//! current OS/arch, extract, store under the app data dir (`<app_data>/bin/`),
//! and wire up by setting the env var the core locator (`tools::external::
//! find_binary`) checks. Adding a future dependency (mmseqs2, foldseek, prodigal,
//! …) is just another entry in [`specs`].

use std::io::Read;
use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};

/// How a downloaded asset is packaged.
pub enum Archive {
    /// The URL is the bare executable. (Reserved for future tools.)
    #[allow(dead_code)]
    Raw,
    /// A `.tar.gz`; extract `member` (matched by file name).
    TarGz,
    /// A `.zip`; extract `member` (matched by file name).
    Zip,
}

/// A per-platform download.
pub struct Asset {
    pub url: &'static str,
    pub archive: Archive,
    /// File name of the binary inside the archive (ignored for `Raw`).
    pub member: &'static str,
}

/// One external tool the app can manage.
pub struct ToolSpec {
    pub id: &'static str,
    /// Final on-disk file name (e.g. `diamond` / `diamond.exe`).
    pub bin_name: String,
    /// Env var the core locator checks (so a downloaded binary is found).
    pub env_var: &'static str,
    pub version: &'static str,
    pub description: &'static str,
    /// The download for the current platform, or `None` if unavailable here.
    pub asset: Option<Asset>,
}

pub fn platform_key() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "linux-x64",
        ("macos", "aarch64") => "macos-arm64",
        ("macos", "x86_64") => "macos-x64",
        ("windows", "x86_64") => "windows-x64",
        _ => "unsupported",
    }
}

/// The registry of managed tools, with assets resolved for the current platform.
pub fn specs() -> Vec<ToolSpec> {
    let exe = |n: &str| if cfg!(windows) { format!("{n}.exe") } else { n.to_string() };

    // Prebuilt binaries for Linux, macOS, and Windows. The version must be at least
    // `MIN_DIAMOND_VERSION` -- `deduplicate --algorithm diamond` fails outright on an
    // older one, and telling a user to "upgrade diamond" is absurd when the app is
    // what installed it. A test below enforces that.
    let diamond_asset = match platform_key() {
        "linux-x64" => Some(Asset {
            url: "https://github.com/bbuchfink/diamond/releases/download/v2.2.5/diamond-linux64.tar.gz",
            archive: Archive::TarGz,
            member: "diamond",
        }),
        "macos-arm64" | "macos-x64" => Some(Asset {
            url: "https://github.com/bbuchfink/diamond/releases/download/v2.2.5/diamond-macos.tar.gz",
            archive: Archive::TarGz,
            member: "diamond",
        }),
        "windows-x64" => Some(Asset {
            url: "https://github.com/bbuchfink/diamond/releases/download/v2.2.5/diamond-windows.zip",
            archive: Archive::Zip,
            member: "diamond.exe",
        }),
        _ => None,
    };

    vec![ToolSpec {
        id: "diamond",
        bin_name: exe("diamond"),
        env_var: "DOMAINATOR_DIAMOND",
        version: "2.2.5",
        description: "DIAMOND — fast protein aligner, for seq-dist diamond_* algorithms.",
        asset: diamond_asset,
    }]
}

/// `<app_data>/bin` (created if missing).
fn managed_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir: {e}"))?
        .join("bin");
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    Ok(dir)
}

/// Path to `spec`'s binary if it's already installed under the managed dir.
pub fn installed_path(app: &AppHandle, spec: &ToolSpec) -> Option<PathBuf> {
    let p = managed_dir(app).ok()?.join(&spec.bin_name);
    p.is_file().then_some(p)
}

/// Point each installed tool's env var at its managed binary so the core locator
/// finds it. Called at startup and before listing.
pub fn ensure_env(app: &AppHandle) {
    for spec in specs() {
        if let Some(p) = installed_path(app, &spec) {
            // SAFETY: single-threaded startup / command context; edition 2021.
            std::env::set_var(spec.env_var, &p);
        }
    }
}

/// Download, extract, install, and wire up `spec`. Returns the binary path.
pub fn download(app: &AppHandle, spec: &ToolSpec) -> Result<PathBuf, String> {
    let asset = spec
        .asset
        .as_ref()
        .ok_or_else(|| format!("{} has no prebuilt binary for {}", spec.id, platform_key()))?;
    let dest = managed_dir(app)?.join(&spec.bin_name);

    let resp = ureq::get(asset.url).call().map_err(|e| format!("downloading {}: {e}", asset.url))?;
    let mut reader = resp.into_reader();

    match asset.archive {
        Archive::Raw => {
            let mut out = std::fs::File::create(&dest).map_err(|e| format!("writing: {e}"))?;
            std::io::copy(&mut reader, &mut out).map_err(|e| format!("writing: {e}"))?;
        }
        Archive::TarGz => extract_targz_member(reader, asset.member, &dest)?,
        Archive::Zip => {
            // ZipArchive needs Read + Seek, so buffer the (small) archive first.
            let mut buf = Vec::new();
            reader.read_to_end(&mut buf).map_err(|e| format!("downloading: {e}"))?;
            extract_zip_member(&buf, asset.member, &dest)?;
        }
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&dest).map_err(|e| e.to_string())?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&dest, perms).map_err(|e| format!("chmod: {e}"))?;
    }

    // SAFETY: see ensure_env.
    std::env::set_var(spec.env_var, &dest);
    Ok(dest)
}

/// Extract the single archive member whose file name is `member` to `dest`.
fn extract_targz_member(reader: impl Read, member: &str, dest: &Path) -> Result<(), String> {
    let gz = flate2::read::GzDecoder::new(reader);
    let mut ar = tar::Archive::new(gz);
    for entry in ar.entries().map_err(|e| format!("reading archive: {e}"))? {
        let mut entry = entry.map_err(|e| format!("reading archive: {e}"))?;
        let path = entry.path().map_err(|e| format!("archive entry: {e}"))?;
        if path.file_name().and_then(|n| n.to_str()) == Some(member) {
            let mut out = std::fs::File::create(dest).map_err(|e| format!("writing {}: {e}", dest.display()))?;
            std::io::copy(&mut entry, &mut out).map_err(|e| format!("extracting: {e}"))?;
            return Ok(());
        }
    }
    Err(format!("'{member}' not found in archive"))
}

/// Extract the archive member whose file name is `member` from a `.zip`.
fn extract_zip_member(bytes: &[u8], member: &str, dest: &Path) -> Result<(), String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| format!("opening zip: {e}"))?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| format!("zip entry: {e}"))?;
        let name = entry.name().to_string();
        if Path::new(&name).file_name().and_then(|n| n.to_str()) == Some(member) {
            let mut out = std::fs::File::create(dest).map_err(|e| format!("writing {}: {e}", dest.display()))?;
            std::io::copy(&mut entry, &mut out).map_err(|e| format!("extracting: {e}"))?;
            return Ok(());
        }
    }
    Err(format!("'{member}' not found in zip"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(v: &str) -> Vec<u32> {
        v.split('.').map(|p| p.parse().unwrap_or(0)).collect()
    }

    /// The whole point of the External tools panel is that a user who has no
    /// diamond can get a working one. Shipping a version older than the core
    /// requires makes it install a binary that fails on the first single-cluster
    /// `deduplicate`, with an error telling them to upgrade the thing they just
    /// installed.
    #[test]
    fn the_managed_diamond_is_new_enough_for_deduplicate() {
        let min = domainator_core::tools::deduplicate_genbank::MIN_DIAMOND_VERSION;
        let spec = specs().into_iter().find(|s| s.id == "diamond").expect("diamond spec");
        assert!(
            parts(spec.version) >= parts(min),
            "the managed diamond is {} but deduplicate needs >= {min}",
            spec.version
        );
    }

    /// The download URL has to name the version the panel advertises, or the app
    /// installs one binary while reporting another.
    #[test]
    fn the_asset_url_matches_the_advertised_version() {
        for spec in specs() {
            let Some(asset) = &spec.asset else { continue };
            assert!(
                asset.url.contains(&format!("v{}", spec.version)),
                "{}: url {:?} does not name version {}",
                spec.id,
                asset.url,
                spec.version
            );
            assert!(asset.url.starts_with("https://"), "{}: downloads must be https", spec.id);
            match asset.archive {
                Archive::TarGz => assert!(asset.url.ends_with(".tar.gz"), "{}", spec.id),
                Archive::Zip => assert!(asset.url.ends_with(".zip"), "{}", spec.id),
                Archive::Raw => {}
            }
        }
    }

    /// `bin_name` is the on-disk file name and `member` is the path inside the
    /// archive; on Windows both need the `.exe` the extractor looks for.
    #[test]
    fn binary_names_carry_the_platform_suffix() {
        for spec in specs() {
            assert_eq!(spec.bin_name.ends_with(".exe"), cfg!(windows), "{}", spec.id);
            assert!(!spec.env_var.is_empty(), "{} has no locator env var", spec.id);
            if let Some(asset) = &spec.asset {
                if !matches!(asset.archive, Archive::Raw) {
                    assert_eq!(asset.member.ends_with(".exe"), cfg!(windows), "{}", spec.id);
                }
            }
        }
    }

    #[test]
    fn an_unsupported_platform_offers_no_download_rather_than_a_wrong_one() {
        // Every key `platform_key` can return is either one the specs handle or
        // the explicit "unsupported" sentinel -- never a silent fallthrough to
        // someone else's binary.
        let key = platform_key();
        assert!(
            ["linux-x64", "macos-arm64", "macos-x64", "windows-x64", "unsupported"].contains(&key),
            "unexpected platform key {key:?}"
        );
        if key == "unsupported" {
            assert!(specs().iter().all(|s| s.asset.is_none()));
        }
    }
}
