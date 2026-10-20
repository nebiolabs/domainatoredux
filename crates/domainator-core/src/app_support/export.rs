//! Export data files out of a workspace: copy them into a directory the user
//! picks, optionally gzipped one by one or bundled into a zip, and optionally
//! split so that every sequence record or HMM profile becomes its own file.
//!
//! Export works in two steps. **Planning** names every output, so a name that
//! already exists at the destination can be reported (and the user asked) before
//! anything is written. **Writing** then streams each source through
//! [`crate::io::open_decompressed`], so plain, gzip and BGZF inputs all split the
//! same way. A record is copied as its original bytes, never parsed and
//! re-serialized: splitting must not change what a record says.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use flate2::write::GzEncoder;
use serde::{Deserialize, Serialize};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use super::workspace::classify_file;

/// How the exported files are packaged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Packaging {
    /// Files land in the destination directory as they are.
    Plain,
    /// Each file is gzipped on its own (`foo.gb` → `foo.gb.gz`). A file that is
    /// already gzip or BGZF is copied unchanged rather than compressed twice.
    Gzip,
    /// Everything goes into one zip archive; the destination is the archive path.
    Zip,
}

/// What to export and how.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportOptions {
    pub packaging: Packaging,
    /// Write every GenBank/FASTA record and HMM profile as its own file. Other
    /// file types are exported whole.
    pub split_records: bool,
}

/// The result of [`export_files`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ExportOutcome {
    /// Nothing was written: these output names already exist in the destination
    /// directory. Call again with `overwrite` to replace them.
    Conflicts { names: Vec<String> },
    /// The export finished: `written` files (or archive entries) at `dest`.
    Done { written: usize, dest: String },
}

/// Export `sources` to `dest`.
///
/// For [`Packaging::Plain`] and [`Packaging::Gzip`], `dest` is an existing
/// directory; for [`Packaging::Zip`] it is the archive file to create (the save
/// dialog that chose it has already confirmed replacing it). Without `overwrite`,
/// a directory export whose outputs collide with existing files writes nothing and
/// returns [`ExportOutcome::Conflicts`]. Writing over one of the sources is always
/// refused.
pub fn export_files(
    sources: &[PathBuf],
    opts: &ExportOptions,
    dest: &Path,
    overwrite: bool,
) -> Result<ExportOutcome, String> {
    if sources.is_empty() {
        return Err("nothing to export".to_string());
    }
    for src in sources {
        if !src.is_file() {
            return Err(format!("not a file: {}", src.display()));
        }
    }
    let zip = opts.packaging == Packaging::Zip;
    if zip {
        if dest.is_dir() {
            return Err(format!(
                "{} is a directory, not an archive name",
                dest.display()
            ));
        }
        let parent = dest.parent().filter(|p| !p.as_os_str().is_empty());
        if parent.is_some_and(|p| !p.is_dir()) {
            return Err(format!("no such directory for {}", dest.display()));
        }
    } else if !dest.is_dir() {
        return Err(format!("not a directory: {}", dest.display()));
    }

    let jobs = plan(sources, opts)?;
    let targets: Vec<PathBuf> = if zip {
        vec![dest.to_path_buf()]
    } else {
        jobs.iter()
            .flat_map(Job::names)
            .map(|n| dest.join(n))
            .collect()
    };
    refuse_overwriting_sources(sources, &targets)?;
    if !zip && !overwrite {
        let names: Vec<String> = jobs
            .iter()
            .flat_map(Job::names)
            .filter(|n| dest.join(n).exists())
            .map(str::to_string)
            .collect();
        if !names.is_empty() {
            return Ok(ExportOutcome::Conflicts { names });
        }
    }

    let mut sink =
        Sink::open(dest, zip).map_err(|e| format!("creating {}: {e}", dest.display()))?;
    let run = jobs.iter().try_for_each(|job| {
        job.write(&mut sink, opts.packaging)
            .map_err(|e| format!("exporting {}: {e}", job.src().display()))
    });
    match run {
        Ok(()) => {
            let written = sink
                .close()
                .map_err(|e| format!("writing {}: {e}", dest.display()))?;
            Ok(ExportOutcome::Done {
                written,
                dest: dest.to_string_lossy().into_owned(),
            })
        }
        Err(e) => {
            sink.abort();
            Err(e)
        }
    }
}

/// A file name made from a record id: characters no file system accepts become
/// `_`, as do a leading dot (which would hide the file) and Windows device names
/// (`CON`, `NUL`, `COM1`, … — reserved with any extension). Never empty.
pub fn safe_file_stem(id: &str) -> String {
    const INVALID: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];
    let mut s: String = id
        .chars()
        .map(|c| {
            if c.is_control() || INVALID.contains(&c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    // Most file systems cap a name at 255 bytes; leave room for the extension
    // and a ` (n)` suffix.
    if s.len() > 200 {
        let cut = (0..=200)
            .rev()
            .find(|&i| s.is_char_boundary(i))
            .unwrap_or(0);
        s.truncate(cut);
    }
    // Windows strips trailing dots and spaces, which would merge names.
    let keep = s.trim_end_matches(['.', ' ']).len();
    s.truncate(keep);
    let s = s.trim_start_matches(' ');
    if s.is_empty() {
        return "record".to_string();
    }
    let mut s = s.to_string();
    if s.starts_with('.') {
        s.replace_range(..1, "_");
    }
    let base = s.split('.').next().unwrap_or("").to_ascii_uppercase();
    let device = matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((base.starts_with("COM") || base.starts_with("LPT"))
            && base.len() == 4
            && base.as_bytes()[3].is_ascii_digit()
            && base.as_bytes()[3] != b'0');
    if device {
        s.insert(0, '_');
    }
    s
}

// ── planning ─────────────────────────────────────────────────────────────────

/// The files a record can be split out of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecordKind {
    Genbank,
    Fasta,
    Hmm,
}

impl RecordKind {
    fn of(path: &Path) -> Option<Self> {
        match classify_file(path)? {
            "genbank" => Some(Self::Genbank),
            "fasta" => Some(Self::Fasta),
            "hmm" => Some(Self::Hmm),
            _ => None,
        }
    }

    /// Whether `line` opens a new record. The same prefixes
    /// `workspace::count_records` counts, plus HMMER2 headers.
    fn is_start(self, line: &[u8]) -> bool {
        match self {
            Self::Genbank => line.starts_with(b"LOCUS"),
            Self::Fasta => line.starts_with(b">"),
            Self::Hmm => line.starts_with(b"HMMER3/") || line.starts_with(b"HMMER2"),
        }
    }

    /// The record id carried by its start line (an HMM's is on its `NAME` line).
    fn id_from_start(self, line: &[u8]) -> Option<String> {
        let text = String::from_utf8_lossy(line);
        match self {
            Self::Genbank => text.split_whitespace().nth(1).map(str::to_string),
            Self::Fasta => text[1..].split_whitespace().next().map(str::to_string),
            Self::Hmm => None,
        }
    }
}

/// One source's share of the export.
enum Job {
    /// The file as it is, under `name`. `compressed` is whether its content is
    /// already gzip/BGZF, so it is never compressed a second time.
    Whole {
        src: PathBuf,
        name: String,
        compressed: bool,
    },
    /// One output per record, named in record order.
    Split {
        src: PathBuf,
        kind: RecordKind,
        names: Vec<String>,
    },
}

impl Job {
    fn src(&self) -> &Path {
        match self {
            Self::Whole { src, .. } | Self::Split { src, .. } => src,
        }
    }

    fn names(&self) -> Box<dyn Iterator<Item = &str> + '_> {
        match self {
            Self::Whole { name, .. } => Box::new(std::iter::once(name.as_str())),
            Self::Split { names, .. } => Box::new(names.iter().map(String::as_str)),
        }
    }

    fn write(&self, sink: &mut Sink, packaging: Packaging) -> io::Result<()> {
        let mtime = fs::metadata(self.src()).and_then(|m| m.modified()).ok();
        match self {
            Self::Whole {
                src,
                name,
                compressed,
            } => {
                let len = fs::metadata(src)?.len();
                let entry = Entry {
                    compress: packaging != Packaging::Plain && !compressed,
                    large: len >= u64::from(u32::MAX),
                    mtime,
                };
                sink.begin(name, &entry)?;
                io::copy(&mut File::open(src)?, sink)?;
                sink.finish()
            }
            Self::Split { src, kind, names } => {
                // A record's size is not known until it has been read; assume a
                // compressed source can expand by up to 16x.
                let len = fs::metadata(src)?.len();
                let bound = if crate::io::has_gzip_magic(src) {
                    len.saturating_mul(16)
                } else {
                    len
                };
                let entry = Entry {
                    compress: packaging != Packaging::Plain,
                    large: bound >= u64::from(u32::MAX),
                    mtime,
                };
                let changed =
                    || io::Error::other("the file changed while it was being exported; try again");
                let mut next = 0usize;
                walk_records(src, *kind, |line, start| {
                    if start {
                        if next > 0 {
                            sink.finish()?;
                        }
                        sink.begin(names.get(next).ok_or_else(changed)?, &entry)?;
                        next += 1;
                    }
                    sink.write_all(line)
                })?;
                if next > 0 {
                    sink.finish()?;
                }
                if next != names.len() {
                    return Err(changed());
                }
                Ok(())
            }
        }
    }
}

/// Name every output, deduplicated across the whole export (case-insensitively,
/// since Windows and macOS file systems are).
fn plan(sources: &[PathBuf], opts: &ExportOptions) -> Result<Vec<Job>, String> {
    let gz = if opts.packaging == Packaging::Gzip {
        ".gz"
    } else {
        ""
    };
    let mut used = Names::default();
    let mut jobs = Vec::with_capacity(sources.len());
    for src in sources {
        let file_name = src
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| format!("not a file: {}", src.display()))?;
        let (stem, ext) = split_name(&file_name);
        let kind = RecordKind::of(src).filter(|_| opts.split_records);
        if let Some(kind) = kind {
            let ids =
                record_ids(src, kind).map_err(|e| format!("reading {}: {e}", src.display()))?;
            // A file with no records splits into nothing; export it whole instead.
            if !ids.is_empty() {
                let rec_ext = format!("{}{gz}", format_ext(&file_name));
                let names = ids
                    .iter()
                    .enumerate()
                    .map(|(i, id)| {
                        let id_stem = match id {
                            Some(id) => safe_file_stem(id),
                            None => safe_file_stem(&format!("{stem}_{}", i + 1)),
                        };
                        used.claim(&id_stem, &rec_ext)
                    })
                    .collect();
                jobs.push(Job::Split {
                    src: src.clone(),
                    kind,
                    names,
                });
                continue;
            }
        }
        let compressed = crate::io::has_gzip_magic(src);
        let ext = if compressed {
            ext.to_string()
        } else {
            format!("{ext}{gz}")
        };
        jobs.push(Job::Whole {
            src: src.clone(),
            name: used.claim(stem, &ext),
            compressed,
        });
    }
    Ok(jobs)
}

/// Output names already handed out, compared case-insensitively.
#[derive(Default)]
struct Names(HashSet<String>);

impl Names {
    /// `stem + ext`, or the first free `stem (n) + ext` — the same suffix scheme
    /// the GUI's import uses, but before the whole compound extension.
    fn claim(&mut self, stem: &str, ext: &str) -> String {
        for n in 1.. {
            let name = if n == 1 {
                format!("{stem}{ext}")
            } else {
                format!("{stem} ({n}){ext}")
            };
            if self.0.insert(name.to_lowercase()) {
                return name;
            }
        }
        unreachable!("an unbounded range always finds a free name")
    }
}

/// The compression suffix of a name (`.gz`/`.bgz`/`.bgzf`, any case), or `""`.
fn compression_suffix(name: &str) -> &str {
    let lower = name.to_ascii_lowercase();
    [".gz", ".bgz", ".bgzf"]
        .iter()
        .find(|s| lower.len() > s.len() && lower.ends_with(*s))
        .map_or("", |s| &name[name.len() - s.len()..])
}

/// `foo.gb.gz` → `("foo", ".gb.gz")`: the format extension together with any
/// compression suffix, so a ` (n)` goes before both.
fn split_name(name: &str) -> (&str, &str) {
    let comp = compression_suffix(name);
    let base = &name[..name.len() - comp.len()];
    match base.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (base, &name[base.len()..]),
    }
}

/// The format extension alone: `foo.gb.gz` → `.gb`.
fn format_ext(name: &str) -> &str {
    let (_, ext) = split_name(name);
    &ext[..ext.len() - compression_suffix(name).len()]
}

/// Stream `path`'s records, calling `on_line(line, is_start)` for every line that
/// belongs to one. Text before the first record (a release header, say) is
/// skipped, as the readers skip it; text after a record's end runs with it.
fn walk_records(
    path: &Path,
    kind: RecordKind,
    mut on_line: impl FnMut(&[u8], bool) -> io::Result<()>,
) -> io::Result<()> {
    let mut reader = BufReader::with_capacity(1 << 16, crate::io::open_decompressed(path)?);
    let mut line = Vec::new();
    let mut inside = false;
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        let start = kind.is_start(&line);
        inside |= start;
        if inside {
            on_line(&line, start)?;
        }
    }
}

/// Each record's id, in file order (`None` when it has none).
fn record_ids(path: &Path, kind: RecordKind) -> io::Result<Vec<Option<String>>> {
    let mut ids: Vec<Option<String>> = Vec::new();
    walk_records(path, kind, |line, start| {
        if start {
            ids.push(kind.id_from_start(line));
        } else if kind == RecordKind::Hmm {
            if let (Some(last @ None), Some(rest)) = (ids.last_mut(), line.strip_prefix(b"NAME")) {
                if rest.first().is_some_and(u8::is_ascii_whitespace) {
                    *last = String::from_utf8_lossy(rest)
                        .split_whitespace()
                        .next()
                        .map(str::to_string);
                }
            }
        }
        Ok(())
    })?;
    Ok(ids)
}

/// An output must never land on a source, which it would destroy (or, for a
/// split, truncate mid-read).
fn refuse_overwriting_sources(sources: &[PathBuf], targets: &[PathBuf]) -> Result<(), String> {
    let sources: HashSet<PathBuf> = sources
        .iter()
        .filter_map(|s| fs::canonicalize(s).ok())
        .collect();
    for target in targets {
        if let Ok(t) = fs::canonicalize(target) {
            if sources.contains(&t) {
                return Err(format!(
                    "exporting here would overwrite {}, which is being exported",
                    target.display()
                ));
            }
        }
    }
    Ok(())
}

// ── writing ──────────────────────────────────────────────────────────────────

/// How one output is written.
struct Entry {
    /// Gzip it (a directory export) or deflate it (a zip entry).
    compress: bool,
    /// It may reach 4 GiB, so a zip entry needs ZIP64 sizes.
    large: bool,
    mtime: Option<std::time::SystemTime>,
}

/// Where outputs go. Every file is written as `<name>.part` and renamed into
/// place only once complete, so a failed export never leaves a truncated file
/// under a real name, and never destroys the file it was about to replace.
enum Sink {
    Dir {
        dir: PathBuf,
        current: Option<DirFile>,
        written: usize,
    },
    Zip {
        archive: PathBuf,
        part: PathBuf,
        // Boxed: the writer dwarfs the `Dir` variant.
        zip: Box<ZipWriter<BufWriter<File>>>,
        written: usize,
    },
}

struct DirFile {
    part: PathBuf,
    target: PathBuf,
    writer: DirWriter,
}

enum DirWriter {
    Plain(BufWriter<File>),
    Gzip(GzEncoder<BufWriter<File>>),
}

fn part_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    path.with_file_name(name)
}

impl Sink {
    fn open(dest: &Path, zip: bool) -> io::Result<Self> {
        if zip {
            let part = part_path(dest);
            let file = BufWriter::new(File::create(&part)?);
            Ok(Self::Zip {
                archive: dest.to_path_buf(),
                part,
                zip: Box::new(ZipWriter::new(file)),
                written: 0,
            })
        } else {
            Ok(Self::Dir {
                dir: dest.to_path_buf(),
                current: None,
                written: 0,
            })
        }
    }

    fn begin(&mut self, name: &str, entry: &Entry) -> io::Result<()> {
        match self {
            Self::Dir { dir, current, .. } => {
                let target = dir.join(name);
                let part = part_path(&target);
                let file = BufWriter::new(File::create(&part)?);
                let writer = if entry.compress {
                    DirWriter::Gzip(GzEncoder::new(file, flate2::Compression::default()))
                } else {
                    DirWriter::Plain(file)
                };
                *current = Some(DirFile {
                    part,
                    target,
                    writer,
                });
            }
            Self::Zip { zip, .. } => {
                let method = if entry.compress {
                    CompressionMethod::Deflated
                } else {
                    CompressionMethod::Stored
                };
                let opts = SimpleFileOptions::default()
                    .compression_method(method)
                    .large_file(entry.large)
                    .last_modified_time(entry.mtime.and_then(zip_time).unwrap_or_default());
                zip.start_file(name, opts).map_err(io::Error::other)?;
            }
        }
        Ok(())
    }

    fn finish(&mut self) -> io::Result<()> {
        match self {
            Self::Dir {
                current, written, ..
            } => {
                let Some(f) = current.take() else {
                    return Ok(());
                };
                let file = match f.writer {
                    DirWriter::Plain(w) => w,
                    DirWriter::Gzip(gz) => gz.finish()?,
                };
                file.into_inner()
                    .map_err(io::IntoInnerError::into_error)?
                    .sync_all()?;
                if let Err(e) = fs::rename(&f.part, &f.target) {
                    let _ = fs::remove_file(&f.part);
                    return Err(e);
                }
                *written += 1;
            }
            // The zip writer closes an entry when the next starts, or at the end.
            Self::Zip { written, .. } => *written += 1,
        }
        Ok(())
    }

    /// Finish the export; returns how many files (or entries) were written.
    fn close(self) -> io::Result<usize> {
        match self {
            Self::Dir { written, .. } => Ok(written),
            Self::Zip {
                archive,
                part,
                zip,
                written,
            } => {
                let done = (*zip)
                    .finish()
                    .map_err(io::Error::other)
                    .and_then(|w| w.into_inner().map_err(io::IntoInnerError::into_error))
                    .and_then(|f| f.sync_all())
                    .and_then(|()| fs::rename(&part, &archive));
                if done.is_err() {
                    let _ = fs::remove_file(&part);
                }
                done.map(|()| written)
            }
        }
    }

    /// Abandon the export, removing the partial output in progress. Files an
    /// earlier step already finished stay.
    fn abort(self) {
        match self {
            Self::Dir { current, .. } => {
                if let Some(f) = current {
                    drop(f.writer);
                    let _ = fs::remove_file(&f.part);
                }
            }
            Self::Zip { part, zip, .. } => {
                drop(zip);
                let _ = fs::remove_file(&part);
            }
        }
    }
}

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::Dir { current, .. } => match current.as_mut().map(|f| &mut f.writer) {
                Some(DirWriter::Plain(w)) => w.write(buf),
                Some(DirWriter::Gzip(w)) => w.write(buf),
                None => Err(io::Error::other("no export file is open")),
            },
            Self::Zip { zip, .. } => zip.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Dir { current, .. } => match current.as_mut().map(|f| &mut f.writer) {
                Some(DirWriter::Plain(w)) => w.flush(),
                Some(DirWriter::Gzip(w)) => w.flush(),
                None => Ok(()),
            },
            Self::Zip { zip, .. } => zip.flush(),
        }
    }
}

/// A file time as a zip timestamp. Zip times carry no zone; this is UTC (there is
/// no timezone database here), and `None` outside zip's 1980–2107 range.
fn zip_time(t: std::time::SystemTime) -> Option<zip::DateTime> {
    let secs = t.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    let (y, m, d) = crate::io::genbank::writer::civil_from_days((secs / 86_400) as i64);
    let sod = secs % 86_400;
    zip::DateTime::from_date_and_time(
        u16::try_from(y).ok()?,
        m as u8,
        d as u8,
        (sod / 3600) as u8,
        (sod % 3600 / 60) as u8,
        (sod % 60) as u8,
    )
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("domainator-export-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::create_dir_all(dir.join("out")).unwrap();
        dir
    }

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut enc = GzEncoder::new(Vec::new(), flate2::Compression::fast());
        enc.write_all(bytes).unwrap();
        enc.finish().unwrap()
    }

    fn gunzip(bytes: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        flate2::read::MultiGzDecoder::new(bytes)
            .read_to_end(&mut out)
            .unwrap();
        out
    }

    fn locus(id: &str) -> String {
        format!(
            "LOCUS       {id}                10 bp    DNA     linear   UNK 01-JAN-1980\n\
             DEFINITION  {id}.\nORIGIN\n        1 acgtacgtac\n//\n"
        )
    }

    const PREAMBLE: &str = "GBBCT1.SEQ          Genetic Sequence Data Bank\n\n";

    fn genbank() -> (String, String) {
        let records = format!("{}{}{}", locus("rec1"), locus("a/b:c"), locus("REC1"));
        (format!("{PREAMBLE}{records}"), records)
    }

    fn opts(packaging: Packaging, split_records: bool) -> ExportOptions {
        ExportOptions {
            packaging,
            split_records,
        }
    }

    fn listing(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn done(written: usize, dest: &Path) -> ExportOutcome {
        ExportOutcome::Done {
            written,
            dest: dest.to_string_lossy().into_owned(),
        }
    }

    #[test]
    fn splitting_a_genbank_writes_each_record_verbatim() {
        let dir = scratch("split-gb");
        let (text, records) = genbank();
        for (name, bytes) in [
            ("plain.gb", text.as_bytes().to_vec()),
            ("packed.gb.gz", gzip(text.as_bytes())),
        ] {
            let src = dir.join("src").join(name);
            fs::write(&src, bytes).unwrap();
            let out = dir.join("out").join(name.replace('.', "_"));
            fs::create_dir_all(&out).unwrap();
            let got = export_files(&[src], &opts(Packaging::Plain, true), &out, false).unwrap();
            assert_eq!(got, done(3, &out));
            // A case-only duplicate gets a suffix; the slash and colon do not survive.
            assert_eq!(listing(&out), ["REC1 (2).gb", "a_b_c.gb", "rec1.gb"]);
            let joined: String = ["rec1.gb", "a_b_c.gb", "REC1 (2).gb"]
                .iter()
                .map(|n| fs::read_to_string(out.join(n)).unwrap())
                .collect();
            assert_eq!(joined, records);
        }
    }

    #[test]
    fn splitting_fasta_and_hmm_names_each_file_after_its_record() {
        let dir = scratch("split-fa-hmm");
        let fasta = dir.join("src/prots.faa");
        fs::write(&fasta, ">p1 first protein\nMKV\nLL\n>p2\nMAA\n").unwrap();

        let mut hmms = Vec::new();
        for name in ["first", "second"] {
            let mut hmm =
                hmmer_pure_rs::hmm::Hmm::new(2, hmmer_pure_rs::alphabet::AlphabetType::Dna, 4);
            hmm.name = name.into();
            hmm.nseq = -1;
            hmm.eff_nseq = -1.0;
            for node in 0..=2 {
                hmm.mat[node] = vec![0.25; 4];
                hmm.ins[node] = vec![0.25; 4];
                hmm.t[node] = [1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0];
            }
            crate::search::hmm_write::write_hmm(&mut hmms, &hmm).unwrap();
        }
        let hmm = dir.join("src/profiles.hmm.gz");
        fs::write(&hmm, gzip(&hmms)).unwrap();

        let out = dir.join("out");
        let got = export_files(&[fasta, hmm], &opts(Packaging::Plain, true), &out, false).unwrap();
        assert_eq!(got, done(4, &out));
        assert_eq!(
            listing(&out),
            ["first.hmm", "p1.faa", "p2.faa", "second.hmm"]
        );
        assert_eq!(
            fs::read_to_string(out.join("p1.faa")).unwrap(),
            ">p1 first protein\nMKV\nLL\n"
        );
        for name in ["first", "second"] {
            let read =
                crate::search::hmm_read::read_hmm_file(&out.join(format!("{name}.hmm"))).unwrap();
            assert_eq!(read.len(), 1);
            assert_eq!(read[0].name, name);
        }
    }

    #[test]
    fn unsplittable_files_export_whole_even_when_splitting() {
        let dir = scratch("unsplittable");
        let table = dir.join("src/hits.tsv");
        fs::write(&table, "a\tb\n").unwrap();
        let empty = dir.join("src/empty.fasta");
        fs::write(&empty, "").unwrap();
        let out = dir.join("out");
        export_files(&[table, empty], &opts(Packaging::Plain, true), &out, false).unwrap();
        assert_eq!(listing(&out), ["empty.fasta", "hits.tsv"]);
    }

    #[test]
    fn gzip_compresses_each_file_once() {
        let dir = scratch("gzip");
        let plain = dir.join("src/a.gb");
        let (text, _) = genbank();
        fs::write(&plain, &text).unwrap();
        let packed = dir.join("src/b.fasta.gz");
        let packed_bytes = gzip(b">x\nAC\n");
        fs::write(&packed, &packed_bytes).unwrap();
        let out = dir.join("out");
        export_files(&[plain, packed], &opts(Packaging::Gzip, false), &out, false).unwrap();
        assert_eq!(listing(&out), ["a.gb.gz", "b.fasta.gz"]);
        assert_eq!(
            gunzip(&fs::read(out.join("a.gb.gz")).unwrap()),
            text.as_bytes()
        );
        // Already compressed: the very same bytes, not gzip inside gzip.
        assert_eq!(fs::read(out.join("b.fasta.gz")).unwrap(), packed_bytes);
    }

    #[test]
    fn gzip_and_split_gzips_every_record() {
        let dir = scratch("gzip-split");
        let src = dir.join("src/x.fa");
        fs::write(&src, ">one\nA\n>two\nC\n").unwrap();
        let out = dir.join("out");
        export_files(&[src], &opts(Packaging::Gzip, true), &out, false).unwrap();
        assert_eq!(listing(&out), ["one.fa.gz", "two.fa.gz"]);
        assert_eq!(
            gunzip(&fs::read(out.join("two.fa.gz")).unwrap()),
            b">two\nC\n"
        );
    }

    #[test]
    fn zip_bundles_everything_into_one_archive() {
        let dir = scratch("zip");
        let plain = dir.join("src/a.gb");
        let (text, _) = genbank();
        fs::write(&plain, &text).unwrap();
        let packed = dir.join("src/b.fasta.gz");
        let packed_bytes = gzip(b">x\nAC\n");
        fs::write(&packed, &packed_bytes).unwrap();
        let archive = dir.join("out/export.zip");
        let got = export_files(
            &[plain, packed],
            &opts(Packaging::Zip, false),
            &archive,
            false,
        )
        .unwrap();
        assert_eq!(got, done(2, &archive));
        assert_eq!(listing(&dir.join("out")), ["export.zip"]);

        let mut zip = zip::ZipArchive::new(File::open(&archive).unwrap()).unwrap();
        let mut a = zip.by_name("a.gb").unwrap();
        assert_eq!(a.compression(), CompressionMethod::Deflated);
        let mut got = String::new();
        a.read_to_string(&mut got).unwrap();
        assert_eq!(got, text);
        drop(a);
        let mut b = zip.by_name("b.fasta.gz").unwrap();
        assert_eq!(b.compression(), CompressionMethod::Stored);
        let mut got = Vec::new();
        b.read_to_end(&mut got).unwrap();
        assert_eq!(got, packed_bytes);
    }

    #[test]
    fn zip_with_split_holds_one_entry_per_record() {
        let dir = scratch("zip-split");
        let src = dir.join("src/x.gbk");
        fs::write(&src, genbank().0).unwrap();
        let archive = dir.join("out/x.zip");
        export_files(&[src], &opts(Packaging::Zip, true), &archive, false).unwrap();
        let zip = zip::ZipArchive::new(File::open(&archive).unwrap()).unwrap();
        let mut names: Vec<&str> = zip.file_names().collect();
        names.sort_unstable();
        assert_eq!(names, ["REC1 (2).gbk", "a_b_c.gbk", "rec1.gbk"]);
    }

    #[test]
    fn existing_files_are_reported_and_kept_until_overwrite_is_confirmed() {
        let dir = scratch("conflicts");
        let src = dir.join("src/x.fa");
        fs::write(&src, ">one\nA\n>two\nC\n").unwrap();
        let out = dir.join("out");
        fs::write(out.join("two.fa"), "old").unwrap();
        let o = opts(Packaging::Plain, true);
        let got = export_files(std::slice::from_ref(&src), &o, &out, false).unwrap();
        assert_eq!(
            got,
            ExportOutcome::Conflicts {
                names: vec!["two.fa".into()]
            }
        );
        assert_eq!(listing(&out), ["two.fa"]);
        assert_eq!(fs::read_to_string(out.join("two.fa")).unwrap(), "old");

        export_files(&[src], &o, &out, true).unwrap();
        assert_eq!(listing(&out), ["one.fa", "two.fa"]);
        assert_eq!(fs::read_to_string(out.join("two.fa")).unwrap(), ">two\nC\n");
    }

    #[test]
    fn exporting_onto_a_source_is_refused() {
        let dir = scratch("onto-source");
        let src = dir.join("src/x.fa");
        fs::write(&src, ">x\nA\n").unwrap();
        let err = export_files(
            std::slice::from_ref(&src),
            &opts(Packaging::Plain, false),
            &dir.join("src"),
            true,
        )
        .unwrap_err();
        assert!(err.contains("would overwrite"), "{err}");
        // A split record named like its source is refused too.
        let err = export_files(
            std::slice::from_ref(&src),
            &opts(Packaging::Plain, true),
            &dir.join("src"),
            true,
        )
        .unwrap_err();
        assert!(err.contains("would overwrite"), "{err}");
        assert_eq!(fs::read_to_string(&src).unwrap(), ">x\nA\n");
    }

    #[test]
    fn a_failed_export_leaves_no_partial_file() {
        let dir = scratch("failed");
        let good = dir.join("src/a.fa");
        fs::write(&good, ">a\nA\n").unwrap();
        let bad = dir.join("src/b.fa");
        fs::write(&bad, ">b\nC\n").unwrap();
        // `b.fa` is written in full, then cannot be renamed onto a directory.
        let out = dir.join("out");
        fs::create_dir_all(out.join("b.fa")).unwrap();
        export_files(
            &[good, bad.clone()],
            &opts(Packaging::Plain, false),
            &out,
            true,
        )
        .unwrap_err();
        assert_eq!(listing(&out), ["a.fa", "b.fa"]);
        assert!(
            out.join("b.fa").is_dir(),
            "the blocking directory itself is untouched"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_zip_export_leaves_no_archive() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("failed-zip");
        let good = dir.join("src/a.fa");
        fs::write(&good, ">a\nA\n").unwrap();
        // Passes the up-front checks, then cannot be opened mid-archive.
        let unreadable = dir.join("src/b.txt");
        fs::write(&unreadable, "x").unwrap();
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).unwrap();
        if File::open(&unreadable).is_ok() {
            return; // running as root: permissions do not stop it
        }
        let archive = dir.join("out/x.zip");
        export_files(
            &[good, unreadable],
            &opts(Packaging::Zip, false),
            &archive,
            false,
        )
        .unwrap_err();
        assert!(listing(&dir.join("out")).is_empty());
    }

    #[test]
    fn names_are_split_before_the_compound_extension() {
        assert_eq!(split_name("foo.gb.gz"), ("foo", ".gb.gz"));
        assert_eq!(split_name("foo.bar.hmm"), ("foo.bar", ".hmm"));
        assert_eq!(split_name("README"), ("README", ""));
        assert_eq!(split_name("db.BGZF"), ("db", ".BGZF"));
        assert_eq!(format_ext("x.fasta.bgz"), ".fasta");
        let mut used = Names::default();
        assert_eq!(used.claim("a", ".gb.gz"), "a.gb.gz");
        assert_eq!(used.claim("A", ".gb.gz"), "A (2).gb.gz");
    }

    #[test]
    fn safe_file_stem_produces_names_every_file_system_accepts() {
        assert_eq!(safe_file_stem("NC_000913.3"), "NC_000913.3");
        assert_eq!(
            safe_file_stem(r#"a<b>c:d"e/f\g|h?i*j"#),
            "a_b_c_d_e_f_g_h_i_j"
        );
        assert_eq!(safe_file_stem("tab\there"), "tab_here");
        assert_eq!(safe_file_stem("trailing. . "), "trailing");
        assert_eq!(safe_file_stem(".hidden"), "_hidden");
        assert_eq!(safe_file_stem(".."), "record");
        assert_eq!(safe_file_stem(""), "record");
        assert_eq!(safe_file_stem("con"), "_con");
        assert_eq!(safe_file_stem("COM1.x"), "_COM1.x");
        assert_eq!(safe_file_stem("COM0"), "COM0");
        assert_eq!(safe_file_stem("console"), "console");
        let long = "é".repeat(150);
        let s = safe_file_stem(&long);
        assert!(s.len() <= 200 && s.chars().all(|c| c == 'é'));
    }
}
