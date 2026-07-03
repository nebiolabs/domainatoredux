//! `hmmer_search`: filter a query HMM file to the profiles whose best score
//! against any reference profile passes `--score_cutoff`. Port of
//! `domainator.hmmer_search` — same HMM-HMM Viterbi engine as `hmmer_compare`
//! (`crate::search::hmm_compare::compare_hmmer`), but a filtering interface: each
//! kept query profile is written out at most once, ordered by input order (or by
//! best score when `--max_hits` is set). Output is an HMM file.

use std::io::Write;
use std::path::Path;

use hmmer_pure_rs::hmmfile::{read_hmm_file, write_hmm};
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
