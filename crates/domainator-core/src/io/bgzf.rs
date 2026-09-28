//! BGZF random access (lifted from the prototype `parser.rs::open_at`).

use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};

use super::genbank::offsets::locus_unit_is_aa;
use super::FileType;

/// Open `path` and position a reader at `seek_to`, decompressing BGZF on the fly.
///
/// For uncompressed input, `seek_to` is a plain byte offset. For BGZF, `seek_to`
/// is a 64-bit virtual offset (`block_start << 16 | within_block`, the encoding
/// `Bio.bgzf` produces): we seek the file to the block boundary (each BGZF block
/// is an independent gzip member), decompress from there, and discard the
/// within-block bytes. The returned reader yields the decompressed stream from the
/// record boundary onward.
pub fn open_at(path: &str, seek_to: u64, compressed: bool) -> io::Result<Box<dyn Read>> {
    let mut file = File::open(path)?;
    if !compressed {
        if seek_to > 0 {
            file.seek(SeekFrom::Start(seek_to))?;
        }
        return Ok(Box::new(BufReader::new(file)));
    }
    let block_start = seek_to >> 16;
    let within = (seek_to & 0xFFFF) as usize;
    file.seek(SeekFrom::Start(block_start))?;
    let mut reader = bgzf::Reader::new(file);
    let mut remaining = within;
    let mut buf = [0u8; 8192];
    while remaining > 0 {
        let want = remaining.min(buf.len());
        let n = reader.read(&mut buf[..want])?;
        if n == 0 {
            break;
        }
        remaining -= n;
    }
    Ok(Box::new(reader))
}

// --- BGZF offset scanning (ported from the prototype bgzf_scan.rs) -----------

const PREFIX_CAP: usize = 256;

fn read_full<R: Read>(r: &mut R, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match r.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

/// BGZF `BC` subfield BSIZE (total block size minus one).
fn bc_bsize(extra: &[u8]) -> Option<u16> {
    let mut i = 0;
    while i + 4 <= extra.len() {
        let (si1, si2) = (extra[i], extra[i + 1]);
        let slen = u16::from_le_bytes([extra[i + 2], extra[i + 3]]) as usize;
        if si1 == 66 && si2 == 67 && slen >= 2 && i + 6 <= extra.len() {
            return Some(u16::from_le_bytes([extra[i + 4], extra[i + 5]]));
        }
        i += 4 + slen;
    }
    None
}

fn for_each_block<F: FnMut(u64, &[u8])>(path: &str, mut f: F) -> io::Result<()> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut dec = libdeflater::Decompressor::new();
    let mut block_start: u64 = 0;
    let mut header = [0u8; 12];
    let mut block: Vec<u8> = Vec::new();
    let mut out: Vec<u8> = Vec::new();
    let err = |m: &str| io::Error::new(io::ErrorKind::InvalidData, m.to_string());

    loop {
        match read_full(&mut reader, &mut header)? {
            0 => break,
            12 => {}
            _ => return Err(err("truncated BGZF header")),
        }
        if header[0] != 0x1f || header[1] != 0x8b || header[2] != 8 || (header[3] & 0x04) == 0 {
            return Err(err("not a BGZF block (gzip magic or FEXTRA flag missing)"));
        }
        let xlen = u16::from_le_bytes([header[10], header[11]]) as usize;
        let mut extra = vec![0u8; xlen];
        if read_full(&mut reader, &mut extra)? != xlen {
            return Err(err("truncated BGZF extra field"));
        }
        let bsize = bc_bsize(&extra).ok_or_else(|| err("BGZF block missing BC subfield"))?;
        let block_size = bsize as usize + 1;
        let consumed = 12 + xlen;
        if block_size < consumed + 8 {
            return Err(err("BGZF block size too small"));
        }
        block.clear();
        block.extend_from_slice(&header);
        block.extend_from_slice(&extra);
        let body = block_size - consumed;
        let base = block.len();
        block.resize(base + body, 0);
        if read_full(&mut reader, &mut block[base..])? != body {
            return Err(err("truncated BGZF block body"));
        }
        let isize_dec = u32::from_le_bytes([
            block[block_size - 4],
            block[block_size - 3],
            block[block_size - 2],
            block[block_size - 1],
        ]) as usize;
        if isize_dec == 0 {
            block_start += block_size as u64;
            continue;
        }
        out.clear();
        out.resize(isize_dec, 0);
        dec.gzip_decompress(&block, &mut out)
            .map_err(|e| err(&format!("BGZF decompress failed: {e:?}")))?;
        f(block_start, &out);
        block_start += block_size as u64;
    }
    Ok(())
}

#[derive(Default)]
struct GbScan {
    out: Vec<(u64, u64)>,
    last_offset: u64,
    cdss: u64,
    is_aa: bool,
    locus_seen: bool,
    line_pending: bool,
    line_voffset: u64,
    prefix: Vec<u8>,
}

impl GbScan {
    fn feed(&mut self, block_start: u64, data: &[u8]) {
        for (i, &b) in data.iter().enumerate() {
            if !self.line_pending {
                self.line_voffset = (block_start << 16) | (i as u64);
                self.prefix.clear();
                self.line_pending = true;
            }
            if b == b'\n' {
                self.classify();
                self.line_pending = false;
            } else if self.prefix.len() < PREFIX_CAP {
                self.prefix.push(b);
            }
        }
    }
    fn classify(&mut self) {
        if self.prefix.starts_with(b"LOCUS ") {
            if self.locus_seen {
                self.out.push((self.last_offset, self.cdss));
            }
            self.locus_seen = true;
            self.is_aa = locus_unit_is_aa(&self.prefix);
            self.cdss = if self.is_aa { 1 } else { 0 };
            self.last_offset = self.line_voffset;
        } else if self.prefix.starts_with(b"     CDS ") && !self.is_aa {
            self.cdss += 1;
        }
    }
    fn finish(mut self) -> Vec<(u64, u64)> {
        if self.line_pending {
            self.classify();
        }
        if self.locus_seen {
            self.out.push((self.last_offset, self.cdss));
        }
        self.out
    }
}

#[derive(Default)]
struct FaScan {
    out: Vec<(u64, u64)>,
    line_pending: bool,
}

impl FaScan {
    fn feed(&mut self, block_start: u64, data: &[u8]) {
        for (i, &b) in data.iter().enumerate() {
            if !self.line_pending {
                self.line_pending = true;
                if b == b'>' {
                    self.out.push(((block_start << 16) | (i as u64), 1));
                }
            }
            if b == b'\n' {
                self.line_pending = false;
            }
        }
    }
}

/// Scan a BGZF file, returning `(virtual_offset, cds_count)` per record (virtual
/// offsets are `block_start << 16 | within_block`, consumed by [`open_at`]).
pub fn scan_offsets(path: &str, filetype: FileType) -> io::Result<Vec<(u64, u64)>> {
    match filetype {
        FileType::Genbank => {
            let mut scan = GbScan::default();
            for_each_block(path, |bs, data| scan.feed(bs, data))?;
            Ok(scan.finish())
        }
        FileType::Fasta => {
            let mut scan = FaScan::default();
            for_each_block(path, |bs, data| scan.feed(bs, data))?;
            Ok(scan.out)
        }
    }
}
