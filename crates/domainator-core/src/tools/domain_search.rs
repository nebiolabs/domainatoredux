//! `domain_search`: search a few references against a large (possibly sharded,
//! BGZF-compressed, `.didx`-indexed) database, returning the hit contigs.
//!
//! Each partition is seek-read independently and run through the `domainate` core
//! with `hits_only=true, best_annotation=true`. Nucleotide hits are then passed
//! through neighborhood extraction (`--cds_range`/`--kb_range`/`--whole_contig`,
//! default: the hit CDS), mirroring the Python `select_by_cds` step.

use std::collections::{BTreeSet, HashSet};
use std::path::Path;
use std::sync::Arc;

use crate::io::partition::partition_seqfile;
use crate::io::shards::resolve_database_shards;
use crate::io::{
    detect_compression, detect_filetype, fasta, genbank::GenbankReader, Compression, FileType,
};
use crate::search::references::References;
use crate::seq::partial::Partial;
use crate::seq::taxonomy::TaxonomyFilter;
use crate::seq::SeqRecord;
use crate::tools::dedup::{Dedup, RecordDeduplicator, TopHits};
use crate::tools::domainate::{domainate, DomainateOptions};
use crate::tools::gene_call::GeneCall;
use crate::tools::neighborhood::{select_by_cds_hits, Range, Region};
use crate::tools::select_by_cds::{pad_records, Anchor};
use crate::{DOMAIN_FEATURE_NAME, DOMAIN_SEARCH_BEST_HIT_NAME};

/// Options for [`domain_search`].
pub struct DomainSearchOptions {
    pub z: f64,
    pub evalue: f64,
    pub min_evalue: f64,
    pub max_overlap: f64,
    /// Add full `Domainator` annotations, not just the best-hit `Domain_Search`.
    pub add_annotations: bool,
    /// Cap on total hit contigs returned (top-N by best domain score). `None` = all.
    pub max_hits: Option<usize>,
    /// Approximate CDSs per partition (also the partition granularity).
    pub batch_size: u64,
    /// Default molecule type for inputs lacking one (FASTA / unit-less LOCUS).
    pub alphabet: String,
    /// Worker threads (`0` = all available cores).
    pub cpu: usize,
    /// Neighborhood extraction for nucleotide hits (`Whole` = whole contig).
    pub range: Range,
    /// Reverse-complement reverse-strand neighborhoods so the hit reads forward.
    pub normalize_direction: bool,
    /// Restrict hit CDSs to a strand (`Some(1)`/`Some(-1)`); `None` = both.
    pub strand: Option<i32>,
    /// Max fractional overlap between emitted regions (`>= 1` disables).
    pub max_region_overlap: f64,
    /// Translate nucleotide CDS hits to peptide records (incompatible with
    /// neighborhood extraction; `range` must be `None`).
    pub translate: bool,
    /// hmmsearch `--max` (skip heuristic filters for maximum sensitivity).
    pub max_mode: bool,
    /// Taxonomy restriction on the searched proteins/CDSs and nucleotide hits, applied
    /// before ranking (`None` = none). The neighborhood around a hit is not filtered.
    pub taxonomy: Option<Arc<TaxonomyFilter>>,
    /// Decoy domain names: if a hit CDS's best hit is a decoy, that hit is dropped.
    pub decoys: Option<HashSet<String>>,
    /// Cap on independent hits emitted per contig (`None` = all).
    pub max_hits_per_contig: Option<usize>,
    /// `--pad`: pad each region so the focus CDSs align at `anchor`. Buffers the
    /// whole result and emits it in score order.
    pub pad: bool,
    pub anchor: Anchor,
    /// `--dedup`: which output records count as copies. With `max_hits`, the
    /// best-scoring copy is the one kept; otherwise the first.
    pub dedup: Dedup,
    /// `--partial`: which proteins/CDSs are searched, by fragment status.
    pub partial: Partial,
    /// `--gene_call`: call genes (with Orphos) on each partition's nucleotide
    /// contigs before searching them (see [`DomainateOptions::gene_call`]).
    pub gene_call: Option<GeneCall>,
}

impl Default for DomainSearchOptions {
    fn default() -> Self {
        DomainSearchOptions {
            z: 1000.0,
            evalue: 0.001,
            min_evalue: 0.0,
            max_overlap: 1.0,
            add_annotations: false,
            max_hits: None,
            batch_size: 10000,
            alphabet: "protein".to_string(),
            cpu: 0,
            range: Range::None,
            normalize_direction: true,
            strand: None,
            max_region_overlap: 1.0,
            translate: false,
            max_mode: false,
            taxonomy: None,
            decoys: None,
            max_hits_per_contig: None,
            pad: false,
            anchor: Anchor::Start,
            dedup: Dedup::None,
            partial: Partial::Include,
            gene_call: None,
        }
    }
}

/// The best-hit domain name for a protein record (its `Domain_Search` feature's
/// `name`, else the highest-scoring `Domainator` feature's `name`).
fn best_hit_name(rec: &SeqRecord) -> Option<String> {
    if let Some(f) = rec
        .features
        .iter()
        .find(|f| f.kind == DOMAIN_SEARCH_BEST_HIT_NAME)
    {
        return f.get_first("name").map(str::to_string);
    }
    rec.features
        .iter()
        .filter(|f| f.kind == DOMAIN_FEATURE_NAME)
        .max_by(|a, b| {
            let sa = a
                .get_first("score")
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(f64::NEG_INFINITY);
            let sb = b
                .get_first("score")
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(f64::NEG_INFINITY);
            sa.total_cmp(&sb)
        })
        .and_then(|f| f.get_first("name").map(str::to_string))
}

/// The **focus CDS's** hit score — Python's `hit_best_score`, which is what
/// `--max_hits` ranks by. `-inf` if the record carries no hit feature.
///
/// Only the `Domain_Search` best-hit feature counts, never the `Domainator`
/// annotations. An extracted region spans the focus CDS *and its neighbors*, so with
/// `--cds_range`/`--kb_range` and `--add_annotations` it carries the neighbors' domain
/// annotations too; taking the maximum over those ranks a weak hit by whatever strong
/// gene happens to sit next to it. On `pDONR201_multi_genemark.gb` that made all 16
/// regions tie at the strongest neighbor's 329.9, so `--max_hits 5` returned five
/// arbitrary regions instead of the five best.
///
/// `select_by_cds_hits` attaches exactly one `Domain_Search` feature per emitted
/// region — the focus CDS's own (`if *cid == focus_cid`), or the single contig hit on
/// the nucleic-acid path — so this reads the focus score by construction.
fn best_score(rec: &SeqRecord) -> f64 {
    rec.features
        .iter()
        .filter(|f| f.kind == DOMAIN_SEARCH_BEST_HIT_NAME)
        .filter_map(|f| f.get_first("score").and_then(|s| s.parse::<f64>().ok()))
        .fold(f64::NEG_INFINITY, f64::max)
}

fn invalid_input(e: String) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, e)
}

/// Upstream `TopHits` over regions, ranked by [`best_score`].
fn top_hits(max_hits: usize, dedup: Dedup) -> std::io::Result<TopHits<Region>> {
    TopHits::new(max_hits, dedup).map_err(invalid_input)
}

fn add_hit(top: &mut TopHits<Region>, region: Region) -> std::io::Result<()> {
    let score = best_score(&region.record);
    top.add(region, |r| &r.record, score).map_err(invalid_input)
}

/// The molecule types of `input_files` (each resolved to its shards), read off the
/// first record of each: upstream `get_input_molecule_types`.
pub fn input_molecule_types(
    input_files: &[String],
    alphabet: &str,
) -> std::io::Result<BTreeSet<String>> {
    let mut types = BTreeSet::new();
    for input in input_files {
        for (path, filetype, compression) in shard_files(input)? {
            let compressed = compression == Compression::Bgzf;
            let first = read_partition(&path, 0, 1, filetype, compressed, alphabet)?;
            types.extend(first.into_iter().filter_map(|r| r.molecule_type));
        }
    }
    Ok(types)
}

/// `input`'s shards, each with its file type and compression.
fn shard_files(input: &str) -> std::io::Result<Vec<(String, FileType, Compression)>> {
    let shards = resolve_database_shards(Path::new(input))
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    shards
        .into_iter()
        .map(|shard| {
            let path = shard.to_str().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "non-UTF8 shard path")
            })?;
            let filetype = detect_filetype(&shard).ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("could not detect file type: {}", shard.display()),
                )
            })?;
            Ok((path.to_string(), filetype, detect_compression(&shard)))
        })
        .collect()
}

/// Read one partition's records (seek-and-read; honors BGZF virtual offsets).
fn read_partition(
    path: &str,
    offset: u64,
    n_records: u64,
    filetype: FileType,
    compressed: bool,
    default_mol: &str,
) -> std::io::Result<Vec<SeqRecord>> {
    match filetype {
        FileType::Genbank => {
            GenbankReader::open(path, offset, compressed, Some(default_mol.to_string()))?
                .take(n_records as usize)
                .collect()
        }
        FileType::Fasta => {
            fasta::read_fasta_partition(path, offset, compressed, n_records, Some(default_mol))
        }
    }
}

/// Search `hmm_dbs` against the `input_files` database(s), returning hit contigs.
pub fn domain_search(
    input_files: &[String],
    refs: &References,
    opts: &DomainSearchOptions,
) -> std::io::Result<Vec<SeqRecord>> {
    if opts.dedup == Dedup::SequenceRc && opts.translate {
        return Err(invalid_input(
            "--dedup sequence_rc is only supported for nucleotide output.".into(),
        ));
    }
    if opts.max_hits == Some(0) {
        return Err(invalid_input("max_hits must be at least 1.".into()));
    }
    // Collect every partition across all inputs/shards, then process them in
    // parallel (each is read independently via its own file handle / BGZF offset).
    let mut parts = Vec::new();
    let mut total_cds: u64 = 0;
    for input in input_files {
        for (shard, filetype, compression) in shard_files(input)? {
            let (shard_total, shard_parts) =
                partition_seqfile(&shard, filetype, compression, opts.batch_size)?;
            total_cds += shard_total;
            parts.extend(shard_parts);
        }
    }

    // `-Z 0` means "use the actual total target count" for per-sequence E-values
    // (the `.didx` headers / offset scan already gave us the per-shard totals).
    let z = if opts.z == 0.0 {
        total_cds as f64
    } else {
        opts.z
    };

    let dopts = DomainateOptions {
        z,
        evalue: opts.evalue,
        min_evalue: opts.min_evalue,
        max_overlap: opts.max_overlap,
        best_annotation: true,
        no_annotations: !opts.add_annotations,
        hits_only: true,
        max_mode: opts.max_mode,
        taxonomy: opts.taxonomy.clone(),
        max_hits_per_contig: opts.max_hits_per_contig,
        // `cpu` is left at 0: domainate runs inside the partition pool below, where its
        // `with_threads` calls run in that pool instead of building their own.
        parallel_profiles: false,
        partial: opts.partial,
        // Partitions already run in parallel, so each Orphos run gets one thread unless
        // there is only the one partition.
        gene_call: opts.gene_call.clone().map(|mut gene_call| {
            if parts.len() > 1 {
                gene_call.options.cpu = 1;
            }
            gene_call
        }),
        ..Default::default()
    };

    // Parallel map preserves partition order; results are concatenated in order.
    let per_part: Vec<std::io::Result<Vec<Region>>> =
        crate::parallel::with_threads(opts.cpu, || {
            use rayon::prelude::*;
            parts
                .par_iter()
                .map(|p| {
                    let records = read_partition(
                        &p.path,
                        p.offset,
                        p.n_records,
                        p.filetype,
                        p.compressed,
                        &opts.alphabet,
                    )?;
                    let mut emitted = Vec::new();
                    for rec in domainate(records, refs, &dopts)? {
                        // Protein hits pass through whole; nucleotide hits go through
                        // neighborhood extraction.
                        if rec.molecule_type.as_deref() == Some("protein") {
                            // Decoy filter: drop the protein if its best hit is a decoy.
                            if let Some(decoys) = &opts.decoys {
                                if best_hit_name(&rec).is_some_and(|n| decoys.contains(&n)) {
                                    continue;
                                }
                            }
                            // A whole-protein hit is not a neighborhood, so there is
                            // nothing to align: `--pad` only right-pads these to a common
                            // length, and the CLI refuses the combination anyway.
                            emitted.push(Region::whole(rec));
                        } else if opts.translate {
                            emitted.extend(
                                crate::tools::neighborhood::translate_peptides(
                                    &rec,
                                    opts.strand,
                                    opts.decoys.as_ref(),
                                )
                                .into_iter()
                                .map(Region::whole),
                            );
                        } else {
                            emitted.extend(select_by_cds_hits(
                                &rec,
                                opts.range,
                                opts.normalize_direction,
                                opts.strand,
                                opts.max_region_overlap,
                                opts.decoys.as_ref(),
                                opts.max_hits_per_contig,
                            )?);
                        }
                    }
                    // With --max_hits each partition keeps only its own top
                    // `max_hits` unique keys before reaching the parent: a key it drops
                    // already has `max_hits` better ones here, and the merge cannot lose
                    // distinct keys (one ID is one sequence), so this is lossless and
                    // bounds the parent's memory by O(partitions * max_hits).
                    match opts.max_hits {
                        None => Ok(emitted),
                        Some(max_hits) => {
                            let mut top = top_hits(max_hits, opts.dedup)?;
                            for region in emitted {
                                add_hit(&mut top, region)?;
                            }
                            Ok(top.into_sorted())
                        }
                    }
                })
                .collect()
        });

    // Partitions merge in order, so "arrival" (the tie-break, and which copy is
    // first) is deterministic here where upstream's process pool makes it arbitrary.
    let mut out: Vec<Region> = Vec::new();
    match opts.max_hits {
        None => {
            let mut deduplicator = RecordDeduplicator::new(opts.dedup);
            for part_result in per_part {
                for region in part_result? {
                    if deduplicator.keep(&region.record).map_err(invalid_input)? {
                        out.push(region);
                    }
                }
            }
        }
        Some(max_hits) => {
            let mut top = top_hits(max_hits, opts.dedup)?;
            for part_result in per_part {
                for region in part_result? {
                    add_hit(&mut top, region)?;
                }
            }
            out = top.into_sorted();
        }
    }

    // Upstream `deduplicate_and_pad(best_first=True)`. `--pad` aligns the focus CDSs
    // across every region, so it can only run once the whole search is in, and
    // sorts by score first whether or not `--max_hits` is set (a stable sort, so a
    // `--max_hits` order stands).
    if opts.pad {
        out.sort_by(|a, b| best_score(&b.record).total_cmp(&best_score(&a.record)));
        pad_records(&mut out, opts.anchor);
    }
    Ok(out.into_iter().map(|r| r.record).collect())
}

#[cfg(test)]
mod tests {
    use super::best_score;
    use crate::seq::location::Location;
    use crate::seq::{Annotations, SeqFeature, SeqRecord, Topology};
    use crate::{DOMAIN_FEATURE_NAME, DOMAIN_SEARCH_BEST_HIT_NAME};

    fn scored(kind: &str, score: &str) -> SeqFeature {
        let mut f = SeqFeature::new(kind, Location::range(0, 30, 1));
        f.set("score", score.to_string());
        f
    }

    fn region(features: Vec<SeqFeature>) -> SeqRecord {
        SeqRecord {
            id: "r".into(),
            name: "r".into(),
            description: String::new(),
            seq: b"ACGT".to_vec(),
            molecule_type: Some("DNA".into()),
            topology: Topology::Linear,
            annotations: Annotations::default(),
            features,
        }
    }

    /// `--max_hits` ranks by the focus CDS's hit score (Python's `hit_best_score`),
    /// never by a neighbor's annotation. An extracted region spans the focus CDS and
    /// its neighbors, so a weak hit next to a strong gene must still rank weak —
    /// ranking by the maximum over all features made every region on
    /// `pDONR201_multi_genemark.gb` tie at the strongest neighbor's score.
    #[test]
    fn max_hits_ranks_by_the_focus_hit_not_a_neighbors_annotation() {
        let weak_focus_strong_neighbor = region(vec![
            scored(DOMAIN_SEARCH_BEST_HIT_NAME, "50.2"),
            scored(DOMAIN_FEATURE_NAME, "329.9"),
        ]);
        let strong_focus = region(vec![scored(DOMAIN_SEARCH_BEST_HIT_NAME, "103.8")]);

        assert_eq!(best_score(&weak_focus_strong_neighbor), 50.2);
        assert_eq!(best_score(&strong_focus), 103.8);
        assert!(
            best_score(&strong_focus) > best_score(&weak_focus_strong_neighbor),
            "the region with the stronger focus hit must rank higher"
        );
    }

    #[test]
    fn best_score_of_a_region_with_no_hit_feature_is_negative_infinity() {
        assert_eq!(
            best_score(&region(vec![scored(DOMAIN_FEATURE_NAME, "99.0")])),
            f64::NEG_INFINITY
        );
        assert_eq!(best_score(&region(vec![])), f64::NEG_INFINITY);
    }
}
