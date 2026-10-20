# Staying in sync with the Python `domainator`

This Rust workspace is a rewrite of the Python `domainator`
(<https://github.com/nebiolabs/domainator>). Correctness is defined by **parity** with
that tool, so as the Python package evolves we periodically pull its changes across. This
file records how far the port is caught up and how to advance it.

## Upstream pin

- **Caught up to:** `02a31c1` — *handling bare /partial qualifiers* (2026-10-04; upstream
  `__version__` 0.12.0). The nine commits since `9c70934`:
- **Wheels / CI** (`4b67f66`, `e55c56c`, `6aa216f`): packaging only; nothing to port.
- **Partial features** (`8bf3c42`), ported. `slice_record_from_location` now truncates
  every feature a slice cuts and marks the cut ends with `<`/`>`. The exceptions: `source`
  never gets new markers, a cut `Domainator`/`Domain_Search` hit is dropped, and a feature
  split across a rotated origin is not cut. A cut CDS gets an updated `/codon_start` and a
  trimmed `/translation`, `merge_parts` keeps markers, and `get_cds_neighborhood` lost
  its containment pre-filter. The port had six slicers; they are now one,
  `neighborhood::slice_features` (with `slice_record` / `slice_linear_features` /
  `extract_common::slice_from_location` around it). `/codon_start` is honored when
  `domainate` places a hit (`overlay`, which now refuses out-of-bounds spans the way
  Python raises), and in fallback translations and `extract-peptides` coordinates. An
  overlong partial-CDS translation is trimmed (`seq/partial.rs`, `on_contig_translation`).
  Fragment status and `--partial` on `domainate`/`domain-search` come from this commit,
  as does `enum-report --fragment`.
- **Fragment support in more tools** (`bdc74bb`), ported. `--partial` is now also on
  `select-by-cds`, `select-by-contig`, `extract-peptides`, `extract-domains`,
  `enum-report` (a row filter) and `summary-report`, and `enum-report` gains
  `--partial_count`. `summary-report` reports `partial_cdss`/`fragment_proteins`. Fragment
  status also reads `NON_TER` features and `Flags: Fragment`. The `find_features` and
  `domainator_db_download` halves have no counterpart, since both tools are unported.
- **Gene calling keeps partial genes** (`f39ec15`), ported along with `--gene_call`
  itself. Prodigal genes running off a linear contig become `<`/`>` partial CDSs, and
  origin-crossing genes on circular contigs are called from a window across the origin.
  The port runs the external Orphos binary in place of pyrodigal and rebuilds every one of
  these rules from its GFF (`tools/gene_call.rs`).
- **Gene-caller `/partial` codes** (`0a8ba4c`), ported. On every GenBank read, a
  prodigal/GeneMark `/partial="10"` code becomes `<`/`>` markers and is removed. An end
  within 2 bases of the contig edge is extended to the edge, with `/codon_start` moved if
  the 5' end moved. Existing markers win, and a bare `/partial` is kept. See
  `seq/partial.rs` (`normalize_partial_codes`), called at the end of
  `io/genbank/parse.rs::parse_record`, the one place every GenBank read goes through.
  `pDONR201_multi_genemark*.gb` carry these codes, so many existing invocations now
  see partial CDSs.
- **Bare `/partial`** (`02a31c1`), ported. A CDS with the legacy `/partial` qualifier with
  no value, and no `<`/`>`, is a `?` fragment. This is one change to
  `seq/partial.rs::feature_fragment_status`, which every `--partial` filter, count and
  column reads. The `extra:test_bare_partial_qualifier_is_a_fragment` invocations cover it.
- **Version bump** (`0394886`): 0.12.0. The viewer shell was regenerated (only
  `DOMAINATOR_VERSION` changed).
- **Found while syncing** (pre-existing, now fixed):
  - GenBank header wrapping now follows Biopython's `_split_multi_line`, with 68 columns
    after the indent; it was 67.
  - A reverse-strand CDS ending at a contig's last base sorted first in `select-by-cds`.
  - `trim-contigs`/`clean-sequences --strip` kept header annotations Python's slice drops.
  - `extract-domains` let a `--domains` match skip `--evalue`/`--score`.
- **Vendored copy:** `external/domainator/` (gitignored; a git clone of upstream, kept at
  the pin above). It is the deterministic, repo-local source for regeneration and drift
  checks. Refresh it with `scripts/sync_upstream.sh` (fast-forwards to a target ref).

Advancing the pin is the **last** step of every sync (see the routine below). Update
`upstream_commit!()` in `crates/domainator-core/src/version.rs` in the same commit. If
upstream's `__version__` changed, update `upstream_version!()` as well. A unit test checks
the commit against the **Caught up to** line, and `sync_upstream.sh` checks the version.

## Sync-surface taxonomy

Only a few kinds of upstream change require action here. For each, the guard that keeps it
honest:

| # | Surface | What it is | Guard |
|---|---------|-----------|-------|
| 1 | **Literal vendored artifacts** | Files copied/rendered verbatim from Python. Today the **only** one is the SSN viewer HTML shell (`crates/domainator-core/assets/ssn_viewer_shell.html`), rendered from `ssn_viewer_html.py`. | Regenerated by `scripts/regen_ssn_viewer_shell.py`; `--check` mode is the drift guard. **Never hand-edit the asset.** |
| 2 | **On-disk format contracts** | GenBank, HDF5, `.didx`, shards, BGZF, `.dsnv` bundle, XGMML, TSV number formatting. | Byte/semantic **parity gates** (`external/domainator/test`, `test/helpers.py`). A format change requires Rust code changes in the owning module. |
| 3 | **Tool / flag / algorithm coverage** | New tools, flags, or modes (e.g. `mst_knn`, `efi_score`, treemap). | The `README.md` per-tool table is the coverage ledger; new behavior needs a port + a parity gate. |
| 4 | **Hand-ported logic tables** | Genetic code, PROSITE patterns, protein chemistry, taxonomy. | Fixture-free `cargo test`s against documented reference values (BioPython etc.). |

## Python file → Rust owner map

Use this to triage which module a changed upstream file lands in:

| Python (`src/domainator/…`) | Rust owner |
|---|---|
| `ssn_viewer_html.py` | `crates/domainator-core/assets/ssn_viewer_shell.html` (via `scripts/regen_ssn_viewer_shell.py`) |
| `docs/algorithms.md` | **reference, not ported** — read `external/domainator/docs/algorithms.md` for the SSN internals (MST, threshold tables, split-event scanning, the cap/window/pin selection, the moving sum, slider stops, dendrogram cutting). Its "Ports that must stay in step" table is the authority on which functions mirror which; this port is a third implementation of that same list. |
| `build_ssn_viewer.py`, `ssn_hierarchy.py` | `crates/domainator-core/src/tools/build_ssn_viewer.rs`, `.../matrix/ssn.rs` |
| `ssn_bundle.py` | `.../tools/ssn_bundle.rs` (the `.dsnv` reader) |
| `ssn_navigator.py` | `.../tools/ssn_navigator.rs` |
| `summary_report.py` | `.../tools/summary_report.rs` (data path only; HTML deferred) |
| `utils.py` palette helpers | `.../palette.rs` |
| `structure_*.py`, `structure_lib.py`, `foldseek.py` | **deferred** — see below |
| `build_ssn.py` | `.../tools/build_ssn.rs` |
| `ssn_edges.py`, `mst_knn` helpers | `.../matrix/ssn.rs`, `.../matrix/mst_knn.rs` |
| `transform_matrix.py`, `data_matrix.py` (HDF5/TSV) | `.../tools/transform_matrix.rs`, `.../matrix/{hdf5_io,text,transform}.rs` |
| `seq_dist.py` | `.../tools/seq_dist.rs` |
| `compare_contigs.py` | `.../tools/compare_contigs.rs` |
| `domainate.py` | `.../tools/domainate.rs`, `.../search/hmmer.rs` |
| `domainate.py` gene calling (`prodigal_CDS_annotate`, `_prodigal_linear_location`, `_prodigal_origin_genes`) | `.../tools/gene_call.rs` (drives the external Orphos binary; also the port-only `gene-call` tool) |
| `domain_search.py`, `select_by_cds.py` | `.../tools/{domain_search,select_by_cds,neighborhood}.rs` |
| `select_by_contig.py`, `enum_report.py` | `.../tools/{select_by_contig,enum_report,select_common}.rs` |
| `matrix_report.py` | `.../tools/matrix_report.rs` |
| `hmmer_compare.py` | `.../tools/hmmer_compare.rs`, `.../search/hmm_compare.rs` |
| GenBank/FASTA/BGZF/didx/shards utils | `.../io/{genbank,fasta,bgzf,didx,shards,partition}.rs` |
| `utils.py` protein/taxonomy helpers | `.../seq/{protein_analysis,taxonomy,taxid,translate}.rs` |
| `utils.py` fragment/partial helpers (`get_fragment_status`, `on_contig_translation`, `codon_offset`, ...) | `.../seq/partial.rs` |
| `utils.slice_record_from_location`, `slice_record`, `_truncate_cds_qualifiers`; `CompoundLocation.merge_parts` | `.../tools/neighborhood.rs` (`slice_features`), `.../seq/location.rs` (`merge_touching_parts`) |
| `extract_domains.py`, `extract_peptides.py`, `extract_unannotated.py` | `.../tools/{extract_domains,extract_peptides,extract_unannotated,extract_common}.rs` |

## The routine

Run `scripts/sync_upstream.sh` (see its header for options). It:

1. Fast-forwards `external/domainator/` to the target upstream ref.
2. Prints `git log --oneline --stat <pin>..<target>` — the changelog to triage against the
   map above.
3. Runs the automated guards it can: `regen_ssn_viewer_shell.py --check`, the `ssn_probe`
   example, the committed-fixture check, and the differential parity sweep (add per-tool
   parity gates for any tool whose Python file changed).
4. Reminds you to port whatever surface #2–#4 changes require, then **advance the pin** in
   this file (and note anything deliberately deferred).

The script does not decide *what* needs porting — a human triages each commit. It removes
the mechanical toil: what changed, where it lands, and which guards to run.

### Upstream's tests are a fifth sync surface

`scripts/parity/` replays every `main([...])` invocation in upstream's pytest suite through
both implementations and diffs the outputs; `scripts/parity/known_gaps.tsv` is the enforced
list of what does not yet reproduce, and `test/data/` is the committed fixture subset those
invocations reference. **`docs/parity.md` is the full workflow** — how to read the gap
list, how to triage a new row, how to migrate a finding into a native `cargo test`, and how
to widen the harvester when a newly interesting test is being skipped.

When the pin moves, the fixture check and the sweep are the guards that notice upstream
testing something the port does not do. The `git diff` on `known_gaps.tsv` is the artifact
worth reading: new rows are features to catch up to, and disappearing rows deserve equal
suspicion — upstream may have stopped exercising something rather than the port having
fixed it.

Its other guards are real comparisons, not smoke tests: `scripts/ssn_probe.py` prints the same
JSON document from `ssn_edges`/`ssn_hierarchy` that `examples/ssn_probe.rs` prints, and
`scripts/json_parity.py` diffs the parsed form. Compare **parsed** JSON, never bytes —
`serde_json` is built with `preserve_order` while Python writes `sort_keys=True`, so byte
equality is not part of any contract here.

## Deliberately deferred

Tracked so a later sync does not mistake them for oversights:

- **The structure subsystem** (`structure_to_genbank`, `structure_dist`, `structure_search`,
  `structure_domainate`, `structure_lib`). Genuinely portable — pure subprocess wrapping of
  `foldseek`/`reseek`, no torch or ESM — but out of scope for this sync. It touches ported
  code only through `SearchResult`'s four optional structural metrics, which **are** carried
  (`tmscore`/`lddt`/`rmsd`/`prob`), so structure-written GenBank stays round-trippable.
- **`merge_impact_axis_labels`** (`ssn_hierarchy.py`). Its only consumers are the deferred
  HTML report and the vendored viewer JS, which carries its own copy.
- **`mst_knn_edge_counts_by_threshold`'s reshape.** Upstream changed its rows to one per
  distinct threshold and flipped `include_equal` to `False`. It has no Rust surface today;
  when `matrix_report --include_mst_knn` is eventually ported, it must follow that shape.
- **`hmmer_build --window_beta`.** Deriving MAXL from a tail mass needs builder calibration
  `hmmer-pure-rs` does not expose; the flag is refused rather than silently ignored.
- **HTML output** for `enum_report`, `hmmer_report`, `matrix_report` and `summary_report`.
  `--json` is offered instead, which is upstream's own recommendation for agents.
  (`deferred:html` in `known_gaps.tsv`.)
- **The ward-clustered GenBank report** `compare_contigs -o`, and its `--mst_knn`
  projections. (`deferred:ward-report`.)
- **Infernal `.cm` references** in `domainate`/`domain_search`. (`deferred:infernal`.)
- **`--progress`** on `seq_dist` and `compare_contigs`. (`deferred:progress`.)
- **`color_genbank --color_domains` / `--color_both`.** (`deferred:color-domains`.)
- **`--partial` on `find_features` and `domainator_db_download`** (`bdc74bb`). Neither tool
  is ported. The GUI's managed UniProt download could take the same filter later.

## What the parity sweep still tracks

`scripts/parity/known_gaps.tsv` holds every upstream invocation that does not reproduce,
machine-checked on each sweep so none can be quietly forgotten. One open `gap:` row remains
(five invocations of it); everything else is a deliberate deferral or a dependency issue:

| class | rows | |
|---|---|---|
| `deferred:html` | 9 | `--html` for `enum_report`, `matrix_report`, `summary_report`; `--json` is offered instead |
| `gap:feature-order` | 5 | `domain_search` emits tied-coordinate features in the other order, below |
| `deferred:ward-report` | 4 | `compare_contigs -o`, the ward-clustered GenBank report |
| `deferred:progress` | 3 | `seq_dist` / `compare_contigs` `--progress`; intentionally not ported |
| `deferred:color-domains` | 3 | `color_genbank --color_domains` / `--color_both`; intentionally not ported |
| `deferred:infernal` | 2 | `.cm` references |
| `upstream:hmmer-pure-rs` | 1 | a `domainate --evalue 1e-40` hit that lands on the other side of the cutoff |

A `tolerated` row is one where the semantic comparator passed but the **bytes differ**, so
the gap list structurally cannot hold it. Diffing those bytes is the single most productive
thing to do with a green sweep: nine divergences were found that way and fixed — GenBank
qualifier wrapping, qualifier quoting, the LOCUS topology column, the missing creation-date
stamp, `/key` vs `/key=""`, the Pfam `GA`/`TC`/`NC` cutoffs, the `.hmm` model-matrix grid,
f64-vs-f32 score rounding, and cluster-table ordering. Byte-identical invocations went from
51 to 133 as a result (136 once the nucleotide single-sequence builder landed).

`gap:feature-order` (5 rows) is the residue of the nucleotide-FASTA gap described under
"Pinned dependency" below. All the hits now match — every coordinate and every
qualifier, `evalue` and `score` included, compare equal as a set — but where two features
share a start coordinate, `domain_search` emits `Domainator` before `Domain_Search` and
Python emits the reverse; one invocation also orders whole records differently. `domainate`
itself is byte-identical, so the reorder is in the slice path, not the search. It survived
this long because protein hits attach to distinct CDS coordinates and rarely tie.

## Pinned dependency: `hmmer-pure-rs`

**`Cargo.toml` pins `hmmer-pure-rs` to a commit on our fork,
[`seanrjohnson-neb/hmmer-pure-rs`](https://github.com/seanrjohnson-neb/hmmer-pure-rs/tree/aarch64-neon-pipeline)
(`99daaca00025e1ee17331b6b359596a90f1e409d`), not a published crates.io release.** The
commit is the official repo's `main` as of 2026-09-20 (`ef68ffe`, version 0.8.0) plus one
change of ours that is waiting to go upstream. `deny.toml` allows the fork as a git source.

Two fixes are in this pin, and the published 0.7.4 has neither:

- **Single-sequence nucleotide models.** 0.7.4 hardcodes BLOSUM62 in
  `build_single_seq_hmm`, so it panics on a 4-letter alphabet and cannot build a nucleotide
  model from one sequence. `search/references.rs` used to work around that with a one-row
  MSA, which is not what pyhmmer does (C's `p7_SingleBuilder`: a DNA1 score matrix plus the
  nucleotide window length nhmmer's E-values are computed against), and eight parity
  invocations diverged. Fixed in
  [henriksson-lab/hmmer-pure-rs#2](https://github.com/henriksson-lab/hmmer-pure-rs/pull/2),
  merged 2026-09-19 and part of `main`. With it our nucleotide hits match pyhmmer's
  coordinates, scores and E-values exactly.
- **aarch64 (Apple Silicon).** Every SIMD path in the crate was gated on `x86_64`. On ARM,
  MSV/Viterbi/Forward and domain definition fell back to the generic DP (hmmsearch ~25x
  slower), about one domain in eight landed a few residues away from the x86/pyhmmer
  envelope, and nhmmer's long-target SSV filter did not exist, so nucleotide searches
  found nothing (`search/nhmmer.rs` returned no hits there). The fork commit adds
  `hmmer_pure_rs::simd::sse`: `std::arch::x86_64` on x86, and the same SSE2 intrinsics
  implemented over NEON on aarch64, so ARM runs the same kernels in the same order.
  Verified under QEMU: `domainate` output byte-identical to x86, and all 536 of the
  crate's integration tests with the same outcome on both architectures. The PR branch is
  `aarch64-neon-pipeline` on the fork (open the PR from there).

Moving to 0.8 also brought one API change: `builder::build_hmm_from_msa` now returns
`Result`, failing as C's `hmmbuild` does when E-value calibration cannot fit
(`tools/hmmer_build.rs` propagates it).

**Re-pinning once the PR merges:** point the `[patch.crates-io]` entry back at
`https://github.com/henriksson-lab/hmmer-pure-rs` with `rev` = the merge commit (not
`main`), swap the fork for the official repo in `deny.toml`'s `allow-git`, `cargo update -p hmmer-pure-rs`, and run the
gates below.

**Migrating to a released version**, once one is published that includes both fixes
(0.7.4 was still the latest on crates.io when this was written; check `cargo info
hmmer-pure-rs`):

1. Delete the `hmmer-pure-rs` entry from `[patch.crates-io]` in `Cargo.toml` and its git
   source from `deny.toml`.
2. Make the `[workspace.dependencies]` requirement match the release (`"0.8"` now).
3. `cargo update -p hmmer-pure-rs`, then build, `cargo test`, a full differential sweep
   (the real gate: no new rows in `known_gaps.tsv`), and the aarch64 check
   (`domainate` under QEMU must match x86 byte for byte).
4. Delete this section.
