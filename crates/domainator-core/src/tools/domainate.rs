//! `domainate`: annotate sequences with hits to reference HMMs.
//!
//! M1 scope: the protein `hmmsearch` path (gate 1a). Nucleotide CDS handling
//! (cds_id generation, translation, the `domainator_<db>` CDS rollup) and
//! phmmer/nhmmer/infernal/foldseek follow.

use std::collections::HashMap;

use crate::search::hmmer::{hmmsearch_db, load_hmm_dbs, HmmDb};
use crate::search::SearchResult;
use crate::seq::SeqRecord;
use crate::{DOMAIN_FEATURE_NAME, DOMAIN_SEARCH_BEST_HIT_NAME};

/// Whether two half-open regions overlap by at least `min_overlap_fraction` of
/// `region2` (port of `utils.regions_overlap`). `>= 1.0` never overlaps.
fn regions_overlap(r1: (i64, i64), r2: (i64, i64), min_overlap_fraction: f64) -> bool {
    if min_overlap_fraction >= 1.0 {
        return false;
    }
    let r2_size = if r2.1 - r2.0 == 0 { 0.1 } else { (r2.1 - r2.0) as f64 };
    let overlap = (r1.1.min(r2.1) - r1.0.max(r2.0)).max(0);
    if overlap == 0 {
        false
    } else {
        (overlap as f64 / r2_size) >= min_overlap_fraction
    }
}

/// Greedily keep the highest-scoring non-overlapping domains (port of
/// `filter_by_overlap`). Input is consumed; assumed already sorted by score desc.
fn filter_by_overlap(hits: Vec<SearchResult>, allowed_overlap: f64, by_db: bool) -> Vec<SearchResult> {
    // Preserve per-db insertion order, matching the Python dict-of-lists flatten.
    let mut order: Vec<Option<String>> = Vec::new();
    let mut kept: HashMap<Option<String>, Vec<SearchResult>> = HashMap::new();
    for hit in hits {
        let key = if by_db { Some(hit.database.clone()) } else { None };
        let bucket = kept.entry(key.clone()).or_insert_with(|| {
            order.push(key.clone());
            Vec::new()
        });
        if bucket.is_empty() {
            bucket.push(hit);
            continue;
        }
        let non_overlap = bucket
            .iter()
            .filter(|k| !regions_overlap((k.start, k.end), (hit.start, hit.end), allowed_overlap))
            .count();
        if non_overlap == bucket.len() {
            bucket.push(hit);
        }
    }
    order.into_iter().flat_map(|k| kept.remove(&k).unwrap_or_default()).collect()
}

/// Add `Domainator` (and optional best-hit) features to a protein contig (port of
/// `add_protein_annotations`). `hits` is for the single protein (cds index 0).
fn add_protein_annotations(
    contig: &mut SeqRecord,
    mut hits: Vec<SearchResult>,
    max_hits: usize,
    max_overlap: f64,
    no_annotations: bool,
    best_annotation: bool,
    overlap_by_db: bool,
) {
    if hits.is_empty() {
        return;
    }
    hits.sort_by(|a, b| b.score.total_cmp(&a.score));
    let cds_id = format!("0_1_{}", contig.len());

    if best_annotation {
        contig.features.push(hits[0].to_feature(DOMAIN_SEARCH_BEST_HIT_NAME, &cds_id));
    }
    if !no_annotations {
        let capped: Vec<SearchResult> = hits.into_iter().take(max_hits).collect();
        for hit in filter_by_overlap(capped, max_overlap, overlap_by_db) {
            contig.features.push(hit.to_feature(DOMAIN_FEATURE_NAME, &cds_id));
        }
    }
}

/// Options for [`domainate`].
pub struct DomainateOptions {
    pub z: f64,
    pub evalue: f64,
    pub min_evalue: f64,
    pub max_hits: usize,
    pub max_overlap: f64,
    pub best_annotation: bool,
    pub no_annotations: bool,
    pub hits_only: bool,
    pub overlap_by_db: bool,
}

impl Default for DomainateOptions {
    fn default() -> Self {
        DomainateOptions {
            z: 1000.0,
            evalue: 0.001,
            min_evalue: 0.0,
            max_hits: usize::MAX,
            max_overlap: 1.0,
            best_annotation: false,
            no_annotations: false,
            hits_only: false,
            overlap_by_db: false,
        }
    }
}

/// Annotate `records` with hits from `hmm_dbs` (protein hmmsearch path). Returns
/// the annotated records (all records, or hit-only when `hits_only`).
pub fn domainate(mut records: Vec<SeqRecord>, hmm_dbs: &[HmmDb], opts: &DomainateOptions) -> Vec<SeqRecord> {
    // Batch every protein peptide across contigs, named "{contig_index},0".
    let mut peptides: Vec<(String, Vec<u8>)> = Vec::new();
    for (i, rec) in records.iter().enumerate() {
        if rec.is_protein() {
            let pep: Vec<u8> = rec.seq.iter().copied().filter(|b| b.is_ascii_alphanumeric()).collect();
            peptides.push((format!("{i},0"), pep));
        }
    }

    // Collect SearchResults per contig index.
    let mut hits_by_contig: HashMap<usize, Vec<SearchResult>> = HashMap::new();
    for db in hmm_dbs {
        for nh in hmmsearch_db(db, &peptides, opts.z, opts.evalue, opts.min_evalue) {
            if let Some(ci) = nh.target.split(',').next().and_then(|s| s.parse::<usize>().ok()) {
                hits_by_contig.entry(ci).or_default().push(nh.result);
            }
        }
    }

    for (i, rec) in records.iter_mut().enumerate() {
        if rec.is_protein() {
            if let Some(hits) = hits_by_contig.remove(&i) {
                add_protein_annotations(
                    rec, hits, opts.max_hits, opts.max_overlap, opts.no_annotations,
                    opts.best_annotation, opts.overlap_by_db,
                );
            }
        }
    }

    if opts.hits_only {
        records.retain(|r| {
            r.features.iter().any(|f| f.kind == DOMAIN_FEATURE_NAME || f.kind == DOMAIN_SEARCH_BEST_HIT_NAME)
        });
    }
    records
}

/// Load HMM reference databases from `.hmm` files.
pub fn read_hmm_references(files: &[String]) -> std::io::Result<Vec<HmmDb>> {
    load_hmm_dbs(files)
}
