//! Record offset scanners (lifted from the prototype `lib.rs`).
//!
//! Equivalent to `utils.get_genbank_offsets` / `get_fasta_offsets`: returns
//! `(byte_offset, cds_count)` per record. `cds_count` is 1 for amino-acid (`aa`)
//! GenBank records and the number of `     CDS ` feature lines for nucleotide
//! (`bp`) records; always 1 for FASTA. These pairs feed the `.didx` index and the
//! partition scheme, so they must match the Python scanner exactly.

use std::fs::File;
use std::io::{self, BufRead, BufReader};

/// 4th whitespace-delimited token of a LOCUS line (the molecule-unit field
/// `aa`/`bp`), matching the Python scanner's `parts[3]`.
pub(crate) fn locus_unit_is_aa(line: &[u8]) -> bool {
    let mut tokens = line
        .split(|&b| b == b' ' || b == b'\t' || b == b'\r' || b == b'\n')
        .filter(|t| !t.is_empty());
    matches!(tokens.nth(3), Some(b"aa"))
}

/// Scan an (uncompressed) GenBank file, returning `(byte_offset, cds_count)` per record.
pub fn genbank_offsets(path: &str) -> io::Result<Vec<(u64, u64)>> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut out: Vec<(u64, u64)> = Vec::new();

    let mut offset: u64 = 0;
    let mut last_offset: u64 = 0;
    let mut cdss: u64 = 0;
    let mut is_aa = false;
    let mut locus_seen = false;
    let mut line: Vec<u8> = Vec::new();

    // Advance to the first LOCUS line.
    loop {
        line.clear();
        let n = reader.read_until(b'\n', &mut line)?;
        if n == 0 {
            break;
        }
        if line.starts_with(b"LOCUS ") {
            locus_seen = true;
            is_aa = locus_unit_is_aa(&line);
            cdss = if is_aa { 1 } else { 0 };
            last_offset = offset;
            offset += n as u64;
            break;
        }
        offset += n as u64;
    }

    // Remaining records.
    loop {
        line.clear();
        let n = reader.read_until(b'\n', &mut line)?;
        if n == 0 {
            break;
        }
        if line.starts_with(b"LOCUS ") {
            out.push((last_offset, cdss));
            is_aa = locus_unit_is_aa(&line);
            cdss = if is_aa { 1 } else { 0 };
            last_offset = offset;
        } else if line.starts_with(b"     CDS ") && !is_aa {
            cdss += 1;
        }
        offset += n as u64;
    }

    if locus_seen {
        out.push((last_offset, cdss));
    }
    Ok(out)
}

/// Scan an (uncompressed) FASTA file, returning `(byte_offset, 1)` per record.
pub fn fasta_offsets(path: &str) -> io::Result<Vec<(u64, u64)>> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut out: Vec<(u64, u64)> = Vec::new();

    let mut offset: u64 = 0;
    let mut line: Vec<u8> = Vec::new();
    loop {
        line.clear();
        let n = reader.read_until(b'\n', &mut line)?;
        if n == 0 {
            break;
        }
        if line.first() == Some(&b'>') {
            out.push((offset, 1));
        }
        offset += n as u64;
    }
    Ok(out)
}
