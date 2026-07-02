//! `filter_domains`: filter Domainator / Domain_Search annotations on already
//! annotated GenBank records by evalue, greedy overlap resolution,
//! database keep/filter, and domain name. Port of
//! `domainator.filter_domains.filter_domains`.

use std::collections::{HashMap, HashSet};

use crate::seq::{SeqFeature, SeqRecord};
use crate::{DOMAIN_FEATURE_NAME, DOMAIN_SEARCH_BEST_HIT_NAME};

/// Which annotation types to filter.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AnnotationType {
    Domainator,
    DomainSearch,
    All,
}

impl AnnotationType {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "domainator" => Ok(AnnotationType::Domainator),
            "domain_search" => Ok(AnnotationType::DomainSearch),
            "all" => Ok(AnnotationType::All),
            _ => Err(format!("Invalid annotation type: {s}")),
        }
    }
    fn matches(self, kind: &str) -> bool {
        match self {
            AnnotationType::Domainator => kind == DOMAIN_FEATURE_NAME,
            AnnotationType::DomainSearch => kind == DOMAIN_SEARCH_BEST_HIT_NAME,
            AnnotationType::All => kind == DOMAIN_FEATURE_NAME || kind == DOMAIN_SEARCH_BEST_HIT_NAME,
        }
    }
}

/// Options for [`filter_domains`].
pub struct FilterDomainsOptions {
    pub evalue: f64,
    pub max_overlap: f64,
    pub databases_keep: Option<HashSet<String>>,
    pub databases_filter: Option<HashSet<String>>,
    pub selected_domains: Option<HashSet<String>>,
    pub annotation_type: AnnotationType,
}

fn feature_score(f: &SeqFeature) -> f64 {
    f.get_first("score").and_then(|v| v.parse().ok()).unwrap_or(f64::NEG_INFINITY)
}

/// Total overlap (bp) between two single-or-compound feature locations.
fn overlap_size(a: &crate::seq::Location, b: &crate::seq::Location) -> i64 {
    let mut total = 0i64;
    for &(as0, ae0, ..) in &a.parts {
        let (a_s, a_e) = if as0 <= ae0 { (as0, ae0) } else { (ae0, as0) };
        for &(bs0, be0, ..) in &b.parts {
            let (b_s, b_e) = if bs0 <= be0 { (bs0, be0) } else { (be0, bs0) };
            total += (a_e.min(b_e) - a_s.max(b_s)).max(0);
        }
    }
    total
}

fn location_len(loc: &crate::seq::Location) -> i64 {
    loc.parts
        .iter()
        .map(|&(s, e, ..)| if s <= e { e - s } else { s - e })
        .sum()
}

/// `domain_overlap`: greedily keep the highest-scoring domains such that no kept
/// pair overlaps more than `allowed_fraction_overlap` (measured against the
/// candidate's own length). Returns the indices (into `feats`) to keep.
fn domain_overlap(feats: &[&SeqFeature], indices: &[usize], allowed: f64) -> Vec<usize> {
    // Sort candidate indices by score, descending (stable — Python's sorted).
    let mut order: Vec<usize> = indices.to_vec();
    order.sort_by(|&a, &b| feature_score(feats[b]).total_cmp(&feature_score(feats[a])));

    let mut kept: Vec<usize> = Vec::new();
    if let Some(&first) = order.first() {
        kept.push(first);
    }
    for &i in order.iter().skip(1) {
        let mut overlaps = false;
        for &k in &kept {
            let denom = location_len(&feats[i].location);
            if denom > 0
                && overlap_size(&feats[i].location, &feats[k].location) as f64 / denom as f64 > allowed
            {
                overlaps = true;
                break;
            }
        }
        if !overlaps {
            kept.push(i);
        }
    }
    kept
}

/// Filter the annotations on a single record in place.
pub fn filter_record(rec: &mut SeqRecord, opts: &FilterDomainsOptions) {
    let feats: Vec<&SeqFeature> = rec.features.iter().collect();

    // `kept`: which feature indices (into rec.features) survive, keyed by cds_id.
    // Mirrors find_kept_annotations: features from non-filtered databases are kept
    // unconditionally; the rest go through evalue/name filters then overlap.
    let mut kept: HashMap<String, HashSet<usize>> = HashMap::new();
    let mut overlap_candidates: HashMap<String, Vec<usize>> = HashMap::new();
    let mut cds_seen: Vec<String> = Vec::new();

    for (idx, feat) in feats.iter().enumerate() {
        if !opts.annotation_type.matches(&feat.kind) {
            continue;
        }
        let cds_id = feat.get_first("cds_id").unwrap_or("").to_string();
        let db = feat.get_first("database").unwrap_or("");
        let name = feat.get_first("name").unwrap_or("");

        if let Some(keep) = &opts.databases_keep {
            if !keep.contains(db) {
                continue;
            }
        }

        let entry = kept.entry(cds_id.clone());
        if let std::collections::hash_map::Entry::Vacant(_) = entry {
            cds_seen.push(cds_id.clone());
        }
        kept.entry(cds_id.clone()).or_default();

        // databases_filter: if set and this db is not in it, keep unconditionally.
        if let Some(filt) = &opts.databases_filter {
            if !filt.contains(db) {
                kept.get_mut(&cds_id).unwrap().insert(idx);
                continue;
            }
        }

        if let Some(sel) = &opts.selected_domains {
            if !sel.contains(name) {
                continue;
            }
        }

        let annot_evalue: f64 =
            feat.get_first("evalue").and_then(|v| v.parse().ok()).unwrap_or(f64::INFINITY);
        if annot_evalue > opts.evalue {
            continue;
        }

        overlap_candidates.entry(cds_id).or_default().push(idx);
    }

    // Resolve overlap per CDS, then merge into kept.
    for (cds_id, cands) in &overlap_candidates {
        let resolved = if opts.max_overlap < 1.0 && !cands.is_empty() {
            domain_overlap(&feats, cands, opts.max_overlap)
        } else {
            cands.clone()
        };
        let set = kept.entry(cds_id.clone()).or_default();
        for i in resolved {
            set.insert(i);
        }
    }

    // Build the new feature list: keep filtered features only if their index
    // survived; keep everything else untouched. Preserve original order.
    let keep_idx: HashSet<usize> = kept.values().flatten().copied().collect();
    let mut new_features = Vec::with_capacity(rec.features.len());
    for (idx, feat) in rec.features.iter().enumerate() {
        if opts.annotation_type.matches(&feat.kind) {
            if keep_idx.contains(&idx) {
                new_features.push(feat.clone());
            }
        } else {
            new_features.push(feat.clone());
        }
    }
    rec.features = new_features;
}

/// Filter all records.
pub fn filter_domains(mut records: Vec<SeqRecord>, opts: &FilterDomainsOptions) -> Vec<SeqRecord> {
    for rec in &mut records {
        filter_record(rec, opts);
    }
    records
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seq::{Annotations, Location, Topology};

    fn dom(cds: &str, name: &str, start: i64, end: i64, score: f64, evalue: &str, db: &str) -> SeqFeature {
        let mut f = SeqFeature::new(DOMAIN_FEATURE_NAME, Location::range(start, end, 1));
        f.set("cds_id", cds);
        f.set("name", name);
        f.set("database", db);
        f.set("score", format!("{score}"));
        f.set("evalue", evalue);
        f
    }

    fn contig(features: Vec<SeqFeature>) -> SeqRecord {
        SeqRecord {
            id: "c".into(),
            name: "c".into(),
            description: String::new(),
            seq: vec![b'A'; 600],
            molecule_type: Some("protein".into()),
            topology: Topology::Linear,
            annotations: Annotations::default(),
            features,
        }
    }

    fn names(rec: &SeqRecord) -> Vec<String> {
        rec.features.iter().filter(|f| f.kind == DOMAIN_FEATURE_NAME)
            .map(|f| f.get_first("name").unwrap().to_string()).collect()
    }

    #[test]
    fn evalue_filter() {
        let rec = contig(vec![
            dom("C1", "A", 0, 100, 10.0, "1e-9", "db"),
            dom("C1", "B", 200, 300, 5.0, "1e-2", "db"),
        ]);
        let out = filter_domains(vec![rec], &FilterDomainsOptions {
            evalue: 1e-5, max_overlap: 1.0, databases_keep: None, databases_filter: None,
            selected_domains: None, annotation_type: AnnotationType::All,
        });
        assert_eq!(names(&out[0]), vec!["A"]);
    }

    #[test]
    fn overlap_resolution_keeps_highest() {
        // Two overlapping domains on the same CDS; keep the higher score.
        let rec = contig(vec![
            dom("C1", "low", 0, 100, 5.0, "1e-9", "db"),
            dom("C1", "high", 10, 110, 20.0, "1e-9", "db"),
        ]);
        let out = filter_domains(vec![rec], &FilterDomainsOptions {
            evalue: 100.0, max_overlap: 0.5, databases_keep: None, databases_filter: None,
            selected_domains: None, annotation_type: AnnotationType::All,
        });
        assert_eq!(names(&out[0]), vec!["high"]);
    }

    #[test]
    fn databases_keep_removes_others() {
        let rec = contig(vec![
            dom("C1", "A", 0, 100, 10.0, "1e-9", "keep"),
            dom("C1", "B", 200, 300, 5.0, "1e-9", "drop"),
        ]);
        let mut keep = HashSet::new();
        keep.insert("keep".to_string());
        let out = filter_domains(vec![rec], &FilterDomainsOptions {
            evalue: 100.0, max_overlap: 1.0, databases_keep: Some(keep), databases_filter: None,
            selected_domains: None, annotation_type: AnnotationType::All,
        });
        assert_eq!(names(&out[0]), vec!["A"]);
    }

    #[test]
    fn databases_filter_leaves_others_untouched() {
        // db "other" not in filter set -> kept regardless of evalue.
        let rec = contig(vec![
            dom("C1", "A", 0, 100, 10.0, "1e2", "filtered"),
            dom("C1", "B", 200, 300, 5.0, "1e2", "other"),
        ]);
        let mut filt = HashSet::new();
        filt.insert("filtered".to_string());
        let out = filter_domains(vec![rec], &FilterDomainsOptions {
            evalue: 1e-5, max_overlap: 1.0, databases_keep: None, databases_filter: Some(filt),
            selected_domains: None, annotation_type: AnnotationType::All,
        });
        // A dropped by evalue, B kept because its db is not being filtered.
        assert_eq!(names(&out[0]), vec!["B"]);
    }

    #[test]
    fn domain_name_filter() {
        let rec = contig(vec![
            dom("C1", "A", 0, 100, 10.0, "1e-9", "db"),
            dom("C1", "B", 200, 300, 5.0, "1e-9", "db"),
        ]);
        let mut sel = HashSet::new();
        sel.insert("B".to_string());
        let out = filter_domains(vec![rec], &FilterDomainsOptions {
            evalue: 100.0, max_overlap: 1.0, databases_keep: None, databases_filter: None,
            selected_domains: Some(sel), annotation_type: AnnotationType::All,
        });
        assert_eq!(names(&out[0]), vec!["B"]);
    }
}
