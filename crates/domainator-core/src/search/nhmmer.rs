//! nhmmer via hmmer-pure-rs: search DNA/RNA profiles against whole nucleotide
//! contigs (both strands), producing [`NamedHit`]s (mirrors `domainate.run_search`
//! for the `nhmmer` program + `nhmmer_hits_to_search_results`).
//!
//! This ports the core long-target driver from hmmer-pure-rs's `subcmd/nhmmer.rs`
//! (the canonical genome-scale search), dropping all text-output formatting. The
//! E-value normalization matches C HMMER's `p7_tophits_ComputeNhmmerEvalues`:
//! `residues_searched` is the total residues over all targets × strands searched,
//! and after the per-hit `lnP += log(N/W)` adjustment `evalue = exp(lnP)` directly.

use std::sync::atomic::{AtomicU64, Ordering};

use hmmer_pure_rs::alphabet::{Alphabet, AlphabetType, DSQ_SENTINEL};
use hmmer_pure_rs::bg::Bg;
use hmmer_pure_rs::pipeline::Pipeline;
use hmmer_pure_rs::profile::{profile_config, Profile, P7_LOCAL};
use hmmer_pure_rs::sequence::Sequence;
use hmmer_pure_rs::simd::oprofile::OProfile;
use hmmer_pure_rs::tophits::{Hit, TopHits, P7_IS_REPORTED};
use hmmer_pure_rs::Hmm;

use super::hmmer::NamedHit;
use super::results::SearchResult;

/// A nucleotide reference database (DNA/RNA HMMs) searched against whole contigs.
pub struct NuclDb {
    pub name: String,
    pub hmms: Vec<Hmm>,
}

/// Run every profile in `db` against the whole-contig `targets` (`(name,
/// residues)`, both strands), returning domains with `min_evalue <= i_evalue <
/// evalue`. Target names follow Python's `"{contig_index},contig,{len}"`.
///
/// `max_mode` maps to nhmmer `--max` (all heuristic filters off). E-values are
/// computed against the residues actually searched (Python passes no `-Z` to
/// `nhmmer`, so the search space is the residue count, not `-Z`).
pub fn nhmmer_db(
    db: &NuclDb,
    targets: &[(String, Vec<u8>)],
    evalue: f64,
    min_evalue: f64,
    max_mode: bool,
) -> Vec<NamedHit> {
    let mut out = Vec::new();
    for hmm in &db.hmms {
        let abc = match hmm.abc_type {
            AlphabetType::Rna => Alphabet::rna(),
            _ => Alphabet::dna(),
        };
        let mut bg = Bg::new(&abc);
        // Configure bias filter HMM with model composition (C nhmmer.c calls
        // p7_bg_SetFilter(bg, om->M, om->compo)).
        bg.set_filter(hmm.m, &hmm.compo);

        let mut gm = Profile::new(hmm.m, &abc);
        profile_config(hmm, &bg, &mut gm, 400, P7_LOCAL);
        let om = OProfile::convert(&gm);

        // Digitize contigs to nhmmer Sequences.
        let sequences: Vec<Sequence> = targets
            .iter()
            .map(|(name, res)| {
                let dsq = abc.digitize(res);
                Sequence {
                    name: name.clone(),
                    acc: String::new(),
                    desc: String::new(),
                    dsq,
                    n: res.len(),
                    l: res.len(),
                    taxid: -1,
                }
            })
            .collect();

        let max_length = if hmm.max_length > 0 {
            hmm.max_length
        } else {
            (hmm.m * 4) as i32
        };
        let do_watson = true;
        let do_crick = abc.complement.is_some();

        let msv_counter = AtomicU64::new(0);
        let bias_counter = AtomicU64::new(0);
        let vit_counter = AtomicU64::new(0);
        let fwd_counter = AtomicU64::new(0);

        use rayon::prelude::*;
        let all_hits: Vec<Hit> = sequences
            .par_iter()
            .flat_map(|sq| {
                let mut hits = Vec::new();
                if do_watson {
                    hits.extend(search_longtarget(
                        sq,
                        hmm,
                        &gm,
                        &om,
                        &bg,
                        max_length,
                        max_mode,
                        false,
                        &msv_counter,
                        &bias_counter,
                        &vit_counter,
                        &fwd_counter,
                    ));
                }
                if do_crick {
                    let mut rc_dsq = sq.dsq.clone();
                    abc.revcomp(&mut rc_dsq, sq.n);
                    let rc_sq = Sequence {
                        name: sq.name.clone(),
                        acc: sq.acc.clone(),
                        desc: sq.desc.clone(),
                        dsq: rc_dsq,
                        n: sq.n,
                        l: sq.l,
                        taxid: sq.taxid,
                    };
                    let mut rc_hits = search_longtarget(
                        &rc_sq,
                        hmm,
                        &gm,
                        &om,
                        &bg,
                        max_length,
                        max_mode,
                        true,
                        &msv_counter,
                        &bias_counter,
                        &vit_counter,
                        &fwd_counter,
                    );
                    // Convert complement-strand coords back to forward strand
                    // (nhmmer convention: minus-strand hits emit iali > jali).
                    for hit in &mut rc_hits {
                        for dom in &mut hit.dcl {
                            let ali_hi = sq.n as i64 - dom.iali + 1;
                            let ali_lo = sq.n as i64 - dom.jali + 1;
                            dom.iali = ali_hi;
                            dom.jali = ali_lo;
                            let env_hi = sq.n as i64 - dom.ienv + 1;
                            let env_lo = sq.n as i64 - dom.jenv + 1;
                            dom.ienv = env_hi;
                            dom.jenv = env_lo;
                            if let Some(ref mut ad) = dom.ad {
                                let ad_hi = sq.n.saturating_sub(ad.sqfrom) + 1;
                                let ad_lo = sq.n.saturating_sub(ad.sqto) + 1;
                                ad.sqfrom = ad_hi;
                                ad.sqto = ad_lo;
                            }
                        }
                    }
                    hits.extend(rc_hits);
                }
                hits
            })
            .collect();

        let mut th = TopHits::new();
        th.hits = all_hits;

        // nhmmer E-value space is in residues: each strand counted once.
        let total_residues: usize = sequences.iter().map(|s| s.n).sum();
        let strand_multiplier = (do_watson as usize) + (do_crick as usize);
        let residues_searched = (total_residues * strand_multiplier.max(1)) as f64;

        let window_length = if hmm.max_length > 0 {
            hmm.max_length as f64
        } else {
            (hmm.m * 4) as f64
        };
        // C p7_tophits_ComputeNhmmerEvalues: lnP += log((float)N/(float)W).
        let nhmmer_ln_nw = ((residues_searched as f32) / (window_length as f32)).ln() as f64;
        for hit in &mut th.hits {
            hit.lnp += nhmmer_ln_nw;
            hit.sortkey = hit.lnp;
            if let Some(dom0) = hit.dcl.first_mut() {
                dom0.lnp = hit.lnp;
            }
        }

        // After nhmmer adjustment, evalue = exp(lnp) directly (no × Z).
        let z = 1.0_f64;
        let mut pli = Pipeline::new();
        pli.new_model(&gm);
        pli.long_target = true;
        pli.e_value_threshold = evalue;
        pli.inc_e = evalue;
        pli.z = residues_searched;
        pli.n_targets = sequences.len() as u64;

        // Match C nhmmer flow: positional sort → dedup → sortkey sort → threshold.
        th.sort_by_seqidx_and_alipos();
        th.remove_duplicates();
        th.sort_by_sortkey();
        th.threshold(&pli, z, z);

        let desc = hmm.desc.clone().unwrap_or_default();
        let acc = hmm.acc.clone().unwrap_or_default();
        let rlen = hmm.m as i64;
        for hit in &th.hits {
            if hit.flags & P7_IS_REPORTED == 0 {
                continue;
            }
            // hit.name is the target contig name `"{i},contig,{len}"`.
            let mut parts = hit.name.split(',');
            let contig_len: Option<i64> = parts.nth(2).and_then(|s| s.parse::<i64>().ok());
            for dom in &hit.dcl {
                let i_evalue = dom.lnp.exp();
                if !(i_evalue < evalue && i_evalue >= min_evalue) {
                    continue;
                }
                // env_from-1 .. env_to; reverse hits swap and set strand -1.
                let mut start = dom.ienv - 1;
                let mut end = dom.jenv;
                let strand: i32;
                if start > end {
                    let (s2, e2) = (end - 1, start + 1);
                    start = s2;
                    end = e2;
                    strand = -1;
                } else {
                    strand = 1;
                }
                if let Some(len) = contig_len {
                    if start >= len {
                        continue;
                    }
                }
                let (rstart, rend, identity) = match &dom.ad {
                    Some(ad) => {
                        let matches = ad.mline.chars().filter(|c| c.is_ascii_alphabetic()).count();
                        let aln_len = (ad.hmmto as i64 - (ad.hmmfrom as i64 - 1)).max(1);
                        (
                            ad.hmmfrom as i64,
                            ad.hmmto as i64,
                            100.0 * matches as f64 / aln_len as f64,
                        )
                    }
                    None => (0, 0, 0.0),
                };
                out.push(NamedHit {
                    target: hit.name.clone(),
                    result: SearchResult {
                        name: hmm.name.clone(),
                        desc: desc.clone(),
                        acc: acc.clone(),
                        evalue: i_evalue,
                        score: dom.bitscore as f64,
                        start,
                        end,
                        database: db.name.clone(),
                        identity,
                        rstart,
                        rend,
                        rlen,
                        program: "nhmmer".to_string(),
                        strand,
                        // Structural metrics are absent for sequence backends.
                        ..SearchResult::default()
                    },
                });
            }
        }
    }
    out
}

/// Search a long target sequence using the SSV longtarget filter (port of
/// hmmer-pure-rs `subcmd/nhmmer.rs::search_longtarget`, sans text output).
#[allow(clippy::too_many_arguments)]
#[cfg(target_arch = "x86_64")]
fn search_longtarget(
    sq: &Sequence,
    hmm: &Hmm,
    gm: &Profile,
    om: &OProfile,
    bg: &Bg,
    max_length: i32,
    do_max: bool,
    is_complement: bool,
    msv_counter: &AtomicU64,
    bias_counter: &AtomicU64,
    vit_counter: &AtomicU64,
    fwd_counter: &AtomicU64,
) -> Vec<Hit> {
    use hmmer_pure_rs::simd::ssv_longtarget;

    if !is_x86_feature_detected!("sse2") {
        return Vec::new();
    }

    let f3 = 3e-5_f64;
    let nobias = do_max;
    let nonull2 = do_max;

    let mut om_ssv = om.clone();
    om_ssv.reconfig_msv_length(max_length);
    let om = &om_ssv;

    // SAFETY: the callee's only contract is SSE2 (`#[target_feature(enable = "sse2")]`);
    // this function is x86_64-only and returned above unless `is_x86_feature_detected!`
    // confirmed it.
    let mut windows = unsafe {
        ssv_longtarget::p7_ssv_filter_longtarget(&sq.dsq, sq.n, om, bg, 0.02, max_length)
    };
    if windows.is_empty() {
        return Vec::new();
    }

    let ml = if max_length > 0 {
        max_length as usize
    } else {
        hmm.m * 4
    };
    let (prefix_lens, suffix_lens) = ssv_longtarget::compute_prefix_suffix_lengths_from_om(om);
    ssv_longtarget::p7_pli_extend_and_merge_windows(
        &mut windows,
        ml,
        sq.n,
        0.0,
        &prefix_lens,
        &suffix_lens,
    );

    // Phase 1a: per-window standard MSV filter (F1 gate).
    let f1_thresh = 0.02_f64;
    let mut msv_filtered: Vec<ssv_longtarget::HmmWindow> = Vec::new();
    for win in &windows {
        let win_start = win.n;
        let win_end = (win.n + win.length - 1).min(sq.n);
        if win_end < win_start || (win_end - win_start + 1) < hmm.m {
            continue;
        }
        let win_len = win_end - win_start + 1;
        let mut sub_dsq = vec![DSQ_SENTINEL];
        sub_dsq.extend_from_slice(&sq.dsq[win_start..=win_end]);
        sub_dsq.push(DSQ_SENTINEL);

        let mut msv_om = om.clone();
        msv_om.reconfig_msv_length(win_len as i32);
        let mut bg_win = bg.clone();
        bg_win.set_length(win_len);
        let nullsc = bg_win.null_one(win_len);
        // SAFETY: SSE2 is confirmed at the top of this function, as above.
        let usc = match unsafe {
            hmmer_pure_rs::simd::msv_filter::p7_msv_filter(&sub_dsq, win_len, &msv_om)
        } {
            hmmer_pure_rs::simd::msv_filter::MsvResult::Ok(sc) => sc,
            hmmer_pure_rs::simd::msv_filter::MsvResult::Overflow => f32::INFINITY,
        };
        let msv_pval = hmmer_pure_rs::pipeline::msv_pvalue(usc, nullsc, &hmm.evparam);
        if msv_pval > f1_thresh {
            continue;
        }
        msv_counter.fetch_add(win_len as u64, Ordering::Relaxed);
        let mut filtered = win.clone();
        filtered.score = usc;
        msv_filtered.push(filtered);
    }
    let mut windows = msv_filtered;

    // Phase 1b: Vit_longtarget per merged SSV window.
    let f2_p_thresh = 3e-3_f64;
    let mut vit_windows: Vec<ssv_longtarget::HmmWindow> = Vec::new();
    for msv in &windows {
        let msv_start = msv.n;
        let msv_end = (msv.n + msv.length - 1).min(sq.n);
        if msv_end < msv_start || (msv_end - msv_start + 1) < hmm.m {
            continue;
        }
        let subseq_len = msv_end - msv_start + 1;
        let mut vit_sub_dsq = vec![DSQ_SENTINEL];
        vit_sub_dsq.extend_from_slice(&sq.dsq[msv_start..=msv_end]);
        vit_sub_dsq.push(DSQ_SENTINEL);

        const B1: usize = 110;
        const B2: usize = 240;
        let max_length_w = if hmm.max_length > 0 {
            hmm.max_length as usize
        } else {
            subseq_len
        };
        let f1_l = subseq_len.min(B1);
        let f2_l = subseq_len.min(B2);
        let loc_window_len = subseq_len.min(max_length_w);

        let mut bg_win = bg.clone();
        bg_win.set_length(subseq_len);
        let nullsc_full = bg_win.null_one(subseq_len);
        let bias_delta = if !nobias {
            bg_win.filter_score(&vit_sub_dsq, subseq_len) - nullsc_full
        } else {
            0.0
        };

        if !nobias {
            let f1_scale = if f1_l == subseq_len {
                1.0_f32
            } else {
                f1_l as f32 / subseq_len as f32
            };
            let filtersc_f1 = nullsc_full + bias_delta * f1_scale;
            let usc = msv.score;
            let bias_pval = hmmer_pure_rs::pipeline::msv_pvalue(usc, filtersc_f1, &hmm.evparam);
            if bias_pval > f1_thresh {
                continue;
            }
        }
        bias_counter.fetch_add(subseq_len as u64, Ordering::Relaxed);

        let mut bg_loc = bg.clone();
        bg_loc.set_length(loc_window_len);
        let nullsc_win = bg_loc.null_one(loc_window_len);

        let f2_scale = if f2_l == subseq_len {
            1.0_f32
        } else {
            f2_l as f32 / subseq_len as f32
        };
        let filtersc = nullsc_win + bias_delta * f2_scale;

        let mut vit_om = om.clone();
        vit_om.reconfig_length(loc_window_len as i32);
        // SAFETY: SSE2 is confirmed at the top of this function, as above.
        let mut sub_windows = unsafe {
            hmmer_pure_rs::simd::vit_filter::p7_viterbi_filter_longtarget(
                &vit_sub_dsq,
                subseq_len,
                &vit_om,
                filtersc,
                f2_p_thresh,
            )
        };
        ssv_longtarget::p7_pli_extend_and_merge_windows(
            &mut sub_windows,
            ml,
            subseq_len,
            0.5,
            &prefix_lens,
            &suffix_lens,
        );
        let mut prev_end: Option<usize> = None;
        for w in sub_windows {
            let abs_n = msv_start + w.n - 1;
            let mut add = w.length as i64;
            if let Some(pe) = prev_end {
                if pe > abs_n {
                    add -= (pe - abs_n) as i64;
                }
            }
            vit_counter.fetch_add(add as u64, Ordering::Relaxed);
            prev_end = Some(abs_n + w.length);
            vit_windows.push(ssv_longtarget::HmmWindow {
                n: abs_n,
                k: w.k,
                length: w.length,
                score: w.score,
                target_len: sq.n,
                complement: is_complement,
                id: 0,
                fm_n: -1,
                fm_start: 0,
                fm_bwt_len: 0,
            });
        }
    }
    if !vit_windows.is_empty() {
        windows = vit_windows;
    }

    // Window splitting for numerical stability in Forward (>80000 residues).
    const MAX_WINDOW_LEN: usize = 80000;
    let overlap_len = 40000_usize.min(ml);
    let mut split_windows: Vec<ssv_longtarget::HmmWindow> = Vec::new();
    for w in &windows {
        if w.length <= MAX_WINDOW_LEN {
            split_windows.push(w.clone());
            continue;
        }
        let mut head = w.clone();
        head.length = MAX_WINDOW_LEN;
        split_windows.push(head);
        let mut new_n = w.n;
        let mut new_len = w.length;
        loop {
            let shift = MAX_WINDOW_LEN - overlap_len;
            new_n += shift;
            new_len = new_len.saturating_sub(shift);
            if new_len == 0 {
                break;
            }
            let chunk = new_len.min(MAX_WINDOW_LEN);
            split_windows.push(ssv_longtarget::HmmWindow {
                n: new_n,
                k: 0,
                length: chunk,
                score: 0.0,
                target_len: w.target_len,
                complement: w.complement,
                id: w.id,
                fm_n: w.fm_n,
                fm_start: w.fm_start,
                fm_bwt_len: w.fm_bwt_len,
            });
            if new_len <= MAX_WINDOW_LEN {
                break;
            }
        }
    }
    windows = split_windows;

    // Phase 2: full pipeline per window.
    let mut all_hits = Vec::new();
    let mut fwd_overlap: usize = 0;
    for (win_idx, win) in windows.iter().enumerate() {
        let win_start = win.n;
        let win_end = (win.n + win.length - 1).min(sq.n);
        let win_len = win_end - win_start + 1;
        if win_len < hmm.m {
            fwd_overlap = 0;
            continue;
        }
        let mut win_dsq = vec![DSQ_SENTINEL];
        win_dsq.extend_from_slice(&sq.dsq[win_start..=win_end]);
        win_dsq.push(DSQ_SENTINEL);
        let win_sq = Sequence {
            name: sq.name.clone(),
            acc: sq.acc.clone(),
            desc: sq.desc.clone(),
            dsq: win_dsq,
            n: win_len,
            l: win_len,
            taxid: sq.taxid,
        };
        let mut lgm = gm.clone();
        let mut lom = om.clone();
        let mut lb = bg.clone();
        lb.set_length(win_len);
        let mut lpli = Pipeline::new();
        lpli.new_model(&lgm);
        lpli.long_target = true;
        lpli.f3 = f3;
        lpli.do_biasfilter = !nobias;
        lpli.do_null2 = !nonull2;
        lpli.do_max = do_max;

        let mut lth = TopHits::new();
        let ran = lpli.run(&mut lgm, &mut lom, &lb, hmm, &win_sq, &mut lth);
        if lpli.n_past_fwd > 0 {
            let add = (win_len as i64) - (fwd_overlap as i64);
            fwd_counter.fetch_add(add as u64, Ordering::Relaxed);
            if win_idx + 1 < windows.len() {
                let next = &windows[win_idx + 1];
                let this_end = win.n + win.length;
                fwd_overlap = this_end.saturating_sub(next.n);
            } else {
                fwd_overlap = 0;
            }
        } else {
            fwd_overlap = 0;
        }
        if ran {
            for src_hit in lth.hits {
                for mut dom in src_hit.dcl {
                    dom.iali += (win_start - 1) as i64;
                    dom.jali += (win_start - 1) as i64;
                    dom.ienv += (win_start - 1) as i64;
                    dom.jenv += (win_start - 1) as i64;
                    if let Some(ref mut ad) = dom.ad {
                        ad.sqfrom += win_start - 1;
                        ad.sqto += win_start - 1;
                    }
                    let new_hit = Hit {
                        name: sq.name.clone(),
                        acc: sq.acc.clone(),
                        desc: sq.desc.clone(),
                        n: sq.n,
                        score: dom.bitscore,
                        bias: dom.dombias.max(0.0),
                        pre_score: dom.bitscore + dom.dombias.max(0.0),
                        sum_score: dom.bitscore,
                        lnp: dom.lnp,
                        pre_lnp: dom.lnp,
                        sum_lnp: dom.lnp,
                        sortkey: dom.lnp,
                        nexpected: src_hit.nexpected,
                        nregions: src_hit.nregions,
                        nclustered: src_hit.nclustered,
                        noverlaps: src_hit.noverlaps,
                        nenvelopes: src_hit.nenvelopes,
                        ndom: 1,
                        nreported: 1,
                        nincluded: 1,
                        flags: src_hit.flags,
                        seqidx: src_hit.seqidx,
                        subseq_start: src_hit.subseq_start,
                        dcl: vec![dom],
                    };
                    all_hits.push(new_hit);
                }
            }
        }
    }
    all_hits
}

#[cfg(not(target_arch = "x86_64"))]
#[allow(clippy::too_many_arguments)]
fn search_longtarget(
    _sq: &Sequence,
    _hmm: &Hmm,
    _gm: &Profile,
    _om: &OProfile,
    _bg: &Bg,
    _max_length: i32,
    _do_max: bool,
    _is_complement: bool,
    _msv_counter: &AtomicU64,
    _bias_counter: &AtomicU64,
    _vit_counter: &AtomicU64,
    _fwd_counter: &AtomicU64,
) -> Vec<Hit> {
    Vec::new()
}
