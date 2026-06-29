//! BGZF random access (lifted from the prototype `parser.rs::open_at`).

use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};

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
