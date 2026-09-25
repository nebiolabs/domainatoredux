//! `extract_unannotated`: extract regions of proteins **not** covered by domain
//! annotations. Protein sequences only. Port of
//! `domainator.extract_unannotated.extract_unannotated`.

use std::collections::HashSet;

use crate::seq::{SeqFeature, SeqRecord};
use crate::tools::extract_common::{revcomp_record, slice_from_location, Loc};
use crate::{DOMAIN_FEATURE_NAME, DOMAIN_SEARCH_BEST_HIT_NAME};

/// Options for [`extract_unannotated`].
#[derive(Default)]
pub struct ExtractUnannotatedOptions {
    pub evalue: Option<f64>,
    pub score: Option<f64>,
    pub keep_direction: bool,
    pub keep_name: bool,
    pub search_hits: bool,
    pub databases: Option<HashSet<String>>,
    pub largest: bool,
    /// Minimum unannotated region length (`None` = no cutoff).
    pub lb: Option<i64>,
}

fn feature_loc_parts(f: &SeqFeature) -> Vec<(i64, i64, i32)> {
    f.location
        .parts
        .iter()
        .map(|&(s, e, st, ..)| if s <= e { (s, e, st) } else { (e, s, st) })
        .collect()
}

/// The domains selected on `rec` (feature filter cascade, mirroring Python).
fn selected_hits<'a>(rec: &'a SeqRecord, opts: &ExtractUnannotatedOptions) -> Vec<&'a SeqFeature> {
    let mut hits = Vec::new();
    for feature in &rec.features {
        let is_search = feature.kind == DOMAIN_SEARCH_BEST_HIT_NAME;
        let is_dom = feature.kind == DOMAIN_FEATURE_NAME;
        if !((opts.search_hits && is_search) || is_dom) {
            continue;
        }
        if let Some(dbs) = &opts.databases {
            if !feature.get_first("database").map(|d| dbs.contains(d)).unwrap_or(false) {
                continue;
            }
        }
        let mut keep = true;
        if let Some(e) = opts.evalue {
            let ev: f64 = feature.get_first("evalue").and_then(|v| v.parse().ok()).unwrap_or(f64::INFINITY);
            if ev > e {
                keep = false;
            }
        } else if let Some(s) = opts.score {
            let sc: f64 = feature.get_first("score").and_then(|v| v.parse().ok()).unwrap_or(f64::NEG_INFINITY);
            if sc < s {
                keep = false;
            }
        }
        if keep {
            hits.push(feature);
        }
    }
    hits
}

/// `CompoundLocation.inverted`: the gaps between the merged domain parts on a
/// sequence of length `length`. Strand of all output parts follows the first
/// domain part's strand. Returns `None` when the domains span the whole sequence.
fn inverted(domain_parts: &[(i64, i64, i32)], length: i64) -> Option<Vec<(i64, i64, i32)>> {
    if length <= 0 {
        return None;
    }
    if domain_parts.is_empty() {
        // No domains -> whole sequence is unannotated (handled by caller as
        // FeatureLocation(0, length)); return a single covering part.
        return Some(vec![(0, length, 0)]);
    }
    let strand = domain_parts[0].2;
    let mut merged: Vec<(i64, i64)> = domain_parts.iter().map(|&(s, e, _)| (s, e)).collect();
    merged.sort();
    let mut m: Vec<(i64, i64)> = Vec::new();
    for (s, e) in merged {
        if let Some(last) = m.last_mut() {
            if s <= last.1 {
                last.1 = last.1.max(e);
                continue;
            }
        }
        m.push((s, e));
    }
    let covered: i64 = m.iter().map(|(s, e)| e - s).sum();
    if covered == length {
        return None;
    }
    let mut out: Vec<(i64, i64, i32)> = Vec::new();
    let mut next_start = 0i64;
    for &(s, e) in &m {
        if s > next_start {
            out.push((next_start, s, strand));
        }
        next_start = e;
        if next_start >= length {
            break;
        }
    }
    if next_start < length {
        out.push((next_start, length, strand));
    }
    if out.is_empty() {
        return None;
    }
    Some(out)
}

/// Extract unannotated regions from all records.
pub fn extract_unannotated(
    records: &[SeqRecord],
    opts: &ExtractUnannotatedOptions,
) -> Result<Vec<SeqRecord>, String> {
    let mut out = Vec::new();
    for rec in records {
        if rec.molecule_type.as_deref() != Some("protein") {
            return Err("extract_unannotated currently only supports protein sequences.".into());
        }
        let rec_len = rec.seq.len() as i64;
        let hits = selected_hits(rec, opts);

        // Build the domains envelope (concatenation of hit parts), then invert.
        let mut domain_parts: Vec<(i64, i64, i32)> = Vec::new();
        for f in &hits {
            domain_parts.extend(feature_loc_parts(f));
        }
        let regions: Vec<(i64, i64, i32)> = if domain_parts.is_empty() {
            vec![(0, rec_len, 1)]
        } else {
            match inverted(&domain_parts, rec_len) {
                Some(r) => r,
                None => continue,
            }
        };

        // Filter by size.
        let mut regions: Vec<(i64, i64, i32)> =
            regions.into_iter().filter(|&(s, e, _)| (e - s) >= opts.lb.unwrap_or(0)).collect();
        if regions.is_empty() {
            continue;
        }
        if opts.largest {
            regions.sort_by_key(|&(s, e, _)| std::cmp::Reverse(e - s));
            regions.truncate(1);
        }

        for (s, e, st) in regions {
            let loc = Loc::single(s, e, st);
            let mut slice = slice_from_location(rec, &loc, &rec.features, false);
            let mut new_name = rec.id.clone();
            if slice.seq.len() as i64 != rec_len {
                new_name = format!("{}_{}:{}", rec.id, loc.stranded_start_hr(), loc.stranded_end_hr());
            }
            // Protein: never reverse-complement (molecule_type == protein).
            let protein = rec.molecule_type.as_deref() == Some("protein");
            if !protein && opts.keep_direction && st == -1 {
                slice = revcomp_record(&slice);
                slice.name = rec.name.clone();
                slice.id = rec.id.clone();
                if !opts.keep_name {
                    slice.name = new_name.clone();
                    slice.id = new_name.clone();
                }
                out.push(slice);
                continue;
            } else if !protein && st == -1 {
                new_name.push_str("rc");
            }
            if !opts.keep_name {
                slice.name = new_name.clone();
                slice.id = new_name;
            }
            out.push(slice);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seq::{Annotations, Location, Topology};

    fn dom(name: &str, start: i64, end: i64) -> SeqFeature {
        let mut f = SeqFeature::new(DOMAIN_FEATURE_NAME, Location::range(start, end, 1));
        f.set("name", name);
        f.set("database", "db");
        f.set("evalue", "1e-9");
        f.set("score", "50.0");
        f
    }

    fn prot(seq: &[u8], features: Vec<SeqFeature>) -> SeqRecord {
        SeqRecord {
            id: "c".into(),
            name: "c".into(),
            description: String::new(),
            seq: seq.to_vec(),
            molecule_type: Some("protein".into()),
            topology: Topology::Linear,
            annotations: Annotations::default(),
            features,
        }
    }

    #[test]
    fn inverted_middle_domain() {
        // domain [5,10) on length 20 -> gaps [0,5) and [10,20)
        let inv = inverted(&[(5, 10, 1)], 20).unwrap();
        assert_eq!(inv, vec![(0, 5, 1), (10, 20, 1)]);
    }

    #[test]
    fn inverted_whole_returns_none() {
        assert!(inverted(&[(0, 20, 1)], 20).is_none());
    }

    #[test]
    fn extract_two_gaps() {
        let r = prot(b"ACDEFGHIKLMNPQRSTVWY", vec![dom("A", 5, 10)]);
        let out = extract_unannotated(&[r], &ExtractUnannotatedOptions::default()).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].seq, b"ACDEF");
        assert_eq!(out[0].id, "c_1:5");
        assert_eq!(out[1].seq, b"MNPQRSTVWY");
        assert_eq!(out[1].id, "c_11:20");
    }

    #[test]
    fn largest_keeps_one() {
        let r = prot(b"ACDEFGHIKLMNPQRSTVWY", vec![dom("A", 5, 10)]);
        let out = extract_unannotated(
            &[r],
            &ExtractUnannotatedOptions { largest: true, ..Default::default() },
        )
        .unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].seq, b"MNPQRSTVWY"); // the longer gap
    }

    #[test]
    fn lb_filter() {
        let r = prot(b"ACDEFGHIKLMNPQRSTVWY", vec![dom("A", 5, 10)]);
        let out = extract_unannotated(
            &[r],
            &ExtractUnannotatedOptions { lb: Some(6), ..Default::default() },
        )
        .unwrap();
        // only [10,20) (len 10) passes; [0,5) (len 5) filtered
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].seq, b"MNPQRSTVWY");
    }

    #[test]
    fn no_domains_whole_sequence() {
        let r = prot(b"ACDEFGHIKL", vec![]);
        let out = extract_unannotated(&[r], &ExtractUnannotatedOptions::default()).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].seq, b"ACDEFGHIKL");
        assert_eq!(out[0].name, "c"); // whole sequence -> keep name
    }

    #[test]
    fn nucleotide_errors() {
        let mut r = prot(b"ACGT", vec![]);
        r.molecule_type = Some("DNA".into());
        assert!(extract_unannotated(&[r], &ExtractUnannotatedOptions::default()).is_err());
    }
}
