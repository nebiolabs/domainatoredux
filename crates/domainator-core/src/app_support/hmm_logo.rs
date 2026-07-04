//! Sequence-logo data for an HMM (skylign-style), computed from a profile's match
//! emission probabilities. The frontend renders stacked letters scaled by
//! information content, plus per-position occupancy / insert / delete bars.
//!
//! Information content per match state: `IC = log2(K) - H`, where
//! `H = -Σ p·log2 p` over the `K` canonical residues (uniform background). Each
//! letter's height is `p·IC` bits. Occupancy follows HMMER's
//! `p7_hmm_CalculateOccupancy` recursion. We do NOT port the GPL Bio-HMM-Logo code
//! — this is the standard, independently-implemented math.

use hmmer_pure_rs::alphabet::AlphabetType;
use hmmer_pure_rs::hmm::{DM, MD, MI, MM};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Letter {
    pub char: String,
    /// Height in bits (`p · IC`).
    pub height: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogoPosition {
    /// 1-based match-state index.
    pub index: usize,
    /// Stack height in bits (`Σ letter heights`).
    pub total_ic: f64,
    /// Match-state occupancy in [0,1].
    pub occupancy: f64,
    /// P(match→insert) at this node — the insert bar.
    pub insert_prob: f64,
    /// P(match→delete) at this node — the delete bar.
    pub delete_prob: f64,
    /// Letters ascending by height (render bottom→top).
    pub letters: Vec<Letter>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HmmLogo {
    pub name: String,
    pub acc: Option<String>,
    pub desc: Option<String>,
    /// `"amino"`, `"dna"`, or `"rna"`.
    pub alphabet: String,
    /// Number of match states.
    pub length: usize,
    /// `log2(K)` — the y-axis maximum (4.32 for protein, 2.0 for nucleotide).
    pub max_height_bits: f64,
    pub positions: Vec<LogoPosition>,
}

/// The canonical residue letters for an alphabet (order matches the emission rows).
fn canonical_letters(abc: AlphabetType) -> &'static str {
    match abc {
        AlphabetType::Amino => "ACDEFGHIKLMNPQRSTVWY",
        AlphabetType::Dna => "ACGT",
        AlphabetType::Rna => "ACGU",
        _ => "ACDEFGHIKLMNPQRSTVWY",
    }
}

fn alphabet_name(abc: AlphabetType) -> &'static str {
    match abc {
        AlphabetType::Amino => "amino",
        AlphabetType::Dna => "dna",
        AlphabetType::Rna => "rna",
        _ => "amino",
    }
}

/// HMMER match-state occupancy recursion (`p7_hmm_CalculateOccupancy`).
/// `t[k]` are the transitions out of node `k`; `t[0]` is the begin node.
fn occupancy(t: &[[f32; 7]], m: usize) -> Vec<f64> {
    let mut mocc = vec![0.0f64; m + 1];
    if m >= 1 {
        mocc[1] = t[0][MM] as f64 + t[0][MI] as f64;
        for k in 2..=m {
            let prev = mocc[k - 1];
            mocc[k] = prev * (t[k - 1][MM] as f64 + t[k - 1][MI] as f64)
                + (1.0 - prev) * t[k - 1][DM] as f64;
        }
    }
    mocc
}

/// A lightweight summary of one profile in an HMM file (for the browser list;
/// scales to files with tens of thousands of models — no emission data returned).
#[derive(Debug, Clone, Serialize)]
pub struct HmmSummary {
    pub index: usize,
    pub name: String,
    pub acc: Option<String>,
    pub desc: Option<String>,
    pub length: usize,
    pub alphabet: String,
}

/// List all profiles in an HMM file as summaries (name/length/alphabet).
pub fn list_hmms(path: &str) -> Result<Vec<HmmSummary>, String> {
    let hmms = hmmer_pure_rs::hmmfile::read_hmm_file(std::path::Path::new(path))
        .map_err(|e| format!("reading HMM: {e:?}"))?;
    Ok(hmms
        .iter()
        .enumerate()
        .map(|(i, h)| HmmSummary {
            index: i,
            name: h.name.clone(),
            acc: h.acc.clone(),
            desc: h.desc.clone(),
            length: h.m,
            alphabet: alphabet_name(h.abc_type).to_string(),
        })
        .collect())
}

/// Read an `.hmm` file and build the logo for profile `index` (0-based). Keeps the
/// `hmmer_pure_rs` dependency inside core so callers (the GUI) don't take it on.
pub fn load_logo_from_file(path: &str, index: usize) -> Result<HmmLogo, String> {
    let hmms = hmmer_pure_rs::hmmfile::read_hmm_file(std::path::Path::new(path))
        .map_err(|e| format!("reading HMM: {e:?}"))?;
    let hmm = hmms
        .get(index)
        .ok_or_else(|| format!("no HMM at index {index} (file has {})", hmms.len()))?;
    Ok(build_logo(hmm))
}

/// Build the logo for one parsed HMM.
pub fn build_logo(hmm: &hmmer_pure_rs::Hmm) -> HmmLogo {
    let k = hmm.abc_k;
    let letters = canonical_letters(hmm.abc_type).chars().collect::<Vec<_>>();
    let max_bits = (k as f64).log2();
    let mocc = occupancy(&hmm.t, hmm.m);

    let mut positions = Vec::with_capacity(hmm.m);
    for node in 1..=hmm.m {
        let probs = &hmm.mat[node];
        // Shannon entropy over the K canonical residues.
        let mut entropy = 0.0f64;
        for &p in probs.iter().take(k) {
            let p = p as f64;
            if p > 0.0 {
                entropy -= p * p.log2();
            }
        }
        let ic = (max_bits - entropy).max(0.0);

        let mut ls: Vec<Letter> = (0..k)
            .map(|i| Letter {
                char: letters.get(i).copied().unwrap_or('?').to_string(),
                height: probs[i] as f64 * ic,
            })
            .filter(|l| l.height > 0.001)
            .collect();
        ls.sort_by(|a, b| a.height.total_cmp(&b.height));

        let t = &hmm.t[node];
        positions.push(LogoPosition {
            index: node,
            total_ic: ic,
            occupancy: mocc[node].clamp(0.0, 1.0),
            insert_prob: t[MI] as f64,
            delete_prob: t[MD] as f64,
            letters: ls,
        });
    }
    HmmLogo {
        name: hmm.name.clone(),
        acc: hmm.acc.clone(),
        desc: hmm.desc.clone(),
        alphabet: alphabet_name(hmm.abc_type).to_string(),
        length: hmm.m,
        max_height_bits: max_bits,
        positions,
    }
}
