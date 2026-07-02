//! `domainate`: annotate sequences with hits to reference HMMs.
//!
//! M1 scope: the protein `hmmsearch` path (gate 1a). Nucleotide CDS handling
//! (cds_id generation, translation, the `domainator_<db>` CDS rollup) and
//! phmmer/nhmmer/infernal/foldseek follow.

use std::collections::{HashMap, HashSet};

use crate::search::hmmer::{hmmsearch_db, load_hmm_dbs, HmmDb};
use crate::search::SearchResult;
use crate::seq::taxid::record_taxid;
use crate::seq::translate::is_searchable_cds;
use crate::seq::{Location, SeqFeature, SeqRecord};
use crate::{DOMAIN_FEATURE_NAME, DOMAIN_SEARCH_BEST_HIT_NAME};

/// `cds_id` for a CDS feature: keep an existing one, else generate from the
/// location parts using Biopython's strand-aware human-readable coordinates
/// (`utils.get_cds_unique_name`; plan risk E6). Per part: `-1` strand ->
/// `{end}_-1_{start+1}`, else `{start+1}_{strand}_{end}`; parts space-joined.
fn cds_unique_name(feature: &SeqFeature) -> String {
    if let Some(existing) = feature.get_first("cds_id") {
        return existing.to_string();
    }
    feature
        .location
        .parts
        .iter()
        .map(|&(s, e, strand, _, _)| {
            if strand == -1 {
                format!("{}_{}_{}", e, strand, s + 1)
            } else {
                format!("{}_{}_{}", s + 1, strand, e)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `clean_rec` for a nucleotide record: assign `cds_id` and a `/translation` to
/// every non-pseudo CDS lacking one. Mutates in place; returns the CDS count.
fn clean_rec_nucleotide(rec: &mut SeqRecord) -> usize {
    let seq = rec.seq.clone();
    let mut count = 0;
    for feature in &mut rec.features {
        if is_searchable_cds(feature) {
            let cds_id = cds_unique_name(feature);
            feature.set("cds_id", cds_id);
            count += 1;
            let needs_translation = feature
                .get_first("translation")
                .map(|t| t.is_empty())
                .unwrap_or(true);
            if needs_translation {
                let tr = crate::seq::translate::translate_dna(
                    &crate::seq::translate::extract_location_seq(&feature.location, &seq),
                );
                feature.set("translation", tr);
            }
        }
    }
    count
}

/// Map a protein-coordinate domain `[start,end)` (aa) back onto a CDS's
/// nucleotide location (port of `SimpleLocation`/`CompoundLocation.overlay`).
fn overlay(loc: &Location, mut offset: i64, mut length: i64) -> Location {
    let mut new_parts: Vec<crate::seq::location::Part> = Vec::new();
    for &(s, e, strand, _, _) in &loc.parts {
        let part_len = e - s;
        if offset >= part_len {
            offset -= part_len;
            continue;
        }
        let section = (part_len - offset).min(length);
        let np = if strand == -1 {
            (e - offset - section, e - offset, strand, false, false)
        } else {
            (s + offset, s + offset + section, strand, false, false)
        };
        new_parts.push(np);
        length -= section;
        offset = 0;
        if length == 0 {
            break;
        }
    }
    if new_parts.len() == 1 {
        Location { operator: None, between: false, parts: new_parts }
    } else {
        Location { operator: loc.operator, between: false, parts: new_parts }
    }
}

/// Add `Domainator` features to a nucleotide contig (port of
/// `add_nucleic_acid_annotations`). `hits_by_cds` maps a CDS feature index to its
/// domain hits; features are placed by overlaying the aa hit span (×3) onto the CDS.
fn add_nucleic_acid_annotations(
    contig: &mut SeqRecord,
    mut hits_by_cds: HashMap<usize, Vec<SearchResult>>,
    max_hits: usize,
    max_overlap: f64,
    no_annotations: bool,
    best_annotation: bool,
    overlap_by_db: bool,
) {
    // Deterministic order: by feature index.
    let mut feature_ids: Vec<usize> = hits_by_cds.keys().copied().collect();
    feature_ids.sort_unstable();
    let mut to_append: Vec<SeqFeature> = Vec::new();
    for fid in feature_ids {
        let mut hits = hits_by_cds.remove(&fid).unwrap();
        hits.sort_by(|a, b| b.score.total_cmp(&a.score));
        let cds_loc = contig.features[fid].location.clone();
        let cds_id = contig.features[fid].get_first("cds_id").unwrap_or("").to_string();

        let emit = |hit: &SearchResult, ftype: &str, out: &mut Vec<SeqFeature>| {
            let annot_len = (hit.end - hit.start) * 3;
            let loc = overlay(&cds_loc, hit.start * 3, annot_len);
            let mut f = hit.to_feature(ftype, &cds_id);
            f.location = loc;
            out.push(f);
        };
        if best_annotation {
            emit(&hits[0], DOMAIN_SEARCH_BEST_HIT_NAME, &mut to_append);
        }
        if !no_annotations {
            let capped: Vec<SearchResult> = hits.into_iter().take(max_hits).collect();
            for hit in filter_by_overlap(capped, max_overlap, overlap_by_db) {
                emit(&hit, DOMAIN_FEATURE_NAME, &mut to_append);
            }
        }
    }
    contig.features.extend(to_append);
}

/// Add the `domainator_<db>` rollup qualifier to CDS features (port of
/// `write_genbank`'s `add_domain_cds_annotations`): per CDS, group its
/// `Domainator` domains by database into `name (desc, evalue, score) & ...`.
pub fn add_domain_cds_annotations(rec: &mut SeqRecord) {
    // cds_id -> ordered [(db, [strings])], in Domainator-feature order.
    let mut by_cds: HashMap<String, Vec<(String, Vec<String>)>> = HashMap::new();
    for f in &rec.features {
        if f.kind != DOMAIN_FEATURE_NAME {
            continue;
        }
        let (cds_id, db, name, eval, score, desc) = match (
            f.get_first("cds_id"), f.get_first("database"), f.get_first("name"),
            f.get_first("evalue"), f.get_first("score"), f.get_first("description"),
        ) {
            (Some(c), Some(db), Some(n), Some(e), Some(s), Some(d)) => (c, db, n, e, s, d),
            _ => continue,
        };
        let s = format!("{name} ({desc}, {eval}, {score})");
        let buckets = by_cds.entry(cds_id.to_string()).or_default();
        match buckets.iter_mut().find(|(d, _)| d == db) {
            Some((_, v)) => v.push(s),
            None => buckets.push((db.to_string(), vec![s])),
        }
    }
    for f in &mut rec.features {
        if f.kind != "CDS" {
            continue;
        }
        let cds_id = f.get_first("cds_id").map(str::to_string);
        let mut seen: Vec<String> = Vec::new();
        if let Some(cds_id) = cds_id {
            if let Some(buckets) = by_cds.get(&cds_id) {
                for (db, strings) in buckets {
                    let key = format!("domainator_{db}");
                    f.set(&key, strings.join(" & "));
                    seen.push(key);
                }
            }
        }
        // Drop stale domainator_* qualifiers whose db had all domains filtered out.
        f.qualifiers.retain(|(k, _)| !k.starts_with("domainator_") || seen.contains(k));
    }
}

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
    /// Per-CDS cap on emitted domains (`--max_domains`; `usize::MAX` = unlimited).
    pub max_hits: usize,
    pub max_overlap: f64,
    pub best_annotation: bool,
    pub no_annotations: bool,
    pub hits_only: bool,
    pub overlap_by_db: bool,
    /// hmmsearch `--max` (skip heuristic filters for maximum sensitivity).
    pub max_mode: bool,
    /// Precompiled allowed-taxid set (`None` = no taxonomy filtering). Records
    /// whose lineage is not in the set are dropped before searching.
    pub allowed_taxids: Option<HashSet<i64>>,
    /// Worker threads for the per-database search (`0` = all cores).
    pub cpu: usize,
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
            max_mode: false,
            allowed_taxids: None,
            cpu: 0,
        }
    }
}

/// Annotate `records` with hits from `hmm_dbs` (protein hmmsearch path). Returns
/// the annotated records (all records, or hit-only when `hits_only`).
pub fn domainate(mut records: Vec<SeqRecord>, hmm_dbs: &[HmmDb], opts: &DomainateOptions) -> Vec<SeqRecord> {
    let empty_dropped = std::collections::HashSet::new();

    // Taxonomy pre-filter: drop records whose lineage is not in the allowed set
    // (mirrors Python `filter_by_taxonomy` applied to the input records).
    if let Some(allowed) = &opts.allowed_taxids {
        records.retain(|rec| allowed.contains(&record_taxid(rec)));
    }

    // clean_rec: nucleotide records get cds_id + /translation on every CDS.
    for rec in records.iter_mut() {
        if !rec.is_protein() {
            clean_rec_nucleotide(rec);
        }
    }

    // Batch peptides across contigs. Protein: "{i},0". Nucleotide: "{i},{feature_index}".
    let mut peptides: Vec<(String, Vec<u8>)> = Vec::new();
    for (i, rec) in records.iter().enumerate() {
        if rec.is_protein() {
            let pep: Vec<u8> = rec.seq.iter().copied().filter(|b| b.is_ascii_alphanumeric()).collect();
            peptides.push((format!("{i},0"), pep));
        } else {
            for (j, pep) in rec.cds_peptides(&empty_dropped) {
                peptides.push((format!("{i},{j}"), pep.into_bytes()));
            }
        }
    }

    // contig index -> (cds feature index -> SearchResults). Databases are searched
    // in parallel (each independent); results are merged in database order so the
    // output is deterministic regardless of thread count.
    let per_db: Vec<Vec<crate::search::hmmer::NamedHit>> = crate::parallel::with_threads(opts.cpu, || {
        use rayon::prelude::*;
        hmm_dbs
            .par_iter()
            .map(|db| hmmsearch_db(db, &peptides, opts.z, opts.evalue, opts.min_evalue, opts.max_mode))
            .collect()
    });
    let mut hits: HashMap<usize, HashMap<usize, Vec<SearchResult>>> = HashMap::new();
    for nhs in per_db {
        for nh in nhs {
            let mut it = nh.target.split(',');
            let ci = it.next().and_then(|s| s.parse::<usize>().ok());
            let fi = it.next().and_then(|s| s.parse::<usize>().ok());
            if let (Some(ci), Some(fi)) = (ci, fi) {
                hits.entry(ci).or_default().entry(fi).or_default().push(nh.result);
            }
        }
    }

    // Contigs that received at least one hit in *this* search (mirrors Python's
    // `hits.keys()`), used by `hits_only` — independent of pre-existing domains.
    let hit_indices: HashSet<usize> = hits.keys().copied().collect();

    for (i, rec) in records.iter_mut().enumerate() {
        let by_cds = match hits.remove(&i) {
            Some(m) => m,
            None => continue,
        };
        if rec.is_protein() {
            let flat: Vec<SearchResult> = by_cds.into_values().flatten().collect();
            add_protein_annotations(
                rec, flat, opts.max_hits, opts.max_overlap, opts.no_annotations,
                opts.best_annotation, opts.overlap_by_db,
            );
        } else {
            add_nucleic_acid_annotations(
                rec, by_cds, opts.max_hits, opts.max_overlap, opts.no_annotations,
                opts.best_annotation, opts.overlap_by_db,
            );
            add_domain_cds_annotations(rec);
        }
    }

    if opts.hits_only {
        let mut idx = 0usize;
        records.retain(|_| {
            let keep = hit_indices.contains(&idx);
            idx += 1;
            keep
        });
    }
    records
}

/// Load HMM reference databases from `.hmm` files.
pub fn read_hmm_references(files: &[String]) -> std::io::Result<Vec<HmmDb>> {
    load_hmm_dbs(files)
}
