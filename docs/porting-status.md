# Porting status

This is the authoritative coverage ledger for the port: what has been reimplemented in
Rust, how faithful each tool is to its Python original, and how the tool is reached in
the desktop app.

**It is keyed by the upstream Python filename** (`domainate.py`, `seq_dist.py`), because
that is the unit the port tracks — not by the Rust subcommand a user types. A single
Python script usually maps to the same name with hyphens (`domainate.py` →
`domainator domainate`), but some rows describe scripts that were deliberately never
ported, and a few describe functionality that exists only in the GUI. For the
user-facing list of what you can actually run, see the
[command-line reference](cli-reference.md) and [the desktop app](gui.md).

Three counts appear in this project and they measure different things:

| Count | What it is |
| --- | --- |
| **34** | subcommands of the `domainator` executable (`domainator --help`), including the `inspect`, `convert` and `schema` dev/meta commands |
| **30** | tools with a `--config` schema (`domainator schema <tool>`) — all but those three |
| **28** | tools wired into the GUI's Tools menu |
| **41 rows below** | upstream Python scripts, ported *and* deliberately unported |

Related: [deliberate deferrals and the upstream pin](https://github.com/nebiolabs/domainatoredux/blob/main/UPSTREAM.md),
and [the parity harness](parity.md) that enforces the port's fidelity.

## Legend

GUI column: how each tool is reachable in the Tauri app (`apps/domainator-gui/`).
**Tools menu** = a schema-driven run dialog (the label in quotes); **Viewer** = a
file-type tab that renders/interacts with the data; **—** = not surfaced yet.

## The ledger

| executable | implementation status | GUI (Tauri app) |
| ------------- | ------------------- | ------------------- |
| build_projection.py | deferred | — |
| build_ssn.py | finished | Tools menu ("Build SSN (XGMML)") |
| build_ssn_viewer.py | finished (`.dsnv` bundle v6 — nodes + MST + hierarchy only, everything else derived by the reader; there is no `--max_merge_events`, the cap is a viewer control; `--categorical`) | Tools menu ("Build SSN viewer"); output `.dsnv` opens in the **SSN viewer** tab (interactive network, node→sequence cross-nav into a detached window). Layout algorithms: force-directed, grid, packed, and **treemap** (fixed-lattice, no edges). The shell also carries upstream's saved sessions, extractions, metadata editing, per-column charts and gradient colors. Viewer shell is generated from the Python source — see `UPSTREAM.md` |
| build_tree.py | deferred | — |
| clean_sequences.py | finished (byte-identical parity) | Tools menu ("Clean sequences") |
| color_genbank.py | finished (byte-identical parity incl. auto-color; `--color_domains`/`--color_both` intentionally not ported) | Tools menu ("Color GenBank") for batch color-by-name/database, **plus** per-feature editing in the contig viewer |
| color_table_to_legend.py | finished (byte-identical SVG) | Tools menu ("Color table → legend") — reads a `table` file (`.tsv`/`.csv`), writes SVG |
| compare_contigs.py | done (`-o` ward report and `--progress` intentionally not ported) | Tools menu ("Compare contigs") |
| deduplicate_genbank.py | finished: hash/cd-hit/diamond/usearch backends, byte-identical incl. the `--cluster_table` row order, which follows the clustering tool's own order rather than the record order | Tools menu ("Deduplicate") — hash backend needs nothing; cd-hit/usearch/diamond need their binary, all three **auto-downloadable** from the External tools panel |
| domain_search.py | protein (hmmsearch) + protein-FASTA (phmmer) + nucleotide-FASTA/HMM (nhmmer) references supported; whole-contig nhmmer hits flow through neighborhood extraction (default range, --add_annotations, and --whole_contig incl. circular rotation naming all at parity). Minor known gaps: protein input + --whole_contig/neighborhood emits whole proteins rather than erroring as Python does; and where two features share a start coordinate the slice path emits `Domainator` before `Domain_Search` where Python emits the reverse (`gap:feature-order`, 5 parity rows — the features themselves, coordinates and qualifiers included, all match). --pad at parity (verified against the committed domain_search_test_out2.gb fixture). Still deferred: CM/foldseek references, --gene_call (prodigal, GPL-deferred), --deduplicate | Tools menu ("Domain search") — incl. neighborhood extraction, taxonomy filters, and decoys; HMM references come from the folder or the Databases panel |
| domainate.py | protein (hmmsearch) + protein-FASTA (phmmer) + nucleotide-FASTA/HMM (nhmmer, whole-contig both-strand + circular origin-wrap) references supported; nucleotide-HMM & circular nhmmer are exact parity, phmmer is exact structural parity (evalue/score within HMMER-impl tolerance), and nucleotide-FASTA nhmmer is now exact parity too — byte-identical output, with coordinates, scores and E-values equal to pyhmmer's — via hmmer-pure-rs's single-sequence builder (see UPSTREAM.md, "Pinned dependency"). Still deferred: CM/foldseek references, --gene_call (prodigal, GPL-deferred), --batch_size/--offset/--recs_to_read | Tools menu ("Domainate") — incl. taxonomy filters |
| domainator_db_download.py | deferred (network fetch + prodigal --gene_call, which is GPL-deferred) | — |
| domainator_format_db.py | finished (shard/BGZF-compress/.didx-index; .didx byte-identical to Python; domain_search round-trip verified both ways. Supports plain + BGZF inputs; plain non-BGZF gzip rejected, matching the rest of the seekable-I/O layer) | not runnable as a tool; the **Databases panel** registers/classifies already-formatted DBs for use as references |
| enum_report.py | finished (`--json` is streaming NDJSON to its own path; html output deferred) | **Enum Report viewer** tab (row-virtualized sortable grid, per-column filters, column picker) + Tools menu export ("Enum report (export)", TSV) |
| extract_domains.py | finished (separate tool; parity-verified) | Tools menu ("Extract domains") |
| extract_peptides.py | finished (separate tool; byte-identical parity) | Tools menu ("Extract peptides") |
| extract_unannotated.py | finished (protein-only, matches Python) | Tools menu ("Extract unannotated") |
| filter_domains.py | finished (parity-verified) | Tools menu ("Filter domains") |
| find_features.py | deferred (too many torch dependencies) | — |
| genbank_to_fasta.py | finished (byte-identical parity) | Tools menu ("GenBank → FASTA") |
| hmmer_build.py | finished (Stockholm MSA -> HMM via hmmer-pure-rs build_hmm_from_msa; sets NAME/ACC/DESC; same NAME/LENG/DESC as Python and re-readable by pyhmmer. Limitations: Stockholm input only (no aligned-FASTA/Clustal yet); emissions not byte-identical to pyhmmer/hmmbuild — different builder/calibration) | Tools menu ("HMM build") — Stockholm `.sto`/`.stockholm`/`.sth` input (a GUI-only file type); output `.hmm` feeds the reference dropdowns |
| infernal_build.py | deferred (avoiding infernal dependency for now) | — |
| kmer_dist.py | deferred (heavy; low priority per plan) | — |
| hmmer_compare.py | finished | Tools menu ("HMM compare") — writes a scores/alignments table |
| hmmer_report.py | finished (TSV byte-identical parity; column order = flag order like enum_report; `--json` is streaming NDJSON; `--alphabet` column; HTML deferred) | Tools menu ("HMM report (export)") — a **column builder** (ordered list of source/acc/desc/length/consensus + `append` constant columns) writes TSV or JSON |
| hmmer_search.py | finished (filter query HMMs by best reference score; count+order parity verified across --score_cutoff/--max_hits; reuses the hmm_compare Viterbi engine + write_hmm) | Tools menu ("HMM search") — keeps query profiles scoring against a reference HMM set; output `.hmm` |
| hmmer_select.py | finished (byte-identical output incl. `--alphabet`, which selects whole input files and rejects a mixed-alphabet set; the port has its own `p7_hmmfile_WriteASCII` writer (`search/hmm_write.rs`) because hmmer-pure-rs's stamps a `HMMER3/e` header and drops the `GA`/`TC`/`NC` Pfam cutoffs) | Tools menu ("HMM select") — pairs with the HMM Logo viewer to curate a reference `.hmm` subset (regex/exact/substring on name/desc/acc) |
| matrix_report.py | finished (`--json` matches upstream's agent contract; a separate GUI payload feeds the viewer tab) | **Matrix Report viewer** tab (histogram + suggested cutoffs) + Tools menu export ("Matrix report (export)", text) |
| ssn_navigator.py | finished (all five `--mode`s verified against Python on both implementations' bundles, at three `--max_merge_events` caps, on v5 and v6 files) | — (the SSN viewer tab covers interactive exploration; the navigator writes stdout JSON with no workspace artifact) |
| partition_seqfile.py | deferred | — |
| select_by_cds.py | finished | Tools menu ("Select by CDS") — name/domain/`domain_expr`/`search_hits`/`unannotated` + neighborhood extraction + pad/dedup/strand/invert |
| select_by_coord.py | finished (parity-verified) | Tools menu ("Select by coordinate") — coordinate table is a `table` file (`.tsv`/`.csv`) |
| select_by_contig.py | finished | Tools menu ("Select by contig") — name/regex, domain content, taxonomy, length/CDS-count filters; `fasta_out` toggles FASTA vs GenBank |
| seq_dist.py | finished (`--progress` intentionally not ported) | Tools menu ("Sequence distance") — `diamond_*` algorithms need the `diamond` binary, **auto-downloadable** from the External tools panel |
| sort_contigs.py | finished (parity; ties may reorder) | Tools menu ("Sort contigs") |
| summary_report.py | finished: `--json` (upstream's agent contract) + text; HTML deferred (matplotlib/seaborn), as is the ASCII length histogram | **Summary dashboard** tab (pure-data stats + length histogram + taxonomy icicle) — GUI-only, sidesteps the matplotlib CLI tool |
| split_by_cluster.py | deferred (optional; skipped for now) | — |
| transform_matrix.py | finished | Tools menu ("Transform matrix") |
| plot_contigs.py | superseded on screen by the GUI contig viewer; no CLI equivalent and no static image export yet | **Contig viewer** — the GUI centerpiece (SVG, viewport virtualization + LOD, db/e-value filters, tooltips, editing); already supersedes the Python plotter for on-screen use (static image export still TODO) |
| trim_contigs.py | finished (parity-verified) | Tools menu ("Trim contigs") |
| domainator_server | deferred (tauri target is the alternative) | — (the Tauri app *is* the alternative) |
