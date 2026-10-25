//! `rotate_contigs`: set the origin of circular contigs relative to a domain. Port of
//! upstream `rotate_contigs.py`.
//!
//! Each contig is rotated on one domain, chosen from the new hits of a reference
//! search (`-r`) and/or its pre-existing `Domainator` (`--domains`) or `Domain_Search`
//! (`--annotation_type domain_search`) annotations: the best score, then the lowest
//! e-value, then the lowest start. The new origin goes at the domain's start, end or
//! center in its own orientation, and the contig is reverse complemented to put the
//! domain on the forward strand unless `keep_direction`.

use std::collections::HashSet;
use std::io;

use crate::search::references::References;
use crate::seq::{SeqFeature, SeqRecord, Topology};
use crate::tools::domainate::{domainate, DomainateOptions};
use crate::tools::extract_common::{circular_envelope, slice_from_location, Loc};
use crate::tools::gene_call::{GeneCall, GeneCallScope};
use crate::tools::select_by_cds::Anchor;
use crate::{DOMAIN_FEATURE_NAME, DOMAIN_SEARCH_BEST_HIT_NAME};

/// What a FASTA record may hold and still be read as nucleotides.
const NUCLEOTIDE_LETTERS: &[u8] = b"ACGTURYSWKMBDHVN-.";

/// Options for [`rotate_contigs`].
pub struct RotateContigsOptions {
    /// hmmsearch's `-Z` for the reference search.
    pub z: f64,
    /// The largest e-value of a domain, new or pre-existing, to rotate on.
    pub evalue: f64,
    /// Rotate on pre-existing `Domainator` annotations with these names.
    pub domains: Option<HashSet<String>>,
    /// Consider only pre-existing annotations from these databases (`None`: all).
    pub databases: Option<HashSet<String>>,
    /// `--annotation_type domain_search`: rotate on pre-existing `Domain_Search` hits.
    pub search_hits: bool,
    /// Where the origin goes, relative to the domain in its own orientation.
    pub anchor: Anchor,
    pub keep_direction: bool,
    /// Rotate linear contigs and FASTA records as though they were circular.
    pub circularize: bool,
    pub keep_name: bool,
    /// Keep the new annotation the contig was rotated on (other new hits never are).
    pub keep_annotation: bool,
    pub gene_call: Option<GeneCall>,
    /// Worker threads for the search (`0`: one per physical core).
    pub cpu: usize,
    pub max_mode: bool,
    /// Contigs searched at a time.
    pub batch_size: usize,
}

impl Default for RotateContigsOptions {
    fn default() -> Self {
        RotateContigsOptions {
            z: 1000.0,
            evalue: 0.001,
            domains: None,
            databases: None,
            search_hits: false,
            anchor: Anchor::Start,
            keep_direction: false,
            circularize: false,
            keep_name: false,
            keep_annotation: false,
            gene_call: None,
            cpu: 0,
            max_mode: false,
            batch_size: 1000,
        }
    }
}

/// The checks upstream's `rotate_contigs` makes before it reads a record, with its
/// messages. `has_references`: whether `-r` was given.
pub fn check_options(has_references: bool, opts: &RotateContigsOptions) -> Result<(), String> {
    if opts.batch_size < 1 {
        return Err("batch_size must be at least 1.".into());
    }
    if !has_references && opts.domains.is_none() && !opts.search_hits {
        return Err(
            "Specify references, domains, or annotation_type=\"domain_search\" to \
                    choose domains to rotate on."
                .into(),
        );
    }
    if !has_references && opts.gene_call.is_some() {
        return Err("gene_call only applies when searching references.".into());
    }
    let call_all = matches!(&opts.gene_call, Some(g) if g.scope == GeneCallScope::All);
    if call_all && (opts.domains.is_some() || opts.search_hits) {
        return Err(
            "gene_call 'all' deletes pre-existing annotations, so it can't be \
                    combined with domains or annotation_type=\"domain_search\"."
                .into(),
        );
    }
    Ok(())
}

/// Rotate `records` (see the module docs). Contigs that cannot be rotated are skipped
/// with a warning: protein records, linear ones unless `circularize`, and those with no
/// qualifying domain.
pub fn rotate_contigs(
    records: Vec<SeqRecord>,
    refs: Option<&References>,
    opts: &RotateContigsOptions,
) -> io::Result<Vec<SeqRecord>> {
    check_options(refs.is_some(), opts)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let mut records = prepare_records(records, opts.circularize).into_iter();
    let mut out = Vec::new();
    loop {
        let chunk: Vec<SeqRecord> = records.by_ref().take(opts.batch_size).collect();
        if chunk.is_empty() {
            break;
        }
        for (rec, is_new) in search(chunk, refs, opts)? {
            let candidates = candidate_features(&rec, &is_new, opts);
            let Some(anchor) = pick_anchor(&rec, &candidates) else {
                crate::log::warn(format_args!(
                    "No qualifying domain found on contig {}. Skipping contig.",
                    rec.id
                ));
                continue;
            };
            let (cut, strand) =
                origin_cut(&rec.features[anchor], rec.seq.len() as i64, opts.anchor);
            let reverse = strand == -1 && !opts.keep_direction;
            let features: Vec<SeqFeature> = rec
                .features
                .iter()
                .enumerate()
                .filter(|&(i, _)| !is_new[i] || (opts.keep_annotation && i == anchor))
                .map(|(_, f)| f.clone())
                .collect();
            out.push(rotate_record(&rec, cut, reverse, &features, opts.keep_name));
        }
    }
    Ok(out)
}

/// Whether `rec` is a protein, or holds letters that are not IUPAC nucleotide codes
/// (FASTA records are read as DNA).
fn is_protein(rec: &SeqRecord) -> bool {
    rec.is_protein()
        || rec
            .seq
            .iter()
            .any(|b| !NUCLEOTIDE_LETTERS.contains(&b.to_ascii_uppercase()))
}

/// The records that can be rotated, made circular; warns about the rest.
fn prepare_records(records: Vec<SeqRecord>, circularize: bool) -> Vec<SeqRecord> {
    let mut out = Vec::with_capacity(records.len());
    for mut rec in records {
        if is_protein(&rec) {
            crate::log::warn(format_args!(
                "Contig {} is a protein sequence. Only nucleotide sequences can be rotated. \
                 Skipping contig.",
                rec.id
            ));
            continue;
        }
        if rec.topology != Topology::Circular {
            if !circularize {
                crate::log::warn(format_args!(
                    "Contig {} is not circular. Use --circularize to rotate linear contigs and \
                     fasta records. Skipping contig.",
                    rec.id
                ));
                continue;
            }
            rec.topology = Topology::Circular;
            rec.annotations.topology = Some(Topology::Circular);
        }
        out.push(rec);
    }
    out
}

/// Search `chunk` against `refs`, returning each record with a flag per feature: whether
/// it is a new hit. domainate keeps the records in order, appends its hits after the
/// existing `Domainator` features and, under `--gene_call all`, deletes those first.
fn search(
    chunk: Vec<SeqRecord>,
    refs: Option<&References>,
    opts: &RotateContigsOptions,
) -> io::Result<Vec<(SeqRecord, Vec<bool>)>> {
    let Some(refs) = refs else {
        return Ok(chunk
            .into_iter()
            .map(|rec| {
                let n = rec.features.len();
                (rec, vec![false; n])
            })
            .collect());
    };
    let call_all = matches!(&opts.gene_call, Some(g) if g.scope == GeneCallScope::All);
    let kept: Vec<usize> = chunk
        .iter()
        .map(|rec| if call_all { 0 } else { domain_count(rec) })
        .collect();
    let search_opts = DomainateOptions {
        z: opts.z,
        evalue: opts.evalue,
        max_mode: opts.max_mode,
        cpu: opts.cpu,
        gene_call: opts.gene_call.clone(),
        ..Default::default()
    };
    let annotated = domainate(chunk, refs, &search_opts)?;
    Ok(annotated
        .into_iter()
        .zip(kept)
        .map(|(rec, kept)| {
            let mut seen = 0;
            let is_new = rec
                .features
                .iter()
                .map(|f| {
                    if f.kind != DOMAIN_FEATURE_NAME {
                        return false;
                    }
                    seen += 1;
                    seen > kept
                })
                .collect();
            (rec, is_new)
        })
        .collect())
}

fn domain_count(rec: &SeqRecord) -> usize {
    rec.features
        .iter()
        .filter(|f| f.kind == DOMAIN_FEATURE_NAME)
        .count()
}

/// A numeric qualifier, or `default` when it is missing or not a number.
fn qualifier_f64(f: &SeqFeature, name: &str, default: f64) -> f64 {
    f.get_first(name)
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

/// The indices of the features of `rec` the contig can be rotated on.
fn candidate_features(rec: &SeqRecord, is_new: &[bool], opts: &RotateContigsOptions) -> Vec<usize> {
    rec.features
        .iter()
        .enumerate()
        .filter(|&(i, f)| {
            let selected = is_new[i] || {
                let by_type = match f.kind.as_str() {
                    DOMAIN_FEATURE_NAME => opts
                        .domains
                        .as_ref()
                        .is_some_and(|d| f.get_first("name").is_some_and(|n| d.contains(n))),
                    DOMAIN_SEARCH_BEST_HIT_NAME => opts.search_hits,
                    _ => false,
                };
                let in_databases = opts
                    .databases
                    .as_ref()
                    .is_none_or(|d| f.get_first("database").is_some_and(|db| d.contains(db)));
                by_type && in_databases
            };
            selected && qualifier_f64(f, "evalue", f64::INFINITY) <= opts.evalue
        })
        .map(|(i, _)| i)
        .collect()
}

/// The candidate with the highest score, then the lowest e-value, then the lowest
/// start; the first of those still tied.
fn pick_anchor(rec: &SeqRecord, candidates: &[usize]) -> Option<usize> {
    let key = |i: usize| {
        let f = &rec.features[i];
        (
            -qualifier_f64(f, "score", f64::NEG_INFINITY),
            qualifier_f64(f, "evalue", f64::INFINITY),
            f.location.start() as f64,
        )
    };
    candidates.iter().copied().reduce(|best, i| {
        if key(i).partial_cmp(&key(best)) == Some(std::cmp::Ordering::Less) {
            i
        } else {
            best
        }
    })
}

/// Where to put the new origin of a circular contig of `contig_length`, relative to
/// `feature` (upstream `origin_cut`): `(cut, strand)`, the new origin falling between
/// bases `cut - 1` and `cut` (0-based, forward strand), and the feature's strand.
fn origin_cut(feature: &SeqFeature, contig_length: i64, anchor: Anchor) -> (i64, i32) {
    let parts = &feature.location.parts;
    let strand = if parts.first().is_some_and(|p| p.2 == -1) {
        -1
    } else {
        1
    };
    let parts: Vec<(i64, i64, i32)> = parts.iter().map(|p| (p.0, p.1, p.2)).collect();
    let envelope = circular_envelope(&parts, contig_length, strand);
    let length: i64 = envelope.iter().map(|(s, e)| e - s).sum();
    // The envelope's first base: one part, or the part before the origin.
    let start = match envelope.as_slice() {
        [only] => only.0,
        _ => envelope
            .iter()
            .find(|p| p.1 == contig_length)
            .map_or(envelope[0].0, |p| p.0),
    };
    let offset = match (anchor, strand == 1) {
        (Anchor::Start, true) | (Anchor::End, false) => 0,
        (Anchor::End, true) | (Anchor::Start, false) => length,
        (Anchor::Center, true) => length / 2,
        (Anchor::Center, false) => length - length / 2,
    };
    ((start + offset).rem_euclid(contig_length.max(1)), strand)
}

/// The whole of a circular contig of `contig_length`, read from a new origin `cut`
/// (upstream `rotation_location`); `reverse` reads the reverse strand from base `cut - 1`.
fn rotation_location(cut: i64, contig_length: i64, reverse: bool) -> Loc {
    let mut parts = if reverse {
        vec![(0, cut), (cut, contig_length)]
    } else {
        vec![(cut, contig_length), (0, cut)]
    };
    parts.retain(|p| p.1 > p.0);
    Loc {
        parts,
        strand: if reverse { -1 } else { 1 },
    }
}

/// Rotate a circular contig to the origin `cut`, reverse complementing it if `reverse`,
/// keeping `features` (one crossing the new origin becomes a join) and every annotation
/// (upstream `rotate_record`). Renamed by `region_id` unless `keep_name`.
fn rotate_record(
    rec: &SeqRecord,
    cut: i64,
    reverse: bool,
    features: &[SeqFeature],
    keep_name: bool,
) -> SeqRecord {
    let location = rotation_location(cut, rec.seq.len() as i64, reverse);
    let mut rotated = slice_from_location(rec, &location, features);
    rotated.annotations = rec.annotations.clone();
    rotated.annotations.topology = Some(Topology::Circular);
    rotated.topology = Topology::Circular;
    if !keep_name {
        rotated.id = location.region_id(rec, false);
    }
    rotated
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seq::location::{CompoundOp, Location};

    const CIRCULAR_SEQUENCE: &str =
        "GCTAACCGTTAGCGATCGTACGATCGATGCTAGTCCGATTAACCGGTTAGGCTTACCGATGG";

    fn feature(parts: &[(i64, i64, i32)]) -> SeqFeature {
        let mut f = SeqFeature::new(
            DOMAIN_FEATURE_NAME,
            Location {
                operator: (parts.len() > 1).then_some(CompoundOp::Join),
                between: false,
                parts: parts
                    .iter()
                    .map(|&(s, e, st)| (s, e, st, false, false))
                    .collect(),
            },
        );
        f.push("name", "dom");
        f.push("evalue", "1e-10");
        f.push("score", "50");
        f
    }

    fn circular(seq: &str, features: Vec<SeqFeature>) -> SeqRecord {
        SeqRecord {
            id: "circ".into(),
            name: "circ".into(),
            description: "circular test".into(),
            seq: seq.as_bytes().to_vec(),
            molecule_type: Some("DNA".into()),
            topology: Topology::Circular,
            annotations: Default::default(),
            features,
        }
    }

    fn revcomp(seq: &str) -> String {
        seq.bytes()
            .rev()
            .map(|b| match b {
                b'A' => 'T',
                b'C' => 'G',
                b'G' => 'C',
                b'T' => 'A',
                other => other as char,
            })
            .collect()
    }

    fn rotated_seq(seq: &str, cut: usize, reverse: bool) -> String {
        let rotated = format!("{}{}", &seq[cut..], &seq[..cut]);
        if reverse {
            revcomp(&rotated)
        } else {
            rotated
        }
    }

    /// `(feature parts, anchor, expected (cut, strand))`.
    type CutCase = (&'static [(i64, i64, i32)], Anchor, (i64, i32));

    /// Upstream `test_origin_cut`.
    #[test]
    fn origin_cut_matches_upstream() {
        use Anchor::{Center, End, Start};
        let cases: &[CutCase] = &[
            (&[(10, 20, 1)], Start, (10, 1)),
            (&[(10, 20, 1)], End, (20, 1)),
            (&[(10, 20, 1)], Center, (15, 1)),
            (&[(10, 21, 1)], Center, (15, 1)),
            (&[(10, 20, -1)], Start, (20, -1)),
            (&[(10, 20, -1)], End, (10, -1)),
            (&[(10, 20, -1)], Center, (15, -1)),
            (&[(10, 21, -1)], Center, (16, -1)),
            (&[(90, 100, 1)], End, (0, 1)),
            (&[(0, 10, -1)], End, (0, -1)),
            (&[(95, 100, 1), (0, 5, 1)], Start, (95, 1)),
            (&[(95, 100, 1), (0, 5, 1)], End, (5, 1)),
            (&[(95, 100, 1), (0, 5, 1)], Center, (0, 1)),
            (&[(0, 5, -1), (95, 100, -1)], Start, (5, -1)),
            (&[(0, 5, -1), (95, 100, -1)], End, (95, -1)),
            (&[(0, 6, -1), (95, 100, -1)], Center, (1, -1)),
        ];
        for &(parts, anchor, expected) in cases {
            assert_eq!(
                origin_cut(&feature(parts), 100, anchor),
                expected,
                "{parts:?} {anchor:?}"
            );
        }
    }

    /// Upstream `test_rotation_location_sequence`.
    #[test]
    fn rotation_reads_the_contig_from_the_cut() {
        let rec = circular(CIRCULAR_SEQUENCE, vec![]);
        for cut in [0, 1, 17, 61] {
            for reverse in [false, true] {
                let rotated = rotate_record(&rec, cut as i64, reverse, &[], false);
                assert_eq!(
                    String::from_utf8(rotated.seq).unwrap(),
                    rotated_seq(CIRCULAR_SEQUENCE, cut, reverse)
                );
                assert_eq!(rotated.topology, Topology::Circular);
                assert_eq!(rotated.annotations.topology, Some(Topology::Circular));
            }
        }
    }

    /// Upstream `test_rotated_name`.
    #[test]
    fn rotated_names() {
        let mut rec = circular(&"A".repeat(100), vec![]);
        rec.id = "c".into();
        let name = |cut, reverse| rotation_location(cut, 100, reverse).region_id(&rec, false);
        assert_eq!(name(0, false), "c");
        assert_eq!(name(10, false), "c_11:10");
        assert_eq!(name(0, true), "c_rc");
        assert_eq!(name(10, true), "c_10:11rc");
    }

    /// Upstream `test_rotate_on_feature`: the domain lands at the start, the end, or
    /// across the origin, in its own orientation, and keeps its sequence.
    #[test]
    fn rotate_on_a_feature() {
        let length = CIRCULAR_SEQUENCE.len() as i64;
        for anchor in [Anchor::Start, Anchor::End, Anchor::Center] {
            for strand in [1, -1] {
                for keep_direction in [false, true] {
                    let rec = circular(CIRCULAR_SEQUENCE, vec![feature(&[(20, 30, strand)])]);
                    let opts = RotateContigsOptions {
                        domains: Some(HashSet::from(["dom".to_string()])),
                        anchor,
                        keep_direction,
                        ..Default::default()
                    };
                    let rotated = rotate_contigs(vec![rec], None, &opts).unwrap().remove(0);
                    let domain = &rotated.features[0];
                    let reverse = strand == -1 && !keep_direction;
                    let out_strand = if strand == -1 && keep_direction {
                        -1
                    } else {
                        1
                    };
                    assert!(domain.location.parts.iter().all(|p| p.2 == out_strand));
                    let mut intervals: Vec<(i64, i64)> =
                        domain.location.parts.iter().map(|p| (p.0, p.1)).collect();
                    intervals.sort();
                    let domain_first = match (anchor, out_strand == 1) {
                        (Anchor::Start, true) | (Anchor::End, false) => Some(true),
                        (Anchor::End, true) | (Anchor::Start, false) => Some(false),
                        (Anchor::Center, _) => None,
                    };
                    let expected = match domain_first {
                        Some(true) => vec![(0, 10)],
                        Some(false) => vec![(length - 10, length)],
                        None => vec![(0, 5), (length - 5, length)],
                    };
                    assert_eq!(intervals, expected, "{anchor:?} {strand} {keep_direction}");
                    assert_eq!(rotated.id.ends_with("rc"), reverse);
                }
            }
        }
    }

    #[test]
    fn the_checks_name_what_is_missing() {
        let none = RotateContigsOptions::default();
        assert!(check_options(false, &none)
            .unwrap_err()
            .starts_with("Specify references, domains"));
        assert!(check_options(true, &none).is_ok());
        let zero_batch = RotateContigsOptions {
            batch_size: 0,
            ..Default::default()
        };
        assert_eq!(
            check_options(true, &zero_batch).unwrap_err(),
            "batch_size must be at least 1."
        );
    }
}
