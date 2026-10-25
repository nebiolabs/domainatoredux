//! `select_by_coord`: extract contig regions using explicit 1-based inclusive
//! coordinate ranges from a labeled coordinate table (columns: `contig`,
//! `start`, `end`, optional `strand`/`file`). Port of
//! `domainator.select_by_coord`.

use std::collections::HashMap;

use crate::seq::{SeqRecord, Topology};
use crate::tools::neighborhood::{region_id, slice_record, Env, SliceLoc};

/// One row of the coordinate table.
#[derive(Clone, Debug, PartialEq)]
pub struct CoordinateSelection {
    pub contig: String,
    /// 1-based inclusive start.
    pub start: i64,
    /// 1-based inclusive end.
    pub end: i64,
    /// `1` (forward) or `-1` (reverse).
    pub strand: i32,
    /// Optional basename filter (matches any input file when `None`).
    pub file: Option<String>,
}

/// Parse a strand cell. Accepts `+/1/f/forward` and `-/-1/r/reverse`
/// (case-insensitive); empty/absent -> forward. Mirrors `_parse_strand`.
pub fn parse_strand(value: Option<&str>, row_number: usize) -> Result<i32, String> {
    let v = match value {
        None => return Ok(1),
        Some(v) => v.trim().to_ascii_lowercase(),
    };
    if v.is_empty() {
        return Ok(1);
    }
    match v.as_str() {
        "+" | "1" | "f" | "forward" => Ok(1),
        "-" | "-1" | "r" | "reverse" => Ok(-1),
        _ => Err(format!(
            "Invalid strand value on row {row_number}: {value:?}"
        )),
    }
}

fn parse_coordinate(value: &str, column: &str, row_number: usize) -> Result<i64, String> {
    let parsed: i64 = value.trim().parse().map_err(|_| {
        format!("Column '{column}' must contain integers. Row {row_number}: {value}")
    })?;
    if parsed < 1 {
        return Err(format!(
            "Column '{column}' must be >= 1. Row {row_number}: {parsed}"
        ));
    }
    Ok(parsed)
}

const ALLOWED_COLUMNS: &[&str] = &["contig", "start", "end", "strand", "file"];
const REQUIRED_COLUMNS: &[&str] = &["contig", "start", "end"];

/// Parse a coordinate table (TSV / CSV / semicolon-delimited, auto-sniffed from
/// the header row) into selections. Mirrors `load_coordinate_table`.
pub fn parse_coordinate_table(text: &str) -> Result<Vec<CoordinateSelection>, String> {
    if text.trim().is_empty() {
        return Err("Coordinate table is empty.".into());
    }
    let mut lines = text.lines();
    let header_line = lines
        .next()
        .ok_or("Coordinate table must include a header row.")?;
    let delim = sniff_delimiter(header_line);

    let fieldnames: Vec<String> = header_line
        .split(delim)
        .map(|n| n.trim_start_matches('\u{feff}').trim().to_string())
        .collect();

    let mut seen = std::collections::HashSet::new();
    for name in &fieldnames {
        if !seen.insert(name.clone()) {
            return Err("Coordinate table contains duplicate column names.".into());
        }
    }
    for name in &fieldnames {
        if !ALLOWED_COLUMNS.contains(&name.as_str()) {
            return Err(format!("Unexpected coordinate table columns: {name}"));
        }
    }
    for req in REQUIRED_COLUMNS {
        if !fieldnames.iter().any(|n| n == req) {
            return Err(format!("Missing required coordinate table columns: {req}"));
        }
    }
    let idx = |name: &str| fieldnames.iter().position(|n| n == name);
    let (ci, si, ei) = (
        idx("contig").unwrap(),
        idx("start").unwrap(),
        idx("end").unwrap(),
    );
    let sti = idx("strand");
    let fi = idx("file");

    let mut selections = Vec::new();
    for (offset, line) in lines.enumerate() {
        let row_number = offset + 2;
        let cells: Vec<&str> = line.split(delim).collect();
        if cells.len() > fieldnames.len() {
            return Err(format!(
                "Row {row_number} has more fields than the header defines."
            ));
        }
        // Skip fully-blank rows.
        if cells.iter().all(|c| c.trim().is_empty()) {
            continue;
        }
        let get = |i: usize| cells.get(i).copied();
        let contig = get(ci).unwrap_or("").trim().to_string();
        if contig.is_empty() {
            return Err(format!(
                "Column 'contig' must not be blank. Row {row_number}"
            ));
        }
        let file = fi
            .and_then(get)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        if let Some(f) = &file {
            let base = std::path::Path::new(f)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            if base != f {
                return Err(format!(
                    "Column 'file' must contain basenames only. Row {row_number}: {f}"
                ));
            }
        }
        selections.push(CoordinateSelection {
            contig,
            start: parse_coordinate(get(si).unwrap_or(""), "start", row_number)?,
            end: parse_coordinate(get(ei).unwrap_or(""), "end", row_number)?,
            strand: parse_strand(sti.and_then(get), row_number)?,
            file,
        });
    }
    Ok(selections)
}

/// Choose the delimiter used by the header (tab / comma / semicolon), preferring
/// tab. Mirrors csv.Sniffer over `\t,;`, defaulting to tab.
fn sniff_delimiter(header: &str) -> char {
    for d in ['\t', ',', ';'] {
        if header.contains(d) {
            return d;
        }
    }
    '\t'
}

/// Extract one region for `selection` from `contig`, applying the LOCUS-name
/// suffix (`_start:end`, plus `rc` on the reverse strand). Mirrors
/// `extract_coordinate_region` / `_build_location`.
pub fn extract_coordinate_region(
    contig: &SeqRecord,
    selection: &CoordinateSelection,
) -> Result<SeqRecord, String> {
    let contig_length = contig.seq.len() as i64;
    if selection.start > contig_length || selection.end > contig_length {
        return Err(format!(
            "Coordinates out of bounds for contig '{}': {}-{} on length {}",
            contig.id, selection.start, selection.end, contig_length
        ));
    }
    let start = selection.start - 1; // 0-based
    let end = selection.end;
    let strand = selection.strand;

    // Build the slice parts respecting origin-wraparound for circular contigs.
    // Parts are extracted and concatenated in listed order (each reverse-strand
    // part revcomp'd individually), matching Python's CompoundLocation.
    let parts: Vec<Env> = if selection.end >= selection.start {
        vec![Env { start, end, strand }]
    } else {
        if contig.topology != Topology::Circular {
            return Err(format!(
                "end < start is only valid for circular contigs. Contig '{}' is not circular.",
                contig.id
            ));
        }
        // Origin-crossing. Python `_build_location` orders parts by strand:
        //   forward: [start, L), [0, end)
        //   reverse: [0, end), [start, L)
        if strand == -1 {
            vec![
                Env {
                    start: 0,
                    end,
                    strand,
                },
                Env {
                    start,
                    end: contig_length,
                    strand,
                },
            ]
        } else {
            vec![
                Env {
                    start,
                    end: contig_length,
                    strand,
                },
                Env {
                    start: 0,
                    end,
                    strand,
                },
            ]
        }
    };

    let id = region_id(contig, &parts, false);
    let slice = SliceLoc {
        parts,
        overall_strand: strand,
        dist_to_start: 0,
        dist_to_end: 0,
    };
    let mut record = slice_record(contig, &slice, &contig.features);
    record.id = id;
    Ok(record)
}

/// Extract regions across `records` for the given selections. `records` is a list
/// of `(basename, contig)`; a selection with a `file` value only matches its
/// basename. Contigs are matched by LOCUS name (Python's `contig.id`).
pub fn select_by_coord(
    records: &[(String, SeqRecord)],
    selections: &[CoordinateSelection],
) -> Result<Vec<SeqRecord>, String> {
    let mut by_contig: HashMap<&str, Vec<&CoordinateSelection>> = HashMap::new();
    for s in selections {
        by_contig.entry(s.contig.as_str()).or_default().push(s);
    }
    let mut out = Vec::new();
    for (basename, contig) in records {
        if let Some(sels) = by_contig.get(contig.id.as_str()) {
            for sel in sels {
                if let Some(f) = &sel.file {
                    if f != basename {
                        continue;
                    }
                }
                out.push(extract_coordinate_region(contig, sel)?);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seq::{Annotations, Location, SeqFeature, Topology};

    fn rec(name: &str, seq: &[u8], topology: Topology) -> SeqRecord {
        SeqRecord {
            id: name.into(),
            name: name.into(),
            description: String::new(),
            seq: seq.to_vec(),
            molecule_type: Some("DNA".into()),
            topology,
            annotations: Annotations::default(),
            features: vec![SeqFeature::new(
                "source",
                Location::range(0, seq.len() as i64, 1),
            )],
        }
    }

    #[test]
    fn strand_parsing() {
        assert_eq!(parse_strand(None, 1).unwrap(), 1);
        assert_eq!(parse_strand(Some(""), 1).unwrap(), 1);
        for f in ["+", "1", "f", "F", "forward", "Forward"] {
            assert_eq!(parse_strand(Some(f), 1).unwrap(), 1, "{f}");
        }
        for r in ["-", "-1", "r", "R", "reverse", "Reverse"] {
            assert_eq!(parse_strand(Some(r), 1).unwrap(), -1, "{r}");
        }
        assert!(parse_strand(Some("x"), 3).is_err());
    }

    #[test]
    fn table_parse_and_validate() {
        let t = "contig\tstart\tend\nc1\t2\t6\nc2\t1\t3\n";
        let s = parse_coordinate_table(t).unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(
            s[0],
            CoordinateSelection {
                contig: "c1".into(),
                start: 2,
                end: 6,
                strand: 1,
                file: None
            }
        );
        // missing required column
        assert!(parse_coordinate_table("contig\tstart\nc\t1\n").is_err());
        // unexpected column
        assert!(parse_coordinate_table("contig\tstart\tend\tbad\nc\t1\t2\t3\n").is_err());
        // coordinate < 1
        assert!(parse_coordinate_table("contig\tstart\tend\nc\t0\t2\n").is_err());
    }

    #[test]
    fn csv_delimiter_sniff() {
        let s = parse_coordinate_table("contig,start,end\nc1,2,6\n").unwrap();
        assert_eq!(s[0].contig, "c1");
        assert_eq!(s[0].start, 2);
    }

    #[test]
    fn forward_region_seq() {
        let r = rec("c", b"AACCGGTT", Topology::Linear);
        let sel = CoordinateSelection {
            contig: "c".into(),
            start: 2,
            end: 6,
            strand: 1,
            file: None,
        };
        let out = extract_coordinate_region(&r, &sel).unwrap();
        assert_eq!(out.seq, b"ACCGG");
        assert_eq!(out.id, "c_2:6");
    }

    #[test]
    fn reverse_region_rc_suffix() {
        let r = rec("c", b"AACCGGTT", Topology::Linear);
        let sel = CoordinateSelection {
            contig: "c".into(),
            start: 1,
            end: 4,
            strand: -1,
            file: None,
        };
        let out = extract_coordinate_region(&r, &sel).unwrap();
        // seq[0:4] = "AACC"; revcomp -> "GGTT"
        assert_eq!(out.seq, b"GGTT");
        // The first base written is position 4.
        assert_eq!(out.id, "c_4:1rc");
    }

    #[test]
    fn circular_wraparound() {
        let r = rec("c", b"AACCGGTT", Topology::Circular);
        // start=7 end=2 -> seq[6:8] + seq[0:2] = "TT" + "AA" = "TTAA"
        let sel = CoordinateSelection {
            contig: "c".into(),
            start: 7,
            end: 2,
            strand: 1,
            file: None,
        };
        let out = extract_coordinate_region(&r, &sel).unwrap();
        assert_eq!(out.seq, b"TTAA");
        assert_eq!(out.id, "c_7:2");
    }

    #[test]
    fn linear_wraparound_errors() {
        let r = rec("c", b"AACCGGTT", Topology::Linear);
        let sel = CoordinateSelection {
            contig: "c".into(),
            start: 7,
            end: 2,
            strand: 1,
            file: None,
        };
        assert!(extract_coordinate_region(&r, &sel).is_err());
    }
}
