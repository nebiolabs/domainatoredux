use std::collections::{BTreeMap, HashSet};

const UNIDENTIFIED_TAXID: i64 = 32644;

#[derive(Clone, Copy, PartialEq, Eq)]
enum GroupBy {
    Taxid,
    Definition,
}

struct Record {
    name: String,
    definition: String,
    taxid: i64,
    domains: Vec<DomainHit>,
}

struct DomainHit {
    name: String,
    evalue: Option<f64>,
}

#[no_mangle]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    let mut buf = Vec::<u8>::with_capacity(len);
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

#[no_mangle]
pub unsafe extern "C" fn dealloc(ptr: *mut u8, len: usize) {
    if !ptr.is_null() {
        drop(Vec::from_raw_parts(ptr, 0, len));
    }
}

#[no_mangle]
pub unsafe extern "C" fn summarize(
    genbank_ptr: *mut u8,
    genbank_len: usize,
    group_by: u32,
    domains_ptr: *mut u8,
    domains_len: usize,
    evalue: f64,
) -> *mut u8 {
    let genbank = String::from_utf8_lossy(std::slice::from_raw_parts(genbank_ptr, genbank_len));
    let domains = String::from_utf8_lossy(std::slice::from_raw_parts(domains_ptr, domains_len));
    let result = match summarize_text(&genbank, group_by, &domains, evalue) {
        Ok(tsv) => tsv,
        Err(err) => format!("ERROR\n{err}"),
    };
    into_result_buffer(result.into_bytes())
}

fn into_result_buffer(bytes: Vec<u8>) -> *mut u8 {
    let len = bytes.len();
    let mut out = Vec::with_capacity(len + 4);
    out.extend_from_slice(&(len as u32).to_le_bytes());
    out.extend_from_slice(&bytes);
    let ptr = out.as_mut_ptr();
    std::mem::forget(out);
    ptr
}

fn summarize_text(
    genbank: &str,
    group_by: u32,
    domains: &str,
    evalue: f64,
) -> Result<String, String> {
    if !evalue.is_finite() || evalue < 0.0 {
        return Err("evalue threshold must be a finite non-negative number".into());
    }
    let group_by = match group_by {
        0 => GroupBy::Taxid,
        1 => GroupBy::Definition,
        _ => return Err("unknown grouping".into()),
    };
    let records = parse_records(genbank);
    if records.is_empty() {
        return Err("no GenBank records found".into());
    }
    let requested = parse_domain_names(domains);
    Ok(write_tsv(
        &summarize_records(&records, group_by, &requested, evalue),
        group_by,
    ))
}

fn parse_domain_names(input: &str) -> HashSet<String> {
    input
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn parse_records(text: &str) -> Vec<Record> {
    let mut records = Vec::new();
    let mut block = Vec::new();
    let mut in_record = false;
    for line in text.lines() {
        if line.starts_with("LOCUS") {
            in_record = true;
            block.clear();
        }
        if in_record {
            block.push(line.to_string());
            if line.starts_with("//") {
                records.push(parse_record(&block));
                block.clear();
                in_record = false;
            }
        }
    }
    if in_record && !block.is_empty() {
        records.push(parse_record(&block));
    }
    records
}

fn parse_record(lines: &[String]) -> Record {
    let mut name = String::new();
    let mut definition = String::new();
    let mut taxid = UNIDENTIFIED_TAXID;
    let mut domains = Vec::new();
    let mut index = 0usize;
    while index < lines.len() {
        let line = &lines[index];
        if line.starts_with("LOCUS") {
            name = line
                .split_whitespace()
                .nth(1)
                .unwrap_or_default()
                .to_string();
            index += 1;
        } else if line.starts_with("DEFINITION") {
            let (value, next) = collect_section(lines, index, "DEFINITION");
            definition = value.strip_suffix('.').unwrap_or(&value).to_string();
            if let Some(parsed) = parse_ox_taxid(&definition) {
                taxid = parsed;
            }
            index = next;
        } else if line.starts_with("FEATURES") {
            let (parsed_domains, parsed_taxid, next) = parse_features(lines, index + 1);
            domains = parsed_domains;
            if taxid == UNIDENTIFIED_TAXID {
                if let Some(parsed) = parsed_taxid {
                    taxid = parsed;
                }
            }
            index = next;
        } else {
            index += 1;
        }
    }
    Record {
        name,
        definition,
        taxid,
        domains,
    }
}

fn collect_section(lines: &[String], start: usize, keyword: &str) -> (String, usize) {
    let mut parts = Vec::new();
    let first = lines[start].get(keyword.len()..).unwrap_or_default().trim();
    if !first.is_empty() {
        parts.push(first.to_string());
    }
    let mut index = start + 1;
    while index < lines.len() {
        let line = &lines[index];
        if is_top_level_section(line) {
            break;
        }
        let continuation = line.trim();
        if !continuation.is_empty() {
            parts.push(continuation.to_string());
        }
        index += 1;
    }
    (parts.join(" "), index)
}

fn is_top_level_section(line: &str) -> bool {
    !line.is_empty() && !line.starts_with(' ')
}

fn parse_features(lines: &[String], start: usize) -> (Vec<DomainHit>, Option<i64>, usize) {
    let mut domains = Vec::new();
    let mut taxid = None;
    let mut current_kind = String::new();
    let mut current_qualifiers = Vec::<(String, String)>::new();
    let mut index = start;
    while index < lines.len() {
        let line = &lines[index];
        if line.starts_with("ORIGIN") || line.starts_with("//") || is_top_level_section(line) {
            break;
        }
        if let Some(kind) = feature_kind(line) {
            flush_feature(&current_kind, &current_qualifiers, &mut domains, &mut taxid);
            current_kind = kind.to_string();
            current_qualifiers.clear();
        } else if let Some((key, value)) = qualifier(line) {
            current_qualifiers.push((key, value));
        }
        index += 1;
    }
    flush_feature(&current_kind, &current_qualifiers, &mut domains, &mut taxid);
    (domains, taxid, index)
}

fn feature_kind(line: &str) -> Option<&str> {
    let bytes = line.as_bytes();
    if bytes.len() > 5 && bytes[..5].iter().all(|b| *b == b' ') && bytes.get(5) != Some(&b' ') {
        line[5..].split_whitespace().next()
    } else {
        None
    }
}

fn qualifier(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    let body = trimmed.strip_prefix('/')?;
    let (key, value) = body.split_once('=').unwrap_or((body, ""));
    Some((key.to_string(), value.trim_matches('"').to_string()))
}

fn flush_feature(
    kind: &str,
    qualifiers: &[(String, String)],
    domains: &mut Vec<DomainHit>,
    taxid: &mut Option<i64>,
) {
    if kind == "Domainator" {
        let name = first_qualifier(qualifiers, "name").unwrap_or_default();
        if !name.is_empty() {
            domains.push(DomainHit {
                name,
                evalue: first_qualifier(qualifiers, "evalue").and_then(|v| v.parse().ok()),
            });
        }
    } else if kind == "source" && taxid.is_none() {
        for (_, value) in qualifiers.iter().filter(|(key, _)| key == "db_xref") {
            if let Some(parsed) = value.strip_prefix("taxon:").and_then(|v| v.parse().ok()) {
                *taxid = Some(parsed);
                break;
            }
        }
    }
}

fn first_qualifier(qualifiers: &[(String, String)], key: &str) -> Option<String> {
    qualifiers
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
}

fn parse_ox_taxid(desc: &str) -> Option<i64> {
    for marker in [" OX=", " TaxID="] {
        if let Some(pos) = desc.find(marker) {
            if let Some(tok) = desc[pos + marker.len()..].split_whitespace().next() {
                if let Ok(taxid) = tok.parse() {
                    return Some(taxid);
                }
            }
        }
    }
    None
}

fn summarize_records(
    records: &[Record],
    group_by: GroupBy,
    requested: &HashSet<String>,
    evalue: f64,
) -> Vec<(String, Vec<String>)> {
    match group_by {
        GroupBy::Taxid => {
            let mut groups = BTreeMap::<i64, Vec<String>>::new();
            for rec in records
                .iter()
                .filter(|rec| has_requested_domains(rec, requested, evalue))
            {
                groups.entry(rec.taxid).or_default().push(rec.name.clone());
            }
            groups
                .into_iter()
                .map(|(key, names)| (key.to_string(), names))
                .collect()
        }
        GroupBy::Definition => {
            let mut groups = BTreeMap::<String, Vec<String>>::new();
            for rec in records
                .iter()
                .filter(|rec| has_requested_domains(rec, requested, evalue))
            {
                groups
                    .entry(rec.definition.clone())
                    .or_default()
                    .push(rec.name.clone());
            }
            groups.into_iter().collect()
        }
    }
}

fn has_requested_domains(rec: &Record, requested: &HashSet<String>, evalue: f64) -> bool {
    if requested.is_empty() {
        return true;
    }
    let present: HashSet<&str> = rec
        .domains
        .iter()
        .filter(|domain| domain.evalue.map(|ev| ev <= evalue).unwrap_or(false))
        .map(|domain| domain.name.as_str())
        .collect();
    requested.iter().all(|name| present.contains(name.as_str()))
}

fn write_tsv(rows: &[(String, Vec<String>)], group_by: GroupBy) -> String {
    let mut out = String::new();
    let header = match group_by {
        GroupBy::Taxid => "taxid",
        GroupBy::Definition => "definition",
    };
    out.push_str(header);
    out.push_str("\tcount\trecords\n");
    for (key, names) in rows {
        out.push_str(&clean_tsv(key));
        out.push('\t');
        out.push_str(&names.len().to_string());
        out.push('\t');
        out.push_str(&clean_tsv(&names.join(",")));
        out.push('\n');
    }
    out
}

fn clean_tsv(value: &str) -> String {
    value.replace(['\t', '\r', '\n'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: &str = r#"LOCUS       c1          12 bp    DNA     linear   BCT 01-JAN-2000
DEFINITION  same.
FEATURES             Location/Qualifiers
     source          1..12
                     /db_xref="taxon:42"
     Domainator      1..6
                     /name="A"
                     /evalue="1e-20"
     Domainator      7..12
                     /name="B"
                     /evalue="1e-2"
ORIGIN
        1 atgaaataat tt
//
LOCUS       c2          12 bp    DNA     linear   BCT 01-JAN-2000
DEFINITION  same.
FEATURES             Location/Qualifiers
     source          1..12
                     /db_xref="taxon:42"
     Domainator      1..6
                     /name="A"
                     /evalue="1e-20"
     Domainator      7..12
                     /name="B"
                     /evalue="1e-20"
ORIGIN
        1 atgaaataat tt
//
"#;

    #[test]
    fn summarizes_and_filters_domains() {
        let tsv = summarize_text(GB, 1, "A, B", 1e-3).unwrap();
        assert_eq!(tsv, "definition\tcount\trecords\nsame\t1\tc2\n");
    }

    #[test]
    fn groups_by_taxid() {
        let tsv = summarize_text(GB, 0, "", 1e-3).unwrap();
        assert_eq!(tsv, "taxid\tcount\trecords\n42\t2\tc1,c2\n");
    }
}
