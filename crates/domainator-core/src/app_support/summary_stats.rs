//! Pure-data summary statistics for a sequence file, for the GUI's Summary
//! dashboard. Computes counts/distributions from `SeqRecord`s (no plotting — the
//! frontend charts these), so it sidesteps the matplotlib dependency that keeps
//! the CLI `summary_report` deferred.

use serde::Serialize;

use crate::seq::SeqRecord;
use crate::seq::taxonomy::NCBITaxonomy;
use crate::DOMAIN_FEATURE_NAME;

#[derive(Debug, Clone, Serialize)]
pub struct NameCount {
    pub name: String,
    pub count: usize,
}

/// A node in the taxonomy hierarchy (for the icicle plot), built from records'
/// embedded ORGANISM lineage (`annotations.taxonomy` + organism) — no external
/// taxdump needed. `count` = records at or under this node.
#[derive(Debug, Clone, Serialize)]
pub struct TaxNode {
    pub name: String,
    pub count: usize,
    pub children: Vec<TaxNode>,
}

/// Mutable builder that merges lineage paths by name, then freezes to `TaxNode`.
#[derive(Default)]
struct TaxBuilder {
    count: usize,
    children: std::collections::HashMap<String, TaxBuilder>,
}

impl TaxBuilder {
    fn insert(&mut self, path: &[String]) {
        self.count += 1;
        if let Some((head, rest)) = path.split_first() {
            self.children.entry(head.clone()).or_default().insert(rest);
        }
    }
    fn freeze(self, name: String) -> TaxNode {
        let mut children: Vec<TaxNode> =
            self.children.into_iter().map(|(n, b)| b.freeze(n)).collect();
        children.sort_by(|a, b| b.count.cmp(&a.count).then(a.name.cmp(&b.name)));
        TaxNode { name, count: self.count, children }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SummaryStats {
    pub n_contigs: usize,
    pub is_protein: bool,
    pub total_residues: u64,
    pub length_min: usize,
    pub length_max: usize,
    pub length_mean: f64,
    pub length_median: usize,
    /// `[bin_low, count]` over contig lengths.
    pub length_histogram: Vec<[f64; 2]>,
    pub n_cds: usize,
    pub n_domain_features: usize,
    pub distinct_domains: usize,
    /// Domain-name frequency (top 20), by the `Domainator` feature `name`.
    pub top_domains: Vec<NameCount>,
    /// Hit counts by `database` qualifier.
    pub databases: Vec<NameCount>,
    /// Feature counts by kind.
    pub feature_kinds: Vec<NameCount>,
    /// Contig counts by organism (top 20).
    pub top_organisms: Vec<NameCount>,
    /// Records with a lineage/organism (root of the taxonomy tree).
    pub n_with_taxonomy: usize,
    /// Taxonomy hierarchy for the icicle plot (root; its children are the top ranks).
    pub taxonomy: TaxNode,
}

fn histogram(vals: &[usize], bins: usize) -> Vec<[f64; 2]> {
    if vals.len() < 2 {
        return Vec::new();
    }
    let min = *vals.iter().min().unwrap() as f64;
    let max = *vals.iter().max().unwrap() as f64;
    if max <= min {
        return Vec::new();
    }
    let width = (max - min) / bins as f64;
    let mut counts = vec![0i64; bins];
    for &v in vals {
        let mut idx = ((v as f64 - min) / width) as usize;
        if idx >= bins {
            idx = bins - 1;
        }
        counts[idx] += 1;
    }
    counts.iter().enumerate().map(|(i, &c)| [min + i as f64 * width, c as f64]).collect()
}

/// Sort a `(name -> count)` map into a descending, truncated `NameCount` list.
fn top_counts(map: std::collections::HashMap<String, usize>, limit: usize) -> Vec<NameCount> {
    let mut v: Vec<NameCount> = map.into_iter().map(|(name, count)| NameCount { name, count }).collect();
    v.sort_by(|a, b| b.count.cmp(&a.count).then(a.name.cmp(&b.name)));
    v.truncate(limit);
    v
}

pub fn summarize(records: &[SeqRecord]) -> SummaryStats {
    summarize_with_taxonomy(records, None)
}

pub fn summarize_with_taxonomy(records: &[SeqRecord], taxonomy: Option<&NCBITaxonomy>) -> SummaryStats {
    use std::collections::HashMap;

    let lengths: Vec<usize> = records.iter().map(|r| r.len()).collect();
    let total_residues: u64 = lengths.iter().map(|&l| l as u64).sum();
    let length_min = lengths.iter().copied().min().unwrap_or(0);
    let length_max = lengths.iter().copied().max().unwrap_or(0);
    let length_mean = if lengths.is_empty() { 0.0 } else { total_residues as f64 / lengths.len() as f64 };
    let mut sorted = lengths.clone();
    sorted.sort_unstable();
    let length_median = sorted.get(sorted.len() / 2).copied().unwrap_or(0);

    let mut n_cds = 0;
    let mut n_domain_features = 0;
    let mut domain_names: HashMap<String, usize> = HashMap::new();
    let mut databases: HashMap<String, usize> = HashMap::new();
    let mut feature_kinds: HashMap<String, usize> = HashMap::new();
    let mut organisms: HashMap<String, usize> = HashMap::new();
    let mut tax = TaxBuilder::default();

    for rec in records {
        let mut path: Vec<String> = rec.annotations.taxonomy.iter().filter(|s| !s.is_empty()).cloned().collect();
        if let Some(org) = &rec.annotations.organism {
            if path.last() != Some(org) {
                path.push(org.clone());
            }
        }
        if let Some(tax) = taxonomy {
            let lineage = tax.lineage(crate::seq::taxid::record_taxid(rec));
            if !lineage.is_empty() {
                path = lineage.into_iter().rev().filter(|&id| id != 1)
                    .map(|id| tax.name(id).map(str::to_string).unwrap_or_else(|| id.to_string()))
                    .collect();
            }
        }
        if !path.is_empty() {
            tax.insert(&path);
        }
        if let Some(org) = &rec.annotations.organism {
            *organisms.entry(org.clone()).or_default() += 1;
        }
        for f in &rec.features {
            *feature_kinds.entry(f.kind.clone()).or_default() += 1;
            if f.kind == "CDS" {
                n_cds += 1;
            }
            if f.kind == DOMAIN_FEATURE_NAME {
                n_domain_features += 1;
                if let Some(name) = f.get_first("name") {
                    *domain_names.entry(name.to_string()).or_default() += 1;
                }
                if let Some(db) = f.get_first("database") {
                    *databases.entry(db.to_string()).or_default() += 1;
                }
            }
        }
    }

    SummaryStats {
        n_contigs: records.len(),
        is_protein: records.first().map(|r| r.is_protein()).unwrap_or(false),
        total_residues,
        length_min,
        length_max,
        length_mean,
        length_median,
        length_histogram: histogram(&lengths, 30),
        n_cds,
        n_domain_features,
        distinct_domains: domain_names.len(),
        top_domains: top_counts(domain_names, 20),
        databases: top_counts(databases, 50),
        feature_kinds: top_counts(feature_kinds, 50),
        top_organisms: top_counts(organisms, 20),
        n_with_taxonomy: tax.count,
        taxonomy: tax.freeze("root".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seq::{Annotations, Location, SeqFeature, Topology};

    fn rec(id: &str, len: usize, organism: Option<&str>, lineage: &[&str]) -> SeqRecord {
        let mut annotations = Annotations {
            taxonomy: lineage.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        };
        annotations.organism = organism.map(str::to_string);
        SeqRecord {
            id: id.into(),
            name: id.into(),
            description: String::new(),
            seq: vec![b'A'; len],
            molecule_type: Some("DNA".into()),
            topology: Topology::Linear,
            annotations,
            features: Vec::new(),
        }
    }

    fn domain(name: &str, db: &str) -> SeqFeature {
        let mut f = SeqFeature::new(crate::DOMAIN_FEATURE_NAME, Location::range(0, 9, 1));
        f.set("name", name);
        f.set("database", db);
        f
    }

    #[test]
    fn lengths_and_totals() {
        let stats = summarize(&[rec("a", 10, None, &[]), rec("b", 30, None, &[]), rec("c", 20, None, &[])]);
        assert_eq!(stats.n_contigs, 3);
        assert_eq!(stats.total_residues, 60);
        assert_eq!((stats.length_min, stats.length_max), (10, 30));
        assert_eq!(stats.length_mean, 20.0);
        assert_eq!(stats.length_median, 20);
    }

    /// The median is the **upper** middle of an even-length set, not the mean of
    /// the two middles -- `sorted[len / 2]` on integer lengths. Pinned because it
    /// is a silent choice the dashboard displays.
    #[test]
    fn even_counts_take_the_upper_middle_length() {
        let stats = summarize(&[rec("a", 10, None, &[]), rec("b", 20, None, &[])]);
        assert_eq!(stats.length_median, 20);
    }

    #[test]
    fn an_empty_file_summarizes_to_zeros() {
        let stats = summarize(&[]);
        assert_eq!(stats.n_contigs, 0);
        assert_eq!(stats.total_residues, 0);
        assert_eq!((stats.length_min, stats.length_max), (0, 0));
        assert_eq!(stats.length_mean, 0.0);
        assert!(!stats.is_protein);
        assert!(stats.length_histogram.is_empty());
        assert_eq!(stats.taxonomy.count, 0);
    }

    /// A histogram needs at least two records *and* a spread; one record, or many
    /// records all the same length, would otherwise divide by a zero-width bin.
    #[test]
    fn a_histogram_needs_two_distinct_lengths() {
        assert!(summarize(&[rec("a", 10, None, &[])]).length_histogram.is_empty());
        assert!(summarize(&[rec("a", 10, None, &[]), rec("b", 10, None, &[])])
            .length_histogram
            .is_empty());

        let stats = summarize(&[rec("a", 0, None, &[]), rec("b", 30, None, &[])]);
        assert_eq!(stats.length_histogram.len(), 30);
        // Bins are `[bin_low, count]` starting at the minimum, and every record
        // lands in one -- including the maximum, which would otherwise fall off
        // the end of the last bin.
        assert_eq!(stats.length_histogram[0][0], 0.0);
        let total: f64 = stats.length_histogram.iter().map(|b| b[1]).sum();
        assert_eq!(total, 2.0);
    }

    #[test]
    fn domain_counts_rank_by_frequency_then_name() {
        let mut r = rec("a", 10, None, &[]);
        r.features = vec![
            domain("B", "pfam"),
            domain("A", "pfam"),
            domain("A", "pfam"),
            SeqFeature::new("CDS", Location::range(0, 9, 1)),
        ];
        let stats = summarize(std::slice::from_ref(&r));
        assert_eq!(stats.n_cds, 1);
        assert_eq!(stats.n_domain_features, 3);
        assert_eq!(stats.distinct_domains, 2);
        let names: Vec<&str> = stats.top_domains.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, vec!["A", "B"]);
        assert_eq!(stats.top_domains[0].count, 2);
        let dbs: Vec<(&str, usize)> =
            stats.databases.iter().map(|n| (n.name.as_str(), n.count)).collect();
        assert_eq!(dbs, vec![("pfam", 3)]);
        // Every feature kind is counted, not just Domainator ones.
        let kinds: Vec<&str> = stats.feature_kinds.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(kinds, vec![crate::DOMAIN_FEATURE_NAME, "CDS"]);
    }

    /// Lineages merge by name into one tree, children sort by descending count,
    /// and the organism becomes the leaf -- unless the lineage already ends with it.
    #[test]
    fn taxonomy_merges_lineages_and_sorts_children() {
        let stats = summarize(&[
            rec("a", 10, Some("E. coli"), &["Bacteria", "Proteobacteria"]),
            rec("b", 10, Some("Salmonella"), &["Bacteria", "Proteobacteria"]),
            rec("c", 10, Some("B. subtilis"), &["Bacteria", "Firmicutes"]),
            rec("d", 10, None, &[]),
        ]);
        assert_eq!(stats.n_with_taxonomy, 3);
        assert_eq!(stats.taxonomy.name, "root");

        let bacteria = &stats.taxonomy.children[0];
        assert_eq!((bacteria.name.as_str(), bacteria.count), ("Bacteria", 3));
        let phyla: Vec<(&str, usize)> =
            bacteria.children.iter().map(|c| (c.name.as_str(), c.count)).collect();
        assert_eq!(phyla, vec![("Proteobacteria", 2), ("Firmicutes", 1)]);
        let leaves: Vec<&str> =
            bacteria.children[0].children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(leaves, vec!["E. coli", "Salmonella"]);
    }

    #[test]
    fn an_organism_already_ending_the_lineage_is_not_repeated() {
        let stats = summarize(&[rec("a", 10, Some("E. coli"), &["Bacteria", "E. coli"])]);
        let bacteria = &stats.taxonomy.children[0];
        assert_eq!(bacteria.children.len(), 1);
        assert_eq!(bacteria.children[0].name, "E. coli");
        assert!(bacteria.children[0].children.is_empty());
    }

    #[test]
    fn taxid_only_record_uses_registered_taxonomy() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/data/taxdmp");
        let taxonomy = NCBITaxonomy::load(&fixture).unwrap();
        let mut record = rec("a", 10, None, &[]);
        let mut source = SeqFeature::new("source", Location::range(0, 10, 1));
        source.set("db_xref", "taxon:562");
        record.features.push(source);
        assert_eq!(summarize(std::slice::from_ref(&record)).n_with_taxonomy, 0);
        let stats = summarize_with_taxonomy(&[record], Some(&taxonomy));
        assert_eq!(stats.n_with_taxonomy, 1);
        assert_eq!(stats.taxonomy.children[0].name, "cellular organisms");
        assert_eq!(stats.taxonomy.children[0].children[0].name, "Bacteria");
        assert!(stats.taxonomy.children[0].count > 0);
    }

    #[test]
    fn gui_taxonomy_matches_summary_report_taxid_priority() {
        use crate::tools::summary_report::{summary_report, SummaryReportOptions};

        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/data/taxdmp");
        let taxonomy = NCBITaxonomy::load(&fixture).unwrap();
        let mut with_ox = rec("ox", 20, None, &[]);
        with_ox.description = "protein OX=562 TaxID=2".into();
        let mut source = SeqFeature::new("source", Location::range(0, 20, 1));
        source.set("db_xref", "taxon:2");
        with_ox.features.push(source);

        let mut with_taxid = with_ox.clone();
        with_taxid.description = "protein TaxID=562".into();

        let mut longest_source = rec("source", 20, None, &[]);
        let mut short = SeqFeature::new("source", Location::range(0, 5, 1));
        short.set("db_xref", "taxon:562");
        let mut long = SeqFeature::new("source", Location::range(0, 20, 1));
        long.set("db_xref", "taxon:2");
        longest_source.features = vec![short, long];

        for (record, expected_taxid) in [(with_ox, 562), (with_taxid, 562), (longest_source, 2)] {
            let report = summary_report(
                std::slice::from_ref(&record),
                &SummaryReportOptions { report_taxonomy: true, ..Default::default() },
                Some(&taxonomy),
            );
            assert_eq!(report.taxonomy[0].taxid, expected_taxid);
            let stats = summarize_with_taxonomy(&[record], Some(&taxonomy));
            assert_eq!(stats.n_with_taxonomy, 1);
            let lineage = taxonomy.lineage(expected_taxid);
            let mut node = &stats.taxonomy;
            for taxid in lineage.into_iter().rev().filter(|&taxid| taxid != 1) {
                node = node.children.iter().find(|child| child.name == taxonomy.name(taxid).unwrap()).unwrap();
            }
            assert_eq!(node.count, 1);
        }
    }
}
