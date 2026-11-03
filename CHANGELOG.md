# Changelog

This project uses its own [SemVer](https://semver.org/) numbering, independent of the
Python `domainator` package. Each entry says which upstream release (and commit) it is at
parity with. How versions are assigned: `docs/development.md`, "Versioning and releases".

## [Unreleased]

### Fixed

- **`genbank_to_fasta` reads its input as GenBank whatever the file is called**, as upstream
  does, so an input staged under a fixed name such as `input_seqs` converts instead of
  failing with "could not detect file type". A `.fasta`/`.fa`/... input is still copied as
  FASTA.
- **`deduplicate_genbank --algorithm cd-hit` finds C cd-hit.** When no `cdhit` (the Rust
  port) is found, `cd-hit` and `cd-hit-est` on `PATH` are used, as upstream uses them, so a
  bioconda environment set up for the Python `domainator` works unchanged. `doctor` shows
  which one was found.

### Changed

- **The `-cuda` builds start on any machine** (#20). They load the CUDA libraries when the
  GPU is first used instead of linking them, so without the driver, cuBLAS or cuRAND they
  run on the CPU and print a warning that names what is missing, instead of failing to
  start. NVRTC is no longer needed. `doctor` lists each CUDA library and whether it was
  found. This uses a vendored candle-core with one line changed (`vendor/candle-core/`).
- **The Linux command-line download runs on glibc 2.28 and newer** (RHEL 8, Ubuntu 20.04
  and later). It is now built in manylinux_2_28. The previous build needed glibc 2.34, and
  on EL9 the loader warned about weak symbols from glibc 2.39.
- **`domainate --foldseek` with several databases runs ProstT5 once**, not once per
  database: the queries' 3Di sequences depend only on the queries, so only `foldseek search`
  repeats per database. Output is unchanged.
- `compare_contigs`' help and its sort-memory error name `--genbank_out` instead of 0.x's
  `-o`, and a non-hdf5 `-o/--hdf_out` now fails with an error that names the path. For
  `compare_contigs`, the error adds that 0.x's `-o` (the sorted GenBank) is `--genbank_out`
  now. This follows upstream `c807588`.

## [1.0.0-rc.1] - 2026-11-02

Parity: Python domainator 1.0.0-rc.2 @ `67bad08`.

- First release under this versioning scheme. The earlier `v0.11.0` tag predates it.
- **The Domainator Viewer** replaces the SSN viewer: one offline page, generated from
  upstream's, for networks, trees and projections, with the same metadata table, editing,
  colors, presets, charts and saved sessions in all three. Its data is a `.dvb` bundle.
  - **`build_ssn --html_out` and `--dvb_out`** write the network for it (with
    `--merge_impact_metric`, `--label_by`, `--categorical` and `--name`), and
    `--xgmml_out` is no longer required. **`build_ssn_viewer` is gone**, as it is upstream.
  - **`build_tree`**, a new tool: a UPGMA tree from a distance or similarity matrix, or a
    Newick tree built elsewhere (`--newick`), written as Newick, as Cytoscape XGMML laid
    out as a rectangular tree, and for the viewer, with an aligned FASTA beside the tree
    (`--msa`).
  - **`build_projection`**, a new tool: a 2D or 3D PCA projection of a matrix's rows, as
    coordinates, XGMML and for the viewer. Upstream's `tsne` and `umap` are not supported
    yet; `--algorithm coords` reads a projection made elsewhere.
  - **`viewer`**, a new tool: the viewer page, opening a bundle or empty.
  - `.dsnv` network bundles are still read, by the viewer, `ssn_navigator` and the
    desktop app, but no longer written.
- **The desktop app** opens `.dvb` and `.dsnv` bundles in one Viewer tab, which can save the
  page as a standalone HTML file, and has a new **Alignment** tab for aligned FASTA
  (`.afa`, or any FASTA file that is an alignment): a residue grid with conservation and
  consensus tracks, row filtering, and export of the selected rows with or without gaps.
  The Tools menu gains Build tree (taking an alignment from the workspace) and Build
  projection, and Build SSN writes a viewer bundle.
- **`enum_report` and `hmmer_report --html_out`** (and `--html_max_height`): upstream's
  new interactive table page, which works offline. It is generated from upstream's own
  page (`scripts/regen_report_shells.py`), so the two write the same file.
- **`summary_report --html_out`**: upstream's new offline page, with a contig length
  histogram that zooms, pans and rebins, sortable domain and co-occurrence tables, and a
  zoomable taxonomy chart. Following NCBI's current ranks, the chart keeps the
  `cellular root` ("cellular organisms") and `acellular root` ("Viruses") nodes beside
  domains, realms and kingdoms; `superkingdom` is gone from NCBI's taxdumps, so ask
  `enum_report` for `taxname:domain` instead.
- **`summary_report -o`** writes upstream's text report, histogram and tables included, so
  it is now the same file `domainator` writes.
- **`matrix_report --html_out` and `--knn_projection`**: upstream's new offline page, with
  clusters and edges by threshold, the split-event chart above a threshold slider, the
  clusters at the chosen threshold, and (with `--knn_projection`) the edges
  `build_ssn --mst --knn k` would keep at that threshold for each `k`. The text report's
  score histogram is now upstream's, so `-o` writes the same file as `domainator`.
- **Infernal covariance models (`.cm`) as references** in `domainate`, `domain_search` and
  `rotate_contigs`: whole contigs are searched on both strands as `cmsearch` does,
  truncated hits at contig ends included, with hits equal to upstream's (coordinates,
  scores and E-values). The search is built in (the infernox crate, a Rust port of
  Infernal 1.1.5), so it needs no Infernal install and runs on Windows.
- **`infernal_build`**, a new tool: builds a CM from an alignment with an external `cmbuild`
  (Infernal's, or infernox's `infernox-cmbuild`) and sets its NAME, ACC and DESC.
- **`plot_contigs`**, a new tool: upstream's contig plot, an offline page that draws each
  contig with its features stacked above it, colored by `--metadata` with `--color_by`.
- **`domain_search` on nucleotide references writes what `domainator` writes**: where a
  contig's search hit and one of its annotations share a start, the hit comes first.
- **`--cds_range` counts protein-coding CDSs only** (`domain_search`, `select_by_cds`):
  hits on a nucleotide reference are never neighbors, though one can be the focus. A
  region used to stop at the next nucleotide hit instead of reaching the next gene.
- **`domainate --foldseek`**: foldseek databases as references, searched by the proteins'
  3Di sequences, which foldseek predicts with its built-in ProstT5 (`--prostt5_weights`,
  `--prostt5_device`, `--foldseek_device`, `--foldseek_path`); needs the foldseek binary
  and `foldseek databases ProstT5`, not PyTorch. Upstream's ESM-2 3B 3Di predictor
  (`--esm2_3di_weights`) is refused. In the desktop app, foldseek databases are added in
  the Databases panel (from any of their files), and the ProstT5 model is a download in
  Reference databases.
- **`find_features`**, a new tool: PROSITE motif matches (`--motif`) on every protein and
  CDS translation as domain annotations, and TMbed's transmembrane regions and signal
  peptides (`--tmbed`, reimplemented on candle: no PyTorch, and Windows too), with
  `--gene_call` and `--partial`. In the desktop app as "Find features (motifs, TMbed)".
  The CLI and app can be built with `--features cuda` (or `metal`) to run TMbed on a GPU.
  The app downloads TMbed's ProtT5 model in Reference databases.
- **The desktop app's External tools** add foldseek (Linux, macOS) and reseek (Linux and
  Windows, on a CPU with AVX-512, which its release binary needs).
- **Building needs Rust 1.94** (was 1.89), for candle 0.11, which runs TMbed.
- **An unclosed `[`, `{` or `(` in a PROSITE motif** makes `enum_report`'s
  `motif_count:` column count nothing, as upstream does, instead of the matches of the
  motif with that character dropped.
- **`split_by_cluster`**, a new tool: one file per cluster of a similarity matrix, from
  GenBank, FASTA, `.hmm` or `.cm` input, with small clusters skipped or pooled and
  optionally each contig centered on its search hit. In the desktop app as "Split by
  cluster".
- **`kmer_dist`**, a new tool: a similarity matrix from exact shared k-mers, with no
  alignment and no external binary. Protein records, CDS translations (calling genes with
  `--gene_call`) or strand-invariant nucleotide k-mers (`--nucleotide_kmers`), scored as
  max containment, min containment or Jaccard, with `--max_hits_per_query`, `--mst` and
  `--knn` as on `seq_dist`. In the desktop app as "K-mer distance".
- **The structure tools**, four new tools on PDB/mmCIF structures through an external
  foldseek or reseek: **`structure_to_genbank`** (a protein record per chain, optionally
  with its 3Di string), **`structure_domainate`** (annotate every chain with the reference
  structures that hit it), **`structure_search`** (select the chains a reference hits)
  and **`structure_dist`** (a similarity or distance matrix, with structural modes such as
  `tmscore` and `lddt_dist` beside `seq_dist`'s). Inputs can be files, directories,
  globs or a prebuilt database (`--db_out` keeps one). In the desktop app under the
  same names, with `.pdb`/`.cif` files recognized in the workspace. A structure file given
  to a sequence tool now says which structure tool to use.
- **`db_download`**, upstream's last unported tool: UniProt (SwissProt, TrEMBL or both) as
  FASTA, or as GenBank with UniProt's annotations as features, and NCBI genome assemblies
  as GenBank, over HTTPS or through the NCBI Datasets CLI (`datasets`, now in `doctor`'s
  list), with upstream's taxonomy, date, accession, `--partial` and `--gene_call` options.
  Genomes are appended in assembly-summary order whatever order their downloads finish
  in. `--uniprot_dir` reads a local UniProt mirror, and an `--ncbi_summary` may name local
  genome directories. In the desktop app as "Download a database".
- **GenBank output matches Biopython in three more places**: `PUBMED` reference lines
  get their three-space indent, an `ORGANISM` of 68 characters is no longer truncated
  (only longer ones are, to 64 and `...`), and `KEYWORDS` and the taxonomy lines no longer
  gain a second `.` when the text already ends with one. A location on another sequence
  (`P68250-2:1`, `AL391218.9:105..208`) is read and written with its reference; it used
  to be read as an empty location.
- **A join whose parts lie on different strands keeps them.** A trans-spliced tRNA's
  `join(308963..309002,complement(380329..380364))` was written back as
  `join(308963..309002,380329..380364)`, putting both parts on the forward strand, by any
  tool that rewrote the record. Found in a real genome (Nanoarchaeum equitans) by
  `db_download --gene_call`.
- **`domain_search --kb_up`/`--kb_down` on a circular contig smaller than the window**
  keeps the window where its padding puts it, as upstream does: `--kb_down 100` around a
  plasmid gene starts the region at that gene. It used to be centered opposite the gene.
- **`domain_search --translate --decoys`** leaves out the CDSs whose best hit is a decoy,
  as region extraction already did; it used to write them.
- **The NCBI taxonomy is downloaded when it is missing**, as `domainator` does, and
  **`--taxonomy_update`** (`domain_search`, `enum_report`, `summary_report`,
  `select_by_cds`, `select_by_contig` and the three extract tools) checks a local
  `taxdump.tar.gz` against NCBI's MD5 and replaces an outdated one. Downloads are checked
  against that MD5 before they replace anything, and runs sharing a database wait for
  each other's download. When `--ncbi_taxonomy_path` holds both, the tarball is now read
  before extracted `.dmp` files, as upstream reads them, so an update is not shadowed by
  older tables.
- **`--progress`** draws a progress bar on `seq_dist` (queries scored, for diamond,
  hmmsearch, phmmer and hmmer_compare alike) and `compare_contigs` (contigs scored), as
  upstream's tqdm bars do, but only when stderr is a terminal and not under `-q`.
  `matrix_report --progress` prints a line as each stage starts, and
  `--profile_stages` each stage's time; both used to be accepted and ignored.
- **Compressed GenBank and FASTA inputs are read** by every tool, gzip or BGZF alike, as
  `domainator` reads them. Most tools used to read such a file as plain text, find no
  records in it, and finish with empty output and no error.
- **`seq_dist` explains a failed diamond run** as `domainator` does: running out of
  scratch disk or memory, a rejected `--diamond_args` option or an unreadable input each
  get a cause and suggested fixes, with diamond's own stderr (repeated lines collapsed)
  and the command to reproduce it. The large-search advisory now shows the scratch
  directory's free space.
- **`hmmer_build --window_beta`** derives a nucleotide profile's MAXL from a tail mass
  as hmmbuild does (`0` gives 4×M), where it used to be refused.
- **`hmmer_build` reads every alignment format hmmbuild does**: Clustal (and MUSCLE-style
  headers), interleaved and sequential PHYLIP, SELEX, A2M and PSI-BLAST, besides Stockholm
  and aligned FASTA, guessed as Easel guesses them (the first line, then the file's suffix
  where the content is ambiguous: `.a2m`, `.phys`, `.pb`). PHYLIP names may hold spaces or
  run into the sequence, and a wider name field is worked out from the data; a space in a
  SELEX sequence is a gap. A compressed alignment is read decompressed. Each format is read
  exactly as `domainator` reads it, so an alignment builds the same profile whatever its
  format.
- **GenBank COMMENT lines are read as Biopython reads them**: without trailing spaces,
  blank lines skipped, and the structured comment split out even when its lines are
  padded, as NCBI pads them. Such a record used to be written back with its structured
  comment inside the free text.
- **One selection contract across the selecting tools**, as upstream 1.0 has it:
  - **Taxonomy filters** (`--include_taxids`, `--exclude_taxids`, `--taxonomy_expr`) are
    on `domain_search`, `select_by_cds`, `select_by_contig`, `extract_domains`,
    `extract_peptides` and `extract_unannotated`, and each judges its own candidate (a
    CDS, an annotation, a record, a hit), never the neighborhood around it. The new
    **`--taxonomy_unknown exclude|include|evaluate|error`** decides what happens to a
    candidate with no taxid the taxonomy knows; it used to slip through filters that
    allowed "unidentified". A criterion taxid the taxonomy lacks is now an error.
    `domain_search` filters each protein, CDS and nucleotide hit, where it used to keep
    or drop whole records.
  - **`domainate` annotates and keeps every record**: its taxonomy filters,
    `--hits_only` and `--no_annotations` are gone; `domain_search` selects.
  - **`--dedup none|id|sequence|sequence_rc`** on `domain_search` (default `none`) and
    `select_by_cds` (default `id`) replaces `--deduplicate`, `--no_deduplicate`,
    `--deduplicate_by_sequence` and `--both_strands`. With `--max_hits`, copies no longer
    use up the top N, and the best-scoring copy is kept.
  - **`--has_search_hit`** selects the `domain_search` hits on `select_by_cds`,
    `select_by_contig`, `extract_peptides`, `extract_domains` and `rotate_contigs`;
    `--annotation_type domainator|domain_search|all` now only says which annotations the
    domain criteria read.
  - **`--invert`** on `select_by_cds` and `extract_peptides` negates the selectors only,
    so a `--strand` restriction still holds; with no selectors it is ignored, with a
    warning.
  - Also new: `extract_peptides --domain_expr`, `select_by_cds --name_field`, and
    `deduplicate_genbank --id` (the same as `--identity`). Renamed: `--keep_name` is
    `--keep_parent_id` on the extract tools, and `extract_unannotated`'s `--min_score`
    and `--min_length` are `--min_annotation_score` and `--min_region_length`.
  - A nucleotide sequence's other strand is its reverse complement only, so
    `deduplicate_genbank --both_strands` no longer merges a sequence with its plain
    complement.
- **`extract_peptides` keeps the contig's DBLINK** on each peptide, as upstream does.
- **`summary_report`'s co-occurrence rows** are in alphabetical order of focus domain.
- **The Domainator Viewer** (upstream's): trees can be rerooted at their midpoint or on a
  selection and their children swapped, nodes and labels take several ordered styles,
  with shapes by category, and views export as an image of the whole view. Bundles are
  `.dvb` version 2; version 1 still opens.
- **A taxid missing from the taxonomy database is warned about**, once per taxid,
  suggesting `--taxonomy_update`; it is still reported as unknown (blank taxonomy).
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
  are installed and which tools need them, plus whether this build can use a GPU: the
  NVIDIA GPUs `nvidia-smi` lists, the CUDA version their driver supports, and whether
  this is a CPU, CUDA or Metal build and can open a device).
- **Errors and messages** go to stderr: errors as one line,
  `domainatoredux <tool>: error: …`, progress at upstream's levels (`-q` for errors only,
  `-v` for detail).
- **New tool, `rotate_contigs`**: rotates circular contigs to start at a chosen domain or
  search hit, reverse-complementing them to put it on the forward strand.
- **New options**: `compare_contigs --genbank_out` writes the contigs sorted by similarity
  (`--linkage`, `--leaf_ordering`, `--name_by_order`); `--symmetrize` on `seq_dist` and
  `transform_matrix`; `--threshold`, and `--mst` with `--knn K` (their union) replacing
  `--mst_knn`; `domain_search --anchor`, and circular `--whole_contig` output; `select_by_contig` length, CDS-count, score and record-count
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
