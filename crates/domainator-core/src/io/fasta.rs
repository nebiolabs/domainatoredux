//! FASTA reader/writer. Case is preserved on read (Biopython does not upper-case
//! FASTA); `id` is the first whitespace token of the title, `description` the full
//! title line.

use std::io::{self, BufRead, BufReader, Write};

use crate::io::bgzf::open_at;
use crate::seq::{Annotations, SeqRecord, Topology};

/// Read all records from an uncompressed FASTA file.
pub fn read_fasta(path: &str, molecule_type: Option<&str>) -> io::Result<Vec<SeqRecord>> {
    read_fasta_partition(path, 0, false, u64::MAX, molecule_type)
}

/// Read up to `max_recs` records from `path` starting at `offset` (a byte offset,
/// or a BGZF virtual offset when `compressed`). `molecule_type` is applied to
/// every record (FASTA carries no molecule type; defaults to "protein").
pub fn read_fasta_partition(
    path: &str,
    offset: u64,
    compressed: bool,
    max_recs: u64,
    molecule_type: Option<&str>,
) -> io::Result<Vec<SeqRecord>> {
    let mut reader = BufReader::new(open_at(path, offset, compressed)?);
    let mol = molecule_type.unwrap_or("protein").to_string();
    let mut out: Vec<SeqRecord> = Vec::new();

    let mut title: Option<String> = None;
    let mut seq: Vec<u8> = Vec::new();
    let mut line: Vec<u8> = Vec::new();

    let flush = |out: &mut Vec<SeqRecord>, title: &mut Option<String>, seq: &mut Vec<u8>, mol: &str| {
        if let Some(t) = title.take() {
            let id = t.split_whitespace().next().unwrap_or("").to_string();
            out.push(SeqRecord {
                id,
                name: t.split_whitespace().next().unwrap_or("").to_string(),
                description: t,
                seq: std::mem::take(seq),
                molecule_type: Some(mol.to_string()),
                topology: Topology::Linear,
                annotations: Annotations::default(),
                features: Vec::new(),
            });
        } else {
            seq.clear();
        }
    };

    loop {
        line.clear();
        let n = reader.read_until(b'\n', &mut line)?;
        if n == 0 {
            break;
        }
        if line.first() == Some(&b'>') {
            flush(&mut out, &mut title, &mut seq, &mol);
            if out.len() as u64 >= max_recs {
                return Ok(out); // partition limit reached; don't start the next record
            }
            let raw = &line[1..];
            let end = raw.iter().rposition(|&b| b != b'\n' && b != b'\r').map_or(0, |i| i + 1);
            title = Some(String::from_utf8_lossy(&raw[..end]).into_owned());
        } else if title.is_some() {
            for &b in &line {
                if b != b'\n' && b != b'\r' {
                    seq.push(b);
                }
            }
        }
    }
    if (out.len() as u64) < max_recs {
        flush(&mut out, &mut title, &mut seq, &mol);
    }
    Ok(out)
}

/// Write records as FASTA, wrapping sequence at 60 columns.
pub fn write_fasta(path: &str, records: &[SeqRecord]) -> io::Result<()> {
    let file = std::fs::File::create(path)?;
    let mut w = io::BufWriter::new(file);
    for rec in records {
        if rec.description.is_empty() {
            writeln!(w, ">{}", rec.id)?;
        } else {
            writeln!(w, ">{}", rec.description)?;
        }
        for chunk in rec.seq.chunks(60) {
            writeln!(w, "{}", String::from_utf8_lossy(chunk))?;
        }
    }
    w.flush()
}
