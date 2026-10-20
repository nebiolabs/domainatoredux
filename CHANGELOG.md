# Changelog

This project uses its own [SemVer](https://semver.org/) numbering, independent of the
Python `domainator` package. Each entry says which upstream release (and commit) it is at
parity with. How versions are assigned: `docs/development.md`, "Versioning and releases".

## Unreleased — 1.0.0

Parity: Python domainator 0.12.0 @ `02a31c1`.

- First release under this versioning scheme. The earlier `v0.11.0` tag predates it.
- **Gene calling.** A new **`gene-call`** tool adds Prodigal-style CDS features to
  nucleotide contigs, in meta mode (the default) or single mode, with a choice of genetic
  code, `--closed` and `--mask`. **`domainate`** and **`domain-search`** now accept
  upstream's `--gene_call all|unannotated`, and its output is byte-identical to the Python
  tool's. Gene calling runs the external GPL program
  [Orphos](https://github.com/FullHuman/orphos), version 0.4.0 or newer. The app's
  **External tools** panel can download it, or you can set `$DOMAINATOR_ORPHOS`. It is
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
  kept in `.domainator/jobs/<job>/`. Database index builds also run in the background and
  can be canceled. Setting `DOMAINATOR_INPROCESS_JOBS=1` runs jobs inside the app as
  before; that escape hatch will be removed in the next release.
- Because app jobs now go through the CLI, some tools respect options they used to ignore.
  **matrix-report** honors `merge_impact_metric`, `max_merge_events` and `json`, and
  **hmmer-report** writes its `json` output (it used to write JSON into the TSV path).
  Every tool now uses exactly the CLI's defaults and checks.
- The `domainator-cli` crate is now a library (`domainator_cli::run`) with a thin binary;
  the CLI itself is unchanged.
- `domainator --version`, the GUI About box, the diagnostics blob and job provenance now
  report the upstream version and commit the build is at parity with.
- `.dsnv` bundles record the upstream version in `domainator_version`, matching what the
  Python tool and the viewer write, so the bundle is now identical to Python's field for
  field.
- **Partial features and fragments** (upstream 0.12.0). Every tool that slices a record
  (`select-by-cds` and `domain-search` neighborhoods, `select-by-coord`, `extract-domains`,
  `extract-peptides`, `extract-unannotated`, `trim-contigs`, `clean-sequences --strip`,
  `enum-report --by cds|domain`) now keeps a feature the slice cuts. The feature is
  truncated, and its cut end is marked `<` or `>`. Markers already present on ends that
  were not cut are kept. A cut CDS gets an updated `/codon_start` and a `/translation`
  trimmed to the codons the slice keeps. A cut `Domainator`/`Domain_Search` hit is dropped.
  Before this, cut features were dropped, or truncated and left unmarked.
- `/codon_start` is honored. Domain hits are placed relative to the reading frame, and a
  missing translation is made in that frame. `extract-peptides` maps coordinates in that
  frame. A partial CDS whose `/translation` includes residues encoded off the edge of the
  contig is trimmed to the on-contig residues before it is searched.
- New `--partial include|exclude|only` on `domainate`, `domain-search`, `select-by-cds`,
  `select-by-contig`, `extract-peptides`, `extract-domains`, `enum-report` and
  `summary-report`. It selects by fragment status: a CDS with a `<`/`>` end, or a protein
  record marked as a fragment (a ` (Fragment)` header, a `Flags: Fragment` definition,
  `NON_TER` features, or a spanning partial CDS/Protein feature). `enum-report` gains
  `--fragment` and `--partial_count` columns. `summary-report` reports
  `partial_cdss` and `fragment_proteins`, and the app's Summary tab shows the same two
  counts as **partial CDS** and **fragments** tiles.
- Gene-caller partial codes (`/partial="10"` from prodigal or GeneMark) are converted to `<`/`>`
  markers when a GenBank file is read, and the qualifier is removed. An end within 2 bases
  of the contig edge is extended to the edge, with `/codon_start` updated. A bare
  `/partial` (no value) is kept, and makes its CDS a fragment with unknown missing ends
  (`?`).
- Fixed, found while syncing: header text in GenBank files is wrapped the way Biopython wraps
  it, so lines can now be the full 80 columns. A reverse-strand CDS ending at the last base
  of a contig no longer sorts ahead of every other CDS in `select-by-cds`. Sliced records
  from `trim-contigs` and `clean-sequences --strip` drop header annotations (and so get
  today's date), as every other slice does. `extract-domains` no longer lets a `--domains`
  match skip the `--evalue`/`--score` checks.
