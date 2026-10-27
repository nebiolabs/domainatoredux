# Changelog

This project uses its own [SemVer](https://semver.org/) numbering, independent of the
Python `domainator` package. Each entry says which upstream release (and commit) it is at
parity with. How versions are assigned: `docs/development.md`, "Versioning and releases".

## Unreleased — 1.0.0

Parity: Python domainator 1.0.0rc1 @ `2e6e51e`.

- First release under this versioning scheme. The earlier `v0.11.0` tag predates it.
- **`enum_report` and `hmmer_report --html_out`** (and `--html_max_height`): upstream's
  new interactive table page, which works offline. It is generated from upstream's own
  page (`scripts/regen_report_shells.py`), so the two write the same file.
- **`summary_report --html_out`**: upstream's new offline page, with the contig length
  histogram, sortable domain and co-occurrence tables, and a zoomable taxonomy chart.
- **`matrix_report --html_out` and `--knn_projection`**: upstream's new offline page, with
  clusters and edges by threshold, the split-event chart above a threshold slider, the
  clusters at the chosen threshold, and (with `--knn_projection`) the edges
  `build_ssn --mst --knn k` would keep at that threshold for each `k`. The text report's
  score histogram is now upstream's, so `-o` writes the same file as `domainator`.
- **`plot_contigs`**, a new tool: upstream's contig plot, an offline page that draws each
  contig with its features stacked above it, colored by `--metadata` with `--color_by`.
- **GenBank COMMENT lines are read as Biopython reads them**: without trailing spaces,
  blank lines skipped, and the structured comment split out even when its lines are
  padded, as NCBI pads them. Such a record used to be written back with its structured
  comment inside the free text.
- **Upstream's 1.0 interface.** Python domainator 1.0 adopted a single `domainator <tool>`
  command, as this port had. The two would clash, so **the executable is now
  `domainatoredux`**, and its tools take **upstream's names and options**:
  `domainatoredux select_by_cds …` accepts exactly what `domainator select_by_cds …`
  does. Options were renamed as in upstream's
  [`MIGRATION.md`](https://github.com/nebiolabs/domainator/blob/version_one/MIGRATION.md),
  every output is an explicit `--<content>_out` option (`-` is stdout), and an output named
  `.gz` is compressed. There are no aliases for the old names.
- **`--config` files are interchangeable with upstream's**: one flat YAML or JSON object
  keyed by option name, report columns as `name:arg` tokens (`taxname:genus`). Options on
  the command line override the file, and every check applies to both. `--print_config`
  writes the resolved configuration in the same format.
- **`domainatoredux describe`** writes upstream's machine-readable manifest of every tool
  and option; it replaces `schema`. A test compares it with upstream's own `describe`, so
  any difference is either deliberate and listed, or a failing test. Also new:
  **`completion`** (shell completion scripts) and **`doctor`** (which external programs
  are installed, and which tools need them).
- **Errors and messages** go to stderr: errors as one line,
  `domainatoredux <tool>: error: …`, progress at upstream's levels (`-q` for errors only,
  `-v` for detail).
- **New tool, `rotate_contigs`**: rotates circular contigs to start at a chosen domain or
  search hit, reverse-complementing them to put it on the forward strand.
- **New options**: `compare_contigs --genbank_out` writes the contigs sorted by similarity
  (`--linkage`, `--leaf_ordering`, `--name_by_order`); `--symmetrize` on `seq_dist` and
  `transform_matrix`; `--threshold`, and `--mst` with `--knn K` (their union) replacing
  `--mst_knn`; `domain_search --deduplicate_by_sequence`, `--anchor`, and circular
  `--whole_contig` output; `select_by_contig` length, CDS-count, score and record-count
  filters; `deduplicate_genbank --identity`, `--cluster_tsv_out` and per-program
  `--<program>_path`/`--<program>_args`; `build_ssn --palette_offset`, to color a second
  column without reusing the colors a first one took.
- **SSN viewer** (upstream's, shared by `build_ssn_viewer` and the GUI): a **Zoom** box in
  View Settings that shows and sets the zoom, so two viewers can draw at the same scale; a
  **Start offset** beside the color picker's palette menu, the viewer's `--palette_offset`;
  loading a second bundle no longer briefly draws the first one's layout against it, and a
  saved extraction opens fitted to its own layout instead of the parent's pan and zoom.
- **HDF5 matrices are written in upstream's format 2.0**: always sparse, with a
  symmetric matrix stored as its upper triangle. Format 1.0 files still load.
- **Behavior now matching upstream 1.0**: extracted regions are named `id_first:last`
  (and `id` / `id_rc` for whole contigs); `-e` keeps hits whose E-value equals the
  cutoff; `--max_overlap` rejects only overlaps above the fraction; boolean domain
  expressions bind `~` before `&` before `|`, and a malformed one fails at startup;
  `--cpu 0` uses one thread per physical core; MGnify `FL=0` headers mark fragments.
- **Renamed with the executable:** the app is **Domainatoredux**, its executable and
  release downloads are `domainatoredux-gui`, its workspace
  folder is `.domainatoredux/`, and the environment variables are `DOMAINATOREDUX_*`
  (`DOMAINATOREDUX_DIAMOND`, `_CDHIT`, `_USEARCH`, `_ORPHOS`). The app's identifier
  changed too, so its preferences and recent-workspace list start fresh.
- The app's tool dialogs are built from the same manifest as `describe`, and its Tools
  menu is grouped by upstream's categories.
- **Gene calling.** A new **`gene_call`** tool adds Prodigal-style CDS features to
  nucleotide contigs, in meta mode (the default) or single mode, with a choice of genetic
  code, `--closed` and `--mask`. **`domainate`** and **`domain_search`** now accept
  upstream's `--gene_call all|unannotated`, and its output is byte-identical to the Python
  tool's. Gene calling runs the external GPL program
  [Orphos](https://github.com/FullHuman/orphos), version 0.4.0 or newer. The app's
  **External tools** panel can download it, or you can set `$DOMAINATOREDUX_ORPHOS`. It is
  also in the app's Tools menu as **Gene call**, and as a **Gene calling** option on
  Domainate and Domain search.
- **Export.** The app can copy files out of a workspace: **File → Export…**, or the
  right-click menus of the file table, the folder tree and the Databases panel. Files can be
  exported as they are, gzipped one by one or bundled into a zip. GenBank, FASTA and HMM
  files can also be split so that every record or profile becomes its own file. Existing
  files at the destination are only replaced after you confirm.
- The app runs each Tools-menu job in its own background process, running the same code as
  the `domainator` CLI. **Cancel now stops a running job** (and any diamond, cd-hit or
  usearch it started) and deletes the partial output files the job created or changed. A
  crashing tool no longer takes the app down with it. Jobs wait in a queue, two at a
  time by default (**Preferences → Jobs at once**). Each job's config and full output are
  kept in `.domainatoredux/jobs/<job>/`. Database index builds also run in the background and
  can be canceled. Setting `DOMAINATOREDUX_INPROCESS_JOBS=1` runs jobs inside the app as
  before; that escape hatch will be removed in the next release.
- Because app jobs now go through the CLI, some tools respect options they used to ignore.
  **matrix_report** honors `merge_impact_metric`, `max_merge_events` and `json_out`, and
  **hmmer_report** writes its `json_out` output (it used to write JSON into the TSV path).
  Every tool now uses exactly the CLI's defaults and checks.
- The `domainator-cli` crate is now a library (`domainator_cli::run`) with a thin binary;
  the CLI itself is unchanged.
- `domainatoredux --version`, the GUI About box, the diagnostics blob and job provenance now
  report the upstream version and commit the build is at parity with.
- `.dsnv` bundles record the upstream version in `domainator_version`, matching what the
  Python tool and the viewer write, so the bundle is now identical to Python's field for
  field.
- **Partial features and fragments** (upstream 0.12.0). Every tool that slices a record
  (`select_by_cds` and `domain_search` neighborhoods, `select_by_coord`, `extract_domains`,
  `extract_peptides`, `extract_unannotated`, `trim_contigs`, `clean_sequences --strip`,
  `enum_report --by cds|domain`) now keeps a feature the slice cuts. The feature is
  truncated, and its cut end is marked `<` or `>`. Markers already present on ends that
  were not cut are kept. A cut CDS gets an updated `/codon_start` and a `/translation`
  trimmed to the codons the slice keeps. A cut `Domainator`/`Domain_Search` hit is dropped.
  Before this, cut features were dropped, or truncated and left unmarked.
- `/codon_start` is honored. Domain hits are placed relative to the reading frame, and a
  missing translation is made in that frame. `extract_peptides` maps coordinates in that
  frame. A partial CDS whose `/translation` includes residues encoded off the edge of the
  contig is trimmed to the on-contig residues before it is searched.
- New `--partial include|exclude|only` on `domainate`, `domain_search`, `select_by_cds`,
  `select_by_contig`, `extract_peptides`, `extract_domains`, `enum_report` and
  `summary_report`. It selects by fragment status: a CDS with a `<`/`>` end, or a protein
  record marked as a fragment (a ` (Fragment)` header, a `Flags: Fragment` definition,
  `NON_TER` features, or a spanning partial CDS/Protein feature). `enum_report` gains
  `--fragment` and `--partial_count` columns. `summary_report` reports
  `partial_cdss` and `fragment_proteins`, and the app's Summary tab shows the same two
  counts as **partial CDS** and **fragments** tiles.
- Gene-caller partial codes (`/partial="10"` from prodigal or GeneMark) are converted to `<`/`>`
  markers when a GenBank file is read, and the qualifier is removed. An end within 2 bases
  of the contig edge is extended to the edge, with `/codon_start` updated. A bare
  `/partial` (no value) is kept, and makes its CDS a fragment with unknown missing ends
  (`?`).
- Fixed, found while syncing: header text in GenBank files is wrapped the way Biopython wraps
  it, so lines can now be the full 80 columns. A reverse-strand CDS ending at the last base
  of a contig no longer sorts ahead of every other CDS in `select_by_cds`. Sliced records
  from `trim_contigs` and `clean_sequences --strip` drop header annotations (and so get
  today's date), as every other slice does. `extract_domains` no longer lets a `--domains`
  match skip the `--evalue`/`--score` checks.
