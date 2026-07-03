//! Serde view-model for the GUI contig/annotation viewer.
//!
//! Turns a parsed [`SeqRecord`] file into a JSON-friendly [`ContigFileView`] with
//! feature colors resolved server-side (so the frontend never re-implements the
//! precedence rules) and the hit qualifiers most useful for filtering
//! (`database`/`evalue`/`score`/`identity`) flattened onto each feature. This
//! mirrors `plot_contigs.py`'s `feature_to_dict` / `get_feature_name` /
//! `get_feature_mouseover` so the GUI renders the same names, colors, and tooltips
//! as the Python plotter.

use serde::Serialize;

use crate::seq::{Location, SeqFeature, SeqRecord, Topology};
use crate::{DOMAIN_FEATURE_NAME, DOMAIN_SEARCH_BEST_HIT_NAME};

/// One span of a feature location (0-based half-open), named for JSON clarity
/// instead of the core tuple `Part = (i64,i64,i32,bool,bool)`.
#[derive(Debug, Clone, Serialize)]
pub struct PartView {
    pub start: i64,
    pub end: i64,
    /// `1` forward, `-1` reverse, `0` unstranded.
    pub strand: i32,
    pub before: bool,
    pub after: bool,
}

/// A single feature, ready to draw: resolved color, display name, flattened filter
/// keys, and the full ordered qualifier list for the tooltip.
#[derive(Debug, Clone, Serialize)]
pub struct FeatureView {
    /// Index into the record's `features` vec (write-back key for later editing).
    pub feat_index: usize,
    pub kind: String,
    pub label: String,
    pub parts: Vec<PartView>,
    pub start: i64,
    pub end: i64,
    pub strand: i32,
    /// Resolved `#RRGGBB`.
    pub color: String,
    /// Where `color` came from: `"qualifier"` or `"default"`.
    pub color_source: String,
    // Flattened hit qualifiers for instant client-side filtering (None if absent):
    pub database: Option<String>,
    pub evalue: Option<f64>,
    pub score: Option<f64>,
    pub identity: Option<f64>,
    pub cds_id: Option<String>,
    /// Every qualifier as `(key, "v1; v2")`, insertion order preserved (tooltip).
    pub qualifiers: Vec<(String, String)>,
}

/// One contig (a `SeqRecord`) and its drawable features.
#[derive(Debug, Clone, Serialize)]
pub struct ContigView {
    /// Position of this record in the file (stable id for provenance handles).
    pub index: usize,
    pub id: String,
    pub name: String,
    pub description: String,
    pub length: usize,
    /// `"linear"` or `"circular"`.
    pub topology: String,
    pub is_protein: bool,
    pub features: Vec<FeatureView>,
}

/// A whole sequence file, plus filter-UI bounds computed over all its features.
#[derive(Debug, Clone, Serialize)]
pub struct ContigFileView {
    pub path: String,
    pub molecule_type: Option<String>,
    pub contigs: Vec<ContigView>,
    /// Distinct `database` qualifier values → database filter checklist.
    pub databases: Vec<String>,
    /// Distinct feature kinds present → kind legend/filter.
    pub feature_kinds: Vec<String>,
    /// `[min, max]` over all hit evalues → evalue slider bounds (None if no hits).
    pub evalue_range: Option<[f64; 2]>,
}

/// Per-kind default fill, mirroring `plot_contigs.py` `feature_to_dict`'s
/// `default_color` map (fallback `#CCCCCC`).
fn default_color(kind: &str) -> &'static str {
    match kind {
        "CDS" | "exon" => "#CCCCCC",
        "repeat_region" => "#0000FF",
        "source" => "#2626FF",
        k if k == DOMAIN_FEATURE_NAME => "#00FFFF",
        k if k == DOMAIN_SEARCH_BEST_HIT_NAME => "#06FF0D",
        _ => "#CCCCCC",
    }
}

/// Normalize a `(#?)([A-Fa-f0-9]{6})` hex string to `#RRGGBB` (uppercase), matching
/// `color_genbank.normalize_color_hex`. Returns `None` if the value isn't a valid
/// 6-digit hex (the Python raises; here we fall back to the default instead).
fn normalize_color_hex(raw: &str) -> Option<String> {
    let s = raw.trim().trim_start_matches('#');
    if s.len() == 6 && s.bytes().all(|b| b.is_ascii_hexdigit()) {
        Some(format!("#{}", s.to_ascii_uppercase()))
    } else {
        None
    }
}

/// Display name for a feature, mirroring `plot_contigs.py` `get_feature_name`
/// (+ `utils.get_cds_name` precedence).
fn feature_name(f: &SeqFeature) -> String {
    let first = |k: &str| f.get_first(k).map(str::to_string);
    match f.kind.as_str() {
        "CDS" | "gene" => first("gene_id")
            .or_else(|| first("locus_tag"))
            .or_else(|| first("gene"))
            .or_else(|| first("protein_id"))
            .or_else(|| first("cds_id"))
            .unwrap_or_else(|| f.kind.clone()),
        "source" => first("organism").unwrap_or_else(|| f.kind.clone()),
        _ => first("name")
            .or_else(|| first("product"))
            .or_else(|| first("gene"))
            .or_else(|| first("locus_tag"))
            .unwrap_or_else(|| f.kind.clone()),
    }
}

fn part_views(loc: &Location) -> Vec<PartView> {
    loc.parts
        .iter()
        .map(|&(start, end, strand, before, after)| PartView { start, end, strand, before, after })
        .collect()
}

fn parse_f64(f: &SeqFeature, key: &str) -> Option<f64> {
    f.get_first(key).and_then(|v| v.trim().parse::<f64>().ok())
}

fn feature_view(feat_index: usize, f: &SeqFeature) -> FeatureView {
    let (color, color_source) = f
        .get_first("color")
        .or_else(|| f.get_first("Color"))
        .and_then(normalize_color_hex)
        .map(|c| (c, "qualifier".to_string()))
        .unwrap_or_else(|| (default_color(&f.kind).to_string(), "default".to_string()));

    let qualifiers = f
        .qualifiers
        .iter()
        .map(|(k, vs)| (k.clone(), vs.join("; ")))
        .collect();

    FeatureView {
        feat_index,
        kind: f.kind.clone(),
        label: feature_name(f),
        parts: part_views(&f.location),
        start: f.location.start(),
        end: f.location.end(),
        strand: f.location.strand(),
        color,
        color_source,
        database: f.get_first("database").map(str::to_string),
        evalue: parse_f64(f, "evalue"),
        score: parse_f64(f, "score"),
        identity: parse_f64(f, "identity"),
        cds_id: f.get_first("cds_id").map(str::to_string),
        qualifiers,
    }
}

fn contig_view(index: usize, rec: &SeqRecord) -> ContigView {
    ContigView {
        index,
        id: rec.id.clone(),
        name: rec.name.clone(),
        description: rec.description.clone(),
        length: rec.len(),
        topology: match rec.topology {
            Topology::Linear => "linear",
            Topology::Circular => "circular",
        }
        .to_string(),
        is_protein: rec.is_protein(),
        features: rec.features.iter().enumerate().map(|(i, f)| feature_view(i, f)).collect(),
    }
}

/// Build the full view for a parsed file. `molecule_type` is taken from the first
/// record; the union of `database` values, the set of feature kinds, and the
/// evalue range are computed for the filter UI.
pub fn build_contig_file_view(path: &str, records: &[SeqRecord]) -> ContigFileView {
    let contigs: Vec<ContigView> =
        records.iter().enumerate().map(|(i, r)| contig_view(i, r)).collect();

    let mut databases: Vec<String> = Vec::new();
    let mut feature_kinds: Vec<String> = Vec::new();
    let mut evalue_min: Option<f64> = None;
    let mut evalue_max: Option<f64> = None;

    for contig in &contigs {
        for f in &contig.features {
            if !feature_kinds.contains(&f.kind) {
                feature_kinds.push(f.kind.clone());
            }
            if let Some(db) = &f.database {
                if !databases.contains(db) {
                    databases.push(db.clone());
                }
            }
            if let Some(e) = f.evalue {
                evalue_min = Some(evalue_min.map_or(e, |m: f64| m.min(e)));
                evalue_max = Some(evalue_max.map_or(e, |m: f64| m.max(e)));
            }
        }
    }
    databases.sort();
    feature_kinds.sort();

    ContigFileView {
        path: path.to_string(),
        molecule_type: records.first().and_then(|r| r.molecule_type.clone()),
        contigs,
        databases,
        feature_kinds,
        evalue_range: match (evalue_min, evalue_max) {
            (Some(lo), Some(hi)) => Some([lo, hi]),
            _ => None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seq::Location;

    fn feat(kind: &str, quals: &[(&str, &str)]) -> SeqFeature {
        let mut f = SeqFeature::new(kind, Location::range(0, 100, 1));
        for (k, v) in quals {
            f.push(k, *v);
        }
        f
    }

    #[test]
    fn color_qualifier_wins_and_normalizes() {
        let fv = feature_view(0, &feat("CDS", &[("Color", "aabbcc")]));
        assert_eq!(fv.color, "#AABBCC");
        assert_eq!(fv.color_source, "qualifier");
    }

    #[test]
    fn falls_back_to_kind_default() {
        let fv = feature_view(0, &feat(DOMAIN_FEATURE_NAME, &[]));
        assert_eq!(fv.color, "#00FFFF");
        assert_eq!(fv.color_source, "default");
    }

    #[test]
    fn flattens_hit_qualifiers() {
        let fv = feature_view(0, &feat(DOMAIN_FEATURE_NAME, &[("database", "Pfam"), ("evalue", "1e-20")]));
        assert_eq!(fv.database.as_deref(), Some("Pfam"));
        assert_eq!(fv.evalue, Some(1e-20));
    }

    #[test]
    fn cds_name_precedence() {
        assert_eq!(feature_name(&feat("CDS", &[("locus_tag", "b0001"), ("gene", "thrA")])), "b0001");
        assert_eq!(feature_name(&feat("CDS", &[("gene", "thrA")])), "thrA");
        assert_eq!(feature_name(&feat("source", &[("organism", "E. coli")])), "E. coli");
    }
}
