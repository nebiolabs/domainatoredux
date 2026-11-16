//! BGZF random access (lifted from the prototype `parser.rs::open_at`).

use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};

use rayon::prelude::*;

use super::genbank::offsets::locus_unit_is_aa;
use super::FileType;

/// Blocks compressed per parallel batch: 256 x 64 KiB bounds a writer's buffer at
/// ~16 MiB of input (plus its compressed copy) while keeping every thread busy.
const COMPRESS_BATCH_BLOCKS: usize = 256;

/// Compress everything `reader` yields into `writer` as BGZF, on the current rayon
/// pool, and write the EOF block. See [`ParallelWriter`].
pub fn compress_parallel<R: Read, W: Write>(
    mut reader: R,
    writer: W,
    level: ::bgzf::CompressionLevel,
) -> io::Result<W> {
    let mut w = ParallelWriter::new(writer, level);
    io::copy(&mut reader, &mut w)?;
    w.finish()
}

/// A BGZF writer that compresses its blocks in parallel on the current rayon pool
/// (the one `--cpu` sized).
///
/// It writes exactly the bytes `bgzf::Writer` writes for the same sequence of
/// `write` and `flush` calls, whatever the thread count: blocks are cut at the same
/// `BGZF_BLOCK_SIZE` boundaries counted from the last flush, a flush ends the
/// current block short, each block is an independent gzip member, and compressed
/// blocks are written back in order. Full blocks are held until a batch of 256
/// (~16 MiB) has built up, so output reaches `inner` in bursts.
///
/// The batch is compressed on the calling thread's pool, and the call waits for
/// it. That is safe as long as no pool worker blocks waiting on the thread that
/// writes, which no tool does: outputs are written on the main thread or inside the
/// pool's own `install`, where the waiting thread runs batch jobs itself.
///
/// Like `bgzf::Writer`, dropping it unfinished compresses what is buffered and
/// writes the EOF block, ignoring errors; call [`ParallelWriter::finish`] to see them.
pub struct ParallelWriter<W: Write> {
    inner: Option<W>,
    level: ::bgzf::CompressionLevel,
    /// Uncompressed bytes not yet in a written block.
    pending: Vec<u8>,
    /// Pending bytes that trigger compressing the full blocks among them.
    batch_bytes: usize,
}

impl<W: Write> ParallelWriter<W> {
    pub fn new(inner: W, level: ::bgzf::CompressionLevel) -> Self {
        Self::with_batch_blocks(inner, level, COMPRESS_BATCH_BLOCKS)
    }

    fn with_batch_blocks(inner: W, level: ::bgzf::CompressionLevel, blocks: usize) -> Self {
        ParallelWriter {
            inner: Some(inner),
            level,
            pending: Vec::new(),
            batch_bytes: blocks.max(1) * ::bgzf::BGZF_BLOCK_SIZE,
        }
    }

    /// Compress and write the first `len` pending bytes as blocks of
    /// `BGZF_BLOCK_SIZE` (the last one shorter when `len` is not a multiple).
    fn write_blocks(&mut self, len: usize) -> io::Result<()> {
        if len == 0 {
            return Ok(());
        }
        let inner = self
            .inner
            .as_mut()
            .ok_or_else(|| io::Error::other("BGZF writer already finished"))?;
        let level = self.level;
        let blocks: Vec<Vec<u8>> = self.pending[..len]
            .par_chunks(::bgzf::BGZF_BLOCK_SIZE)
            .map_init(
                || ::bgzf::Compressor::new(level),
                |compressor, chunk| {
                    let mut out = Vec::with_capacity(chunk.len() / 2);
                    compressor
                        .compress(chunk, &mut out)
                        .map(|()| out)
                        .map_err(io::Error::other)
                },
            )
            .collect::<io::Result<_>>()?;
        for compressed in &blocks {
            inner.write_all(compressed)?;
        }
        self.pending.drain(..len);
        Ok(())
    }

    /// Write what is buffered and the EOF block, and return the inner writer,
    /// flushed.
    pub fn finish(mut self) -> io::Result<W> {
        self.write_blocks(self.pending.len())?;
        let mut inner = self.inner.take().expect("checked by write_blocks");
        let mut eof = Vec::new();
        ::bgzf::Compressor::append_eof(&mut eof);
        inner.write_all(&eof)?;
        inner.flush()?;
        Ok(inner)
    }
}

impl<W: Write> Write for ParallelWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.inner.is_none() {
            return Err(io::Error::other("BGZF writer already finished"));
        }
        self.pending.extend_from_slice(buf);
        if self.pending.len() >= self.batch_bytes {
            let full = self.pending.len() / ::bgzf::BGZF_BLOCK_SIZE * ::bgzf::BGZF_BLOCK_SIZE;
            self.write_blocks(full)?;
        }
        Ok(buf.len())
    }

    /// Write every buffered byte, ending the current block short (as
    /// `bgzf::Writer::flush` does), and flush the inner writer. No EOF block.
    fn flush(&mut self) -> io::Result<()> {
        self.write_blocks(self.pending.len())?;
        match self.inner.as_mut() {
            Some(inner) => inner.flush(),
            None => Ok(()),
        }
    }
}

impl<W: Write> Drop for ParallelWriter<W> {
    fn drop(&mut self) {
        if self.inner.is_some() {
            let _ = self.write_blocks(self.pending.len());
            if let Some(inner) = self.inner.as_mut() {
                let mut eof = Vec::new();
                ::bgzf::Compressor::append_eof(&mut eof);
                let _ = inner.write_all(&eof);
                let _ = inner.flush();
            }
        }
    }
}

/// Open `path` and position a reader at `seek_to`, decompressing BGZF on the fly.
///
/// For uncompressed input, `seek_to` is a plain byte offset. For BGZF, `seek_to`
/// is a 64-bit virtual offset (`block_start << 16 | within_block`, the encoding
/// `Bio.bgzf` produces): we seek the file to the block boundary (each BGZF block
/// is an independent gzip member), decompress from there, and discard the
/// within-block bytes. The returned reader yields the decompressed stream from the
/// record boundary onward.
pub fn open_at(path: &str, seek_to: u64, compressed: bool) -> io::Result<Box<dyn Read + Send>> {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Text-like input that compresses but is not one repeated byte.
    fn sample(len: usize) -> Vec<u8> {
        let mut state = 0x2545_f491_u32;
        (0..len)
            .map(|i| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                if i % 61 == 60 {
                    b'\n'
                } else {
                    b"ACGT"[(state % 4) as usize]
                }
            })
            .collect()
    }

    fn serial(input: &[u8], level: ::bgzf::CompressionLevel) -> Vec<u8> {
        let mut w = ::bgzf::Writer::new(Vec::new(), level);
        w.write_all(input).unwrap();
        w.finish().unwrap()
    }

    /// Writes `input` in pieces of `piece` bytes, flushing after every
    /// `flush_every` pieces, through either writer.
    fn drive<W: Write>(w: &mut W, input: &[u8], piece: usize, flush_every: usize) {
        for (i, chunk) in input.chunks(piece).enumerate() {
            w.write_all(chunk).unwrap();
            if flush_every > 0 && i % flush_every == flush_every - 1 {
                w.flush().unwrap();
            }
        }
    }

    /// The parallel writer writes exactly what `bgzf::Writer` writes for the same
    /// writes and flushes, at any thread count, including inputs that end on a block
    /// or batch boundary and flushes that end a block early.
    #[test]
    fn parallel_writer_matches_the_streaming_writer() {
        let block = ::bgzf::BGZF_BLOCK_SIZE;
        let level = ::bgzf::CompressionLevel::new(6).unwrap();
        let cases = [
            (0, 1000, 0),
            (1, 1, 0),
            (block - 1, 4096, 0),
            (block, block, 0),
            (block + 1, 8192, 0),
            (3 * block, 3 * block, 0),
            (7 * block + 123, 10_000, 0),
            (7 * block + 123, 10_000, 9),
            (5 * block, 70_000, 1),
        ];
        for (len, piece, flush_every) in cases {
            let input = sample(len);
            let mut serial = ::bgzf::Writer::new(Vec::new(), level);
            drive(&mut serial, &input, piece, flush_every);
            let expected = serial.finish().unwrap();
            for threads in [1, 4] {
                let pool = rayon::ThreadPoolBuilder::new()
                    .num_threads(threads)
                    .build()
                    .unwrap();
                let got = pool.install(|| {
                    let mut w = ParallelWriter::with_batch_blocks(Vec::new(), level, 3);
                    drive(&mut w, &input, piece, flush_every);
                    w.finish().unwrap()
                });
                let case =
                    format!("len {len}, piece {piece}, flush {flush_every}, {threads} threads");
                assert!(got == expected, "{case}");
                let mut back = Vec::new();
                ::bgzf::Reader::new(got.as_slice())
                    .read_to_end(&mut back)
                    .unwrap();
                assert!(back == input, "{case}: round trip");
            }
        }
        let input = sample(2 * block + 5);
        let got = compress_parallel(input.as_slice(), Vec::new(), level).unwrap();
        assert!(got == serial(&input, level), "compress_parallel");
    }

    /// Dropped unfinished, it still writes what it buffered and the EOF block, as
    /// `bgzf::Writer` does.
    #[test]
    fn dropping_an_unfinished_writer_ends_the_stream() {
        let level = ::bgzf::CompressionLevel::new(6).unwrap();
        let input = sample(100_000);
        let mut got = Vec::new();
        {
            let mut w = ParallelWriter::new(&mut got, level);
            w.write_all(&input).unwrap();
        }
        let mut expected = Vec::new();
        {
            let mut w = ::bgzf::Writer::new(&mut expected, level);
            w.write_all(&input).unwrap();
        }
        assert!(got == expected);
    }
}
