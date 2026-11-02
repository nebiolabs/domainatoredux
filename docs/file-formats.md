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
| `Domain_Search` | the single best hit that caused this record to be selected. `domain_search` writes one per focus CDS. |
| `CDS` | a coding sequence, either from the input or from gene calling done elsewhere. |

Qualifiers worth knowing:

- **`/domainator_<db>`** on a CDS — a rollup of that CDS's domain annotations for one
  reference database, e.g. `/domainator_Pfam="Sod_Fe_N; Sod_Fe_C"`. Regenerated whenever a
  file is written, so it always matches the features present.
- **`/Color`** — a hex color honored by the [contig viewer](gui.md#viewers), Geneious
  and other viewers. Written by [`color_genbank`](cli-reference.md#color_genbank).
- **`/evalue`, `/score`, `/identity`** on a domain feature — the search statistics for
  that hit.
- **`/cds_id`** — a stable identifier for a CDS.

Two conventions that matter if you compare output between implementations:

- **Circular contigs.** A feature or an extracted window may cross the origin, in which
  case its location has several parts, listed in the order the bases are read rather than
  in ascending coordinate order.
- **Extracted records are renamed** for the region they came from, the way every
  Domainator tool names them: `contig_{first}:{last}`, where `first` and `last` are the
  1-based contig positions of the region's first and last bases as written, plus `rc`
  when the region reads the contig's reverse strand. So `contig_1391:2rc` reads from 1391
  down to 2, and `contig_4000:120` (first > last without `rc`) wraps the origin of a
  circular contig. The whole contig is `contig`, or `contig_rc` reverse complemented.
  The name goes on the LOCUS line; ACCESSION/VERSION keep the contig's.

A record with no date is stamped with today's date when written, matching the Python
suite's behavior (ours in UTC, theirs in local time).

### Partial features and fragments

Domainator keeps the GenBank partial markers `<` and `>` on feature locations. `<` means
the feature starts before the given position, and `>` means it ends after it. For example,
`complement(803..>2688)` is a CDS on the minus strand that runs off the end of the contig,
so its 5' end (and its protein's N-terminus) is missing.

**Slicing.** Tools that extract part of a record keep the features the slice cuts, and
mark the cut ends with `<` or `>`. These tools are the `select_by_cds` and `domain_search`
neighborhoods, `select_by_coord`, `extract_domains`, `extract_peptides`,
`extract_unannotated`, `trim_contigs`, `clean_sequences --strip` and
`enum_report --by cds|domain`. A marker already on an end that the slice keeps is kept.
There are three exceptions:

- `source` features don't get new markers.
- `Domainator` and `Domain_Search` features that the slice cuts are dropped, because their
  scores and coordinates describe the whole hit.
- A feature split across the ends of a rotated circular contig is not cut, since all of
  its sequence is still in the record. It is written as a `join` without markers.

**Reading frame and translation.** When a CDS is cut, its `/codon_start` is updated so its
reading frame is preserved. Its `/translation` is trimmed to the residues whose codons are
completely kept, following the NCBI convention that a partial CDS's translation covers only
the part on the contig. If the kept part is not contiguous (an interior exon was cut out),
the `/translation` is removed, and `domainate` translates it again the next time the record
goes through it.

Some files give a partial CDS the translation of the full protein, including residues
encoded off the edge of the contig. When `domainate` and `domain_search` read such a CDS,
they trim the translation to the on-contig residues, from the partial end. If both ends
are partial, they can't tell where the extra residues belong, so they translate the CDS
again from the contig and print a warning. A complete 3' end is assumed to end in a stop
codon, which GenBank translations omit and Domainator's own translations include as `*`.

Domain hits on a CDS are placed relative to its `/codon_start`: residue 1 of the
translation starts `codon_start - 1` bases into the CDS.

**Gene-caller partial codes.** prodigal and GeneMark mark partial genes with a two-digit
code, which some conversions copy into a `/partial` qualifier, e.g. `/partial="10"`. The
first digit is the left (low-coordinate) end and the second the right end, whatever the
strand; `1` means the gene runs off that end (or into a gap). Domainator converts these
codes to `<`/`>` markers when it reads a GenBank file, and removes the qualifier, because the
code is tied to the coordinates and would be wrong after a reverse complement. A partial
end within 2 bases of the contig edge is extended to the edge, with `/codon_start` set if
the 5' end moved; a partial end elsewhere (a gap) is marked where it is. For example,
`2..106 /partial="10"` becomes `<1..106 /codon_start=2`. Existing `<`/`>` markers take
precedence over a code. A bare `/partial` with no value says nothing about which end is
missing, so it is left as is, and its CDS is a fragment with unknown missing ends (`?`).

**Fragment status.** Like a taxid, each protein or CDS has a fragment status:

- A CDS is a fragment if either outer end of its location has a `<` or `>`. Its status is
  `N` (5' end, and so the N-terminus, missing), `C` (3' end missing), or `NC` (both). A
  CDS with a bare `/partial` qualifier (no value) and no markers is `?`.
- For a protein record, a `CDS` or `Protein` feature spanning the record decides, the same
  way. `domain_search --translate` writes such a feature, and NCBI GenPept records carry one.
- Otherwise, UniProt `NON_TER` features on the first or last residue decide: `N`, `C` or
  `NC`.
- Otherwise, a protein record is a fragment with unknown missing ends, `?`, if its
  description contains ` (Fragment)` or ` (Fragments)`, or a `Flags:` list naming
  `Fragment(s)`. The first is how UniProt marks fragments in FASTA headers, e.g.
  `>tr|A0A0|A0A0_ECOLI Superoxide dismutase (Fragment) OS=Escherichia coli OX=562`. The
  second is the `DE   Flags: Fragment;` line of a UniProt text entry. The status is never
  written into descriptions, so headers come back out unchanged.

These tools take `--partial include|exclude|only` (default `include`) to select by
fragment status. `--partial` is never inverted by `--invert`.

| Tool | What `--partial` selects |
| --- | --- |
| `domainate`, `domain_search` | which CDSs/proteins are searched. `domainate` still writes every record. |
| `select_by_cds` | which CDSs regions are extracted around |
| `extract_peptides` | which CDSs peptides are extracted from |
| `extract_domains` | which domains are extracted, by the CDS or protein they are on. Contig-level nucleotide hits are never on fragments. |
| `select_by_contig` | which contigs are selected. A protein record is judged by its own status; a nucleotide contig is a fragment if it contains any partial CDS. |
| `enum_report` | which rows are reported: by contig (as for `select_by_contig`), by CDS, or by domain (by the CDS or protein the domain is on) |
| `summary_report` | which CDSs/proteins are counted. Filtered protein records are skipped. On nucleotide contigs, filtered CDSs and the domains on them aren't counted, but the contig is. The report always gives the number of partial CDSs and protein fragments among those counted. |

`enum_report --columns fragment` adds a `fragment` column with the status (`complete`, `N`, `C`,
`NC` or `?`) for rows that are a single protein or CDS: protein records, CDSs, and domains
on protein records (which get the status of their protein). For other rows it is empty,
with a warning. `enum_report --columns partial_count` adds a `partial_count` column. By contig, it
is the number of partial CDSs on the contig (for a protein record, 1 if it is a fragment).
By CDS, it is 1 if the CDS or protein is a fragment. By domain, it is 1 if the CDS or protein
the domain is on is a fragment, whether or not the domain reaches the missing end. In every
other case it is 0.

FASTA output (`--fasta_out`, `genbank_to_fasta`) carries no locations, so a fragment's
status survives only if its header already says ` (Fragment)`.

## FASTA

`.fasta` `.fa` `.fna` `.faa` `.pep`

Accepted as input everywhere, and offered as output by tools with a `--fasta_out` flag,
plus [`genbank_to_fasta`](cli-reference.md#genbank_to_fasta). It cannot carry annotations,
so it is an endpoint rather than an intermediate. Use `--alphabet` to tell a tool whether
to read FASTA as protein (the default) or nucleotide.

### Aligned FASTA

`.afa` (and any FASTA file whose sequences are all the same length)

A multiple sequence alignment, one gapped sequence (`-` or `.` for a gap) per record.
[`build_tree --msa`](cli-reference.md#build_tree) shows one beside a tree, matching
sequences to leaves by id, and the desktop app's Alignment tab displays one. A file with
a repeated id, or with sequences of different lengths, is not an alignment.

## Score matrices

`.hdf5` `.hdf` `.h5` (binary), `.tsv` (text)

A pairwise matrix of scores between sequences, profiles or contigs, produced by
[`seq_dist`](cli-reference.md#seq_dist),
[`compare_contigs`](cli-reference.md#compare_contigs) and
[`transform_matrix`](cli-reference.md#transform_matrix).

Two outputs, chosen by which flag you pass:

| Flag | Form | Use when |
| --- | --- | --- |
| `-o/--hdf_out` | CSR (sparse) HDF5 | the usual choice; most pairs score zero |
| `--dense_text_out` | TSV, every cell | you want to read it in R, Excel or by eye |

The HDF5 format is the Python suite's matrix file format 2.0: same layout and attributes,
readable by its `DataMatrix.from_file`, and the file extension selects the reader. Row
and column labels are stored with the data; a matrix with column labels is asymmetric
(query × reference), one without has the same labels on both axes (all-vs-all). Zeros
are not stored. A matrix whose values are also exactly symmetric is stored as its upper
triangle, diagonal included (`ARRAY_TYPE=SPARSE_CSR_UPPER_TRIANGLE`), which roughly
halves the file; one that is only nearly symmetric, like raw DIAMOND bit scores, is
stored in full. Versions before 2.0 of either suite cannot read a triangle file, and
fail rather than misreading it.

Version 1.x files, including the dense HDF5 files older versions wrote, are still read.
A file from a later major version is refused.

What a missing (zero) cell means depends on the matrix's `DATA_TYPE`: for scores it
means "no hit", and the network tools treat it as "no edge"; for distances (a
`DATA_TYPE` ending in `_dist`) it means distance 0. The network tools refuse distance
matrices.

`transform_matrix` reads any of them, so `transform_matrix -i m.tsv -o m.hdf5` is a
conversion.

## Networks

### `.dvb` — the Domainator Viewer bundle

Written with `--dvb_out` by [`build_ssn`](cli-reference.md#build_ssn),
[`build_tree`](cli-reference.md#build_tree) and
[`build_projection`](cli-reference.md#build_projection): a gzipped JSON bundle holding a
set of nodes (sequences), their metadata table, the viewer's starting colors and labels,
and one view of them —

- **a network** (`build_ssn`): the maximum spanning tree above `--threshold` and the cluster
  hierarchy, **and nothing else**. Everything the viewer shows (split events, slider stops,
  cluster membership at a given cutoff) is derived from those when the file is read, so
  there is no event cap baked into the file: you choose it while looking at the network,
  in the viewer or with [`ssn_navigator`](cli-reference.md#ssn_navigator);
- **a tree** (`build_tree`): each node's parent, branch length, label and support value,
  and optionally an alignment of the leaves (`--msa`);
- **a projection** (`build_projection`): each node's coordinates.

The viewer's "Save session" writes the same format, with its state added, so a saved
session opens wherever a bundle does. `--html_out` writes a standalone HTML page with the
bundle embedded, which can be emailed or put on a web server with nothing else alongside
it, and [`viewer`](cli-reference.md#viewer) writes one for a bundle you already have.
Upstream defines the exact schema in its `docs/file_formats.md`.

### `.dsnv` — the older network bundle

Networks were written as `.dsnv` bundles before the `.dvb` format held trees and
projections too. They are still read — by the viewer, `viewer -i`,
[`ssn_navigator`](cli-reference.md#ssn_navigator) and the desktop app, as the `.dvb` network
they correspond to — but never written. `.ssnv` is an older spelling of the same thing.

### `.nwk` — Newick trees

Written by [`build_tree`](cli-reference.md#build_tree) `-o/--newick_out`, and read with
`--newick`, which reads Newick the way Biopython does: quoted labels, `[comments]`, and
numeric internal labels taken as support values.

### `.xgmml` — Cytoscape

Written by [`build_ssn`](cli-reference.md#build_ssn). A static network: the edges are
fixed at the `--threshold` cutoff you chose, unlike the `.dvb` bundle. Node attributes carry any
metadata you joined on with `--metadata`, and `--cluster` adds a cluster id; `--mark_mst`
gives each edge a boolean `MST` attribute.

## HMM profiles

`.hmm` `.hmmer` (optionally `.gz`)

HMMER3 profile files, interchangeable with HMMER and pyhmmer. Domainator writes the
`HMMER3/f` format including the Pfam `GA`/`TC`/`NC` gathering cutoffs, so
`hmmsearch --cut_ga` still works on a file that has been through
[`hmmer_select`](cli-reference.md#hmmer_select) — which reproduces the profiles it keeps
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

Build profiles with [`hmmer_build`](cli-reference.md#hmmer_build) from an alignment in
any format hmmbuild reads: Stockholm, aligned FASTA, Clustal, PHYLIP (interleaved or
sequential), SELEX, A2M or PSI-BLAST. Formats are told apart as Easel tells them apart: by
the first line, then by the suffix where the content cannot decide. An aligned FASTA file
is A2M only when named `.a2m`, a PHYLIP file is sequential when named `.phys` (otherwise
the layout decides), and a SELEX-like file is PSI-BLAST only when named `.pb`. A2M and
PSI-BLAST mark consensus columns by case, which the profile keeps as its RF annotation.

A profile built here equals `domainator hmmer_build`'s apart from its DATE and COM lines,
and now and then the last printed digit of one COMPO or STATS value, where the builder's
arithmetic rounds a near-tie the other way.

## Formatted databases

For repeated searching of something large, [`format_db`](cli-reference.md#format_db)
prepares a directory:

- **shards** (`--shards N`) split the records across files so they can be searched in
  parallel;
- **BGZF compression** (`--compress`) — block-gzip, which is gzip-compatible
  to any reader but *seekable*, so a record can be fetched without decompressing the whole
  file;
- a **`.didx` index** (`--index`) mapping record names to virtual offsets.

Pass the directory as `-i` to [`domain_search`](cli-reference.md#domain_search).

These are hard on-disk contracts and the `.didx` files are byte-compatible with the Python
suite's, in both directions. Two consequences:

- **Plain gzip is rejected.** A `.gz` that is not BGZF cannot be seeked into, so the whole
  indexed-I/O layer refuses it rather than silently falling back to a full scan. Recompress
  it with `format_db --compress`.
- **An index is checked for staleness** by size and modification time, and a stale one is
  an error, not a warning.
- Having both an unsharded file and a shard directory present is an error — Domainator
  will not guess which you meant.

## Tables

`.tsv` `.csv` `.tab`

Used for report output ([`enum_report`](cli-reference.md#enum_report),
[`hmmer_report`](cli-reference.md#hmmer_report)), for network metadata
(`--metadata`), for color tables (`--color_table`), and for coordinate lists
([`select_by_coord`](cli-reference.md#select_by_coord)). Plain delimited text with a header
row; editable in the [Table viewer](gui.md#viewers) or any spreadsheet.

## JSON

Report tools emit machine-readable output with `--json_out`, which is independent of `-o`.
See [Config files and agent output](config-and-agents.md) for the shape of each.
