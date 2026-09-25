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
(description and color), and any subsets the SSN viewer extracts. **Your data files are
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

**Saving.** A file's line endings are preserved — the editors detect CRLF / LF / CR on
open, show which in the toolbar, and write the same back, so saving a Windows file on
Linux does not rewrite every line in it. The Table and Text views are editors; everything else writes through as you
act, so there is nothing to save. **File → Save** (Ctrl/Cmd+S) writes the editor you are
looking at, and is grayed out when there is nothing unsaved. If you switch tabs, close the
tab or close the window with unsaved edits, you are asked whether to **Save**, **Discard**
or **Cancel** first. Files above 64 MB do not open in an editor — they can still be used as
tool inputs.

There is a native menu bar (File / Edit / View / Tools / Help) with the usual
accelerators: Ctrl+O open workspace, Ctrl+S save, Ctrl+W close tab, Ctrl+Shift+T reopen
closed tab, Ctrl+, preferences, F5 refresh, Ctrl+1 / Ctrl+2 toggle the
sidebar and file table, Ctrl+Q quit. **File → Quit** closes every window, and each one
still asks about its unsaved edits first.

## Viewers

| File | Sub-views | What you do there |
| --- | --- | --- |
| `.gb` `.gbk` `.genbank` `.gbff`, `.fasta` `.fa` `.fna` `.faa` `.pep` | **Contig View** | Scroll and zoom a gene map. Joined features and circular origin-wrapping are drawn correctly. Filter by feature kind, by database, or with an E-value slider. The toolbar shows the cursor's coordinate and the selection's `start:end` and length as you drag. Select a range and copy it as nucleotides or as a translation (Ctrl/Cmd+Shift+C); extract a selection or whole contigs to a new file. Click a feature to pin its full annotation — the same fields the hover tooltip shows — and, for GenBank, recolor or delete it from the bottom of that panel, or add an annotation. **View** adjusts lane and contig spacing, ruler and label font sizes, bp-vs-kb tick numbering, feature labels inside / outside / both, and whether each contig gets its own ruler or one shared ruler is pinned across them all. **Export** saves the current view or the whole file as SVG or PNG. Very dense contigs collapse to a density strip until you zoom in. |
| | **Enum Report** | The [`enum-report`](cli-reference.md#enum-report) table, live: per contig, CDS or domain, sortable, with per-column filters. **Columns** opens the same column builder as the export (every column type, parameters included), and **Taxonomy** picks the database the taxonomy columns use. |
| | **Summary** | A dashboard of the dataset: counts, a contig-length histogram, top domains / databases / organisms, and a taxonomy icicle plot built from the records' own lineage — no taxdump needed. This one has no CLI equivalent. |
| `.hdf5` `.hdf` `.h5` | **Matrix Report** | The score histogram and suggested clustering cutoffs. Click a threshold to select it. |
| `.dsnv` (and legacy `.ssnv`) | **SSN Viewer** | The interactive network: drag the threshold slider to split it into clusters, switch between force-directed, grid, packed and treemap layouts, edit and chart metadata columns, color by gradient, and save sessions. Select nodes and click **Open sequence(s)** to pull those records out of the workspace into a new sequence tab. |
| `.hmm` `.hmmer` | **Profiles** | A filterable table of every profile in the file. Opens instantly even for tens of thousands of profiles, because it never computes a logo up front. Extract a subset to a new `.hmm`. |
| | **Logo** | A sequence logo for one profile, with an occupancy strip, horizontal virtualization, and **Export SVG**. |
| `.tsv` `.csv` `.tab` | **Table** | View and edit a delimited table. Every row is editable however long the file is — rendering is virtualized, and past the point where a browser stops growing a scrollbar the scroll position maps onto the row range instead. |
| `.txt` `.text` `.list` | **Text** | A plain-text editor. |
| `.svg` | **Image** | View a rendered SVG with zoom. |
| `.sto` `.stockholm` `.sth`, `.cm` | — | Recognized so they appear in tool dropdowns; no viewer. |

## Running tools

The **Tools** menu runs 28 of the command-line tools, grouped as Annotation & search,
Selection & extraction, Matrices & networks, Reports, HMM profiles, Color and
Conversion. The **Tools** button in the middle of the toolbar offers the same groups cut
down to the tools that take the selected file as an input, and the dialog it opens starts
with that file filled in.

Each dialog is generated from the same JSON Schema the CLI prints
(`domainator schema <tool>`), so the options and their types are exactly the CLI's — they
cannot drift apart. Booleans render as checkboxes, enumerations as dropdowns, lists as
comma-separated fields, and every option carries its description inline. Where the app
starts an option at a different value from the command line, the dialog says so under it.

- **Input fields** are dropdowns in two groups: the matching **registered databases**, and
  the files in the current folder whose type fits. Depending on the field, one group or the
  other comes first, and the dropdown starts on the file you have selected or on a
  database such as Pfam-A.
- **Output fields** are filenames; the app fills in a sensible default from the input's
  name and writes into the folder you are looking at.
- Output types are chosen so that pipelines chain: the matrix tools write `.hdf5`, which
  appears in the SSN tools' input dropdowns, whose `.dsnv` opens in the SSN viewer.
- **Report columns** (enum-report and hmmer-report) are an ordered list: add a column, pick
  its type, and fill in what that type needs: ranks for a taxon name, a feature and
  qualifier, a PROSITE pattern, a pH. The order of the list is the order of the columns
  in the report, and ↑/↓ reorder it. Common ranks and qualifiers are one-click chips, and
  anything else can be typed.
- **Taxonomy** (taxonomy filters, and enum-report's taxid / taxon-name / lineage columns)
  comes from the **Taxonomy database** field, which starts on the NCBI Taxonomy download
  from [Reference databases](#reference-databases) once you have installed it.

The whole SSN pipeline runs from the menu: **domain-search → domainate → seq-dist (or
compare-contigs) → transform-matrix → build-ssn / build-ssn-viewer**, including
neighborhood extraction, taxonomy filters and decoys.

## Jobs and provenance

Each run happens in its own background process and appears in the **Jobs** panel with
status, result message and clickable output files. A tool run from the app runs the same
code as `domainator <tool>` on the command line, with the dialog's settings passed as a
config file; the config and the tool's full output are kept in
`.domainator/jobs/<job>/worker-config.json` and `worker.log`. Right-click a job to clone its parameters into a fresh
dialog, copy its parameters as JSON, reveal its output, or remove it from the history.

Every completed run writes a provenance record under `.domainator/jobs/`, and stamps each
output file with the job that made it. That record is what lets the SSN viewer trace a
network back to the sequence file it came from, which is how **Open sequence(s)** works.

**Jobs at once.** By default two jobs run at the same time and the rest wait in the queue,
marked *queued*. Change the number in **Preferences → Jobs at once**. Each job uses the
**Default threads** setting (all cores unless you change it), so running several at once
mostly makes them share the cores.

**Cancel** stops a running job right away, along with any program it started (diamond,
cd-hit, usearch). A queued job is simply removed from the queue. A tool stopped partway
can leave a truncated file that still looks valid, so a cancel **deletes the job's
declared output files that it created or changed**, and the job message lists them. Only
files named in the tool's output fields are ever deleted. If an output file existed
before the run and the tool had already overwritten it, its old contents were gone
before you pressed Cancel; the message says so.

If the app closes or crashes while jobs are running, their processes stop too. Those
jobs show as interrupted next time, and their outputs may be incomplete.

**Building a database index** (Databases panel) also runs in the background and can be
canceled; a canceled build removes its partial index.

## Databases

The **Databases** panel lists the reference files your tools search against or annotate
with. The list is **shared by every workspace**: add a `.hmm` or a formatted database once,
and it appears in the reference dropdown of every tool that takes one, in any workspace and
from any folder. A database is registered by path and never copied — it stays where it is
on disk, so a 40 GB file costs nothing to add. Each entry shows its molecule type, format
and shard count; the **?** beside the heading explains how databases differ from the files
in a workspace.

Right-click a database for the same actions as a file: **Open**, **Open in New Window**,
**Rename…**, **Copy Name**, **Copy Full Path**, **Show in File Manager** and **Delete…**.
Rename and Delete act on every file the database is made of — each shard and its `.didx`
index — and Delete lists those files before it removes anything. **Remove from List** (or
the ×) forgets a database without touching its files. A sharded database has no single file
to open, and a managed download cannot be renamed.

**Indexes.** A FASTA or GenBank database with no `.didx` index — or one whose data has
changed since it was indexed — shows a **Build index** button. Searches work without one,
but have to scan the whole file first; the index lets `domain-search` seek, and gives
`-Z 0` its target count instantly. The index is written beside the data, not as a copy. A
plain-gzip database is tagged `gzip`: it cannot be indexed until it is recompressed as BGZF
with [`format-db`](cli-reference.md#format-db) (`--compress`). HMM libraries need no index.

Each panel in the sidebar can be resized by dragging the grip under it; double-click the
grip to fit the panel to its contents again.

The panel registers databases that are *already formatted*; to build one, use
[`format-db`](cli-reference.md#format-db) on the command line — or let the app fetch and
format one for you, below.

## Reference databases

The **Reference databases** panel downloads and formats the public databases most runs
start from. Click **Download** and the app fetches the file, converts it to the form the
tools need, and registers it in the Databases panel, ready for `domainate`,
`domain-search` and `seq-dist`, or, for NCBI Taxonomy, for any tool's taxonomy options.

| Database | Download | On disk | What it is |
| --- | --- | --- | --- |
| UniProtKB/Swiss-Prot | ≈90 MB | ≈105 MB | Manually reviewed, curated proteins. The one to start with. |
| UniRef50 | ≈8 GB | ≈9 GB | UniProt clustered at 50% identity. |
| UniRef90 | ≈30 GB | ≈34 GB | UniProt clustered at 90% identity. |
| UniProtKB/TrEMBL | ≈38 GB | ≈43 GB | The full unreviewed knowledgebase. Expect hours. |
| Pfam-A | ≈400 MB | ≈400 MB | The Pfam-A profile HMM library, kept gzipped. |
| NCBI Taxonomy | ≈80 MB | ≈80 MB | The NCBI taxonomy tree (`taxdump.tar.gz`), for taxonomy filters and enum-report's taxonomy columns. |

Everything but Swiss-Prot, Pfam-A and NCBI Taxonomy asks you to confirm the size first, and any download can be
canceled while it runs; a canceled or failed download leaves nothing behind.

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
Pfam-A is kept as the **plain gzip** file EBI publishes (`Pfam-A.hmm.gz`): every tool reads
a gzipped profile library directly, and nothing seeks in one, so there is nothing to gain
from expanding it to 1.6 GB. (An install made by an earlier version, stored as `Pfam-A.hmm`,
is replaced by the gzipped file on its next update.) NCBI Taxonomy is kept as the
`taxdump.tar.gz` NCBI publishes, which the tools read without unpacking. The
sequence-database index is built
after the download, as a second pass over the file, and the panel shows it
as a separate step.

**Versions.** Each database's release (`2026_03` for UniProt, `38.2` for Pfam) is read
from the publisher's own release file at download time and shown next to it. NCBI does
not number taxonomy releases, so NCBI Taxonomy shows the start of the file's MD5
checksum (`md5 7b5d0589`), which changes whenever NCBI rebuilds the file. Nothing is
checked automatically: press **Check** to ask whether a newer release exists, and if one
has appeared the row offers **Update**, which re-downloads in place. The answer to the last
check is remembered, so the panel still shows it after a restart.

The files live in the app's data directory, not in the workspace, so they are downloaded
once and shared by every workspace you open. Removing a database from this panel deletes
the downloaded files; **Remove from List** in the **Databases** panel only takes it off the
shared list.

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

- **No progress bars.** Jobs report only start and finish, not progress. The job's
  `worker.log` (above) shows what the tool has printed so far.
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
