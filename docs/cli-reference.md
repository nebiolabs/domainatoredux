# Command-line reference

```
domainatoredux <tool> [options]
```

`domainatoredux --help` lists all 36 tools, plus `describe`, `completion` and `doctor`;
`domainatoredux <tool> --help` is the exhaustive,
always-current list of that tool's flags. This page gives each tool's purpose, the flags
that matter in practice, and a worked example. Filenames in the examples are
placeholders.

## Conventions that apply everywhere

These trip people up, so read them once:

**Subcommands and flags use underscores.** The tool is
`domainatoredux domain_search`; its flags are `--max_overlap`, `--dense_text`,
`--add_annotations`. Both mirror the Python suite's names exactly: upstream's executable
is `domainator`, this one is `domainatoredux`, so the two can be installed side by side.

**The Domainator 1.0 interface.** Upstream 1.0 renamed many options and made every
output its own `--<content>_out` option; every tool here follows it. In particular:

- outputs are `-o/--genbank_out`, `--fasta_out`, `--tsv_out`, ... and at least one is
  required. Nothing goes to stdout unless you ask for it with `-`, and at most one output
  can be `-`. A path ending in `.gz` or `.bgz` is written compressed (BGZF), and a file
  appears only once the run has succeeded.
- `-i -` reads GenBank from stdin, except on the tools that read their input more than
  once, which say so. Leaving out `-i` is an error.
- Progress messages and warnings go to stderr. `-q/--quiet` shows errors only, and
  `-v/--verbose` adds detail.
- `--config` takes upstream's format: an object keyed by option name, as `--print_config`
  writes it. See [Config files](config-and-agents.md).

`domainatoredux describe` lists every tool's options exactly.

**There are no global flags.** `--config`, `--print_config`, `--cpu`, `--json_out` and
`--max_output_gb` are declared per tool, so they must come *after* the tool name, and a
tool that doesn't declare one doesn't accept it.

```bash
domainatoredux domainate --cpu 8 -i in.gb -r db.hmm -o out.gb   # correct
domainatoredux --cpu 8 domainate -i in.gb -r db.hmm -o out.gb   # error
```

**A config file counts toward required arguments.** Its keys become command-line options
before parsing, so an input given in `--config` satisfies `-i`, and a missing one is the
same usage error either way. See [Config files](config-and-agents.md).

**`-i` is the file being edited, `-r` is what you're editing it by.** See
[Concepts](concepts.md#the--i---r-convention).

**`--cpu 0` means one thread per physical core** this process may use, and 0 is the
default. Only `domainate`, `domain_search`, `gene_call`, `deduplicate_genbank`,
`seq_dist`, `compare_contigs`, `hmmer_compare` and `hmmer_search` are threaded; output
is deterministic regardless of thread count.

**stdin and stdout.** `-i -` reads GenBank from stdin and `-o -` writes to stdout, so
tools pipe:

```bash
domainatoredux select_by_contig -i in.gb --domains Sod_Fe_N | domainatoredux enum_report -i - --columns domains -o -
```

A stdin input is spilled to a temporary file under your system temp directory, so a
read-only or tiny `TMPDIR` will cause failures on large piped inputs.

**`--max_output_gb`** (default 25, `0` disables) aborts a run whose projected matrix or
network output would exceed that size, before it writes. On `domainate`, `domain_search`, `seq_dist`,
`compare_contigs`, `transform_matrix`, `build_ssn`, `build_tree`, `hmmer_compare` and
`hmmer_search`.

<a id="taxonomy-filters"></a>**Taxonomy filters** (`--include_taxids`, `--exclude_taxids`,
`--taxonomy_expr`) keep what lies in (or outside) the given NCBI taxa: `--include_taxids
2` keeps bacteria, `--exclude_taxids 1224` drops proteobacteria (exclusion wins), and
`--taxonomy_expr '2 & ~1224'` combines taxids with `&`, `|`, `~` and parentheses. On
`domain_search`, `select_by_cds`, `select_by_contig`, `extract_domains`,
`extract_peptides` and `extract_unannotated`; each judges its own candidate (a record, a
CDS, an annotation or a hit), never the neighborhood extracted around it, and `--invert`
never inverts them. A candidate's taxid comes from a UniProt `OX=`/UniRef `TaxID=` header,
or else the longest `source` feature covering it. `--taxonomy_unknown` says what to do
when that taxid is missing, "unidentified", or not in the taxonomy: `exclude` it (the
default), `include` it, `evaluate` it as an empty lineage (so `~2` keeps it and `2` does
not), or stop with an `error`. A taxid given as a criterion that the taxonomy does not
know is an error.

They need an NCBI taxdump, given by `--ncbi_taxonomy_path`; it defaults to
`/tmp/ncbi_taxonomy_$USER` and is only loaded if you actually use a taxonomy filter or
column. It can be a directory holding `taxdump.tar.gz` or the extracted `.dmp` files, or
the path of a taxdump archive itself (such as the desktop app's NCBI Taxonomy download).
A directory with neither is created if need be and NCBI's `taxdump.tar.gz` (about 65 MB)
is downloaded into it. With `--taxonomy_update`, a local `taxdump.tar.gz` is checked
against the MD5 NCBI publishes and replaced if it differs; if NCBI cannot be reached, the
local copy is used with a warning. Every download is checked against that MD5 before it
replaces anything, and runs that share a database wait for each other's download rather
than both fetching it. enum_report's `--rank_lineage` loads it too; the Python tool does
not, and crashes on that column alone.

**`--partial include|exclude|only`** selects by fragment status: a CDS with a `<`/`>`
end, or a protein record marked as a fragment (UniProt ` (Fragment)`, `NON_TER`, ...).
`exclude` skips fragments and `only` keeps only them; `--invert` never inverts it. On
`domainate`, `domain_search`, `select_by_cds`, `select_by_contig`, `extract_peptides`,
`extract_domains`, `enum_report` and `summary_report`. What each one selects, and how
slicing marks the features it cuts: [File formats](file-formats.md#partial-features-and-fragments).

## The tools at a glance

| Role | Tools |
| --- | --- |
| [I/O and diagnostics](#io-and-diagnostics) | `inspect`, `convert`, `genbank_to_fasta`, `describe`, `completion`, `doctor` |
| [Annotation and search](#annotation-and-search) | `domainate`, `domain_search`, `gene_call`, `filter_domains`, `find_features` |
| [Selection and extraction](#selection-and-extraction) | `select_by_cds`, `select_by_contig`, `select_by_coord`, `extract_domains`, `extract_peptides`, `extract_unannotated`, `trim_contigs`, `rotate_contigs`, `sort_contigs`, `clean_sequences`, `deduplicate_genbank` |
| [Color and plots](#color-and-plots) | `color_genbank`, `color_table_to_legend`, `plot_contigs` |
| [Matrices and networks](#matrices-and-networks) | `seq_dist`, `compare_contigs`, `transform_matrix`, `build_ssn`, `build_tree`, `build_projection`, `viewer`, `split_by_cluster` |
| [Reports](#reports) | `enum_report`, `summary_report`, `matrix_report`, `ssn_navigator` |
| [HMM profiles](#hmm-profiles) | `hmmer_build`, `hmmer_select`, `hmmer_search`, `hmmer_compare`, `hmmer_report`, `infernal_build` |
| [Database preparation](#database-preparation) | `format_db` |

---

## I/O and diagnostics

### `inspect`

Parse a sequence file and report a per-record summary. The fastest way to check that a
file is readable and see what is in it. Takes a positional path, not `-i`.

```bash
domainatoredux inspect contigs.gb
domainatoredux inspect contigs.gb --verbose
```

### `convert`

Read GenBank or FASTA and write it back out as GenBank. Two positional arguments, no
flags. Useful for normalizing a file or checking round-trip fidelity.

```bash
domainatoredux convert input.fasta output.gb
```

### `genbank_to_fasta`

Write the sequences of GenBank records as FASTA, dropping annotations.

```bash
domainatoredux genbank_to_fasta -i annotated.gb -o sequences.fasta
```

### `describe`

Print the tool manifest: a JSON description of every tool and option (flags, type,
bounds, default, choices, and which options are files to read or write), in upstream's
`domainator describe` format. Meant for programs that drive the tools.

```bash
domainatoredux describe --indent 1 > manifest.json
domainatoredux describe --tool domainate sort_contigs
```

### `completion`

Print a shell completion script, for `bash`, `zsh`, `fish`, `powershell` or `elvish`.

```bash
domainatoredux completion bash > ~/.local/share/bash-completion/completions/domainatoredux
```

### `doctor`

List the optional external programs (`diamond`, `cdhit`, `usearch`, `orphos`, `cmbuild`): for each,
whether a tool would find it and where (by the lookup order in
[Installation](installation.md#optional-external-programs)), the commands that need it,
and how to install it.

```bash
domainatoredux doctor
```

---

## Annotation and search

### `domainate`

**Annotate sequences with hits to references, keeping every input record.** The workhorse
annotator: protein HMMs and protein FASTA are searched against CDSs (or against protein
records directly), nucleotide FASTA, nucleotide HMMs and Infernal covariance models
(`.cm`, searched as `cmsearch` does) against whole contigs on both strands, wrapping the
origin on circular contigs. Give `-r` several files to annotate
against several databases in one pass. Writes `-o/--genbank_out` and/or `--fasta_out`
(at least one; `-` is standard output). The input is read twice, so it cannot come from
standard input.

Key flags: `-e/--evalue` (default 0.001, at most 10) and `--min_evalue`; `-Z` (database
size for E-value calculation, default 1000; `0` counts the proteins and CDSs in the
input); `--max_overlap` (default 1, i.e. disabled) to drop annotations overlapping a
better hit, with `--overlap_by_db` to apply that per database; `--max_hits_per_cds`
(default: all) and `--max_mode`; `--alphabet` (default `protein`) to say how to read
FASTA input; `--cpu` to parallelize independent reference profiles, including profiles
in one HMM file; `--max_output_gb` (default 25, `0` disables) to stop before the outputs
grow past that size. Every input record is written, hit or not: to keep only the
records that hit, or to filter by taxonomy, use [`domain_search`](#domain_search). `--gene_call all|unannotated` calls genes first, exactly
as [`gene_call`](#gene_call) does in its default meta mode, so a bare genome can be
annotated in one step; give a nonzero `-Z` with it, since the database size is fixed
before genes are called.

```bash
domainatoredux domainate -i contigs.gb -r pfam-A.hmm rebase_gold.fasta \
  -o annotated.gb -e 1e-5 --cpu 0
```

### `domain_search`

**Keep the records (or the neighborhoods) that hit a reference.** Where `domainate`
annotates everything, `domain_search` selects. It reads possibly sharded, BGZF-compressed
and `.didx`-indexed databases (see [`format_db`](#format_db)), so it is the tool for
searching something large. Give `-i` several inputs to search them together; they cannot
come from standard input, and at least one of `-o/--genbank_out` and `--fasta_out` is
required.

It also does **neighborhood extraction**: rather than the hit CDS alone, emit the
surrounding region. `--whole_contig` for the entire contig (a circular contig stays circular, rotated so the
hit is in the middle); `--cds_range`/`--cds_up`/
`--cds_down` for a window measured in genes (protein-coding CDSs only: hits on a nucleotide
reference are never counted as neighbors); `--kb_range`/`--kb_up`/`--kb_down` for a
window in kilobases. `--strand f|r` restricts by strand, `--keep_direction` preserves
orientation, `--pad` pads with `N` so the focus CDSs line up at `--anchor start|end|center`
(padded records are linear), `--translate` emits protein instead of nucleotide, and
`--max_region_overlap` deduplicates overlapping windows. `--dedup` sets which outputs
count as copies: `none` (the default), `id` (a repeated record id, such as a region
extracted for several hits), `sequence` (an identical sequence, ignoring case and a
terminal `*`), or `sequence_rc` (also a nucleotide reverse complement). With
`--max_hits`, the best-scoring copy is the one kept.

Other key flags: `--add_annotations` to also write `Domainator` features (not just the
`Domain_Search` best hit); `--max_hits` and `--max_hits_per_contig` to cap results
(ranked by the focus CDS's own hit score); `--decoys`/`--decoys_file` to reject hits that
score better against a decoy; `-e/--evalue` (at most 10) and `-Z` (`0` counts the
input's proteins and CDSs); `--alphabet` for FASTA input; `--batch_size` (default 10000);
[taxonomy filters](#taxonomy-filters), applied to each searched protein or CDS and to
each nucleotide hit before ranking; `--max_output_gb` (default 25); `--gene_call
all|unannotated`, as on `domainate`.

```bash
# every gene within 10 kb of a Cas9 hit, across a formatted database
domainatoredux domain_search -i genomes_db/ -r cas9.hmm -o neighborhoods.gb \
  --kb_range 10 --add_annotations -e 1e-10
```

### `gene_call`

**Predict genes on nucleotide contigs, adding a `CDS` feature for each.** Domainator works
on CDSs, so a genome or contig with no gene annotation has to be gene-called first. This
is Prodigal's algorithm, run through [Orphos](https://github.com/FullHuman/orphos), an
external program you install once (see [Installation](installation.md#optional-external-programs)).
Each CDS gets a `/translation`, a `/gene_id` (`<contig>_<n>`) and a `/cds_id`. A gene
running off the end of a linear contig is kept as a partial CDS, extended to the edge and
marked `<`/`>`, with a `/codon_start` when its 5' end is cut. On a circular contig, genes
crossing the origin are called as joins across it.

Key flags: `--mode meta` (the default: pre-trained models chosen per contig, right for
metagenomes, plasmids, phages and short contigs) or `--mode single` (train on all input
records together, which suits one complete genome); `--scope unannotated` (the default:
only contigs with no CDS) or `--scope all` (delete every CDS, and the `Domainator`
annotations on them, then re-call); `--translation_table` (default 11; in meta mode each
contig uses its best model's table, 11 or 4); `--closed` to forbid genes running off the
ends; `--mask` to forbid genes across runs of `N`; `--alphabet` (default `nucleotide`)
for FASTA input; `--orphos_path` to name the `orphos` binary.

This tool has no counterpart in the Python suite, which calls genes only inside
`domainate`/`domain_search --gene_call`. Those flags drive the same code here, and their
output matches the Python tool's byte for byte.

```bash
domainatoredux gene_call -i assembly.fasta -o genes.gb
domainatoredux gene_call -i genome.fasta -o genes.gb --mode single
```

### `filter_domains`

Remove domain annotations without touching the sequences — the cleanup pass after
annotating generously.

Key flags: `-e/--evalue` and `--max_overlap` thresholds; `--domains`/`--domains_file`
to keep only named domains; `--keep_databases` (drop every other database's
annotations) and `--databases` (filter only these databases) to act per database;
`--annotation_type` to choose `domainator`, `domain_search` or `all`.

```bash
domainatoredux filter_domains -i annotated.gb -o clean.gb -e 1e-10 --max_overlap 0.5
```

### `find_features`

Annotate each protein, or each CDS's translation, with the matches of PROSITE patterns
(`--motif`; the syntax is EMBOSS `fuzzpro`'s: `x` any residue, `[DE]` either, `{P}` any
but, `(2,4)` a repeat, `<`/`>` the ends). Each match is a `Domainator` annotation with
program and database `MOTIF`, score and identity 100, e-value 0, and a name made from
its pattern that `--domain_expr` can use (`C-x(2)-C` is `C_x__2____C`). `--gene_call`
and `--partial` work as on `domainate`, and a run with no motif still adds a `cds_id`
and an on-contig translation to every CDS.

Upstream's `--tmbed` (transmembrane regions and signal peptides from a protein language
model) is not supported; it and its `--device`, `--model_dir`, `--tmbed_model_dir` and
`--batch_size` options are accepted, and `--tmbed` is refused.

```bash
domainatoredux find_features -i proteins.gb -o motifs.gb --motif "C-x(2)-C" "[DE](2)HS{P}"
```

---

## Selection and extraction

### `select_by_cds`

Extract contig regions around selected CDSs. Same neighborhood geometry as
`domain_search`, but selecting on what is *already annotated* rather than by running a
search.

Select CDSs by `--cds`/`--cds_file` (name; `--name_field` picks the qualifier matched
first), `--domains`/`--domains_file` (domain content), `--domain_expr` (a boolean
expression such as `'Sod_Fe_N & ~Sod_Fe_C'`), `--unannotated`, or `--has_search_hit`
(the CDSs a `domain_search` marked as hits). These selectors combine with OR; with none,
every CDS is selected. `--annotation_type domainator|domain_search|all` says whose names
the domain selectors read. `--contigs`, `--strand`, `--partial` and the
[taxonomy filters](#taxonomy-filters) then restrict the result, judged on the selected
CDS itself; `--invert` negates the selectors only, never these restrictions. Set the
window with `--whole_contig`, `--cds_range`/`--cds_up`/`--cds_down` or
`--kb_range`/`--kb_up`/`--kb_down`. `--include_nucleic_acids` keeps nucleotide records,
`--dedup` (default `id`, see [`domain_search`](#domain_search)) drops copies of a region
extracted for several CDSs (`--dedup none` keeps them), and `--pad` pads the regions with
`N` so the focus CDSs line up (at their start, end or center: `--anchor`).

```bash
domainatoredux select_by_cds -i annotated.gb -o operons.gb \
  --domain_expr 'Cas9 or Cas12a' --cds_up 3 --cds_down 3
```

### `select_by_contig`

Select whole contigs, no extraction. Filter by name (`--contigs`, `--contigs_file`,
`--contigs_regex`), description (`--definition_regex`), sequence (`--sequence_regex`),
domain content (`--domains`, `--domain_expr`, `--annotation_type domainator|domain_search|all`),
search hits (`--has_search_hit`, or a `--min_search_score`/`--max_search_score` range);
any of these selects a contig. Size (`--min_length`/`--max_length`, `--min_cds`/`--max_cds`),
`--partial` and the [taxonomy filters](#taxonomy-filters) then restrict the result, and
`--invert` negates the selectors only. `--max_records` takes the first N;
`-o/--genbank_out` and `--fasta_out` choose the outputs.

```bash
domainatoredux select_by_contig -i all.gb -o big_hits.gb \
  --domains Cas9 --min_length 5000 --min_cds 3
```

### `select_by_coord`

Extract regions at explicit 1-based coordinates listed in a TSV/CSV table.

```bash
domainatoredux select_by_coord -i contigs.gb --coords regions.tsv -o regions.gb
```

### `extract_domains`

Extract just the domain-annotated regions as their own records, of the input's sequence
type (nucleotide input gives nucleotide regions; for protein domains from contigs, run
[`extract_peptides`](#extract_peptides) first). `--extend_range`, or
`--extend_up`/`--extend_down`, extend the boundaries in residues or bases (negative
values trim), `--combine` takes one span enclosing a record's selected annotations,
`--splice` joins them, and `--keep_direction` and `--keep_parent_id` control orientation
and naming. Filter by `-e/--evalue`, `--min_score`, `--domains`, `--databases`,
`--partial` and the [taxonomy filters](#taxonomy-filters); `--annotation_type
domainator|domain_search|all` says which annotations to extract, and `--has_search_hit`
adds the `Domain_Search` best hits whatever their name.

```bash
domainatoredux extract_domains -i annotated.gb --fasta_out domains.fasta \
  --domains Sod_Fe_N --extend_range 10
```

### `extract_peptides`

Extract the peptide sequence of selected CDSs. The selectors are the same as
[`select_by_cds`](#select_by_cds)'s — `--cds`/`--cds_file`, `--domains`,
`--domain_expr`, `--unannotated`, `--has_search_hit` — as are the restrictions
(`--contigs`, `--strand`, `--partial`, taxonomy) and `--invert`. `--name_field` chooses
the qualifier used for names; `--keep_parent_id` names each peptide after its contig.

```bash
domainatoredux extract_peptides -i annotated.gb --fasta_out proteins.fasta --domains Cas9
```

### `extract_unannotated`

Extract the parts of proteins that *aren't* covered by a domain annotation — for finding
novel regions. `--largest` keeps only the longest gap per protein,
`--min_region_length` sets a minimum gap length, and `-e/--evalue`,
`--min_annotation_score`, `--databases` and `--annotation_type` decide which annotations
count. The [taxonomy filters](#taxonomy-filters) choose the proteins.

```bash
domainatoredux extract_unannotated -i annotated.gb --fasta_out novel.fasta --min_region_length 50 --largest
```

### `trim_contigs`

Trim contig ends, either by size (`--kb_range`, `--kb_left`, `--kb_right`), by gene count
(`--cds_range`, `--cds_left`, `--cds_right`), or by stripping end CDSs that carry a
domain (`--domains`, `--domain_expr`) or carry none (`--unannotated`); `-e/--evalue` and
`--databases` decide which annotations count. A contig with fewer CDSs than a CDS trim
removes is dropped.

```bash
domainatoredux trim_contigs -i neighborhoods.gb -o trimmed.gb --cds_range 1
```

### `rotate_contigs`

Set the origin of circular contigs relative to a domain. The domain is chosen from the
new hits of a reference search (`-r`), the pre-existing annotations named by
`--domains`/`--domains_file` (of `--annotation_type`, `Domainator` by default), every
pre-existing annotation of `--annotation_type domain_search|all`, or the `Domain_Search`
hits (`--has_search_hit`), all under `-e/--evalue` (named ones also under `--databases`):
the highest score wins, then the lowest E-value, then the lowest position. `--anchor`
puts the origin at the domain's `start`, `end` or `center`, in the domain's own
orientation, and the contig is reverse complemented to put the domain on the forward
strand unless `--keep_direction`. Features crossing the new origin become joins. A
rotated contig is named `{id}_{first}:{last}` (with `rc` when reversed) unless
`--keep_name`; new search hits are dropped unless `--keep_annotation` keeps the chosen
one. Protein records, linear records (unless `--circularize`), and contigs with no
qualifying domain are skipped with a warning. Protein references need CDS annotations,
so FASTA contigs need `--gene_call`.

```bash
domainatoredux rotate_contigs -i plasmids.gb -o rotated.gb -r repA.hmm --anchor start
```

### `sort_contigs`

Sort contigs by their best `Domain_Search` score, or by their best `Domainator`
annotation among `--domains` (and `--domains_file`, a text, HMM, FASTA or GenBank file
whose names or ids are used), optionally only from `--databases`. A contig with no
matching annotation is an error. `--max_hits` keeps the best N. Writes `-o/--genbank_out`,
`--fasta_out`, or both.

```bash
domainatoredux sort_contigs -i hits.gb -o sorted.gb --max_hits 100
```

### `clean_sequences`

Housekeeping before phylogenetics or alignment: `--clean_name` makes names newick-safe,
`--deduplicate_names` disambiguates collisions, `--strip` removes unwanted residues and
`--filter_by_aa` drops records containing them. `--alphabet nucleotide` reads FASTA input
as DNA (default `protein`); the counts go to standard error, or to `--log_out`.

```bash
domainatoredux clean_sequences -i proteins.fasta --fasta_out clean.fasta --clean_name --deduplicate_names
```

### `deduplicate_genbank`

Collapse redundant sequences, keeping one representative per cluster.

`--algorithm` selects the backend: **`cd-hit`** (the default), `diamond`, `usearch` — each
needing that external binary — or **`hash`**, which is pure Rust, needs nothing, and does
exact deduplication only (so it requires `--identity 1`). `--identity` is required and is
the identity threshold in `[0,1]` (`--id` is the same option). `--cdhit_path`, `--cdhit_est_path`, `--usearch_path` and
`--diamond_path` point at the binaries; `--cluster_tsv_out` writes cluster membership;
`--prefix_count`/`--suffix_count` annotate representative names with the cluster size;
`--both_strands` counts a nucleotide sequence's reverse complement as a copy. `--cdhit_args`, `--usearch_args` and `--diamond_args`
pass extra options to the program, as one quoted string (`--cdhit_args="-s 0.9"`); an
option Domainator sets itself is refused.

```bash
domainatoredux deduplicate_genbank -i proteins.fasta --fasta_out representatives.fasta \
  --algorithm hash --identity 1
domainatoredux deduplicate_genbank -i proteins.fasta --fasta_out representatives.fasta \
  --identity 0.9 --cluster_tsv_out clusters.tsv
```

---

## Color and plots

### `color_genbank`

Assign hex colors to features via the `Color` qualifier, which viewers (including
Geneious and the [desktop app](gui.md)) honor. `--target domains|cdss|domains_and_cdss`
chooses what gets colored; colors come from a `--color_table` or are assigned
automatically and can be saved with `--color_table_out` so runs stay consistent.
`--search_hit_color` highlights `Domain_Search` features; `--clear` strips existing
colors.

```bash
domainatoredux color_genbank -i annotated.gb -o colored.gb \
  --target domains_and_cdss --color_table_out colors.tsv
```

### `color_table_to_legend`

Render a color table as an SVG legend to accompany a figure.

```bash
domainatoredux color_table_to_legend -i colors.tsv -o legend.svg --title "Domains"
```

### `plot_contigs`

Draw contigs and their annotations as an HTML page that works offline: each contig is a
line with its features drawn above it as arrows, stacked so overlapping features stay
visible. Zoom with the mouse wheel, drag to pan, and click a contig or feature to list
its details. `--types` picks the feature types drawn (CDS, exon, repeat, RNA and domain
features by default), and a feature's `Color` qualifier sets its fill. `--color_by`
colors the contig lines by a column of the `--metadata` tables, using the automatic
palette or a `--color_table`. The page is upstream's own, so `domainator plot_contigs`
writes the same plot. It reads best for a few hundred contigs of up to tens of kilobases.

```bash
domainatoredux plot_contigs -i colored.gb -o contigs.html \
  --metadata contigs.tsv --color_by species
```

---

## Matrices and networks

### `seq_dist`

**Pairwise similarity by local alignment.** Omit `-r` for all-vs-all. Write the matrix with
`-o/--hdf_out` (sparse HDF5; the name must end `.h5`, `.hdf5` or `.hdf`) and/or
`--dense_text_out` (TSV, `-` for standard output) — at least one is required. Formats
come from the file names unless `--input_format`/`--reference_format` say otherwise.

`--algorithm` picks the engine: `diamond_f`, `diamond_d`, `diamond_mids`, `diamond_s`,
`diamond_mors`, `diamond_vs`, `diamond_us` (the default, ultra-sensitive) all need the
external `diamond` binary; **`hmmer`** (phmmer/hmmsearch) and `hmmer_compare` (HMM-vs-HMM,
selected automatically for two HMM inputs) are built in. `--mode` sets what the cells
contain (`score` by default); `--threshold` drops raw scores at or below it;
`--max_hits_per_query` keeps each input's best hits (diamond's `--max-target-seqs`, or the
hmmer hit list); `--mst` and `--knn K` prune the graph to the maximum spanning forest, the
OR-symmetric K nearest neighbors, or both (all-vs-all only, `--mode score`, `bool` or
`efi_score`); `--symmetrize max|min|mean` reconciles the two directions of each pair before
`--mode`; `--diamond_args="-b 0.4 -c 4"` forwards DIAMOND options and `--diamond_path`
names the binary; `--max_output_gb` (default 25) caps each output.

```bash
domainatoredux seq_dist -i proteins.fasta -o dist.hdf5 --mode score
domainatoredux seq_dist -i proteins.fasta -o dist.hdf5 --algorithm hmmer --cpu 8
```

### `compare_contigs`

**Pairwise similarity by shared domain content** — the right comparison for gene
neighborhoods rather than single proteins. Scores by Jaccard index (`--ji`) and adjacency
index (`--ai`), weighted; at least one must be non-zero and neither negative. Restrict to
particular `--databases`, or to some contigs with `--contigs`/`--contigs_file`; `-i -`
reads standard input. The same `-o/--hdf_out`/`--dense_text_out` outputs and
`--threshold`/`--max_hits_per_query`/`--mst`/`--knn` pruning as `seq_dist`.

`--genbank_out` writes the contigs themselves, ordered so that similar neighborhoods sit
together: hierarchical clustering (`--linkage`: `average` by default, or `ward`,
`complete`, `single`, `weighted`) on the distance `1 - score / max(score)`, a pair with no
score at distance 1, then `--leaf_ordering`: `greedy` (the default) flips subtrees so the
most similar contigs meet at each junction, `optimal` is scipy's optimal leaf ordering
(very slow beyond a few thousand contigs), `none` keeps the plain dendrogram order. The
order is the one upstream's scipy-based sort gives, tie for tie. `--name_by_order`
prefixes each id with its position. The sort holds every pairwise distance in memory, so
a run that would need more than is available fails before scoring
(`--skip_sort_memory_check` tries anyway).

```bash
domainatoredux compare_contigs -i neighborhoods.gb -o similarity.hdf5 --ji 0.5 --ai 0.5
domainatoredux compare_contigs -i neighborhoods.gb --genbank_out sorted.gb --name_by_order
```

### `transform_matrix`

Normalize, convert or sparsify an existing matrix. `--mode` is one of `none` (the
default: keep the values), `score`, `bool`, `norm_score`, `row_norm_score`, `score_dist`,
`efi_score`, `efi_score_dist`, and converts only from a `score` matrix; `--value_type`
says what the input holds when the file does not. `--symmetrize max|min|mean` reconciles
the two directions of each pair first; `--mst` and `--knn K` prune (not a distance
matrix); `--threshold` zeroes values at or below it after the transform. Writes
`-o/--hdf_out` and/or `--dense_text_out`, so reading a TSV and writing HDF5 converts.

```bash
domainatoredux transform_matrix -i dist.hdf5 -o normalized.hdf5 --mode efi_score
domainatoredux transform_matrix -i scores.tsv -o pruned.hdf5 --mst --knn 5 --threshold 50
```

### `build_ssn`

Build a network from a similarity matrix (not a `_dist` one), as a **Cytoscape XGMML**
file (`-o/--xgmml_out`), the **Domainator Viewer** (`--html_out`, a self-contained page),
or its bundle (`--dvb_out`, which the desktop app's Viewer tab and
[`ssn_navigator`](#ssn_navigator) open). Give at least one of them. `--threshold` is the score cutoff, keeping edges strictly above it — the
single most important choice, so consult [`matrix_report`](#matrix_report) first. `--mst`
keeps only maximum-spanning-tree edges and `--knn K` each node's K best; together they
keep the union, which keeps a large network legible without splitting clusters.
`--mark_mst` gives every edge a boolean `MST` attribute instead of pruning. `--cluster`
assigns connected-component cluster ids and `--cluster_tsv_out` writes them out (with a
`contig cluster` header unless `--no_cluster_header`). `--metadata` joins TSV columns
onto nodes, and `--color_by`/`--color_table`/`--color_table_out` color by one of them;
`--max_color_groups N` colors only the N largest groups, and `--palette_offset N` starts N
colors into the palette, so a second column can be colored without reusing the colors a
first one took.

```bash
domainatoredux build_ssn -i dist.hdf5 -o network.xgmml --threshold 100 \
  --cluster_tsv_out clusters.tsv --metadata annotations.tsv
```

The viewer needs no cutoff, because it lets you move the threshold instead: it shows the
network's maximum spanning tree above `--threshold`, whatever `--mst` and `--knn` say.
`--label_by` chooses the column that labels nodes, `--color_table` sets the starting colors
of the `--color_by` column, `--categorical` marks numeric columns to color as categories
rather than a gradient, `--merge_impact_metric` how the split chart measures a merge, and
`--name` the network's title (by default the output's file name).

```bash
domainatoredux build_ssn -i dist.hdf5 --dvb_out network.dvb --html_out network.html \
  --metadata annotations.tsv --color_by phylum --categorical phylum
```

There is deliberately no `--max_merge_events` here: the bundle stores only nodes, the
spanning tree and the hierarchy, and everything else is derived when read, so the viewer
(and [`ssn_navigator`](#ssn_navigator)) chooses that cap at read time.

### `build_tree`

Build a **UPGMA tree** from a square distance or similarity matrix (`-i`), or read one
built elsewhere from a Newick file (`--newick`; numeric internal labels are read as
support values). `--distance auto` uses a `_dist` matrix (or a TSV, which records no data
type) as is and turns similarity scores into distances, 1 − score / min(row max, column
max); `precomputed` and `score_dist` force either. Writes Newick (`-o/--newick_out`, `-`
for stdout, branch lengths to six significant digits), Cytoscape XGMML laid out as a
rectangular tree (`--xgmml_out`), and the Domainator Viewer (`--html_out`, `--dvb_out`).
`--msa` shows an aligned FASTA of the leaves beside the tree in the viewer. `--subset`/
`--subset_file` keep only some leaves, and the coloring and labeling options are
`build_ssn`'s.

```bash
domainatoredux build_tree -i dist.hdf5 -o tree.nwk --dvb_out tree.dvb --msa aligned.afa
domainatoredux build_tree --newick ml_tree.nwk --metadata annotations.tsv --color_by phylum \
  --html_out tree.html
```

### `build_projection`

Project a matrix's rows into **2D or 3D** (`--three_d`) with PCA, each row a point whose
features are its columns. A mostly-empty matrix is projected uncentered (scikit-learn's
`TruncatedSVD`, which upstream uses there), and the coordinates are shifted to start at 0
and scaled so the largest spans `--scale`. `--algorithm coords` reads coordinates already
computed (a `--coords_out` table) instead. Writes the coordinates (`--coords_out`),
Cytoscape XGMML (`-o/--xgmml_out`) and the Domainator Viewer (`--html_out`, `--dvb_out`).
Upstream's `tsne` and `umap` are not supported yet: project with the Python suite and read
its `--coords_out` with `--algorithm coords`.

```bash
domainatoredux build_projection -i dist.hdf5 --coords_out xy.tsv --dvb_out projection.dvb \
  --metadata annotations.tsv --color_by phylum
```

### `viewer`

Write the **Domainator Viewer** page (`-o/--html_out`) opening a bundle (`-i`, a `.dvb`
from any of the three tools above or a saved session, or an older `.dsnv`), or, without
`-i`, an empty page that opens the bundles you choose in it. `--name` sets the title.

```bash
domainatoredux viewer -i network.dvb -o network.html
```

### `split_by_cluster`

Split a file into one file per cluster of a similarity matrix (`--matrix`, HDF5 or dense
text), so each family can be aligned, built into a profile or searched on its own. The
input (`-i`) is GenBank, FASTA, an `.hmm` profile library or an Infernal `.cm` file, and
its record or profile names must be exactly the matrix's labels. Clusters are the
connected components of the scores above `--threshold`, numbered from 1 (the largest) as
`build_ssn` numbers them, and each is written as `<number><ext>` into `--output_dir` in
the input's format and compression (`.hmm.gz` profiles stay plain gzip; GenBank and FASTA
are BGZF). Clusters smaller than `--min_cluster_size` are skipped, or pooled into
`small_clusters<ext>` with `--write_small_clusters`. With `--pad_on_search_hits`, each
GenBank contig is centered on its first `domain_search` hit and padded with `N` so the
hits line up within each file. An existing output file is never overwritten.

```bash
domainatoredux split_by_cluster -i hits.gb --matrix dist.hdf5 --threshold 100 \
  --output_dir families --min_cluster_size 5 --write_small_clusters
```

---

## Reports

### `enum_report`

**One row per contig, CDS or domain**, chosen with `--by contig|cds|domain`.
`--columns` takes the columns in output order. A column with arguments takes them after
`:`, as in `qualifier:CDS:product`; `--columns` can be given more than once, and the
order across them is kept. `--help` lists every column.

Columns without arguments: `domains`, `domain_descriptions`, `score`, `identity`,
`architecture`, `architecture_detailed`, `start`, `end`, `strand`, `length`, `topology`,
`sequence`, `cds_count`, `domain_count`, `definition`, `filename`, `domain_search`,
`taxid_lineage`, `taxname_lineage`, `rank_lineage`, `isoelectric_point`, `starts_with`,
`molecular_weight`, `partial_count`, `fragment` (`complete`, `N`, `C`, `NC` or `?`).

Columns with arguments: `taxid:<rank>[:<rank>...]`, `taxname:<rank>[:<rank>...]`
(`self` is the record's own taxon), `qualifier:<feature>:<qualifier>`,
`feature_count:<feature>`, `motif_count:<prosite-pattern>`, `net_charge:<pH>`,
`repeat_count:<length>`, `named_domain_count:<domain>`, and
`append:<name>:<str|int|float>:<value>`, a constant column. The last argument keeps any
`:` it contains.

Outputs are `-o/--tsv_out`, `--html_out` and `--json_out` (streaming NDJSON, one object
per row); give at least one. `--html_out` writes a table page that opens in any browser,
offline: sort, filter each column (`> 10` on a number column), page, move and resize
columns, right-click a cell to copy its value, and download the filtered rows as
CSV/TSV/JSON. It shows floats to one decimal place, as the TSV does; the page holds the
same rows as `--json_out`, so copies and downloads keep every digit.
`--html_max_height` caps the table's height in pixels. `--partial` filters rows by
fragment status. The taxonomy columns read
`--ncbi_taxonomy_path`, downloaded if missing and refreshed by `--taxonomy_update` (see
[Taxonomy filters](#taxonomy-filters)).

```bash
domainatoredux enum_report -i annotated.gb --by cds \
  --columns domains length molecular_weight isoelectric_point taxname:genus:species \
  -o table.tsv
```

### `summary_report`

Dataset-level statistics: contig and CDS counts, length distribution, domain frequency,
taxonomy. `--contigs`/`--contigs_file` choose which contigs are counted and `--databases`
which domain annotations; `--domains`/`--domains_file` name focus domains whose
co-occurrence with every other domain is reported; `--taxonomy` adds a taxonomic
breakdown. The counts include partial CDSs and protein fragments (`partial_cdss`,
`fragment_proteins`), and `--partial` restricts what is counted.

Outputs (at least one is required): `-o/--text_out` the text report, `--html_out` a page
that opens in any browser, offline (a length histogram that zooms, pans and rebins, sortable domain and
co-occurrence tables, and with `--taxonomy` a zoomable taxonomy chart), `--json_out` one
compact JSON object (`schema_version` 1), `--domains_tsv_out` the domain frequency table,
and `--co_occurrence_count_out`/`--co_occurrence_fraction_out` domain-by-domain matrices
(dense TSV) of how many contigs carry both domains, or what fraction of the row domain's
contigs also carry the column domain.

```bash
domainatoredux summary_report -i annotated.gb -o summary.txt
domainatoredux summary_report -i annotated.gb --json_out - | jq .domain_frequency
domainatoredux summary_report -i annotated.gb --html_out summary.html --taxonomy
```

### `matrix_report`

Summarize a similarity matrix: node and edge counts, score statistics, a histogram, and
**suggested clustering thresholds** — the score values where the network structure
changes. Run this before choosing `build_ssn --threshold`.

`--max_merge_events` (default 500, `0` = all) caps the split-event series in the JSON
report; the text report always lists the 25 strongest events.
`--merge_impact_metric min_child|product` changes how an event's significance is scored.
`--json_out` writes the summary as one compact JSON object (`schema_version` 1).
`--html_out` writes an interactive page that works offline: clusters and edges kept by
threshold, the split events, and a threshold slider showing the clusters at each cut.
`--knn_projection` adds, to the text report and the page, how many edges
`build_ssn --threshold T --mst --knn k` would keep for each threshold and `k` from 1 to
25. At least one of `-o/--text_out`, `--html_out` and `--json_out` is required, and any
may be `-`. The input must be a similarity matrix, not a `_dist` one.

```bash
domainatoredux matrix_report -i dist.hdf5 -o report.txt
domainatoredux matrix_report -i dist.hdf5 --json_out report.json
domainatoredux matrix_report -i dist.hdf5 --html_out report.html --knn_projection
```

### `ssn_navigator`

Query the network in a built `.dvb` bundle (from `build_ssn --dvb_out`, or an older
`.dsnv`) from a script — the non-interactive counterpart to the viewer.
Output is always one compact JSON object, starting with `schema_version` and `query`, to
`-o/--json_out` (required; `-` is stdout).

`--query` selects the question: `overview` (default) for the shape of the network,
`thresholds` for the cluster-count-versus-cutoff table, `clusters` for the clusters at a
given `--threshold`, `cluster` for one cluster's contents (`--cluster`, `--members`), and
`node` for one node's context (`--node`). `--min_size` and `--top_n` filter and cap
cluster listings.

`--node` must match a node's label exactly, as it appears in the bundle — usually the full
sequence id, not a prefix.

```bash
domainatoredux ssn_navigator -i network.dvb -o - --query overview
domainatoredux ssn_navigator -i network.dvb -o - --query clusters --threshold 120 --min_size 3
domainatoredux ssn_navigator -i network.dvb -o - --query node \
  --node "FeSOD_A0A1F4ZT98|unreviewed|Superoxide"
```

---

## HMM profiles

Domainator treats HMM profiles as first-class data: you can build them, subset them,
compare them to each other, and build similarity networks *of profiles*.

Every tool here, and `domainate`/`seq_dist`, reads gzip- or BGZF-compressed `.hmm` files
as-is. The three that write profiles (`hmmer_build`, `hmmer_select`, `hmmer_search`) write
plain gzip when `-o` ends in `.gz` and refuse `.bgz`, which HMMER cannot read — see
[HMM profiles](file-formats.md#hmm-profiles).

### `hmmer_build`

Build a profile from a multiple sequence alignment, **Stockholm** or **aligned FASTA**
(told apart by the first line; `-i -` reads standard input). `--name` is required.
`--acc` and `--desc` set the accession and description, `--alphabet` overrides alphabet
detection, and `--window_length` (or `--window_beta`, a tail mass to derive it from)
applies to nucleotide models. The profile goes to
`-o/--hmm_out` (`-` for stdout, `.gz` for gzip).

```bash
domainatoredux hmmer_build -i alignment.sto -o profile.hmm --name MyDomain --desc "My domain"
```

Upstream reads every format hmmbuild does; this port reads those two.

### `infernal_build`

Build an Infernal covariance model (CM) from a multiple sequence alignment, by running
Infernal's `cmbuild` (see [Installation](installation.md#optional-external-programs)), and
set its NAME (`--name`, required), ACC (`--acc`) and DESC (`--desc`). `--noss` builds a
model with no base pairs, which an alignment without secondary-structure annotation (such
as aligned FASTA) needs; `--hand` and `--symfrac` are passed to `cmbuild`, as is
`--input_format` (`stockholm`, `pfam` or `selex`; aligned FASTA is recognized without it).
The model goes to `-o/--cm_out` (`-` for stdout); `-i -` reads standard input. Exactly one
alignment per run.

```bash
domainatoredux infernal_build -i family.sto -o family.cm --name MyRNA --acc RF99999
```

A freshly built model has no E-value calibration. Run Infernal's `cmcalibrate` on it before
using it as a `domainate` reference.

### `hmmer_select`

Pull a subset of profiles out of one or more `.hmm` files by text matching on their NAME,
DESC or ACC fields. `--regex`, `--exact` and `--contains` are the match modes, `--field`
restricts which field is matched, `--case_sensitive` and `--alphabet` refine further.
The matches go to `-o/--hmm_out` (`-` for stdout); `-i -` reads standard input.

```bash
domainatoredux hmmer_select -i Pfam-A.hmm.gz -o nucleases.hmm --contains nuclease --field desc
```

### `hmmer_search`

Keep the query profiles that score well against a set of reference profiles — a profile
"search" rather than a sequence search. `--min_score` (default 10 for amino acid
references, the equivalent, about 4.63, for nucleotide ones) and `--max_hits` control what
is kept. The kept profiles go to `-o/--hmm_out` (`-` for stdout, `.gz` for gzip).

```bash
domainatoredux hmmer_search -i candidates.hmm -r known.hmm -o related.hmm --min_score 20
```

### `hmmer_compare`

Align profiles against profiles (hhsearch-like, by profile-vs-profile Viterbi).
`-o/--tsv_out` writes a `query reference score` table and `--alignments_out` each hit's
row followed by its alignment; at least one is required. `--min_score` (default 0) and
`--max_hits_per_query` limit the hits.

```bash
domainatoredux hmmer_compare -i query.hmm -r pfam-A.hmm -o scores.tsv \
  --alignments_out alignments.txt
```

To build a network *of* profiles, feed the same files to
[`seq_dist`](#seq_dist) — with two HMM inputs it selects `hmmer_compare` automatically —
and then to [`build_ssn`](#build_ssn).

### `hmmer_report`

Tabulate profile metadata. Like `enum_report`, `--columns` takes the columns in output
order: `source`, `acc`, `desc`, `length`, `consensus`, `alphabet`, and
`append:<name>:<str|int|float>:<value>`. Outputs are `-o/--tsv_out`, `--html_out` (the
same interactive table page as `enum_report`'s, with `--html_max_height`) and `--json_out`
(streaming NDJSON).

```bash
domainatoredux hmmer_report -i pfam-A.hmm --columns acc desc length -o profiles.tsv \
  --html_out profiles.html
```

---

## Database preparation

### `format_db`

Prepare a sequence collection for fast repeated searching by
[`domain_search`](#domain_search): split it into shards (`--shards`), BGZF-compress it
(`--compress`) so it stays seekable, and build a `.didx` index (`--index`).
`--output_dir` chooses where the result goes, `--name` names a single-input database,
`--level` sets the compression level (1–12, default 6), `--cpu` the worker threads,
`--force` overwrites, and `--log_out` writes the log to a file instead of stderr.

```bash
domainatoredux format_db -i uniprot.fasta --output_dir uniprot_db \
  --shards 8 --compress --index
domainatoredux domain_search -i uniprot_db/ -r query.hmm -o hits.gb
```

Formatted databases are byte-compatible with the Python suite's, in both directions.
Plain (non-BGZF) gzip is rejected, because the whole seekable-I/O layer depends on block
compression. See [File formats](file-formats.md#formatted-databases).
