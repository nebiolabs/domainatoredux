# domainatoredux local patches (not upstream)

This directory is the published `infernox` 0.2.0 crate (crates.io), wired in through
`[patch.crates-io]` in the workspace `Cargo.toml`. It carries three patches. The text
meant for the infernox maintainers (three PRs) is in `UPSTREAM_PR.md`.

Check the result with `diff -r ~/.cargo/registry/src/*/infernox-0.2.0 vendor/infernox`
(`PATCHES.md`, `UPSTREAM_PR.md` and `ISSUE_hmmonly_score_precision.md` are ours). To drop
the patches, see UPSTREAM.md, "Pinned dependency: `infernox`".

`cargo test --lib` on this crate (run from a copy outside the workspace) has 5 failures
in `cm_dpsearch_trunc::tests`, with or without the patches: they read a CM from the
author's machine (`/mnt/DAS/...`), which the published crate doesn't include.

## 1. `FaithfulConfig::whole_sequence` (an added option)

With the option unset (the default) the library behaves exactly as the release.

- `src/cm_search.rs`: `FaithfulConfig::whole_sequence` (default `false`). When it is set,
  `window_tasks` makes one window per (sequence, strand) instead of chunking each sequence
  into `CM_MAX_RESIDUE_COUNT` (100 000) residue windows with a maxW overlap.
- `src/bin/cmsearch.rs`, `src/bin/cmscan.rs`: the new field is set to `false` in their
  struct literals, so the binaries are unchanged.

Why: domainatoredux must give the same hits as upstream's `domainate`, which searches with
`pyinfernal.cmsearch`. pyinfernal runs `cm_pli_NewSeq` + `cm_Pipeline` once per whole
in-memory sequence and strand (`cm.pyx`, `_search_loop`; the bottom strand is a
reverse-complemented copy of the whole sequence). C `cmsearch`, which infernox
reproduces, reads a file in `esl_sqio_ReadWindow` chunks. The filter stages score each
window, so on a contig longer than 100 kb the two can keep or drop different
near-threshold hits.

## 2. HMM-only pipeline: run the MSV composition-bias filter (a bug fix)

This one changes default behavior, for the `infernox-cmsearch`/`cmscan` binaries too, and
brings them in line with C. A model with no base pairs (or any model under `--hmmonly`) is
searched by Infernal's HMM-only pipeline. In that pass C runs Filter 1B, the MSV
composition-bias window filter, thresholded at F1. Infernal 1.1.5, `cm_pipeline.c`
`pli_p7_filter`:

```c
int    cur_do_msvbias = (pli->cur_pass_idx == PLI_PASS_HMM_ONLY_ANY) ? pli->do_bias_hmmonly : pli->do_msvbias;  /* 2515 */
double cur_F1b        = (pli->cur_pass_idx == PLI_PASS_HMM_ONLY_ANY) ? pli->F1_hmmonly      : pli->F1b;         /* 2521 */
```

`do_bias_hmmonly` is TRUE unless `--hmmmax` or `--hmmnobias` is given (619-632).
`hmmonly_one_strand` passed `None` for the bias filter, on the stated assumption that null2
in domain definition covered it. That assumption is wrong. Without the filter nearly every
SSV window reaches Viterbi and Forward, and on low-complexity sequence the extra windows
produce hits C never reports. The fix:

- `src/cm_search.rs`:
  - `Thresh::hmm_do_bias` (from `HmmonlyFilterCfg::do_bias`).
  - `hmmonly_one_strand` passes `std_msvbias_params(th.hmm_do_bias, th.hmm_f1)` to
    `f3_filter_sequence`.
  - `hmmonly_pass_stats` reports `n_past_msvbias`/`pos_past_msvbias`. It used to report
    the SSV counts as a placeholder.
  - Two stale comments about F2 not being wired are removed.
- `src/cm_pipeline.rs`: a window the F1b filter drops also clears `surv_f2`/`surv_f2b`.
  C sets `survAA[stage][i]` only when a window reaches a stage, so the Viterbi line used to
  over-count. This only affects stats, and the same bug was latent in the CM pass under
  `--doF1b`.

Measured on a 142 kb phage (NC_023573.1) with Rfam 15.1 at `-Z 0.142094`:
- DLEU2_1 (RF02105): C reports 2 hits, infernox 0.2.0 reported 23, patched infernox
  reports C's 2. The whole HMM-only statistics block is now identical to C's.
- `--hmmnobias` in C reproduces infernox 0.2.0's 23 hits exactly.
- Across all 42 zero-basepair families that differed, the patched hits equal C's. Two
  E-values differ in the last printed digit at the same score.
- `--hmmnobias`, `--hmmmax` and `--hmmonly` (on base-paired models) also match C in hits
  and statistics.

## 3. SSV filter: break ties between model positions in striped order (a bug fix)

This one changes default behavior too, for the binaries as well, and brings them in line
with C. When a row of the SSV longtarget DP reaches the F1 threshold, the diagonal is
traced back from the model position with the row's best score. C finds that position by
scanning the row in its striped SIMD layout, vector `q` outer and lane `z` inner, keeping
the first strict maximum. hmmer 3.4 `msvfilter.c:373-381`:

```c
for (q = 0; q < Q; q++) {
  u.v = dp[q];
  for (k = 0; k < 16; k++) {
    if (u.b[k] >= sc_thresh && u.b[k] > rem_sc && (q+Q*k+1) <= om->M) {
      end = (q+Q*k+1);
      rem_sc = u.b[k];
    } } ... }
```

Lane `z` of vector `q` is model position `q + Q*z + 1`, so on a tie C keeps the position
with the smallest `(q, z)`, which is not in general the lowest position. `ssv_diagonals`
rescanned the row in natural order and kept the lowest. A different end position traces a
different diagonal, which extends to a different window. Uint8 scores saturate, so ties
are common.

- `src/cm_pipeline.rs`, `ssv_diagonals`:
  - The argmax scans in C's order (`Q = p7O_NQB(M) = max(2, (M-1)/16 + 1)`).
  - `p1`, `nullsc` and `invP` are rounded to `float` as in C (`p7_bg.c:191`,
    `p7_bg_NullOne`, `msvfilter.c:312`) before `sc_thresh`'s `ceil()`.
  - `build_msv_filter` rounds the MSV `mu`/`lambda` to `float`, as `om->evparam` is.
  - The precision changes alter nothing on the input below. hmmer-pure-rs found the same
    `double` vs `float` difference moving `ceil()` on other inputs.

Measured on the 142 kb phage NC_023573.1 with Rfam 15.1 at `-Z 0.142094`:

| Family | Options | C SSV windows | infernox 0.2.0 | patched |
|---|---|---|---|---|
| S16-Flavobacteria | default | 2820 | 2830 | 2820 |
| S16-Flavobacteria | `--toponly --notrunc` | 1456 | 1469 | 1456 |
| snoPyro_CD | default | 877 | 875 | 877 |

- The 6 base-paired families that differed from C (IRES_Tobamo, S16-Flavobacteria,
  mir-7616, plasmodium_snoR26, psm_mec_RNA, snoPyro_CD) now match C in hits, scores and
  the whole statistics block, with default options and with `--toponly --notrunc`,
  `--nonull3` and `--F1 0.5`. Before, with default options, they had 9 hits C doesn't
  report and missed 3.
- The 42 zero-basepair families (HMM-only pipeline, which uses the same SSV code) still
  match. Two of their E-values differ from C in the last printed digit at the same score
  (DLEU2_4 1.4 vs 1.5, TtnuCD7 0.97 vs 0.98), as they did before this patch. The cause
  is HMM-only hit scores differing from C's by up to 5e-4 bits; not yet diagnosed, see
  `ISSUE_hmmonly_score_precision.md`.
