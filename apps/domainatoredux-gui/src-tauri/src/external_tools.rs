//! Flexible registry + downloader for external executable dependencies.
//!
//! Each [`ToolSpec`] describes a binary the app can fetch on demand for the
//! current OS/arch, extract, store under the app data dir (`<app_data>/bin/`),
//! and wire up by setting the env var the core locator (`tools::external::
//! find_binary`) checks. Adding a future dependency (mmseqs2, foldseek, …) is just
//! another entry in [`specs`].

use std::io::Read;
use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};

use domainator_core::tools::deduplicate_genbank::{
    CDHIT_BIN_NAME, CDHIT_ENV_VAR, DIAMOND_BIN_NAME, DIAMOND_ENV_VAR, USEARCH_BIN_NAME,
    USEARCH_ENV_VAR,
};
use domainator_core::tools::gene_call::{ORPHOS_BIN_NAME, ORPHOS_ENV_VAR};

/// How a downloaded asset is packaged.
pub enum Archive {
    /// The URL is the bare executable; `member` is unused (usearch ships this way).
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
    let exe = |n: &str| {
        if cfg!(windows) {
            format!("{n}.exe")
        } else {
            n.to_string()
        }
    };

    // Prebuilt binaries for Linux, macOS, and Windows. The version must be at least
    // `MIN_DIAMOND_VERSION` -- `deduplicate_genbank --algorithm diamond` fails outright on an
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

    // The Rust cd-hit port: one `cdhit` multiplexer whose first argument is the
    // program name, which is what `deduplicate_genbank`'s cd-hit backend drives. GPL, so it
    // is downloaded at runtime and never bundled with the app. No Intel-macOS build
    // is published; the panel renders that as "n/a on macos-x64".
    let cdhit_asset = match platform_key() {
        "linux-x64" => Some(Asset {
            url: "https://github.com/seanrjohnson-neb/cdhit-rust/releases/download/v0.1.0/cdhit-v0.1.0-x86_64-unknown-linux-gnu.tar.gz",
            archive: Archive::TarGz,
            member: "cdhit",
        }),
        "macos-arm64" => Some(Asset {
            url: "https://github.com/seanrjohnson-neb/cdhit-rust/releases/download/v0.1.0/cdhit-v0.1.0-aarch64-apple-darwin.tar.gz",
            archive: Archive::TarGz,
            member: "cdhit",
        }),
        "windows-x64" => Some(Asset {
            url: "https://github.com/seanrjohnson-neb/cdhit-rust/releases/download/v0.1.0/cdhit-v0.1.0-x86_64-pc-windows-msvc.zip",
            archive: Archive::Zip,
            member: "cdhit.exe",
        }),
        _ => None,
    };

    // usearch v12 is the first open-source release, and ships bare per-platform
    // executables rather than archives -- the one `Archive::Raw` user. The asset is
    // named per platform but installs as `usearch`, which is what the core locator
    // looks for.
    let usearch_url = match platform_key() {
        "linux-x64" => Some("https://github.com/rcedgar/usearch12/releases/download/v12.0-beta1/usearch_linux_x86_12.0-beta"),
        "macos-arm64" => Some("https://github.com/rcedgar/usearch12/releases/download/v12.0-beta1/usearch_osx_m_12.0-beta"),
        "macos-x64" => Some("https://github.com/rcedgar/usearch12/releases/download/v12.0-beta1/usearch_osx_x86_12.0-beta"),
        "windows-x64" => Some("https://github.com/rcedgar/usearch12/releases/download/v12.0-beta1/usearch_win_12.0-beta.exe"),
        _ => None,
    };
    let usearch_asset = usearch_url.map(|url| Asset {
        url,
        archive: Archive::Raw,
        member: "",
    });

    // Orphos, the Rust Prodigal: gene calling for the gene_call tool and
    // `--gene_call`. GPL, so downloaded at runtime and never bundled. Each archive
    // holds one binary named for its platform, installed as `orphos`. No Intel-macOS
    // build has been published since v0.1.0, which predates the flags gene calling
    // relies on.
    let orphos_asset = match platform_key() {
        "linux-x64" => Some(Asset {
            url: "https://github.com/FullHuman/orphos/releases/download/v0.4.0/orphos-cli-x86_64-linux.tar.gz",
            archive: Archive::TarGz,
            member: "orphos-cli-x86_64-linux",
        }),
        "macos-arm64" => Some(Asset {
            url: "https://github.com/FullHuman/orphos/releases/download/v0.4.0/orphos-cli-aarch64-darwin.tar.gz",
            archive: Archive::TarGz,
            member: "orphos-cli-aarch64-darwin",
        }),
        "windows-x64" => Some(Asset {
            url: "https://github.com/FullHuman/orphos/releases/download/v0.4.0/orphos-cli-x86_64-windows.zip",
            archive: Archive::Zip,
            member: "orphos-cli-x86_64-windows.exe",
        }),
        _ => None,
    };

    vec![
        ToolSpec {
            id: "diamond",
            bin_name: exe(DIAMOND_BIN_NAME),
            env_var: DIAMOND_ENV_VAR,
            version: "2.2.5",
            description: "DIAMOND — fast protein aligner, for seq_dist diamond_* algorithms.",
            asset: diamond_asset,
        },
        ToolSpec {
            id: "cdhit",
            bin_name: exe(CDHIT_BIN_NAME),
            env_var: CDHIT_ENV_VAR,
            version: "0.1.0",
            description:
                "cd-hit (Rust port) — sequence clustering, for deduplicate_genbank --algorithm cd-hit.",
            asset: cdhit_asset,
        },
        ToolSpec {
            id: "usearch",
            bin_name: exe(USEARCH_BIN_NAME),
            env_var: USEARCH_ENV_VAR,
            version: "12.0-beta1",
            description: "USEARCH v12 — sequence clustering, for deduplicate_genbank --algorithm usearch.",
            asset: usearch_asset,
        },
        ToolSpec {
            id: "orphos",
            bin_name: exe(ORPHOS_BIN_NAME),
            env_var: ORPHOS_ENV_VAR,
            version: "0.4.0",
            description: "Orphos (Rust Prodigal) — gene calling, for gene_call and --gene_call.",
            asset: orphos_asset,
        },
    ]
}

/// `<app_data>/bin` (created if missing).
pub(crate) fn managed_dir(app: &AppHandle) -> Result<PathBuf, String> {
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

    let resp = ureq::get(asset.url)
        .call()
        .map_err(|e| format!("downloading {}: {e}", asset.url))?;
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
            reader
                .read_to_end(&mut buf)
                .map_err(|e| format!("downloading: {e}"))?;
            extract_zip_member(&buf, asset.member, &dest)?;
        }
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&dest)
            .map_err(|e| e.to_string())?
            .permissions();
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
            let mut out = std::fs::File::create(dest)
                .map_err(|e| format!("writing {}: {e}", dest.display()))?;
            std::io::copy(&mut entry, &mut out).map_err(|e| format!("extracting: {e}"))?;
            return Ok(());
        }
    }
    Err(format!("'{member}' not found in archive"))
}

/// Extract the archive member whose file name is `member` from a `.zip`.
fn extract_zip_member(bytes: &[u8], member: &str, dest: &Path) -> Result<(), String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| format!("opening zip: {e}"))?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| format!("zip entry: {e}"))?;
        let name = entry
            .name()
            .map_err(|e| format!("zip entry name: {e}"))?
            .into_owned();
        if Path::new(&name).file_name().and_then(|n| n.to_str()) == Some(member) {
            let mut out = std::fs::File::create(dest)
                .map_err(|e| format!("writing {}: {e}", dest.display()))?;
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
    /// `deduplicate_genbank`, with an error telling them to upgrade the thing they just
    /// installed.
    #[test]
    fn the_managed_diamond_is_new_enough_for_deduplicate() {
        let min = domainator_core::tools::deduplicate_genbank::MIN_DIAMOND_VERSION;
        let spec = specs()
            .into_iter()
            .find(|s| s.id == "diamond")
            .expect("diamond spec");
        assert!(
            parts(spec.version) >= parts(min),
            "the managed diamond is {} but deduplicate_genbank needs >= {min}",
            spec.version
        );
    }

    /// The panel writes each download to `<app_data>/bin/<bin_name>` and exports
    /// `env_var`; the core locates the same binary by its own name and env var. The
    /// two lists are hand-maintained in different crates, so a rename on either side
    /// leaves a download that lands where nothing looks for it -- the tool still
    /// reports "not found" right after the app says it installed it.
    /// Same for Orphos: before 0.4.0 its `-g` was ignored and its flags took values.
    #[test]
    fn the_managed_orphos_is_new_enough_for_gene_calling() {
        let min = domainator_core::tools::gene_call::MIN_ORPHOS_VERSION;
        let spec = specs()
            .into_iter()
            .find(|s| s.id == "orphos")
            .expect("orphos spec");
        assert!(
            parts(spec.version) >= parts(min),
            "the managed orphos is {} but gene calling needs >= {min}",
            spec.version
        );
    }

    #[test]
    fn managed_tools_match_the_core_locator() {
        let exe = |n: &str| {
            if cfg!(windows) {
                format!("{n}.exe")
            } else {
                n.to_string()
            }
        };
        let want = [
            ("diamond", DIAMOND_BIN_NAME, DIAMOND_ENV_VAR),
            ("cdhit", CDHIT_BIN_NAME, CDHIT_ENV_VAR),
            ("usearch", USEARCH_BIN_NAME, USEARCH_ENV_VAR),
            ("orphos", ORPHOS_BIN_NAME, ORPHOS_ENV_VAR),
        ];
        let specs = specs();
        for (id, bin, env) in want {
            let spec = specs
                .iter()
                .find(|s| s.id == id)
                .unwrap_or_else(|| panic!("no {id} spec"));
            assert_eq!(spec.bin_name, exe(bin), "{id} bin_name");
            assert_eq!(spec.env_var, env, "{id} env_var");
        }
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
            assert!(
                asset.url.starts_with("https://"),
                "{}: downloads must be https",
                spec.id
            );
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
            assert_eq!(
                spec.bin_name.ends_with(".exe"),
                cfg!(windows),
                "{}",
                spec.id
            );
            assert!(
                !spec.env_var.is_empty(),
                "{} has no locator env var",
                spec.id
            );
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
            [
                "linux-x64",
                "macos-arm64",
                "macos-x64",
                "windows-x64",
                "unsupported"
            ]
            .contains(&key),
            "unexpected platform key {key:?}"
        );
        if key == "unsupported" {
            assert!(specs().iter().all(|s| s.asset.is_none()));
        }
    }
}
