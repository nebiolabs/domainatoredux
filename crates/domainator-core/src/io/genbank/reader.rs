//! GenBank reader: frames `LOCUS`..`//` records and parses them with our own
//! parser (see [`super::parse`]). No gb-io dependency, so the reader is robust to
//! inputs gb-io diverges on (adversarial qualifiers, `*` stop codons, protein
//! `aa` LOCUS layouts).
//!
//! Field mapping: `id` = LOCUS, `name` = VERSION (or ACCESSION) for round-tripping.
//! Sequences are upper-cased (GenBank).

use std::io::{BufRead, BufReader, Read};

use crate::io::bgzf::open_at;
use crate::seq::SeqRecord;

use super::parse::{parse_record, record_identity};

/// One framed GenBank record: its header lines (LOCUS .. just before ORIGIN) and
/// the residues extracted directly from the ORIGIN block (case preserved, digits
/// and whitespace stripped), so any residue survives regardless of validators.
struct RecordBlock {
    header_lines: Vec<String>,
    residues: Vec<u8>,
    had_origin: bool,
}

/// Streaming GenBank reader over a (possibly BGZF) file from a record offset.
pub struct GenbankReader {
    lines: BufReader<Box<dyn Read>>,
    default_molecule_type: Option<String>,
}

impl GenbankReader {
    /// Open `path` at `seek_to` (byte offset, or BGZF virtual offset when
    /// `compressed`). `default_molecule_type` applies when a record's LOCUS lacks one.
    pub fn open(
        path: &str,
        seek_to: u64,
        compressed: bool,
        default_molecule_type: Option<String>,
    ) -> std::io::Result<Self> {
        Ok(GenbankReader {
            lines: BufReader::new(open_at(path, seek_to, compressed)?),
            default_molecule_type,
        })
    }

    /// Read the next `LOCUS`..`//` block, or `None` at EOF.
    fn next_block(&mut self) -> std::io::Result<Option<RecordBlock>> {
        let mut header_lines: Vec<String> = Vec::new();
        let mut residues: Vec<u8> = Vec::new();
        let mut had_origin = false;
        let mut in_origin = false;
        let mut started = false;
        let mut raw: Vec<u8> = Vec::new();

        loop {
            raw.clear();
            let n = self.lines.read_until(b'\n', &mut raw)?;
            if n == 0 {
                break;
            }
            // Strip trailing newline / CR.
            let end = raw.iter().rposition(|&b| b != b'\n' && b != b'\r').map_or(0, |i| i + 1);
            let line_bytes = &raw[..end];

            if !started {
                if line_bytes.starts_with(b"LOCUS") {
                    started = true;
                } else {
                    continue; // skip preamble before the first record
                }
            }

            if line_bytes.starts_with(b"//") {
                return Ok(Some(RecordBlock { header_lines, residues, had_origin }));
            }

            if in_origin {
                // Extract residues (letters + `*` stop / `-` gap); ORIGIN's leading
                // digits are line numbers, not sequence.
                for &b in line_bytes {
                    if b.is_ascii_alphabetic() || b == b'*' || b == b'-' {
                        residues.push(b);
                    }
                }
            } else {
                header_lines.push(String::from_utf8_lossy(line_bytes).into_owned());
                if line_bytes.starts_with(b"ORIGIN") {
                    in_origin = true;
                    had_origin = true;
                }
            }
        }

        if started {
            Ok(Some(RecordBlock { header_lines, residues, had_origin }))
        } else {
            Ok(None)
        }
    }
}

impl Iterator for GenbankReader {
    type Item = std::io::Result<SeqRecord>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.next_block() {
            Ok(Some(block)) => Some(Ok(parse_record(
                &block.header_lines,
                block.residues,
                block.had_origin,
                self.default_molecule_type.as_deref(),
            ))),
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        }
    }
}

/// Read all records from an uncompressed GenBank file.
pub fn read_genbank(path: &str) -> std::io::Result<Vec<SeqRecord>> {
    GenbankReader::open(path, 0, false, None)?.collect()
}

/// Fast scan of an uncompressed GenBank file: each record's `(id, name)` in file
/// order, WITHOUT parsing features or reading the sequence (framing stops at
/// `ORIGIN`). For resolving a label to a (file, contig index) far more cheaply than
/// [`read_genbank`], which matters when a workspace holds large annotated genomes.
/// id/name match [`parse_record`] exactly (via `record_identity`).
pub fn scan_genbank_ids(path: &str) -> std::io::Result<Vec<(String, String)>> {
    let mut reader = BufReader::new(open_at(path, 0, false)?);
    let mut out: Vec<(String, String)> = Vec::new();
    let mut header_lines: Vec<String> = Vec::new();
    let mut started = false;
    let mut in_origin = false;
    let mut raw: Vec<u8> = Vec::new();
    loop {
        raw.clear();
        let n = reader.read_until(b'\n', &mut raw)?;
        if n == 0 {
            break;
        }
        let end = raw.iter().rposition(|&b| b != b'\n' && b != b'\r').map_or(0, |i| i + 1);
        let line_bytes = &raw[..end];

        if line_bytes.starts_with(b"LOCUS") {
            // A LOCUS begins a record; flush any prior one lacking a closing `//`.
            if !header_lines.is_empty() {
                out.push(record_identity(&header_lines));
                header_lines.clear();
            }
            started = true;
            in_origin = false;
            header_lines.push(String::from_utf8_lossy(line_bytes).into_owned());
            continue;
        }
        if !started {
            continue; // skip preamble before the first record
        }
        if line_bytes.starts_with(b"//") {
            out.push(record_identity(&header_lines));
            header_lines.clear();
            started = false;
            in_origin = false;
            continue;
        }
        if in_origin {
            continue; // sequence body — nothing we need
        }
        if line_bytes.starts_with(b"ORIGIN") {
            in_origin = true;
            continue;
        }
        header_lines.push(String::from_utf8_lossy(line_bytes).into_owned());
    }
    if !header_lines.is_empty() {
        out.push(record_identity(&header_lines));
    }
    Ok(out)
}
