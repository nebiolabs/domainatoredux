//! `hmmer_search`: filter a query HMM file to the profiles whose best score
//! against any reference profile passes `--score_cutoff`. Port of
//! `domainator.hmmer_search` — same HMM-HMM Viterbi engine as `hmmer_compare`
//! (`crate::search::hmm_compare::compare_hmmer`), but a filtering interface: each
//! kept query profile is written out at most once, ordered by input order (or by
//! best score when `--max_hits` is set). Output is an HMM file.

use std::io::Write;
use std::path::Path;

use crate::search::hmm_write::write_hmm;
use crate::search::hmm_read::read_hmm_file;
use hmmer_pure_rs::{Alphabet, Bg, Hmm};
use rayon::prelude::*;

use crate::search::hmm_compare::compare_hmmer;

/// Options for [`hmmer_search`].
pub struct HmmerSearchOptions {
    /// Keep a query if its best reference score is `>=` this (Python default 10.0).
    pub score_cutoff: f64,
    /// Keep at most this many query profiles, ranked by best score (`None` = all).
    pub max_hits: Option<usize>,
    /// Worker threads (`0` = all cores).
    pub cpu: usize,
    /// Fail if the projected HMM output exceeds this many GB (`0`/`None` disables).
    pub max_output_gb: Option<f64>,
}

fn read_all(files: &[String]) -> Result<Vec<Hmm>, String> {
    let mut out = Vec::new();
    for f in files {
        let hmms = read_hmm_file(Path::new(f)).map_err(|e| format!("reading {f}: {e:?}"))?;
        out.extend(hmms);
    }
    Ok(out)
}

/// Filter query HMMs by best score against the references, writing kept profiles
/// to `out`. Returns the number written.
pub fn hmmer_search<W: Write>(
    input_files: &[String],
    reference_files: &[String],
    opts: &HmmerSearchOptions,
    out: &mut W,
) -> Result<usize, String> {
    let queries = read_all(input_files)?;
    let references = read_all(reference_files)?;
    if references.is_empty() {
        return Err("no reference profiles found".into());
    }
    // Both sides must share an alphabet: hmmer_search aligns profiles column by column.
    // Checked here, before the worker pool, so a mismatch is one clear line.
    {
        use crate::search::hmm_compare::{check_alphabets_match, common_hmm_alphabet};
        let q = common_hmm_alphabet(&queries, "input")?;
        let r = common_hmm_alphabet(&references, "reference")?;
        check_alphabets_match(q, r, &input_files.join(", "), "hmmer_search")?;
    }
    if queries.is_empty() {
        return Ok(0);
    }

    let abc = Alphabet::new(queries[0].abc_type);
    let bg = Bg::new(&abc);

    // Best reference score per query (independent; parallel over queries). A query
    // is kept iff its best score passes the cutoff.
    let best: Vec<Option<f64>> = crate::parallel::with_threads(opts.cpu, || {
        queries
            .par_iter()
            .map(|q| {
                let mut best = f64::NEG_INFINITY;
                for r in &references {
                    let s = compare_hmmer(q, r, &bg.f) as f64;
                    if s > best {
                        best = s;
                    }
                }
                if best >= opts.score_cutoff {
                    Some(best)
                } else {
                    None
                }
            })
            .collect()
    });

    let mut kept: Vec<(usize, f64)> =
        best.iter().enumerate().filter_map(|(i, b)| b.map(|s| (i, s))).collect();
    if let Some(n) = opts.max_hits {
        // Rank by best score descending; ties keep input order.
        kept.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        kept.truncate(n);
    }

    let max_bytes = match opts.max_output_gb {
        Some(gb) if gb < 0.0 => return Err("--max_output_gb must be >= 0.".into()),
        Some(gb) if gb > 0.0 => Some((gb * 1_000_000_000.0) as u64),
        _ => None,
    };
    let mut written: u64 = 0;
    let mut buf: Vec<u8> = Vec::new();
    for (i, _) in &kept {
        buf.clear();
        write_hmm(&mut buf, &queries[*i]).map_err(|e| format!("{e:?}"))?;
        if let Some(max) = max_bytes {
            written += buf.len() as u64;
            if written > max {
                return Err(format!(
                    "Projected hmmer_search HMM output size {:.2} GB exceeds the limit. \
                     Raise --max_output_gb or use --max_hits / --score_cutoff.",
                    written as f64 / 1_000_000_000.0
                ));
            }
        }
        out.write_all(&buf).map_err(|e| e.to_string())?;
    }
    Ok(kept.len())
}

/// Resolve the default `--score_cutoff` from the reference profiles' alphabet.
///
/// Returns `(cutoff, alphabet_name)`. Profile scores are log-odds sums whose per-column
/// ceiling is `log(K)`, so a protein-tuned threshold is roughly twice as strict on
/// nucleotides — where it silently returns an empty result indistinguishable from "no
/// homologs found". HMMER locks an `.hmm` file to the alphabet of its first profile, so
/// one peek is enough.
///
/// Only the default is scaled; an explicitly supplied `--score_cutoff` is taken literally.
pub fn default_score_cutoff_for(reference_files: &[String]) -> Result<(f64, &'static str), String> {
    use crate::search::hmm_compare::{alphabet_name, default_score_cutoff};
    use hmmer_pure_rs::alphabet::AlphabetType;

    let records = crate::tools::hmmer_report::read_hmms_to_iterators(reference_files)?;
    let abc = records
        .iter()
        .flat_map(|(_, hmms)| hmms.iter())
        .next()
        .map(|h| h.abc_type)
        .unwrap_or(AlphabetType::Amino);
    Ok((default_score_cutoff(abc), alphabet_name(abc)))
}

/// Python's `f"{x:g}"`: 6 significant digits, trailing zeros and a bare `.0` stripped,
/// switching to exponent form outside `[1e-4, 1e6)`.
///
/// Used only for the `--score_cutoff` notice, so it stays beside it rather than joining
/// the matrix number formatters.
pub fn py_g(x: f64) -> String {
    if x == 0.0 {
        return "0".to_string();
    }
    let exp = x.abs().log10().floor() as i32;
    if !(-5..6).contains(&exp) {
        let mantissa = format!("{:.5e}", x);
        // Rust writes `4.62756e0`; Python writes `4.62756e+00`.
        let (m, e) = mantissa.split_once('e').unwrap();
        let m = m.trim_end_matches('0').trim_end_matches('.');
        let e: i32 = e.parse().unwrap_or(0);
        return format!("{m}e{}{:02}", if e < 0 { '-' } else { '+' }, e.abs());
    }
    let decimals = (5 - exp).max(0) as usize;
    let text = format!("{x:.decimals$}");
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        text
    }
}

#[cfg(test)]
mod score_cutoff_tests {
    use super::py_g;

    #[test]
    fn py_g_matches_pythons_g_format() {
        // Values checked against Python's f"{x:g}".
        assert_eq!(py_g(10.0), "10");
        assert_eq!(py_g(4.627564518750634), "4.62756");
        assert_eq!(py_g(0.5), "0.5");
        assert_eq!(py_g(0.0), "0");
        assert_eq!(py_g(123456.0), "123456");
        assert_eq!(py_g(1234567.0), "1.23457e+06");
    }
}
