//! `structure_domainate` (upstream `structure_domainate.py`): annotate every input
//! structure chain with the reference structures that hit it, as `Domainator`
//! features. Records without hits are kept.
//!
//! References are the query and the input database the target, so `--evalue` is
//! computed against the database being annotated; annotation coordinates are positions
//! on the input record, and the matching reference positions go in `/rstart`, `/rend`
//! and `/rlen`.

use std::collections::HashMap;
use std::io;

use super::domainate::add_protein_annotations;
use super::structure_lib::{
    build_protein_record, hit_to_search_result, prepare_databases, warn_on_saturation, Aligner,
    SearchRequest,
};
use crate::search::results::SearchResult;
use crate::seq::SeqRecord;

/// `structure_domainate`'s options, named like its command line.
#[derive(Debug, Clone)]
pub struct StructureDomainateOptions {
    pub evalue: f64,
    pub min_evalue: f64,
    /// Annotations per record; `None` for all.
    pub max_hits_per_cds: Option<usize>,
    pub max_overlap: f64,
    pub overlap_by_db: bool,
    pub alignment_type: Option<i64>,
    pub metrics: Vec<String>,
    pub max_seqs: Option<usize>,
    /// Keep the database built from the input structures at this prefix.
    pub db_out: Option<String>,
    /// The `/database` value; the reference set's name by default.
    pub database_name: Option<String>,
    pub hits_tsv_out: Option<String>,
}

impl Default for StructureDomainateOptions {
    fn default() -> Self {
        StructureDomainateOptions {
            evalue: 0.001,
            min_evalue: 0.0,
            max_hits_per_cds: None,
            max_overlap: 1.0,
            overlap_by_db: false,
            alignment_type: None,
            metrics: Vec::new(),
            max_seqs: None,
            db_out: None,
            database_name: None,
            hits_tsv_out: None,
        }
    }
}

/// One annotated protein record per input structure chain, in database order.
pub fn structure_domainate(
    inputs: &[String],
    references: &[String],
    aligner: &Aligner,
    opts: &StructureDomainateOptions,
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
            want_tseq: false,
            alignment_type: opts.alignment_type,
            metrics: &opts.metrics,
            sort_by_target: false,
        },
        prepared.work_dir.path(),
    )?;
    // Every input record is walked anyway, so hits are held keyed by input record,
    // bounded by references x --max_seqs.
    let mut hit_counts: HashMap<String, usize> = HashMap::new();
    let mut by_target: HashMap<String, Vec<SearchResult>> = HashMap::new();
    for hit in hits {
        let hit = hit?;
        *hit_counts.entry(hit.query.clone()).or_default() += 1;
        if let Some(result) =
            hit_to_search_result(&hit, &db_name, opts.evalue, opts.min_evalue, aligner.name())
        {
            by_target
                .entry(hit.target.clone())
                .or_default()
                .push(result);
        }
    }
    warn_on_saturation(
        &hit_counts,
        aligner.effective_max_seqs(opts.max_seqs),
        "reference",
    );
    if let Some(path) = &opts.hits_tsv_out {
        aligner.copy_hits_tsv(prepared.work_dir.path(), path)?;
    }

    let mut records = Vec::new();
    for (name, sequence, source) in aligner.iter_sequences(&prepared.input_db)? {
        let mut record = build_protein_record(&name, &sequence, source.as_deref(), None);
        if let Some(hits) = by_target.get(&name).filter(|h| !h.is_empty()) {
            add_protein_annotations(
                &mut record,
                hits.clone(),
                opts.max_hits_per_cds.unwrap_or(usize::MAX),
                opts.max_overlap,
                false,
                false,
                opts.overlap_by_db,
            );
        }
        records.push(record);
    }
    Ok(records)
}
