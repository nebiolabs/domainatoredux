# The desktop app

Domainator's desktop app is a workspace browser with viewers: you point it at a folder of
data files, click a file to open it in an appropriate viewer, and run tools from a menu.
It is inspired by Geneious, and it is the best way to *look* at Domainator's output —
especially contigs and similarity networks.

Install it from the [Releases page](https://github.com/nebiolabs/domainatoredux/releases);
see [Installation](installation.md).

## It browses a real folder

There is no "import into a project" step. The app reads and writes **ordinary files in an
ordinary directory**, so anything you produce on the command line shows up in the app, and
anything the app produces can be fed straight back into the CLI.

Click **Open workspace…** and choose a folder. On later launches it reopens the folder you
were last in, and offers the others in a **Recent** list. A workspace whose folder is
currently unavailable (an unplugged drive, say) is hidden rather than forgotten.

App state lives in a single hidden directory at the workspace root, `.domainator/`:
the file manifest, the database registry, job provenance records, per-file notes
(description and colour), and any subsets the SSN viewer extracts. **Your data files are
never rewritten to carry app state.** `.domainator/` is hidden from the tree, refused as
a folder name, and protected from deletion.

## The layout

- **Left sidebar** — the folder tree, plus the Jobs, Databases, Reference databases and
  External tools panels.
- **File table** — the recognized data files in the selected folder, with type, size and
  record count. Click a row to open it.
- **Viewer dock** — a tab per open file. Each file type offers one or more **sub-views**,
  shown as a second row of tabs. The ⧉ button pops any tab into its own window, which is
  how you compare two contigs side by side.
- **Status bar** — workspace path, running jobs, unsaved changes.

There is a native menu bar (File / Edit / View / Tools / Window / Help) with the usual
accelerators: Ctrl+O open workspace, Ctrl+S save, Ctrl+W close tab, Ctrl+Shift+T reopen
closed tab, Ctrl+F find, Ctrl+, preferences, F5 refresh, Ctrl+1 / Ctrl+2 toggle the
sidebar and file table, Ctrl+Shift+O pop out the current tab.

## Viewers

| File | Sub-views | What you do there |
| --- | --- | --- |
| `.gb` `.gbk` `.genbank` `.gbff`, `.fasta` `.fa` `.fna` `.faa` `.pep` | **Contig View** | Scroll and zoom a gene map. Joined features and circular origin-wrapping are drawn correctly. Filter by feature kind, by database, or with an E-value slider. The toolbar shows the cursor's coordinate and the selection's `start:end` and length as you drag. Select a range and copy it as nucleotides or as a translation (Ctrl/Cmd+Shift+C); extract a selection or whole contigs to a new file. Click a feature to pin its full annotation — the same fields the hover tooltip shows — and, for GenBank, recolour or delete it from the bottom of that panel, or add an annotation. **View** adjusts lane and contig spacing, ruler and label font sizes, bp-vs-kb tick numbering, feature labels inside / outside / both, and whether each contig gets its own ruler or one shared ruler is pinned across them all. **Export** saves the current view or the whole file as SVG or PNG. Very dense contigs collapse to a density strip until you zoom in. |
| | **Enum Report** | The [`enum-report`](cli-reference.md#enum-report) table, live: per contig, CDS or domain, sortable, with per-column filters and a picker over ~19 analyses. |
| | **Summary** | A dashboard of the dataset: counts, a contig-length histogram, top domains / databases / organisms, and a taxonomy icicle plot built from the records' own lineage — no taxdump needed. This one has no CLI equivalent. |
| `.hdf5` `.hdf` `.h5` | **Matrix Report** | The score histogram and suggested clustering cutoffs. Click a threshold to select it. |
| `.dsnv` (and legacy `.ssnv`) | **SSN Viewer** | The interactive network: drag the threshold slider to split it into clusters, switch between force-directed, grid, packed and treemap layouts, edit and chart metadata columns, colour by gradient, and save sessions. Select nodes and click **Open sequence(s)** to pull those records out of the workspace into a new sequence tab. |
| `.hmm` `.hmmer` | **Profiles** | A filterable table of every profile in the file. Opens instantly even for tens of thousands of profiles, because it never computes a logo up front. Extract a subset to a new `.hmm`. |
| | **Logo** | A sequence logo for one profile, with an occupancy strip, horizontal virtualization, and **Export SVG**. |
| `.tsv` `.csv` `.tab` | **Table** | View and edit a delimited table. |
| `.txt` `.text` `.list` | **Text** | A plain-text editor. |
| `.svg` | **Image** | View a rendered SVG with zoom. |
| `.sto` `.stockholm` `.sth`, `.cm` | — | Recognized so they appear in tool dropdowns; no viewer. |

## Running tools

The **Tools** menu runs 28 of the command-line tools, grouped as Annotation & search,
Selection & extraction, Matrices & networks, Reports, HMM profiles, Colour and
Conversion.

Each dialog is generated from the same JSON Schema the CLI prints
(`domainator schema <tool>`), so the options, their types and their help text are exactly
the CLI's — they cannot drift apart. Booleans render as checkboxes, enumerations as
dropdowns, lists as comma-separated fields, and every option carries its description
inline.

- **Input fields** are dropdowns of the files in the current folder whose type fits, plus
  any matching **registered databases** (marked `(db)`).
- **Output fields** are filenames; the app fills in a sensible default from the input's
  name and writes into the folder you are looking at.
- Output types are chosen so that pipelines chain: the matrix tools write `.hdf5`, which
  appears in the SSN tools' input dropdowns, whose `.dsnv` opens in the SSN viewer.

The whole SSN pipeline runs from the menu: **domain-search → domainate → seq-dist (or
compare-contigs) → transform-matrix → build-ssn / build-ssn-viewer**, including
neighborhood extraction, taxonomy filters and decoys.

## Jobs and provenance

Runs happen off the UI thread and appear in the **Jobs** panel with status, result
message and clickable output files. Right-click a job to clone its parameters into a fresh
dialog, copy its parameters as JSON, reveal its output, or remove it from the history.

Every completed run writes a provenance record under `.domainator/jobs/`, and stamps each
output file with the job that made it. That record is what lets the SSN viewer trace a
network back to the sequence file it came from, which is how **Open sequence(s)** works.

**Cancel is best-effort.** It stops a job that has not started yet, but a tool already
running will finish — none of the core tools currently have interruption points. Any
output it had already written is left in place rather than deleted. The job message says
as much.

## Databases

The **Databases** panel is a workspace-wide registry of reference files: add a `.hmm` or a
formatted database once, and it appears in the reference dropdown of every tool that takes
one, from any folder. Each entry shows its molecule type, format and shard count.

The panel registers databases that are *already formatted*; to build one, use
[`format-db`](cli-reference.md#format-db) on the command line — or let the app fetch and
format one for you, below.

## Reference databases

The **Reference databases** panel downloads and formats the public databases most runs
start from. Click **Download** and the app fetches the file, converts it to the form the
tools need, and registers it in the Databases panel, ready for `domainate`,
`domain-search` and `seq-dist`.

| Database | Download | On disk | What it is |
| --- | --- | --- | --- |
| UniProtKB/Swiss-Prot | ≈90 MB | ≈105 MB | Manually reviewed, curated proteins. The one to start with. |
| UniRef50 | ≈8 GB | ≈9 GB | UniProt clustered at 50% identity. |
| UniRef90 | ≈30 GB | ≈34 GB | UniProt clustered at 90% identity. |
| UniProtKB/TrEMBL | ≈38 GB | ≈43 GB | The full unreviewed knowledgebase. Expect hours. |
| Pfam-A | ≈400 MB | ≈1.6 GB | The Pfam-A profile HMM library. |

Everything but Swiss-Prot asks you to confirm the size first, and any download can be
cancelled while it runs; a cancelled or failed download leaves nothing behind.

**Free space is checked first.** The panel shows how much room is free on the volume the
databases live on, and a download that cannot fit is disabled with what it would need —
no transfer starts only to die three hours in. The "on disk" column above includes the
`.didx` index, which is about a tenth of a sequence database again. An **update** needs
more room than a first install: the new copy is written alongside the old one and only
replaces it at the end, so both exist at the peak. If the platform will not report free
space, nothing is blocked on it.

**How they are stored.** Sequence databases are written as **BGZF** with a `.didx` index —
the same formatting [`format-db`](cli-reference.md#format-db) produces, which is what lets
`domain-search` seek into them instead of rescanning. The published files are plain gzip,
which is not seekable, so they are recompressed on the way in; that happens in the same
pass as the download, so a 40 GB database never needs a 250 GB uncompressed intermediate.
Pfam-A is stored **uncompressed**, because nothing reads a compressed profile library. The
index is built after the download, as a second pass over the file, and the panel shows it
as a separate step.

**Versions.** Each database's release (`2026_03` for UniProt, `38.2` for Pfam) is read
from the publisher's own release file at download time and shown next to it. Nothing is
checked automatically: press **Check** to ask whether a newer release exists, and if one
has appeared the row offers **Update**, which re-downloads in place. The answer to the last
check is remembered, so the panel still shows it after a restart.

The files live in the app's data directory, not in the workspace, so they are downloaded
once and shared by every workspace you open — opening a new workspace registers the ones
you already have. Removing a database from this panel deletes the downloaded files;
removing it from the **Databases** panel only unregisters it from that workspace.

## External tools

Three optional programs can be installed for you — click **Download** in the **External
tools** panel and the app fetches the binary, installs it into its own data directory, and
sets the environment variable the tools look for.

| Program | Version | Needed for | Platforms |
| --- | --- | --- | --- |
| `diamond` | 2.2.5 | `seq-dist --algorithm diamond_*`, `deduplicate --algorithm diamond` | Linux x64, macOS (arm64 and Intel), Windows x64 |
| `cdhit` | 0.1.0 | `deduplicate --algorithm cd-hit` | Linux x64, macOS arm64, Windows x64 — shown as `n/a` on Intel macOS |
| `usearch` | 12.0-beta1 | `deduplicate --algorithm usearch` | Linux x64, macOS (arm64 and Intel), Windows x64 |

Nothing else in Domainator needs an external program; the HMMER3 implementation is built
in. cd-hit and usearch are GPL-licensed and are downloaded at your request, never bundled.

## Preferences and diagnostics

**Preferences** (Ctrl+,) covers reopening the last workspace on launch, light/dark/system
theme, the default thread count, and whether to check for updates at startup. It also
shows where the app keeps its data.

**About** reports the app, Tauri and webview versions, your OS and architecture, and the
installed external tools, with a copyable diagnostics blob — attach it when
[filing an issue](https://github.com/nebiolabs/domainatoredux/issues).

## Current limitations

Worth knowing before you hit them:

- **Cancelling a running tool doesn't stop it** (see above).
- **No job queue and no progress bars.** Runs are not throttled, so two or three heavy
  jobs at once will saturate every core and make the window sluggish. Jobs report only
  start and finish, not progress.
- **Compressed sequence files don't open in viewers.** BGZF and gzip inputs work fine as
  *tool* inputs, but the contig viewer needs an uncompressed file.
- **FASTA is read-only in the contig viewer** — there is nowhere in a FASTA file to store
  features.
- **No static image export from the contig viewer** yet. The HMM logo does export SVG.
- **The matrix report assumes a symmetric matrix**; a query × reference matrix is not
  meaningfully displayed.
- **`.cm` and Stockholm files have no viewer**, only tool-dropdown recognition.
- **The SSN viewer doesn't follow the app theme** — it is a shared artifact generated from
  the Python viewer.
- Some tools are not in the menu at all: see the GUI column of
  [Porting status](porting-status.md).
