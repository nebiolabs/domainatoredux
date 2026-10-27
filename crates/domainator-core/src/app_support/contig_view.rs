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

use crate::seq::taxid::record_taxid;
use crate::seq::{Location, SeqFeature, SeqRecord, Topology};
use crate::UNIDENTIFIED_TAXID;

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

/// One row of a record's header summary (gutter hover / contig inspector).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct InfoRow {
    /// Flat-file section name (`LOCUS`, `DEFINITION`, `REFERENCE 1`, ...).
    pub key: String,
    pub value: String,
    /// Too long for a hover: shown only in the right-click inspector.
    pub detail: bool,
}

/// One contig (a `SeqRecord`) and its drawable features.
#[derive(Debug, Clone, Serialize)]
pub struct ContigView {
    /// Position of this record in the file (stable id for provenance handles).
    pub index: usize,
    pub id: String,
    /// VERSION (or ACCESSION) for GenBank; the id for FASTA.
    pub name: String,
    pub description: String,
    /// DEFINITION for a label: `description` without a leading copy of the id
    /// (a FASTA title carries the id as its first word).
    pub definition: String,
    /// Primary ACCESSION.
    pub accession: Option<String>,
    /// SOURCE, else ORGANISM.
    pub source: Option<String>,
    pub length: usize,
    /// `"linear"` or `"circular"`.
    pub topology: String,
    pub is_protein: bool,
    /// Header annotations, in flat-file order.
    pub info: Vec<InfoRow>,
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
        .map(|&(start, end, strand, before, after)| PartView {
            start,
            end,
            strand,
            before,
            after,
        })
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
        .unwrap_or_else(|| {
            (
                crate::tools::plot_contigs::default_feature_color(&f.kind).to_string(),
                "default".to_string(),
            )
        });

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

/// `description` minus a leading `id` (and the space after it).
fn definition(rec: &SeqRecord) -> String {
    let d = rec.description.trim();
    if d == rec.id {
        return String::new();
    }
    match d.strip_prefix(rec.id.as_str()) {
        Some(rest) if !rec.id.is_empty() && rest.starts_with(char::is_whitespace) => {
            rest.trim_start().to_string()
        }
        _ => d.to_string(),
    }
}

/// The record's header annotations as display rows, in flat-file order. Empty
/// sections are skipped; references and comments are marked `detail`.
fn record_info(rec: &SeqRecord) -> Vec<InfoRow> {
    let a = &rec.annotations;
    let mut rows = Vec::new();
    let mut push = |key: &str, value: String, detail: bool| {
        if !value.trim().is_empty() {
            rows.push(InfoRow {
                key: key.to_string(),
                value,
                detail,
            });
        }
    };
    let unit = if rec.is_protein() { "aa" } else { "bp" };
    let topology = a.topology.unwrap_or(rec.topology);
    let locus: Vec<String> = [
        Some(rec.id.clone()),
        Some(format!("{} {unit}", rec.len())),
        rec.molecule_type.clone(),
        Some(topology.as_str().to_string()),
        a.division.clone(),
        a.date.clone(),
    ]
    .into_iter()
    .flatten()
    .filter(|s| !s.is_empty())
    .collect();
    push("LOCUS", locus.join("  "), false);
    push("DEFINITION", definition(rec), false);
    push("ACCESSION", a.accessions.join(" "), false);
    if a.sequence_version.is_some() {
        let gi =
            a.gi.as_deref()
                .map(|g| format!("  GI:{g}"))
                .unwrap_or_default();
        push("VERSION", format!("{}{gi}", rec.name), false);
    }
    push("DBLINK", a.dblink.join("\n"), false);
    let keywords: Vec<&str> = a
        .keywords
        .iter()
        .map(|k| k.trim())
        .filter(|k| !k.is_empty() && *k != ".")
        .collect();
    push("KEYWORDS", keywords.join("; "), false);
    push("DBSOURCE", a.db_source.clone().unwrap_or_default(), false);
    push("SOURCE", a.source.clone().unwrap_or_default(), false);
    push("ORGANISM", a.organism.clone().unwrap_or_default(), false);
    push("Lineage", a.taxonomy.join("; "), false);
    let taxid = record_taxid(rec);
    if taxid != UNIDENTIFIED_TAXID {
        push("Taxid", taxid.to_string(), false);
    }
    for (i, r) in a.references.iter().enumerate() {
        let lines: Vec<String> = [
            r.location_text.clone().unwrap_or_default(),
            r.authors.clone(),
            r.consrtm.clone(),
            r.title.clone(),
            r.journal.clone(),
            if r.pubmed_id.is_empty() {
                String::new()
            } else {
                format!("PUBMED {}", r.pubmed_id)
            },
            r.remark.clone(),
        ]
        .into_iter()
        .filter(|s| !s.trim().is_empty())
        .collect();
        push(&format!("REFERENCE {}", i + 1), lines.join("\n"), true);
    }
    push("COMMENT", a.comment.clone().unwrap_or_default(), true);
    for (block, kvs) in &a.structured_comment {
        let lines: Vec<String> = kvs.iter().map(|(k, v)| format!("{k} :: {v}")).collect();
        push(block, lines.join("\n"), true);
    }
    rows
}

fn contig_view(index: usize, rec: &SeqRecord) -> ContigView {
    ContigView {
        index,
        id: rec.id.clone(),
        name: rec.name.clone(),
        description: rec.description.clone(),
        definition: definition(rec),
        accession: rec.annotations.accessions.first().cloned(),
        source: rec
            .annotations
            .source
            .clone()
            .or_else(|| rec.annotations.organism.clone())
            .filter(|s| !s.trim().is_empty()),
        length: rec.len(),
        topology: match rec.topology {
            Topology::Linear => "linear",
            Topology::Circular => "circular",
        }
        .to_string(),
        is_protein: rec.is_protein(),
        info: record_info(rec),
        features: rec
            .features
            .iter()
            .enumerate()
            .map(|(i, f)| feature_view(i, f))
            .collect(),
    }
}

/// Build the full view for a parsed file. `molecule_type` is taken from the first
/// record; the union of `database` values, the set of feature kinds, and the
/// evalue range are computed for the filter UI.
pub fn build_contig_file_view(path: &str, records: &[SeqRecord]) -> ContigFileView {
    let contigs: Vec<ContigView> = records
        .iter()
        .enumerate()
        .map(|(i, r)| contig_view(i, r))
        .collect();

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
    use crate::seq::{Annotations, Location, Reference};
    use crate::DOMAIN_FEATURE_NAME;

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
        let fv = feature_view(
            0,
            &feat(
                DOMAIN_FEATURE_NAME,
                &[("database", "Pfam"), ("evalue", "1e-20")],
            ),
        );
        assert_eq!(fv.database.as_deref(), Some("Pfam"));
        assert_eq!(fv.evalue, Some(1e-20));
    }

    fn record(id: &str, name: &str, description: &str) -> SeqRecord {
        SeqRecord {
            id: id.to_string(),
            name: name.to_string(),
            description: description.to_string(),
            seq: b"ACGTACGT".to_vec(),
            molecule_type: Some("DNA".to_string()),
            topology: Topology::Linear,
            annotations: Annotations::default(),
            features: Vec::new(),
        }
    }

    #[test]
    fn genbank_header_reaches_the_view() {
        let mut rec = record(
            "BX548174_369054:361090rc",
            "BX548174.1",
            "Prochlorococcus marinus MED4 complete genome",
        );
        let a = &mut rec.annotations;
        a.accessions = vec!["BX548174".into()];
        a.sequence_version = Some(1);
        a.division = Some("UNK".into());
        a.date = Some("14-FEB-2024".into());
        a.dblink = vec![
            "BioProject: PRJNA213".into(),
            "BioSample: SAMEA3138209".into(),
        ];
        a.keywords = vec![String::new()];
        a.source = Some("Prochlorococcus marinus MED4".into());
        a.organism = Some("Prochlorococcus marinus".into());
        a.taxonomy = vec!["Bacteria".into(), "Cyanobacteriota".into()];
        a.references = vec![Reference {
            location_text: Some("(bases 1 to 8)".into()),
            title: "A genome".into(),
            pubmed_id: "123".into(),
            ..Reference::default()
        }];
        a.comment = Some("Annotated by hand".into());
        a.topology = Some(Topology::Circular);

        let cv = contig_view(0, &rec);
        assert_eq!(cv.name, "BX548174.1");
        assert_eq!(cv.accession.as_deref(), Some("BX548174"));
        assert_eq!(cv.source.as_deref(), Some("Prochlorococcus marinus MED4"));
        assert_eq!(cv.definition, rec.description);
        let keys: Vec<(&str, bool)> = cv.info.iter().map(|r| (r.key.as_str(), r.detail)).collect();
        assert_eq!(
            keys,
            [
                ("LOCUS", false),
                ("DEFINITION", false),
                ("ACCESSION", false),
                ("VERSION", false),
                ("DBLINK", false),
                ("SOURCE", false),
                ("ORGANISM", false),
                ("Lineage", false),
                ("REFERENCE 1", true),
                ("COMMENT", true),
            ]
        );
        // The declared LOCUS topology wins over the geometric default.
        assert_eq!(
            cv.info[0].value,
            "BX548174_369054:361090rc  8 bp  DNA  circular  UNK  14-FEB-2024"
        );
        assert_eq!(
            cv.info[4].value,
            "BioProject: PRJNA213\nBioSample: SAMEA3138209"
        );
        assert_eq!(cv.info[8].value, "(bases 1 to 8)\nA genome\nPUBMED 123");
    }

    #[test]
    fn fasta_definition_drops_the_id() {
        let cv = contig_view(0, &record("seq1", "seq1", "seq1 some protein OS=E. coli"));
        assert_eq!(cv.definition, "some protein OS=E. coli");
        assert_eq!(cv.accession, None);
        assert_eq!(cv.source, None);
        let keys: Vec<&str> = cv.info.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(keys, ["LOCUS", "DEFINITION"]);
        assert_eq!(
            contig_view(0, &record("seq1", "seq1", "seq1")).definition,
            ""
        );
        assert_eq!(
            contig_view(0, &record("seq1", "seq1", "seq10 x")).definition,
            "seq10 x"
        );
    }

    #[test]
    fn cds_name_precedence() {
        assert_eq!(
            feature_name(&feat("CDS", &[("locus_tag", "b0001"), ("gene", "thrA")])),
            "b0001"
        );
        assert_eq!(feature_name(&feat("CDS", &[("gene", "thrA")])), "thrA");
        assert_eq!(
            feature_name(&feat("source", &[("organism", "E. coli")])),
            "E. coli"
        );
    }
}
