//! Registry + downloader for managed *reference databases* (UniProt, Pfam).
//!
//! The sibling of `external_tools.rs`, and deliberately shaped like it: a static
//! list of [`DbSpec`]s, a managed directory under the app data dir, and one
//! `download`-shaped entry point. What differs is everything the size forces —
//! these are downloads measured in tens of gigabytes, so every step here is
//! streamed, cancellable, and reports progress, and nothing is ever buffered whole.
//!
//! What "installed" means, per kind:
//!
//! * **FASTA** (UniProtKB, UniRef) — stored as **BGZF** with a `.didx` offset
//!   index, which is what makes it a *formatted* database `domain_search` can seek
//!   into rather than a file it must rescan every run. The upstream files are plain
//!   gzip, which is not seekable, so they are recompressed; the gunzip and the BGZF
//!   compression happen in the same pass as the download, because materializing
//!   TrEMBL's ~250 GB uncompressed intermediate is not an option. An asset that
//!   *is* already BGZF is copied through untouched.
//! * **HMM** (Pfam-A) — stored **uncompressed**. Nothing in the port reads a
//!   compressed profile library: `search/hmm_read.rs` opens a plain file, and so
//!   does the header scan behind the HMM logo viewer and the Databases panel. The
//!   `.gz` is therefore expanded on the way in rather than kept.
//!
//! Versions are read from each source's own small release file rather than
//! inferred from a URL, and recorded in `manifest.json` next to the databases.
//! Nothing here polls: a check is something the user asks for.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use domainator_core::tools::format_db::{format_db, FormatDbOptions};

/// How an installed database is stored, which is decided by what can read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbKind {
    /// Sequence database: BGZF + `.didx`.
    Fasta,
    /// HMM profile library: uncompressed.
    Hmm,
}

impl DbKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            DbKind::Fasta => "fasta",
            DbKind::Hmm => "hmm",
        }
    }
}

/// One downloadable reference database.
pub struct DbSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    /// The data file to download.
    pub url: &'static str,
    /// A small text file naming the current release.
    pub version_url: &'static str,
    /// Whether `version_url` is gzip-compressed (Pfam's is).
    pub version_gzipped: bool,
    /// Pulls the release string out of `version_url`'s contents.
    pub parse_version: fn(&str) -> Option<String>,
    pub kind: DbKind,
    /// Final file name under the managed directory.
    pub file_name: &'static str,
    /// Indicative sizes for the pre-download warning and the free-space preflight,
    /// in bytes. These are what the panel can show *before* any network call; the
    /// real download size comes from the response's `Content-Length` once a download
    /// starts. They drift by a few percent per release, which is well inside what
    /// "≈40 GB" is telling the user.
    ///
    /// `disk_bytes` is the **installed** footprint, `.didx` included — the index is
    /// 16 bytes per record, which is about a tenth of a BGZF FASTA again and not
    /// something a space check can round away. Round these *up*: being wrong in this
    /// direction costs a needless warning, and in the other it fills someone's disk.
    pub download_bytes: u64,
    pub disk_bytes: u64,
}

impl DbSpec {
    /// Whether the panel must get an explicit confirmation before starting.
    /// Swiss-Prot (~94 MB) is a click; everything else is an evening.
    ///
    /// Both sizes count, because they are large for different reasons: TrEMBL is a
    /// 41 GB *transfer*, while Pfam-A is a 420 MB transfer that lands as 1.7 GB
    /// once expanded to the uncompressed form the HMM reader needs. Gating on the
    /// download alone would start that one silently.
    pub fn needs_confirmation(&self) -> bool {
        self.download_bytes > LARGE_BYTES || self.disk_bytes > LARGE_BYTES
    }
}

/// Above this, a download is not something to start by accident.
const LARGE_BYTES: u64 = 1 << 30; // 1 GiB

/// UniProt's `reldate.txt` and `relnotes.txt` both name the release as the token
/// after the word "Release" (`UniProt Knowledgebase Release 2026_03 consists of:`),
/// so one parser serves the knowledgebase and UniRef alike.
fn uniprot_release(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let after = line.split_once("Release")?.1;
        let token = after.split_whitespace().next()?;
        (!token.is_empty()).then(|| token.trim_end_matches(',').to_string())
    })
}

/// Pfam's `Pfam.version` is a `key : value` block whose first row is the release
/// (`Pfam release       : 38.2`).
fn pfam_release(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim().eq_ignore_ascii_case("Pfam release").then(|| value.trim().to_string())
    })
}

/// A file in UniProt's `current_release` tree. `current_release` is a live symlink
/// on the server, so "update to the newest version" is the same URL fetched again —
/// which is why the release has to be read from `reldate.txt`/`relnotes.txt` rather
/// than inferred from the path.
macro_rules! uniprot {
    ($path:literal) => {
        concat!("https://ftp.uniprot.org/pub/databases/uniprot/current_release/", $path)
    };
}

/// UniRef sits outside the knowledgebase directory and has no `reldate.txt` of its
/// own; it is rebuilt per UniProt release, which `relnotes.txt` names.
const UNIPROT_KB_RELDATE: &str = uniprot!("knowledgebase/complete/reldate.txt");
const UNIPROT_RELNOTES: &str = uniprot!("relnotes.txt");

/// Every reference database the app can fetch.
pub fn specs() -> Vec<DbSpec> {
    vec![
        DbSpec {
            id: "swissprot",
            name: "UniProtKB/Swiss-Prot",
            description: "Manually reviewed, curated protein sequences. The small, high-quality UniProt set.",
            url: uniprot!("knowledgebase/complete/uniprot_sprot.fasta.gz"),
            version_url: UNIPROT_KB_RELDATE,
            version_gzipped: false,
            parse_version: uniprot_release,
            kind: DbKind::Fasta,
            file_name: "uniprot_sprot.fasta.bgz",
            download_bytes: 94_000_000,
            disk_bytes: 110_000_000,
        },
        DbSpec {
            id: "uniref50",
            name: "UniRef50",
            description: "UniProt clustered at 50% identity — whole-of-UniProt coverage at a workable size.",
            url: uniprot!("uniref/uniref50/uniref50.fasta.gz"),
            version_url: UNIPROT_RELNOTES,
            version_gzipped: false,
            parse_version: uniprot_release,
            kind: DbKind::Fasta,
            file_name: "uniref50.fasta.bgz",
            download_bytes: 8_800_000_000,
            disk_bytes: 10_000_000_000,
        },
        DbSpec {
            id: "uniref90",
            name: "UniRef90",
            description: "UniProt clustered at 90% identity — near-complete coverage, far smaller than TrEMBL.",
            url: uniprot!("uniref/uniref90/uniref90.fasta.gz"),
            version_url: UNIPROT_RELNOTES,
            version_gzipped: false,
            parse_version: uniprot_release,
            kind: DbKind::Fasta,
            file_name: "uniref90.fasta.bgz",
            download_bytes: 32_000_000_000,
            disk_bytes: 36_000_000_000,
        },
        DbSpec {
            id: "trembl",
            name: "UniProtKB/TrEMBL",
            description: "The full unreviewed UniProt knowledgebase. Complete, and very large — expect hours.",
            url: uniprot!("knowledgebase/complete/uniprot_trembl.fasta.gz"),
            version_url: UNIPROT_KB_RELDATE,
            version_gzipped: false,
            parse_version: uniprot_release,
            kind: DbKind::Fasta,
            file_name: "uniprot_trembl.fasta.bgz",
            download_bytes: 41_000_000_000,
            disk_bytes: 46_000_000_000,
        },
        DbSpec {
            id: "pfam",
            name: "Pfam-A",
            description: "The Pfam-A profile HMM library, for domainate / hmmer-search. Stored uncompressed.",
            url: "https://ftp.ebi.ac.uk/pub/databases/Pfam/current_release/Pfam-A.hmm.gz",
            version_url: "https://ftp.ebi.ac.uk/pub/databases/Pfam/current_release/Pfam.version.gz",
            version_gzipped: true,
            parse_version: pfam_release,
            kind: DbKind::Hmm,
            file_name: "Pfam-A.hmm",
            download_bytes: 420_000_000,
            disk_bytes: 1_700_000_000,
        },
    ]
}

pub fn spec(id: &str) -> Option<DbSpec> {
    specs().into_iter().find(|s| s.id == id)
}

/// One installed database, as recorded in the manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledDb {
    pub id: String,
    /// The release string as of the download (e.g. `2026_03`, `38.2`).
    pub version: String,
    pub path: String,
    pub bytes: u64,
    /// u128 millis as a string, the JSON-safe convention `JobInfo` already uses.
    pub installed_at_ms: String,
}

/// The result of a user-requested "check for a newer release".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionCheck {
    pub id: String,
    pub version: String,
    pub checked_at_ms: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DbManifest {
    #[serde(default)]
    pub databases: Vec<InstalledDb>,
    /// Remembered so the panel can still say "an update is available" after a
    /// restart without going back to the network on its own.
    #[serde(default)]
    pub checks: Vec<VersionCheck>,
}

/// `<app_data>/databases` (created if missing).
pub fn managed_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir: {e}"))?
        .join("databases");
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    Ok(dir)
}

fn manifest_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(managed_dir(app)?.join("manifest.json"))
}

pub fn load_manifest(app: &AppHandle) -> DbManifest {
    manifest_path(app)
        .ok()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_manifest(app: &AppHandle, m: &DbManifest) -> Result<(), String> {
    let path = manifest_path(app)?;
    let bytes = serde_json::to_vec_pretty(m).map_err(|e| e.to_string())?;
    // Write-then-rename: a manifest truncated by a crash mid-write would lose track
    // of a database that took a night to download.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &bytes).map_err(|e| format!("writing manifest: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("writing manifest: {e}"))
}

pub fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// Fetch and parse `spec`'s current release string.
pub fn remote_version(spec: &DbSpec) -> Result<String, String> {
    let resp = ureq::get(spec.version_url)
        .call()
        .map_err(|e| format!("fetching {}: {e}", spec.version_url))?;
    let text = if spec.version_gzipped {
        let mut s = String::new();
        flate2::read::GzDecoder::new(resp.into_reader())
            .take(1 << 20)
            .read_to_string(&mut s)
            .map_err(|e| format!("reading {}: {e}", spec.version_url))?;
        s
    } else {
        resp.into_string().map_err(|e| format!("reading {}: {e}", spec.version_url))?
    };
    (spec.parse_version)(&text)
        .ok_or_else(|| format!("could not find a release version in {}", spec.version_url))
}

/// Record a user-requested version check (and return it).
pub fn record_check(app: &AppHandle, id: &str, version: &str) -> Result<VersionCheck, String> {
    let check = VersionCheck {
        id: id.to_string(),
        version: version.to_string(),
        checked_at_ms: now_ms().to_string(),
    };
    let mut m = load_manifest(app);
    m.checks.retain(|c| c.id != id);
    m.checks.push(check.clone());
    save_manifest(app, &m)?;
    Ok(check)
}

/// A reader that counts bytes, reports progress, and aborts on a cancel flag.
///
/// The cancel error is deliberately *not* `ErrorKind::Interrupted`: `io::copy` and
/// friends retry that one, so a cancelled 40 GB download would simply carry on.
struct ProgressReader<'a, R: Read> {
    inner: R,
    done: u64,
    total: u64,
    cancel: &'a AtomicBool,
    report: &'a dyn Fn(&str, u64, u64),
    /// Bytes at the last emitted event — the UI does not need 10k events a second.
    last_reported: u64,
}

/// Progress is emitted at most once per this many bytes of input.
const PROGRESS_STEP: u64 = 8 << 20; // 8 MiB

impl<R: Read> Read for ProgressReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(std::io::Error::other("cancelled"));
        }
        let n = self.inner.read(buf)?;
        self.done += n as u64;
        if self.done - self.last_reported >= PROGRESS_STEP || n == 0 {
            self.last_reported = self.done;
            (self.report)("downloading", self.done, self.total);
        }
        Ok(n)
    }
}

/// Whether `head` is the start of a BGZF member (gzip with the `BC` extra field)
/// or of plain gzip. Mirrors the probe in `domainator_core::io::detect_compression`,
/// which cannot be reused here because it takes a path and we have a live stream.
fn sniff(head: &[u8]) -> Compression {
    let gzip = head.len() >= 3 && head[0] == 0x1f && head[1] == 0x8b && head[2] == 0x08;
    if !gzip {
        return Compression::None;
    }
    if head.len() >= 18 && (head[3] & 0x04) != 0 && head[12] == b'B' && head[13] == b'C' {
        Compression::Bgzf
    } else {
        Compression::Gzip
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Compression {
    None,
    Gzip,
    Bgzf,
}

/// Read up to `n` bytes without failing on a short stream (`read_exact` errors at
/// EOF, and a truncated download should be reported as a download problem).
fn peek<R: Read>(r: &mut R, n: usize) -> std::io::Result<Vec<u8>> {
    let mut buf = vec![0u8; n];
    let mut filled = 0;
    while filled < n {
        match r.read(&mut buf[filled..])? {
            0 => break,
            k => filled += k,
        }
    }
    buf.truncate(filled);
    Ok(buf)
}

/// Free space available to this user on the filesystem holding `dir`, or `None`
/// when it cannot be determined (an unusual filesystem, a path that has gone away).
/// `None` is never treated as "no space": an unknown is not a refusal.
pub fn free_space(dir: &Path) -> Option<u64> {
    fs4::available_space(dir).ok()
}

/// Bytes `spec` needs free before it can be installed into `dir`.
///
/// The second term is the one that is easy to miss: an **update** writes the new
/// copy alongside the old one and only replaces it at the closing rename, so at the
/// peak both exist. Updating TrEMBL in place needs room for two TrEMBLs, and a check
/// that ignored that would pass and then fill the disk four hours in.
pub fn required_bytes(spec: &DbSpec, dir: &Path) -> u64 {
    let existing = std::fs::metadata(dir.join(spec.file_name)).map(|m| m.len()).unwrap_or(0);
    let existing_index = std::fs::metadata(didx_path(&dir.join(spec.file_name)))
        .map(|m| m.len())
        .unwrap_or(0);
    spec.disk_bytes + existing + existing_index
}

fn didx_path(db: &Path) -> PathBuf {
    domainator_core::io::didx::index_path_for(db)
}

/// Refuse a download that cannot fit, before a single byte is fetched.
///
/// The sizes are estimates, so this is deliberately the *only* gate: it refuses
/// when the shortfall is certain and stays out of the way otherwise. Running out
/// of space mid-transfer is recoverable — the partial file is cleaned up — but only
/// after hours of downloading, which is the cost this avoids.
pub fn check_space(spec: &DbSpec, dir: &Path) -> Result<(), String> {
    let Some(free) = free_space(dir) else { return Ok(()) };
    let need = required_bytes(spec, dir);
    if free >= need {
        return Ok(());
    }
    Err(format!(
        "not enough free space for {}: about {} is needed and {} is free on {}.",
        spec.name,
        human_bytes(need),
        human_bytes(free),
        dir.display()
    ))
}

/// Bytes as a short human string, for messages a user reads. Binary units, matching
/// what the panel renders.
pub fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 || v >= 100.0 {
        format!("{} {}", v.round(), UNITS[i])
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

/// Download, convert, index, and record `spec`. Returns its manifest entry.
///
/// `report(phase, done, total)` is called as it goes: `downloading` counts
/// compressed input bytes (`total` 0 when the server sends no `Content-Length`),
/// then `indexing` is a single indeterminate step.
pub fn install(
    app: &AppHandle,
    spec: &DbSpec,
    cancel: &AtomicBool,
    report: &dyn Fn(&str, u64, u64),
) -> Result<InstalledDb, String> {
    let dir = managed_dir(app)?;
    // Cheapest check first, and before any network: a download that cannot fit
    // should not spend hours discovering that.
    check_space(spec, &dir)?;

    // Read the release next, so the manifest records the version that was actually
    // being served when the bytes were taken rather than whatever is current when a
    // multi-hour download finishes.
    let version = remote_version(spec)?;

    let final_path = dir.join(spec.file_name);
    // A partial file must never be mistaken for an installed database, so the whole
    // download lands beside the real name and is renamed on success only.
    let part = dir.join(format!("{}.part", spec.file_name));
    let _ = std::fs::remove_file(&part);

    let resp = ureq::get(spec.url).call().map_err(|e| format!("downloading {}: {e}", spec.url))?;
    let total = resp
        .header("Content-Length")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    report("downloading", 0, total);

    let mut source = ProgressReader {
        inner: resp.into_reader(),
        done: 0,
        total,
        cancel,
        report,
        last_reported: 0,
    };

    let head = peek(&mut source, 18).map_err(|e| format!("downloading {}: {e}", spec.url))?;
    let compression = sniff(&head);
    // Put the sniffed bytes back in front of the rest of the stream.
    let stream = std::io::Cursor::new(head).chain(source);

    let outcome = write_payload(spec, compression, stream, &part);
    if let Err(e) = outcome {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    if cancel.load(Ordering::Relaxed) {
        let _ = std::fs::remove_file(&part);
        return Err("cancelled".into());
    }

    std::fs::rename(&part, &final_path)
        .map_err(|e| format!("installing {}: {e}", final_path.display()))?;

    if spec.kind == DbKind::Fasta {
        report("indexing", 0, 0);
        let path = final_path.to_string_lossy().into_owned();
        let opts = FormatDbOptions { index: true, ..Default::default() };
        // format_db's index-only fast path recognizes that the single output *is*
        // the input and writes the `.didx` in place -- no copy of a 40 GB file.
        format_db(&[path], &opts, &mut std::io::sink())
            .map_err(|e| format!("indexing {}: {e}", final_path.display()))?;
    }

    let bytes = std::fs::metadata(&final_path).map(|m| m.len()).unwrap_or(0);
    let entry = InstalledDb {
        id: spec.id.to_string(),
        version: version.clone(),
        path: final_path.to_string_lossy().into_owned(),
        bytes,
        installed_at_ms: now_ms().to_string(),
    };

    let mut m = load_manifest(app);
    m.databases.retain(|d| d.id != entry.id);
    m.databases.push(entry.clone());
    m.checks.retain(|c| c.id != entry.id);
    m.checks.push(VersionCheck {
        id: entry.id.clone(),
        version,
        checked_at_ms: now_ms().to_string(),
    });
    save_manifest(app, &m)?;
    Ok(entry)
}

/// Stream `stream` into `dest` in the form this database is stored in.
///
/// This is the pass that does the format conversion, and it is a *single* pass on
/// purpose: decompressing TrEMBL to disk before recompressing it would need ~250 GB
/// of scratch space to produce a 43 GB file.
fn write_payload<R: Read>(
    spec: &DbSpec,
    compression: Compression,
    stream: R,
    dest: &Path,
) -> Result<(), String> {
    let out = std::fs::File::create(dest).map_err(|e| format!("writing {}: {e}", dest.display()))?;
    let mut out = std::io::BufWriter::with_capacity(1 << 20, out);

    // Decoded bytes, whatever the asset arrived as. A BGZF asset staying BGZF is the
    // one case that skips this and is copied through verbatim.
    let decode = |stream: R| -> Box<dyn Read> {
        match compression {
            Compression::None => Box::new(stream),
            _ => Box::new(flate2::read::GzDecoder::new(stream)),
        }
    };

    match spec.kind {
        // An already-BGZF asset is exactly the storage format; copying it through
        // preserves the publisher's block boundaries instead of re-deriving them.
        DbKind::Fasta if compression == Compression::Bgzf => {
            let mut stream = stream;
            std::io::copy(&mut stream, &mut out).map_err(|e| format!("writing: {e}"))?;
        }
        DbKind::Fasta => {
            let level = bgzf::CompressionLevel::new(6)
                .map_err(|e| format!("bad compression level: {e:?}"))?;
            let mut writer = bgzf::Writer::new(out, level);
            std::io::copy(&mut decode(stream), &mut writer).map_err(|e| format!("writing: {e}"))?;
            let mut inner = writer.finish().map_err(|e| format!("finishing BGZF: {e}"))?;
            return inner.flush().map_err(|e| format!("writing: {e}"));
        }
        DbKind::Hmm => {
            std::io::copy(&mut decode(stream), &mut out).map_err(|e| format!("writing: {e}"))?;
        }
    }
    out.flush().map_err(|e| format!("writing: {e}"))
}

/// Delete an installed database's files and forget it. The version check is kept:
/// it is what the panel shows as "latest", and it is still true.
pub fn uninstall(app: &AppHandle, id: &str) -> Result<Option<String>, String> {
    let mut m = load_manifest(app);
    let Some(entry) = m.databases.iter().find(|d| d.id == id).cloned() else {
        return Ok(None);
    };
    let path = PathBuf::from(&entry.path);
    // The `.didx` sidecar is ours too; leaving it behind would make a stale index
    // for whatever is written to that name next.
    for p in [didx_path(&path), path.clone()] {
        if p.exists() {
            std::fs::remove_file(&p).map_err(|e| format!("removing {}: {e}", p.display()))?;
        }
    }
    m.databases.retain(|d| d.id != id);
    save_manifest(app, &m)?;
    Ok(Some(entry.path))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The preflight has to count the copy already installed, not just the new one.
    /// An update writes the new file alongside the old and replaces it only at the
    /// closing rename, so the peak is both at once -- updating TrEMBL needs room for
    /// two TrEMBLs. A check that ignored that would pass and then fill the disk
    /// hours into the download, which is exactly the failure it exists to prevent.
    #[test]
    fn an_update_is_required_to_fit_beside_the_copy_it_replaces() {
        let dir = scratch("space");
        let mut spec = fasta_spec("x.fasta.bgz");
        spec.disk_bytes = 1_000;

        // Nothing installed: just the new copy.
        assert_eq!(required_bytes(&spec, &dir), 1_000);

        // An existing database and its index both survive until the rename.
        std::fs::write(dir.join(spec.file_name), vec![0u8; 400]).unwrap();
        std::fs::write(didx_path(&dir.join(spec.file_name)), vec![0u8; 50]).unwrap();
        assert_eq!(required_bytes(&spec, &dir), 1_450);
    }

    /// A shortfall must be refused before any bytes move, and an *unknown* free
    /// figure must not be refused at all -- an unreadable filesystem is not a
    /// reason to make the panel useless.
    #[test]
    fn a_certain_shortfall_is_refused_and_an_unknown_one_is_not() {
        let dir = scratch("space-check");
        let free = free_space(&dir).expect("a temp dir on a normal filesystem reports free space");

        let mut fits = fasta_spec("fits.fasta.bgz");
        fits.disk_bytes = 1;
        assert!(check_space(&fits, &dir).is_ok());

        let mut huge = fasta_spec("huge.fasta.bgz");
        huge.disk_bytes = free.saturating_add(1 << 40);
        let err = check_space(&huge, &dir).unwrap_err();
        // The message has to name all three things a user needs in order to act.
        assert!(err.contains("not enough free space"), "{err}");
        assert!(err.contains(huge.name), "{err}");
        assert!(err.contains(&dir.display().to_string()), "{err}");

        // A path that does not exist yields no figure, and no refusal.
        let gone = dir.join("no-such-directory");
        assert!(free_space(&gone).is_none());
        assert!(check_space(&huge, &gone).is_ok(), "an unknown must never block a download");
    }

    /// The figures in these messages are the ones a user decides on, so the
    /// rounding has to be legible rather than merely correct.
    #[test]
    fn byte_sizes_read_the_way_a_person_would_write_them() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1024), "1.0 KB");
        // Binary units throughout: the specs are written in the publishers' decimal
        // figures, so 94 MB of download reads as 89.6 MiB here.
        assert_eq!(human_bytes(94_000_000), "89.6 MB");
        assert_eq!(human_bytes(46_000_000_000), "42.8 GB");
        // Under 100 keeps one decimal; at or above it, none -- "8.2 GB" is useful
        // where "8 GB" is not, and "104.0 GB" is just noise.
        assert_eq!(human_bytes(8_800_000_000), "8.2 GB");
        assert_eq!(human_bytes(112 * (1 << 30)), "112 GB");
    }

    /// Live check against the publishers, run by hand:
    /// `cargo test -p domainator-gui -- --ignored refs_are_live`.
    ///
    /// Ignored by default because it needs the network, and CI must not go red
    /// because an FTP mirror is slow. But it is the only thing that can catch the
    /// failure that actually happens here: UniProt or EBI moving a path, or
    /// changing the shape of a release file, at which point every download in the
    /// panel breaks and no offline test notices.
    #[test]
    #[ignore = "hits the real UniProt and EBI servers"]
    fn refs_are_live() {
        for spec in specs() {
            let version = remote_version(&spec)
                .unwrap_or_else(|e| panic!("{}: version: {e}", spec.id));
            assert!(!version.is_empty(), "{}: empty version", spec.id);

            let resp = ureq::head(spec.url)
                .call()
                .unwrap_or_else(|e| panic!("{}: HEAD {}: {e}", spec.id, spec.url));
            let len: u64 = resp
                .header("Content-Length")
                .and_then(|v| v.parse().ok())
                .unwrap_or_else(|| panic!("{}: no Content-Length", spec.id));

            // The advertised size is what the confirmation dialog quotes, so it may
            // drift with a release but must not be off by an order of magnitude.
            let ratio = len as f64 / spec.download_bytes as f64;
            assert!(
                (0.5..2.0).contains(&ratio),
                "{}: advertised {} bytes, server says {len} -- update the spec",
                spec.id,
                spec.download_bytes
            );
            println!("{:<10} {:>6}  {:>13} bytes", spec.id, version, len);
        }
    }

    /// Every spec's identity has to line up with what the rest of the app assumes:
    /// ids are unique (the manifest and the cancel registry are keyed by them), file
    /// names are unique (they share one directory), and every URL is https.
    #[test]
    fn the_registry_is_internally_consistent() {
        let specs = specs();
        let mut ids: Vec<&str> = specs.iter().map(|s| s.id).collect();
        let n = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), n, "duplicate database id");

        let mut names: Vec<&str> = specs.iter().map(|s| s.file_name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), n, "two databases claim the same file name");

        for s in &specs {
            assert!(s.url.starts_with("https://"), "{}: downloads must be https", s.id);
            assert!(s.version_url.starts_with("https://"), "{}: version URL must be https", s.id);
            assert!(s.disk_bytes > 0 && s.download_bytes > 0, "{}: sizes are advisory but must be set", s.id);
        }
    }

    /// The stored file name *is* the storage contract, and both halves of it are
    /// load-bearing. A FASTA database that does not end `.bgz` is not seekable, so
    /// `domain_search` cannot use the `.didx` this module writes next to it; an HMM
    /// library that ends in any compression suffix cannot be opened at all, because
    /// `search/hmm_read.rs` only ever opens a plain file.
    #[test]
    fn storage_format_matches_what_the_readers_require() {
        for s in specs() {
            match s.kind {
                DbKind::Fasta => {
                    assert!(s.file_name.ends_with(".bgz"), "{} must be stored BGZF", s.id);
                    assert!(
                        s.file_name.contains(".fasta."),
                        "{}: the reader picks the format off the name",
                        s.id
                    );
                }
                DbKind::Hmm => {
                    assert!(s.file_name.ends_with(".hmm"), "{} must be stored uncompressed", s.id);
                }
            }
            // Whatever the name, the app's own classifier has to agree it is a
            // database of that kind -- this is what the Databases panel will run.
            let classified = domainator_core::app_support::workspace::classify_file(Path::new(
                s.file_name,
            ));
            assert_eq!(classified, Some(s.kind.as_str()), "{}", s.id);
        }
    }

    /// The confirmation gate is the only thing standing between a stray click and a
    /// 40 GB download, and Swiss-Prot is the only set small enough to skip it.
    #[test]
    fn only_the_small_database_downloads_without_confirmation() {
        for s in specs() {
            assert_eq!(
                s.needs_confirmation(),
                s.id != "swissprot",
                "{} confirmation gate",
                s.id
            );
        }
    }

    /// Parsed from the real files' shapes; a release string that comes back empty or
    /// wrong is displayed to the user as the installed version forever after.
    #[test]
    fn release_versions_are_parsed_from_their_publishers_files() {
        let reldate = "UniProt Knowledgebase Release 2026_03 consists of:\n\
                       UniProtKB/Swiss-Prot Release 2026_03 of 02-Sep-2026\n";
        assert_eq!(uniprot_release(reldate).as_deref(), Some("2026_03"));
        assert_eq!(uniprot_release("UniProt Release 2026_03\n").as_deref(), Some("2026_03"));
        assert_eq!(uniprot_release("nothing useful here\n"), None);

        let pfam = "Pfam release       : 38.2\n\
                    Pfam-A families    : 30134\n\
                    Date               : 2026-01\n";
        assert_eq!(pfam_release(pfam).as_deref(), Some("38.2"));
        // "Based on UniProtKB : 2025_03" is a different key and must not match.
        assert_eq!(pfam_release("Based on UniProtKB : 2025_03\n"), None);
        assert_eq!(pfam_release("no colon here\n"), None);
    }

    /// Which branch of `write_payload` runs is decided by these 18 bytes, and the
    /// two gzip cases look identical until byte 12. Getting it wrong either stores
    /// plain gzip under a `.bgz` name (unseekable, so the database silently does not
    /// work) or re-deflates an already-BGZF file for nothing.
    #[test]
    fn bgzf_is_distinguished_from_plain_gzip_by_its_extra_field() {
        let mut bgzf = vec![0u8; 18];
        bgzf[0..3].copy_from_slice(&[0x1f, 0x8b, 0x08]);
        bgzf[3] = 0x04; // FEXTRA
        bgzf[12] = b'B';
        bgzf[13] = b'C';
        assert_eq!(sniff(&bgzf), Compression::Bgzf);

        // Same magic, no extra field: what UniProt and Pfam actually serve.
        let mut plain = bgzf.clone();
        plain[3] = 0x00;
        assert_eq!(sniff(&plain), Compression::Gzip);

        // FEXTRA set, but some other subfield.
        let mut other = bgzf.clone();
        other[12] = b'X';
        assert_eq!(sniff(&other), Compression::Gzip);

        assert_eq!(sniff(b">seq\nMKV"), Compression::None);
        // A stream too short to carry the extra field is not BGZF.
        assert_eq!(sniff(&bgzf[..4]), Compression::Gzip);
        assert_eq!(sniff(b""), Compression::None);
    }

    /// `peek` has to hand back exactly what it consumed, or the bytes it swallowed
    /// are gone from the payload — for a FASTA that is a lost first record.
    #[test]
    fn peeking_the_header_does_not_consume_the_stream() {
        let data: Vec<u8> = (0u8..=255).collect();
        let mut src = std::io::Cursor::new(data.clone());
        let head = peek(&mut src, 18).unwrap();
        assert_eq!(head, data[..18]);

        let mut rest = Vec::new();
        std::io::Cursor::new(head).chain(src).read_to_end(&mut rest).unwrap();
        assert_eq!(rest, data, "the peeked bytes must be put back in front");

        // A stream shorter than the peek is truncated, not an error.
        let mut short = std::io::Cursor::new(b"abc".to_vec());
        assert_eq!(peek(&mut short, 18).unwrap(), b"abc");
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("domainator-refdb-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn fasta_spec(file_name: &'static str) -> DbSpec {
        DbSpec {
            id: "test",
            name: "test",
            description: "",
            url: "https://example.invalid/x.fasta.gz",
            version_url: "https://example.invalid/v",
            version_gzipped: false,
            parse_version: uniprot_release,
            kind: DbKind::Fasta,
            file_name,
            download_bytes: 1,
            disk_bytes: 1,
        }
    }

    /// The whole conversion chain, on the exact shape UniProt serves: plain gzip in,
    /// BGZF plus a `.didx` out, and the result readable *as a database*.
    ///
    /// Every step of this is silently survivable if it goes wrong -- a file lands on
    /// disk at a plausible size either way. Storing the gzip bytes verbatim, or
    /// writing an index that does not match the blocks, both produce a database that
    /// looks installed and fails only later inside a `domain_search` run.
    #[test]
    fn a_gzipped_fasta_is_installed_as_an_indexed_bgzf_database() {
        use domainator_core::io::{detect_compression, didx};

        let dir = scratch("install");
        let records = "\
>sp|P00001|A Alpha
MKVLAWIYPQ
>sp|P00002|B Beta
MTTQAPMFTQ
>sp|P00003|C Gamma
MSDKIIHLTD
";
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        enc.write_all(records.as_bytes()).unwrap();
        let gz = enc.finish().unwrap();
        assert_eq!(sniff(&gz), Compression::Gzip, "the fixture must be plain gzip, as UniProt's is");

        let spec = fasta_spec("test.fasta.bgz");
        let dest = dir.join(spec.file_name);
        write_payload(&spec, Compression::Gzip, std::io::Cursor::new(gz), &dest).unwrap();

        // Seekable, which plain gzip is not -- this is the point of recompressing.
        assert_eq!(detect_compression(&dest), domainator_core::io::Compression::Bgzf);

        // And it still gunzips to exactly the published bytes.
        let mut back = String::new();
        flate2::read::MultiGzDecoder::new(std::fs::File::open(&dest).unwrap())
            .read_to_string(&mut back)
            .unwrap();
        assert_eq!(back, records);

        // The index step `install` runs. It must write the sidecar in place rather
        // than copying a (potentially 40 GB) file to a new name.
        let opts = FormatDbOptions { index: true, ..Default::default() };
        format_db(&[dest.to_string_lossy().into_owned()], &opts, &mut std::io::sink()).unwrap();
        let idx = didx::index_path_for(&dest);
        assert!(idx.is_file(), "format_db wrote no .didx");
        assert!(dest.is_file(), "the database itself must stay where it was indexed");

        // The index has to agree with the file: one offset per record, each of which
        // really is where that record starts.
        let (offsets, _cds) = didx::read_index(
            &dest,
            domainator_core::io::FileType::Fasta,
            domainator_core::io::Compression::Bgzf,
        )
        .expect("the .didx we just wrote is readable")
        // `None` here means the index was judged stale against the file it was
        // just built from -- which would make every run silently rescan.
        .expect("the .didx must be fresh for the file it indexes");
        assert_eq!(offsets.len(), 3, "one offset per record");
        for (i, off) in offsets.iter().enumerate() {
            let recs = domainator_core::io::fasta::read_fasta_partition(
                dest.to_str().unwrap(),
                *off,
                true,
                1,
                None,
            )
            .unwrap();
            assert_eq!(recs[0].id, format!("sp|P0000{}|{}", i + 1, ["A", "B", "C"][i]));
        }
    }

    /// Pfam is the other half of the storage contract: the `.gz` is expanded, not
    /// kept, because `search/hmm_read.rs` opens a plain file and there is no
    /// compressed-HMM reader anywhere in the port to fall back on.
    #[test]
    fn a_gzipped_hmm_library_is_installed_expanded() {
        let dir = scratch("hmm");
        let profile = "HMMER3/f [3.4]\nNAME  PF00001\nALPH  amino\n//\n";
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        enc.write_all(profile.as_bytes()).unwrap();
        let gz = enc.finish().unwrap();

        let spec = DbSpec { kind: DbKind::Hmm, file_name: "test.hmm", ..fasta_spec("test.hmm") };
        let dest = dir.join(spec.file_name);
        write_payload(&spec, Compression::Gzip, std::io::Cursor::new(gz), &dest).unwrap();

        assert_eq!(std::fs::read_to_string(&dest).unwrap(), profile);
        // And the app's own classifier agrees it is a usable protein HMM database.
        let entry =
            domainator_core::app_support::registry::classify_database(dest.to_str().unwrap())
                .unwrap();
        assert_eq!((entry.format.as_str(), entry.molecule.as_str()), ("hmm", "protein"));
    }

    /// An asset that already arrives as BGZF is the storage format, and must be
    /// stored byte for byte rather than decoded and re-deflated: the publisher's
    /// block boundaries are as good as ours, and a round trip through deflate is a
    /// gratuitous rewrite of tens of gigabytes.
    #[test]
    fn an_already_bgzf_asset_is_copied_through_verbatim() {
        let dir = scratch("passthrough");
        let mut buf = Vec::new();
        {
            let mut w = bgzf::Writer::new(&mut buf, bgzf::CompressionLevel::new(6).unwrap());
            w.write_all(b">a\nMKV\n").unwrap();
            w.finish().unwrap();
        }
        assert_eq!(sniff(&buf), Compression::Bgzf);

        let spec = fasta_spec("passthrough.fasta.bgz");
        let dest = dir.join(spec.file_name);
        write_payload(&spec, Compression::Bgzf, std::io::Cursor::new(buf.clone()), &dest).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), buf);
    }

    /// A cancel must stop the read rather than be retried away. `io::copy` loops on
    /// `ErrorKind::Interrupted`, so using it here would make cancelling a 40 GB
    /// download do nothing at all.
    #[test]
    fn cancelling_stops_the_stream_with_a_non_retryable_error() {
        let cancel = AtomicBool::new(false);
        let noop = |_: &str, _: u64, _: u64| {};
        let mut r = ProgressReader {
            inner: std::io::Cursor::new(vec![7u8; 4096]),
            done: 0,
            total: 4096,
            cancel: &cancel,
            report: &noop,
            last_reported: 0,
        };
        let mut buf = [0u8; 16];
        assert_eq!(r.read(&mut buf).unwrap(), 16);

        cancel.store(true, Ordering::Relaxed);
        let err = r.read(&mut buf).unwrap_err();
        assert_ne!(err.kind(), std::io::ErrorKind::Interrupted, "io::copy would retry this");
        assert!(err.to_string().contains("cancelled"));

        // And the error is what a copy of the whole stream surfaces.
        let mut sink = Vec::new();
        assert!(std::io::copy(&mut r, &mut sink).is_err());
    }
}
