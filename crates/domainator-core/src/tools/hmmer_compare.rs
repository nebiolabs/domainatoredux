//! `hmmer_compare`: align HMM profiles against reference HMM profiles and report
//! scores (and optionally the alignments), like a slow hhsearch for HMMER3
//! profiles. Mirrors `domainator.hmmer_compare`. Unlike `seq_dist`, the output is
//! a hit *table* (a profile may appear many times), not a square matrix.

use std::cmp::Ordering;
use std::io::{self, Write};
use std::path::Path;

use hmmer_pure_rs::{Alphabet, Bg, Hmm};

use crate::search::hmm_compare::{compare_hmmer, compare_hmmer_aligned};

/// Options for [`hmmer_compare`].
pub struct HmmerCompareOptions {
    /// Report alignments with score `>= score_cutoff`.
    pub score_cutoff: f64,
    /// Emit the rendered alignment block under each hit.
    pub alignments: bool,
    /// Keep at most this many top hits per query (`None` = all).
    pub k: Option<usize>,
    /// Worker threads (`0` = all available cores).
    pub cpu: usize,
}

impl Default for HmmerCompareOptions {
    fn default() -> Self {
        HmmerCompareOptions { score_cutoff: 0.0, alignments: false, k: None, cpu: 0 }
    }
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// Read and concatenate all HMM profiles from `files`, in order.
fn load_all_hmms(files: &[String]) -> io::Result<Vec<Hmm>> {
    let mut out = Vec::new();
    for f in files {
        let hmms = hmmer_pure_rs::hmmfile::read_hmm_file(Path::new(f))
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{e:?}")))?;
        out.extend(hmms);
    }
    Ok(out)
}

/// One scored (and optionally aligned) hit.
struct Hit {
    query: String,
    reference: String,
    score: f64,
    alignment: Option<String>,
}

/// Compare every query profile against every reference profile, writing a TSV
/// (`query`, `reference`, `score`) with optional alignment blocks to `out`.
pub fn hmmer_compare<W: Write>(
    query_files: &[String],
    reference_files: &[String],
    opts: &HmmerCompareOptions,
    out: &mut W,
) -> io::Result<()> {
    let queries = load_all_hmms(query_files)?;
    let references = load_all_hmms(reference_files)?;

    writeln!(out, "query\treference\tscore")?;
    if queries.is_empty() || references.is_empty() {
        return Ok(());
    }
    let bg = Bg::new(&Alphabet::new(queries[0].abc_type));

    // Score each query against all references in parallel; `map` preserves query
    // order so output is identical regardless of thread count.
    let per_query: Vec<Vec<Hit>> = crate::parallel::with_threads(opts.cpu, || {
        use rayon::prelude::*;
        queries
            .par_iter()
            .map(|q| {
                let mut hits: Vec<Hit> = Vec::new();
                for r in &references {
                    let (score, alignment) = if opts.alignments {
                        let a = compare_hmmer_aligned(q, r, &bg.f);
                        (round2(a.score as f64), Some(a.text))
                    } else {
                        (round2(compare_hmmer(q, r, &bg.f) as f64), None)
                    };
                    if score >= opts.score_cutoff {
                        hits.push(Hit {
                            query: q.name.clone(),
                            reference: r.name.clone(),
                            score,
                            alignment,
                        });
                    }
                }
                // Sort by descending score (ties broken by reference name).
                hits.sort_by(|a, b| {
                    b.score
                        .partial_cmp(&a.score)
                        .unwrap_or(Ordering::Equal)
                        .then_with(|| a.reference.cmp(&b.reference))
                });
                if let Some(k) = opts.k {
                    hits.truncate(k);
                }
                hits
            })
            .collect()
    });

    for hits in per_query {
        for hit in hits {
            writeln!(out, "{}\t{}\t{:.2}", hit.query, hit.reference, hit.score)?;
            if let Some(text) = hit.alignment {
                writeln!(out, "{text}\n\n\n")?;
            }
        }
    }
    Ok(())
}
