# Architecture

A map of the codebase, for people changing it. See
[Developing Domainator](development.md) for how to build and verify.

## Workspace layout

A virtual Cargo workspace: three library/binary crates, two Tauri apps, one WASM app.

| Crate | Role |
| --- | --- |
| `crates/domainator-core/` | the data model, all I/O, and every tool's logic as library functions. Also `app_support/`, the GUI-facing seam. |
| `crates/domainator-config/` | per-tool serde config structs, `merge_cli`, `--print_config`, the draft-07 schema emitter, and `resolve.rs` (config → `*Options`), shared by the CLI **and** the GUI so the two cannot drift. |
| `crates/domainator-cli/` | thin clap front end — arg parsing and I/O wiring only. Binary name `domainatoredux`. |
| `apps/domainator-gui/` | the Tauri v2 + Svelte 5 desktop app. Its Rust side is a thin `#[tauri::command]` layer over `app_support`. |
| `apps/genbank-summary/`, `wasm_apps/genbank-summary/` | a small standalone Tauri app and its browser/WASM twin. |
| `vendor/hdf5-pure/` | a patched dependency (below). |

Every tool is a library function as well as a subcommand, mirroring the Python suite's
convention, which is what makes the GUI able to reuse them directly.

## Core module map

- **`seq/`** — `SeqRecord`, `SeqFeature`, `Annotations`, and the lean `Location` model
  (`parts: Vec<(start, end, strand, before, after)>`, 0-based half-open, complement-joins
  pre-reversed). Plus `translate.rs` (genetic code, revcomp, location extraction),
  `taxid.rs`, `taxonomy.rs` (taxdmp parser, lineage/rank/name, filter and expression
  compilation) and `protein_analysis.rs` (isoelectric point, net charge, molecular weight,
  PROSITE motif counting).
- **`io/`** — the project's own GenBank `parse`/`reader`/`writer`/`offsets`, plus `fasta`,
  `bgzf` (random access and virtual offsets), `didx`, `shards` and `partition`.
- **`search/`** — `hmmer` (hmmsearch for `domainate`), `seq_compare` (phmmer/hmmsearch
  scoring), `hmm_compare` (profile-vs-profile Viterbi), `results`
  (`SearchResult` → GenBank feature), and the project's own `hmm_read`/`hmm_write`.
- **`matrix/`** — `DataMatrix` (dense and CSR) with `hdf5_io` and `text`, `transform`,
  `mst_knn`, `cytoscape` (XGMML) and `ssn` (the SSN engine: `MaxTree`, merge events,
  component hierarchy, threshold tables).
- **`tools/`** — one module per tool, plus shared pieces: `neighborhood` (the
  `select_by_cds` geometry, reused by `domain_search`'s hit path), `select_common`
  (`DomainatorCds`, `BooleanEvaluator`, name-list parsing, the `--databases` rollup filter)
  and `external.rs` (the `find_binary` locator for subprocess tools).
- **`app_support/`** — the GUI seam: workspace file classification, the provenance/job DAG,
  contig view and edit, HMM logo, summary statistics.
- **`parallel.rs`** — `with_threads(cpu, f)` (`cpu == 0` → all cores). Tools parallelize
  with rayon over their natural unit (profiles, references, partitions, rows), and output
  is **deterministic** regardless of thread count, because results funnel through
  order-independent maps and sorts.

## External binaries

The only non-pure-Rust runtime dependencies, all optional, all driven as subprocesses
through `tools/external.rs::find_binary` (env var → bundled `external/<dir>/<name>` →
`PATH`): **`diamond`** for `seq_dist` and `deduplicate_genbank`, **`cdhit`** and **`usearch`**
for `deduplicate_genbank`, and **`orphos`** for gene calling (`tools/gene_call.rs`: the `gene_call`
tool and `domainate`/`domain_search --gene_call`). Their locator identity lives in
`deduplicate_genbank`'s and `gene_call`'s `*_BIN_NAME` / `*_ENV_VAR` constants, to which
the GUI's External tools registry is pinned by a test.

cd-hit, usearch and Orphos are GPL. Driving them across a process boundary, downloaded at
runtime and never bundled, is what keeps this project's licensing unaffected — **do not
link or vendor any of them.** That rules out the `orphos-core` library crate, and every
other Prodigal implementation (`prodigal-rs`, pyrodigal), for the same reason.

Orphos is run only for its GFF output (`-f gff`): its GenBank output has no strand or
partial markers, it has no protein output, and its `-t` training file is a no-op.
Everything upstream's `prodigal_CDS_annotate` derives from a pyrodigal gene is rebuilt
from the GFF instead: the edge-extended partial location and its `/codon_start`, the
translation (in the table each sequence's model reports, so meta mode's table-4 models
come out right), and the genes crossing a circular contig's origin, found by a second
sequence in the same run (the origin window). Sequences go to Orphos named by index, never
by their own ids.

## Cross-cutting invariants

These gate parity. Grep the owning module before changing any of them.

- **Sequence identifiers:** `id` is the LOCUS name for GenBank and the first header
  word for FASTA. `name` retains GenBank's VERSION (or ACCESSION) only for round-tripping;
  user-facing labels, coordinate suffixes, matrix axes, and metadata keys use `id`.
  This matches the Python tool's LOCUS-based identifiers without depending on its
  Biopython id/name swap.
- **Ordered qualifiers** (`Vec<(String, Vec<String>)>`) — insertion order is load-bearing
  for the writer and for the `cds_id` / `domainator_<db>` rollups.
- **`/translation` is the only qualifier that wraps.** Upstream patched its vendored
  Biopython so `/translation` wraps at 80 columns and every other qualifier is written on
  one line however long. Wrapping the rest still reads back equal, which is why parity
  gates never caught it, and it is what made breaking a long no-space token hazardous.
- **A record with no `date` is stamped with today's date**, not Biopython's
  `01-JAN-1980` fallback (ours in UTC, Python's local, so they differ only across local
  midnight; every comparator skips `date`).
- **The HMM writer emits all five annotation columns on every match line**, `-` for the
  ones a profile lacks — MAP included, whose column is present even under `MAP no`, because
  C's reader consumes that token unconditionally. Omitting it shifts the remaining columns
  left and makes the file unreadable. Every fixture is `MAP yes`, so no parity gate covers
  this; the `hmmer_select` round-trip tests are the guard.
- **Number formatting** (`%.1e` via a custom `py_sci`, `%.1f`, NumPy `str(float64)` for
  TSV) is replicated to match Python's output exactly.
- **`.didx` / shard resolution / BGZF offsets** are hard on-disk contracts: `.didx` is
  byte-compatible, staleness is rejected via size + mtime, and both an unsharded file and
  shards present is an error — refuse to guess.
- **The parser must never reject a record** (odd LOCUS layouts, missing molecule unit, `*`
  stop codons).
- **A forward window that wraps the origin keeps its parts in rotation order.** Sorting
  ascending yields the same arc rotated to a different origin — a silently wrong sequence —
  and collapses the coordinate suffix to "whole contig". A *reverse* wrapped window is the
  mirror image and does want ascending order.
- **Slicing across the origin remaps into every overlapping slice part, then merges.** One
  feature part can reach into both parts of a wrapped slice; each contributes a piece, and
  the pieces are then coalesced (same-strand, touching inclusively) so a junction cut
  becomes one range rather than a spurious `join`. Merging is gated on the original
  operator, so `order` locations keep their parts apart.
- **`--max_hits` ranks by the focus CDS's own hit score** — the `Domain_Search` feature
  only, never the `Domainator` annotations. An extracted region spans neighbors, so
  ranking by the maximum over all features would score a weak hit by whatever strong gene
  sits next to it.
- **Coordinate suffixes come from the human-readable stranded properties**, not a
  hand-rolled `start + 1 : end` — the naive version is the forward rule and is off by one
  in each direction on a reverse slice.
- **The `domainator_<db>` rollup is regenerated by the writer** from the CDS's
  `Domainator` features *in current feature order*, and only then are features sorted for
  output. So `slice_record` must not pre-sort its features, and `domainate` appends domains
  in score-descending order. This is why the pass-through tools reproduce the original's
  rollup string exactly.
- **Threshold tables carry one row per distinct threshold**, each describing exactly the
  graph `build_ssn --threshold T` keeps (edges scoring *strictly above* T), with a trailing
  synthetic `threshold = 0` row when every score is positive. Keying rows by MST edge
  position instead was an upstream bug.

`serde_json` is built with **`preserve_order`** workspace-wide, so JSON object key order is
insertion order. Python writes with `sort_keys=True`, which is why parity compares *parsed*
JSON and byte equality is not a contract there.

## Vendored `hdf5-pure`

`vendor/hdf5-pure/` is a local copy of `hdf5-pure` 0.20.0 wired in via `[patch.crates-io]`,
carrying two additive patches (see its `CHANGELOG.md`):

1. `AttrValue::VarLenUtf8(String)` — a scalar variable-length UTF-8 string attribute, so
   h5py reads the matrix attributes back as `str`. The published crate only offers
   fixed-length strings, which h5py returns as `bytes`.
2. Enum attributes decode to their underlying integer instead of being silently dropped
   from `attrs()` — h5py stores a numpy-bool attribute (like `SYMMETRIC_LABELS`) as an enum
   over `int8`.

Matrix symmetry is determined on read by the **presence of `COL_LABELS`**, which is the
actual on-disk contract and is robust even when the `SYMMETRIC_LABELS` attribute is absent.

## `hmmer-pure-rs` pinned to a commit

`hmmer-pure-rs` is pinned by `rev` to a commit on our fork,
`seanrjohnson-neb/hmmer-pure-rs`, rather than a published crate. The pinned commit is the
official repo's `main` (0.8.0, which includes `henriksson-lab/hmmer-pure-rs#2`, the
single-sequence nucleotide builder the nhmmer FASTA path needs to match pyhmmer) plus a
change, offered upstream, that runs the crate's SSE kernels on aarch64 through a NEON
implementation of the SSE2 intrinsics. Without it an Apple Silicon build takes the generic
DP everywhere: it is far slower, lands some domain envelopes a few residues away from the
x86 and pyhmmer results, and cannot run nhmmer's long-target filter at all.

**Don't "clean up" the pin and don't move it to a branch head** — a later commit would
silently change CI. `UPSTREAM.md` has the re-pin and migration-back steps.
