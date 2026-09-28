//! HMM-vs-HMM comparison (`seq_dist --algorithm hmmer_compare`): a 5-state
//! Viterbi profile-profile local alignment (Söding / HH-suite style), ported from
//! `domainator.hmmer_search.compare_hmmer`. The score is a log-odds co-emission
//! sum maximized over a local alignment of the two profiles.
//!
//! hmmer-pure-rs exposes match-emission probabilities (`Hmm.mat`), transition
//! probabilities (`Hmm.t`, indices `MM..DD` matching the Python layout), and the
//! Swiss-Prot background (`Bg::new(&abc).f`) — so this is a direct, dependency-free
//! port. Alignment traceback output is deferred (only the score is needed for the
//! distance matrix).

use rayon::prelude::*;

use hmmer_pure_rs::hmm::{DD, DM, II, IM, MD, MI, MM};
use hmmer_pure_rs::{Alphabet, Bg, Hmm};

use super::seq_compare::CmpResult;

// DP state indices (S{query}{reference}); MM=match/match, etc.
const SMM: usize = 0;
const SMI: usize = 1;
const SIM: usize = 2;
const SDG: usize = 3;
const SGD: usize = 4;

/// Round to 2 decimals the way Python's `round(x, 2)` does — ties to **even**,
/// not away from zero (which is what Rust's `round` gives) — and **in f32**.
///
/// The profile-profile DP is float32 on both sides (numba `nb.float32` there,
/// `f32` here), and `round(numpy_float32, 2)` returns a float32. So the score
/// that reaches a matrix is a float32 widened to double: `146.4` is stored and
/// printed as `146.39999389648438`. Rounding in f64 instead yields a clean
/// `146.4`, which is a visible difference in `seq-dist --dense_text`.
fn round2_f32(x: f32) -> f32 {
    (x * 100.0).round_ties_even() / 100.0
}

/// `-ln` of each of the 7 transition probabilities (impossible transitions -> +inf).
fn neg_log7(t: &[f32; 7]) -> [f32; 7] {
    let mut out = [0.0f32; 7];
    for i in 0..7 {
        out[i] = -t[i].ln();
    }
    out
}

/// Co-emission log-odds score for two match-emission distributions:
/// `log( sum_a q_a * r_a / bg_a )`. Mirrors `hmmer_search.Saa`.
fn saa(q: &[f32], r: &[f32], bg: &[f32], k: usize) -> f32 {
    let mut s = 0.0f32;
    for a in 0..k {
        s += q[a] * r[a] / bg[a];
    }
    s.ln()
}

#[inline]
fn max5(a: f32, b: f32, c: f32, d: f32, e: f32) -> f32 {
    a.max(b).max(c).max(d).max(e)
}

/// Viterbi profile-profile local-alignment score between query `q` and
/// reference `r`. `bg` is the background residue frequency array (length `K`).
pub fn compare_hmmer(q: &Hmm, r: &Hmm, bg: &[f32]) -> f32 {
    let nq = q.mat.len(); // M_q + 1 (row 0 unused)
    let nr = r.mat.len(); // M_r + 1
    let k = q.abc_k;
    if nq < 2 || nr < 2 {
        return 0.0;
    }

    let qt: Vec<[f32; 7]> = q.t.iter().map(neg_log7).collect();
    let rt: Vec<[f32; 7]> = r.t.iter().map(neg_log7).collect();

    // scores[qi][ri][state]; boundary row/col stay 0 (local-alignment start).
    let mut scores = vec![vec![[0.0f32; 5]; nr]; nq];
    let mut best = 0.0f32;

    for qi in 1..nq {
        for ri in 1..nr {
            // SMM: best diagonal predecessor (or a fresh local start at 0), plus the
            // co-emission match score.
            let smm_prev = max5(
                scores[qi - 1][ri - 1][SMM] - qt[qi - 1][MM] - rt[ri - 1][MM],
                scores[qi - 1][ri - 1][SMI] - qt[qi - 1][MM] - rt[ri - 1][IM],
                scores[qi - 1][ri - 1][SIM] - qt[qi - 1][IM] - rt[ri - 1][MM],
                scores[qi - 1][ri - 1][SDG] - qt[qi - 1][DM] - rt[ri - 1][MM],
                scores[qi - 1][ri - 1][SGD] - qt[qi - 1][MM] - rt[ri - 1][DM],
            )
            .max(0.0);
            let match_score = saa(&q.mat[qi], &r.mat[ri], bg, k);
            let smm = smm_prev + match_score;
            scores[qi][ri][SMM] = smm;

            // SGD: gap in query, delete in reference.
            let sgd = (scores[qi][ri - 1][SMM] - rt[ri - 1][MD])
                .max(scores[qi][ri - 1][SGD] - rt[ri - 1][DD]);
            scores[qi][ri][SGD] = sgd;

            // SIM: insert in query relative to reference match.
            let sim = (scores[qi][ri - 1][SMM] - qt[qi][MI] - rt[ri - 1][MM])
                .max(scores[qi][ri - 1][SIM] - qt[qi][II] - rt[ri - 1][MM]);
            scores[qi][ri][SIM] = sim;

            // SDG: delete in query, gap in reference.
            let sdg = (scores[qi - 1][ri][SMM] - qt[qi - 1][MD])
                .max(scores[qi - 1][ri][SDG] - qt[qi - 1][DD]);
            scores[qi][ri][SDG] = sdg;

            // SMI: match in query, insert in reference.
            let smi = (scores[qi - 1][ri][SMM] - qt[qi - 1][MM] - rt[ri][MI])
                .max(scores[qi - 1][ri][SMI] - qt[qi - 1][MM] - rt[ri][II]);
            scores[qi][ri][SMI] = smi;

            best = best.max(max5(smm, smi, sim, sdg, sgd));
        }
    }
    best
}

/// Backtrace sentinel for a local-alignment start (mirrors `_STOP_FLAG`).
const STOP: u8 = u8::MAX;

/// A rendered profile-profile alignment (HH-suite style three-line block).
pub struct Alignment {
    pub score: f32,
    pub text: String,
}

/// HH-suite midline symbol for a per-column match log-odds score.
///
/// `scale` rescales the published amino acid cutoffs onto the profile's own alphabet.
/// Column scores are natural-log odds against the alphabet background, and a nucleotide
/// column tops out near log(4) = 1.39 — so without this, `|` would be unreachable on
/// DNA and RNA and the whole midline would collapse into `.` and `+`. The scale is
/// exactly 1.0 for amino acids, so protein output is unchanged.
fn midline_symbol(score: f32, scale: f32) -> char {
    if score < -1.5 * scale {
        '='
    } else if score < -0.5 * scale {
        '-'
    } else if score < 0.5 * scale {
        '.'
    } else if score < 1.5 * scale {
        '+'
    } else {
        '|'
    }
}

/// Viterbi profile-profile alignment that also records the traceback and renders
/// the alignment text. Mirrors `compare_hmmer` + `hmmer_search.traceback`.
pub fn compare_hmmer_aligned(q: &Hmm, r: &Hmm, bg: &[f32]) -> Alignment {
    let nq = q.mat.len();
    let nr = r.mat.len();
    let k = q.abc_k;
    if nq < 2 || nr < 2 {
        return Alignment {
            score: 0.0,
            text: String::new(),
        };
    }

    let qt: Vec<[f32; 7]> = q.t.iter().map(neg_log7).collect();
    let rt: Vec<[f32; 7]> = r.t.iter().map(neg_log7).collect();

    let mut scores = vec![vec![[0.0f32; 5]; nr]; nq];
    let mut bt = vec![vec![[STOP; 5]; nr]; nq];
    let mut msc = vec![vec![0.0f32; nr]; nq];
    let mut best = 0.0f32;
    let mut max_index = (0usize, 0usize, SMM as u8);

    for qi in 1..nq {
        for ri in 1..nr {
            // SMM with backtrace, replicating max6's exact tie-break order
            // (STOP/SMM init, then SIM, SMI, SDG, SGD on strict >).
            let cmm = scores[qi - 1][ri - 1][SMM] - qt[qi - 1][MM] - rt[ri - 1][MM];
            let cmi = scores[qi - 1][ri - 1][SMI] - qt[qi - 1][MM] - rt[ri - 1][IM];
            let cim = scores[qi - 1][ri - 1][SIM] - qt[qi - 1][IM] - rt[ri - 1][MM];
            let cdg = scores[qi - 1][ri - 1][SDG] - qt[qi - 1][DM] - rt[ri - 1][MM];
            let cgd = scores[qi - 1][ri - 1][SGD] - qt[qi - 1][MM] - rt[ri - 1][DM];
            let (mut sc, mut layer): (f32, u8) = if 0.0 > cmm {
                (0.0, STOP)
            } else {
                (cmm, SMM as u8)
            };
            if cim > sc {
                sc = cim;
                layer = SIM as u8;
            }
            if cmi > sc {
                sc = cmi;
                layer = SMI as u8;
            }
            if cdg > sc {
                sc = cdg;
                layer = SDG as u8;
            }
            if cgd > sc {
                sc = cgd;
                layer = SGD as u8;
            }
            let match_score = saa(&q.mat[qi], &r.mat[ri], bg, k);
            msc[qi][ri] = match_score;
            scores[qi][ri][SMM] = sc + match_score;
            bt[qi][ri][SMM] = layer;

            // SGD
            let a = scores[qi][ri - 1][SMM] - rt[ri - 1][MD];
            let b = scores[qi][ri - 1][SGD] - rt[ri - 1][DD];
            scores[qi][ri][SGD] = a.max(b);
            bt[qi][ri][SGD] = if a > b { SMM as u8 } else { SGD as u8 };

            // SIM
            let a = scores[qi][ri - 1][SMM] - qt[qi][MI] - rt[ri - 1][MM];
            let b = scores[qi][ri - 1][SIM] - qt[qi][II] - rt[ri - 1][MM];
            scores[qi][ri][SIM] = a.max(b);
            bt[qi][ri][SIM] = if a > b { SMM as u8 } else { SIM as u8 };

            // SDG
            let a = scores[qi - 1][ri][SMM] - qt[qi - 1][MD];
            let b = scores[qi - 1][ri][SDG] - qt[qi - 1][DD];
            scores[qi][ri][SDG] = a.max(b);
            bt[qi][ri][SDG] = if a > b { SMM as u8 } else { SDG as u8 };

            // SMI
            let a = scores[qi - 1][ri][SMM] - qt[qi - 1][MM] - rt[ri][MI];
            let b = scores[qi - 1][ri][SMI] - qt[qi - 1][MM] - rt[ri][II];
            scores[qi][ri][SMI] = a.max(b);
            bt[qi][ri][SMI] = if a > b { SMM as u8 } else { SMI as u8 };

            // argmax (C-order: state layer 0..4) with strict-> (first occurrence wins).
            for (layer, &score) in scores[qi][ri].iter().enumerate() {
                if score > best {
                    best = score;
                    max_index = (qi, ri, layer as u8);
                }
            }
        }
    }

    let text = render_traceback(q, r, &bt, &msc, max_index);
    Alignment { score: best, text }
}

const POSITION_PADDING: usize = 10;

/// Render the three-line alignment block by walking the backtrace from `start`.
fn render_traceback(
    q: &Hmm,
    r: &Hmm,
    bt: &[Vec<[u8; 5]>],
    msc: &[Vec<f32>],
    start: (usize, usize, u8),
) -> String {
    let qcons = q.consensus.as_deref().unwrap_or(&[]);
    let rcons = r.consensus.as_deref().unwrap_or(&[]);
    let qc = |i: usize| -> char { qcons.get(i).map(|&b| b as char).unwrap_or('?') };
    let rc = |i: usize| -> char { rcons.get(i).map(|&b| b as char).unwrap_or('?') };

    let (qend, rend) = (start.0, start.1);
    let (mut qstart, mut rstart) = (start.0, start.1);
    let mut ptr = start;
    let mut prev = bt[ptr.0][ptr.1][ptr.2 as usize];

    // compare_hmmer has already checked that q and r share an alphabet.
    let midline_scale = alphabet_score_scale(q.abc_type) as f32;

    let mut qline: Vec<char> = Vec::new();
    let mut midline: Vec<char> = Vec::new();
    let mut rline: Vec<char> = Vec::new();

    loop {
        let (qi, ri, layer) = ptr;
        let next: (usize, usize, u8) = match layer as usize {
            SMM => {
                qline.push(qc(qi));
                rline.push(rc(ri));
                qstart = qi;
                rstart = ri;
                midline.push(midline_symbol(msc[qi][ri], midline_scale));
                (qi - 1, ri - 1, prev)
            }
            SMI => {
                qline.push(qc(qi));
                rline.push('-');
                midline.push(' ');
                qstart = qi;
                (qi - 1, ri, prev)
            }
            SIM => {
                qline.push('-');
                rline.push(rc(ri));
                midline.push(' ');
                rstart = ri;
                (qi, ri - 1, prev)
            }
            SDG => {
                qline.push(qc(qi));
                rline.push('-');
                midline.push(' ');
                qstart = qi;
                (qi - 1, ri, prev)
            }
            SGD => {
                qline.push('-');
                rline.push(rc(ri));
                midline.push(' ');
                rstart = ri;
                (qi, ri - 1, prev)
            }
            _ => unreachable!(),
        };
        if prev == STOP {
            break;
        }
        ptr = next;
        prev = bt[ptr.0][ptr.1][ptr.2 as usize];
    }

    let rev: fn(&[char]) -> String = |v| v.iter().rev().collect();
    let pad = POSITION_PADDING;
    format!(
        "{:<pad$}{}{:>pad$}\n{}{}{}\n{:<pad$}{}{:>pad$}",
        qstart.to_string(),
        rev(&qline),
        qend.to_string(),
        " ".repeat(pad),
        rev(&midline),
        " ".repeat(pad),
        rstart.to_string(),
        rev(&rline),
        rend.to_string(),
        pad = pad,
    )
}

/// Score every query HMM against every reference HMM, keeping pairs with
/// `score > lb`. `query == subject` orientation: row = input HMM, col = reference HMM.
pub fn hmm_compare_scores(query: &[Hmm], reference: &[Hmm], lb: f64) -> Vec<CmpResult> {
    if query.is_empty() || reference.is_empty() {
        return Vec::new();
    }
    let abc = Alphabet::new(query[0].abc_type);
    let bg = Bg::new(&abc);

    // Each (query, reference) comparison is independent; parallelize over queries
    // (the bg and reference set are shared read-only).
    query
        .par_iter()
        .flat_map_iter(|q| {
            let mut row = Vec::new();
            for r in reference {
                let score = round2_f32(compare_hmmer(q, r, &bg.f)) as f64;
                if score <= lb {
                    continue;
                }
                row.push(CmpResult {
                    score,
                    query: q.name.clone(),
                    subject: r.name.clone(),
                });
            }
            row.into_iter()
        })
        .collect()
}

// --- Alphabet helpers (mirroring domainator.utils) ---------------------------

/// The profile's alphabet as pyhmmer spells it: `amino`, `DNA` or `RNA`.
///
/// Note the inconsistent casing — it is pyhmmer's `Alphabet.type`, and
/// `hmmer_report --alphabet` prints it verbatim.
pub fn alphabet_name(abc: hmmer_pure_rs::alphabet::AlphabetType) -> &'static str {
    use hmmer_pure_rs::alphabet::AlphabetType;
    match abc {
        AlphabetType::Amino => "amino",
        AlphabetType::Dna => "DNA",
        AlphabetType::Rna => "RNA",
        AlphabetType::Unknown => "",
    }
}

/// Whether an alphabet is nucleic acid (DNA or RNA).
pub fn is_nucleic_acid_alphabet(abc: hmmer_pure_rs::alphabet::AlphabetType) -> bool {
    use hmmer_pure_rs::alphabet::AlphabetType;
    matches!(abc, AlphabetType::Dna | AlphabetType::Rna)
}

/// How this alphabet's score scale compares to the amino acid one: `log(K)/log(20)`.
///
/// Profile-profile scores are sums of natural-log-odds column scores, and the per-column
/// ceiling depends on the alphabet — about log(4)=1.39 for DNA/RNA against log(20)=3.0
/// for amino acids. A threshold tuned on proteins is therefore roughly twice as strict
/// on nucleotides, where it silently returns an empty result that looks exactly like
/// "no homologs found".
///
/// Exactly `1.0` for amino, so protein behavior is bit-identical.
pub fn alphabet_score_scale(abc: hmmer_pure_rs::alphabet::AlphabetType) -> f64 {
    use hmmer_pure_rs::alphabet::AlphabetType;
    let k = match abc {
        AlphabetType::Amino | AlphabetType::Unknown => return 1.0,
        AlphabetType::Dna | AlphabetType::Rna => 4.0_f64,
    };
    k.ln() / 20.0_f64.ln()
}

/// Default `--score_cutoff` for amino acid profiles.
pub const DEFAULT_SCORE_CUTOFF_AMINO: f64 = 10.0;

/// Alphabet-appropriate default for `--score_cutoff`.
///
/// Only the *default* is scaled: a value the user supplied explicitly is always taken
/// literally.
pub fn default_score_cutoff(abc: hmmer_pure_rs::alphabet::AlphabetType) -> f64 {
    DEFAULT_SCORE_CUTOFF_AMINO * alphabet_score_scale(abc)
}

/// Error text for a profile-profile comparison whose two sides disagree on alphabet.
///
/// Produced in the parent before any worker pool starts, so a mismatch reads as one
/// clear line rather than a panic from inside a rayon worker.
pub fn alphabet_mismatch_message(
    input: hmmer_pure_rs::alphabet::AlphabetType,
    reference: hmmer_pure_rs::alphabet::AlphabetType,
    input_name: &str,
    tool_name: &str,
) -> String {
    format!(
        "Input (-i) profiles in '{input_name}' are {}, but the reference (-r) profiles are \
         {}. {tool_name} aligns profiles column by column, so both sides must use the same \
         alphabet. Rebuild one side with hmmer-build --alphabet, or use domainate / \
         domain-search to search nucleotide profiles against sequences.",
        alphabet_name(input),
        alphabet_name(reference)
    )
}

/// The single alphabet shared by every profile in `hmms`, or an error naming the two
/// that disagree.
///
/// HMMER locks an `.hmm` file to the alphabet of its first profile, so a file mixing
/// alphabets cannot be read past the first one — and concatenating files of different
/// alphabets produces exactly such a file. The `hmmer_*` tools therefore reject a mixed
/// set rather than writing one. Returns `None` when there are no profiles at all.
pub fn common_hmm_alphabet(
    hmms: &[Hmm],
    role: &str,
) -> Result<Option<hmmer_pure_rs::alphabet::AlphabetType>, String> {
    let mut found: Option<(hmmer_pure_rs::alphabet::AlphabetType, &str)> = None;
    for hmm in hmms {
        match found {
            None => found = Some((hmm.abc_type, hmm.name.as_str())),
            Some((abc, first_name)) if abc != hmm.abc_type => {
                return Err(format!(
                    "The {role} profiles mix alphabets: '{first_name}' is {}, but '{}' is \
                     {}. An .hmm file is locked to the alphabet of its first profile, so a \
                     mixed set cannot be read back. Split them by alphabet, or rebuild one \
                     side with hmmer-build --alphabet.",
                    alphabet_name(abc),
                    hmm.name,
                    alphabet_name(hmm.abc_type)
                ))
            }
            Some(_) => {}
        }
    }
    Ok(found.map(|(abc, _)| abc))
}

/// Error when the `-i` and `-r` sides of a profile-profile comparison disagree.
pub fn check_alphabets_match(
    input: Option<hmmer_pure_rs::alphabet::AlphabetType>,
    reference: Option<hmmer_pure_rs::alphabet::AlphabetType>,
    input_name: &str,
    tool_name: &str,
) -> Result<(), String> {
    match (input, reference) {
        (Some(i), Some(r)) if i != r => Err(alphabet_mismatch_message(i, r, input_name, tool_name)),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod alphabet_tests {
    use super::*;
    use hmmer_pure_rs::alphabet::AlphabetType;

    #[test]
    fn amino_scale_is_exactly_one() {
        // Load-bearing: any drift here would change every protein default silently.
        assert_eq!(alphabet_score_scale(AlphabetType::Amino), 1.0);
        assert_eq!(default_score_cutoff(AlphabetType::Amino), 10.0);
    }

    #[test]
    fn nucleotide_scale_halves_the_default() {
        let scale = alphabet_score_scale(AlphabetType::Dna);
        assert!((scale - (4.0_f64.ln() / 20.0_f64.ln())).abs() < 1e-12);
        assert!((scale - 0.46276).abs() < 1e-4, "got {scale}");
        // DNA and RNA share a scale: same alphabet size.
        assert_eq!(scale, alphabet_score_scale(AlphabetType::Rna));
        assert!((default_score_cutoff(AlphabetType::Dna) - 4.6276).abs() < 1e-3);
    }

    #[test]
    fn alphabet_names_match_pyhmmer_casing() {
        assert_eq!(alphabet_name(AlphabetType::Amino), "amino");
        assert_eq!(alphabet_name(AlphabetType::Dna), "DNA");
        assert_eq!(alphabet_name(AlphabetType::Rna), "RNA");
        assert!(is_nucleic_acid_alphabet(AlphabetType::Dna));
        assert!(!is_nucleic_acid_alphabet(AlphabetType::Amino));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The profile-profile DP is float32 on both sides and `round(np.float32, 2)`
    /// stays float32, so the value that reaches a matrix is a *float32* widened to
    /// double. Rounding in f64 gives a clean 146.4 where the reference file says
    /// 146.39999389648438.
    #[test]
    fn scores_round_in_f32_so_they_widen_like_numpy() {
        assert_eq!(round2_f32(146.40234) as f64, 146.39999389648438);
        assert_eq!(round2_f32(5.0512) as f64, 5.050000190734863);
    }

    /// Python's `round` -- builtin on floats and `np.round` on numpy scalars --
    /// breaks ties to even, unlike Rust's `round`, which goes away from zero.
    #[test]
    fn ties_round_to_even() {
        // Both scale to an exact `.5` in f32, so these really are ties: 12 is even
        // and 38 is even. Rust's own `round` would give 0.13 for the first.
        assert_eq!(round2_f32(0.125), 0.12);
        assert_eq!(round2_f32(0.375), 0.38);
    }
}
