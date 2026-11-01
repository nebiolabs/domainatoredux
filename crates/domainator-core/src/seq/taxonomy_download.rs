//! Fetching NCBI's taxdump: the download half of upstream `NCBITaxonomy.__init__`
//! (`taxonomy/ncbi.py`), behind `--taxonomy_update` and every tool's
//! `--ncbi_taxonomy_path`.
//!
//! [`open_taxonomy`] makes sure the path holds a database, then loads it. A path with
//! neither `taxdump.tar.gz` nor a complete set of extracted `.dmp` files gets the
//! tarball downloaded; with `update`, the local tarball's MD5 is compared with NCBI's
//! `.md5` file and a different one is replaced (extracted files alone cannot be
//! compared, so they get a tarball, which [`NCBITaxonomy::load`] then reads first).
//! A failed comparison keeps the local copy with a warning, as upstream does.
//!
//! Runs that share a database coordinate through an advisory lock on
//! `taxdump.tar.gz.lock` beside it, upstream's name: shared while checking and
//! reading, exclusive while downloading. A download lands in `taxdump.tar.gz.part`,
//! is checked against NCBI's MD5 when that could be fetched (upstream does not check
//! it), and is renamed into place only then, so a reader never sees half a file.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use md5::{Digest, Md5};

use super::taxonomy::{has_extracted_dmps, NCBITaxonomy};

/// NCBI's taxdump and its MD5, upstream's URLs.
pub const TAXDUMP_URL: &str = "https://ftp.ncbi.nlm.nih.gov/pub/taxonomy/taxdump.tar.gz";
pub const TAXDUMP_MD5_URL: &str = "https://ftp.ncbi.nlm.nih.gov/pub/taxonomy/taxdump.tar.gz.md5";

/// Upstream's `requests` timeout, used for connecting and for each read.
const TIMEOUT: Duration = Duration::from_secs(60);

/// Where the taxdump and its MD5 are fetched from: NCBI, or a test server.
struct Remote<'a> {
    archive: &'a str,
    md5: &'a str,
}

const NCBI: Remote<'static> = Remote {
    archive: TAXDUMP_URL,
    md5: TAXDUMP_MD5_URL,
};

/// Load the taxonomy at `path`, downloading NCBI's taxdump first when `path` has no
/// database or, with `update`, an outdated one (upstream `NCBITaxonomy(path,
/// overwrite=update)`).
///
/// `path` is a directory, created if missing, as upstream takes it, or the path of
/// an existing taxdump archive itself (how the desktop app registers its download),
/// which `update` refreshes in place.
pub fn open_taxonomy(path: &Path, update: bool) -> std::io::Result<NCBITaxonomy> {
    open_with(path, update, &NCBI)
}

fn open_with(path: &Path, update: bool, remote: &Remote<'_>) -> std::io::Result<NCBITaxonomy> {
    let (archive, dmp_dir) = if path.is_file() {
        (path.to_path_buf(), None)
    } else {
        std::fs::create_dir_all(path)?;
        (path.join("taxdump.tar.gz"), Some(path))
    };
    let lock = lock_path(&archive);

    // Check under a shared lock, so concurrent readers are not serialized, and take
    // the exclusive one only to download.
    let plan = {
        let _shared = Lock::shared(&lock);
        plan(&archive, dmp_dir, update, remote)?
    };
    if let Plan::Download(expected) = plan {
        let _exclusive = Lock::exclusive(&lock);
        // Another run may have fetched it while this one waited for the lock.
        let done = match &expected {
            Some(md5) => archive.exists() && md5_file(&archive)? == *md5,
            None => archive.exists(),
        };
        if !done {
            download(remote.archive, &archive, expected.as_deref())?;
        }
    }

    crate::log::info(format_args!("Loading NCBI taxonomy database"));
    let _shared = Lock::shared(&lock);
    let taxonomy = NCBITaxonomy::load(path)?;
    crate::log::info(format_args!("Loaded NCBI taxonomy database"));
    Ok(taxonomy)
}

enum Plan {
    Keep,
    /// Download, checking against this MD5 when it is known.
    Download(Option<String>),
}

fn plan(
    archive: &Path,
    dmp_dir: Option<&Path>,
    update: bool,
    remote: &Remote<'_>,
) -> std::io::Result<Plan> {
    if !archive.exists() {
        if !update && dmp_dir.is_some_and(has_extracted_dmps) {
            return Ok(Plan::Keep);
        }
        let expected = remote_md5(remote.md5)
            .inspect_err(|e| crate::log::debug(format_args!("{e}")))
            .ok();
        return Ok(Plan::Download(expected));
    }
    if !update {
        return Ok(Plan::Keep);
    }
    match remote_md5(remote.md5) {
        Ok(expected) => {
            let local = md5_file(archive)?;
            if local == expected {
                crate::log::info(format_args!("Local taxdump md5 matches server"));
                Ok(Plan::Keep)
            } else {
                crate::log::info(format_args!(
                    "Local taxdump md5 does not match server {local}, {expected}"
                ));
                Ok(Plan::Download(Some(expected)))
            }
        }
        Err(e) => {
            crate::log::warn(format_args!(
                "Could not check md5 of {}: {e}",
                file_name(archive)
            ));
            Ok(Plan::Keep)
        }
    }
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(TIMEOUT)
        .timeout_read(TIMEOUT)
        .build()
}

/// The hash in NCBI's `.md5` file, `<md5sum>  <filename>`.
fn remote_md5(url: &str) -> std::io::Result<String> {
    let text = agent()
        .get(url)
        .call()
        .map_err(|e| std::io::Error::other(format!("fetching {url}: {e}")))?
        .into_string()
        .map_err(|e| std::io::Error::other(format!("reading {url}: {e}")))?;
    text.split_whitespace()
        .next()
        .filter(|hash| hash.len() == 32 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| std::io::Error::other(format!("{url} does not hold an MD5")))
}

/// Stream `url` into `dest` through `dest.part`, renamed into place once it is all
/// there and, when `expected` is given, has that MD5.
fn download(url: &str, dest: &Path, expected: Option<&str>) -> std::io::Result<()> {
    crate::log::info(format_args!("Downloading {}", file_name(dest)));
    let mut part = dest.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);
    let result = (|| {
        let response = agent()
            .get(url)
            .call()
            .map_err(|e| std::io::Error::other(format!("downloading {url}: {e}")))?;
        let mut reader = response.into_reader();
        let mut out = File::create(&part)?;
        let mut hasher = Md5::new();
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let n = reader
                .read(&mut buf)
                .map_err(|e| std::io::Error::other(format!("downloading {url}: {e}")))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            out.write_all(&buf[..n])?;
        }
        out.sync_all()?;
        let got = hex(&hasher.finalize());
        if let Some(expected) = expected {
            if got != expected {
                return Err(std::io::Error::other(format!(
                    "downloaded {url}, but its MD5 is {got}, not the {expected} NCBI lists; \
                     the copy at {} was left as it was",
                    dest.display()
                )));
            }
        }
        std::fs::rename(&part, dest)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result
}

fn md5_file(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Md5::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            return Ok(hex(&hasher.finalize()));
        }
        hasher.update(&buf[..n]);
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// `taxdump.tar.gz.lock` for `taxdump.tar.gz`.
fn lock_path(archive: &Path) -> PathBuf {
    let mut lock = archive.as_os_str().to_owned();
    lock.push(".lock");
    PathBuf::from(lock)
}

/// An advisory lock on a lock file, released on drop. A lock file that cannot be
/// created (a database in a read-only directory) leaves the run unlocked rather than
/// failing it: there is nothing such a run could overwrite.
struct Lock {
    _file: Option<File>,
}

impl Lock {
    fn shared(path: &Path) -> Lock {
        Lock::acquire(path, false)
    }

    fn exclusive(path: &Path) -> Lock {
        Lock::acquire(path, true)
    }

    fn acquire(path: &Path, exclusive: bool) -> Lock {
        let file = match OpenOptions::new().create(true).append(true).open(path) {
            Ok(file) => file,
            Err(e) => {
                crate::log::debug(format_args!("not locking {}: {e}", path.display()));
                return Lock { _file: None };
            }
        };
        let attempt = if exclusive {
            file.try_lock()
        } else {
            file.try_lock_shared()
        };
        let locked = match attempt {
            Ok(()) => Ok(()),
            Err(TryLockError::WouldBlock) => {
                let kind = if exclusive { "exclusive" } else { "shared" };
                crate::log::info(format_args!("Waiting for {kind} lock: {}", path.display()));
                if exclusive {
                    file.lock()
                } else {
                    file.lock_shared()
                }
            }
            Err(TryLockError::Error(e)) => Err(e),
        };
        match locked {
            Ok(()) => Lock { _file: Some(file) },
            Err(e) => {
                crate::log::debug(format_args!("not locking {}: {e}", path.display()));
                Lock { _file: None }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seq::taxonomy::fixtures::{dmp_texts, taxdump};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    /// A one-file-at-a-time HTTP server for `/taxdump.tar.gz` and its `.md5`,
    /// recording the paths requested.
    struct FakeNcbi {
        base: String,
        requests: Arc<Mutex<Vec<String>>>,
    }

    impl FakeNcbi {
        /// Serve `archive`, listing `md5` as its hash (`None`: the `.md5` is a 404).
        fn start(archive: Vec<u8>, md5: Option<String>) -> FakeNcbi {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let requests = Arc::new(Mutex::new(Vec::new()));
            let seen = Arc::clone(&requests);
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { return };
                    let mut head = Vec::new();
                    let mut byte = [0u8; 1];
                    while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap_or(0) == 1 {
                        head.push(byte[0]);
                    }
                    let head = String::from_utf8_lossy(&head);
                    let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
                    seen.lock().unwrap().push(path.clone());
                    let body = match path.as_str() {
                        "/taxdump.tar.gz" => Some(archive.clone()),
                        "/taxdump.tar.gz.md5" => md5
                            .as_ref()
                            .map(|m| format!("{m}  taxdump.tar.gz\n").into_bytes()),
                        _ => None,
                    };
                    let (status, body) = match body {
                        Some(body) => ("200 OK", body),
                        None => ("404 Not Found", Vec::new()),
                    };
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(&body);
                }
            });
            FakeNcbi { base, requests }
        }

        fn remote(&self) -> (String, String) {
            (
                format!("{}/taxdump.tar.gz", self.base),
                format!("{}/taxdump.tar.gz.md5", self.base),
            )
        }

        fn requests(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }
    }

    fn open(path: &Path, update: bool, server: &FakeNcbi) -> std::io::Result<NCBITaxonomy> {
        let (archive, md5) = server.remote();
        open_with(
            path,
            update,
            &Remote {
                archive: &archive,
                md5: &md5,
            },
        )
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("domainator-taxdl-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn md5_of(bytes: &[u8]) -> String {
        hex(&Md5::digest(bytes))
    }

    /// A taxdump whose E. coli is called something else, to tell two copies apart.
    fn other_taxdump() -> Vec<u8> {
        let [nodes, names, merged, delnodes] = dmp_texts();
        let names = names.replace("Escherichia coli", "Bacterium coli");
        crate::seq::taxonomy::fixtures::targz(&[
            ("nodes.dmp", nodes),
            ("names.dmp", &names),
            ("merged.dmp", merged),
            ("delnodes.dmp", delnodes),
        ])
    }

    #[test]
    fn a_missing_database_is_downloaded_and_checked() {
        let served = taxdump();
        let server = FakeNcbi::start(served.clone(), Some(md5_of(&served)));
        let dir = scratch("missing").join("nested");
        let t = open(&dir, false, &server).unwrap();
        assert_eq!(t.name(562), Some("Escherichia coli"));
        assert_eq!(
            server.requests(),
            ["/taxdump.tar.gz.md5", "/taxdump.tar.gz"]
        );
        assert_eq!(std::fs::read(dir.join("taxdump.tar.gz")).unwrap(), served);
        assert!(!dir.join("taxdump.tar.gz.part").exists());
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn a_present_database_is_not_checked_without_update() {
        let server = FakeNcbi::start(other_taxdump(), None);
        let dir = scratch("present");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("taxdump.tar.gz"), taxdump()).unwrap();
        let t = open(&dir, false, &server).unwrap();
        assert_eq!(t.name(562), Some("Escherichia coli"));
        assert!(server.requests().is_empty());

        // Extracted files alone count as a database too (upstream's fallback).
        std::fs::remove_file(dir.join("taxdump.tar.gz")).unwrap();
        for (name, text) in ["nodes.dmp", "names.dmp", "merged.dmp", "delnodes.dmp"]
            .into_iter()
            .zip(dmp_texts())
        {
            std::fs::write(dir.join(name), text).unwrap();
        }
        open(&dir, false, &server).unwrap();
        assert!(server.requests().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn update_keeps_a_current_copy_and_replaces_an_outdated_one() {
        let served = other_taxdump();
        let server = FakeNcbi::start(served.clone(), Some(md5_of(&served)));
        let dir = scratch("update");
        std::fs::create_dir_all(&dir).unwrap();

        std::fs::write(dir.join("taxdump.tar.gz"), &served).unwrap();
        let t = open(&dir, true, &server).unwrap();
        assert_eq!(t.name(562), Some("Bacterium coli"));
        assert_eq!(server.requests(), ["/taxdump.tar.gz.md5"]);

        std::fs::write(dir.join("taxdump.tar.gz"), taxdump()).unwrap();
        let t = open(&dir, true, &server).unwrap();
        assert_eq!(t.name(562), Some("Bacterium coli"));
        assert_eq!(server.requests().len(), 3);

        // The archive's own path is refreshed in place.
        let archive = dir.join("taxdump.tar.gz");
        std::fs::write(&archive, taxdump()).unwrap();
        let t = open(&archive, true, &server).unwrap();
        assert_eq!(t.name(562), Some("Bacterium coli"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unreachable_server_keeps_a_local_copy_and_fails_without_one() {
        let closed = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", closed.local_addr().unwrap());
        drop(closed);
        let (archive_url, md5_url) = (
            format!("{base}/taxdump.tar.gz"),
            format!("{base}/taxdump.tar.gz.md5"),
        );
        let remote = Remote {
            archive: &archive_url,
            md5: &md5_url,
        };
        let dir = scratch("unreachable");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("taxdump.tar.gz"), taxdump()).unwrap();
        let t = open_with(&dir, true, &remote).unwrap();
        assert_eq!(t.name(562), Some("Escherichia coli"));

        std::fs::remove_file(dir.join("taxdump.tar.gz")).unwrap();
        let e = open_with(&dir, false, &remote).err().unwrap();
        assert!(e.to_string().contains(&archive_url), "{e}");
        assert!(!dir.join("taxdump.tar.gz.part").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_download_that_does_not_match_its_md5_is_discarded() {
        let server = FakeNcbi::start(other_taxdump(), Some(md5_of(b"something else")));
        let dir = scratch("corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("taxdump.tar.gz"), taxdump()).unwrap();
        let e = open(&dir, true, &server).err().unwrap();
        assert!(e.to_string().contains("MD5"), "{e}");
        assert_eq!(
            std::fs::read(dir.join("taxdump.tar.gz")).unwrap(),
            taxdump()
        );
        assert!(!dir.join("taxdump.tar.gz.part").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
