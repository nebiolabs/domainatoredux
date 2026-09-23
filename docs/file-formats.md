# File formats

What each file Domainator reads or writes actually contains, and how it relates to the
Python suite's files.

## Compatibility with the Python suite

Files written here are readable by the Python Domainator suite, and files written by the
Python suite are readable here. The guarantee is **semantic, not byte-for-byte**: a file
round-trips to an equal record under the Python suite's own comparators, but the bytes may
differ in places that carry no meaning — JSON object key order, for instance.

In practice most output *is* byte-identical, and where it deliberately isn't,
[Porting status](porting-status.md) and the [parity harness](parity.md) record it.

## GenBank — the working format

`.gb` `.gbk` `.genbank` `.gbff`

Domainator's primary format, because it carries sequence *and* annotations together (see
[Concepts](concepts.md#genbank-carries-both-sequence-and-annotation)).

Feature types Domainator writes:

| Feature | Meaning |
| --- | --- |
| `Domainator` | a domain annotation — this region matched this reference profile. `domainate` writes these. |
| `Domain_Search` | the single best hit that caused this record to be selected. `domain-search` writes one per focus CDS. |
| `CDS` | a coding sequence, either from the input or from gene calling done elsewhere. |

Qualifiers worth knowing:

- **`/domainator_<db>`** on a CDS — a rollup of that CDS's domain annotations for one
  reference database, e.g. `/domainator_Pfam="Sod_Fe_N; Sod_Fe_C"`. Regenerated whenever a
  file is written, so it always matches the features present.
- **`/Color`** — a hex color honored by the [contig viewer](gui.md#viewers), Geneious
  and other viewers. Written by [`color-genbank`](cli-reference.md#color-genbank).
- **`/evalue`, `/score`, `/identity`** on a domain feature — the search statistics for
  that hit.
- **`/cds_id`** — a stable identifier for a CDS.

Two conventions that matter if you compare output between implementations:

- **Circular contigs.** A feature or an extracted window may cross the origin, in which
  case its location has several parts, listed in the order the bases are read rather than
  in ascending coordinate order.
- **Extracted records are renamed** for the region they came from — `contig_1391:2` — with
  an `rc` suffix when the slice is reverse-complemented. The suffix goes on the LOCUS
  name.

A record with no date is stamped with today's date when written, matching the Python
suite's behavior (ours in UTC, theirs in local time).

## FASTA

`.fasta` `.fa` `.fna` `.faa` `.pep`

Accepted as input everywhere, and offered as output by tools with a `--fasta_out` flag,
plus [`genbank-to-fasta`](cli-reference.md#genbank-to-fasta). It cannot carry annotations,
so it is an endpoint rather than an intermediate. Use `--fasta_type` to tell a tool whether
to read FASTA as protein (the default) or nucleotide.

## Score matrices

`.hdf5` `.hdf` `.h5` (binary), `.tsv` (text)

A pairwise matrix of scores between sequences, profiles or contigs, produced by
[`seq-dist`](cli-reference.md#seq-dist),
[`compare-contigs`](cli-reference.md#compare-contigs) and
[`transform-matrix`](cli-reference.md#transform-matrix).

Three representations, chosen by which output flag you pass:

| Flag | Form | Use when |
| --- | --- | --- |
| `--dense` | dense HDF5 | the usual choice |
| `--sparse` | CSR (sparse) HDF5 | most pairs score zero, so the dense form would be huge |
| `--dense_text` | TSV | you want to read it in R, Excel or by eye |

The format is the Python suite's: same HDF5 layout and attributes, readable by its
`DataMatrix.from_file`, and the file extension selects the reader. Row and column labels
are stored with the data; a matrix with column labels is asymmetric (query × reference),
one without is symmetric (all-vs-all).

`transform-matrix` converts freely between the three, so `--dense` in and `--sparse` out
is a conversion.

## Networks

### `.dsnv` — the interactive viewer bundle

Written by [`build-ssn-viewer`](cli-reference.md#build-ssn-viewer): a gzipped JSON bundle
holding the nodes, the maximum spanning tree, and the cluster hierarchy — **and nothing
else**. Everything a viewer shows (split events, slider stops, cluster membership at a
given cutoff) is derived from those when the file is read.

That is why there is no threshold and no event cap baked into the file: you choose them
while looking at the network, in the viewer or with
[`ssn-navigator`](cli-reference.md#ssn-navigator).

`.ssnv` is the older spelling of the same thing. It is still read; new files are always
written as `.dsnv`.

With `--html` you also get a standalone HTML page, and with `--embed_data` the bundle is
embedded inside it, so the page is fully self-contained and can be emailed or put on a
web server with nothing else alongside it.

### `.xgmml` — Cytoscape

Written by [`build-ssn`](cli-reference.md#build-ssn). A static network: the edges are
fixed at the `--lb` cutoff you chose, unlike the `.dsnv` bundle. Node attributes carry any
metadata you joined on with `--metadata`, and `--cluster` adds a cluster id.

## HMM profiles

`.hmm` `.hmmer` (optionally `.gz`)

HMMER3 profile files, interchangeable with HMMER and pyhmmer. Domainator writes the
`HMMER3/f` format including the Pfam `GA`/`TC`/`NC` gathering cutoffs, so
`hmmsearch --cut_ga` still works on a file that has been through
[`hmmer-select`](cli-reference.md#hmmer-select) — which reproduces the profiles it keeps
byte for byte.

**Compression.** A `.hmm` input may be gzip- or BGZF-compressed anywhere a plain one is
accepted. The decision is made from the file's **magic bytes**, not its name, so a
compressed file under any name reads correctly, and so does an uncompressed file that
happens to be named `.gz`. The database name annotations are labeled with ignores the
compression suffix: `pfam.hmm` and `pfam.hmm.gz` both annotate as `pfam`, so compressing a
reference never changes the output.

Output is deliberately asymmetric with sequence files: an `-o` path ending in `.gz` is
written as **plain gzip**, and `.bgz`/`.bgzf` is **refused** before any work is done. HMMER
cannot open a BGZF profile file (easel rejects its header), so writing one would produce a
file `hmmsearch` and pyhmmer could not read; and BGZF's only advantage, seekability, is
useless in a profile file. Output to stdout is never compressed, and `--max_output_gb`
counts the profile bytes *before* compression. This is the opposite of the advice for
[formatted databases](#formatted-databases), which should be BGZF.

`hmmpress` sidecars (`.h3f` `.h3i` `.h3m` `.h3p`) are **never read**. They are tied to the
`.hmm` only by file name, so a profile file edited after pressing would silently yield the
old profiles. Leftover sidecars are harmless and safe to delete.

Build profiles with [`hmmer-build`](cli-reference.md#hmmer-build) from a **Stockholm**
alignment (`.sto` `.stockholm` `.sth`), the only alignment format currently read.

Profiles built here are re-readable by pyhmmer and carry the same NAME, LENG and DESC, but
their emission probabilities are not byte-identical to `hmmbuild`'s — a different builder
and calibration. For identical profiles, build with HMMER and use them here.

## Formatted databases

For repeated searching of something large, [`format-db`](cli-reference.md#format-db)
prepares a directory:

- **shards** (`--shards N`) split the records across files so they can be searched in
  parallel;
- **BGZF compression** (`--compress`, or `--bgzf`) — block-gzip, which is gzip-compatible
  to any reader but *seekable*, so a record can be fetched without decompressing the whole
  file;
- a **`.didx` index** (`--index`) mapping record names to virtual offsets.

Pass the directory as `-i` to [`domain-search`](cli-reference.md#domain-search).

These are hard on-disk contracts and the `.didx` files are byte-compatible with the Python
suite's, in both directions. Two consequences:

- **Plain gzip is rejected.** A `.gz` that is not BGZF cannot be seeked into, so the whole
  indexed-I/O layer refuses it rather than silently falling back to a full scan. Recompress
  it with `format-db --compress`.
- **An index is checked for staleness** by size and modification time, and a stale one is
  an error, not a warning.
- Having both an unsharded file and a shard directory present is an error — Domainator
  will not guess which you meant.

## Tables

`.tsv` `.csv` `.tab`

Used for report output ([`enum-report`](cli-reference.md#enum-report),
[`hmmer-report`](cli-reference.md#hmmer-report)), for network metadata
(`--metadata`), for color tables (`--color_table`), and for coordinate lists
([`select-by-coord`](cli-reference.md#select-by-coord)). Plain delimited text with a header
row; editable in the [Table viewer](gui.md#viewers) or any spreadsheet.

## JSON

Report tools emit machine-readable output with `--json`, which is independent of `-o`.
See [Config files and agent output](config-and-agents.md) for the shape of each.
