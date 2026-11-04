# domainatoredux local patches (not upstream)

This directory is the published `infernox` 0.2.0 crate (crates.io), wired in through
`[patch.crates-io]` in the workspace `Cargo.toml`. It carries two patches. The text meant
for the infernox maintainers (one PR, one bug report) is in `UPSTREAM_PR.md`.

Check the result with `diff -r ~/.cargo/registry/src/*/infernox-0.2.0 vendor/infernox`
(`PATCHES.md` and `UPSTREAM_PR.md` are ours). To drop the patches, see UPSTREAM.md,
"Pinned dependency: `infernox`".

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

## Known, not patched: CM-pipeline SSV window definition

Six base-paired families (CM pipeline, not HMM-only) still differ from C on the same input:
IRES_Tobamo, S16-Flavobacteria, mir-7616, plasmodium_snoR26, psm_mec_RNA and snoPyro_CD.
The first divergent stage is the SSV window count:

| Family | Options | C | infernox |
|---|---|---|---|
| S16-Flavobacteria | default | 2820 | 2830 |
| S16-Flavobacteria | `--toponly --notrunc` | 1456 | 1469 |
| snoPyro_CD | default | 877 | 875 |

So the cause is in window definition (`f1_msv_windows`: SSV longtarget, ExtendAndMerge,
split > 2*cmW). It is not in the truncated passes or the bottom strand. The HMM-only pass
uses the same code and matches C, probably because its F1 is 0.02 rather than 0.35, so far
fewer windows reach the merge and split edge cases. Not yet diagnosed. The next step is to
diff the `(ws, we)` lists on `S16-Flavobacteria --toponly --notrunc`.
