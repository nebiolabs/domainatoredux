//! `structure_search` (upstream `structure_search.py`): select the records of a
//! structure database that a reference hits, each with a `Domain_Search` best-hit
//! feature (and, with `--add_annotations`, `Domainator` features for every retained
//! hit).
//!
//! Records carry the target's full observed sequence, not a crop to the alignment.
//! foldseek reports that sequence with each hit, so the searched database is never read
//! end to end; reseek does not, so its selected records' sequences come from one pass
//! over the database afterwards. Hits arrive sorted by target, one record's at a time.

use std::collections::{HashMap, HashSet};
use std::io;

use super::domainate::add_protein_annotations;
use super::structure_lib::{
    build_protein_record, hit_to_search_result, prepare_databases, warn_on_saturation, Aligner,
    Hits, SearchRequest,
};
use crate::search::results::SearchResult;
use crate::seq::SeqRecord;

/// `structure_search`'s options, named like its command line.
#[derive(Debug, Clone)]
pub struct StructureSearchOptions {
    pub evalue: f64,
    pub min_evalue: f64,
    /// Return at most this many records, the best-scoring ones, best first.
    pub max_hits: Option<usize>,
    pub max_overlap: f64,
    pub overlap_by_db: bool,
    pub add_annotations: bool,
    pub alignment_type: Option<i64>,
    pub metrics: Vec<String>,
    pub max_seqs: Option<usize>,
    pub db_out: Option<String>,
    pub database_name: Option<String>,
    pub hits_tsv_out: Option<String>,
}

impl Default for StructureSearchOptions {
    fn default() -> Self {
        StructureSearchOptions {
            evalue: 0.001,
            min_evalue: 0.0,
            max_hits: None,
            max_overlap: 1.0,
            overlap_by_db: false,
            add_annotations: false,
            alignment_type: None,
            metrics: Vec::new(),
            max_seqs: None,
            db_out: None,
            database_name: None,
            hits_tsv_out: None,
        }
    }
}

/// One target's retained hits and the sequence its hits reported (upstream
/// `group_hits_by_target`).
struct Group {
    target: String,
    hits: Vec<SearchResult>,
    sequence: String,
}

/// Group target-sorted hits, dropping hits outside the E-value bounds and targets left
/// with none. `counts` tallies every hit per query, for the saturation warning.
fn group_hits_by_target(
    hits: Hits,
    db_name: &str,
    opts: &StructureSearchOptions,
    program: &str,
    counts: &mut HashMap<String, usize>,
) -> io::Result<Vec<Group>> {
    let mut groups: Vec<Group> = Vec::new();
    let mut current: Option<Group> = None;
    for hit in hits {
        let hit = hit?;
        *counts.entry(hit.query.clone()).or_default() += 1;
        let Some(result) =
            hit_to_search_result(&hit, db_name, opts.evalue, opts.min_evalue, program)
        else {
            continue;
        };
        if current.as_ref().is_none_or(|g| g.target != hit.target) {
            groups.extend(current.take().filter(|g| !g.hits.is_empty()));
            current = Some(Group {
                target: hit.target.clone(),
                hits: Vec::new(),
                sequence: String::new(),
            });
        }
        if let Some(group) = current.as_mut() {
            group.hits.push(result);
            if group.sequence.is_empty() && !hit.tseq.is_empty() {
                group.sequence = hit.tseq;
            }
        }
    }
    groups.extend(current.filter(|g| !g.hits.is_empty()));
    Ok(groups)
}

fn build_record(group: Group, opts: &StructureSearchOptions) -> io::Result<SeqRecord> {
    if group.sequence.is_empty() {
        return Err(io::Error::other(format!(
            "The backend reported a hit to '{}' without its sequence, so the output record \
             cannot be built. This usually means the target database is missing entries it \
             claims to contain.",
            group.target
        )));
    }
    let mut record = build_protein_record(&group.target, &group.sequence, None, None);
    add_protein_annotations(
        &mut record,
        group.hits,
        usize::MAX,
        opts.max_overlap,
        !opts.add_annotations,
        true,
        opts.overlap_by_db,
    );
    Ok(record)
}

fn best_score(group: &Group) -> f64 {
    group
        .hits
        .iter()
        .map(|h| h.score)
        .fold(f64::NEG_INFINITY, f64::max)
}

/// With `max_hits`, the best `max_hits` groups by best hit score, best first, ties in
/// arrival order (upstream's `(score, -counter)` heap).
fn select_best(mut groups: Vec<Group>, max_hits: Option<usize>) -> Vec<Group> {
    if let Some(n) = max_hits {
        groups.sort_by(|a, b| best_score(b).total_cmp(&best_score(a)));
        groups.truncate(n);
    }
    groups
}

/// The database records that matched a reference: in target order, or best first with
/// `--max_hits`.
pub fn structure_search(
    inputs: &[String],
    references: &[String],
    aligner: &Aligner,
    opts: &StructureSearchOptions,
) -> io::Result<Vec<SeqRecord>> {
    let prepared = prepare_databases(aligner, inputs, Some(references), opts.db_out.as_deref())?;
    aligner.check_capabilities(
        opts.alignment_type,
        &opts.metrics,
        &[&prepared.input_db, &prepared.reference_db],
    )?;
    let db_name = opts
        .database_name
        .clone()
        .unwrap_or_else(|| prepared.reference_label.clone());
    let hits = aligner.search(
        &prepared.reference_db,
        &prepared.input_db,
        &SearchRequest {
            evalue: opts.evalue,
            max_seqs: opts.max_seqs,
            want_tseq: true,
            alignment_type: opts.alignment_type,
            metrics: &opts.metrics,
            sort_by_target: true,
        },
        prepared.work_dir.path(),
    )?;
    let mut counts = HashMap::new();
    let groups = group_hits_by_target(hits, &db_name, opts, aligner.name(), &mut counts)?;
    let selected = select_best(groups, opts.max_hits);

    let records = if aligner.provides_target_sequence() {
        selected
            .into_iter()
            .map(|g| build_record(g, opts))
            .collect::<io::Result<Vec<_>>>()?
    } else {
        with_database_sequences(selected, aligner, &prepared.input_db, opts)?
    };
    warn_on_saturation(
        &counts,
        aligner.effective_max_seqs(opts.max_seqs),
        "reference",
    );
    if let Some(path) = &opts.hits_tsv_out {
        aligner.copy_hits_tsv(prepared.work_dir.path(), path)?;
    }
    Ok(records)
}

/// Records for a backend whose hits carry no target sequences (reseek): the sequences
/// come from one pass over the database. Without `--max_hits` records come out in
/// database order, as upstream's do.
fn with_database_sequences(
    selected: Vec<Group>,
    aligner: &Aligner,
    db: &super::structure_lib::StructureDb,
    opts: &StructureSearchOptions,
) -> io::Result<Vec<SeqRecord>> {
    if selected.is_empty() {
        return Ok(Vec::new());
    }
    let wanted: HashSet<&str> = selected.iter().map(|g| g.target.as_str()).collect();
    let mut found: Vec<(String, String)> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for (name, sequence, _) in aligner.iter_sequences(db)? {
        if wanted.contains(name.as_str()) {
            if seen.insert(name.clone()) {
                found.push((name, sequence));
            } else if let Some(slot) = found.iter_mut().find(|(n, _)| *n == name) {
                slot.1 = sequence;
            }
            if seen.len() == wanted.len() {
                break;
            }
        }
    }
    let mut missing: Vec<&str> = wanted
        .iter()
        .copied()
        .filter(|t| !seen.contains(*t))
        .collect();
    if !missing.is_empty() {
        missing.sort_unstable();
        let examples: Vec<String> = missing.iter().take(3).map(|t| format!("'{t}'")).collect();
        return Err(io::Error::other(format!(
            "{} hit target(s) are absent from the searched database (for example [{}]), so \
             their records cannot be built.",
            missing.len(),
            examples.join(", ")
        )));
    }
    let order: Vec<String> = if opts.max_hits.is_some() {
        // Best first, the order select_best left them in.
        selected.iter().map(|g| g.target.clone()).collect()
    } else {
        found.iter().map(|(n, _)| n.clone()).collect()
    };
    let mut sequences: HashMap<String, String> = found.into_iter().collect();
    let mut groups: HashMap<String, Group> = selected
        .into_iter()
        .map(|g| (g.target.clone(), g))
        .collect();
    let mut records = Vec::new();
    for target in order {
        if let Some(mut group) = groups.remove(&target) {
            group.sequence = sequences.remove(&target).unwrap_or_default();
            records.push(build_record(group, opts)?);
        }
    }
    Ok(records)
}
