# Staying in sync with the Python `domainator`

This Rust workspace is a rewrite of the Python `domainator`
(<https://github.com/nebiolabs/domainator>). Correctness is defined by **parity** with
that tool, so as the Python package evolves we periodically pull its changes across. This
file records how far the port is caught up and how to advance it.

## Upstream pin

- **Caught up to:** `0aa79fe` — *foldseek pLM annotation better parallelism*
  (2026-11-03; upstream `__version__` still 1.0.0-rc.2, two commits past tag
  `v1.0.0-rc.2`, on `main`).
- **`67bad08..0aa79fe`**, ported. `c807588` rewords `compare_contigs`' help and its sort
  memory error to name `--genbank_out` instead of the 0.x `-o`. It also changes the
  `-o/--hdf_out` extension error, which now names the path and, for `compare_contigs`,
  says that 0.x's `-o` is now `--genbank_out` (`output::check_hdf_out`). `0aa79fe` builds
  `domainate --foldseek`'s query database, and so runs ProstT5, once per batch instead of
  once per database (`foldseek::build_query_db`/`search_db`). It was made in both at once,
  and output is unchanged.
- **`d3ef00e..67bad08`** (release candidates 1 and 2), ported: `doctor`'s GPU section
  (`c6f1b54`). It lists the GPUs `nvidia-smi` finds and the CUDA version their driver
  supports, then says what this build can use. Upstream's PyTorch line is the build's
  candle backend here: CPU only, CUDA with the number of devices it can open, or Metal.
  The `--esm2_3di_device` user is left out because that model is refused here. The
  rest needs nothing ported: the version bump (`UPSTREAM_VERSION`, and so the
  regenerated viewer shell's `domainatorVersion`), docs, packaging metadata,
  `scripts/release_check.sh`, and a test that now detects a GPU without PyTorch.
- **`d3ef00e`**, ported: `domainate --prostt5_weights`/`--prostt5_device`, foldseek's
  built-in ProstT5 as the `--foldseek` 3Di predictor (`search/foldseek.rs`; ESM-2 3B 3Di
  stays refused, see "Deliberately deferred"), and TMbed test fixtures
  (`test/data/tmbed/`) with tests that pin TMbed's predictions, which the port's
  reimplementation reproduces byte for byte (see "TMbed").
- **`fb840ec`**, ported with `hmmer_build`'s MSA formats (`io/msa_formats.rs`):
  `hmmer_build` opens its input MSA by path, and copies a stream (stdin) to a temporary
  file with no suffix first. pyhmmer's `MSAFile` on a file object segfaulted on a blank or
  unguessable file and failed on SELEX, and since a file object has no name the suffix
  rules that select A2M (`.a2m`), sequential PHYLIP (`.phys`) and PSI-BLAST (`.pb`) never
  applied. Adds one alignment in every format, plus hand-made PHYLIP, SELEX and Clustal
  cases, under `test/data/msa_formats/`.
- **`6c9945a`**, ported with `db_download`: `--uniprot_dir` reads UniProt from a local
  mirror and an `--ncbi_summary` whose `ftp_path` is a local directory reads genomes from
  disk, so the tests run offline on `test/data/db_download/` and the sweep replays them.
  NCBI genomes are written by each worker to a temporary file and appended by the parent
  in summary order (they used to be appended by the workers as they finished). Also
  fixes UniProt `--append` (it overwrote), `ncbi_nonredundant_all` without a taxonomy
  filter (it failed), and `ncbi_representative_all` keeping `refseq_category == na`
  genomes (it now drops them, as its docstring says).
- **`fc10e7b`**, ported with the structure tools: the by-target hit sort runs
  `LC_ALL=C sort -s` (byte order, stable; `structure_lib.sort_by_column`), so it no longer
  depends on the machine's locale, and reseek databases are built with `-threads 1`, whose
  record order is the input order (a multithreaded `-convert` stores chains as threads
  finish). Also moves two `domain_search` tests' gzip/BGZF copies out of `shared_datadir`.
- **`8c4821f`**, nothing to port: `domain_search --cpu N` writes its records in input
  order (`_mp.imap_bounded`, ordered, replaces `imap_unordered_bounded`; partitions go
  through `pool.imap`), so which duplicate `--dedup` keeps no longer depends on timing.
  The port was already deterministic. Also commits `kmer_dist`'s noncanonical-letter input
  as `test/data/kmer_dist_noncanonical.faa`, so that invocation replays, and points a
  stale `test_dedup` patch at `_domainate`.
- **`3b4d76e`**, ported: a taxonomy directory holding a full set of extracted `.dmp`
  files counts as a database and is no longer downloaded over (`taxonomy::has_extracted_dmps`;
  upstream's own extracted-only test was fetching the real taxdump). Also drops a dead
  `except ImportError` around TMbed: a missing TMbed was already an error with an install
  hint, as a missing ProtT5 model is here (it is downloaded when `network` is built in).
- **`f346a05`**, ported: `split_by_cluster` splits a real Infernal `.cm` file, each CM
  block kept with the filter HMM that follows it under the same name (it used to report
  duplicate labels), and `find_features --gene_call all` drops the `Domainator`
  annotations of the CDSs it replaces, as `domainate` does. Its `split_by_cluster` tests
  read committed matrices (`test/data/split_by_cluster/`) instead of writing them at run
  time, so the sweep replays them.
- **`aa6ef74`**, ported: `--cds_range` counts protein-coding CDSs only
  (`select_by_cds.cds_range_members`, `neighborhood::cds_range_members`); a nucleotide hit
  can be the focus but is never a neighbor. Also ignores the taxonomy loader's
  `*.tar.gz.lock` files. See "What the parity sweep still tracks".
- **`cfa4193`**, nothing to port: a browser test finds the Bacteria node by label, and
  the empty `test/data/taxdmp/taxdump.tar.gz.lock` (a lock file the taxonomy loader leaves
  beside the tarball) is deleted.
- **`bc91aca`**, ported: NCBI dropped `superkingdom`, so `summary_report`'s
  `TREE_KEPT_RANKS` keeps `cellular root` and `acellular root` instead
  (`tools/summary_report.rs`), and the `taxid:`/`taxname:` column help (`columns.rs`)
  says `domain`. The `taxdmp` fixtures follow NCBI's current ranks, as do the inline
  taxdumps of our own tests.
- **`b2e219c`, `aa7078c`, `3ee1b6a`, `ea5fd76`**, viewer: tree rooting (midpoint, on a
  selection, restore) and child swapping, ordered node and label styles with
  categorical shapes, and image export of the whole view. All of it is the page's own
  JS, so the regenerated `assets/viewer_shell.html` is the port, its GUI host hook
  unchanged. The `.dvb` version is 2 (`viewer_bundle.rs`), which only adds optional
  saved-session fields; version 1 is still read.
- **`ab6af3e`**, help text only, ported into the args structs: `domainate` keeps every
  record, `domain_search` selects, `extract_domains` keeps the input's sequence type.
- **`7e48c69`**, interface only: `--contigs`/`--domains` and their `_file` twins are
  upstream's shared `add_list_and_file` pair, as `parse_name_list` already read them.
- **`49a0953`**, renames: `--keep_name` is `--keep_parent_id` on the three extract
  tools, and `extract_unannotated`'s `--min_score`/`--min_length` are
  `--min_annotation_score`/`--min_region_length` (`rotate_contigs` keeps `--keep_name`).
- **`6fef445`, `664c70d`**, ported: the shared selection contract.
  `--annotation_type domainator|domain_search|all` now only says whose names the
  domain selectors read (`select_common::AnnotationType`, one enum for every tool);
  selecting the `domain_search` hits themselves is the new **`--has_search_hit`** on
  `select_by_cds`, `select_by_contig` (with the score range), `extract_peptides`,
  `extract_domains` and `rotate_contigs`. `select_by_cds` and `extract_peptides` share
  their OR-selectors-then-AND-restrictions logic: **`--invert` negates the selectors
  before the strand restriction**, never after it, and is ignored with a warning when
  there are no selectors. `extract_peptides` gains `--domain_expr`, `select_by_cds`
  gains `--name_field` (`cds_name_precedence`).
- **`aac61a9`**, ported: taxonomy filters move from `domainate` (which loses them, and
  `--hits_only`/`--no_annotations`, to `domain_search`) to every selector, through one
  contract: `seq::taxonomy::TaxonomyFilter`, compiled by `NCBITaxonomy::compile_filter`
  with the new **`--taxonomy_unknown exclude|include|evaluate|error`** for a candidate
  whose taxid the database cannot resolve (missing, 32644, deleted, absent). Before,
  such a candidate passed whenever 32644 was allowed. A criterion the database does not
  know is now an error, and a malformed taxon on the source a candidate's taxid comes
  from is an error rather than "unknown". Each tool judges its own candidate: the focus
  CDS (`select_by_cds`, `extract_peptides`), the annotation before extension
  (`extract_domains`), the record (`select_by_contig`, after selection;
  `extract_unannotated`). `domain_search` now judges each protein and CDS before the
  search, and each whole-contig nucleotide hit by the source covering it before
  ranking (`domainate::nucleotide_contig_searchable` and the hit filter), where the
  port used to drop whole records by one taxid. The shared options are one flattened
  `TaxonomyArgs`.
- **`09ddc4d`, `d852e03`**, ported: `--dedup none|id|sequence|sequence_rc` replaces
  `domain_search`'s `--deduplicate`/`--deduplicate_by_sequence`/`--both_strands` and
  `select_by_cds`'s `--deduplicate` (default `id` there), through `tools/dedup.rs`
  (upstream `dedup.py`). With `--max_hits` the top-N is now unique by the policy and a
  better-scoring copy replaces its representative (`TopHits`). A sequence's
  "other strand" is its reverse complement only: `sequence_keys` dropped the plain
  complement, which changes `deduplicate_genbank --both_strands` too. `--id` is an alias
  of `deduplicate_genbank --identity` (the manifest now lists visible aliases).
- **`b10240a`**, ported: `summary_report`'s co-occurrence rows are sorted by focus domain,
  in the library function now, not only in the CLI.
- **`60095af`**, nothing to port: infernox already drops the hits Infernal marks as
  duplicates (see "Pinned dependency: `infernox`").
- **`9b7ad26`**, nothing to port: a TODO comment.
- Upstream's new `test_taxonomy_filters.py`, `test_cds_selection.py`,
  `test_annotation_selection.py` and `test_dedup.py` call the tools as library
  functions or through `getattr`, so the harvester sees none of them. Their behaviors are
  covered by `extra:` invocations built on upstream's own `mixed()`/`protein()`
  builders (`generated_fixtures.py`, `_selection_fixtures`). Writing them found one
  older difference: `extract_peptides` dropped the contig's DBLINK, which upstream
  copies onto each peptide.
- **`961d817`**, nothing to port: TODO comments deleted, no behavior changed.
- **`73c7b84`**, ported: the **unified Domainator Viewer**. One page and one bundle format
  (`.dvb`, `tools/viewer_bundle.rs`) for networks, trees and projections; the shell is
  `assets/viewer_shell.html`, now rendered from `viewer_html.py` by
  `scripts/regen_viewer_shell.py`, whose GUI host hook moved to `js/common/metadata_table.js`
  so that selection reaches the app from all three views. `build_ssn_viewer` is gone
  upstream and here: `build_ssn --html_out/--dvb_out` (`--merge_impact_metric`,
  `--label_by`, `--categorical`, `--name`) writes the network, and the new `viewer` tool
  writes the page. `.dsnv` is read, never written: `ssn_bundle::load_bundle` upgrades it to
  the `.dvb` it corresponds to, and `ssn_navigator` reads `views.ssn`. Ported with it, as
  new tools here: **`build_tree`** (UPGMA on the existing scipy `linkage` port, a
  Biopython-faithful Newick reader in `io/newick.rs`, `--msa` aligned FASTA via
  `io/alignment.rs`, rectangular-tree XGMML) and **`build_projection`** (`pca` and
  `coords`; `tsne`/`umap` deferred, below). The shared `report.css` lost some rules, so the
  report shells were regenerated. The harvester now also reads the qualified
  `<tool>.main([...])` calls in other tools' test modules (test_viewer.py builds its
  networks with `build_ssn.main`), and resolves `tmp_path` and `os.path.join`; the sweep
  compares viewer pages, `.dvb` bundles and projection coordinates (`differential.py`).
- **`d3ccc35`**, page only: matrix_report's slider opens at the lowest threshold. The
  regenerated `report_shells/matrix_report.html` is the whole port.
- **`377cdff`**, ported. The report pages' CSS and JS changed (compact figure blocks in
  the new `common/panels.js`, restyled matrix_report sliders, no text selection while
  dragging a chart), which regenerated the summary_report, matrix_report and plot_contigs
  shells. summary_report's page now bins the contig lengths itself as its histogram zooms,
  so its data carries `length_counts` (distinct lengths and their counts) in place of
  `length_histogram`, and `PAYLOAD_VERSION` is 2 (`report_html.rs`). The `-o` text report
  is now laid out by `text_plot.text_table` instead of pandas' `to_string`; the port
  writes it with `text_plot::text_table`, so the four upstream tests that write it
  reproduce, and the `deferred:text-report` rows and their `extra:` stand-ins are gone.
- **`0118934`**, nothing to port: matplotlib and seaborn are gone from upstream entirely.
- **`de65a3e`**, nothing to port: `build_projection --png_out` became an `--html_out` page.
  The tool is unported here, so only the manifest snapshot changed, which its
  `deferred:unported-tool` allowlist row already covers.
- **`5405402`**, ported. Two bugs the example reports turned up. A taxid missing from the
  taxonomy database crashed upstream (`KeyError` in `lineage`); it is now unknown, with one
  warning per taxid, and summary_report gives it a blank name and rank. The port already
  returned an empty lineage, and now warns the same way. summary_report's domain table
  used pandas' unstable sort, so equal counts came out scrambled past 16 domains;
  upstream now sorts stably, the first-seen order the port already used.
- **`2e6e51e`**, nothing to port. matplotlib and seaborn moved to upstream's `projection`
  extra (only `build_projection --png_out` uses them; that tool is unported here), and
  `web_libs.py` is gone. `AutoColormap`'s palette is now the literal `colors.AUTO_COLORS`,
  equal to `color_genbank.rs`'s `AUTO_PALETTE`. The removed domain distance code in
  summary_report never reached an output. The manifest and every shell are unchanged.
- **`d7a75a4`**, ported: `plot_contigs` is a new tool here (`tools/plot_contigs.rs`). Its
  page moved onto `report_html.py` (`report_shells/plot_contigs.html`), and upstream's
  Python now builds only the data: each contig's details and line color, each drawn
  feature's name, type, color, parts and qualifiers. The page stacks the features
  itself, so nothing layout-sensitive is left to reproduce. Contig details read the
  record's annotations, and matching Biopython's `comment` exposed a reader bug: COMMENT
  lines kept their trailing spaces, and a padded structured comment was never split out
  (fixed in `parse_comment`). The six harvested invocations reproduce; `differential.py`
  now recognizes a page by its first bytes, since these tests name it with no extension.
- **`fb6b947`**, ported. matrix_report's `--html_out` page moved onto `report_html.py`
  (`report_shells/matrix_report.html`, new shared scripts `common/line_chart.js` and
  `common/pack.js`). Its data is the `--json_out` document plus the MST, the threshold
  tables as columns, the moving sum, the slider stops and the `--knn_projection` counts
  (`matrix_report::to_html_data`); where the slider puts each stop is now the page's own
  layout, so nothing rounding-sensitive is left to reproduce. `--knn_projection` is ported
  (`ssn::mst_knn_edge_counts_by_threshold`, checked against `mst_knn_edge_list` at every
  threshold and `k`), and so is upstream's `text_plot.text_histogram` (`text_plot.rs`),
  which makes the `-o` text report byte-identical too. The two matrix_report invocations
  that write a page reproduce; only gzip's bytes differ, and one `edge_scores.mean` in
  its last digit (numpy's pairwise summation), as in `--json_out`.
- **`920d690`**, ported. summary_report's `--html_out` page moved onto `report_html.py`:
  the `--json_out` document plus a `length_histogram` and a `taxonomy_tree`, computed by
  integer rules this port reproduces (`summary_report::taxonomy_tree`; the histogram later
  moved into the page);
  `common/charts.js` and `common/hierarchy.js` draw them. The shell is
  `report_shells/summary_report.html`. `table.js` gained an `idPrefix`, which regenerated
  the table shell too.
- **`460f120`**, tests only: the HTML report tests call `main([...])` with literal lists
  again, and hmmer_report's page gained one, so the harvester replays all four.
- **`a1aaa2f`**, ported. Upstream's `--html_out` pages are now built by `report_html.py`
  from package files in `web/`, with no third-party JavaScript, and their data in one
  `report-data` block; the enum_report/hmmer_report table is the first page on it.
  Both tools' `--html_out` and `--html_max_height` are ported the way the SSN viewer is:
  `scripts/regen_report_shells.py` renders upstream's `report_shell()` into
  `crates/domainator-core/assets/report_shells/`, and `tools/report_html.rs` fills its
  tokens. `differential.py` compares an `.html` page byte for byte outside its data and
  parsed inside it. All four HTML invocations reproduce byte for byte.
- **Since `f18dc15`** (`000b1c4`, `44acbce`), ported. The viewer's Zoom box, color-picker
  start offset and two fixes (a stale layout drawn while a new bundle loads, an
  extraction inheriting the parent's pan/zoom) arrived with the regenerated shell.
  `--palette_offset` is ported for `build_ssn`, the one of its three tools this port has
  (`build_projection` and `plot_contigs` are unported); `test_build_ssn_palette_offset`
  reproduces byte for byte.
- **`f18dc15`** — *bugfix* (2026-10-25). The 29 commits since `02a31c1` are upstream's 1.0 refactor
  and the features it gathered. The port followed it in eight phases on the
  `upstream-1.0-sync` branch; `CHANGELOG.md` (1.0.0) lists what changed for users.
- **The 1.0 interface** (`9207113`, `e7af752`, `8519ff8`), ported. Upstream adopted one
  dispatcher, `domainator <tool>` (so ours is now `domainatoredux`), renamed options per
  its `MIGRATION.md`, made every output an explicit `--<content>_out` option, and added
  `domainator describe`, a manifest of every command. Each tool's interface is now one
  clap struct (`crates/domainator-config/src/args/`), `describe` writes upstream's
  manifest format, and `--config` files are interchangeable with upstream's. The
  manifest golden test (`crates/domainator-cli/tests/upstream_manifest.rs`, against
  `scripts/parity/upstream_manifest.json`) fails on any option difference not listed
  in `scripts/parity/manifest_allowlist.tsv`. `completion` and `doctor` are ported too.
- **File formats** (`7bcb10e`, `a5a6fba`, `3bb5c00`), ported. HDF5 matrices are
  version 2.0: sparse only, canonical CSR, and a symmetric matrix stored as its upper
  triangle. Versions 1 and 2 are read. `.dsnv` and dense text are unchanged.
- **Behavior changes**, ported: region names follow `model.region_id` (`3b75ce2`); `-e`
  keeps hits with E-value `<=` the cutoff; `--max_overlap` rejects only overlaps
  strictly above it; boolean expressions parse with `~` > `&` > `|` and fail at startup
  when malformed; XGMML escapes once; mgnify `FL=0` means fragment status `?`
  (`d9003f8`); `--cpu 0` means physical cores; upstream's stderr logging levels.
- **New behavior**, ported: `domain_search --deduplicate_by_sequence` (`109be05`) and
  circular `--whole_contig` (`7866f33`); `--mst` and `--knn` as one union and
  `--symmetrize` (`b0db413`); `rotate_contigs` (`1f7d215`); `compare_contigs`'s
  sorted `--genbank_out` with `--linkage`/`--leaf_ordering` (`af4f675`), through a
  port of scipy's hierarchy code (`matrix/hierarchy.rs`).
- **Upstream Biopython** (`8801d78`): upstream dropped its vendored Biopython for the
  stock package plus subclass patches. Our GenBank writer was already byte-identical;
  the comparison turned up one difference, in structured comments, now fixed.
- **Not ported, nothing to port:** library API (`fd8c78e`), performance (`923361d`,
  `ad4b37d`'s free-threaded pools), packaging and conda (`bfa0026`, `359353c`,
  `93ff59d`), docs (`9a8efb2`, `f902c98`, `9389c89`), tests (`2bbf8d7`, harvested by the
  sweep), `db_download` (`1bf7f72`, the tool is unported). The SSN viewer moved into
  `ssn_viewer/` (`5421906`); the shell was regenerated from it.
- **Upstream bug found while syncing:** `seq_dist --algorithm hmmer` all-vs-all raised
  "phmmer result stream yielded query … out of order"; fixed upstream in `f18dc15`.
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
| 1 | **Literal vendored artifacts** | Files copied/rendered verbatim from Python: the Domainator Viewer HTML shell (`crates/domainator-core/assets/viewer_shell.html`), rendered from `viewer_html.py`, and the `--html_out` report shells (`crates/domainator-core/assets/report_shells/`), rendered from `report_html.report_shell()`. | Regenerated by `scripts/regen_viewer_shell.py` and `scripts/regen_report_shells.py`; `--check` mode is the drift guard. **Never hand-edit the assets.** |
| 2 | **On-disk format contracts** | GenBank, HDF5, `.didx`, shards, BGZF, `.dvb` bundle (and the `.dsnv` it replaced, read only), Newick, XGMML, TSV number formatting. | Byte/semantic **parity gates** (`external/domainator/test`, `test/helpers.py`). A format change requires Rust code changes in the owning module. |
| 3 | **Tool / flag / algorithm coverage** | New tools, flags, or modes (e.g. `rotate_contigs`, `--symmetrize`, treemap). | The manifest golden test fails on every option upstream adds; `docs/porting-status.md` is the per-tool ledger; new behavior needs a port + a parity gate. |
| 4 | **Hand-ported logic tables** | Genetic code, PROSITE patterns, protein chemistry, taxonomy. | Fixture-free `cargo test`s against documented reference values (BioPython etc.). |

## Python file → Rust owner map

Use this to triage which module a changed upstream file lands in:

| Python (`src/domainator/…`) | Rust owner |
|---|---|
| `viewer_html.py`, `viewer_app/` (the Domainator Viewer's HTML, CSS and JS sources) | `crates/domainator-core/assets/viewer_shell.html` (via `scripts/regen_viewer_shell.py`), filled by `crates/domainator-core/src/tools/viewer_html.rs`; `differential.py`'s `_compare_viewer_html` |
| `viewer_bundle.py` | `.../tools/viewer_bundle.rs` (the `.dvb` format), `.../tools/node_table.rs` (`read_node_metadata`) |
| `viewer.py` | `crates/domainator-cli/src/commands/viewer.rs` |
| `build_tree.py` | `.../tools/build_tree.rs`, `.../io/newick.rs` (Bio.Phylo's `NewickIO`), `.../io/alignment.rs` (`read_msa`) |
| `build_projection.py` | `.../tools/build_projection.rs` (`pca`/`coords`; `tsne`/`umap` deferred) |
| `report_html.py`, `web/` (the report pages' HTML, CSS and JS) | `crates/domainator-core/assets/report_shells/` (via `scripts/regen_report_shells.py`), filled by `crates/domainator-core/src/tools/report_html.rs`; `scripts/parity/differential.py`'s `_compare_html` reads the `report-data`/`report-rows` blocks. A new page needs its name in `regen_report_shells.py`'s `REPORTS` and an `include_str!` in `report_html.rs`; a change to the envelope needs `PAYLOAD_VERSION` |
| `cli/registry.py`, `cli/manifest.py`, `cli/common.py` (each tool's `build_parser`) | `crates/domainator-config/src/{registry,manifest,config_file,types}.rs`, `.../args/<tool>.rs`; the I/O and error rules in `crates/domainator-cli/src/{lib,output}.rs`. Refresh `scripts/parity/upstream_manifest.json` and the allowlist on any change |
| `cli/doctor.py`, `cli/completion.py` | `crates/domainator-cli/src/commands/doctor.rs` (programs from `crates/domainator-core/src/tools/external.rs`), `completion` in `crates/domainator-cli/src/lib.rs` |
| `report_columns.py`, `table_writers.py` | `crates/domainator-config/src/columns.rs` (the catalogs and token parser), the TSV/NDJSON/HTML writers in `crates/domainator-core/src/tools/{enum_report,hmmer_report}.rs` (the HTML one through `report_html::TablePage`) |
| `external.py` | `crates/domainator-core/src/tools/external.rs` (`find_binary`, `EXTERNAL_PROGRAMS`) |
| `output_guardrails.py` | `crates/domainator-core/src/tools/output_guardrails.rs`, `.../matrix/estimate.rs` |
| `docs/algorithms.md` | **reference, not ported** — read `external/domainator/docs/algorithms.md` for the SSN internals (MST, threshold tables, split-event scanning, the cap/window/pin selection, the moving sum, slider stops, dendrogram cutting). Its "Ports that must stay in step" table is the authority on which functions mirror which; this port is a third implementation of that same list. |
| `ssn_hierarchy.py` | `crates/domainator-core/src/matrix/ssn.rs` |
| `ssn_bundle.py` | `.../tools/ssn_bundle.rs` (`ssn_view`, the network view's reader, and the `.dsnv` upgrade) |
| `ssn_navigator.py` | `.../tools/ssn_navigator.rs` |
| `summary_report.py` | `.../tools/summary_report.rs` (`length_counts`, `taxonomy_tree` feed the HTML page; `render_text` is `SummaryTextWriter`) |
| `colors.py` | `.../palette.rs` |
| `structure_lib.py` | `crates/domainator-core/src/tools/structure_lib.rs` (input resolution, the foldseek/reseek drivers, the by-target sort, hits → `SearchResult`) |
| `structure_to_genbank.py`, `structure_domainate.py`, `structure_search.py`, `structure_dist.py` | `.../tools/structure_{to_genbank,domainate,search,dist}.rs` |
| `foldseek.py` (`domainate --foldseek`) | `crates/domainator-core/src/search/foldseek.rs` (the ProstT5 route only: `createdb --prostt5-model`, `search`, `convertalis`; ESM-2 3B 3Di is refused, see below) |
| `db_download.py` | `.../tools/db_download.rs` (the UniProt streams, the assembly summary and its filters, the `direct`/`datasets` backends); the SwissProt conversion is `.../io/swissprot.rs`, a port of Biopython's `Bio.SwissProt._read` and `SeqIO.SwissIO` (a Biopython upgrade that changes them needs the same change) |
| `build_ssn.py` | `.../tools/build_ssn.rs` |
| `ssn_edges.py`, `edge_types.py` | `.../matrix/ssn.rs` |
| `sparsify.py` (`--mst`/`--knn`), `matrix_output.py`, `sparse_scores.py` | `.../matrix/mst_knn.rs`, `.../matrix/hdf5_io.rs`, and each matrix tool's output path |
| `transform_matrix.py`, `data_matrix.py` (HDF5/TSV) | `.../tools/transform_matrix.rs`, `.../matrix/{hdf5_io,text,transform}.rs` |
| `seq_dist.py` | `.../tools/seq_dist.rs` |
| `compare_contigs.py` | `.../tools/compare_contigs.rs`; its sort's scipy calls (`linkage`, `leaves_list`, `optimal_leaf_ordering`) are `.../matrix/hierarchy.rs` |
| `domainate.py` | `.../tools/domainate.rs`, `.../search/hmmer.rs`; the `infernal` program (`infernal_hits_to_search_results`) is `.../search/infernal.rs` |
| `domainate.py` gene calling (`prodigal_CDS_annotate`, `_prodigal_linear_location`, `_prodigal_origin_genes`) | `.../tools/gene_call.rs` (drives the external Orphos binary; also the port-only `gene_call` tool) |
| `domain_search.py`, `select_by_cds.py` | `.../tools/{domain_search,select_by_cds,neighborhood}.rs` |
| `dedup.py` (`--dedup`, `RecordDeduplicator`, `TopHits`) | `.../tools/dedup.rs` |
| `select_by_contig.py`, `enum_report.py` | `.../tools/{select_by_contig,enum_report,select_common}.rs` |
| `matrix_report.py` | `.../tools/matrix_report.rs` |
| `text_plot.py` (the text reports' histograms and tables) | `.../text_plot.rs` |
| `hmmer_compare.py` | `.../tools/hmmer_compare.rs`, `.../search/hmm_compare.rs` |
| `seqio.py`, `genbank.py` (Biopython subclasses), `lean_record.py`, `offsets.py`, `db_index.py`, `bgzf_compress.py` | `.../io/{genbank,fasta,bgzf,didx,shards,partition}.rs` |
| `hmmio.py` | `.../search/{hmm_read,hmm_write,references}.rs`; `read_infernal_cms` is `.../search/infernal.rs` (`load_cm_db`) |
| `infernal_build.py` | `.../tools/infernal_build.rs` (drives the external `cmbuild`) |
| `expr.py` | `.../tools/select_common.rs` (`BooleanEvaluator`), `.../seq/taxonomy.rs` (`--taxonomy_expr`) |
| `model.py` (`region_id`, the per-CDS view, `annotation_feature_types`, `cds_name_precedence`) | `.../tools/neighborhood.rs` (`region_id`), `.../tools/select_common.rs` (`AnnotationType`) |
| `taxonomy/`, the protein chemistry in `enum_report.py` | `.../seq/{protein_analysis,taxonomy,taxid,translate}.rs`; `taxonomy/records.py`'s `TaxonomyFilter` is `seq/taxonomy.rs`'s, and `cli/common.py`'s `add_taxonomy_filters` is `args/common.rs`'s `TaxonomyArgs` |
| `partial.py` (`get_fragment_status`, `on_contig_translation`, `codon_offset`, ...) | `.../seq/partial.rs` |
| `locations.py` (`slice_record_from_location`, `slice_record`, `_truncate_cds_qualifiers`), `location_ops.py` (`merge_parts`) | `.../tools/neighborhood.rs` (`slice_features`), `.../seq/location.rs` (`merge_touching_parts`) |
| `rotate_contigs.py` | `.../tools/rotate_contigs.rs` (the search is `domainate`, the slice `extract_common`) |
| `extract_domains.py`, `extract_peptides.py`, `extract_unannotated.py` | `.../tools/{extract_domains,extract_peptides,extract_unannotated,extract_common}.rs` |

## The routine

Run `scripts/sync_upstream.sh` (see its header for options). It:

1. Fast-forwards `external/domainator/` to the target upstream ref.
2. Prints `git log --oneline --stat <pin>..<target>` — the changelog to triage against the
   map above.
3. Runs the automated guards it can: `regen_viewer_shell.py --check`, the `ssn_probe`
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

- **`domainate --esm2_3di_weights`** (`foldseekBuilder` in `foldseek.py`): ESM-2 3B 3Di
  predicts 3Di in PyTorch. `--foldseek` itself is ported through upstream's other
  predictor, `--prostt5_weights`, where foldseek runs ProstT5 itself, so both sides call
  the same binary and the output is byte-identical (per device: CPU and GPU ProstT5 differ
  slightly from each other). `--esm2_3di_weights`/`--esm2_3di_device` are accepted, so
  upstream configs load, and `--esm2_3di_weights` is refused with a pointer to
  `--prostt5_weights`.
- **`merge_impact_axis_labels`** (`ssn_hierarchy.py`). Its consumers are `viewer_html.py`
  (the viewer config's `splitAxisLabels`) and matrix_report's page; both generated shells
  carry its values pre-rendered, so a shell regeneration is all a change to it needs.
- **`build_projection --algorithm tsne|umap`** (scikit-learn's `TSNE`, umap-learn): both
  are stochastic, so there is nothing to hold a port to beyond the shape of the output. The
  CLI keeps them in its choices, so the manifest matches, and refuses them at run time
  (`deferred:projection-umap-tsne`). A Python projection reads back in with
  `--algorithm coords`.

## What the parity sweep still tracks

`scripts/parity/known_gaps.tsv` holds every upstream invocation that does not reproduce,
machine-checked on each sweep so none can be quietly forgotten. No open `gap:` row remains;
the two left are `deferred:projection-umap-tsne`, above.

A `tolerated` row is one where the semantic comparator passed but the **bytes differ**, so
the gap list structurally cannot hold it. Diffing those bytes is the single most productive
thing to do with a green sweep: nine divergences were found that way and fixed — GenBank
qualifier wrapping, qualifier quoting, the LOCUS topology column, the missing creation-date
stamp, `/key` vs `/key=""`, the Pfam `GA`/`TC`/`NC` cutoffs, the `.hmm` model-matrix grid,
f64-vs-f32 score rounding, and cluster-table ordering. Byte-identical invocations went from
51 to 133 as a result (136 once the nucleotide single-sequence builder landed).

The last one, `gap:feature-order` (six `domain_search` invocations on nucleotide
references), was in `select_by_cds_hits`: it put every `Domainator` feature ahead of the
focus `Domain_Search` feature, where upstream lists features by its sorted pseudo-CDS
list, in which each whole-contig annotation, `Domain_Search` and `Domainator` alike, is an
entry of its own. Where two features shared a start, the writer's stable sort kept the
wrong one first.

The port and upstream also disagreed on which entries of that list `--cds_range` counts
(the port counted the `Domain_Search` pseudo-CDSs, upstream every pseudo-CDS), which no
upstream test exercised. Upstream now counts protein-coding CDSs only
(`select_by_cds.cds_range_members`: the focus may be a nucleotide hit, but never a
neighbor), and the port follows (`neighborhood::cds_range_members`);
`extra:nucleotide_multi_hit_cds_range` covers it in the sweep.

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

## Patched dependency: `candle-core`

**`vendor/candle-core/` is the published `candle-core` 0.11.1 with one line changed**, wired
in by `[patch.crates-io]` in `Cargo.toml`. The `cudarc` dependency's `"dynamic-linking"`
feature becomes `"dynamic-loading"` (`vendor/candle-core/PATCHES.md`).

Why: with `dynamic-linking`, cudarc's build script links the driver's `libcuda`, cuBLAS,
cuBLASLt, cuRAND and NVRTC, so a `cuda` build would not start on a machine without them,
not even to run on the CPU (issue #20). With `dynamic-loading` they are opened on first use.
cudarc's build script panics if both features are on, and Cargo features are additive, so
this cannot be done from our side with a feature flag. Under `dynamic-loading` cudarc
**panics** on a library it cannot open, so `domainator_plm::cuda_libraries()` checks each
one (`is_culib_present`) before any device is opened. That function lists the libraries
candle reaches inside `Device::new_cuda` (driver, cuBLAS, cuBLASLt, cuRAND). Re-check that
list on a candle bump: a new library reached before or during device creation needs a line
there too.

**Known trade-off: which version gets loaded.** cudarc tries file names in its
`get_lib_name_candidates` order, which puts the unversioned `libcublas.so` **before**
`libcublas.so.12`. On a machine where an older toolkit's dev symlink is the first
`libcublas.so` the loader finds, a CUDA 12 build loads cuBLAS 11. The linked build used to
refuse to start there instead, since it bound the `libcublas.so.12` soname. Checked
2026-11-03 on a machine with CUDA 11's `libcublas.so` in the ld cache: TMbed ran on the GPU
with cuBLAS 11 and with cuBLAS 12, and the two outputs differed only in last-digit scores,
as GPU and CPU runs do. A cuBLAS old enough to lack a symbol cudarc wants would panic at
first use. The fix belongs in cudarc (try `lib{name}.so.{major}` first) and has not been
sent.

The release workflow's "No CUDA library is linked" step is the guard: it fails a CUDA build
whose binary still names a CUDA library as a load-time dependency.

**Updating candle:** copy the new release from `~/.cargo/registry/src/*/candle-core-X.Y.Z`
over `vendor/candle-core/` (keep `PATCHES.md`), reapply the one-line change, and confirm with
`diff -r` that it is the only difference. **Dropping the patch,** once candle makes the
linking mode a feature (or switches to `dynamic-loading`): delete the `[patch.crates-io]`
entry and `vendor/candle-core/`, enable that feature in `crates/domainator-plm/Cargo.toml`
if there is one, `cargo update -p candle-core`, and do a CUDA build to check that the
workflow guard still passes. Keep `crates/domainator-plm/build.rs` (the CUDA library search path) for as long
as candle-kernels asks for `-lcudart` without supplying one.

## TMbed (`find_features --tmbed`): reimplemented, at upstream's pin

Upstream runs TMbed (`tmbed @ 8cee893523eb655bc9485c00c65336d27a236191`) through PyTorch;
`crates/domainator-plm` reimplements it on candle: the ProtT5 encoder (`t5.rs`, from HF
`modeling_t5.py`, transformers 4.x, including its f16 rules: `wo` kept in f32, the
residual stream promoted), the CNN ensemble (`tmbed/cnn.rs`, from `tmbed/model.py`) and
the Viterbi decoder (`tmbed/viterbi.rs`). The five CNN models are TMbed's own
(Apache-2.0), converted to `assets/tmbed/cnn.safetensors` by
`scripts/tmbed/make_assets.py`, which checks their SHA-256 against that commit and also
writes the goldens `cargo test -p domainator-plm` compares the CNN and decoder with.

A TMbed bump means: update the commit in `make_assets.py` (and the hashes), rerun it in an
env with the new TMbed, diff `model.py`/`viterbi.py`/`tmbed.py`/`embed.py` against the Rust,
and rerun the find_features sweep with that env (`docs/parity.md`, "External binaries").
The ProtT5 weights are downloaded at a pinned Hugging Face revision
(`tools/tmbed_model.rs`, `PROT_T5_REVISION`) and checked against their SHA-256.

## Pinned dependency: `infernox`

`.cm` references are searched in-process by [infernox](https://github.com/necoli1822/infernox),
a BSD-licensed Rust port of Infernal 1.1.5 whose `cmsearch` is byte-identical to C's. It is
pinned exactly (`infernox = "=0.2.0"`), because a release that changed one hit would change
annotations, so a bump is a re-verification: the sweep's infernal rows (the upstream ones
plus the `extra:infernal_*` set, which cuts the CopA site so the truncated passes run) and
the aarch64 check. `search/infernal.rs` reproduces what `pyinfernal.cmsearch(cms, block)`
does, which differs from the `cmsearch` command line and from infernox's library defaults:

- **`Z` is the block's residue count, not doubled** for two strands (pyinfernal sets
  `Z = targets.total_length()`; cmsearch's default is twice that). infernox takes `Z` in
  megabases and truncates, so `z_megabases` steps the float until it lands exactly.
- **Truncated hits are on** (`notrunc: false`); infernox's `FaithfulConfig::default()` has
  them off, and that alone drops every hit cut by a contig end.
- **`E = 10`**, pyinfernal's pipeline default, is the scan cutoff; `--evalue` (capped at 10
  upstream anyway) and `--min_evalue` only filter.
- **Strict Forward arithmetic** (`set_forward_strict(true)`): infernox otherwise fuses
  multiply-adds where the CPU has FMA, which moves scores in the last bits and makes x86 and
  ARM disagree. Strict costs about 15%.

Hits Infernal marks as overlapping duplicates are dropped, as `cmsearch` drops them.
pyinfernal keeps them in `TopHits` (flagged `duplicate`), and upstream's
`infernal_hits_to_search_results` annotated them too, writing the same locus twice. That was
fixed upstream to skip `hit.duplicate`, with `test_domainator_infernal_skips_duplicate_hits`;
the sweep row of that name needs an upstream checkout carrying the fix.
