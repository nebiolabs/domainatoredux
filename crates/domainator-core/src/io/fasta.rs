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

/// Fast scan of an uncompressed FASTA file: each record's `(id, name)` in file
/// order, reading only `>` header lines (sequence bodies are skipped). For FASTA,
/// `id` and `name` are both the first whitespace token of the title (matching
/// [`read_fasta`]). Used for cheap cross-file record resolution.
pub fn scan_fasta_ids(path: &str) -> io::Result<Vec<(String, String)>> {
    let mut reader = BufReader::new(open_at(path, 0, false)?);
    let mut out: Vec<(String, String)> = Vec::new();
    let mut raw: Vec<u8> = Vec::new();
    loop {
        raw.clear();
        let n = reader.read_until(b'\n', &mut raw)?;
        if n == 0 {
            break;
        }
        if raw.first() == Some(&b'>') {
            let end = raw
                .iter()
                .rposition(|&b| b != b'\n' && b != b'\r')
                .map_or(0, |i| i + 1);
            let title = String::from_utf8_lossy(&raw[1..end]);
            let id = title.split_whitespace().next().unwrap_or("").to_string();
            out.push((id.clone(), id));
        }
    }
    Ok(out)
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
    let reader = BufReader::new(open_at(path, offset, compressed)?);
    parse_fasta(reader, max_recs, molecule_type)
}

/// Read every record of a FASTA that may be gzip- or BGZF-compressed, deciding by
/// the file's magic bytes rather than its name (see
/// [`super::open_decompressed`]). For whole-file reads such as reference
/// databases, where nothing seeks, so plain gzip is as good as BGZF.
pub fn read_fasta_decompressed(
    path: &str,
    molecule_type: Option<&str>,
) -> io::Result<Vec<SeqRecord>> {
    let reader = BufReader::new(super::open_decompressed(std::path::Path::new(path))?);
    parse_fasta(reader, u64::MAX, molecule_type)
}

fn parse_fasta<R: BufRead>(
    mut reader: R,
    max_recs: u64,
    molecule_type: Option<&str>,
) -> io::Result<Vec<SeqRecord>> {
    let mol = molecule_type.unwrap_or("protein").to_string();
    let mut out: Vec<SeqRecord> = Vec::new();

    let mut title: Option<String> = None;
    let mut seq: Vec<u8> = Vec::new();
    let mut line: Vec<u8> = Vec::new();

    let flush =
        |out: &mut Vec<SeqRecord>, title: &mut Option<String>, seq: &mut Vec<u8>, mol: &str| {
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
            title = Some(String::from_utf8_lossy(rstrip(&line[1..])).into_owned());
        } else if title.is_some() {
            // Upstream (Biopython) strips each line's trailing whitespace and drops
            // spaces, tabs and carriage returns anywhere in the sequence.
            seq.extend(
                rstrip(&line)
                    .iter()
                    .filter(|&&b| !matches!(b, b' ' | b'\t' | b'\r')),
            );
        }
    }
    if (out.len() as u64) < max_recs {
        flush(&mut out, &mut title, &mut seq, &mol);
    }
    Ok(out)
}

/// `line` without its trailing whitespace, as Python's `str.rstrip()` removes it.
fn rstrip(line: &[u8]) -> &[u8] {
    let end = line
        .iter()
        .rposition(|b| !matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c))
        .map_or(0, |i| i + 1);
    &line[..end]
}

/// Write records as FASTA, wrapping sequence at 60 columns.
/// Biopython-style FASTA title (`Bio.SeqIO.FastaIO.FastaWriter`): `id` alone when
/// there is no description, the description alone when it already starts with the
/// id, else `"{id} {description}"`.
pub fn fasta_title(id: &str, description: &str) -> String {
    let d = description.trim();
    if d.is_empty() {
        id.to_string()
    } else if d.split_whitespace().next() == Some(id) {
        d.to_string()
    } else {
        format!("{id} {d}")
    }
}

/// Write records as FASTA with Biopython's header convention, taking the record
/// **id** (LOCUS for GenBank, header for FASTA) as
/// the sequence identifier. Wraps sequence at 60 columns.
pub fn write_fasta_bp<W: Write>(w: &mut W, records: &[SeqRecord]) -> io::Result<()> {
    for rec in records {
        writeln!(w, ">{}", fasta_title(&rec.id, &rec.description))?;
        for chunk in rec.seq.chunks(60) {
            writeln!(w, "{}", String::from_utf8_lossy(chunk))?;
        }
    }
    Ok(())
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seq::{Annotations, SeqRecord, Topology};

    /// Biopython's rules, which upstream reads FASTA with: the title loses its
    /// trailing whitespace, and spaces, tabs and carriage returns vanish from the
    /// sequence wherever they are.
    #[test]
    fn whitespace_in_sequence_lines_is_dropped() {
        let text = b">a desc \t\r\n\tAC\x0cGT\tA \r\n  CC \x0c\n>b\nGG";
        let recs = parse_fasta(&text[..], u64::MAX, None).unwrap();
        assert_eq!(recs[0].description, "a desc");
        assert_eq!(recs[0].seq, b"AC\x0cGTACC");
        assert_eq!(recs[1].seq, b"GG");
    }

    #[test]
    fn fasta_round_trip() {
        let recs = vec![
            SeqRecord {
                id: "seq1".into(),
                name: "seq1".into(),
                description: "seq1 a description".into(),
                seq: b"MKAILV".to_vec(),
                molecule_type: Some("protein".into()),
                topology: Topology::Linear,
                annotations: Annotations::default(),
                features: vec![],
            },
            SeqRecord {
                id: "seq2".into(),
                name: "seq2".into(),
                description: String::new(),
                seq: b"ACGTACGTACGTACGT".to_vec(),
                molecule_type: Some("protein".into()),
                topology: Topology::Linear,
                annotations: Annotations::default(),
                features: vec![],
            },
        ];
        let path = std::env::temp_dir().join(format!("dom_fa_{}.fasta", std::process::id()));
        let ps = path.to_str().unwrap();
        write_fasta(ps, &recs).unwrap();
        let back = read_fasta(ps, Some("protein")).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(back.len(), 2);
        // id is the first header token; sequence survives.
        assert_eq!(back[0].id, "seq1");
        assert_eq!(back[0].seq, b"MKAILV");
        assert_eq!(back[1].id, "seq2");
        assert_eq!(back[1].seq, b"ACGTACGTACGTACGT");
    }
}
