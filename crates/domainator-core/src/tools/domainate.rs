//! `domainate`: annotate sequences with hits to reference HMMs / sequences.
//!
//! Protein references (`.hmm` -> hmmsearch, protein FASTA -> phmmer) search the
//! input peptides (whole protein records or translated CDSs); nucleotide
//! references (nucleotide FASTA/`.hmm` -> nhmmer) search whole nucleotide contigs
//! on both strands, as do Infernal covariance models (`.cm` -> cmsearch); both
//! annotate contig-level hits (`cds_id="."`, circular origin-wrap handled as a
//! `join`). foldseek remains deferred.

use std::collections::{HashMap, HashSet};

use super::gene_call::{call_genes, clear_cds_annotations, GeneCall, GeneCallScope};
use crate::search::hmmer::hmmsearch_db;
use crate::search::infernal::{cmsearch_db, CmDb};
use crate::search::nhmmer::nhmmer_db;
use crate::search::references::{read_references, References};
use crate::search::SearchResult;
use crate::seq::partial::{
    codon_offset, feature_fragment_status, on_contig_cds_translation, record_fragment_status,
    Partial,
};
use crate::seq::taxid::record_taxid;
use crate::seq::translate::is_searchable_cds;
use crate::seq::Topology;
use crate::seq::{Location, SeqFeature, SeqRecord};
use crate::{DOMAIN_FEATURE_NAME, DOMAIN_SEARCH_BEST_HIT_NAME};

/// The longest protein upstream searches (`domainate.MAX_PROTEIN_SIZE`); a longer one
/// is skipped with a warning.
const MAX_PROTEIN_SIZE: usize = 100_000;
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

/// `clean_rec` for a nucleotide record: assign `cds_id` to every non-pseudo CDS and
/// set its `/translation` to the on-contig translation (`utils.normalize_cds_translation`):
/// translated from the contig at its `/codon_start` frame when absent or empty, and
/// trimmed when a partial CDS carries residues encoded off the edge of the contig.
/// Mutates in place; returns the CDS count.
fn clean_rec_nucleotide(rec: &mut SeqRecord) -> usize {
    let seq = rec.seq.clone();
    let mut count = 0;
    for feature in &mut rec.features {
        if is_searchable_cds(feature) {
            let cds_id = cds_unique_name(feature);
            feature.set("cds_id", cds_id.clone());
            count += 1;
            let (translation, both_ends_partial) = on_contig_cds_translation(feature, &seq);
            if both_ends_partial {
                crate::log::warn(format_args!(
                    "Translation of partial CDS {cds_id} is longer than its location, and both \
                     ends are partial. Re-translating from the contig."
                ));
            }
            feature.set("translation", translation);
        }
    }
    count
}

/// Map a protein-coordinate domain onto a CDS's nucleotide location: `length` bases
/// starting `offset` bases into the CDS (port of `SimpleLocation`/
/// `CompoundLocation.overlay`). `None` where Python raises -- a negative offset or
/// length, or a span running past the end of the location.
fn overlay(loc: &Location, mut offset: i64, mut length: i64) -> Option<Location> {
    if offset < 0 || length < 0 || offset + length > loc.len() {
        return None;
    }
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
    Some(if new_parts.len() == 1 {
        Location {
            operator: None,
            between: false,
            parts: new_parts,
        }
    } else {
        Location {
            operator: loc.operator,
            between: false,
            parts: new_parts,
        }
    })
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
    let contig_name = contig.name.clone();
    for fid in feature_ids {
        let mut hits = hits_by_cds.remove(&fid).unwrap();
        hits.sort_by(|a, b| b.score.total_cmp(&a.score));
        let cds_loc = contig.features[fid].location.clone();
        let cds_id = contig.features[fid]
            .get_first("cds_id")
            .unwrap_or("")
            .to_string();
        // Residue 0 starts `codon_start - 1` bases into the CDS.
        let frame = codon_offset(&contig.features[fid]);

        // `None` when the hit cannot be placed on the CDS (Python warns and skips it).
        let place = |hit: &SearchResult, ftype: &str| -> Option<SeqFeature> {
            let annot_len = (hit.end - hit.start) * 3;
            let Some(loc) = overlay(&cds_loc, frame + hit.start * 3, annot_len) else {
                crate::log::warn(format_args!(
                    "Could not overlay location for {} on {}, {cds_id}. Skipping annotation.",
                    hit.name, contig_name
                ));
                return None;
            };
            let mut f = hit.to_feature(ftype, &cds_id);
            f.location = loc;
            Some(f)
        };
        if best_annotation {
            // Python `continue`s past the whole CDS when the best hit cannot be placed.
            match place(&hits[0], DOMAIN_SEARCH_BEST_HIT_NAME) {
                Some(f) => to_append.push(f),
                None => continue,
            }
        }
        if !no_annotations {
            let capped: Vec<SearchResult> = hits.into_iter().take(max_hits).collect();
            for hit in filter_by_overlap(capped, max_overlap, overlap_by_db) {
                to_append.extend(place(&hit, DOMAIN_FEATURE_NAME));
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
            f.get_first("cds_id"),
            f.get_first("database"),
            f.get_first("name"),
            f.get_first("evalue"),
            f.get_first("score"),
            f.get_first("description"),
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
        f.qualifiers
            .retain(|(k, _)| !k.starts_with("domainator_") || seen.contains(k));
    }
}

/// The fraction of half-open `r2` that overlaps `r1` (`locations.overlap_fraction`).
fn overlap_fraction(r1: (i64, i64), r2: (i64, i64)) -> f64 {
    let overlap = (r1.1.min(r2.1) - r1.0.max(r2.0)).max(0);
    if overlap == 0 {
        0.0
    } else {
        overlap as f64 / (r2.1 - r2.0) as f64
    }
}

/// Greedily keep the highest-scoring non-overlapping domains (port of
/// `filter_by_overlap`). Input is consumed; assumed already sorted by score desc.
fn filter_by_overlap(
    hits: Vec<SearchResult>,
    allowed_overlap: f64,
    by_db: bool,
) -> Vec<SearchResult> {
    // Preserve per-db insertion order, matching the Python dict-of-lists flatten.
    let mut order: Vec<Option<String>> = Vec::new();
    let mut kept: HashMap<Option<String>, Vec<SearchResult>> = HashMap::new();
    for hit in hits {
        let key = if by_db {
            Some(hit.database.clone())
        } else {
            None
        };
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
            // Same rule as filter_domains: overlapping a kept hit by more than the
            // allowed fraction rejects a hit.
            .filter(|k| overlap_fraction((k.start, k.end), (hit.start, hit.end)) <= allowed_overlap)
            .count();
        if non_overlap == bucket.len() {
            bucket.push(hit);
        }
    }
    order
        .into_iter()
        .flat_map(|k| kept.remove(&k).unwrap_or_default())
        .collect()
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
        contig
            .features
            .push(hits[0].to_feature(DOMAIN_SEARCH_BEST_HIT_NAME, &cds_id));
    }
    if !no_annotations {
        let capped: Vec<SearchResult> = hits.into_iter().take(max_hits).collect();
        for hit in filter_by_overlap(capped, max_overlap, overlap_by_db) {
            contig
                .features
                .push(hit.to_feature(DOMAIN_FEATURE_NAME, &cds_id));
        }
    }
}

/// The forward `[start, end)` segments a (possibly origin-spanning) contig hit
/// covers on a circular contig (port of `circular_hit_segments`).
fn circular_hit_segments(start: i64, end: i64, contig_length: i64) -> Vec<(i64, i64)> {
    if end <= contig_length {
        vec![(start, end)]
    } else {
        vec![(start, contig_length), (0, end - contig_length)]
    }
}

/// Whether `container`'s segments fully cover `contained`'s (port of
/// `circular_hit_contains`). Used to drop origin-wrap duplicate hits.
fn circular_hit_contains(
    container: &SearchResult,
    contained: &SearchResult,
    contig_length: i64,
) -> bool {
    let container_segments = circular_hit_segments(container.start, container.end, contig_length);
    for cs in circular_hit_segments(contained.start, contained.end, contig_length) {
        if !container_segments
            .iter()
            .any(|seg| seg.0 <= cs.0 && seg.1 >= cs.1)
        {
            return false;
        }
    }
    true
}

/// Drop origin-wrap duplicate hits on a circular contig (port of
/// `deduplicate_contig_hits`): keep the highest-scoring, skip any later hit of the
/// same name/database/program contained by a kept one. No-op for linear contigs.
fn deduplicate_contig_hits(
    topology: Topology,
    contig_length: i64,
    mut hits: Vec<SearchResult>,
) -> Vec<SearchResult> {
    if !matches!(topology, Topology::Circular) {
        return hits;
    }
    hits.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut kept: Vec<SearchResult> = Vec::new();
    for hit in hits {
        let dup = kept.iter().any(|existing| {
            existing.name == hit.name
                && existing.database == hit.database
                && existing.program == hit.program
                && circular_hit_contains(existing, &hit, contig_length)
        });
        if !dup {
            kept.push(hit);
        }
    }
    kept
}

/// Build a contig-level hit's feature location (port of `build_contig_hit_location`):
/// an origin-spanning hit on a circular contig becomes a `join` of two ranges,
/// otherwise a single clamped range.
fn build_contig_hit_location(
    topology: Topology,
    contig_length: i64,
    hit: &SearchResult,
) -> Location {
    if matches!(topology, Topology::Circular) && hit.end > contig_length {
        Location {
            operator: Some(crate::seq::location::CompoundOp::Join),
            between: false,
            parts: vec![
                (hit.start, contig_length, hit.strand, false, false),
                (0, hit.end - contig_length, hit.strand, false, false),
            ],
        }
    } else {
        Location::range(hit.start, hit.end.min(contig_length), hit.strand)
    }
}

/// Emit a contig-level `Domainator`/best-hit feature for `hit` (cds_id = ".").
fn contig_hit_feature(
    topology: Topology,
    contig_length: i64,
    hit: &SearchResult,
    ftype: &str,
) -> SeqFeature {
    let mut f = hit.to_feature(ftype, ".");
    f.location = build_contig_hit_location(topology, contig_length, hit);
    f
}

/// Add whole-contig `Domainator` (+ optional best-hit) features from nhmmer hits
/// (port of `add_contig_nucleic_acid_annotations`). Hits are deduplicated for
/// origin-wrap on circular contigs, then sorted by score desc; features carry
/// `cds_id="."`.
// One argument over clippy's threshold, and deliberately the same shape as its
// sibling `add_nucleic_acid_annotations` (which has seven); taking `&DomainateOptions`
// in one but not the other would hide that they are the same operation.
#[allow(clippy::too_many_arguments)]
fn add_contig_nucleic_acid_annotations(
    contig: &mut SeqRecord,
    hits_list: Vec<SearchResult>,
    max_hits: usize,
    max_overlap: f64,
    no_annotations: bool,
    best_annotation: bool,
    overlap_by_db: bool,
    max_hits_per_contig: Option<usize>,
) {
    let topology = contig.topology;
    let contig_length = contig.len() as i64;
    let mut hits_list = deduplicate_contig_hits(topology, contig_length, hits_list);
    if hits_list.is_empty() {
        return;
    }
    hits_list.sort_by(|a, b| b.score.total_cmp(&a.score));

    if best_annotation {
        let capped: Vec<SearchResult> = match max_hits_per_contig {
            Some(n) => hits_list.iter().take(n).cloned().collect(),
            None => hits_list.clone(),
        };
        for hit in filter_by_overlap(capped, max_overlap, overlap_by_db) {
            contig.features.push(contig_hit_feature(
                topology,
                contig_length,
                &hit,
                DOMAIN_SEARCH_BEST_HIT_NAME,
            ));
        }
    }

    if no_annotations {
        return;
    }

    let capped: Vec<SearchResult> = hits_list.into_iter().take(max_hits).collect();
    for hit in filter_by_overlap(capped, max_overlap, overlap_by_db) {
        contig.features.push(contig_hit_feature(
            topology,
            contig_length,
            &hit,
            DOMAIN_FEATURE_NAME,
        ));
    }
}

/// The nucleotide records as whole-contig search targets (`get_contig_search_sequence`):
/// `"{i},contig,{len}"` and the residues, a circular contig followed by its first
/// `extension` bases again.
fn whole_contig_targets(records: &[SeqRecord], extension: usize) -> Vec<(String, Vec<u8>)> {
    let mut targets: Vec<(String, Vec<u8>)> = Vec::new();
    for (i, rec) in records.iter().enumerate() {
        if rec.is_protein() {
            continue;
        }
        let mut seq: Vec<u8> = rec
            .seq
            .iter()
            .copied()
            .filter(|b| b.is_ascii_alphabetic())
            .collect();
        if matches!(rec.topology, Topology::Circular) && extension > 0 {
            let ext = extension.min(seq.len());
            let prefix: Vec<u8> = seq[..ext].to_vec();
            seq.extend_from_slice(&prefix);
        }
        targets.push((format!("{i},contig,{}", rec.len()), seq));
    }
    targets
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
    /// Worker threads for the profile and database searches (`0` = all cores).
    pub cpu: usize,
    pub parallel_profiles: bool,
    /// Cap on independent whole-contig (nhmmer) best hits per contig (`None` =
    /// unlimited). Only affects the contig-level best-annotation path.
    pub max_hits_per_contig: Option<usize>,
    /// `--partial`: which proteins/CDSs are searched, by fragment status. Records
    /// are written either way; a filtered protein/CDS is just never searched.
    pub partial: Partial,
    /// `--gene_call`: call genes (with Orphos) on nucleotide contigs before
    /// searching. `None` keeps the existing CDS annotations as they are.
    pub gene_call: Option<GeneCall>,
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
            parallel_profiles: true,
            max_hits_per_contig: None,
            partial: Partial::Include,
            gene_call: None,
        }
    }
}

/// Annotate `records` with hits from `refs`. Protein references (hmmsearch /
/// phmmer) search the input peptides (whole protein records or translated CDSs);
/// nucleotide references (nhmmer) search whole nucleotide contigs on both strands.
/// Returns the annotated records (all, or hit-only when `hits_only`); fails only
/// when gene calling does (`opts.gene_call`).
pub fn domainate(
    mut records: Vec<SeqRecord>,
    refs: &References,
    opts: &DomainateOptions,
) -> std::io::Result<Vec<SeqRecord>> {
    let empty_dropped = HashSet::new();

    // Taxonomy pre-filter: drop records whose lineage is not in the allowed set
    // (mirrors Python `filter_by_taxonomy` applied to the input records).
    if let Some(allowed) = &opts.allowed_taxids {
        records.retain(|rec| allowed.contains(&record_taxid(rec)));
    }

    // clean_rec: nucleotide records get cds_id + /translation on every CDS. With
    // `--gene_call all` every CDS (and the domains anchored on them) goes first; then
    // the nucleotide records left with no CDS get genes called, as in Python, where
    // `prodigal_CDS_annotate` runs right after `clean_rec` when it counts no CDS.
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

    // Protein search (hmmsearch + phmmer). Collect peptides across contigs.
    // Protein record: "{i},0". Nucleotide record's CDSs: "{i},{feature_index}".
    let mut peptides: Vec<(String, Vec<u8>)> = Vec::new();
    // Upstream skips a protein too long to search, with a warning.
    let mut add = |name: String, pep: Vec<u8>, rec: &SeqRecord| {
        if pep.len() > MAX_PROTEIN_SIZE {
            crate::log::warn(format_args!(
                "Skipping protein longer than {MAX_PROTEIN_SIZE} aa: {}",
                rec.id
            ));
        } else {
            peptides.push((name, pep));
        }
    };
    for (i, rec) in records.iter().enumerate() {
        if rec.is_protein() {
            if !opts.partial.allows(record_fragment_status(rec)) {
                continue;
            }
            let pep: Vec<u8> = rec
                .seq
                .iter()
                .copied()
                .filter(|b| b.is_ascii_alphanumeric())
                .collect();
            add(format!("{i},0"), pep, rec);
        } else {
            for (j, pep) in rec.cds_peptides(&empty_dropped) {
                if !opts
                    .partial
                    .allows(feature_fragment_status(&rec.features[j]))
                {
                    continue;
                }
                add(format!("{i},{j}"), pep.into_bytes(), rec);
            }
        }
    }

    // contig index -> (cds feature index -> SearchResults). Independent profiles
    // and databases run in parallel for domainate; domain_search processes its
    // partitions in parallel instead. Merge in database and profile order.
    let mut hits: HashMap<usize, HashMap<usize, Vec<SearchResult>>> = HashMap::new();
    if !refs.protein.is_empty() && !peptides.is_empty() {
        let per_db: Vec<Vec<crate::search::hmmer::NamedHit>> =
            crate::parallel::with_threads(opts.cpu, || {
                use rayon::prelude::*;
                let search_db = |db: &crate::search::hmmer::HmmDb| {
                    hmmsearch_db(
                        db,
                        &peptides,
                        opts.z,
                        opts.evalue,
                        opts.min_evalue,
                        opts.max_mode,
                        opts.parallel_profiles,
                    )
                };
                if opts.parallel_profiles {
                    refs.protein.par_iter().map(search_db).collect()
                } else {
                    refs.protein.iter().map(search_db).collect()
                }
            });
        for nhs in per_db {
            for nh in nhs {
                let mut it = nh.target.split(',');
                let ci = it.next().and_then(|s| s.parse::<usize>().ok());
                let fi = it.next().and_then(|s| s.parse::<usize>().ok());
                if let (Some(ci), Some(fi)) = (ci, fi) {
                    hits.entry(ci)
                        .or_default()
                        .entry(fi)
                        .or_default()
                        .push(nh.result);
                }
            }
        }
    }

    // Nucleic-acid searches (nhmmer, then cmsearch) over whole contigs, both
    // strands. Each builds its own targets, named "{i},contig,{len}": circular
    // contigs are extended by that program's longest reference so origin-spanning
    // hits are found.
    let mut contig_hits: HashMap<usize, Vec<SearchResult>> = HashMap::new();
    let mut add_contig_hits = |per_db: Vec<Vec<crate::search::hmmer::NamedHit>>| {
        for nhs in per_db {
            for nh in nhs {
                if let Some(ci) = nh
                    .target
                    .split(',')
                    .next()
                    .and_then(|s| s.parse::<usize>().ok())
                {
                    contig_hits.entry(ci).or_default().push(nh.result);
                }
            }
        }
    };
    if !refs.nucleotide.is_empty() {
        let nhmmer_extension: usize = refs
            .nucleotide
            .iter()
            .flat_map(|db| db.hmms.iter().map(|h| h.m))
            .max()
            .unwrap_or(0);
        let targets = whole_contig_targets(&records, nhmmer_extension);
        if !targets.is_empty() {
            let per_db: Vec<Vec<crate::search::hmmer::NamedHit>> =
                crate::parallel::with_threads(opts.cpu, || {
                    use rayon::prelude::*;
                    refs.nucleotide
                        .par_iter()
                        .map(|db| {
                            nhmmer_db(db, &targets, opts.evalue, opts.min_evalue, opts.max_mode)
                        })
                        .collect()
                });
            add_contig_hits(per_db);
        }
    }
    if !refs.infernal.is_empty() {
        // Python extends by the largest `CM.M`, the model's state count.
        let infernal_extension: usize = refs
            .infernal
            .iter()
            .map(CmDb::max_states)
            .max()
            .unwrap_or(0);
        let targets = whole_contig_targets(&records, infernal_extension);
        if !targets.is_empty() {
            // Each search spreads its own windows over the pool, so databases run
            // one after another.
            let per_db = crate::parallel::with_threads(opts.cpu, || {
                refs.infernal
                    .iter()
                    .map(|db| cmsearch_db(db, &targets, opts.evalue, opts.min_evalue))
                    .collect::<std::io::Result<Vec<_>>>()
            })?;
            add_contig_hits(per_db);
        }
    }

    // Contigs that received at least one hit in *this* search (mirrors Python's
    // `hits.keys()`), used by `hits_only` — independent of pre-existing domains.
    let mut hit_indices: HashSet<usize> = hits.keys().copied().collect();
    hit_indices.extend(contig_hits.keys().copied());

    for (i, rec) in records.iter_mut().enumerate() {
        let by_cds = hits.remove(&i);
        let ncontig = contig_hits.remove(&i);
        if by_cds.is_none() && ncontig.is_none() {
            continue;
        }
        if rec.is_protein() {
            if let Some(by_cds) = by_cds {
                let flat: Vec<SearchResult> = by_cds.into_values().flatten().collect();
                add_protein_annotations(
                    rec,
                    flat,
                    opts.max_hits,
                    opts.max_overlap,
                    opts.no_annotations,
                    opts.best_annotation,
                    opts.overlap_by_db,
                );
            }
        } else {
            if let Some(chits) = ncontig {
                add_contig_nucleic_acid_annotations(
                    rec,
                    chits,
                    opts.max_hits,
                    opts.max_overlap,
                    opts.no_annotations,
                    opts.best_annotation,
                    opts.overlap_by_db,
                    opts.max_hits_per_contig,
                );
            }
            if let Some(by_cds) = by_cds {
                add_nucleic_acid_annotations(
                    rec,
                    by_cds,
                    opts.max_hits,
                    opts.max_overlap,
                    opts.no_annotations,
                    opts.best_annotation,
                    opts.overlap_by_db,
                );
            }
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
    Ok(records)
}

/// Load and classify reference files (protein hmmsearch/phmmer + nucleotide
/// nhmmer). Nucleotide references cannot be combined with protein input; that is
/// validated by the caller once the input molecule type is known.
pub fn read_hmm_references(files: &[String]) -> std::io::Result<References> {
    read_references(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seq::location::CompoundOp;

    /// `test_simple_location_overlay_bounds`: the bounds are relative to the
    /// feature, not to its position on the contig.
    #[test]
    fn overlay_bounds() {
        for (start, end, strand) in [(1000, 1300, 1), (1000, 1300, -1), (100, 400, 1)] {
            let loc = Location::range(start, end, strand);
            assert_eq!(overlay(&loc, 0, 300), Some(loc.clone()));
            let expected = if strand == -1 {
                Location::range(1210, 1270, -1)
            } else {
                Location::range(start + 30, start + 90, 1)
            };
            assert_eq!(overlay(&loc, 30, 60), Some(expected));
            for (offset, length) in [(150, 300), (0, 301), (-1, 10), (0, -1)] {
                assert_eq!(overlay(&loc, offset, length), None, "{offset} {length}");
            }
        }
    }

    /// Upstream `filter_by_overlap`: a hit is rejected only when it overlaps a better
    /// kept hit by *more* than the allowed fraction of its own length.
    #[test]
    fn max_overlap_rejects_only_overlaps_above_the_fraction() {
        let at = |start, end| SearchResult {
            start,
            end,
            ..Default::default()
        };
        // The second hit overlaps the first by 5 of its 10 positions.
        let kept = |allowed| filter_by_overlap(vec![at(0, 10), at(5, 15)], allowed, false).len();
        assert_eq!(kept(0.5), 2);
        assert_eq!(kept(0.4), 1);
        assert_eq!(kept(0.0), 1);
        // Touching is not overlapping.
        assert_eq!(
            filter_by_overlap(vec![at(0, 10), at(10, 20)], 0.0, false).len(),
            2
        );
    }

    fn hit(start: i64, end: i64, strand: i32, name: &str) -> SearchResult {
        SearchResult {
            name: name.into(),
            desc: String::new(),
            acc: String::new(),
            evalue: 1e-10,
            score: 50.0,
            start,
            end,
            database: "db".into(),
            identity: 100.0,
            rstart: 1,
            rend: 10,
            rlen: 10,
            program: "nhmmer".into(),
            strand,
            ..SearchResult::default()
        }
    }

    #[test]
    fn contig_hit_location_linear_clamps_to_length() {
        let h = hit(5, 20, 1, "q");
        let loc = build_contig_hit_location(Topology::Linear, 15, &h);
        assert!(loc.operator.is_none());
        assert_eq!(loc.parts, vec![(5, 15, 1, false, false)]);
    }

    #[test]
    fn contig_hit_location_circular_wrap_becomes_join() {
        // On a length-100 circular contig, a hit ending at 110 spans the origin.
        let h = hit(90, 110, 1, "q");
        let loc = build_contig_hit_location(Topology::Circular, 100, &h);
        assert_eq!(loc.operator, Some(CompoundOp::Join));
        assert_eq!(
            loc.parts,
            vec![(90, 100, 1, false, false), (0, 10, 1, false, false)]
        );
    }

    #[test]
    fn contig_hit_location_circular_no_wrap_is_simple_range() {
        let h = hit(10, 50, -1, "q");
        let loc = build_contig_hit_location(Topology::Circular, 100, &h);
        assert!(loc.operator.is_none());
        assert_eq!(loc.parts, vec![(10, 50, -1, false, false)]);
    }

    #[test]
    fn circular_dedup_drops_origin_wrap_duplicate() {
        // A full-length hit (higher score) contains a wrapped duplicate of the
        // same name/db/program; the duplicate is dropped on a circular contig.
        let mut big = hit(0, 100, 1, "q");
        big.score = 60.0;
        let wrapped = hit(90, 110, 1, "q"); // segments (90,100),(0,10) ⊆ (0,100)
        let kept = deduplicate_contig_hits(Topology::Circular, 100, vec![big, wrapped]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].score, 60.0);
    }

    #[test]
    fn circular_dedup_is_noop_on_linear() {
        let a = hit(0, 100, 1, "q");
        let b = hit(90, 110, 1, "q");
        let kept = deduplicate_contig_hits(Topology::Linear, 100, vec![a, b]);
        assert_eq!(kept.len(), 2);
    }
}
