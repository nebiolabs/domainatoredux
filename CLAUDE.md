# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

A from-scratch **pure-Rust** rewrite of `domainator`, a Python bioinformatics suite for protein-domain / genome-neighborhood analysis and building sequence similarity networks (SSNs). Goals (see `README.md`): cross-platform (incl. Windows), a single `domainator <tool> [opts]` executable, a later Tauri GUI, and easier agentic integration.

**Hard requirement:** on-disk compatibility with files produced by the original Python tool — GenBank, HDF5 matrices, and formatted databases (shards / BGZF / `.didx` indexes). Compatibility is **semantic** (re-readable by the original, equal under the Python suite's comparators), *not* byte-identical.

## Build / run

```bash
cargo build                      # debug build of the workspace
cargo build --release            # optimized (lto); use for any timing/benchmarks
cargo build --workspace 2>&1 | grep -c warning:   # keep this at 0
./target/debug/domainator <tool> [opts]           # run a tool
```

Tools: `inspect`, `convert` (dev/round-trip), `domainate`, `domain-search`, `seq-dist`, `compare-contigs`, `hmmer-compare`. CLI flags use the Python suite's **underscore** names (e.g. `--max_overlap`, `--dense_text`, `--whole_contig`, `-Z`/`--z`), set explicitly via clap `long = "..."` — don't assume clap's kebab default.

Dev/diagnostic examples live in `crates/domainator-core/examples/` (`offsets`, `didx_tool`, `shards_tool`, `matrix_probe`, ...): `cargo run -p domainator-core --example matrix_probe`.

## Verification model (important — there is no committed test suite)

Correctness is checked by **parity against the original Python tool**, not by `cargo test`. The reference lives at:
- Python env: `/home/sean/miniforge3/envs/domainator/bin/python`
- Vendored Python source: `external/domainator/` (gitignored; identical to `/home/sean/scripts/python/domainator`), with fixtures under `external/domainator/test/data/` (or the equivalent path in the conda checkout) and helpers in `test/helpers.py`.

Typical gate: run the Rust tool and the Python tool on the same fixture, then compare in the conda env:
- **GenBank**: `helpers.compare_seqfiles` / `compare_seqrecords`. These compare `{seq, id, description, name}`, annotations (iterating the **first** record's keys, skipping `date`), feature **count**, and per-feature **qualifiers** — *not* feature coordinates/types. Pass `skip_qualifiers={'evalue','score','identity'}` for search output (see numeric tolerance below).
- **HDF5/TSV matrices**: `DataMatrix.from_file(...).toarray()` + `np.allclose` and label/`data_type` checks.
- Records often come back in a **different order** than Python (Rust preserves partition/input order; Python uses a process pool), so compare as a **set keyed by id**, not positionally.

Search numerics need **not** match pyhmmer exactly — `hmmer-pure-rs` is a different HMMER3 implementation. Compare structure (which domains hit which CDS, coordinates, which edges exist) and apply tolerance to `evalue`/`score`/`identity` and matrix values. In practice most gates land at exact equality anyway.

## Architecture

Virtual Cargo workspace, two crates:
- **`crates/domainator-core/`** — the data model, all I/O, and all tool logic as library functions (GUI-reusable; mirrors the Python convention that every tool is also a library function).
- **`crates/domainator-cli/`** — thin clap front-end; arg parsing + I/O wiring only, calls into core.

Core module map:
- `seq/` — `SeqRecord`, `SeqFeature`, `Annotations`, and the **lean `Location`** model (`parts: Vec<(start,end,strand,before,after)>`, 0-based half-open, complement-joins pre-reversed). `translate.rs` (genetic code, revcomp, location extraction), `taxid.rs`.
- `io/` — own GenBank `parse`/`reader`/`writer`/`offsets`, `fasta`, `bgzf` (random access + virtual offsets), `didx` (`.didx` index), `shards`, `partition`.
- `search/` — `hmmer` (hmmsearch for domainate), `seq_compare` (phmmer/hmmsearch scoring), `hmm_compare` (profile-vs-profile Viterbi), `results` (`SearchResult` → GenBank feature).
- `matrix/` — `DataMatrix` (dense/CSR) + `hdf5_io`, `text` (TSV), `transform` (`--mode`s).
- `tools/` — `domainate`, `domain_search`, `seq_dist`, `compare_contigs`, `hmmer_compare`, `neighborhood` (select_by_cds).
- `parallel.rs` — `with_threads(cpu, f)` wrapper (`cpu == 0` → all cores). Tools parallelize with rayon over their natural unit (profiles / references / partitions / rows); output is **deterministic** regardless of thread count (results funnel through order-independent maps/sorts).

Key cross-cutting invariants (these gate parity — grep the modules for details before changing):
- **No Biopython id/name swap in memory:** `name` = LOCUS name, `id` = accession.version. BUT the original Python GenBank *parser* swaps them, so when a gate re-reads output with `parse_seqfiles`, LOCUS→`id` and VERSION→`name`. Consequence: a coordinate suffix or marker that the Python tool appends to `record.id` must be written to our **`name`** (LOCUS line); likewise per-record qualifier values like `source_contig` use the LOCUS name (our `name`).
- **Ordered qualifiers** (`Vec<(String, Vec<String>)>`) — insertion order is load-bearing for the writer and for `cds_id` / `domainator_<db>` rollups.
- **GenBank writer** wraps qualifier lines at word boundaries only (`break_long_words=False`) — breaking a long no-space token makes the reader rejoin with a spurious space.
- **Number formatting** (`%.1e` via a custom `py_sci`, `%.1f`, NumPy `str(float64)` for TSV) is replicated to match Python output exactly.
- **`.didx` / shard resolution / BGZF offsets** are hard on-disk contracts (`io/didx.rs`, `io/shards.rs`, `io/bgzf.rs`): `.didx` is byte-compatible; staleness is rejected via size+mtime_ns; both unsharded + shards present is an error (refuse to guess).
- **Parser must never reject a record** (handles odd LOCUS layouts, missing molecule unit, `*` stop codons).

### Vendored `hdf5-pure` (patched dependency)

`vendor/hdf5-pure/` is a local copy of `hdf5-pure` 0.20.0 wired via `[patch.crates-io]` in the root `Cargo.toml`. It carries one additive patch: `AttrValue::VarLenUtf8(String)` — a scalar variable-length UTF-8 string attribute so h5py reads the matrix attrs (`ARRAY_TYPE`/`DATA_TYPE`/`MATRIX_FILE_VERSION`) back as `str` (the published crate only offers fixed-length strings, which h5py returns as `bytes`). See `vendor/hdf5-pure/CHANGELOG.md`. Matrix `SYMMETRIC_LABELS` is written as an int (h5py reads it truthily).

## Scope / status

The approved plan is at `/home/sean/.claude/plans/make-a-plan-for-kind-brooks.md`. The prioritized SSN pipeline (README steps 1–3) is complete and Python-verified: GenBank/FASTA I/O, `domainate`, `domain_search` (shards/BGZF/`.didx`, `-Z 0`, neighborhood extraction for linear **and** circular contigs, `--translate`, strand filter, overlap dedup), `seq_dist` (hmmsearch / phmmer / hmmer_compare + dense/sparse/TSV), `compare_contigs` (JI/AI), the standalone `hmmer-compare` (with alignments), and rayon multithreading.

Deferred (per plan / no clean path): DIAMOND (`seq_dist --algorithm diamond_*`; no Rust crate — `diamond-rs` is GitHub-only/early), the ward-clustered GenBank report (`compare_contigs -o`) + `--mst_knn`, gene-calling (`prodigal-rs`), Infernal/Foldseek/ESM, and the Tauri GUI (README step 4).
