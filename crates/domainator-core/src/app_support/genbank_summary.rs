//! Small GenBank summarizer for the bespoke Tauri app.

use std::collections::{BTreeMap, HashSet};
use std::io::Write;

use crate::seq::taxid::record_taxid;
use crate::seq::{SeqFeature, SeqRecord};
use crate::DOMAIN_FEATURE_NAME;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummaryGroupBy {
    Taxid,
    Definition,
}

impl SummaryGroupBy {
    pub fn column_name(self) -> &'static str {
        match self {
            SummaryGroupBy::Taxid => "taxid",
            SummaryGroupBy::Definition => "definition",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SummaryOptions {
    pub group_by: SummaryGroupBy,
    pub domain_names: Vec<String>,
    pub evalue: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryRow {
    pub key: String,
    pub count: usize,
    pub record_names: Vec<String>,
}

pub fn parse_domain_names(input: &str) -> Vec<String> {
    input
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

pub fn summarize_records(records: &[SeqRecord], opts: &SummaryOptions) -> Vec<SummaryRow> {
    let requested: HashSet<&str> = opts.domain_names.iter().map(String::as_str).collect();
    match opts.group_by {
        SummaryGroupBy::Taxid => summarize_taxids(records, &requested, opts.evalue),
        SummaryGroupBy::Definition => summarize_definitions(records, &requested, opts.evalue),
    }
}

fn summarize_taxids(
    records: &[SeqRecord],
    requested: &HashSet<&str>,
    evalue: f64,
) -> Vec<SummaryRow> {
    let mut groups: BTreeMap<i64, Vec<String>> = BTreeMap::new();
    for rec in records {
        if record_has_domains(rec, requested, evalue) {
            groups
                .entry(record_taxid(rec))
                .or_default()
                .push(rec.id.clone());
        }
    }
    groups
        .into_iter()
        .map(|(key, record_names)| SummaryRow {
            key: key.to_string(),
            count: record_names.len(),
            record_names,
        })
        .collect()
}

fn summarize_definitions(
    records: &[SeqRecord],
    requested: &HashSet<&str>,
    evalue: f64,
) -> Vec<SummaryRow> {
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for rec in records {
        if record_has_domains(rec, requested, evalue) {
            groups
                .entry(rec.description.clone())
                .or_default()
                .push(rec.id.clone());
        }
    }
    groups
        .into_iter()
        .map(|(key, record_names)| SummaryRow {
            key,
            count: record_names.len(),
            record_names,
        })
        .collect()
}

pub fn write_tsv<W: Write>(
    rows: &[SummaryRow],
    group_by: SummaryGroupBy,
    out: &mut W,
) -> Result<(), String> {
    writeln!(out, "{}\tcount\trecords", group_by.column_name()).map_err(|e| e.to_string())?;
    for row in rows {
        writeln!(
            out,
            "{}\t{}\t{}",
            clean_tsv_cell(&row.key),
            row.count,
            clean_tsv_cell(&row.record_names.join(","))
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn summarize_records_tsv(
    records: &[SeqRecord],
    opts: &SummaryOptions,
) -> Result<String, String> {
    let rows = summarize_records(records, opts);
    let mut out = Vec::new();
    write_tsv(&rows, opts.group_by, &mut out)?;
    String::from_utf8(out).map_err(|e| e.to_string())
}

fn record_has_domains(rec: &SeqRecord, requested: &HashSet<&str>, evalue: f64) -> bool {
    if requested.is_empty() {
        return true;
    }
    let present: HashSet<&str> = rec
        .features
        .iter()
        .filter_map(|f| passing_domain_name(f, evalue))
        .collect();
    requested.iter().all(|name| present.contains(name))
}

fn passing_domain_name(feature: &SeqFeature, evalue: f64) -> Option<&str> {
    if feature.kind != DOMAIN_FEATURE_NAME {
        return None;
    }
    let domain_evalue = feature.get_first("evalue")?.parse::<f64>().ok()?;
    if domain_evalue <= evalue {
        feature.get_first("name")
    } else {
        None
    }
}

fn clean_tsv_cell(value: &str) -> String {
    value.replace(['\t', '\r', '\n'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seq::{Location, SeqRecord, Topology};

    fn rec(name: &str, definition: &str, taxid: i64, features: Vec<SeqFeature>) -> SeqRecord {
        let mut source = SeqFeature::new("source", Location::range(0, 12, 1));
        source.set("db_xref", format!("taxon:{taxid}"));
        let mut all_features = vec![source];
        all_features.extend(features);
        SeqRecord {
            id: name.into(),
            name: name.into(),
            description: definition.into(),
            seq: b"ATGAAATAATTT".to_vec(),
            molecule_type: Some("DNA".into()),
            topology: Topology::Linear,
            annotations: Default::default(),
            features: all_features,
        }
    }

    fn domain(name: &str, evalue: &str) -> SeqFeature {
        let mut f = SeqFeature::new(DOMAIN_FEATURE_NAME, Location::range(0, 6, 1));
        f.set("name", name);
        f.set("evalue", evalue);
        f
    }

    #[test]
    fn groups_by_definition() {
        let records = vec![
            rec("c1", "same", 1, vec![]),
            rec("c2", "same", 2, vec![]),
            rec("c3", "other", 1, vec![]),
        ];
        let opts = SummaryOptions {
            group_by: SummaryGroupBy::Definition,
            domain_names: Vec::new(),
            evalue: 1e-3,
        };

        let rows = summarize_records(&records, &opts);

        assert_eq!(
            rows[0],
            SummaryRow {
                key: "other".into(),
                count: 1,
                record_names: vec!["c3".into()]
            }
        );
        assert_eq!(
            rows[1],
            SummaryRow {
                key: "same".into(),
                count: 2,
                record_names: vec!["c1".into(), "c2".into()]
            }
        );
    }

    #[test]
    fn groups_by_taxid() {
        let records = vec![
            rec("c1", "a", 42, vec![]),
            rec("c2", "b", 42, vec![]),
            rec("c3", "c", 7, vec![]),
        ];
        let opts = SummaryOptions {
            group_by: SummaryGroupBy::Taxid,
            domain_names: Vec::new(),
            evalue: 1e-3,
        };

        let rows = summarize_records(&records, &opts);

        assert_eq!(
            rows[0],
            SummaryRow {
                key: "7".into(),
                count: 1,
                record_names: vec!["c3".into()]
            }
        );
        assert_eq!(
            rows[1],
            SummaryRow {
                key: "42".into(),
                count: 2,
                record_names: vec!["c1".into(), "c2".into()]
            }
        );
    }

    #[test]
    fn requires_all_requested_domains_anywhere_on_contig() {
        let records = vec![
            rec(
                "keep",
                "a",
                1,
                vec![domain("A", "1e-20"), domain("B", "1e-10")],
            ),
            rec("drop", "a", 1, vec![domain("A", "1e-20")]),
        ];
        let opts = SummaryOptions {
            group_by: SummaryGroupBy::Definition,
            domain_names: vec!["A".into(), "B".into()],
            evalue: 1e-3,
        };

        let rows = summarize_records(&records, &opts);

        assert_eq!(
            rows,
            vec![SummaryRow {
                key: "a".into(),
                count: 1,
                record_names: vec!["keep".into()]
            }]
        );
    }

    #[test]
    fn ignores_domains_above_or_missing_evalue() {
        let records = vec![
            rec("high", "a", 1, vec![domain("A", "1e-1")]),
            rec(
                "missing",
                "a",
                1,
                vec![{
                    let mut f = SeqFeature::new(DOMAIN_FEATURE_NAME, Location::range(0, 6, 1));
                    f.set("name", "A");
                    f
                }],
            ),
            rec("keep", "a", 1, vec![domain("A", "1e-20")]),
        ];
        let opts = SummaryOptions {
            group_by: SummaryGroupBy::Definition,
            domain_names: vec!["A".into()],
            evalue: 1e-3,
        };

        let rows = summarize_records(&records, &opts);

        assert_eq!(
            rows,
            vec![SummaryRow {
                key: "a".into(),
                count: 1,
                record_names: vec!["keep".into()]
            }]
        );
    }

    #[test]
    fn writes_tsv() {
        let rows = vec![SummaryRow {
            key: "some\tdefinition".into(),
            count: 2,
            record_names: vec!["c1".into(), "c2".into()],
        }];
        let mut out = Vec::new();

        write_tsv(&rows, SummaryGroupBy::Definition, &mut out).unwrap();

        assert_eq!(
            String::from_utf8(out).unwrap(),
            "definition\tcount\trecords\nsome definition\t2\tc1,c2\n"
        );
    }

    #[test]
    fn parses_domain_names() {
        assert_eq!(parse_domain_names(" A, B ,,C "), vec!["A", "B", "C"]);
    }
}
