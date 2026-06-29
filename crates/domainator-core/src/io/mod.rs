//! Sequence and database I/O: GenBank/FASTA reading and writing, BGZF random
//! access, offset scanning, `.didx` indexes, and shard resolution.

pub mod bgzf;
pub mod didx;
pub mod fasta;
pub mod genbank;
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
