//! Partitioning a (possibly sharded/compressed) database into seek-and-read
//! chunks for parallel search (port of `partition_seqfile` + `utils.get_offsets`).

use std::io;
use std::path::Path;

use super::didx;
use super::genbank::offsets::{fasta_offsets, genbank_offsets};
use super::{Compression, FileType};

/// A unit of work: read `n_records` records from `path` starting at `offset`
/// (a byte offset, or a BGZF virtual offset when `compressed`).
#[derive(Debug, Clone)]
pub struct Partition {
    pub path: String,
    pub offset: u64,
    pub n_records: u64,
    pub filetype: FileType,
    pub compressed: bool,
}

/// Per-record `(offset, cds_count)` for `path`: a fresh `.didx` if present, else a
/// scan. Mirrors `utils.get_offsets`.
pub fn get_offsets(
    path: &str,
    filetype: FileType,
    compression: Compression,
) -> io::Result<(Vec<u64>, Vec<u64>)> {
    if let Some((offsets, cds)) = didx::read_index(Path::new(path), filetype, compression)? {
        return Ok((offsets, cds));
    }
    let pairs = match (filetype, compression) {
        (FileType::Genbank, Compression::None) => genbank_offsets(path)?,
        (FileType::Fasta, Compression::None) => fasta_offsets(path)?,
        (_, Compression::Bgzf) => super::bgzf::scan_offsets(path, filetype)?,
        (_, Compression::Gzip) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "plain gzip is not seekable; recompress as BGZF or decompress",
            ))
        }
    };
    let offsets = pairs.iter().map(|p| p.0).collect();
    let cds = pairs.iter().map(|p| p.1).collect();
    Ok((offsets, cds))
}

/// Greedily cut `path` into partitions of ~`cdss_per_partition` CDSs (port of
/// `partition_seqfile`). Returns `(total_cds, partitions)`.
pub fn partition_seqfile(
    path: &str,
    filetype: FileType,
    compression: Compression,
    cdss_per_partition: u64,
) -> io::Result<(u64, Vec<Partition>)> {
    let (offsets, cds_counts) = get_offsets(path, filetype, compression)?;
    let total: u64 = cds_counts.iter().sum();
    let compressed = compression == Compression::Bgzf;
    let mut out = Vec::new();
    let mut running = 0u64;
    let mut next_offset = offsets.first().copied().unwrap_or(0);
    let mut recs_in_buffer = 0u64;
    for i in 0..offsets.len() {
        running += cds_counts[i];
        recs_in_buffer += 1;
        if running >= cdss_per_partition {
            out.push(Partition { path: path.to_string(), offset: next_offset, n_records: recs_in_buffer, filetype, compressed });
            running = 0;
            recs_in_buffer = 0;
            if i + 1 < offsets.len() {
                next_offset = offsets[i + 1];
            }
        }
    }
    if recs_in_buffer > 0 {
        out.push(Partition { path: path.to_string(), offset: next_offset, n_records: recs_in_buffer, filetype, compressed });
    }
    Ok((total, out))
}
