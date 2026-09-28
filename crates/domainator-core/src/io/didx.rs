//! `.didx` record-offset indexes — a byte-compatible port of `db_index.py`.
//!
//! Little-endian, 56-byte header followed by `record_count` `(offset, cds_count)`
//! `u64` pairs. A reader NEVER trusts a stale/incompatible index (a wrong offset
//! would seek mid-record); on any mismatch it reports a miss so the caller rescans.

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use super::{Compression, FileType};

const MAGIC: &[u8; 4] = b"DIDX";
const VER_MAJOR: u16 = 1;
const VER_MINOR: u16 = 0;
pub const HEADER_SIZE: usize = 56;
const PAIR_SIZE: usize = 16;
pub const INDEX_EXTENSION: &str = ".didx";

fn filetype_flag(ft: FileType) -> u8 {
    match ft {
        FileType::Genbank => 0,
        FileType::Fasta => 1,
    }
}

/// Map compression to the on-disk flag. Plain gzip has no seekable offset space.
fn compression_flag(c: Compression) -> io::Result<u8> {
    match c {
        Compression::None => Ok(0),
        Compression::Bgzf => Ok(1),
        Compression::Gzip => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "cannot build a .didx for plain-gzip input; use uncompressed or BGZF",
        )),
    }
}

/// The `.didx` sidecar path for a database file/shard (append to the full name).
pub fn index_path_for(source: &Path) -> PathBuf {
    let mut s = source.as_os_str().to_os_string();
    s.push(INDEX_EXTENSION);
    PathBuf::from(s)
}

/// `(size, mtime_ns)` staleness fingerprint, matching Python `os.stat`.
pub fn source_fingerprint(source: &Path) -> io::Result<(u64, u64)> {
    let md = std::fs::metadata(source)?;
    let mtime_ns = md
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map_err(|e| io::Error::other(e.to_string()))?
        .as_nanos() as u64;
    Ok((md.len(), mtime_ns))
}

/// Write a `.didx` for `source` from parallel `offsets`/`cds_counts`. Returns the
/// index path. Byte-identical to `db_index.write_index`.
pub fn write_index(
    source: &Path,
    offsets: &[u64],
    cds_counts: &[u64],
    filetype: FileType,
    compression: Compression,
) -> io::Result<PathBuf> {
    assert_eq!(
        offsets.len(),
        cds_counts.len(),
        "offsets and cds_counts length mismatch"
    );
    let comp_flag = compression_flag(compression)?;
    let (size, mtime_ns) = source_fingerprint(source)?;
    let n = offsets.len() as u64;
    let total_cds: u64 = cds_counts.iter().sum();

    let mut buf: Vec<u8> = Vec::with_capacity(HEADER_SIZE + offsets.len() * PAIR_SIZE);
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&VER_MAJOR.to_le_bytes());
    buf.extend_from_slice(&VER_MINOR.to_le_bytes());
    buf.push(filetype_flag(filetype));
    buf.push(comp_flag);
    buf.extend_from_slice(&[0u8; 6]); // reserved pad (8-byte alignment)
    buf.extend_from_slice(&size.to_le_bytes());
    buf.extend_from_slice(&mtime_ns.to_le_bytes());
    buf.extend_from_slice(&n.to_le_bytes());
    buf.extend_from_slice(&total_cds.to_le_bytes());
    buf.extend_from_slice(&0u64.to_le_bytes()); // flags
    debug_assert_eq!(buf.len(), HEADER_SIZE);
    for (off, cds) in offsets.iter().zip(cds_counts) {
        buf.extend_from_slice(&off.to_le_bytes());
        buf.extend_from_slice(&cds.to_le_bytes());
    }

    let path = index_path_for(source);
    File::create(&path)?.write_all(&buf)?;
    Ok(path)
}

/// A validated, fresh index header.
struct Header {
    record_count: u64,
    total_cds: u64,
}

fn validate_header(
    index_path: &Path,
    source: &Path,
    filetype: FileType,
    compression: Compression,
) -> io::Result<Option<(File, Header)>> {
    let mut f = match File::open(index_path) {
        Ok(f) => f,
        Err(_) => return Ok(None), // no index -> silent miss
    };
    let mut hdr = [0u8; HEADER_SIZE];
    if f.read_exact(&mut hdr).is_err() {
        return Ok(None); // truncated
    }
    if &hdr[0..4] != MAGIC {
        return Ok(None);
    }
    let ver_major = u16::from_le_bytes([hdr[4], hdr[5]]);
    if ver_major != VER_MAJOR {
        return Ok(None);
    }
    let ft_flag = hdr[8];
    let comp_flag = hdr[9];
    if ft_flag != filetype_flag(filetype) || comp_flag != compression_flag(compression)? {
        return Ok(None);
    }
    let size = u64::from_le_bytes(hdr[16..24].try_into().unwrap());
    let mtime = u64::from_le_bytes(hdr[24..32].try_into().unwrap());
    let n = u64::from_le_bytes(hdr[32..40].try_into().unwrap());
    let total_cds = u64::from_le_bytes(hdr[40..48].try_into().unwrap());

    // Exact-size guard.
    let expected = HEADER_SIZE as u64 + n * PAIR_SIZE as u64;
    if f.metadata()?.len() != expected {
        return Ok(None);
    }
    // Staleness: source must be unchanged.
    let cur = match source_fingerprint(source) {
        Ok(fp) => fp,
        Err(_) => return Ok(None),
    };
    if cur != (size, mtime) {
        return Ok(None);
    }
    Ok(Some((
        f,
        Header {
            record_count: n,
            total_cds,
        },
    )))
}

/// Read `(offsets, cds_counts)` from a valid, fresh `.didx`, or `None` to signal
/// the caller to scan.
pub fn read_index(
    source: &Path,
    filetype: FileType,
    compression: Compression,
) -> io::Result<Option<(Vec<u64>, Vec<u64>)>> {
    let index_path = index_path_for(source);
    let (mut f, hdr) = match validate_header(&index_path, source, filetype, compression)? {
        Some(v) => v,
        None => return Ok(None),
    };
    let mut body = vec![0u8; hdr.record_count as usize * PAIR_SIZE];
    f.read_exact(&mut body)?;
    let mut offsets = Vec::with_capacity(hdr.record_count as usize);
    let mut cds = Vec::with_capacity(hdr.record_count as usize);
    for pair in body.as_chunks::<PAIR_SIZE>().0 {
        offsets.push(u64::from_le_bytes(pair[0..8].try_into().unwrap()));
        cds.push(u64::from_le_bytes(pair[8..16].try_into().unwrap()));
    }
    Ok(Some((offsets, cds)))
}

/// Total CDS/protein count from a valid, fresh `.didx` header (O(1)), or `None`.
/// Used by `domain_search -Z 0` to read the target count without scanning.
pub fn read_total_cds(
    source: &Path,
    filetype: FileType,
    compression: Compression,
) -> io::Result<Option<u64>> {
    let index_path = index_path_for(source);
    Ok(validate_header(&index_path, source, filetype, compression)?.map(|(_, h)| h.total_cds))
}
