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

use crate::search::hmm_compare::{
    check_alphabets_match, common_file_alphabet, compare_hmmer, mixed_alphabets_message,
};
use crate::tools::top_n::TopN;

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
    let mut searcher = HmmerSearcher::new(references, opts)?;
    for (file, profiles) in queries {
        searcher.search(file, profiles, out)?;
    }
    searcher.finish(out)
}

/// [`hmmer_search`] a batch of query profiles at a time, so the queries can be
/// streamed: without `--max_hits` each kept profile is written as its batch is scored;
/// with it, at most that many are held (upstream's heap).
pub struct HmmerSearcher<'a> {
    references: Vec<&'a Hmm>,
    reference_alphabet: Option<AlphabetType>,
    min_score: f64,
    opts: &'a HmmerSearchOptions,
    /// With `--max_hits`, the best kept profiles so far.
    best: Option<TopN<Hmm>>,
    kept: usize,
    written: u64,
    /// The query file being read: its name, alphabet, and first profile's name.
    file: Option<(String, AlphabetType, String)>,
}

impl<'a> HmmerSearcher<'a> {
    pub fn new(
        references: &'a [(String, Vec<Hmm>)],
        opts: &'a HmmerSearchOptions,
    ) -> Result<Self, String> {
        let reference_alphabet = common_file_alphabet(references, "reference")?;
        let min_score = opts
            .min_score
            .unwrap_or_else(|| default_min_score(reference_alphabet).0);
        Ok(HmmerSearcher {
            references: references.iter().flat_map(|(_, h)| h).collect(),
            reference_alphabet,
            min_score,
            opts,
            best: opts.max_hits.map(|n| TopN::new(Some(n))),
            kept: 0,
            written: 0,
            file: None,
        })
    }

    /// Score the next `profiles` of the query file `file`: a query is kept iff its best
    /// reference score passes the cutoff. Each file's alphabet is checked first, so a
    /// mismatch is one clear line.
    pub fn search<W: Write>(
        &mut self,
        file: &str,
        profiles: &[Hmm],
        out: &mut W,
    ) -> Result<(), String> {
        let Some(first) = profiles.first() else {
            return Ok(()); // no profiles in this file
        };
        if self.file.as_ref().is_none_or(|(name, ..)| name != file) {
            self.file = Some((file.to_string(), first.abc_type, first.name.clone()));
        }
        let (_, alphabet, first_name) = self.file.as_ref().expect("set above");
        let alphabet = *alphabet;
        if let Some(mixed) = profiles.iter().find(|h| h.abc_type != alphabet) {
            return Err(mixed_alphabets_message(
                "input", first_name, alphabet, mixed,
            ));
        }
        check_alphabets_match(
            Some(alphabet),
            self.reference_alphabet,
            file,
            "hmmer_search",
        )?;
        let bg = Bg::new(&Alphabet::new(alphabet));
        let (references, min_score) = (&self.references, self.min_score);
        let best: Vec<Option<f64>> = crate::parallel::with_threads(self.opts.cpu, || {
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
        for (profile, best) in profiles.iter().zip(best) {
            let Some(score) = best else {
                continue;
            };
            match self.best.as_mut() {
                // Ranked by best score descending; ties keep input order.
                Some(top) => top.push(score, profile.clone()),
                None => self.write(profile, out)?,
            }
        }
        Ok(())
    }

    fn write<W: Write>(&mut self, profile: &Hmm, out: &mut W) -> Result<(), String> {
        let mut buf: Vec<u8> = Vec::new();
        write_hmm(&mut buf, profile).map_err(|e| format!("{e:?}"))?;
        self.written += buf.len() as u64;
        if let Some(max) = self.opts.max_output_bytes.filter(|&max| self.written > max) {
            return Err(crate::tools::output_guardrails::output_limit_message(
                &self.opts.output_description,
                self.written,
                max,
                &["--max_hits", "--min_score"],
                None,
            ));
        }
        out.write_all(&buf).map_err(|e| e.to_string())?;
        self.kept += 1;
        Ok(())
    }

    /// Write what `--max_hits` held; how many profiles were written in all.
    pub fn finish<W: Write>(mut self, out: &mut W) -> Result<usize, String> {
        if let Some(top) = self.best.take() {
            for profile in top.into_sorted_vec() {
                self.write(&profile, out)?;
            }
        }
        Ok(self.kept)
    }
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
