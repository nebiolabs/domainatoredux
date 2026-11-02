//! `find_features`: annotate each CDS or protein with features found by an algorithm
//! rather than a reference search. Port of upstream `find_features.py`.
//!
//! Two algorithms: `--motif`, PROSITE patterns, each match a `Domainator` feature with
//! program and database `MOTIF`, score and identity 100 and e-value 0; and `--tmbed`,
//! TMbed's transmembrane strands and helices and signal peptides (`domainator-plm`,
//! with the `plm` feature), each region a `Domainator` feature with program and database
//! `TMbed`, its score and identity the region's mean probability in percent, and a
//! pseudo e-value `exp(-score)`. Records are cleaned as `domainate` cleans them (a
//! `cds_id` and an on-contig translation on every CDS) and, with `--gene_call`, get
//! genes called first, so a run with neither still normalizes the translations.

use regex::Regex;

use crate::search::results::SearchResult;
use crate::seq::partial::{codon_offset, feature_fragment_status, record_fragment_status, Partial};
use crate::seq::protein_analysis::{motif_matches, prosite_regex};
use crate::seq::{SeqFeature, SeqRecord};
use crate::tools::domainate::{clean_rec_nucleotide, overlay};
use crate::tools::gene_call::{call_genes, clear_cds_annotations, GeneCall, GeneCallScope};
use crate::tools::select_common::BooleanEvaluator;
use crate::DOMAIN_FEATURE_NAME;

#[derive(Default)]
pub struct FindFeaturesOptions {
    /// PROSITE patterns to search every protein and CDS translation for.
    pub motifs: Vec<String>,
    /// Which proteins and CDSs are searched, by fragment status. Every record is
    /// returned either way.
    pub partial: Partial,
    /// Call genes before searching (upstream `prodigal_CDS_annotate`).
    pub gene_call: Option<GeneCall>,
    /// Run TMbed (`--tmbed`).
    #[cfg(feature = "plm")]
    pub tmbed: Option<TmbedRun>,
}

/// A loaded TMbed and its `--batch_size`: upstream's sequences per group of records,
/// and TMbed's residue budget per encoder batch.
#[cfg(feature = "plm")]
pub struct TmbedRun {
    pub model: domainator_plm::tmbed::Tmbed,
    pub batch_size: i64,
}

/// A compiled motif: the pattern (the features' description), its feature name, and
/// its regex.
struct Motif {
    pattern: String,
    name: String,
    regex: Regex,
}

/// Annotate `records` (upstream `find_features`), returning all of them.
pub fn find_features(
    mut records: Vec<SeqRecord>,
    opts: &FindFeaturesOptions,
) -> std::io::Result<Vec<SeqRecord>> {
    let motifs: Vec<Motif> = opts
        .motifs
        .iter()
        .filter_map(|pattern| match prosite_regex(pattern) {
            Some(regex) => Some(Motif {
                pattern: pattern.clone(),
                name: BooleanEvaluator::sanitize_identifier(pattern),
                regex,
            }),
            None => {
                crate::log::warn(format_args!("Invalid PROSITE pattern '{pattern}'"));
                None
            }
        })
        .collect();

    // clean_rec, then genes for the nucleotide records left with no CDS, as in
    // `domainate`; `--gene_call all` first drops every CDS and the domains on them.
    if let Some(gene_call) = &opts.gene_call {
        if gene_call.scope == GeneCallScope::All {
            clear_cds_annotations(&mut records);
        }
    }
    let mut unannotated = Vec::new();
    for (i, rec) in records.iter_mut().enumerate() {
        if !rec.is_protein() && clean_rec_nucleotide(rec) == 0 {
            unannotated.push(i);
        }
    }
    if let Some(gene_call) = &opts.gene_call {
        call_genes(&mut records, &unannotated, &gene_call.options)?;
    }

    if !motifs.is_empty() {
        for rec in &mut records {
            annotate_motifs(rec, &motifs, opts.partial);
        }
    }
    #[cfg(feature = "plm")]
    if let Some(run) = &opts.tmbed {
        annotate_tmbed(&mut records, run, opts.partial)?;
    }
    Ok(records)
}

/// The proteins `find_features` annotates in `rec`, as `(CDS feature index, sequence)`:
/// the record's own sequence (`None`) for a protein record, else each CDS translation,
/// in feature order; fragments as `partial` allows.
fn annotated_proteins(rec: &SeqRecord, partial: Partial) -> Vec<(Option<usize>, String)> {
    if rec.is_protein() {
        if partial.allows(record_fragment_status(rec)) {
            return vec![(None, String::from_utf8_lossy(&rec.seq).into_owned())];
        }
        return Vec::new();
    }
    rec.features
        .iter()
        .enumerate()
        .filter(|(_, f)| f.kind == "CDS" && partial.allows(feature_fragment_status(f)))
        .filter_map(|(i, f)| f.get_first("translation").map(|t| (Some(i), t.to_string())))
        .collect()
}

/// A feature for `start..end` of the protein: on a protein record, that range; on a
/// nucleotide record, mapped through CDS `cds_index`'s location, or `None` (with a
/// warning) where it cannot be (upstream `add_feature_annotations`).
fn place_feature(
    rec: &SeqRecord,
    cds_index: Option<usize>,
    mut feature: SeqFeature,
    start: i64,
    end: i64,
) -> Option<SeqFeature> {
    match cds_index {
        None => {
            feature.location = crate::seq::location::Location::range(start, end, 1);
            Some(feature)
        }
        Some(i) => {
            let cds = &rec.features[i];
            let offset = codon_offset(cds) + start * 3;
            let Some(location) = overlay(&cds.location, offset, (end - start) * 3) else {
                crate::log::warn(format_args!(
                    "Could not overlay location for {} on {}. Skipping annotation.",
                    feature.get_first("name").unwrap_or_default(),
                    rec.id
                ));
                return None;
            };
            feature.location = location;
            Some(feature)
        }
    }
}

/// TMbed's regions on every protein and CDS translation (upstream `_process_batch`):
/// records are taken in groups holding at least `batch_size` proteins, as upstream
/// groups them, and each group's distinct sequences predicted together.
#[cfg(feature = "plm")]
fn annotate_tmbed(
    records: &mut [SeqRecord],
    run: &TmbedRun,
    partial: Partial,
) -> std::io::Result<()> {
    use std::collections::HashMap;

    let threshold = usize::try_from(run.batch_size).unwrap_or(0);
    let mut start = 0;
    while start < records.len() {
        let mut end = start;
        let mut proteins: Vec<Vec<(Option<usize>, String)>> = Vec::new();
        let mut count = 0;
        while end < records.len() {
            let found = annotated_proteins(&records[end], partial);
            count += found.len();
            proteins.push(found);
            end += 1;
            if count >= threshold {
                break;
            }
        }
        let mut distinct: Vec<&str> = Vec::new();
        let mut index: HashMap<&str, usize> = HashMap::new();
        for (_, seq) in proteins.iter().flatten() {
            index.entry(seq.as_str()).or_insert_with(|| {
                distinct.push(seq.as_str());
                distinct.len() - 1
            });
        }
        let predictions = if distinct.is_empty() {
            Vec::new()
        } else {
            run.model
                .predict(&distinct, run.batch_size)
                .map_err(|e| std::io::Error::other(format!("TMbed failed: {e}")))?
        };
        for (rec, found) in records[start..end].iter_mut().zip(&proteins) {
            let mut added = Vec::new();
            for (cds_index, seq) in found {
                let prediction = &predictions[index[seq.as_str()]];
                let cds_id = cds_id(rec, *cds_index);
                for region in tmbed_regions(&prediction.labels, &prediction.probabilities) {
                    let feature = region_feature(&region, &cds_id);
                    added.extend(place_feature(
                        rec,
                        *cds_index,
                        feature,
                        region.start,
                        region.end,
                    ));
                }
            }
            rec.features.extend(added);
        }
        start = end;
    }
    Ok(())
}

/// A TMbed region: a run of one transmembrane or signal peptide label.
#[cfg(feature = "plm")]
#[derive(Debug, PartialEq)]
struct TmbedRegion {
    name: &'static str,
    desc: &'static str,
    /// The region's mean probability of its class, in percent.
    score: f64,
    start: i64,
    end: i64,
}

/// Upstream `parse_tmbed_predictions`: runs of the same label among B, b, H, h and S
/// (inside and outside are not reported), each scored by its class's mean probability
/// (beta strands B, helices H, signal peptides S; `_create_feature_result`).
#[cfg(feature = "plm")]
fn tmbed_regions(labels: &[u8], probabilities: &[[f32; 5]]) -> Vec<TmbedRegion> {
    let mut regions = Vec::new();
    let mut start = 0;
    while start < labels.len() {
        let label = labels[start];
        let mut end = start + 1;
        while end < labels.len() && labels[end] == label {
            end += 1;
        }
        let kind = match label {
            0 | 1 => Some(("transmembrane_beta_strand", "transmembrane beta strand", 0)),
            2 | 3 => Some(("transmembrane_alpha_helix", "transmembrane alpha helix", 1)),
            4 => Some(("signal_peptide", "signal peptide", 2)),
            _ => None,
        };
        if let Some((name, desc, class)) = kind {
            let sum: f64 = probabilities[start..end]
                .iter()
                .map(|p| f64::from(p[class]))
                .sum();
            regions.push(TmbedRegion {
                name,
                desc,
                score: sum / (end - start) as f64 * 100.0,
                start: start as i64,
                end: end as i64,
            });
        }
        start = end;
    }
    regions
}

/// The feature for a TMbed region, before it is placed: score and identity the
/// region's probability score, e-value `exp(-score)` (upstream `score_to_pseudo_evalue`).
#[cfg(feature = "plm")]
fn region_feature(region: &TmbedRegion, cds_id: &str) -> SeqFeature {
    exact_feature(
        region.name,
        region.desc,
        "TMbed",
        region.score,
        (-region.score).exp(),
        region.start,
        region.end,
        cds_id,
    )
}

/// Add a `Domainator` feature for every motif match in `rec`'s protein, or in each of
/// its CDS translations, in CDS order, then motif order, then match order (upstream
/// `_process_batch` and `add_feature_annotations`).
fn annotate_motifs(rec: &mut SeqRecord, motifs: &[Motif], partial: Partial) {
    let mut added: Vec<SeqFeature> = Vec::new();
    for (cds_index, seq) in annotated_proteins(rec, partial) {
        let cds_id = cds_id(rec, cds_index);
        for (motif, start, end) in matches(motifs, &seq) {
            let feature = motif_feature(motif, start, end, &cds_id);
            added.extend(place_feature(rec, cds_index, feature, start, end));
        }
    }
    rec.features.extend(added);
}

/// The `cds_id` of a feature on CDS `cds_index`, or on the whole protein record.
fn cds_id(rec: &SeqRecord, cds_index: Option<usize>) -> String {
    match cds_index {
        None => format!("0_1_{}", rec.seq.len()),
        Some(i) => rec.features[i]
            .get_first("cds_id")
            .unwrap_or(".")
            .to_string(),
    }
}

/// Every match of every motif in `seq`, motif by motif, as `(motif, start, end)`.
fn matches<'a>(motifs: &'a [Motif], seq: &str) -> Vec<(&'a Motif, i64, i64)> {
    motifs
        .iter()
        .flat_map(|motif| {
            motif_matches(&motif.regex, seq)
                .into_iter()
                .map(move |(s, e)| (motif, s as i64, e as i64))
        })
        .collect()
}

/// The feature for one motif match: an exact match, so score and identity 100 and
/// e-value 0.
fn motif_feature(motif: &Motif, start: i64, end: i64, cds_id: &str) -> SeqFeature {
    exact_feature(
        &motif.name,
        &motif.pattern,
        "MOTIF",
        100.0,
        0.0,
        start,
        end,
        cds_id,
    )
}

/// A `Domainator` feature from an algorithm rather than a reference: `program` is
/// also its database, its identity is its score, and with no reference profile
/// `rstart`/`rend`/`rlen` are `.`.
#[allow(clippy::too_many_arguments)] // upstream's FeatureResult fields, one per argument
fn exact_feature(
    name: &str,
    desc: &str,
    program: &str,
    score: f64,
    evalue: f64,
    start: i64,
    end: i64,
    cds_id: &str,
) -> SeqFeature {
    let hit = SearchResult {
        name: name.to_string(),
        desc: desc.to_string(),
        acc: String::new(),
        evalue,
        score,
        start,
        end,
        database: program.to_string(),
        identity: score,
        rstart: 0,
        rend: 0,
        rlen: 0,
        program: program.to_string(),
        strand: 1,
        tmscore: None,
        lddt: None,
        rmsd: None,
        prob: None,
    };
    let mut f = hit.to_feature(DOMAIN_FEATURE_NAME, cds_id);
    for key in ["rstart", "rend", "rlen"] {
        f.set(key, ".");
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seq::location::Location;
    use crate::seq::{Annotations, Topology};

    /// Upstream `parse_tmbed_predictions`: one region per run of B/b, H/h or S (b and B
    /// are separate runs), inside and outside skipped, scored by the class's mean.
    #[cfg(feature = "plm")]
    #[test]
    fn tmbed_regions_are_runs_scored_by_their_class() {
        let p = |b: f32, h: f32, s: f32| [b, h, s, 0.0, 0.0];
        let labels = [4, 4, 5, 2, 2, 3, 6, 0];
        let probabilities = [
            p(0.0, 0.0, 0.9),
            p(0.0, 0.0, 0.7),
            p(0.0, 0.0, 0.0),
            p(0.0, 0.5, 0.0),
            p(0.0, 1.0, 0.0),
            p(0.0, 0.25, 0.0),
            p(0.0, 0.0, 0.0),
            p(0.5, 0.0, 0.0),
        ];
        let regions = tmbed_regions(&labels, &probabilities);
        let summary: Vec<(&str, i64, i64, f64)> = regions
            .iter()
            .map(|r| (r.name, r.start, r.end, r.score))
            .collect();
        assert_eq!(
            summary,
            [
                (
                    "signal_peptide",
                    0,
                    2,
                    (0.9f32 as f64 + 0.7f32 as f64) / 2.0 * 100.0
                ),
                ("transmembrane_alpha_helix", 3, 5, 75.0),
                ("transmembrane_alpha_helix", 5, 6, 25.0),
                ("transmembrane_beta_strand", 7, 8, 50.0),
            ]
        );
        let f = region_feature(&regions[1], "0_1_8");
        let q = |k| f.get_first(k).unwrap().to_string();
        assert_eq!(
            [
                q("program"),
                q("database"),
                q("score"),
                q("identity"),
                q("evalue")
            ],
            ["TMbed", "TMbed", "75.0", "75.0", "2.7e-33"]
        );
        assert_eq!(q("description"), "transmembrane alpha helix");
        assert_eq!([q("rstart"), q("rlen")], [".", "."]);
    }

    /// Upstream `sanitize_motif_name` on the patterns its help and tests use.
    #[test]
    fn motif_names_are_upstreams() {
        for (pattern, name) in [
            (
                "[DE](2)HS{P}X(2)PX(2,4)C",
                "[DE]__2___HS{P}X__2___PX__2,4___C",
            ),
            ("<M-x(2)-[ST]>", "Nterm_M_x__2____[ST]_Cterm"),
            ("A.", "A"),
            ("a|b&~c d", "a_OR_b_AND__NOT_c_d"),
        ] {
            assert_eq!(BooleanEvaluator::sanitize_identifier(pattern), name);
        }
    }

    fn record(seq: &[u8], molecule_type: &str, features: Vec<SeqFeature>) -> SeqRecord {
        SeqRecord {
            id: "r".into(),
            name: "r".into(),
            description: String::new(),
            seq: seq.to_vec(),
            molecule_type: Some(molecule_type.into()),
            topology: Topology::Linear,
            annotations: Annotations::default(),
            features,
        }
    }

    /// Upstream `test_motif_integration`: one match in a protein, written as an exact
    /// hit on the whole protein's `cds_id`.
    #[test]
    fn a_protein_motif_match_is_an_exact_hit() {
        let rec = record(b"MKFLDDHSAXXPXXXXCILLFNILCLFPVLA", "protein", vec![]);
        let opts = FindFeaturesOptions {
            motifs: vec!["[DE](2)HS".into()],
            ..Default::default()
        };
        let out = find_features(vec![rec], &opts).unwrap();
        let f = &out[0].features[0];
        assert_eq!(f.kind, DOMAIN_FEATURE_NAME);
        assert_eq!(f.location, Location::range(4, 8, 1));
        let q = |k| f.get_first(k).unwrap();
        assert_eq!(
            [
                q("program"),
                q("database"),
                q("description"),
                q("evalue"),
                q("score")
            ],
            ["MOTIF", "MOTIF", "[DE](2)HS", "0.0e+00", "100.0"]
        );
        assert_eq!(
            [q("identity"), q("cds_id"), q("rstart"), q("rlen")],
            ["100.0", "0_1_31", ".", "."]
        );
        assert_eq!(q("name"), "[DE]__2___HS");
    }

    /// A CDS match is placed on the contig through the CDS's location, minus strand
    /// included, and an invalid pattern is skipped rather than searched for.
    #[test]
    fn a_cds_motif_match_lands_on_the_contig() {
        // ATG GCT AAA TGG (M A K W), reverse-complemented onto the minus strand.
        let forward = b"ATGGCTAAATGG";
        let mut contig = b"CC".to_vec();
        contig.extend(crate::seq::translate::revcomp(forward));
        contig.extend(b"GG");
        let cds = SeqFeature::new("CDS", Location::range(2, 14, -1));
        let rec = record(&contig, "DNA", vec![cds]);
        let opts = FindFeaturesOptions {
            motifs: vec!["K-W".into(), "[AK".into()],
            ..Default::default()
        };
        let out = find_features(vec![rec], &opts).unwrap();
        let cds = &out[0].features[0];
        assert_eq!(cds.get_first("translation"), Some("MAKW"));
        let hits: Vec<&SeqFeature> = out[0]
            .features
            .iter()
            .filter(|f| f.kind == DOMAIN_FEATURE_NAME)
            .collect();
        assert_eq!(hits.len(), 1);
        // K and W are the CDS's last two codons: contig 2..8 on the minus strand.
        assert_eq!(hits[0].location, Location::range(2, 8, -1));
        assert_eq!(hits[0].get_first("cds_id"), cds.get_first("cds_id"));
    }
}
