//! Sequence and database I/O: GenBank/FASTA reading and writing, BGZF random
//! access, offset scanning, `.didx` indexes, and shard resolution.

pub mod alignment;
pub mod bgzf;
pub mod didx;
pub mod fasta;
pub mod genbank;
pub mod msa_formats;
pub mod newick;
pub mod partition;
pub mod python_json;
pub mod shards;
pub mod swissprot;

use std::path::Path;

/// Sequence file format, as detected from a filename extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    Genbank,
    Fasta,
}

/// Compression of a sequence file (plain gzip is readable but not seekable).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    None,
    Gzip,
    Bgzf,
}

/// Detect compression: `.bgz`/`.bgzf` are BGZF; a `.gz` file is probed for the
/// BGZF `BC` extra subfield (BGZF is a special gzip) and classified accordingly.
pub fn detect_compression(path: &Path) -> Compression {
    let name = match path.file_name().and_then(|s| s.to_str()) {
        Some(n) => n.to_ascii_lowercase(),
        None => return Compression::None,
    };
    if name.ends_with(".bgz") || name.ends_with(".bgzf") {
        return Compression::Bgzf;
    }
    if name.ends_with(".gz") {
        // Probe magic: gzip (1f 8b 08) with FEXTRA flag set and a BGZF BC subfield.
        // Only the fixed-size header is needed, not the whole file.
        if let Ok(bytes) = read_prefix(path, 18) {
            if bytes.len() >= 18
                && bytes[0] == 0x1f
                && bytes[1] == 0x8b
                && bytes[2] == 0x08
                && (bytes[3] & 0x04) != 0
                && bytes[12] == b'B'
                && bytes[13] == b'C'
            {
                return Compression::Bgzf;
            }
        }
        return Compression::Gzip;
    }
    Compression::None
}

/// Compression suffixes, mirroring `utils.COMPRESSION_EXTENSIONS`.
pub const COMPRESSION_EXTENSIONS: [&str; 3] = ["gz", "bgz", "bgzf"];

const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

/// Up to `n` bytes from the start of `path`.
fn read_prefix(path: &Path, n: usize) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut buf = Vec::with_capacity(n);
    std::fs::File::open(path)?
        .take(n as u64)
        .read_to_end(&mut buf)?;
    Ok(buf)
}

/// Whether the file *content* is gzip (plain or BGZF), judged by its magic bytes.
/// Mirrors `utils.detect_compression(path) is not None`; an unreadable path is
/// "not compressed", as it is there.
pub fn has_gzip_magic(path: &Path) -> bool {
    read_prefix(path, 2).is_ok_and(|b| b == GZIP_MAGIC)
}

/// Whether the path's *name* marks it compressed (`.gz`/`.bgz`/`.bgzf`, any case).
/// Mirrors `utils.is_compressed_path`.
pub fn is_compressed_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| COMPRESSION_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Open a file for sequential reading, decompressing gzip or BGZF by content.
///
/// The decision is the magic bytes, never the name, so a compressed file under
/// any name reads correctly, and so does an uncompressed file named `.gz`.
/// `MultiGzDecoder` rather than `GzDecoder`: BGZF is a chain of gzip members, and
/// a single-member decoder silently stops after the first 64 KiB block.
pub fn open_decompressed(path: &Path) -> std::io::Result<Box<dyn std::io::Read>> {
    let file = std::fs::File::open(path)?;
    if has_gzip_magic(path) {
        Ok(Box::new(flate2::read::MultiGzDecoder::new(
            std::io::BufReader::new(file),
        )))
    } else {
        Ok(Box::new(file))
    }
}

/// The database name annotations are labeled with, from a reference path.
///
/// Port of `utils.db_name_from_path`: strip one trailing compression suffix, then
/// the format suffix, so `refs.hmm` and `refs.hmm.gz` both give `refs`. Only one
/// format suffix goes, and only when something is left, so `some.long.name.hmm`
/// gives `some.long.name` and `db.gz` gives `db`.
pub fn db_name_from_path(path: &str) -> String {
    let base = Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(path);
    let stem = if is_compressed_path(Path::new(base)) {
        Path::new(base)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(base)
    } else {
        base
    };
    match Path::new(stem).file_stem().and_then(|s| s.to_str()) {
        Some(t) if !t.is_empty() => t.to_string(),
        _ => stem.to_string(),
    }
}

/// Detect the sequence format from a filename, ignoring any compression suffix.
/// Mirrors `utils.FILE_TYPE_ALIASES`.
pub fn detect_filetype(path: &Path) -> Option<FileType> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    let stripped = name
        .strip_suffix(".gz")
        .or_else(|| name.strip_suffix(".bgz"))
        .or_else(|| name.strip_suffix(".bgzf"))
        .unwrap_or(&name);
    let ext = stripped.rsplit('.').next()?;
    match ext {
        "gb" | "gbk" | "genbank" | "gbff" => Some(FileType::Genbank),
        "fasta" | "fa" | "faa" | "fna" | "pep" => Some(FileType::Fasta),
        _ => None,
    }
}

/// Whether `path` is an existing file that is neither a regular file nor a directory:
/// a character device (`/dev/null`, `/dev/stderr`), a FIFO (a shell's `>(...)`), or a
/// socket, and `NUL` on Windows. Such an output must be written directly: it cannot be
/// replaced by renaming a temporary file over it, and its directory (`/dev`) usually
/// cannot hold one.
pub fn is_special_file(path: &Path) -> bool {
    #[cfg(windows)]
    {
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        if path.extension().is_none() && stem.eq_ignore_ascii_case("nul") {
            return true;
        }
    }
    std::fs::metadata(path).is_ok_and(|m| {
        let kind = m.file_type();
        !kind.is_file() && !kind.is_dir()
    })
}

/// A unique temporary name beside `target` for writing it before a rename into place:
/// `.{name}.{pid}.tmp` in the same directory, so the rename never crosses filesystems,
/// and hidden behind a leading `.` that no shard or output pattern matches.
pub fn temp_path_beside(target: &Path) -> std::path::PathBuf {
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    target.with_file_name(format!(".{name}.{}.tmp", std::process::id()))
}

/// Remove the partial output a failed run left at `path`, but only a regular file:
/// a run that wrote to `/dev/null` or a FIFO must never delete it.
pub fn remove_partial_output(path: &Path) {
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file()) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn devices_are_special_files_and_never_removed() {
        assert!(is_special_file(Path::new("/dev/null")));
        let dir =
            std::env::temp_dir().join(format!("domainator_io_special_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let regular = dir.join("out.txt");
        std::fs::write(&regular, b"x").unwrap();
        assert!(!is_special_file(&regular));
        assert!(!is_special_file(&dir));
        assert!(!is_special_file(&dir.join("missing.txt")));
        remove_partial_output(Path::new("/dev/null"));
        assert!(Path::new("/dev/null").exists());
        remove_partial_output(&regular);
        assert!(!regular.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn db_name_strips_one_compression_then_one_format_suffix() {
        // The cases upstream's `test_db_name_from_path` pins.
        for (path, expected) in [
            ("refs.hmm", "refs"),
            ("refs.hmm.gz", "refs"),
            ("refs.hmm.bgz", "refs"),
            ("refs.HMM.GZ", "refs"),
            ("/some/dir/refs.hmm.gz", "refs"),
            ("some.long.name.hmm", "some.long.name"),
            ("some.long.name.hmm.gz", "some.long.name"),
            ("refs.v1", "refs"),
            ("db.gz", "db"),
            ("refs", "refs"),
        ] {
            assert_eq!(db_name_from_path(path), expected, "{path}");
        }
    }

    #[test]
    fn compression_is_judged_by_content_not_name() {
        let dir = std::env::temp_dir().join(format!("domainator_io_magic_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let plain_named_gz = dir.join("plain.hmm.gz");
        std::fs::write(&plain_named_gz, b"HMMER3/f\n").unwrap();
        let gz_named_plain = dir.join("packed.hmm");
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut enc, b"HMMER3/f\n").unwrap();
        std::fs::write(&gz_named_plain, enc.finish().unwrap()).unwrap();

        assert!(!has_gzip_magic(&plain_named_gz));
        assert!(has_gzip_magic(&gz_named_plain));
        for p in [&plain_named_gz, &gz_named_plain] {
            let mut text = String::new();
            std::io::Read::read_to_string(&mut open_decompressed(p).unwrap(), &mut text).unwrap();
            assert_eq!(text, "HMMER3/f\n");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Upstream's `parse_seqfiles` reads plain, gzip and BGZF sequence files alike.
    /// The whole-file readers used to open every file as plain text, so a compressed
    /// input parsed as zero records and the tool succeeded with empty output.
    #[test]
    fn sequence_files_read_the_same_plain_gzip_or_bgzf() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("seq_compression_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let genbank = "LOCUS       p1 6 aa                     UNK 01-JAN-2000\n\
                       DEFINITION  one.\nFEATURES             Location/Qualifiers\n\
                       ORIGIN\n        1 acdefg\n//\n\
                       LOCUS       p2 3 aa                     UNK 01-JAN-2000\n\
                       DEFINITION  two.\nFEATURES             Location/Qualifiers\n\
                       ORIGIN\n        1 klm\n//\n";
        let fasta = ">p1 one\nACDEFG\n>p2 two\nKLM\n";
        for (ext, text) in [("gb", genbank), ("fasta", fasta)] {
            let plain = dir.join(format!("in.{ext}"));
            std::fs::write(&plain, text).unwrap();
            let gzip = dir.join(format!("in.{ext}.gz"));
            let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
            gz.write_all(text.as_bytes()).unwrap();
            std::fs::write(&gzip, gz.finish().unwrap()).unwrap();
            let bgzf_path = dir.join(format!("in.{ext}.bgz"));
            let level = ::bgzf::CompressionLevel::new(6).unwrap();
            let mut w = ::bgzf::Writer::new(std::fs::File::create(&bgzf_path).unwrap(), level);
            w.write_all(text.as_bytes()).unwrap();
            w.finish().unwrap();
            let read = |p: &Path| {
                let p = p.to_str().unwrap();
                let records = if ext == "gb" {
                    genbank::reader::read_genbank(p).unwrap()
                } else {
                    fasta::read_fasta(p, None).unwrap()
                };
                records
                    .into_iter()
                    .map(|r| (r.id, r.seq))
                    .collect::<Vec<_>>()
            };
            let expected = read(&plain);
            assert_eq!(expected.len(), 2, "{ext}");
            assert_eq!(read(&gzip), expected, "{ext}.gz");
            assert_eq!(read(&bgzf_path), expected, "{ext}.bgz");
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
