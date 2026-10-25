//! `hmmer_compare`: align HMM profiles against reference HMM profiles and report
//! scores (and optionally the alignments), like a slow hhsearch for HMMER3
//! profiles. Mirrors `domainator.hmmer_compare`. Unlike `seq_dist`, the output is
//! a hit *table* (a profile may appear many times), not a square matrix.

use std::cmp::Ordering;
use std::io::{self, Write};

use hmmer_pure_rs::{Alphabet, Bg, Hmm};

use crate::search::hmm_compare::{compare_hmmer, compare_hmmer_aligned};

/// Options for [`hmmer_compare`].
pub struct HmmerCompareOptions {
    /// Report alignments with score `>= min_score` (the unrounded score).
    pub min_score: f64,
    /// Keep at most this many top hits per query (`None` = all).
    pub max_hits_per_query: Option<usize>,
    /// Worker threads (`0` = all available cores).
    pub cpu: usize,
    /// Fail once the outputs would exceed this many bytes (`None` disables).
    pub max_output_bytes: Option<u64>,
}

impl Default for HmmerCompareOptions {
    fn default() -> Self {
        HmmerCompareOptions {
            min_score: 0.0,
            max_hits_per_query: None,
            cpu: 0,
            max_output_bytes: None,
        }
    }
}

/// Round to 2 decimals the way Python's `round(x, 2)` does: ties to **even**,
/// not away from zero (Rust's `f32::round`/`f64::round`).
fn round2(x: f64) -> f64 {
    (x * 100.0).round_ties_even() / 100.0
}

/// One scored (and optionally aligned) hit.
struct Hit {
    query: String,
    reference: String,
    score: f64,
    alignment: Option<String>,
}

/// Compare every query profile against every reference profile (upstream
/// `hmmer_compare.hmmer_compare`). Each hit is a `query reference score` row in
/// `tsv_out` (after a header), and the same row followed by its alignment and a blank
/// line in `alignments_out`; either can be `None`.
///
/// `queries` holds each query file's name and profiles, in order; a file's profiles
/// must share the references' alphabet.
pub fn hmmer_compare(
    queries: &[(String, Vec<Hmm>)],
    references: &[Hmm],
    opts: &HmmerCompareOptions,
    mut tsv_out: Option<&mut dyn Write>,
    mut alignments_out: Option<&mut dyn Write>,
) -> io::Result<()> {
    use crate::search::hmm_compare::{check_alphabets_match, common_hmm_alphabet};
    let to_err = |m: String| io::Error::new(io::ErrorKind::InvalidInput, m);
    let reference_alphabet = common_hmm_alphabet(references, "reference").map_err(to_err)?;

    let mut written: u64 = 0;
    let mut account = |bytes: u64| -> io::Result<()> {
        written += bytes;
        match opts.max_output_bytes {
            Some(max) if written > max => Err(io::Error::other(
                crate::tools::output_guardrails::output_limit_message(
                    "hmmer_compare output",
                    written,
                    max,
                    &["--max_hits_per_query", "--min_score", "--alignments_out"],
                    None,
                ),
            )),
            _ => Ok(()),
        }
    };
    let header = "query\treference\tscore\n";
    if let Some(out) = tsv_out.as_mut() {
        account(header.len() as u64)?;
        out.write_all(header.as_bytes())?;
    }
    let aligned = alignments_out.is_some();

    for (file, profiles) in queries {
        let Some(alphabet) = common_hmm_alphabet(profiles, "input").map_err(to_err)? else {
            continue; // no profiles in this file
        };
        check_alphabets_match(Some(alphabet), reference_alphabet, file, "hmmer_compare")
            .map_err(to_err)?;
        let bg = Bg::new(&Alphabet::new(profiles[0].abc_type));
        // Score each query against all references in parallel; `map` preserves query
        // order so output is identical regardless of thread count.
        let per_query: Vec<Vec<Hit>> = crate::parallel::with_threads(opts.cpu, || {
            use rayon::prelude::*;
            profiles
                .par_iter()
                .map(|q| query_hits(q, references, &bg, aligned, opts))
                .collect()
        });
        for hit in per_query.into_iter().flatten() {
            let row = format!(
                "{}\t{}\t{:.2}\n",
                hit.query,
                hit.reference,
                round2(hit.score)
            );
            let alignment = hit.alignment.map(|text| format!("{row}{text}\n\n"));
            let tsv_bytes = if tsv_out.is_some() { row.len() } else { 0 };
            account((tsv_bytes + alignment.as_ref().map_or(0, String::len)) as u64)?;
            if let Some(out) = tsv_out.as_mut() {
                out.write_all(row.as_bytes())?;
            }
            if let (Some(out), Some(alignment)) = (alignments_out.as_mut(), alignment) {
                out.write_all(alignment.as_bytes())?;
            }
        }
    }
    Ok(())
}

/// One query's hits at or above the cutoff, best first (ties by reference name), at
/// most `max_hits_per_query` of them.
fn query_hits(
    query: &Hmm,
    references: &[Hmm],
    bg: &Bg,
    aligned: bool,
    opts: &HmmerCompareOptions,
) -> Vec<Hit> {
    let mut hits: Vec<Hit> = Vec::new();
    for r in references {
        let (score, alignment) = if aligned {
            let a = compare_hmmer_aligned(query, r, &bg.f);
            (f64::from(a.score), Some(a.text))
        } else {
            (f64::from(compare_hmmer(query, r, &bg.f)), None)
        };
        if score >= opts.min_score {
            hits.push(Hit {
                query: query.name.clone(),
                reference: r.name.clone(),
                score,
                alignment,
            });
        }
    }
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.reference.cmp(&b.reference))
    });
    if let Some(k) = opts.max_hits_per_query {
        hits.truncate(k);
    }
    hits
}
