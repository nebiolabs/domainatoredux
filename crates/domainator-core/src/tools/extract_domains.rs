//! `extract_domains`: extract domain-annotated regions from annotated contigs,
//! optionally combining or splicing multiple hits, padding, and reorienting.
//! Port of `domainator.extract_domains.extract_domains`.

use std::collections::HashSet;

use crate::seq::{SeqFeature, SeqRecord, Topology};
use crate::tools::extract_common::{pad_location, revcomp_record, slice_from_location, Loc};
use crate::{DOMAIN_FEATURE_NAME, DOMAIN_SEARCH_BEST_HIT_NAME};

/// Options for [`extract_domains`].
#[derive(Default)]
pub struct ExtractDomainsOptions {
    pub evalue: Option<f64>,
    pub score: Option<f64>,
    pub domains: Option<HashSet<String>>,
    pub pad_up: i64,
    pub pad_down: i64,
    pub combine: bool,
    pub keep_direction: bool,
    pub keep_name: bool,
    pub search_hits: bool,
    pub databases: Option<HashSet<String>>,
    pub splice: bool,
}

fn is_protein(rec: &SeqRecord) -> bool {
    rec.molecule_type.as_deref() == Some("protein")
}

/// Which domain features are selected on `rec`, mirroring the Python filter cascade.
fn selected_hits<'a>(rec: &'a SeqRecord, opts: &ExtractDomainsOptions) -> Vec<&'a SeqFeature> {
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
        if let Some(domains) = opts.domains.as_ref().filter(|_| is_dom) {
            let name = feature.get_first("name").unwrap_or("");
            if !domains.contains(name) {
                keep = false;
            }
        } else if opts.search_hits && is_dom && opts.domains.is_none() {
            keep = false;
        } else if let Some(e) = opts.evalue {
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

/// Convert a feature's location to a [`Loc`] (forward sub-parts + overall strand).
fn feature_loc(f: &SeqFeature) -> Loc {
    let parts: Vec<(i64, i64)> = f
        .location
        .parts
        .iter()
        .map(|&(s, e, ..)| if s <= e { (s, e) } else { (e, s) })
        .collect();
    let strand = f.location.parts.first().map(|p| p.2).unwrap_or(1);
    Loc { parts, strand }
}

/// Extract domain regions from all records.
pub fn extract_domains(
    records: &[SeqRecord],
    opts: &ExtractDomainsOptions,
) -> Result<Vec<SeqRecord>, String> {
    if opts.splice && opts.combine {
        return Err("Cannot use --splice and --combine at the same time.".into());
    }
    let mut out = Vec::new();
    for rec in records {
        let rec_len = rec.seq.len() as i64;
        let protein = is_protein(rec);
        let hits = selected_hits(rec, opts);
        if hits.is_empty() {
            continue;
        }

        if opts.combine {
            if rec.topology == Topology::Circular {
                return Err(format!(
                    "Cannot combine domains on circular contigs because the direction to join on is ambiguous (you could go either way around the circle). Contig {} is circular.",
                    rec.id
                ));
            }
            // Domain with the lowest start / highest end.
            let start_i = hits.iter().enumerate().min_by_key(|(_, f)| feature_loc(f).start()).map(|(i, _)| i).unwrap();
            let end_i = hits.iter().enumerate().max_by_key(|(_, f)| feature_loc(f).end()).map(|(i, _)| i).unwrap();
            let start_strand = hits[start_i].location.parts.first().map(|p| p.2).unwrap_or(1);
            let (mut start, mut end) = if start_strand == -1 {
                (feature_loc(hits[start_i]).start() - opts.pad_down, feature_loc(hits[end_i]).end() + opts.pad_up)
            } else {
                (feature_loc(hits[start_i]).start() - opts.pad_up, feature_loc(hits[end_i]).end() + opts.pad_down)
            };
            let new_strand = if opts.keep_direction { 1 } else { start_strand };
            if start < 0 {
                start = 0;
            }
            if end > rec_len {
                end = rec_len;
            }
            let loc = Loc::single(start, end, new_strand);
            let mut slice = slice_from_location(rec, &loc, &rec.features, false);
            let mut new_name = rec.id.clone();
            if slice.seq.len() as i64 != rec_len {
                new_name = format!("{}_{}:{}", rec.id, loc.stranded_start_hr(), loc.stranded_end_hr());
            }
            if !protein && new_strand == -1 {
                new_name.push_str("rc");
            }
            // The name/rc marker goes on the LOCUS name (Python's record.id, which
            // the writer puts on LOCUS); ACCESSION/VERSION are left unchanged.
            if !opts.keep_name {
                slice.id = new_name;
            }
            out.push(slice);
        } else if opts.splice {
            // Merge padded parts of all hit domains; delete intervening sequence.
            let mut splice_parts: Vec<(i64, i64, i32)> = Vec::new();
            for domain in &hits {
                let padded = pad_location(rec, &feature_loc(domain), opts.pad_up, opts.pad_down);
                for &(s, e) in &padded.parts {
                    let mut st = padded.strand;
                    if opts.keep_direction && (st == -1 || st == 0) {
                        st = 1;
                    }
                    splice_parts.push((s, e, st));
                }
            }
            if splice_parts.is_empty() {
                continue;
            }
            let merged = merge_parts(&splice_parts);
            let strand = merged[0].2;
            let loc = Loc { parts: merged.iter().map(|&(s, e, _)| (s, e)).collect(), strand };
            let mut slice = slice_from_location(rec, &loc, &rec.features, true);
            if rec.topology == Topology::Circular {
                slice.topology = Topology::Circular;
            }
            if !opts.keep_name {
                let mut new_name = rec.id.clone();
                if slice.seq.len() as i64 != rec_len {
                    let min_start = merged.iter().map(|p| p.0).min().unwrap();
                    let max_end = merged.iter().map(|p| p.1).max().unwrap();
                    let (start_label, end_label) = if strand == -1 {
                        (max_end, min_start + 1)
                    } else {
                        (min_start + 1, max_end)
                    };
                    new_name = format!("{}_{}:{}", rec.id, start_label, end_label);
                    if !protein && strand == -1 && !opts.keep_direction {
                        new_name.push_str("rc");
                    }
                }
                slice.id = new_name;
            }
            out.push(slice);
        } else {
            // One region per hit domain.
            for domain in &hits {
                let strand = domain.location.parts.first().map(|p| p.2).unwrap_or(1);
                let loc = pad_location(rec, &feature_loc(domain), opts.pad_up, opts.pad_down);
                let mut slice = slice_from_location(rec, &loc, &rec.features, false);
                let mut new_name = rec.id.clone();
                if slice.seq.len() as i64 != rec_len {
                    new_name = format!("{}_{}:{}", rec.id, loc.stranded_start_hr(), loc.stranded_end_hr());
                }
                if !protein && opts.keep_direction && strand == -1 {
                    // slice already flipped to forward; flip back to original.
                    slice = revcomp_record(&slice);
                    if !opts.keep_name {
                        slice.id = new_name;
                    }
                    out.push(slice);
                    continue;
                } else if !protein && strand == -1 {
                    new_name.push_str("rc");
                }
                if !opts.keep_name {
                    slice.id = new_name;
                }
                out.push(slice);
            }
        }
    }
    Ok(out)
}

/// `CompoundLocation.merge_parts`: sort by start, merge overlapping/adjacent
/// intervals (keeping strand of the first).
fn merge_parts(parts: &[(i64, i64, i32)]) -> Vec<(i64, i64, i32)> {
    let mut sorted = parts.to_vec();
    sorted.sort_by_key(|p| (p.0, p.1));
    let mut merged: Vec<(i64, i64, i32)> = Vec::new();
    for (s, e, st) in sorted {
        if let Some(last) = merged.last_mut() {
            if s <= last.1 {
                last.1 = last.1.max(e);
                continue;
            }
        }
        merged.push((s, e, st));
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seq::{Annotations, Location, Topology};

    fn dom(name: &str, start: i64, end: i64, strand: i32, db: &str) -> SeqFeature {
        let mut f = SeqFeature::new(DOMAIN_FEATURE_NAME, Location::range(start, end, strand));
        f.set("name", name);
        f.set("database", db);
        f.set("evalue", "1e-9");
        f.set("score", "50.0");
        f
    }

    fn rec(seq: &[u8], features: Vec<SeqFeature>, protein: bool) -> SeqRecord {
        SeqRecord {
            id: "c".into(),
            name: "c".into(),
            description: String::new(),
            seq: seq.to_vec(),
            molecule_type: Some(if protein { "protein" } else { "DNA" }.into()),
            topology: Topology::Linear,
            annotations: Annotations::default(),
            features,
        }
    }

    #[test]
    fn single_forward_protein() {
        // protein 20 aa, domain [5,10)
        let r = rec(b"ACDEFGHIKLMNPQRSTVWY", vec![dom("A", 5, 10, 1, "db")], true);
        let opts = ExtractDomainsOptions { domains: Some(["A".to_string()].into_iter().collect()), ..Default::default() };
        let out = extract_domains(&[r], &opts).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].seq, b"GHIKL"); // indices 5..10
        assert_eq!(out[0].id, "c_6:10");
    }

    #[test]
    fn single_reverse_dna_rc_suffix() {
        // DNA 12 nt, domain [0,6) reverse strand.
        let r = rec(b"AAACCCGGGTTT", vec![dom("A", 0, 6, -1, "db")], false);
        let opts = ExtractDomainsOptions { domains: Some(["A".to_string()].into_iter().collect()), ..Default::default() };
        let out = extract_domains(&[r], &opts).unwrap();
        // seq[0:6]="AAACCC"; revcomp -> "GGGTTT"
        assert_eq!(out[0].seq, b"GGGTTT");
        assert_eq!(out[0].id, "c_6:1rc"); // stranded_start_hr = end = 6, stranded_end_hr = start+1 = 1
    }

    #[test]
    fn keep_direction_reverse_dna() {
        let r = rec(b"AAACCCGGGTTT", vec![dom("A", 0, 6, -1, "db")], false);
        let opts = ExtractDomainsOptions {
            domains: Some(["A".to_string()].into_iter().collect()),
            keep_direction: true,
            ..Default::default()
        };
        let out = extract_domains(&[r], &opts).unwrap();
        // keep_direction flips back to forward orientation "AAACCC"
        assert_eq!(out[0].seq, b"AAACCC");
        assert_eq!(out[0].id, "c_6:1"); // no rc suffix
    }

    #[test]
    fn combine_two_domains() {
        let r = rec(b"ACDEFGHIKLMNPQRSTVWY", vec![dom("A", 2, 5, 1, "db"), dom("B", 12, 15, 1, "db")], true);
        let opts = ExtractDomainsOptions {
            domains: Some(["A".to_string(), "B".to_string()].into_iter().collect()),
            combine: true,
            ..Default::default()
        };
        let out = extract_domains(&[r], &opts).unwrap();
        // envelope [2,15)
        assert_eq!(out[0].seq, b"DEFGHIKLMNPQR");
        assert_eq!(out[0].id, "c_3:15");
    }

    #[test]
    fn splice_two_domains() {
        let r = rec(b"ACDEFGHIKLMNPQRSTVWY", vec![dom("A", 2, 5, 1, "db"), dom("B", 12, 15, 1, "db")], true);
        let opts = ExtractDomainsOptions {
            domains: Some(["A".to_string(), "B".to_string()].into_iter().collect()),
            splice: true,
            ..Default::default()
        };
        let out = extract_domains(&[r], &opts).unwrap();
        // splice [2,5)+[12,15) -> "DEF"+"PQR" = "DEFPQR"
        assert_eq!(out[0].seq, b"DEFPQR");
    }

    #[test]
    fn splice_combine_conflict_errors() {
        let r = rec(b"ACDEF", vec![dom("A", 0, 3, 1, "db")], true);
        let opts = ExtractDomainsOptions { splice: true, combine: true, ..Default::default() };
        assert!(extract_domains(&[r], &opts).is_err());
    }
}
