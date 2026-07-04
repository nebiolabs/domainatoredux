//! Pure-data summary statistics for a sequence file, for the GUI's Summary
//! dashboard. Computes counts/distributions from `SeqRecord`s (no plotting — the
//! frontend charts these), so it sidesteps the matplotlib dependency that keeps
//! the CLI `summary_report` deferred.

use serde::Serialize;

use crate::seq::SeqRecord;
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
        // Taxonomy path: the ORGANISM lineage, with organism as the leaf.
        let mut path: Vec<String> = rec.annotations.taxonomy.iter().filter(|s| !s.is_empty()).cloned().collect();
        if let Some(org) = &rec.annotations.organism {
            if path.last() != Some(org) {
                path.push(org.clone());
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
