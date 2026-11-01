//! `find_features`: annotate each CDS or protein with features found by an algorithm
//! rather than a reference search. Port of upstream `find_features.py`, less TMbed.
//!
//! The algorithm ported is `--motif`: PROSITE patterns, each match a `Domainator`
//! feature with program and database `MOTIF`, score and identity 100 and e-value 0.
//! Records are cleaned as `domainate` cleans them (a `cds_id` and an on-contig
//! translation on every CDS) and, with `--gene_call`, get genes called first, so a run
//! with no motif still normalizes the translations. TMbed (`--tmbed`) runs a protein
//! language model upstream and is refused here (see [`TMBED_UNSUPPORTED`]).

use regex::Regex;

use crate::search::results::SearchResult;
use crate::seq::partial::{codon_offset, feature_fragment_status, record_fragment_status, Partial};
use crate::seq::protein_analysis::{motif_matches, prosite_regex};
use crate::seq::{SeqFeature, SeqRecord};
use crate::tools::domainate::{clean_rec_nucleotide, overlay};
use crate::tools::gene_call::{call_genes, clear_cds_annotations, GeneCall, GeneCallScope};
use crate::tools::select_common::BooleanEvaluator;
use crate::DOMAIN_FEATURE_NAME;

/// Why `--tmbed` is refused: upstream runs TMbed (ProtT5 embeddings and a CNN) in
/// process through torch, which this port does not carry.
pub const TMBED_UNSUPPORTED: &str = "--tmbed is not supported by domainatoredux: TMbed runs \
    a protein language model through torch. Run upstream's `domainator find_features \
    --tmbed` with its optional TMbed package installed, then use the annotated file here.";

#[derive(Default)]
pub struct FindFeaturesOptions {
    /// PROSITE patterns to search every protein and CDS translation for.
    pub motifs: Vec<String>,
    /// Which proteins and CDSs are searched, by fragment status. Every record is
    /// returned either way.
    pub partial: Partial,
    /// Call genes before searching (upstream `prodigal_CDS_annotate`).
    pub gene_call: Option<GeneCall>,
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
    Ok(records)
}

/// Add a `Domainator` feature for every motif match in `rec`'s protein, or in each of
/// its CDS translations, in CDS order, then motif order, then match order (upstream
/// `_process_batch` and `add_feature_annotations`).
fn annotate_motifs(rec: &mut SeqRecord, motifs: &[Motif], partial: Partial) {
    let mut added: Vec<SeqFeature> = Vec::new();
    if rec.is_protein() {
        if partial.allows(record_fragment_status(rec)) {
            let seq = String::from_utf8_lossy(&rec.seq).into_owned();
            let cds_id = format!("0_1_{}", rec.seq.len());
            for (motif, start, end) in matches(motifs, &seq) {
                let mut f = motif_feature(motif, start, end, &cds_id);
                f.location = crate::seq::location::Location::range(start, end, 1);
                added.push(f);
            }
        }
    } else {
        for cds in &rec.features {
            let Some(translation) = cds.get_first("translation") else {
                continue;
            };
            if cds.kind != "CDS" || !partial.allows(feature_fragment_status(cds)) {
                continue;
            }
            let cds_id = cds.get_first("cds_id").unwrap_or(".");
            for (motif, start, end) in matches(motifs, translation) {
                let offset = codon_offset(cds) + start * 3;
                let Some(location) = overlay(&cds.location, offset, (end - start) * 3) else {
                    crate::log::warn(format_args!(
                        "Could not overlay location for {} on {}. Skipping annotation.",
                        motif.name, rec.id
                    ));
                    continue;
                };
                let mut f = motif_feature(motif, start, end, cds_id);
                f.location = location;
                added.push(f);
            }
        }
    }
    rec.features.extend(added);
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
/// e-value 0, with no reference profile, so `rstart`/`rend`/`rlen` are `.`.
fn motif_feature(motif: &Motif, start: i64, end: i64, cds_id: &str) -> SeqFeature {
    let hit = SearchResult {
        name: motif.name.clone(),
        desc: motif.pattern.clone(),
        acc: String::new(),
        evalue: 0.0,
        score: 100.0,
        start,
        end,
        database: "MOTIF".into(),
        identity: 100.0,
        rstart: 0,
        rend: 0,
        rlen: 0,
        program: "MOTIF".into(),
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
