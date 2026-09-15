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
use hmmer_pure_rs::hmm::{DM, II, MD, MI, MM};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Letter {
    pub char: String,
    /// Height in bits (`p · IC`).
    pub height: f64,
    /// Raw match-state emission probability for this residue (`p`).
    pub prob: f64,
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
    /// Expected insert length at this node (`1 / (1 - P(insert→insert))`).
    pub insert_length: f64,
    /// Letters ascending by height (render bottom→top). Includes every canonical
    /// residue (even sub-pixel ones) so the click popup can list full frequencies;
    /// the renderer skips letters too short to see.
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
    // Header-only index scan (no emission-matrix parsing) — orders of magnitude
    // cheaper on large libraries (Pfam ≈ 20k models) than read_hmm_file.
    Ok(index_hmm_file(path)?
        .iter()
        .enumerate()
        .map(|(i, e)| HmmSummary {
            index: i,
            name: e.name.clone(),
            acc: e.acc.clone(),
            desc: e.desc.clone(),
            length: e.length,
            alphabet: e.alphabet.clone(),
        })
        .collect())
}

/// One profile's location + header fields within a multi-model HMM file, found by
/// a single header-only scan — no emission matrices parsed. `byte_start..byte_end`
/// is the exact byte range of the profile's text (from its `HMMER3/…` line through
/// the line after `//`), so a profile can be re-parsed or copied out by seeking
/// instead of re-reading the whole (possibly gigabyte-scale) file.
#[derive(Debug, Clone)]
pub struct HmmEntry {
    pub byte_start: u64,
    pub byte_end: u64,
    pub name: String,
    pub acc: Option<String>,
    pub desc: Option<String>,
    pub length: usize,
    pub alphabet: String,
}

/// Scan an HMM file once and return each profile's byte range + header fields
/// WITHOUT parsing emission matrices. This is the fast path behind `list_hmms`,
/// single-profile logo loading, and profile extraction on large libraries.
///
/// Header keys are matched on the first whitespace-delimited token, exactly like
/// `hmmer_pure_rs`'s reader, so DESC values containing spaces are preserved.
pub fn index_hmm_file(path: &str) -> Result<Vec<HmmEntry>, String> {
    use std::io::{BufRead, BufReader};
    let file = std::fs::File::open(path).map_err(|e| format!("opening {path}: {e}"))?;
    let mut reader = BufReader::new(file);
    let mut entries: Vec<HmmEntry> = Vec::new();
    let mut cur: Option<HmmEntry> = None;
    let mut offset: u64 = 0;
    let mut buf: Vec<u8> = Vec::new();
    loop {
        buf.clear();
        let n = reader
            .read_until(b'\n', &mut buf)
            .map_err(|e| format!("reading {path}: {e}"))?;
        if n == 0 {
            break;
        }
        let line_start = offset;
        offset += n as u64;
        let text = String::from_utf8_lossy(&buf);
        let trimmed = text.trim();
        if trimmed.starts_with("HMMER3/") || trimmed.starts_with("HMMER2") {
            // A stray record without a closing `//` is dropped (mirrors an
            // incomplete profile being unusable); start a fresh one here.
            cur = Some(HmmEntry {
                byte_start: line_start,
                byte_end: line_start,
                name: String::new(),
                acc: None,
                desc: None,
                length: 0,
                alphabet: "amino".into(),
            });
            continue;
        }
        let Some(e) = cur.as_mut() else { continue };
        if trimmed == "//" {
            e.byte_end = offset;
            entries.push(cur.take().unwrap());
            continue;
        }
        let mut it = trimmed.splitn(2, char::is_whitespace);
        let key = it.next().unwrap_or("");
        let val = it.next().unwrap_or("").trim();
        match key {
            "NAME" => e.name = val.to_string(),
            "ACC" if !val.is_empty() => e.acc = Some(val.to_string()),
            "DESC" if !val.is_empty() => e.desc = Some(val.to_string()),
            "LENG" => e.length = val.parse().unwrap_or(0),
            "ALPH" => e.alphabet = val.to_ascii_lowercase(),
            _ => {}
        }
    }
    Ok(entries)
}

/// Build a logo from the raw bytes of a SINGLE profile (as sliced out of a larger
/// file via an [`HmmEntry`] byte range). Keeps `hmmer_pure_rs` inside core.
pub fn logo_from_hmm_bytes(bytes: &[u8]) -> Result<HmmLogo, String> {
    let reader = std::io::BufReader::new(bytes);
    let hmms = hmmer_pure_rs::hmmfile::read_hmms(reader).map_err(|e| format!("reading HMM: {e:?}"))?;
    let hmm = hmms.first().ok_or("no HMM in profile block")?;
    Ok(build_logo(hmm))
}

/// Write the profiles at `indices` (0-based, in the given order) from `source` to a
/// new `.hmm` file at `out_path`. Keeps the `hmmer_pure_rs` dependency inside core.
pub fn extract_hmms_to_file(source: &str, indices: &[usize], out_path: &str) -> Result<(), String> {
    use std::io::{BufWriter, Write};
    let hmms = crate::search::hmm_read::read_hmm_file(std::path::Path::new(source))
        .map_err(|e| format!("reading HMM: {e:?}"))?;
    let file = std::fs::File::create(out_path).map_err(|e| format!("creating {out_path}: {e}"))?;
    let mut w = BufWriter::new(file);
    let mut n = 0usize;
    for &i in indices {
        if let Some(h) = hmms.get(i) {
            crate::search::hmm_write::write_hmm(&mut w, h).map_err(|e| format!("writing HMM: {e:?}"))?;
            n += 1;
        }
    }
    w.flush().map_err(|e| format!("flushing {out_path}: {e}"))?;
    if n == 0 {
        return Err("no profiles selected".to_string());
    }
    Ok(())
}

/// Read an `.hmm` file and build the logo for profile `index` (0-based). Keeps the
/// `hmmer_pure_rs` dependency inside core so callers (the GUI) don't take it on.
pub fn load_logo_from_file(path: &str, index: usize) -> Result<HmmLogo, String> {
    let hmms = crate::search::hmm_read::read_hmm_file(std::path::Path::new(path))
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
    // `node` is HMMER's 1-based node index into both `hmm.mat` and `mocc`, so
    // iterating either one would lose the correspondence the loop is built on.
    #[allow(clippy::needless_range_loop)]
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

        // Every canonical residue (not just the visible ones) so the click popup
        // can show full emission frequencies; the renderer skips sub-pixel letters.
        let mut ls: Vec<Letter> = (0..k)
            .map(|i| Letter {
                char: letters.get(i).copied().unwrap_or('?').to_string(),
                height: probs[i] as f64 * ic,
                prob: probs[i] as f64,
            })
            .collect();
        ls.sort_by(|a, b| a.height.total_cmp(&b.height));

        let t = &hmm.t[node];
        let ii = t[II] as f64;
        positions.push(LogoPosition {
            index: node,
            total_ic: ic,
            occupancy: mocc[node].clamp(0.0, 1.0),
            insert_prob: t[MI] as f64,
            delete_prob: t[MD] as f64,
            insert_length: if ii < 1.0 { 1.0 / (1.0 - ii) } else { f64::INFINITY },
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
