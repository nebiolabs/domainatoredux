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
    /// Fail if the projected TSV output exceeds this many GB (`0`/`None` disables).
    pub max_output_gb: Option<f64>,
}

impl Default for HmmerCompareOptions {
    fn default() -> Self {
        HmmerCompareOptions { score_cutoff: 0.0, alignments: false, k: None, cpu: 0, max_output_gb: None }
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

    // Both sides must share an alphabet: hmmer_compare aligns profiles column by column,
    // and nucleotide scores run at roughly half the amino acid scale.
    {
        use crate::search::hmm_compare::{check_alphabets_match, common_hmm_alphabet};
        let to_err = |m: String| io::Error::new(io::ErrorKind::InvalidInput, m);
        let q = common_hmm_alphabet(&queries, "input").map_err(to_err)?;
        let r = common_hmm_alphabet(&references, "reference").map_err(to_err)?;
        check_alphabets_match(q, r, &query_files.join(", "), "hmmer_compare").map_err(to_err)?;
    }

    // Output-size guardrail (uncompressed TSV bytes; default via caller, 0/None disables).
    let max_bytes: Option<u64> = match opts.max_output_gb {
        Some(gb) if gb < 0.0 => {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "--max_output_gb must be >= 0."));
        }
        Some(gb) if gb > 0.0 => Some((gb * 1_000_000_000.0) as u64),
        _ => None,
    };
    let over_limit = |projected: u64, max: u64| -> io::Error {
        io::Error::other(format!(
                "Projected hmmer_compare TSV output size is {:.2} GB, which exceeds the configured \
                 limit of {:.2} GB. Reduce the projected output size or raise the limit with \
                 --max_output_gb. Relevant options: -k, --score_cutoff, --alignments.",
                projected as f64 / 1_000_000_000.0,
                max as f64 / 1_000_000_000.0
            ),
        )
    };

    let header = "query\treference\tscore\n";
    let mut written: u64 = header.len() as u64;
    if let Some(max) = max_bytes {
        if written > max {
            return Err(over_limit(written, max));
        }
    }
    out.write_all(header.as_bytes())?;
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
            let mut rendered = format!("{}\t{}\t{:.2}\n", hit.query, hit.reference, hit.score);
            if let Some(text) = &hit.alignment {
                rendered.push_str(text);
                rendered.push_str("\n\n\n\n");
            }
            if let Some(max) = max_bytes {
                written += rendered.len() as u64;
                if written > max {
                    return Err(over_limit(written, max));
                }
            }
            out.write_all(rendered.as_bytes())?;
        }
    }
    Ok(())
}
