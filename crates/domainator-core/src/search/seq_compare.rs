//! Pairwise similarity scoring for `seq_dist`: `phmmer` (build a single-sequence
//! HMM per reference and search the inputs) and `hmmsearch` (search HMM profiles
//! against the input sequences). Mirrors `seq_dist.run_hmmsearch`: the matrix
//! value is the full-sequence bit score (`hit.score`), rounded to two decimals,
//! for query=input-sequence (row) vs subject=reference (col).

use rayon::prelude::*;

use hmmer_pure_rs::profile::{profile_config, P7_LOCAL};
use hmmer_pure_rs::sequence::Sequence;
use hmmer_pure_rs::tophits::P7_IS_REPORTED;
use hmmer_pure_rs::{Alphabet, Bg, Hmm, OProfile, Pipeline, Profile, TopHits};

/// One (score, query, subject) comparison, like `seq_dist.CmpResult`.
pub struct CmpResult {
    pub score: f64,
    /// Input-sequence name (matrix row).
    pub query: String,
    /// Reference name — sequence id (phmmer) or HMM name (hmmsearch) (matrix col).
    pub subject: String,
}

/// phmmer gap-open / gap-extend probabilities (HMMER defaults).
const POPEN: f32 = 0.02;
const PEXTEND: f32 = 0.4;

/// Round to 2 decimals the way Python's `round(x, 2)` does: ties to **even**,
/// not away from zero (Rust's `f32::round`/`f64::round`).
fn round2(x: f64) -> f64 {
    (x * 100.0).round_ties_even() / 100.0
}

fn make_sequence(name: &str, abc: &Alphabet, residues: &[u8]) -> Sequence {
    Sequence {
        name: name.to_string(),
        acc: String::new(),
        desc: String::new(),
        dsq: abc.digitize(residues),
        n: residues.len(),
        l: residues.len(),
        taxid: -1,
    }
}

/// Run `gm`/`om` (a configured model named `subject`, with background `bg`)
/// against every target, returning reported hits with `score > lb`.
fn search_model(
    subject: &str,
    mut gm: Profile,
    mut om: OProfile,
    bg: &Bg,
    hmm: &Hmm,
    targets: &[Sequence],
    lb: f64,
) -> Vec<CmpResult> {
    let mut out = Vec::new();
    let mut th = TopHits::new();
    let mut pli = Pipeline::new();
    pli.new_model(&gm);
    for sq in targets {
        if sq.n == 0 {
            continue;
        }
        let mut lb_bg = bg.clone();
        lb_bg.set_length(sq.n);
        pli.run(&mut gm, &mut om, &lb_bg, hmm, sq, &mut th);
    }

    let z = targets.len() as f64;
    let thr = Pipeline::new();
    th.threshold(&thr, z, z);

    for hit in &th.hits {
        if hit.flags & P7_IS_REPORTED == 0 {
            continue;
        }
        let score = round2(hit.score as f64);
        if score <= lb {
            continue;
        }
        out.push(CmpResult {
            score,
            query: hit.name.clone(),
            subject: subject.to_string(),
        });
    }
    out
}

/// `--algorithm hmmer` with an HMM reference: search each profile against the
/// input sequences. `input_seqs` are `(name, residues)`.
pub fn hmmsearch_scores(input_seqs: &[(String, Vec<u8>)], hmms: &[Hmm], lb: f64) -> Vec<CmpResult> {
    // Each profile is independent: build its own profile/pipeline and search the
    // shared (read-only) input sequences. Parallelize over profiles.
    hmms.par_iter()
        .flat_map_iter(|hmm| {
            let abc = Alphabet::new(hmm.abc_type);
            let bg = Bg::new(&abc);
            let targets: Vec<Sequence> = input_seqs
                .iter()
                .map(|(n, r)| make_sequence(n, &abc, r))
                .collect();
            let mut gm = Profile::new(hmm.m, &abc);
            profile_config(hmm, &bg, &mut gm, 400, P7_LOCAL);
            let om = OProfile::convert(&gm);
            search_model(&hmm.name, gm, om, &bg, hmm, &targets, lb).into_iter()
        })
        .collect()
}

/// `--algorithm hmmer` with a sequence reference (phmmer): build a single-sequence
/// HMM from each reference and search it against the input sequences.
pub fn phmmer_scores(
    input_seqs: &[(String, Vec<u8>)],
    ref_seqs: &[(String, Vec<u8>)],
    lb: f64,
) -> Vec<CmpResult> {
    let abc = Alphabet::amino();
    let bg = Bg::new(&abc);
    let targets: Vec<Sequence> = input_seqs
        .iter()
        .map(|(n, r)| make_sequence(n, &abc, r))
        .collect();

    // Each reference builds its own single-sequence profile and searches the
    // shared (read-only) targets. Parallelize over references.
    ref_seqs
        .par_iter()
        .filter(|(_, ref_res)| !ref_res.is_empty())
        .flat_map_iter(|(ref_name, ref_res)| {
            let dsq = abc.digitize(ref_res);
            let hmm = hmmer_pure_rs::seqmodel::build_single_seq_hmm(
                ref_name,
                &dsq,
                ref_res.len(),
                &abc,
                &bg,
                POPEN,
                PEXTEND,
            );
            let mut local_bg = bg.clone();
            local_bg.set_filter(hmm.m, &hmm.compo);
            let mut gm = Profile::new(hmm.m, &abc);
            profile_config(&hmm, &local_bg, &mut gm, 400, P7_LOCAL);
            let om = OProfile::convert(&gm);
            search_model(ref_name, gm, om, &local_bg, &hmm, &targets, lb).into_iter()
        })
        .collect()
}
