# domainatoredux local patch (not upstream)

This directory is the published `infernox` 0.2.0 crate (crates.io), wired in through
`[patch.crates-io]` in the workspace `Cargo.toml`. One option is added; with it unset
(the default) the library behaves exactly as the release:

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

Check the result with `diff -r ~/.cargo/registry/src/*/infernox-0.2.0 vendor/infernox`.
To drop the patch, see UPSTREAM.md, "Pinned dependency: `infernox`".

`cargo test --lib` on this crate (run from a copy outside the workspace) has 5 failures
in `cm_dpsearch_trunc::tests`, with or without the patch: they read a CM from the
author's machine (`/mnt/DAS/...`), which the published crate doesn't include.
