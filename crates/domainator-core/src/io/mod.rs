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
        if let Ok(bytes) = std::fs::read(path) {
            if bytes.len() >= 18
                && bytes[0] == 0x1f && bytes[1] == 0x8b && bytes[2] == 0x08
                && (bytes[3] & 0x04) != 0
                && bytes[12] == b'B' && bytes[13] == b'C'
            {
                return Compression::Bgzf;
            }
        }
        return Compression::Gzip;
    }
    Compression::None
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
