//! A hand-written GenBank record parser producing our [`SeqRecord`].
//!
//! We own this end-to-end (rather than wrapping gb-io) so we control fidelity and
//! are robust to inputs gb-io diverges on — adversarial qualifier continuations
//! (`/x="...(a=b)..."`), `*` stop codons in sequences, protein `aa` LOCUS layouts.
//! The target is round-trip parity with Biopython via `helpers.compare_seqfiles`.

use crate::seq::location::parse_location;
use crate::seq::{Annotations, Reference, SeqFeature, SeqRecord, Topology};

/// Split a `;`-delimited GenBank list field, stripping a trailing `.`. Biopython
/// yields `[""]` for an empty/`.`-only field (KEYWORDS), so we mirror that.
fn split_semicolon_keywords(value: &str) -> Vec<String> {
    let trimmed = value.trim();
    let body = trimmed.strip_suffix('.').unwrap_or(trimmed);
    if body.trim().is_empty() {
        return vec![String::new()];
    }
    body.split(';').map(|s| s.trim().to_string()).collect()
}

fn split_taxonomy(value: &str) -> Vec<String> {
    let trimmed = value.trim();
    let body = trimmed.strip_suffix('.').unwrap_or(trimmed);
    if body.trim().is_empty() {
        return Vec::new();
    }
    body.split(';').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

const MONTHS: [&str; 12] = [
    "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
];

fn looks_like_date(tok: &str) -> bool {
    // DD-MON-YYYY
    let parts: Vec<&str> = tok.split('-').collect();
    parts.len() == 3
        && parts[0].len() == 2
        && parts[0].chars().all(|c| c.is_ascii_digit())
        && MONTHS.contains(&parts[1])
        && parts[2].len() == 4
        && parts[2].chars().all(|c| c.is_ascii_digit())
}

/// Parsed LOCUS fields.
struct Locus {
    name: String,
    molecule_type: Option<String>,
    topology: Option<Topology>,
    division: Option<String>,
    date: Option<String>,
    is_aa: bool,
}

fn parse_locus(line: &str) -> Locus {
    let toks: Vec<&str> = line.split_whitespace().collect();
    let name = toks.get(1).map(|s| s.to_string()).unwrap_or_default();
    // Find the units token (bp/aa); molecule type, topology, division, date follow.
    let unit_idx = toks.iter().position(|t| *t == "bp" || *t == "aa");
    let is_aa = matches!(unit_idx.map(|i| toks[i]), Some("aa"));
    let mut molecule_type = None;
    let mut topology = None;
    let mut division = None;
    let mut date = None;
    if let Some(ui) = unit_idx {
        for &tok in &toks[ui + 1..] {
            if tok == "linear" {
                topology = Some(Topology::Linear);
            } else if tok == "circular" {
                topology = Some(Topology::Circular);
            } else if looks_like_date(tok) {
                date = Some(tok.to_string());
            } else if tok.len() == 3 && tok.chars().all(|c| c.is_ascii_uppercase()) && division.is_none() && molecule_type.is_some() {
                division = Some(tok.to_string());
            } else if molecule_type.is_none() && !is_aa {
                molecule_type = Some(tok.to_string());
            } else if division.is_none() {
                division = Some(tok.to_string());
            }
        }
    }
    if is_aa {
        molecule_type = Some("protein".to_string());
    }
    Locus { name, molecule_type, topology, division, date, is_aa }
}

/// One framed header section: the 12-col keyword and its (joined) lines.
struct Section {
    keyword: String,
    /// Raw lines belonging to this section, including continuations and
    /// sub-keyword lines (for REFERENCE), with the leading indent preserved.
    lines: Vec<String>,
}

/// Returns true if a line begins a new top-level section (keyword in cols 0..n,
/// not a continuation/sub-keyword indented by spaces).
fn is_section_start(line: &str) -> bool {
    !line.is_empty() && !line.starts_with(' ')
}

/// Compute just the `(id, name)` that a full [`parse_record`] would assign, from
/// the header lines alone — no feature/sequence work. Used by the fast id scanner
/// (`super::reader::scan_genbank_ids`) so cross-file record resolution (e.g. SSN
/// node → source record) matches the real parser exactly without paying for a full
/// parse. Mirrors the id/name derivation at the end of [`parse_record`].
pub fn record_identity(header_lines: &[String]) -> (String, String) {
    let mut sections: Vec<Section> = Vec::new();
    for line in header_lines {
        if is_section_start(line) {
            let keyword = line.split_whitespace().next().unwrap_or("").to_string();
            sections.push(Section { keyword, lines: vec![line.clone()] });
        } else if let Some(last) = sections.last_mut() {
            last.lines.push(line.clone());
        }
    }

    let mut name = String::new();
    let mut first_accession: Option<String> = None;
    let mut version_token: Option<String> = None;
    for sec in &sections {
        match sec.keyword.as_str() {
            "LOCUS" if !sec.lines.is_empty() => name = parse_locus(&sec.lines[0]).name,
            "ACCESSION" => {
                let val = join_section_value(sec, "ACCESSION");
                if first_accession.is_none() {
                    first_accession = val.split_whitespace().next().map(str::to_string);
                }
            }
            "VERSION" => {
                let val = join_section_value(sec, "VERSION");
                for tok in val.split_whitespace() {
                    if tok.strip_prefix("GI:").is_none() && version_token.is_none() {
                        version_token = Some(tok.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    let id = version_token.or(first_accession).unwrap_or_else(|| name.clone());
    (id, name)
}

/// Parse one record from its raw header lines (LOCUS..before ORIGIN) plus the
/// already-extracted residues. `default_molecule_type` applies when LOCUS lacks one.
pub fn parse_record(
    header_lines: &[String],
    residues: Vec<u8>,
    had_origin: bool,
    default_molecule_type: Option<&str>,
) -> SeqRecord {
    // Group header lines into top-level sections.
    let mut sections: Vec<Section> = Vec::new();
    for line in header_lines {
        if is_section_start(line) {
            let keyword = line.split_whitespace().next().unwrap_or("").to_string();
            sections.push(Section { keyword, lines: vec![line.clone()] });
        } else if let Some(last) = sections.last_mut() {
            last.lines.push(line.clone());
        }
    }

    let mut rec = SeqRecord {
        id: String::new(),
        name: String::new(),
        description: String::new(),
        seq: if had_origin { residues.to_ascii_uppercase() } else { Vec::new() },
        molecule_type: None,
        topology: Topology::Linear,
        annotations: Annotations::default(),
        features: Vec::new(),
    };
    let mut version_token: Option<String> = None;

    for sec in &sections {
        match sec.keyword.as_str() {
            "LOCUS" => {
                let locus = parse_locus(&sec.lines[0]);
                rec.name = locus.name.clone();
                rec.molecule_type = locus
                    .molecule_type
                    .or_else(|| default_molecule_type.map(str::to_string))
                    .or_else(|| if locus.is_aa { Some("protein".into()) } else { Some("DNA".into()) });
                if let Some(t) = locus.topology {
                    rec.topology = t;
                }
                rec.annotations.division = locus.division;
                rec.annotations.date = locus.date;
            }
            "DEFINITION" => {
                // Biopython's GenBank parser strips a single trailing '.' from
                // DEFINITION; mirror that so `record.description` matches.
                let mut d = join_section_value(sec, "DEFINITION");
                if d.ends_with('.') {
                    d.pop();
                }
                rec.description = d;
            }
            "ACCESSION" => {
                let val = join_section_value(sec, "ACCESSION");
                rec.annotations.accessions =
                    val.split_whitespace().map(str::to_string).collect();
            }
            "VERSION" => {
                let val = join_section_value(sec, "VERSION");
                for tok in val.split_whitespace() {
                    if let Some(gi) = tok.strip_prefix("GI:") {
                        rec.annotations.gi = Some(gi.to_string());
                    } else if version_token.is_none() {
                        version_token = Some(tok.to_string());
                    }
                }
            }
            "DBLINK" => {
                rec.annotations.dblink = sec
                    .lines
                    .iter()
                    .enumerate()
                    .map(|(i, l)| if i == 0 { l["DBLINK".len()..].trim() } else { l.trim() })
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect();
            }
            "DBSOURCE" => {
                rec.annotations.db_source = Some(join_section_value(sec, "DBSOURCE"));
            }
            "KEYWORDS" => {
                rec.annotations.keywords = split_semicolon_keywords(&join_section_value(sec, "KEYWORDS"));
            }
            "SOURCE" => {
                parse_source(sec, &mut rec.annotations);
            }
            "REFERENCE" => {
                rec.annotations.references.push(parse_reference(sec));
            }
            "COMMENT" => {
                parse_comment(sec, &mut rec.annotations);
            }
            "FEATURES" => {
                rec.features = parse_features(&sec.lines);
            }
            _ => {}
        }
    }

    // VERSION → id + sequence_version.
    if let Some(ver) = version_token {
        rec.id = ver.clone();
        if let Some((_, n)) = ver.rsplit_once('.') {
            if let Ok(v) = n.parse::<i64>() {
                rec.annotations.sequence_version = Some(v);
            }
        }
    }
    if rec.id.is_empty() {
        rec.id = rec.annotations.accessions.first().cloned().unwrap_or_else(|| rec.name.clone());
    }
    rec
}

/// Join a header section's value: the first line after the keyword column, plus
/// continuation lines (indented), separated by a single space.
fn join_section_value(sec: &Section, keyword: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    for (i, line) in sec.lines.iter().enumerate() {
        let text = if i == 0 {
            line.get(keyword.len()..).unwrap_or("").trim()
        } else {
            line.trim()
        };
        if !text.is_empty() {
            parts.push(text.to_string());
        }
    }
    parts.join(" ")
}

fn parse_source(sec: &Section, ann: &mut Annotations) {
    // SOURCE line, then "  ORGANISM" sub-line, then taxonomy continuation lines.
    let mut source_parts: Vec<String> = Vec::new();
    let mut organism: Option<String> = None;
    let mut taxonomy_lines: Vec<String> = Vec::new();
    let mut in_organism = false;
    for (i, line) in sec.lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if i == 0 {
            source_parts.push(line["SOURCE".len()..].trim().to_string());
        } else if let Some(rest) = trimmed.strip_prefix("ORGANISM") {
            in_organism = true;
            organism = Some(rest.trim().to_string());
        } else if in_organism {
            taxonomy_lines.push(line.trim().to_string());
        } else {
            source_parts.push(line.trim().to_string());
        }
    }
    let src = source_parts.join(" ").trim().to_string();
    ann.source = Some(src);
    if let Some(org) = organism {
        ann.organism = Some(org);
        ann.taxonomy = split_taxonomy(&taxonomy_lines.join(" "));
    }
}

fn parse_reference(sec: &Section) -> Reference {
    let mut r = Reference::default();
    // REFERENCE line: "REFERENCE   1  (bases 1 to 2552)" -> location_text after number.
    if let Some(first) = sec.lines.first() {
        let after = first["REFERENCE".len()..].trim();
        // Drop the leading reference number; keep the rest (base range).
        let mut it = after.splitn(2, char::is_whitespace);
        let _num = it.next();
        let rest = it.next().unwrap_or("").trim();
        if !rest.is_empty() {
            r.location_text = Some(rest.to_string());
        }
    }
    // Sub-keywords are indented (col 2): AUTHORS, CONSRTM, TITLE, JOURNAL, PUBMED,
    // MEDLINE, REMARK. Continuation lines are further indented.
    let mut current: Option<&mut String> = None;
    let mut buf: Vec<(String, String)> = Vec::new();
    let mut order: Vec<String> = Vec::new();
    for line in &sec.lines[1..] {
        let trimmed = line.trim_start();
        let sub = ["AUTHORS", "CONSRTM", "TITLE", "JOURNAL", "PUBMED", "MEDLINE", "REMARK"]
            .iter()
            .find(|kw| trimmed.starts_with(**kw));
        if let Some(kw) = sub {
            let val = trimmed[kw.len()..].trim().to_string();
            order.push(kw.to_string());
            buf.push((kw.to_string(), val));
        } else if let Some((_, v)) = buf.last_mut() {
            // continuation
            if !v.is_empty() {
                v.push(' ');
            }
            v.push_str(line.trim());
        }
    }
    let _ = (&mut current, &order);
    for (kw, val) in buf {
        match kw.as_str() {
            "AUTHORS" => r.authors = val,
            "CONSRTM" => r.consrtm = val,
            "TITLE" => r.title = val,
            "JOURNAL" => r.journal = val,
            "PUBMED" => r.pubmed_id = val,
            "MEDLINE" => r.medline_id = val,
            "REMARK" => r.remark = val,
            _ => {}
        }
    }
    r
}

fn parse_comment(sec: &Section, ann: &mut Annotations) {
    // Lines after the COMMENT keyword. A structured comment is delimited by
    // "##<Name>-START##" / "##<Name>-END##" with "key :: value" lines between.
    let mut raw: Vec<String> = Vec::new();
    for (i, line) in sec.lines.iter().enumerate() {
        let text = if i == 0 { line["COMMENT".len()..].trim_start() } else { line.trim_start() };
        raw.push(text.to_string());
    }
    let mut free: Vec<String> = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        let line = &raw[i];
        if let Some(name) = line.strip_prefix("##").and_then(|s| s.strip_suffix("-START##")) {
            let mut entries: Vec<(String, String)> = Vec::new();
            i += 1;
            while i < raw.len() {
                let inner = &raw[i];
                if inner.starts_with("##") && inner.ends_with("-END##") {
                    break;
                }
                if let Some((k, v)) = inner.split_once("::") {
                    entries.push((k.trim().to_string(), v.trim().to_string()));
                }
                i += 1;
            }
            ann.structured_comment.push((name.to_string(), entries));
        } else {
            free.push(line.clone());
        }
        i += 1;
    }
    let joined = free.join("\n");
    let trimmed = joined.trim_matches('\n');
    if !trimmed.is_empty() {
        ann.comment = Some(trimmed.to_string());
    }
}

fn parse_features(lines: &[String]) -> Vec<SeqFeature> {
    let mut features: Vec<SeqFeature> = Vec::new();
    // lines[0] is "FEATURES             Location/Qualifiers"; skip it.
    let mut i = 1;
    while i < lines.len() {
        let line = &lines[i];
        // A feature line: 5-space indent, key starts at col 5, location at col 21.
        if line.len() > 5 && line.as_bytes()[5] != b' ' && line.starts_with("     ") {
            let kind = line[5..].split_whitespace().next().unwrap_or("").to_string();
            let mut loc_text = line[5 + kind.len()..].trim().to_string();
            // Location may continue on following indented (col 21) lines until a
            // qualifier (/) or the next feature.
            let mut j = i + 1;
            while j < lines.len() {
                let l = &lines[j];
                let t = l.trim_start();
                if l.starts_with("                     ") && !t.starts_with('/') {
                    loc_text.push_str(t);
                    j += 1;
                } else {
                    break;
                }
            }
            let location = parse_location(&loc_text);
            let (quals, next) = parse_qualifiers(lines, j);
            features.push(SeqFeature { kind, location, qualifiers: quals });
            i = next;
        } else {
            i += 1;
        }
    }
    features
}

/// Parse qualifier lines starting at index `start`, until the next feature line or
/// end. Returns the qualifiers and the index to resume from.
fn parse_qualifiers(lines: &[String], start: usize) -> (Vec<(String, Vec<String>)>, usize) {
    let mut quals: Vec<(String, Vec<String>)> = Vec::new();
    let mut i = start;
    let qual_indent = "                     "; // 21 spaces
    while i < lines.len() {
        let line = &lines[i];
        if !line.starts_with(qual_indent) {
            break; // next feature or section
        }
        let trimmed = line.trim_start();
        if !trimmed.starts_with('/') {
            break;
        }
        // /key or /key=value (value may be quoted and span lines). Trailing
        // whitespace after a closing quote is insignificant, so trim line-right
        // before testing for the close (else we'd swallow the next feature).
        let body = trimmed[1..].trim_end();
        let (key, mut value, has_value, mut open_quote) = match body.split_once('=') {
            Some((k, v)) => {
                let k = k.to_string();
                let v = v.trim_end();
                if let Some(stripped) = v.strip_prefix('"') {
                    if let Some(end) = stripped.strip_suffix('"') {
                        (k, end.to_string(), true, false)
                    } else {
                        (k, stripped.to_string(), true, true)
                    }
                } else {
                    (k, v.to_string(), true, false)
                }
            }
            None => (body.to_string(), String::new(), false, false),
        };
        i += 1;
        // Continuation lines for an open quoted value.
        while open_quote && i < lines.len() {
            let ctrim = lines[i].trim().trim_end();
            let join_sep = if key == "translation" { "" } else { " " };
            if let Some(end) = ctrim.strip_suffix('"') {
                value.push_str(join_sep);
                value.push_str(end);
                open_quote = false;
            } else {
                value.push_str(join_sep);
                value.push_str(ctrim);
            }
            i += 1;
        }
        // GenBank escapes a literal double-quote inside a value by doubling it.
        if has_value {
            value = value.replace("\"\"", "\"");
        }
        push_qualifier(&mut quals, &key, value, has_value);
    }
    (quals, i)
}

fn push_qualifier(quals: &mut Vec<(String, Vec<String>)>, key: &str, value: String, _has_value: bool) {
    if let Some((_, vs)) = quals.iter_mut().find(|(k, _)| k == key) {
        vs.push(value);
    } else {
        quals.push((key.to_string(), vec![value]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &str) -> Vec<String> {
        s.lines().map(str::to_string).collect()
    }

    #[test]
    fn record_identity_matches_parse_record_derivation() {
        // VERSION wins the id; LOCUS name is the name.
        let (id, name) = record_identity(&lines(
            "LOCUS       ABC12345             1000 bp    DNA     linear   BCT 01-JAN-2020\n\
             ACCESSION   ABC12345\n\
             VERSION     ABC12345.2  GI:999\n\
             DEFINITION  Something.",
        ));
        assert_eq!(name, "ABC12345");
        assert_eq!(id, "ABC12345.2"); // GI: token skipped, .version kept

        // No VERSION → first ACCESSION becomes the id.
        let (id, name) = record_identity(&lines(
            "LOCUS       myLocus              500 bp    DNA     linear   01-JAN-2020\n\
             ACCESSION   ACC001 ACC002",
        ));
        assert_eq!(name, "myLocus");
        assert_eq!(id, "ACC001");

        // Neither VERSION nor ACCESSION → id falls back to the LOCUS name.
        let (id, name) = record_identity(&lines(
            "LOCUS       onlyName             12 aa    linear",
        ));
        assert_eq!(name, "onlyName");
        assert_eq!(id, "onlyName");
    }
}
