# Drafts for necoli1822/infernox

These are two separate submissions. Nothing has been sent yet. Each patch is described in
`PATCHES.md`, and the code is in this directory (diff against the published 0.2.0).

---

## PR 1: HMM-only pipeline skips the MSV composition-bias filter (F1b)

**Title:** HMM-only pipeline: run the MSV bias filter (F1b) as C does

For a model with no base pairs, or any model under `--hmmonly`, `cmsearch` uses the
HMM-only pipeline. In that pass Infernal runs Filter 1B, the MSV composition-bias window
filter, with F1 as its threshold (Infernal 1.1.5 `cm_pipeline.c`, `pli_p7_filter`):

```c
int    cur_do_msvbias = (pli->cur_pass_idx == PLI_PASS_HMM_ONLY_ANY) ? pli->do_bias_hmmonly : pli->do_msvbias;  /* 2515 */
double cur_F1b        = (pli->cur_pass_idx == PLI_PASS_HMM_ONLY_ANY) ? pli->F1_hmmonly      : pli->F1b;         /* 2521 */
```

`do_bias_hmmonly` is TRUE unless `--hmmmax` or `--hmmnobias` is given. infernox's
`hmmonly_one_strand` passes `None` for `f3_filter_sequence`'s `msvbias` argument, with a
comment saying null2 in domain definition makes the filter unnecessary. It doesn't. Without
the filter nearly every SSV window goes on to Viterbi and Forward, and on low-complexity
sequence those windows produce hits C doesn't report.

**Reproduction** (Rfam 15.1, Staphylococcus phage phiSA012 NC_023573.1, 142 kb):

```
cmfetch Rfam.cm DLEU2_1 > dleu.cm
cmsearch          -Z 0.142094 dleu.cm NC_023573.fa   # 2 hits
infernox-cmsearch -Z 0.142094 dleu.cm NC_023573.fa   # 23 hits
cmsearch --hmmnobias -Z 0.142094 dleu.cm NC_023573.fa   # 23 hits, the same as infernox
```

HMM-only statistics block:

| Stage | C | infernox 0.2.0 | with this PR |
|---|---|---|---|
| SSV | 474 | 474 | 474 |
| MSV bias | 16 | 474 | 16 |
| Viterbi | 8 | 49 | 8 |
| Forward | 2 | 42 | 2 |
| hits | 2 | 23 | 2 |

With the PR, the whole statistics block is identical to C's.

**Change.** The F1b code already exists and is used by the CM pass under `--doF1b`:
`MsvBiasParams`, `std_msvbias_params`, and the F1b block of `f3_filter_sequence`.
- `cm_search.rs`:
  - Add `Thresh::hmm_do_bias`, taken from `HmmonlyFilterCfg::do_bias`.
  - Pass `self.std_msvbias_params(th.hmm_do_bias, th.hmm_f1)` in `hmmonly_one_strand`.
  - Make `hmmonly_pass_stats` report `n_past_msvbias`/`pos_past_msvbias` instead of the SSV
    counts.
- `cm_pipeline.rs`: when F1b drops a window, also clear `surv_f2`/`surv_f2b`. C only sets
  `survAA[stage][i]` once a window reaches that stage. Before this, the Viterbi count
  included windows F1b had dropped. That affects statistics only, and the CM pass under
  `--doF1b` had the same bug.

**Tested.**
- All 42 zero-basepair Rfam families that gave different hits on this phage now match C hit
  for hit. In two hits the last printed E-value digit differs at the same score.
- `--hmmnobias`, `--hmmmax`, and `--hmmonly` on base-paired models match C in hits and
  statistics.

---

## PR 2: `FaithfulConfig::whole_sequence`, search each sequence as one window (pyinfernal mode)

`FaithfulSearcher` reproduces C `cmsearch`, which reads targets with `esl_sqio_ReadWindow`
in `CM_MAX_RESIDUE_COUNT` (100 000) residue chunks with a maxW overlap. pyinfernal, the
Python binding most pipelines use, searches in-memory sequences differently. `_search_loop`
in `cm.pyx` runs `cm_pli_NewSeq` + `cm_Pipeline` once per whole sequence, then once more on
a reverse-complemented copy of the whole sequence. The filter stages score each window, so
on sequences longer than 100 kb the two can report different near-threshold hits.

This PR adds `whole_sequence: bool` to `FaithfulConfig`. It defaults to `false`, so
existing callers and the `cmsearch`/`cmscan` binaries are unchanged. When it is set,
`window_tasks` emits one task per (sequence, strand) with no overlap context, which is
pyinfernal's loop. Coordinates, overlap removal and residue accounting go through the same
code; with `ctx = 0` there is no overlap to subtract.

Motivation: domainatoredux, a Rust port of the domainator suite, has to match the Python
suite's pyinfernal-based results. The test was NC_023573.1 (Staphylococcus phage phiSA12,
142 kb) searched with Rfam 15.1 at E <= 10:
- The windowed search matched C `cmsearch` but differed from pyinfernal in 20 families,
  each with hits near the 100 kb boundary.
- With `whole_sequence` set, all 20 match pyinfernal.

---

## Possible issue: CM-pipeline SSV window definition (no fix yet)

On the same input with `-Z 0.142094`, six base-paired families still differ from C:
IRES_Tobamo, S16-Flavobacteria, mir-7616, plasmodium_snoR26, psm_mec_RNA and snoPyro_CD.
They already differ at the first stage, the SSV window count:

| Family | Options | C | infernox |
|---|---|---|---|
| S16-Flavobacteria | default | 2820 | 2830 |
| S16-Flavobacteria | `--toponly --notrunc` | 1456 | 1469 |
| snoPyro_CD | default | 877 | 875 |

The cause is therefore in `f1_msv_windows` (SSV longtarget, ExtendAndMerge, split > 2*cmW),
not in the truncated passes or the bottom strand. I'll file this once I've found the first
window that differs.
