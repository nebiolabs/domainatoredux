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

Tools: `inspect`, `convert` (dev/round-trip), `domainate`, `domain-search`, `seq-dist`, `compare-contigs`, `hmmer-compare`, `transform-matrix`, `build-ssn`, `build-ssn-viewer`, `matrix-report`, `select-by-contig`, `select-by-cds`, `enum-report`, plus `schema <tool>`. CLI flags use the Python suite's **underscore** names (e.g. `--max_overlap`, `--dense_text`, `--whole_contig`, `-Z`/`--z`), set explicitly via clap `long = "..."` — don't assume clap's kebab default. (`enum-report`'s args are parsed by hand in the CLI because column order follows flag order, which clap derive does not preserve.)

**Config files / agent interface** (`crates/domainator-cli/src/config.rs`): **every tool** (`domainate`, `domain-search`, `seq-dist`, `compare-contigs`, `hmmer-compare`, `transform-matrix`, `build-ssn`, `build-ssn-viewer`, `matrix-report`, `select-by-contig`, `select-by-cds`, `enum-report`) accepts `--config <file.json|.yml>` (a **native Rust** serde schema, *not* byte-compatible with Python's jsonargparse files) and `--print_config` (dump resolved params as YAML). Merge order is config = base, CLI flags override (scalars replace, lists replace when non-empty, bools OR, `enum-report` columns append). `domainator schema <tool>` emits a draft-07 JSON Schema for any tool. `enum-report` also has `--json` for a JSON array of typed row objects instead of TSV. To make merge correct, **defaulted scalar and required-path flags became `Option` in clap** (defaults applied via `unwrap_or` after merge; required paths like `-i`/`-o`/`-r` are validated post-merge so they can come from the config file); `enum-report`'s columns serialize as an ordered list of `{"type": "...", ...}` specs (a cleaner fit than the flag-order CLI).

Dev/diagnostic examples live in `crates/domainator-core/examples/` (`offsets`, `didx_tool`, `shards_tool`, `matrix_probe`, ...): `cargo run -p domainator-core --example matrix_probe`.

## Verification model (parity-first; a small native test suite exists)

End-to-end correctness is checked by **parity against the original Python tool** — that is the primary gate. There is also a small committed `cargo test` suite (`#[cfg(test)] mod tests` in the relevant modules) covering the **pure, deterministic, fixture-free** logic: `BooleanEvaluator`, `protein_analysis` (MW / pI / charge / PROSITE) against BioPython's documented values, `NCBITaxonomy` (inline mini-taxdmp), and the circular-neighborhood/range math. It complements parity (it needs no conda env); it does not replace it. The reference lives at:
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
- `seq/` — `SeqRecord`, `SeqFeature`, `Annotations`, and the **lean `Location`** model (`parts: Vec<(start,end,strand,before,after)>`, 0-based half-open, complement-joins pre-reversed). `translate.rs` (genetic code, revcomp, location extraction), `taxid.rs`, `taxonomy.rs` (`NCBITaxonomy` taxdmp parser + lineage/rank/name + `compile_taxonomy_filter`/`compile_taxonomy_expression`; reads extracted `.dmp` or `taxdump.tar.gz`), `protein_analysis.rs` (ProtParam isoelectric point / net charge, `molecular_weight`, PROSITE `motif_count` via the `regex` crate).
- `io/` — own GenBank `parse`/`reader`/`writer`/`offsets`, `fasta`, `bgzf` (random access + virtual offsets), `didx` (`.didx` index), `shards`, `partition`.
- `search/` — `hmmer` (hmmsearch for domainate), `seq_compare` (phmmer/hmmsearch scoring), `hmm_compare` (profile-vs-profile Viterbi), `results` (`SearchResult` → GenBank feature).
- `matrix/` — `DataMatrix` (dense/CSR) + `hdf5_io` (read+write; symmetry inferred from `COL_LABELS` **presence** on read — the actual on-disk contract, robust even if the `SYMMETRIC_LABELS` attr is absent. The vendored `hdf5-pure` was also patched to decode enum attributes so h5py's numpy-bool `SYMMETRIC_LABELS` is no longer silently dropped — see `vendor/hdf5-pure/CHANGELOG.md`), `text` (TSV read+write), `transform` (`--mode`s + `apply_lower_bound`), `mst_knn` (MST∪kNN pruner; shares `UnionFind` and `mst_knn_kept` with `ssn`), `cytoscape` (XGMML writer), and `ssn` (the SSN engine: `MaxTree` + `component_size_summary_by_threshold`/merge events + `build_mst_component_hierarchy` + `edge_scores_desc`/`mst_knn_edge_list`, mirroring `ssn_edges`/`ssn_hierarchy`). `read_matrix(path)` sniffs HDF5-vs-TSV by extension like Python's `DataMatrix.from_file`. MST edge ordering matches the Python **dense** `sorted_undirected_edges` fast path (score desc, ties by row-major index desc).
- `tools/` — `domainate`, `domain_search`, `seq_dist`, `compare_contigs`, `hmmer_compare`, `transform_matrix` (mode/lb/mst_knn + dense↔sparse conversion), `build_ssn` (XGMML SSN: default/`--mst`/`--mst_knn` edges, connected-component clustering, TSV metadata, colouring), `build_ssn_viewer` (the O(n) gzip-JSON `.ssnv` bundle = MST + hierarchy + merge events, plus the vendored HTML shell in `crates/domainator-core/assets/ssn_viewer_shell.html` filled via two string tokens), `matrix_report` (text report + agent-friendly `--json`; `--include_mst_knn`/HTML deferred), `neighborhood` (the `select_by_cds` geometry, shared by `domain_search`'s hit path and the standalone `select_by_cds`), `select_by_contig`, `select_by_cds`, `enum_report`, and `select_common` (shared `DomainatorCds.list_from_contig`, `BooleanEvaluator`, `get_sources`/`get_non_domainator_features`, name-list parsing, the `--databases` rollup filter). `seq_dist`'s `diamond_*` algorithms shell out to the external **`diamond`** binary (the only non-pure-Rust runtime dependency): build a db from the reference, then `blastp --outfmt 6 qseqid sseqid bitscore --algo 0 --max-hsps 1`, with the hit table streamed line-by-line into the per-cell max map (never a temp output file). The binary is located via `$DOMAINATOR_DIAMOND`/`$DIAMOND`, then a bundled `external/diamond/diamond` (searched upward from the exe, then cwd), then bare `diamond` on `PATH`. The normalized query/db FASTA + db live in a self-cleaning temp dir.
- `parallel.rs` — `with_threads(cpu, f)` wrapper (`cpu == 0` → all cores). Tools parallelize with rayon over their natural unit (profiles / references / partitions / rows); output is **deterministic** regardless of thread count (results funnel through order-independent maps/sorts).

Key cross-cutting invariants (these gate parity — grep the modules for details before changing):
- **No Biopython id/name swap in memory:** `name` = LOCUS name, `id` = accession.version. BUT the original Python GenBank *parser* swaps them, so when a gate re-reads output with `parse_seqfiles`, LOCUS→`id` and VERSION→`name`. Consequence: a coordinate suffix or marker that the Python tool appends to `record.id` must be written to our **`name`** (LOCUS line); likewise per-record qualifier values like `source_contig` use the LOCUS name (our `name`).
- **Ordered qualifiers** (`Vec<(String, Vec<String>)>`) — insertion order is load-bearing for the writer and for `cds_id` / `domainator_<db>` rollups.
- **GenBank writer** wraps qualifier lines at word boundaries only (`break_long_words=False`) — breaking a long no-space token makes the reader rejoin with a spurious space.
- **Number formatting** (`%.1e` via a custom `py_sci`, `%.1f`, NumPy `str(float64)` for TSV) is replicated to match Python output exactly.
- **`.didx` / shard resolution / BGZF offsets** are hard on-disk contracts (`io/didx.rs`, `io/shards.rs`, `io/bgzf.rs`): `.didx` is byte-compatible; staleness is rejected via size+mtime_ns; both unsharded + shards present is an error (refuse to guess).
- **Parser must never reject a record** (handles odd LOCUS layouts, missing molecule unit, `*` stop codons).
- **GenBank DEFINITION**: the reader strips a single trailing `.` (Biopython does), the writer re-appends it — so `record.description` matches `parse_seqfiles` and round-trips.
- **`domainator_<db>` rollup is regenerated by the writer** (`write_record`) from the CDS's `Domainator` features *in current feature order*, then features are sorted by `location.start` for output — mirroring `utils.write_genbank` (rollup built before its sort). `slice_record` therefore must **not** pre-sort its features (it preserves the input order so the rollup ordering matches), and `domainate` appends domains in score-descending order. This is why pass-through tools (`select_by_*`) reproduce the original's rollup string.

### Vendored `hdf5-pure` (patched dependency)

`vendor/hdf5-pure/` is a local copy of `hdf5-pure` 0.20.0 wired via `[patch.crates-io]` in the root `Cargo.toml`. It carries two additive patches (see `vendor/hdf5-pure/CHANGELOG.md`): (1) `AttrValue::VarLenUtf8(String)` — a scalar variable-length UTF-8 string attribute so h5py reads the matrix attrs (`ARRAY_TYPE`/`DATA_TYPE`/`MATRIX_FILE_VERSION`) back as `str` (the published crate only offers fixed-length strings, which h5py returns as `bytes`); and (2) enum attributes decode to their underlying integer instead of being silently dropped from `attrs()` — h5py stores a numpy-bool attribute (like `SYMMETRIC_LABELS`) as an enum over `int8`. Matrix `SYMMETRIC_LABELS` is written as an int (h5py reads it truthily); on read, symmetry is determined by `COL_LABELS` presence (the on-disk contract), independent of that attr.

## Scope / status

The approved plan is at `/home/sean/.claude/plans/make-a-plan-for-kind-brooks.md`. The prioritized SSN pipeline (README steps 1–3) is complete and Python-verified: GenBank/FASTA I/O, `domainate`, `domain_search` (shards/BGZF/`.didx`, `-Z 0`, neighborhood extraction for linear **and** circular contigs, `--translate`, strand filter, overlap dedup), `seq_dist` (DIAMOND `diamond_*` via the external binary — now the default `--algorithm`, exact-parity-verified against the Python tool when both use the same diamond version; plus hmmsearch / phmmer / hmmer_compare + dense/sparse/TSV), `compare_contigs` (JI/AI), the standalone `hmmer-compare` (with alignments), and rayon multithreading.

Also complete and Python-verified: `select_by_contig`, `select_by_cds` (standalone CDS selection by name/domain/`--domain_expr`/`--search_hits`/`--unannotated`, neighborhood extraction reusing `neighborhood.rs`, `--pad`/dedup/`--strand`/`--invert`/`--include_nucleic_acids`), and `enum_report` (TSV; all `--by contig|cds|domain` columns including taxonomy, protein chemistry, and PROSITE motif). Parity gates: `select_by_contig` 21/21, `select_by_cds` 27/27, `enum_report` 27/27 (full `compare_seqfiles` / exact TSV equality).

**SSN building & viewing (README step 4 data path)** — complete and Python-verified against `FeSOD_dist.{dense,sparse}.hdf5`/`.tsv`: `transform_matrix` (all modes incl. `efi_score`, dense↔sparse, `--mst_knn`; `np.allclose` + byte-identical `dense_text`), `build_ssn` (XGMML parses equal — node/edge/att sets — for default/`--mst`/`--mst_knn`/`--cluster`/metadata, byte-identical `cluster_tsv`), `build_ssn_viewer` (`.ssnv` bundle field-for-field identical after gunzip; embed + standalone HTML shells render the vendored upstream viewer), and `matrix_report` (`--json` numerically identical to Python's report payload + suggested cutoffs; text report structurally equivalent). The MST/hierarchy/merge-event core (`matrix/ssn.rs`) is cross-checked against `ssn_edges`/`ssn_hierarchy` via `cargo run -p domainator-core --example ssn_probe`.

Deferred (per plan / no clean path): the ward-clustered GenBank report (`compare_contigs -o`) + `--mst_knn`, gene-calling (`prodigal-rs`), Infernal/Foldseek/ESM, the Tauri GUI (README step 4 UI), `enum_report`'s HTML output (`--html`; TSV only), and `matrix_report`'s `--include_mst_knn` projections + interactive HTML.
