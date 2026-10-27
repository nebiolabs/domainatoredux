//! `hmmer_search`: filter a query HMM file to the profiles whose best score
//! against any reference profile passes `--min_score`. Port of
//! `domainator.hmmer_search` — same HMM-HMM Viterbi engine as `hmmer_compare`
//! (`crate::search::hmm_compare::compare_hmmer`), but a filtering interface: each
//! kept query profile is written out at most once, ordered by input order (or by
//! best score when `--max_hits` is set). Output is an HMM file.

use std::io::Write;

use crate::search::hmm_write::write_hmm;
use hmmer_pure_rs::alphabet::AlphabetType;
use hmmer_pure_rs::{Alphabet, Bg, Hmm};
use rayon::prelude::*;

use crate::search::hmm_compare::compare_hmmer;

/// Options for [`hmmer_search`].
pub struct HmmerSearchOptions {
    /// Keep a query if its best reference score is `>=` this; `None` is the default
    /// for the references' alphabet ([`default_min_score`]).
    pub min_score: Option<f64>,
    /// Keep at most this many query profiles, ranked by best score (`None` = all).
    pub max_hits: Option<usize>,
    /// Worker threads (`0` = all cores).
    pub cpu: usize,
    /// Fail once the output would exceed this many bytes (`None` disables).
    pub max_output_bytes: Option<u64>,
    /// Names the output in the size-limit message.
    pub output_description: String,
}

/// Filter query HMMs by best score against the references, writing kept profiles
/// to `out`. `queries` and `references` hold each file's name and profiles, in order;
/// the reference files must share one alphabet, and each query file must use it too.
/// Returns how many profiles were written.
pub fn hmmer_search<W: Write>(
    queries: &[(String, Vec<Hmm>)],
    references: &[(String, Vec<Hmm>)],
    opts: &HmmerSearchOptions,
    out: &mut W,
) -> Result<usize, String> {
    use crate::search::hmm_compare::{
        check_alphabets_match, common_file_alphabet, common_hmm_alphabet,
    };
    let reference_alphabet = common_file_alphabet(references, "reference")?;
    let references: Vec<&Hmm> = references.iter().flat_map(|(_, h)| h).collect();
    let min_score = opts
        .min_score
        .unwrap_or_else(|| default_min_score(reference_alphabet).0);

    // Best reference score per query, file by file (each file's alphabet checked
    // first, so a mismatch is one clear line); a query is kept iff its best score
    // passes the cutoff.
    let mut kept: Vec<(&Hmm, f64)> = Vec::new();
    for (file, profiles) in queries {
        let Some(alphabet) = common_hmm_alphabet(profiles, "input")? else {
            continue; // no profiles in this file
        };
        check_alphabets_match(Some(alphabet), reference_alphabet, file, "hmmer_search")?;
        let bg = Bg::new(&Alphabet::new(alphabet));
        let best: Vec<Option<f64>> = crate::parallel::with_threads(opts.cpu, || {
            profiles
                .par_iter()
                .map(|q| {
                    references
                        .iter()
                        .map(|r| f64::from(compare_hmmer(q, r, &bg.f)))
                        .filter(|&score| score >= min_score)
                        .reduce(f64::max)
                })
                .collect()
        });
        kept.extend(
            profiles
                .iter()
                .zip(best)
                .filter_map(|(q, b)| b.map(|s| (q, s))),
        );
    }
    if let Some(n) = opts.max_hits {
        // Rank by best score descending; ties keep input order.
        kept.sort_by(|a, b| b.1.total_cmp(&a.1));
        kept.truncate(n);
    }

    let mut written: u64 = 0;
    let mut buf: Vec<u8> = Vec::new();
    for (profile, _) in &kept {
        buf.clear();
        write_hmm(&mut buf, profile).map_err(|e| format!("{e:?}"))?;
        written += buf.len() as u64;
        if let Some(max) = opts.max_output_bytes.filter(|&max| written > max) {
            return Err(crate::tools::output_guardrails::output_limit_message(
                &opts.output_description,
                written,
                max,
                &["--max_hits", "--min_score"],
                None,
            ));
        }
        out.write_all(&buf).map_err(|e| e.to_string())?;
    }
    Ok(kept.len())
}

/// The default `--min_score` for references of `alphabet`, and the alphabet's name
/// (amino when there are no references).
///
/// Profile scores are log-odds sums whose per-column ceiling is `log(K)`, so a
/// protein-tuned threshold is roughly twice as strict on nucleotides — where it
/// silently returns an empty result indistinguishable from "no homologs found". Only
/// the default is scaled; an explicit `--min_score` is taken literally.
pub fn default_min_score(alphabet: Option<AlphabetType>) -> (f64, &'static str) {
    use crate::search::hmm_compare::{alphabet_name, default_score_cutoff};
    let alphabet = alphabet.unwrap_or(AlphabetType::Amino);
    (default_score_cutoff(alphabet), alphabet_name(alphabet))
}
