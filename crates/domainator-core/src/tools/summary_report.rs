//! `summary_report`: dataset-level statistics over a domainator-annotated GenBank file.
//!
//! Port of `domainator.summary_report`: contig/CDS counts and length stats, per-domain
//! frequency with average score, focus-domain co-occurrence, and (optionally) taxonomy
//! counts.
//!
//! The JSON payload is one of upstream's four documented agent contracts. The HTML page
//! is upstream's generated shell ([`report_html`](crate::tools::report_html)) filled with
//! [`to_html_data`]: that document plus the contig lengths and the taxonomy tree the page
//! draws (the page bins the lengths itself). The text report ([`render_text`]) is
//! upstream's, laid out with [`text_table`].

use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::{json, Map, Value};

use crate::matrix::text::py_float_str;
use crate::matrix::{DataMatrix, Storage};
use crate::seq::partial::{feature_fragment_status, record_fragment_status, Partial};
use crate::seq::taxonomy::NCBITaxonomy;
use crate::seq::SeqRecord;
use crate::text_plot::{text_histogram, text_table};
use crate::tools::select_common::get_cds_unique_name;
use crate::DOMAIN_FEATURE_NAME;

/// One row of the domain-frequency table.
#[derive(Debug, Clone)]
pub struct DomainRow {
    pub domain: String,
    /// `; `-joined, since a domain name can occur in more than one database.
    pub database: String,
    pub count: u64,
    pub description: String,
    pub avg_score: f64,
}

/// One row of the taxonomy table.
#[derive(Debug, Clone)]
pub struct TaxonRow {
    pub taxid: i64,
    pub name: String,
    pub rank: String,
    pub count: u64,
    /// `(taxid, name, rank)` from the root to this taxon (upstream `TaxonomyData`'s
    /// `lineage`/`names`/`ranks`, ranks lowercased), for the HTML page's tree.
    pub lineage: Vec<(i64, String, String)>,
}

/// Length statistics over the contigs; `None` when there are no records.
#[derive(Debug, Clone)]
pub struct LengthStats {
    pub min: i64,
    pub max: i64,
    pub mean: f64,
    pub median: f64,
    pub total: i64,
}

#[derive(Debug, Clone, Default)]
pub struct SummaryReport {
    pub contigs: usize,
    pub cdss: usize,
    /// Counted CDSs with a `<`/`>` partial end.
    pub partial_cdss: usize,
    /// Counted protein records that are fragments ([`record_fragment_status`]).
    pub fragment_proteins: usize,
    pub cds_per_10kb: f64,
    pub length: Option<LengthStats>,
    /// Every counted contig's length, in input order.
    pub lengths: Vec<f64>,
    pub domains: Vec<DomainRow>,
    /// `(row_domain, [(col_domain, percent)])` for the requested focus domains, each
    /// row's columns in the order upstream's dict holds them (the order the pairs were
    /// first seen); empty when none were given.
    pub cooccurrence: Vec<(String, Vec<(String, f64)>)>,
    /// Every counted domain, in first-encounter order: the axes of the co-occurrence
    /// matrices.
    pub cooccurrence_domains: Vec<String>,
    /// Row-major `cooccurrence_domains`² counts: how many contigs carry both domains.
    /// The diagonal is how many contigs carry each domain.
    pub cooccurrence_counts: Vec<u64>,
    /// Whether taxonomy was requested; the JSON then has a `taxonomy` list even when
    /// it is empty.
    pub report_taxonomy: bool,
    pub taxonomy: Vec<TaxonRow>,
}

#[derive(Debug, Clone, Default)]
pub struct SummaryReportOptions {
    /// Domains to report co-occurrence for, one row each in alphabetical order (upstream
    /// `b10240a`). Empty means no co-occurrence section.
    pub focus_domains: Vec<String>,
    /// Restrict counted domains to these databases; empty means all.
    pub databases: Vec<String>,
    pub report_taxonomy: bool,
    /// `--partial`: which CDSs/proteins are counted. A filtered protein record is
    /// skipped entirely; on a nucleotide contig the filtered CDSs and the domains on
    /// them are not counted, but the contig is.
    pub partial: Partial,
}

/// `np.percentile(values, 50)` — linear interpolation, not nearest-rank.
fn median(sorted: &[f64]) -> f64 {
    crate::stats::percentile(sorted, 50.0)
}

/// Accumulator for one domain name across the whole input.
#[derive(Default)]
struct DomainAcc {
    count: u64,
    avg_score: f64,
    databases: HashSet<String>,
    descriptions: HashSet<String>,
    /// How many contigs carrying this domain also carry each other domain, in the
    /// order the pairs were first seen, as upstream's dict keeps them.
    correlations: Vec<(String, u64)>,
    /// Where each domain is in `correlations`.
    correlation_index: HashMap<String, usize>,
}

impl DomainAcc {
    fn correlation(&self, other: &str) -> Option<u64> {
        self.correlation_index
            .get(other)
            .map(|&i| self.correlations[i].1)
    }

    fn add_correlation(&mut self, other: &str) {
        match self.correlation_index.get(other) {
            Some(&i) => self.correlations[i].1 += 1,
            None => {
                self.correlation_index
                    .insert(other.to_string(), self.correlations.len());
                self.correlations.push((other.to_string(), 1));
            }
        }
    }
}

pub fn summary_report(
    records: &[SeqRecord],
    opts: &SummaryReportOptions,
    taxonomy: Option<&NCBITaxonomy>,
) -> SummaryReport {
    let db_filter: Option<HashSet<&str>> = if opts.databases.is_empty() {
        None
    } else {
        Some(opts.databases.iter().map(String::as_str).collect())
    };

    let mut lengths: Vec<f64> = Vec::with_capacity(records.len());
    let mut num_contigs = 0usize;
    let mut num_cdss = 0usize;
    let mut num_partial_cdss = 0usize;
    let mut num_fragment_proteins = 0usize;
    let mut domains: HashMap<String, DomainAcc> = HashMap::new();
    // First-encounter order. Python sorts an insertion-ordered dict with a *stable*
    // sort, so equal counts keep the order the domains were first seen in -- not
    // alphabetical order, which a HashMap would otherwise impose.
    let mut domain_order: Vec<String> = Vec::new();
    // taxid -> count, insertion-ordered like Python's dict.
    let mut taxa_order: Vec<i64> = Vec::new();
    let mut taxa_counts: HashMap<i64, u64> = HashMap::new();

    for contig in records {
        // cds_ids of the CDSs the partial filter drops.
        let mut excluded_cds_ids: HashSet<String> = HashSet::new();
        if !opts.partial.is_include() {
            if contig.is_protein() {
                if !opts.partial.allows(record_fragment_status(contig)) {
                    continue;
                }
            } else {
                excluded_cds_ids = contig
                    .features
                    .iter()
                    .filter(|f| f.kind == "CDS" && !opts.partial.allows(feature_fragment_status(f)))
                    .map(get_cds_unique_name)
                    .collect();
            }
        }

        lengths.push(contig.len() as f64);
        num_contigs += 1;
        if contig.is_protein() && record_fragment_status(contig).is_some() {
            num_fragment_proteins += 1;
        }

        if opts.report_taxonomy {
            if let Some(tax) = taxonomy {
                let taxid = tax.normalized_taxid(contig_taxid(contig));
                if taxa_counts
                    .insert(taxid, taxa_counts.get(&taxid).copied().unwrap_or(0) + 1)
                    .is_none()
                {
                    taxa_order.push(taxid);
                }
            }
        }

        // Domain names present on THIS contig, for the co-occurrence counts, in feature
        // order (upstream's dict).
        let mut contig_domains: Vec<String> = Vec::new();
        for feature in &contig.features {
            if feature.kind == "CDS"
                && (excluded_cds_ids.is_empty()
                    || !excluded_cds_ids.contains(&get_cds_unique_name(feature)))
            {
                num_cdss += 1;
                if feature_fragment_status(feature).is_some() {
                    num_partial_cdss += 1;
                }
            }
            if feature.kind != DOMAIN_FEATURE_NAME {
                continue;
            }
            if !excluded_cds_ids.is_empty()
                && excluded_cds_ids.contains(feature.get_first("cds_id").unwrap_or("."))
            {
                continue;
            }
            let db = qualifier(feature, "database").unwrap_or_default();
            if let Some(filter) = &db_filter {
                if !filter.contains(db.as_str()) {
                    continue;
                }
            }
            let name = qualifier(feature, "name").unwrap_or_default();
            let description = qualifier(feature, "description").unwrap_or_default();
            let score: f64 = qualifier(feature, "score")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.0);

            if !contig_domains.contains(&name) {
                contig_domains.push(name.clone());
            }
            if !domains.contains_key(&name) {
                domain_order.push(name.clone());
            }
            let acc = domains.entry(name).or_default();
            acc.databases.insert(db);
            acc.descriptions.insert(description);
            acc.count += 1;
            // Online mean, as Python computes it.
            let inv = 1.0 / acc.count as f64;
            acc.avg_score = inv * score + (1.0 - inv) * acc.avg_score;
        }

        // Co-occurrence is per *contig*, counted once per domain pair regardless of how
        // many copies of each the contig carries.
        for d1 in &contig_domains {
            for d2 in &contig_domains {
                if let Some(acc) = domains.get_mut(d1) {
                    acc.add_correlation(d2);
                }
            }
        }
    }

    let total: f64 = lengths.iter().sum();
    // The 0.01 keeps an empty input from dividing by zero, exactly as Python does.
    let cds_per_10kb = num_cdss as f64 / ((total + 0.01) / 10000.0);

    let length = if lengths.is_empty() {
        None
    } else {
        let mut sorted = lengths.clone();
        sorted.sort_by(f64::total_cmp);
        Some(LengthStats {
            min: sorted[0] as i64,
            max: sorted[sorted.len() - 1] as i64,
            mean: total / lengths.len() as f64,
            median: median(&sorted),
            total: total as i64,
        })
    };

    // Co-occurrence, as a percentage of the contigs carrying the row domain.
    let mut cooccurrence: Vec<(String, Vec<(String, f64)>)> = Vec::new();
    let mut focus_domains: Vec<&String> = opts.focus_domains.iter().collect();
    focus_domains.sort();
    focus_domains.dedup();
    for focus in focus_domains {
        let Some(acc) = domains.get(focus) else {
            continue;
        };
        let Some(self_count) = acc.correlation(focus) else {
            continue;
        };
        if self_count == 0 {
            continue;
        }
        let row = acc
            .correlations
            .iter()
            .map(|(other, n)| (other.clone(), 100.0 * *n as f64 / self_count as f64))
            .collect();
        cooccurrence.push((focus.clone(), row));
    }

    let n_domains = domain_order.len();
    let mut cooccurrence_counts = vec![0u64; n_domains * n_domains];
    for (i, d1) in domain_order.iter().enumerate() {
        for (j, d2) in domain_order.iter().enumerate() {
            cooccurrence_counts[i * n_domains + j] = domains[d1].correlation(d2).unwrap_or(0);
        }
    }
    let cooccurrence_domains = domain_order.clone();

    // Frequency table, most common first (upstream sorts with kind="stable").
    let mut rows: Vec<DomainRow> = domain_order
        .into_iter()
        .filter_map(|domain| {
            let acc = domains.remove(&domain)?;
            Some(DomainRow {
                domain,
                database: join_sorted(acc.databases),
                count: acc.count,
                description: join_sorted(acc.descriptions),
                avg_score: acc.avg_score,
            })
        })
        .collect();
    // Stable, so equal counts keep first-encounter order.
    rows.sort_by_key(|a| std::cmp::Reverse(a.count));

    let taxonomy_rows = taxa_order
        .into_iter()
        .map(|taxid| {
            let (name, rank) = taxonomy
                .map(|t| {
                    (
                        t.name(taxid).unwrap_or_default().to_string(),
                        t.rank(taxid).unwrap_or_default().to_string(),
                    )
                })
                .unwrap_or_default();
            let lineage = taxonomy
                .map(|t| {
                    t.lineage(taxid)
                        .into_iter()
                        .rev()
                        .map(|id| {
                            let name = t.name(id).unwrap_or_default().to_string();
                            (id, name, t.rank(id).unwrap_or_default().to_lowercase())
                        })
                        .collect()
                })
                .unwrap_or_default();
            TaxonRow {
                taxid,
                name,
                rank,
                count: taxa_counts[&taxid],
                lineage,
            }
        })
        .collect();

    SummaryReport {
        contigs: num_contigs,
        cdss: num_cdss,
        partial_cdss: num_partial_cdss,
        fragment_proteins: num_fragment_proteins,
        cds_per_10kb,
        length,
        lengths,
        domains: rows,
        cooccurrence,
        cooccurrence_domains,
        cooccurrence_counts,
        report_taxonomy: opts.report_taxonomy,
        taxonomy: taxonomy_rows,
    }
}

/// The domain-by-domain co-occurrence matrix over every counted domain (upstream
/// `write_cooccurence_matrix`): contig counts, or with `fraction` each row divided by
/// its diagonal, the fraction of the row domain's contigs that also carry the column
/// domain.
pub fn cooccurrence_matrix(r: &SummaryReport, fraction: bool) -> DataMatrix {
    let n = r.cooccurrence_domains.len();
    let mut values: Vec<f64> = r.cooccurrence_counts.iter().map(|&c| c as f64).collect();
    if fraction {
        for i in 0..n {
            let diagonal = values[i * n + i];
            for v in &mut values[i * n..(i + 1) * n] {
                *v /= diagonal;
            }
        }
    }
    DataMatrix {
        rows: r.cooccurrence_domains.clone(),
        cols: r.cooccurrence_domains.clone(),
        row_lengths: None,
        col_lengths: None,
        data_type: String::new(),
        storage: Storage::Dense(values),
    }
}

/// The domain-frequency table as TSV, as pandas' `to_csv(sep="\t", index=False)`
/// writes it: a header, one row per domain, and floats in their shortest form.
pub fn domains_tsv(r: &SummaryReport) -> String {
    let mut s = String::from("domain\tdatabase\tcount\tdescription\tavg score\n");
    for d in &r.domains {
        s.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\n",
            csv_field(&d.domain),
            csv_field(&d.database),
            d.count,
            csv_field(&d.description),
            py_float_str(d.avg_score)
        ));
    }
    s
}

/// A field as Python's `csv` module writes it with `QUOTE_MINIMAL` and a tab
/// delimiter: quoted, with quotes doubled, only if it holds a tab, quote or line break.
fn csv_field(field: &str) -> String {
    if field.contains(['\t', '"', '\n', '\r']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

/// `; `-join a set in sorted order. Python joins a Python set, whose iteration order is
/// arbitrary; sorting makes the output reproducible run to run.
fn join_sorted(values: HashSet<String>) -> String {
    let mut v: Vec<String> = values.into_iter().collect();
    v.sort();
    v.join("; ")
}

fn qualifier(feature: &crate::seq::SeqFeature, key: &str) -> Option<String> {
    feature
        .qualifiers
        .iter()
        .find(|(k, _)| k == key)
        .and_then(|(_, vals)| vals.first().cloned())
}

fn contig_taxid(record: &SeqRecord) -> i64 {
    crate::seq::taxid::record_taxid(record)
}

/// Every co-occurrence row's columns, in first-seen order across the rows (the columns
/// pandas gives a DataFrame built from them).
fn cooccurrence_columns(r: &SummaryReport) -> Vec<&String> {
    let mut columns: Vec<&String> = Vec::new();
    for (_, row) in &r.cooccurrence {
        for (column, _) in row {
            if !columns.contains(&column) {
                columns.push(column);
            }
        }
    }
    columns
}

/// The version of the `--json_out` document (upstream `JSON_SCHEMA_VERSION`).
pub const JSON_SCHEMA_VERSION: i64 = 1;

/// The agent-facing `--json_out` payload, matching Python's `SummaryJSONWriter`.
pub fn to_json(r: &SummaryReport) -> Value {
    let mut payload = Map::new();
    payload.insert("schema_version".into(), json!(JSON_SCHEMA_VERSION));
    payload.insert(
        "contig_stats".into(),
        json!({
            "contigs": r.contigs,
            "cdss": r.cdss,
            "partial_cdss": r.partial_cdss,
            "fragment_proteins": r.fragment_proteins,
            "cds_per_10kb": r.cds_per_10kb,
            "length": match &r.length {
                Some(l) => json!({
                    "min": l.min, "max": l.max, "mean": l.mean,
                    "median": l.median, "total": l.total,
                }),
                None => Value::Null,
            },
        }),
    );
    payload.insert(
        "domain_frequency".into(),
        Value::Array(
            r.domains
                .iter()
                .map(|d| {
                    json!({
                        "domain": d.domain,
                        "database": d.database,
                        "count": d.count,
                        "description": d.description,
                        "avg_score": d.avg_score,
                    })
                })
                .collect(),
        ),
    );
    payload.insert(
        "domain_cooccurrence".into(),
        if r.cooccurrence.is_empty() {
            Value::Null
        } else {
            // Upstream builds a DataFrame from the rows, so every row has every row's
            // columns, and pandas' fillna("") makes a missing one an empty string.
            let columns = cooccurrence_columns(r);
            let mut outer = Map::new();
            for (row, cols) in &r.cooccurrence {
                let inner: Map<String, Value> = columns
                    .iter()
                    .map(|&c| {
                        let value = cols.iter().find(|(d, _)| d == c).map(|(_, v)| *v);
                        (c.clone(), value.map_or(json!(""), |v| json!(v)))
                    })
                    .collect();
                outer.insert(row.clone(), Value::Object(inner));
            }
            Value::Object(outer)
        },
    );
    if r.report_taxonomy {
        payload.insert(
            "taxonomy".into(),
            Value::Array(
                r.taxonomy
                    .iter()
                    .map(|t| {
                        json!({
                            "taxid": t.taxid, "name": t.name,
                            "rank": t.rank, "count": t.count,
                        })
                    })
                    .collect(),
            ),
        );
    }
    Value::Object(payload)
}

/// The contig lengths for the page's histogram (upstream `length_counts`): the distinct
/// lengths in ascending order and how many contigs have each, or `None` for no contigs.
pub fn length_counts(lengths: &[f64]) -> Option<Value> {
    if lengths.is_empty() {
        return None;
    }
    let mut tally: BTreeMap<i64, u64> = BTreeMap::new();
    for &length in lengths {
        *tally.entry(length as i64).or_default() += 1;
    }
    Some(json!({
        "lengths": tally.keys().collect::<Vec<_>>(),
        "counts": tally.values().collect::<Vec<_>>(),
    }))
}

/// Ranks kept in the taxonomy tree even when they add nothing (upstream `TREE_KEPT_RANKS`).
const TREE_KEPT_RANKS: [&str; 5] = [
    "cellular root",
    "acellular root",
    "domain",
    "realm",
    "kingdom",
];

#[derive(Default)]
struct TreeNode {
    taxid: i64,
    name: String,
    rank: String,
    count: u64,
    /// In insertion order; [`TreeNode::finish`] sorts them.
    children: Vec<TreeNode>,
}

impl TreeNode {
    fn child(&mut self, taxid: i64, name: &str, rank: &str) -> &mut TreeNode {
        let i = match self.children.iter().position(|c| c.taxid == taxid) {
            Some(i) => i,
            None => {
                self.children.push(TreeNode {
                    taxid,
                    name: name.to_string(),
                    rank: rank.to_string(),
                    ..TreeNode::default()
                });
                self.children.len() - 1
            }
        };
        &mut self.children[i]
    }

    /// The node as JSON, and its total count.
    fn finish(self) -> (Value, u64) {
        let mut children: Vec<(Value, u64, String, i64)> = self
            .children
            .into_iter()
            .map(|mut child| {
                while child.children.len() == 1
                    && child.count == 0
                    && !TREE_KEPT_RANKS.contains(&child.rank.as_str())
                {
                    child = child.children.pop().expect("one child");
                }
                let (name, taxid) = (child.name.clone(), child.taxid);
                let (value, total) = child.finish();
                (value, total, name, taxid)
            })
            .collect();
        children.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| a.2.cmp(&b.2))
                .then(a.3.cmp(&b.3))
        });
        let total = self.count + children.iter().map(|c| c.1).sum::<u64>();
        let value = json!({
            "taxid": self.taxid,
            "name": self.name,
            "rank": self.rank,
            "count": self.count,
            "children": children.into_iter().map(|c| c.0).collect::<Vec<_>>(),
        });
        (value, total)
    }
}

/// Upstream `taxonomy_tree`: nodes `{taxid, name, rank, count, children}`, `count` being
/// the contigs assigned to that taxon itself, under a root `{taxid: 1, name: "All"}`. A
/// taxon hangs from its nearest ancestor with a rank; a node with one child and no contigs
/// of its own is replaced by that child unless its rank is in `TREE_KEPT_RANKS`; children
/// are ordered by total contigs, most first, then by name and taxid.
pub fn taxonomy_tree(taxa: &[TaxonRow]) -> Value {
    let mut root = TreeNode {
        taxid: 1,
        name: "All".into(),
        rank: "no rank".into(),
        ..TreeNode::default()
    };
    for taxon in taxa {
        let last = taxon.lineage.len().saturating_sub(1);
        let mut node = &mut root;
        for (i, (taxid, name, rank)) in taxon.lineage.iter().enumerate() {
            if *taxid == 1 || (i < last && rank == "no rank") {
                continue;
            }
            node = node.child(*taxid, name, rank);
        }
        node.count += taxon.count;
    }
    root.finish().0
}

/// The data of the `--html_out` page (upstream `SummaryHTMLWriter`): the `--json_out`
/// document, the contig lengths, and with `--taxonomy` the taxonomy tree.
pub fn to_html_data(r: &SummaryReport) -> Value {
    json!({
        "summary": to_json(r),
        "length_counts": length_counts(&r.lengths),
        "taxonomy_tree": if r.report_taxonomy { taxonomy_tree(&r.taxonomy) } else { Value::Null },
    })
}

/// The text report (upstream `SummaryTextWriter`).
pub fn render_text(r: &SummaryReport) -> String {
    let mut s = format!(
        "Contig Stats\n\ncontigs: {}\nCDSs: {}\npartial CDSs: {}\nprotein fragments: {}\n\
         CDSs per 10 kb: {:.2}\n\n",
        r.contigs, r.cdss, r.partial_cdss, r.fragment_proteins, r.cds_per_10kb
    );
    if !r.lengths.is_empty() {
        s.push_str(&text_histogram(&r.lengths, "Contig Lengths", 50, 20));
        s.push('\n');
    }

    let rows: Vec<Vec<String>> = r
        .domains
        .iter()
        .map(|d| {
            vec![
                d.domain.clone(),
                d.database.clone(),
                d.count.to_string(),
                d.description.clone(),
                format!("{:.1}", d.avg_score),
            ]
        })
        .collect();
    s.push_str("Domain Frequency\n");
    s.push_str(&text_table(
        &["domain", "database", "count", "description", "avg score"],
        &rows,
        &[false, false, true, false, true],
    ));
    s.push('\n');

    if !r.cooccurrence.is_empty() {
        let columns = cooccurrence_columns(r);
        let rows: Vec<Vec<String>> = r
            .cooccurrence
            .iter()
            .map(|(row, values)| {
                let mut cells = vec![row.clone()];
                cells.extend(columns.iter().map(|&c| {
                    values
                        .iter()
                        .find(|(d, _)| d == c)
                        .map_or(String::new(), |(_, v)| format!("{v:.1}"))
                }));
                cells
            })
            .collect();
        let mut header = vec!["domain"];
        header.extend(columns.iter().map(|c| c.as_str()));
        let mut right = vec![false];
        right.extend(columns.iter().map(|_| true));
        s.push_str(
            "Domain Co-occurrence (the percent of the contigs with the row's domain that \
             also have the column's domain)\n",
        );
        s.push_str(&text_table(&header, &rows, &right));
        s.push('\n');
    }

    if r.report_taxonomy {
        let rows: Vec<Vec<String>> = r
            .taxonomy
            .iter()
            .map(|t| {
                vec![
                    t.taxid.to_string(),
                    t.name.clone(),
                    t.rank.clone(),
                    t.count.to_string(),
                ]
            })
            .collect();
        s.push_str("Taxonomy\n");
        s.push_str(&text_table(
            &["taxid", "name", "rank", "count"],
            &rows,
            &[true, false, false, true],
        ));
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_counts_tally_the_distinct_lengths() {
        assert_eq!(length_counts(&[]), None);
        assert_eq!(
            length_counts(&[7.0]),
            Some(json!({"lengths": [7], "counts": [1]}))
        );
        assert_eq!(
            length_counts(&[140.0, 95.0, 113.0, 95.0]),
            Some(json!({"lengths": [95, 113, 140], "counts": [2, 1, 1]}))
        );
    }

    fn taxon(lineage: &[(i64, &str, &str)], count: u64) -> TaxonRow {
        let &(taxid, name, rank) = lineage.last().unwrap();
        TaxonRow {
            taxid,
            name: name.into(),
            rank: rank.into(),
            count,
            lineage: lineage
                .iter()
                .map(|&(id, n, r)| (id, n.to_string(), r.to_string()))
                .collect(),
        }
    }

    #[test]
    fn taxonomy_tree_skips_unranked_and_single_child_taxa() {
        let bacteria = [
            (1, "root", "no rank"),
            (131567, "cellular organisms", "cellular root"),
            (2, "Bacteria", "domain"),
        ];
        let bacillus = [
            (1239, "Bacillota", "phylum"),
            (91061, "Bacilli", "class"),
            (1386, "Bacillus", "genus"),
        ];
        let escherichia = [
            (1224, "Pseudomonadota", "phylum"),
            (1236, "Gammaproteobacteria", "class"),
            (561, "Escherichia", "genus"),
        ];
        let taxa = [
            taxon(&[&bacteria[..], &bacillus[..]].concat(), 3),
            taxon(&[&bacteria[..], &escherichia[..]].concat(), 5),
            // root > Viruses (acellular root) > Riboviria (realm) > Orthornavirae (kingdom) > Pisuviricota (phylum)
            taxon(
                &[
                    (1, "root", "no rank"),
                    (10239, "Viruses", "acellular root"),
                    (2559587, "Riboviria", "realm"),
                    (2732396, "Orthornavirae", "kingdom"),
                    (2732408, "Pisuviricota", "phylum"),
                ],
                2,
            ),
            // An unranked taxon of its own is kept.
            taxon(
                &[
                    (1, "root", "no rank"),
                    (12908, "unclassified sequences", "no rank"),
                ],
                1,
            ),
        ];
        let leaf = |taxid: i64, name: &str, rank: &str, count: u64| json!({"taxid": taxid, "name": name, "rank": rank, "count": count, "children": []});
        // The cellular and acellular roots, Bacteria (a domain), the realm and the kingdom are
        // kept although each has one child; the single-child phyla and classes are not.
        assert_eq!(
            taxonomy_tree(&taxa),
            json!({"taxid": 1, "name": "All", "rank": "no rank", "count": 0, "children": [
                {"taxid": 131567, "name": "cellular organisms", "rank": "cellular root", "count": 0, "children": [
                    {"taxid": 2, "name": "Bacteria", "rank": "domain", "count": 0, "children": [
                        leaf(561, "Escherichia", "genus", 5),
                        leaf(1386, "Bacillus", "genus", 3),
                    ]},
                ]},
                {"taxid": 10239, "name": "Viruses", "rank": "acellular root", "count": 0, "children": [
                    {"taxid": 2559587, "name": "Riboviria", "rank": "realm", "count": 0, "children": [
                        {"taxid": 2732396, "name": "Orthornavirae", "rank": "kingdom", "count": 0, "children": [
                            leaf(2732408, "Pisuviricota", "phylum", 2),
                        ]},
                    ]},
                ]},
                leaf(12908, "unclassified sequences", "no rank", 1),
            ]})
        );
    }
}
