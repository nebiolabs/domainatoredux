//! `extract_domains`: extract domain-annotated regions from annotated contigs,
//! optionally combining or splicing multiple hits, padding, and reorienting.
//! Port of `domainator.extract_domains.extract_domains`.

use std::collections::{HashMap, HashSet};

use crate::seq::partial::{cds_fragment_status, Partial};
use crate::seq::taxonomy::TaxonomyFilter;
use crate::seq::{SeqFeature, SeqRecord, Topology};
use crate::tools::extract_common::{
    pad_location, revcomp_record, slice_from_location, slice_from_parts, Loc,
};
use crate::tools::neighborhood::{region_id, Env};
use crate::tools::select_common::{get_cds_unique_name, AnnotationType};
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
    /// `--keep_parent_id`: name each region after its record, not its coordinates.
    pub keep_parent_id: bool,
    /// Which annotations to extract.
    pub annotation_type: AnnotationType,
    /// `--has_search_hit`: also extract the search hits, whatever their names or
    /// databases; alone (no `domains`), only them.
    pub has_search_hit: bool,
    pub databases: Option<HashSet<String>>,
    pub splice: bool,
    /// `--partial`: domains by the fragment status of the CDS or protein they are on.
    pub partial: Partial,
    /// Taxonomy restriction on each annotation, judged before extension, combining
    /// or splicing; a protein's by the record's taxid.
    pub taxonomy: Option<TaxonomyFilter>,
}

fn is_protein(rec: &SeqRecord) -> bool {
    rec.molecule_type.as_deref() == Some("protein")
}

/// Which domain features are selected on `rec`, mirroring the Python filter cascade.
///
/// An annotation is a candidate when it is of `annotation_type` and passes `domains`
/// and `databases`, or when it is a search hit under `has_search_hit` (which, without
/// `domains`, makes the search hits the only candidates). Python's `elif` chain then
/// stops at the first test that rejects, so a candidate is kept only when every test
/// passes: `--evalue`, `--min_score`, `--partial`, then taxonomy.
fn selected_hits<'a>(
    rec: &'a SeqRecord,
    opts: &ExtractDomainsOptions,
) -> Result<Vec<&'a SeqFeature>, String> {
    // `--partial` judges a domain by the CDS (or protein record) it is on.
    let cds_by_id: HashMap<String, &SeqFeature> = if opts.partial.is_include() {
        HashMap::new()
    } else {
        rec.features
            .iter()
            .filter(|f| f.kind == "CDS")
            .map(|f| (get_cds_unique_name(f), f))
            .collect()
    };
    let mut hits = Vec::new();
    for feature in &rec.features {
        if feature.kind != DOMAIN_FEATURE_NAME && feature.kind != DOMAIN_SEARCH_BEST_HIT_NAME {
            continue;
        }
        let name = feature.get_first("name").unwrap_or("");
        let search_hit = opts.has_search_hit && feature.kind == DOMAIN_SEARCH_BEST_HIT_NAME;
        let selected = opts.annotation_type.includes(&feature.kind)
            && (opts.domains.is_some() || !opts.has_search_hit)
            && opts.domains.as_ref().is_none_or(|d| d.contains(name))
            && opts.databases.as_ref().is_none_or(|dbs| {
                feature
                    .get_first("database")
                    .is_some_and(|d| dbs.contains(d))
            });
        if !(selected || search_hit) {
            continue;
        }
        let evalue: f64 = feature
            .get_first("evalue")
            .and_then(|v| v.parse().ok())
            .unwrap_or(f64::INFINITY);
        let score: f64 = feature
            .get_first("score")
            .and_then(|v| v.parse().ok())
            .unwrap_or(f64::NEG_INFINITY);
        let rejected =
            opts.evalue.is_some_and(|e| evalue > e) || opts.score.is_some_and(|s| score < s);
        let keep = if rejected {
            false
        } else if opts.partial.is_include() {
            true
        } else {
            let status = if is_protein(rec) {
                cds_fragment_status(rec, None)
            } else {
                match cds_by_id.get(feature.get_first("cds_id").unwrap_or(".")) {
                    Some(cds) => cds_fragment_status(rec, Some(cds)),
                    None => None, // a contig-level nucleotide hit is never on a fragment
                }
            };
            opts.partial.allows(status)
        };
        let keep = match (&opts.taxonomy, keep) {
            (Some(taxonomy), true) => taxonomy.matches(rec, Some(feature))?,
            _ => keep,
        };
        if keep {
            hits.push(feature);
        }
    }
    Ok(hits)
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
        let hits = selected_hits(rec, opts)?;
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
            let start_i = hits
                .iter()
                .enumerate()
                .min_by_key(|(_, f)| feature_loc(f).start())
                .map(|(i, _)| i)
                .unwrap();
            let end_i = hits
                .iter()
                .enumerate()
                .max_by_key(|(_, f)| feature_loc(f).end())
                .map(|(i, _)| i)
                .unwrap();
            let start_strand = hits[start_i]
                .location
                .parts
                .first()
                .map(|p| p.2)
                .unwrap_or(1);
            let (mut start, mut end) = if start_strand == -1 {
                (
                    feature_loc(hits[start_i]).start() - opts.pad_down,
                    feature_loc(hits[end_i]).end() + opts.pad_up,
                )
            } else {
                (
                    feature_loc(hits[start_i]).start() - opts.pad_up,
                    feature_loc(hits[end_i]).end() + opts.pad_down,
                )
            };
            let new_strand = if opts.keep_direction { 1 } else { start_strand };
            if start < 0 {
                start = 0;
            }
            if end > rec_len {
                end = rec_len;
            }
            let loc = Loc::single(start, end, new_strand);
            let mut slice = slice_from_location(rec, &loc, &rec.features);
            // The region id goes on the LOCUS name (Python's record.id);
            // ACCESSION/VERSION are left unchanged.
            if !opts.keep_parent_id {
                slice.id = loc.region_id(rec, false);
            }
            out.push(slice);
        } else if opts.splice {
            // Glue the padded domains together, deleting the sequence between them.
            // As for --combine, the domain with the lowest start decides the direction,
            // and the domains are glued in the order they are read: right to left on
            // the reverse strand. A domain's own parts are already in reading order,
            // which keeps one spanning the origin of a circular contig intact.
            let first_strand = hits
                .iter()
                .min_by_key(|f| feature_loc(f).start())
                .map(|f| feature_loc(f).strand)
                .unwrap_or(1);
            let reverse = !opts.keep_direction && first_strand == -1;
            let mut ordered = hits.clone();
            if reverse {
                ordered.sort_by_key(|f| std::cmp::Reverse(feature_loc(f).parts[0].1));
            } else {
                ordered.sort_by_key(|f| feature_loc(f).parts[0].0);
            }
            let mut splice_parts: Vec<Env> = Vec::new();
            for domain in &ordered {
                let padded = pad_location(rec, &feature_loc(domain), opts.pad_up, opts.pad_down);
                let mut parts: Vec<Env> = padded
                    .parts
                    .iter()
                    .map(|&(start, end)| Env {
                        start,
                        end,
                        strand: padded.strand,
                    })
                    .collect();
                if opts.keep_direction {
                    // Reverse-strand parts are read right to left, so read them left
                    // to right on the forward strand.
                    if padded.strand == -1 {
                        parts.reverse();
                    }
                    for p in &mut parts {
                        p.strand = 1;
                    }
                }
                splice_parts.extend(parts);
            }
            if splice_parts.is_empty() {
                continue;
            }
            let merged = merge_parts(&splice_parts);
            let mut slice = slice_from_parts(rec, merged.clone(), &rec.features);
            if rec.topology == Topology::Circular {
                slice.topology = Topology::Circular;
            }
            if !opts.keep_parent_id {
                slice.id = region_id(rec, &merged, false);
            }
            out.push(slice);
        } else {
            // One region per hit domain.
            for domain in &hits {
                let strand = domain.location.parts.first().map(|p| p.2).unwrap_or(1);
                let loc = pad_location(rec, &feature_loc(domain), opts.pad_up, opts.pad_down);
                let mut slice = slice_from_location(rec, &loc, &rec.features);
                // The slice reads a reverse-strand domain forward; flip it back to keep
                // the original direction.
                let flip = !protein && opts.keep_direction && strand == -1;
                if flip {
                    slice = revcomp_record(&slice);
                }
                if !opts.keep_parent_id {
                    slice.id = loc.region_id(rec, flip);
                }
                out.push(slice);
            }
        }
    }
    Ok(out)
}

/// `location_ops.merge_parts`: merge consecutive parts on the same strand that
/// touch or overlap. Parts are not sorted, which would scramble a join.
fn merge_parts(parts: &[Env]) -> Vec<Env> {
    let mut merged: Vec<Env> = Vec::new();
    for &part in parts {
        if let Some(last) = merged.last_mut() {
            if part.strand == last.strand && part.start <= last.end && last.start <= part.end {
                last.start = last.start.min(part.start);
                last.end = last.end.max(part.end);
                continue;
            }
        }
        merged.push(part);
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
        let r = rec(
            b"ACDEFGHIKLMNPQRSTVWY",
            vec![dom("A", 5, 10, 1, "db")],
            true,
        );
        let opts = ExtractDomainsOptions {
            domains: Some(["A".to_string()].into_iter().collect()),
            ..Default::default()
        };
        let out = extract_domains(&[r], &opts).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].seq, b"GHIKL"); // indices 5..10
        assert_eq!(out[0].id, "c_6:10");
    }

    #[test]
    fn single_reverse_dna_rc_suffix() {
        // DNA 12 nt, domain [0,6) reverse strand.
        let r = rec(b"AAACCCGGGTTT", vec![dom("A", 0, 6, -1, "db")], false);
        let opts = ExtractDomainsOptions {
            domains: Some(["A".to_string()].into_iter().collect()),
            ..Default::default()
        };
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
        // Named in the parent's direction, with no rc suffix.
        assert_eq!(out[0].id, "c_1:6");
    }

    #[test]
    fn combine_two_domains() {
        let r = rec(
            b"ACDEFGHIKLMNPQRSTVWY",
            vec![dom("A", 2, 5, 1, "db"), dom("B", 12, 15, 1, "db")],
            true,
        );
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
        let r = rec(
            b"ACDEFGHIKLMNPQRSTVWY",
            vec![dom("A", 2, 5, 1, "db"), dom("B", 12, 15, 1, "db")],
            true,
        );
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
        let opts = ExtractDomainsOptions {
            splice: true,
            combine: true,
            ..Default::default()
        };
        assert!(extract_domains(&[r], &opts).is_err());
    }
}
