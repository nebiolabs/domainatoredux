//! `domain_search`: search a few references against a large (possibly sharded,
//! BGZF-compressed, `.didx`-indexed) database, returning the hit contigs.
//!
//! M2 scope: the protein / `--whole_contig` SSN-feeding path. Each partition is
//! seek-read independently and run through the `domainate` core with
//! `hits_only=true, best_annotation=true`; neighborhood extraction
//! (`--cds_range`/`--kb_range`) is deferred.

use std::path::Path;

use crate::io::partition::partition_seqfile;
use crate::io::shards::resolve_database_shards;
use crate::io::{detect_compression, detect_filetype, fasta, genbank::GenbankReader, FileType};
use crate::search::hmmer::HmmDb;
use crate::seq::SeqRecord;
use crate::tools::domainate::{domainate, DomainateOptions};
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
    pub fasta_type: String,
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
            fasta_type: "protein".to_string(),
        }
    }
}

/// A record's best domain bit-score (over its `Domainator`/`Domain_Search`
/// features); `-inf` if it has none. Used for `--max_hits` ranking.
fn best_score(rec: &SeqRecord) -> f64 {
    rec.features
        .iter()
        .filter(|f| f.kind == DOMAIN_FEATURE_NAME || f.kind == DOMAIN_SEARCH_BEST_HIT_NAME)
        .filter_map(|f| f.get_first("score").and_then(|s| s.parse::<f64>().ok()))
        .fold(f64::NEG_INFINITY, f64::max)
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
        FileType::Genbank => GenbankReader::open(path, offset, compressed, Some(default_mol.to_string()))?
            .take(n_records as usize)
            .collect(),
        FileType::Fasta => {
            fasta::read_fasta_partition(path, offset, compressed, n_records, Some(default_mol))
        }
    }
}

/// Search `hmm_dbs` against the `input_files` database(s), returning hit contigs.
pub fn domain_search(
    input_files: &[String],
    hmm_dbs: &[HmmDb],
    opts: &DomainSearchOptions,
) -> std::io::Result<Vec<SeqRecord>> {
    let mut out: Vec<SeqRecord> = Vec::new();

    for input in input_files {
        let shards = resolve_database_shards(Path::new(input))
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        for shard in shards {
            let shard_str = shard.to_str().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "non-UTF8 shard path")
            })?;
            let filetype = detect_filetype(&shard).ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("could not detect file type: {}", shard.display()),
                )
            })?;
            let compression = detect_compression(&shard);
            let (_total, parts) =
                partition_seqfile(shard_str, filetype, compression, opts.batch_size)?;

            for p in parts {
                let records = read_partition(
                    &p.path, p.offset, p.n_records, p.filetype, p.compressed, &opts.fasta_type,
                )?;
                let dopts = DomainateOptions {
                    z: opts.z,
                    evalue: opts.evalue,
                    min_evalue: opts.min_evalue,
                    max_overlap: opts.max_overlap,
                    best_annotation: true,
                    no_annotations: !opts.add_annotations,
                    hits_only: true,
                    ..Default::default()
                };
                out.extend(domainate(records, hmm_dbs, &dopts));
            }
        }
    }

    if let Some(max_hits) = opts.max_hits {
        out.sort_by(|a, b| best_score(b).total_cmp(&best_score(a)));
        out.truncate(max_hits);
    }
    Ok(out)
}
