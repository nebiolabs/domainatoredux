//! Taxid resolution (lifted from the prototype `parser.rs`).
//!
//! Resolution order (matching `utils.get_taxid` / `location_taxid`):
//!   1. ` OX=` (UniProt) or ` TaxID=` (UniRef) token in the description;
//!   2. the longest `source` feature with `db_xref="taxon:N"` covering the region;
//!   3. default [`crate::UNIDENTIFIED_TAXID`] (32644), internal-only, never written.

use super::{Location, SeqFeature, SeqRecord};
use crate::UNIDENTIFIED_TAXID;

/// Parse the integer from a feature's first `/db_xref="taxon:NNN"` qualifier.
pub fn feature_taxon(feature: &SeqFeature) -> Option<i64> {
    for (k, vs) in &feature.qualifiers {
        if k == "db_xref" {
            for v in vs {
                if let Some(rest) = v.trim_matches('"').strip_prefix("taxon:") {
                    if let Ok(t) = rest.trim().parse::<i64>() {
                        return Some(t);
                    }
                }
            }
        }
    }
    None
}

/// Parse a taxid from a FASTA description's ` OX=` / ` TaxID=` token.
pub fn parse_ox_taxid(desc: &str) -> Option<i64> {
    for marker in [" OX=", " TaxID="] {
        if let Some(pos) = desc.find(marker) {
            if let Some(tok) = desc[pos + marker.len()..].split_whitespace().next() {
                if let Ok(t) = tok.parse::<i64>() {
                    return Some(t);
                }
            }
        }
    }
    None
}

fn merge_intervals(mut iv: Vec<(i64, i64)>) -> Vec<(i64, i64)> {
    iv.sort();
    let mut out: Vec<(i64, i64)> = Vec::new();
    for (s, e) in iv {
        match out.last_mut() {
            Some(last) if s <= last.1 => {
                if e > last.1 {
                    last.1 = e;
                }
            }
            _ => out.push((s, e)),
        }
    }
    out
}

fn intervals_cover(outer_merged: &[(i64, i64)], inner: &[(i64, i64)]) -> bool {
    inner.iter().all(|&(s, e)| outer_merged.iter().any(|&(os, oe)| os <= s && e <= oe))
}

/// A `source` feature reduced to what taxonomy resolution needs.
struct SourceInfo {
    taxon: Option<i64>,
    merged: Vec<(i64, i64)>,
    length: i64,
}

/// Collect `source` features as [`SourceInfo`], sorted longest-first (so the first
/// one covering a region is the longest covering it — `get_taxid`'s rule).
fn collect_sources(features: &[SeqFeature]) -> Vec<SourceInfo> {
    let mut v: Vec<SourceInfo> = features
        .iter()
        .filter(|f| f.kind == "source")
        .map(|f| {
            let intervals = f.location.intervals();
            let length: i64 = intervals.iter().map(|&(s, e)| e - s).sum();
            SourceInfo { taxon: feature_taxon(f), merged: merge_intervals(intervals), length }
        })
        .collect();
    v.sort_by(|a, b| b.length.cmp(&a.length));
    v
}

fn longest_covering<'a>(sources: &'a [SourceInfo], region: &[(i64, i64)]) -> Option<&'a SourceInfo> {
    sources.iter().find(|src| intervals_cover(&src.merged, region))
}

/// Taxid covering a feature `location`: the taxon of the longest covering `source`
/// (`UNIDENTIFIED_TAXID` if covered by a source without a taxon, `None` if no
/// source covers it).
pub fn location_taxid(features: &[SeqFeature], location: &Location) -> Option<i64> {
    let sources = collect_sources(features);
    longest_covering(&sources, &location.intervals()).map(|s| s.taxon.unwrap_or(UNIDENTIFIED_TAXID))
}

/// Whole-record taxid: ` OX=`/`TaxID=` in the description, else the longest
/// `source` feature's taxon, else [`crate::UNIDENTIFIED_TAXID`].
pub fn record_taxid(record: &SeqRecord) -> i64 {
    if let Some(t) = parse_ox_taxid(&record.description) {
        return t;
    }
    collect_sources(&record.features)
        .first()
        .and_then(|s| s.taxon)
        .unwrap_or(UNIDENTIFIED_TAXID)
}
