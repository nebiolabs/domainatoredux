//! `summary-report`: dataset-level statistics over a domainator-annotated GenBank file.
//!
//! Port of `domainator.summary_report`'s data path: contig/CDS counts and length stats,
//! per-domain frequency with average score, focus-domain co-occurrence, and (optionally)
//! taxonomy counts.
//!
//! HTML output stays deferred — it is matplotlib/seaborn plots with no clean pure-Rust
//! path, the same reason the tool as a whole was deferred before. The `--json` payload
//! is one of upstream's four documented agent contracts and needs no plotting, so it is
//! ported in full; the text report reproduces everything except the ASCII length
//! histogram (bashplotlib is not replicated, as in `matrix_report`).

use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::{json, Map, Value};

use crate::seq::taxonomy::NCBITaxonomy;
use crate::seq::SeqRecord;
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
    pub cds_per_10kb: f64,
    pub length: Option<LengthStats>,
    pub domains: Vec<DomainRow>,
    /// `{row_domain: {col_domain: percent}}` for the requested focus domains; empty
    /// when none were given.
    pub cooccurrence: BTreeMap<String, BTreeMap<String, f64>>,
    pub taxonomy: Vec<TaxonRow>,
}

#[derive(Debug, Clone, Default)]
pub struct SummaryReportOptions {
    /// Domains to report co-occurrence for. Empty means no co-occurrence section.
    pub focus_domains: Vec<String>,
    /// Restrict counted domains to these databases; empty means all.
    pub databases: Vec<String>,
    pub report_taxonomy: bool,
}

/// `np.percentile(values, 50)` — linear interpolation, not nearest-rank.
fn median(sorted: &[f64]) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let pos = (sorted.len() - 1) as f64 / 2.0;
    let lo = pos.floor() as usize;
    if lo + 1 >= sorted.len() {
        return sorted[sorted.len() - 1];
    }
    sorted[lo] + (pos - lo as f64) * (sorted[lo + 1] - sorted[lo])
}

/// Accumulator for one domain name across the whole input.
#[derive(Default)]
struct DomainAcc {
    count: u64,
    avg_score: f64,
    databases: HashSet<String>,
    descriptions: HashSet<String>,
    /// How many contigs carrying this domain also carry each other domain.
    correlations: HashMap<String, u64>,
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
    let mut num_cdss = 0usize;
    let mut domains: HashMap<String, DomainAcc> = HashMap::new();
    // First-encounter order. Python sorts an insertion-ordered dict with a *stable*
    // sort, so equal counts keep the order the domains were first seen in -- not
    // alphabetical order, which a HashMap would otherwise impose.
    let mut domain_order: Vec<String> = Vec::new();
    // taxid -> count, insertion-ordered like Python's dict.
    let mut taxa_order: Vec<i64> = Vec::new();
    let mut taxa_counts: HashMap<i64, u64> = HashMap::new();

    for contig in records {
        lengths.push(contig.len() as f64);

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

        // Domain names present on THIS contig, for the co-occurrence counts.
        let mut contig_domains: HashSet<String> = HashSet::new();
        for feature in &contig.features {
            if feature.kind == "CDS" {
                num_cdss += 1;
            }
            if feature.kind != DOMAIN_FEATURE_NAME {
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

            contig_domains.insert(name.clone());
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
                    *acc.correlations.entry(d2.clone()).or_insert(0) += 1;
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
    let mut cooccurrence: BTreeMap<String, BTreeMap<String, f64>> = BTreeMap::new();
    for focus in &opts.focus_domains {
        let Some(acc) = domains.get(focus) else {
            continue;
        };
        let Some(&self_count) = acc.correlations.get(focus) else {
            continue;
        };
        if self_count == 0 {
            continue;
        }
        let row = acc
            .correlations
            .iter()
            .map(|(other, &n)| (other.clone(), 100.0 * n as f64 / self_count as f64))
            .collect();
        cooccurrence.insert(focus.clone(), row);
    }

    // Frequency table, most common first; ties by name so the order is deterministic
    // (Python's sort is stable over an insertion-ordered dict, which a HashMap is not).
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
            TaxonRow {
                taxid,
                name,
                rank,
                count: taxa_counts[&taxid],
            }
        })
        .collect();

    SummaryReport {
        contigs: records.len(),
        cdss: num_cdss,
        cds_per_10kb,
        length,
        domains: rows,
        cooccurrence,
        taxonomy: taxonomy_rows,
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

/// The agent-facing `--json` payload, matching Python's `SummaryJSONWriter`.
pub fn to_json(r: &SummaryReport) -> Value {
    let mut payload = Map::new();
    payload.insert(
        "contig_stats".into(),
        json!({
            "contigs": r.contigs,
            "cdss": r.cdss,
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
            let mut outer = Map::new();
            for (row, cols) in &r.cooccurrence {
                let inner: Map<String, Value> =
                    cols.iter().map(|(c, v)| (c.clone(), json!(v))).collect();
                outer.insert(row.clone(), Value::Object(inner));
            }
            Value::Object(outer)
        },
    );
    if !r.taxonomy.is_empty() {
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

/// The human-readable text report.
///
/// Reproduces every section except the ASCII contig-length histogram, which bashplotlib
/// renders and this port does not replicate (the same gap `matrix_report` has).
pub fn render_text(r: &SummaryReport) -> String {
    let mut s = String::new();
    s.push_str("Contig Stats\n\n");
    s.push_str(&format!("contigs: {}\n", r.contigs));
    s.push_str(&format!("CDSs: {}\n", r.cdss));
    s.push_str(&format!("CDSs per 10 kb: {:.2}\n\n", r.cds_per_10kb));

    s.push_str("Domain Stats\n        \n        Domain Frequency\n\n");
    if r.domains.is_empty() {
        s.push_str("  (no domain annotations found)\n");
    } else {
        s.push_str(&format!(
            "{:<24} {:<20} {:>7}  {:<44} {:>9}\n",
            "domain", "database", "count", "description", "avg score"
        ));
        for d in &r.domains {
            s.push_str(&format!(
                "{:<24} {:<20} {:>7}  {:<44} {:>9.0}\n",
                d.domain, d.database, d.count, d.description, d.avg_score
            ));
        }
    }
    s.push('\n');

    if !r.cooccurrence.is_empty() {
        s.push_str(
            "Domain Co-occurence (what percent of the time when the row-domain occurs \
             does the column domain occur on the same contig)\n",
        );
        let cols: Vec<&String> = r
            .cooccurrence
            .values()
            .flat_map(|m| m.keys())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        s.push_str(&format!("{:<24}", ""));
        for c in &cols {
            s.push_str(&format!("{c:>14}"));
        }
        s.push('\n');
        for (row, vals) in &r.cooccurrence {
            s.push_str(&format!("{row:<24}"));
            for c in &cols {
                match vals.get(*c) {
                    Some(v) => s.push_str(&format!("{v:>14.1}")),
                    None => s.push_str(&format!("{:>14}", "")),
                }
            }
            s.push('\n');
        }
        s.push('\n');
    }

    if !r.taxonomy.is_empty() {
        s.push_str("Taxonomy\n");
        s.push_str(&format!(
            "{:<10} {:<34} {:<14} {:>7}\n",
            "taxid", "name", "rank", "count"
        ));
        for t in &r.taxonomy {
            s.push_str(&format!(
                "{:<10} {:<34} {:<14} {:>7}\n",
                t.taxid, t.name, t.rank, t.count
            ));
        }
    }
    s
}
