# Staying in sync with the Python `domainator`

This Rust workspace is a rewrite of the Python `domainator`
(<https://github.com/nebiolabs/domainator>). Correctness is defined by **parity** with
that tool, so as the Python package evolves we periodically pull its changes across. This
file records how far the port is caught up and how to advance it.

## Upstream pin

- **Caught up to:** `961d817` — *delete some TODOs* (2026-10-28; upstream `__version__`
  1.0.0rc1, on the `version_one` branch).
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
| `structure_*.py`, `structure_lib.py`, `foldseek.py` | **deferred** — see below |
| `build_ssn.py` | `.../tools/build_ssn.rs` |
| `ssn_edges.py`, `edge_types.py` | `.../matrix/ssn.rs` |
| `sparsify.py` (`--mst`/`--knn`), `matrix_output.py`, `sparse_scores.py` | `.../matrix/mst_knn.rs`, `.../matrix/hdf5_io.rs`, and each matrix tool's output path |
| `transform_matrix.py`, `data_matrix.py` (HDF5/TSV) | `.../tools/transform_matrix.rs`, `.../matrix/{hdf5_io,text,transform}.rs` |
| `seq_dist.py` | `.../tools/seq_dist.rs` |
| `compare_contigs.py` | `.../tools/compare_contigs.rs`; its sort's scipy calls (`linkage`, `leaves_list`, `optimal_leaf_ordering`) are `.../matrix/hierarchy.rs` |
| `domainate.py` | `.../tools/domainate.rs`, `.../search/hmmer.rs`; the `infernal` program (`infernal_hits_to_search_results`) is `.../search/infernal.rs` |
| `domainate.py` gene calling (`prodigal_CDS_annotate`, `_prodigal_linear_location`, `_prodigal_origin_genes`) | `.../tools/gene_call.rs` (drives the external Orphos binary; also the port-only `gene_call` tool) |
| `domain_search.py`, `select_by_cds.py` | `.../tools/{domain_search,select_by_cds,neighborhood}.rs` |
| `select_by_contig.py`, `enum_report.py` | `.../tools/{select_by_contig,enum_report,select_common}.rs` |
| `matrix_report.py` | `.../tools/matrix_report.rs` |
| `text_plot.py` (the text reports' histograms and tables) | `.../text_plot.rs` |
| `hmmer_compare.py` | `.../tools/hmmer_compare.rs`, `.../search/hmm_compare.rs` |
| `seqio.py`, `genbank.py` (Biopython subclasses), `lean_record.py`, `offsets.py`, `db_index.py`, `bgzf_compress.py` | `.../io/{genbank,fasta,bgzf,didx,shards,partition}.rs` |
| `hmmio.py` | `.../search/{hmm_read,hmm_write,references}.rs`; `read_infernal_cms` is `.../search/infernal.rs` (`load_cm_db`) |
| `infernal_build.py` | `.../tools/infernal_build.rs` (drives the external `cmbuild`) |
| `expr.py` | `.../tools/select_common.rs` (`BooleanEvaluator`), `.../seq/taxonomy.rs` (`--taxonomy_expr`) |
| `model.py` (`region_id`, the per-CDS view) | `.../tools/neighborhood.rs` (`region_id`), `.../tools/select_common.rs` |
| `taxonomy/`, the protein chemistry in `enum_report.py` | `.../seq/{protein_analysis,taxonomy,taxid,translate}.rs` |
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

- **The structure subsystem** (`structure_to_genbank`, `structure_dist`, `structure_search`,
  `structure_domainate`, `structure_lib`). Genuinely portable — pure subprocess wrapping of
  `foldseek`/`reseek`, no torch or ESM — but out of scope for this sync. It touches ported
  code only through `SearchResult`'s four optional structural metrics, which **are** carried
  (`tmscore`/`lddt`/`rmsd`/`prob`), so structure-written GenBank stays round-trippable.
- **`merge_impact_axis_labels`** (`ssn_hierarchy.py`). Its only consumers are the
  vendored viewer shell and matrix_report's page shell, which each carry their own copy.
- **`hmmer_build --window_beta`.** Deriving MAXL from a tail mass needs builder calibration
  `hmmer-pure-rs` does not expose; the flag is refused rather than silently ignored.
- **`--progress`** draws no progress bar: `seq_dist` and `compare_contigs` accept and
  ignore it.
- **`seq_dist`'s diamond diagnostics**: the explained failures (out of disk or memory, a
  rejected `--diamond_args` option, unreadable input: `build_diamond_failure_message`); a
  failed run reports diamond's own stderr instead. The advisory before a large all-vs-all
  search is ported without its free-disk-space note, which needs a disk-usage call.
- **`--partial` on `find_features` and `db_download`** (`bdc74bb`). Neither tool
  is ported. The GUI's managed UniProt download could take the same filter later.
- **`build_projection --algorithm tsne|umap`** (scikit-learn's `TSNE`, umap-learn): both
  are stochastic, so there is nothing to hold a port to beyond the shape of the output. The
  CLI keeps them in its choices, so the manifest matches, and refuses them at run time
  (`deferred:projection-umap-tsne`). A Python projection reads back in with
  `--algorithm coords`.
- **Unported tools:** `kmer_dist`, `split_by_cluster`,
  `db_download` (the app's
  managed downloads cover UniProt, Pfam and NCBI Taxonomy),
  `find_features` (torch), and the structure tools above. Their manifest entries are
  `deferred:` rows of `scripts/parity/manifest_allowlist.tsv`.

## What the parity sweep still tracks

`scripts/parity/known_gaps.tsv` holds every upstream invocation that does not reproduce,
machine-checked on each sweep so none can be quietly forgotten. One open `gap:` row remains
(six invocations of it); everything else is a deliberate deferral:

| class | rows | |
|---|---|---|
| `gap:feature-order` | 6 | `domain_search` emits tied-coordinate features in the other order, below |

A `tolerated` row is one where the semantic comparator passed but the **bytes differ**, so
the gap list structurally cannot hold it. Diffing those bytes is the single most productive
thing to do with a green sweep: nine divergences were found that way and fixed — GenBank
qualifier wrapping, qualifier quoting, the LOCUS topology column, the missing creation-date
stamp, `/key` vs `/key=""`, the Pfam `GA`/`TC`/`NC` cutoffs, the `.hmm` model-matrix grid,
f64-vs-f32 score rounding, and cluster-table ordering. Byte-identical invocations went from
51 to 133 as a result (136 once the nucleotide single-sequence builder landed).

`gap:feature-order` (6 rows; the sixth is the whole-contig `.cm` invocation, which ties the
same way) is the residue of the nucleotide-FASTA gap described under
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
