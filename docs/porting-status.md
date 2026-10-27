# Porting status

This is the authoritative coverage ledger for the port: what has been reimplemented in
Rust, how faithful each tool is to its Python original, and how the tool is reached in
the desktop app.

**It is keyed by upstream's tool name**, as `domainator describe` lists them. Since 1.0
the port's subcommands have the same names (`domainator select_by_cds` →
`domainatoredux select_by_cds`), but some rows describe tools that were deliberately
never ported, and a few describe functionality that exists only in the GUI. For the
user-facing list of what you can actually run, see the
[command-line reference](cli-reference.md) and [the desktop app](gui.md).

Three counts appear in this project and they measure different things:

| Count | What it is |
| --- | --- |
| **38** | subcommands of the `domainatoredux` executable (`domainatoredux --help`): 35 tools, including the `inspect` and `convert` dev commands, plus the `describe`, `completion` and `doctor` builtins |
| **33** | tools in the manifest (`domainatoredux describe`), each with a `--config` file: all but `inspect` and `convert` |
| **30** | tools wired into the GUI's Tools menu: the manifest's, less `format_db`, `ssn_navigator` and `summary_report`, which the app covers with panels and viewers |
| **44 rows below** | upstream's tools (`domainator describe`), ported *and* deliberately unported |

Related: [deliberate deferrals and the upstream pin](https://github.com/nebiolabs/domainatoredux/blob/main/UPSTREAM.md),
and [the parity harness](parity.md) that enforces the port's fidelity.

Partial features and fragments (upstream 0.12.0) are ported across every tool that has
them. That covers marker-preserving slicing with `<`/`>` on cut ends, `/codon_start`
handling, `--partial` on the eight tools that take it, and `enum_report`'s `fragment` and
`partial_count` columns. They are checked by the parity sweep's `extra:` invocations; see
[File formats](file-formats.md#partial-features-and-fragments). `find_features` and
`db_download` gained `--partial` upstream, but neither tool is ported.

Upstream 1.0's interface is ported for every tool in the table: the same tool and option
names, explicit `--<content>_out` outputs, interchangeable `--config` files, and
`describe`. The options a ported tool still lacks are the rows of
`scripts/parity/manifest_allowlist.tsv` (classed `deferred:`); a test fails if the
manifest and that list disagree, so this table cannot claim more than the CLI has. See
[Parity](parity.md#the-interface-describe-against-upstreams-manifest).

Gene calling (`--gene_call all|unannotated` on `domainate` and `domain_search`) is ported
by driving the external, GPL [Orphos](https://github.com/FullHuman/orphos) binary (a Rust
Prodigal) where upstream uses pyrodigal in-process; see `docs/architecture.md`, "External
binaries". Its 13 `extra:` parity invocations are byte-identical, and a stress comparison
of `prodigal_CDS_annotate` on random genomes (circular and linear, N runs, contigs down to
45 bp) matched all 889 genes. The same code is also exposed as **`gene_call`**, a tool
with no upstream counterpart (so it has no row below) that adds Orphos's single mode, a
translation table, `--closed` and `--mask`; it is in the GUI's Tools menu as "Gene call".

## Legend

GUI column: how each tool is reachable in the Tauri app (`apps/domainatoredux-gui/`).
**Tools menu** = a run dialog built from the tool's manifest entry (the label in quotes); **Viewer** = a
file-type tab that renders/interacts with the data; **—** = not surfaced yet.

## The ledger

| upstream tool | implementation status | GUI (Tauri app) |
| ------------- | ------------------- | ------------------- |
| build_projection | deferred | — |
| build_ssn | finished | Tools menu ("Build SSN (XGMML)") |
| build_ssn_viewer | finished (`.dsnv` bundle v6 — nodes + MST + hierarchy only, everything else derived by the reader; there is no `--max_merge_events`, the cap is a viewer control; `--categorical`) | Tools menu ("Build SSN viewer"); output `.dsnv` opens in the **SSN viewer** tab (interactive network, node→sequence cross-nav into a detached window). Layout algorithms: force-directed, grid, packed, and **treemap** (fixed-lattice, no edges). The shell also carries upstream's saved sessions, extractions, metadata editing, per-column charts and gradient colors. Viewer shell is generated from the Python source — see `UPSTREAM.md` |
| build_tree | deferred | — |
| clean_sequences | finished (byte-identical parity) | Tools menu ("Clean sequences") |
| color_genbank | finished (byte-identical parity incl. auto-color) | Tools menu ("Color GenBank") for batch color-by-name/database, **plus** per-feature editing in the contig viewer |
| color_table_to_legend | finished (byte-identical SVG) | Tools menu ("Color table → legend") — reads a `table` file (`.tsv`/`.csv`), writes SVG |
| compare_contigs | finished, including the sorted `--genbank_out` (scipy's `linkage`/`optimal_leaf_ordering` ported in `matrix/hierarchy.rs`, order byte-identical); `--progress` accepted, no bar drawn | Tools menu ("Compare contigs") |
| db_download | deferred (network fetch; its `--gene_call` could now reuse `tools/gene_call.rs`) | — |
| deduplicate_genbank | finished: hash/cd-hit/diamond/usearch backends, byte-identical incl. the `--cluster_tsv_out` row order, which follows the clustering tool's own order rather than the record order | Tools menu ("Deduplicate") — hash backend needs nothing; cd-hit/usearch/diamond need their binary, all three **auto-downloadable** from the External tools panel |
| domain_search | protein (hmmsearch) + protein-FASTA (phmmer) + nucleotide-FASTA/HMM (nhmmer) references supported; whole-contig nhmmer hits flow through neighborhood extraction (default range, --add_annotations, and --whole_contig incl. circular rotation naming all at parity). Minor known gap: where two features share a start coordinate the slice path emits `Domainator` before `Domain_Search` where Python emits the reverse (`gap:feature-order`, 5 parity rows — the features themselves, coordinates and qualifiers included, all match). --pad at parity (verified against the committed domain_search_test_out2.gb fixture). --gene_call ported (external Orphos binary; byte-identical). `--deduplicate`, `--deduplicate_by_sequence`/`--both_strands`, `--anchor` and circular `--pad` at parity. Still deferred: CM references | Tools menu ("Domain search") — incl. neighborhood extraction, taxonomy filters, and decoys; HMM references come from the folder or the Databases panel |
| domainate | protein (hmmsearch) + protein-FASTA (phmmer) + nucleotide-FASTA/HMM (nhmmer, whole-contig both-strand + circular origin-wrap) references supported; nucleotide-HMM & circular nhmmer are exact parity, phmmer is exact structural parity (evalue/score within HMMER-impl tolerance), and nucleotide-FASTA nhmmer is now exact parity too — byte-identical output, with coordinates, scores and E-values equal to pyhmmer's — via hmmer-pure-rs's single-sequence builder (see UPSTREAM.md, "Pinned dependency"). --gene_call ported (external Orphos binary; byte-identical). `--max_output_gb` checked per record as upstream does; `--batch_size` accepted and ignored (all records are searched at once). Still deferred: CM/foldseek references | Tools menu ("Domainate") — incl. taxonomy filters |
| enum_report | finished (`--json_out` is streaming NDJSON to its own path; `--html_out` is upstream's generated table page, byte-identical) | **Enum Report viewer** tab (row-virtualized sortable grid, per-column filters, the export's column builder, taxonomy database picker) + Tools menu export ("Enum report (export)", TSV): a column builder over every column type, parameterized and taxonomy ones included, with taxonomy from the managed NCBI Taxonomy download |
| extract_domains | finished (separate tool; parity-verified) | Tools menu ("Extract domains") |
| extract_peptides | finished (separate tool; byte-identical parity) | Tools menu ("Extract peptides") |
| extract_unannotated | finished (protein-only, matches Python) | Tools menu ("Extract unannotated") |
| filter_domains | finished (parity-verified) | Tools menu ("Filter domains") |
| find_features | deferred (too many torch dependencies) | — |
| format_db | finished (shard/BGZF-compress/.didx-index; .didx byte-identical to Python; domain_search round-trip verified both ways. Supports plain + BGZF inputs; plain non-BGZF gzip rejected, matching the rest of the seekable-I/O layer) | not runnable as a tool; the **Databases panel** registers/classifies already-formatted DBs for use as references |
| genbank_to_fasta | finished (byte-identical parity) | Tools menu ("GenBank → FASTA") |
| hmmer_build | finished (gzip/BGZF `.hmm` read by magic bytes, `.gz` output as plain gzip, `.bgz` refused; Stockholm MSA -> HMM via hmmer-pure-rs build_hmm_from_msa; sets NAME/ACC/DESC; same NAME/LENG/DESC as Python and re-readable by pyhmmer. Limitations: Stockholm and aligned-FASTA input only (no Clustal/A2M/PHYLIP); emissions not byte-identical to pyhmmer/hmmbuild — different builder/calibration) | Tools menu ("HMM build") — Stockholm `.sto`/`.stockholm`/`.sth` input (a GUI-only file type); output `.hmm` feeds the reference dropdowns |
| hmmer_compare | finished | Tools menu ("HMM compare") — writes a scores/alignments table |
| hmmer_report | finished (TSV byte-identical parity; column order = flag order like enum_report; `--json_out` is streaming NDJSON; `--alphabet` column; `--html_out` is upstream's generated table page, byte-identical) | Tools menu ("HMM report (export)") — a **column builder** (ordered list of source/acc/desc/length/consensus/alphabet + `append` constant columns) writes TSV or JSON |
| hmmer_search | finished (gzip/BGZF `.hmm` read by magic bytes, `.gz` output as plain gzip, `.bgz` refused; filter query HMMs by best reference score; count+order parity verified across --score_cutoff/--max_hits; reuses the hmm_compare Viterbi engine + write_hmm) | Tools menu ("HMM search") — keeps query profiles scoring against a reference HMM set; output `.hmm` |
| hmmer_select | finished (gzip/BGZF `.hmm` read by magic bytes, `.gz` output as plain gzip, `.bgz` refused; byte-identical output incl. `--alphabet`, which selects whole input files and rejects a mixed-alphabet set; the port has its own `p7_hmmfile_WriteASCII` writer (`search/hmm_write.rs`) because hmmer-pure-rs's stamps a `HMMER3/e` header and drops the `GA`/`TC`/`NC` Pfam cutoffs) | Tools menu ("HMM select") — pairs with the HMM Logo viewer to curate a reference `.hmm` subset (regex/exact/substring on name/desc/acc) |
| infernal_build | deferred (avoiding infernal dependency for now) | — |
| kmer_dist | deferred (heavy; low priority per plan) | — |
| matrix_report | finished (text, `--json_out` (upstream's agent contract) and `--html_out` (upstream's generated page) all match upstream, `--knn_projection` included; a separate GUI payload feeds the viewer tab) | **Matrix Report viewer** tab (histogram + suggested cutoffs) + Tools menu export ("Matrix report (export)", text) |
| plot_contigs | superseded on screen by the GUI contig viewer; no CLI equivalent and no static image export yet | **Contig viewer** — the GUI centerpiece (SVG, viewport virtualization + LOD, db/e-value filters, tooltips, editing); already supersedes the Python plotter for on-screen use (static image export still TODO) |
| rotate_contigs | finished (parity-verified; Infernal `.cm` references deferred, as for domainate) | Tools menu ("Rotate contigs") |
| select_by_cds | finished | Tools menu ("Select by CDS") — name/domain/`domain_expr`/`annotation_type`/`unannotated` + neighborhood extraction + pad/dedup/strand/invert |
| select_by_contig | finished | Tools menu ("Select by contig") — name/regex, domain content, taxonomy, length/CDS-count filters; GenBank and/or FASTA output |
| select_by_coord | finished (parity-verified) | Tools menu ("Select by coordinate") — coordinate table is a `table` file (`.tsv`/`.csv`) |
| seq_dist | finished, `--symmetrize` included (`--progress` accepted, no bar drawn; the explained diamond failures are not ported, and the large-search advisory has no free-disk-space note) | Tools menu ("Sequence distance") — `diamond_*` algorithms need the `diamond` binary, **auto-downloadable** from the External tools panel |
| sort_contigs | finished (byte-identical parity; ties keep input order) | Tools menu ("Sort contigs") |
| split_by_cluster | deferred (optional; skipped for now) | — |
| ssn_navigator | finished (all five `--query` modes verified against Python on both implementations' bundles, at three `--max_merge_events` caps, on v5 and v6 files) | — (the SSN viewer tab covers interactive exploration; the navigator writes stdout JSON with no workspace artifact) |
| structure_dist | deferred (portable: foldseek/reseek subprocesses, no torch; `SearchResult` already carries the structure metrics, so their GenBank output round-trips) | — |
| structure_domainate | deferred (portable: foldseek/reseek subprocesses, no torch; `SearchResult` already carries the structure metrics, so their GenBank output round-trips) | — |
| structure_search | deferred (portable: foldseek/reseek subprocesses, no torch; `SearchResult` already carries the structure metrics, so their GenBank output round-trips) | — |
| structure_to_genbank | deferred (portable: foldseek/reseek subprocesses, no torch; `SearchResult` already carries the structure metrics, so their GenBank output round-trips) | — |
| summary_report | finished: `--json_out` (upstream's agent contract), `--html_out` (upstream's generated page) + text; the text report's ASCII length histogram and pandas table layout are not reproduced | **Summary dashboard** tab (pure-data stats + length histogram + taxonomy icicle) — GUI-only, sidesteps the matplotlib CLI tool |
| transform_matrix | finished | Tools menu ("Transform matrix") |
| trim_contigs | finished (parity-verified) | Tools menu ("Trim contigs") |
