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
| **50** | subcommands of the `domainatoredux` executable (`domainatoredux --help`): 47 tools, including the `inspect` and `convert` dev commands, plus the `describe`, `completion` and `doctor` builtins |
| **45** | tools in the manifest (`domainatoredux describe`), each with a `--config` file: all but `inspect` and `convert` |
| **41** | tools wired into the GUI's Tools menu: the manifest's, less `format_db`, `ssn_navigator`, `summary_report` and `viewer`, which the app covers with panels and viewers |
| **44 rows below** | upstream's tools (`domainator describe`), ported *and* deliberately unported |

Related: [deliberate deferrals and the upstream pin](https://github.com/nebiolabs/domainatoredux/blob/main/UPSTREAM.md),
and [the parity harness](parity.md) that enforces the port's fidelity.

Partial features and fragments (upstream 0.12.0) are ported across every tool that has
them. That covers marker-preserving slicing with `<`/`>` on cut ends, `/codon_start`
handling, `--partial` on the eight tools that take it, and `enum_report`'s `fragment` and
`partial_count` columns. They are checked by the parity sweep's `extra:` invocations; see
[File formats](file-formats.md#partial-features-and-fragments), and `db_download`'s
`--partial` by its own invocations.

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
| build_projection | partial: `pca` and `coords` (scikit-learn's `PCA`, or `TruncatedSVD` for a mostly-empty matrix, as an exact SVD with `svd_flip`'s signs, via `faer`; coordinates within 1e-3 of upstream's, whose sparse solver is randomized). `tsne` and `umap` are refused (stochastic; deferred) | Tools menu ("Build projection (PCA)"); output `.dvb` opens in the **Viewer** tab |
| build_ssn | finished, including the Domainator Viewer outputs (`--html_out`, `--dvb_out`) | Tools menu ("Build SSN"); output `.dvb` opens in the **Viewer** tab (network, node→sequence extraction). Layout algorithms: force-directed, grid, packed, and **treemap**. The viewer also carries upstream's saved sessions, extractions, metadata editing, per-column charts and gradient colors. It is generated from the Python source — see `UPSTREAM.md` |
| build_tree | finished: UPGMA (scipy's `average` linkage, `matrix/hierarchy.rs`) or a Newick tree read as Biopython reads it; Newick, rectangular-tree XGMML and the viewer bundle with `--msa` byte-identical or parse-equal | Tools menu ("Build tree"); output `.dvb` opens in the **Viewer** tab, `--msa` takes aligned FASTA from the workspace, which also opens in the **Alignment** tab |
| clean_sequences | finished (byte-identical parity) | Tools menu ("Clean sequences") |
| color_genbank | finished (byte-identical parity incl. auto-color) | Tools menu ("Color GenBank") for batch color-by-name/database, **plus** per-feature editing in the contig viewer |
| color_table_to_legend | finished (byte-identical SVG) | Tools menu ("Color table → legend") — reads a `table` file (`.tsv`/`.csv`), writes SVG |
| compare_contigs | finished, including the sorted `--genbank_out` (scipy's `linkage`/`optimal_leaf_ordering` ported in `matrix/hierarchy.rs`, order byte-identical); `--progress` draws a bar on a terminal | Tools menu ("Compare contigs") |
| db_download | finished: UniProt as FASTA or as GenBank (the SwissProt flat-file parser, `io/swissprot.rs`, is a port of Biopython's, byte-identical to upstream on 92,000 SwissProt and TrEMBL entries), NCBI genomes over HTTPS (`direct`) or the NCBI Datasets CLI (`datasets`), every database's selection, every filter, `--gene_call` through Orphos. Every harvested invocation is byte-identical; they replay offline against the local mirrors in `test/data/db_download/` (`--uniprot_dir`, and an `--ncbi_summary` whose `ftp_path` names local directories) | Tools menu ("Download a database"); the **Databases** panel's managed UniProt, Pfam and taxonomy downloads are separate |
| deduplicate_genbank | finished: hash/cd-hit/diamond/usearch backends, byte-identical incl. the `--cluster_tsv_out` row order, which follows the clustering tool's own order rather than the record order | Tools menu ("Deduplicate") — hash backend needs nothing; cd-hit/usearch/diamond need their binary, all three **auto-downloadable** from the External tools panel |
| domain_search | protein (hmmsearch) + protein-FASTA (phmmer) + nucleotide-FASTA/HMM (nhmmer) references supported; whole-contig nhmmer hits flow through neighborhood extraction (default range, --add_annotations, and --whole_contig incl. circular rotation naming all at parity). --pad at parity (verified against the committed domain_search_test_out2.gb fixture). --gene_call ported (external Orphos binary; byte-identical). `--dedup` (all four policies, with and without `--max_hits`), per-protein/CDS/hit taxonomy filters with `--taxonomy_unknown`, `--anchor` and circular `--pad` at parity. Infernal `.cm` references via infernox, as for domainate. Every harvested invocation is byte-identical, including the whole-contig nucleotide and `.cm` ones; `--cds_range` counts protein-coding CDSs only, never nucleotide hits, as upstream does | Tools menu ("Domain search") — incl. neighborhood extraction, taxonomy filters, and decoys; HMM references come from the folder or the Databases panel |
| domainate | protein (hmmsearch) + protein-FASTA (phmmer) + nucleotide-FASTA/HMM (nhmmer, whole-contig both-strand + circular origin-wrap) references supported; nucleotide-HMM & circular nhmmer are exact parity, phmmer is exact structural parity (evalue/score within HMMER-impl tolerance), and nucleotide-FASTA nhmmer is now exact parity too — byte-identical output, with coordinates, scores and E-values equal to pyhmmer's — via hmmer-pure-rs's single-sequence builder (see UPSTREAM.md, "Pinned dependency"). --gene_call ported (external Orphos binary; byte-identical). `--max_output_gb` checked per record as upstream does; `--batch_size` accepted and ignored (all records are searched at once). Infernal `.cm` references (whole-contig `cmsearch`, truncated hits included) via the infernox crate, a byte-parity port of Infernal 1.1.5: byte-identical output, every hit's coordinates, score and E-value equal to pyinfernal's. Still deferred: foldseek references. Taxonomy filters, `--hits_only` and `--no_annotations` left with upstream 1.0 (`domain_search` selects) | Tools menu ("Domainate") |
| enum_report | finished (`--json_out` is streaming NDJSON to its own path; `--html_out` is upstream's generated table page, byte-identical) | **Enum Report viewer** tab (row-virtualized sortable grid, per-column filters, the export's column builder, taxonomy database picker) + Tools menu export ("Enum report (export)", TSV): a column builder over every column type, parameterized and taxonomy ones included, with taxonomy from the managed NCBI Taxonomy download |
| extract_domains | finished (separate tool; parity-verified, `--has_search_hit` and taxonomy filters included) | Tools menu ("Extract domains") |
| extract_peptides | finished (separate tool; byte-identical parity, `--domain_expr`, `--has_search_hit` and taxonomy filters included) | Tools menu ("Extract peptides") |
| extract_unannotated | finished (protein-only, matches Python, taxonomy filters included) | Tools menu ("Extract unannotated") |
| filter_domains | finished (parity-verified) | Tools menu ("Filter domains") |
| find_features | partial: `--motif` (PROSITE patterns), `--gene_call` and `--partial`, byte-identical on 9 `extra:` invocations (upstream's own tests build their inputs or loop over values). `--tmbed` is refused: TMbed needs torch and a protein language model; its options are accepted so upstream configs load | Tools menu ("Find features (motifs)") |
| format_db | finished (shard/BGZF-compress/.didx-index; .didx byte-identical to Python; domain_search round-trip verified both ways. Supports plain + BGZF inputs; plain non-BGZF gzip rejected, matching the rest of the seekable-I/O layer) | not runnable as a tool; the **Databases panel** registers/classifies already-formatted DBs for use as references |
| genbank_to_fasta | finished (byte-identical parity) | Tools menu ("GenBank → FASTA") |
| hmmer_build | finished (gzip/BGZF `.hmm` read by magic bytes, `.gz` output as plain gzip, `.bgz` refused; every hmmbuild MSA format, guessed as Easel guesses it (`io/msa_formats.rs`: Clustal, PHYLIP, SELEX and PSI-BLAST read rule for rule as Easel reads them; Stockholm, aligned FASTA and A2M by hmmer-pure-rs) -> HMM via hmmer-pure-rs build_hmm_from_msa; sets NAME/ACC/DESC; profile equal to Python's apart from DATE/COM in every format; on about 1 random alignment in 13 one COMPO or STATS value differs in its last printed digit, a builder rounding near-tie) | Tools menu ("HMM build") — Stockholm `.sto`/`.stockholm`/`.sth` (a GUI-only file type) or aligned FASTA `.afa` input; output `.hmm` feeds the reference dropdowns |
| hmmer_compare | finished | Tools menu ("HMM compare") — writes a scores/alignments table |
| hmmer_report | finished (TSV byte-identical parity; column order = flag order like enum_report; `--json_out` is streaming NDJSON; `--alphabet` column; `--html_out` is upstream's generated table page, byte-identical) | Tools menu ("HMM report (export)") — a **column builder** (ordered list of source/acc/desc/length/consensus/alphabet + `append` constant columns) writes TSV or JSON |
| hmmer_search | finished (gzip/BGZF `.hmm` read by magic bytes, `.gz` output as plain gzip, `.bgz` refused; filter query HMMs by best reference score; count+order parity verified across --score_cutoff/--max_hits; reuses the hmm_compare Viterbi engine + write_hmm) | Tools menu ("HMM search") — keeps query profiles scoring against a reference HMM set; output `.hmm` |
| hmmer_select | finished (gzip/BGZF `.hmm` read by magic bytes, `.gz` output as plain gzip, `.bgz` refused; byte-identical output incl. `--alphabet`, which selects whole input files and rejects a mixed-alphabet set; the port has its own `p7_hmmfile_WriteASCII` writer (`search/hmm_write.rs`) because hmmer-pure-rs's stamps a `HMMER3/e` header and drops the `GA`/`TC`/`NC` Pfam cutoffs) | Tools menu ("HMM select") — pairs with the HMM Logo viewer to curate a reference `.hmm` subset (regex/exact/substring on name/desc/acc) |
| infernal_build | finished: runs an external `cmbuild` (C Infernal, or infernox's `infernox-cmbuild`) as upstream does, then sets NAME/ACC/DESC; output equal to Python's apart from the DATE and COM lines | Tools menu ("CM build") — Stockholm input; output `.cm` feeds the reference dropdowns |
| kmer_dist | finished: exact k-mer sets (protein, translated-CDS with `--gene_call`, and strand-invariant `--nucleotide_kmers`), all three metrics, `--max_hits_per_query`, `--mst`/`--knn` with the streaming accumulator's tie order; values equal to Python's on all 44 harvested invocations that replay (the other 6 are upstream's error cases, or build their input at run time) | Tools menu ("K-mer distance") |
| matrix_report | finished (text, `--json_out` (upstream's agent contract) and `--html_out` (upstream's generated page) all match upstream, `--knn_projection` included; a separate GUI payload feeds the viewer tab) | **Matrix Report viewer** tab (histogram + suggested cutoffs) + Tools menu export ("Matrix report (export)", text) |
| plot_contigs | finished (`--html_out` is upstream's generated page; its data match upstream's on every harvested invocation) | Tools menu ("Plot contigs (HTML)") writes the page; the **Contig viewer** — the GUI centerpiece (SVG, viewport virtualization + LOD, db/e-value filters, tooltips, editing) — is the on-screen equivalent |
| rotate_contigs | finished (parity-verified, Infernal `.cm` references included) | Tools menu ("Rotate contigs") |
| select_by_cds | finished (upstream 1.0's selection contract: `--has_search_hit`, `--name_field`, `--dedup`, taxonomy filters on the focus CDS) | Tools menu ("Select by CDS") — name/domain/`domain_expr`/`annotation_type`/`unannotated`/search hits + neighborhood extraction + pad/dedup/strand/invert/taxonomy |
| select_by_contig | finished | Tools menu ("Select by contig") — name/regex, domain content, search hits, taxonomy, length/CDS-count filters; GenBank and/or FASTA output |
| select_by_coord | finished (parity-verified) | Tools menu ("Select by coordinate") — coordinate table is a `table` file (`.tsv`/`.csv`) |
| seq_dist | finished, `--symmetrize` included (`--progress` draws a bar of the queries on a terminal); a failed diamond run is explained as upstream explains it (scratch disk or memory exhausted, a rejected `--diamond_args` option, unreadable input), and the large-search advisory notes the scratch filesystem's free space | Tools menu ("Sequence distance") — `diamond_*` algorithms need the `diamond` binary, **auto-downloadable** from the External tools panel |
| sort_contigs | finished (byte-identical parity; ties keep input order) | Tools menu ("Sort contigs") |
| split_by_cluster | finished: GenBank, FASTA, `.hmm` (plain or gzip) and `.cm` inputs, `--pad_on_search_hits`; byte-identical on all 8 harvested invocations and 3 `extra:` ones (gzip HMM, BGZF GenBank, a two-model CM) | Tools menu ("Split by cluster"), writing a folder of files |
| ssn_navigator | finished (all five `--query` modes verified against Python on both implementations' bundles, at three `--max_merge_events` caps, on `.dvb` and v5/v6 `.dsnv` files) | — (the SSN viewer tab covers interactive exploration; the navigator writes stdout JSON with no workspace artifact) |
| structure_dist | finished: every score-based and structural `--mode`, `--symmetrize`, `--max_hits_per_query`, and `--mst`/`--knn` with the streaming accumulator's tie order; matrices equal to Python's on all 37 harvested invocations that replay, and `--hits_tsv_out` the same rows (foldseek writes them in thread order) | Tools menu ("Structure distance") |
| structure_domainate | finished: foldseek and reseek backends, `--metrics`, `--alignment_type`, `--max_seqs`, prebuilt and `--db_out` databases; byte-identical GenBank on every harvested invocation that replays, with foldseek and reseek; the rest are upstream's own error cases, which both refuse, or second steps that read an earlier step's database | Tools menu ("Structure domainate") |
| structure_search | finished: hits grouped by target (sorted stably on the target's bytes, as upstream's `LC_ALL=C sort -s`), `--max_hits` best first, `--add_annotations`, reseek's sequences from its database; byte-identical GenBank on every harvested invocation that replays, with foldseek and reseek; the rest are upstream's own error cases, which both refuse, or second steps that read an earlier step's database | Tools menu ("Structure search") |
| structure_to_genbank | finished: files, directories, globs and prebuilt databases, `--store_3di`, `--db_out`; byte-identical GenBank on every harvested invocation that replays, with foldseek and reseek; the rest are upstream's own error cases, which both refuse, or second steps that read an earlier step's database | Tools menu ("Structures to GenBank") |
| summary_report | finished: `--json_out` (upstream's agent contract), `--html_out` (upstream's generated page) and the `-o` text report, all as upstream writes them | **Summary dashboard** tab (pure-data stats + length histogram + taxonomy icicle) — GUI-only, separate from the CLI tool's page |
| transform_matrix | finished | Tools menu ("Transform matrix") |
| trim_contigs | finished (parity-verified) | Tools menu ("Trim contigs") |
| viewer | finished (byte-identical outside the embedded bundle, apart from the GUI host hook; reads `.dvb` and `.dsnv`) | — (the **Viewer** tab's **Save as HTML page** writes the same page) |
