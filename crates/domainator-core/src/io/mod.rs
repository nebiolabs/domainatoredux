//! Sequence and database I/O: GenBank/FASTA reading and writing, BGZF random
//! access, offset scanning, `.didx` indexes, and shard resolution.

pub mod bgzf;
pub mod didx;
pub mod fasta;
pub mod genbank;
pub mod partition;
pub mod shards;

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
        "gb" | "gbk" | "genbank" => Some(FileType::Genbank),
        "fasta" | "fa" | "faa" | "fna" => Some(FileType::Fasta),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
