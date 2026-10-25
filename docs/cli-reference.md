# Command-line reference

```
domainatoredux <tool> [options]
```

`domainatoredux --help` lists all 34 tools, plus `describe` and `completion`;
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

**The Domainator 1.0 interface, tool by tool.** Upstream 1.0 renamed many options and
made every output its own `--<content>_out` option. This port is moving each tool to that
interface, and these tools have moved so far: `sort_contigs`, `enum_report`,
`hmmer_report`. On a migrated tool:

- outputs are `-o/--genbank_out`, `--fasta_out`, `--tsv_out`, ... and at least one is
  required. Nothing goes to stdout unless you ask for it with `-`, and at most one output
  can be `-`. A path ending in `.gz` or `.bgz` is written compressed (BGZF), and a file
  appears only once the run has succeeded.
- `-i -` reads GenBank from stdin. Leaving out `-i` is an error.
- `-v/--verbose` and `-q/--quiet` are accepted.
- `--config` takes upstream's format: an object keyed by option name, as `--print_config`
  writes it. See [Config files](config-and-agents.md).

`domainatoredux describe` lists every tool's options exactly, including which ones a tool
has not migrated yet.

**There are no global flags.** `--config`, `--print_config`, `--cpu`, `--json` and
`--max_output_gb` are declared per tool, so they must come *after* the tool name, and a
tool that doesn't declare one doesn't accept it.

```bash
domainatoredux domainate --cpu 8 -i in.gb -r db.hmm -o out.gb   # correct
domainatoredux --cpu 8 domainate -i in.gb -r db.hmm -o out.gb   # error
```

**Required arguments are checked after the config file is merged.** A missing input is
reported as `missing required 'input' (-i / --input or config)` rather than a usage error,
because it might legitimately have come from `--config`. See
[Config files](config-and-agents.md).

**`-i` is the file being edited, `-r` is what you're editing it by.** See
[Concepts](concepts.md#the--i---r-convention).

**`--cpu 0` means all cores**, and 0 is the default. Only `domainate`, `domain_search`,
`gene_call`, `deduplicate_genbank`, `seq_dist`, `compare_contigs`, `hmmer_compare` and
`hmmer_search` are threaded; output is deterministic regardless of thread count.

**stdin and stdout.** Tools that accept multiple inputs read GenBank from stdin when `-i`
is omitted, and most tools write to stdout when `-o` is omitted, so they pipe:

```bash
domainatoredux select_by_contig -i in.gb --domains Sod_Fe_N | domainatoredux enum_report -i - --columns domains -o -
```

A stdin input is spilled to a temporary file under your system temp directory, so a
read-only or tiny `TMPDIR` will cause failures on large piped inputs.

**`--max_output_gb`** (default 25, `0` disables) aborts a run whose projected matrix or
network output would exceed that size, before it writes. On `domain_search`, `seq_dist`,
`compare_contigs`, `transform_matrix`, `build_ssn`, `build_ssn_viewer`, `hmmer_compare`
and `hmmer_search`.

**Taxonomy flags** (`--include_taxids`, `--exclude_taxids`, `--taxonomy_expr`) need an
NCBI taxdump, given by `--ncbi_taxonomy_path`; it defaults to
`/tmp/ncbi_taxonomy_$USER` and is only loaded if you actually use a taxonomy filter or
column. It can be a directory of extracted `.dmp` files, a directory holding
`taxdump.tar.gz`, or the path of a taxdump archive itself (such as the desktop app's NCBI
Taxonomy download). enum_report's `--rank_lineage` loads it too; the Python tool does not,
and crashes on that column alone.

**`--partial include|exclude|only`** selects by fragment status: a CDS with a `<`/`>`
end, or a protein record marked as a fragment (UniProt ` (Fragment)`, `NON_TER`, ...).
`exclude` skips fragments and `only` keeps only them; `--invert` never inverts it. On
`domainate`, `domain_search`, `select_by_cds`, `select_by_contig`, `extract_peptides`,
`extract_domains`, `enum_report` and `summary_report`. What each one selects, and how
slicing marks the features it cuts: [File formats](file-formats.md#partial-features-and-fragments).

## The tools at a glance

| Role | Tools |
| --- | --- |
| [I/O and diagnostics](#io-and-diagnostics) | `inspect`, `convert`, `genbank_to_fasta`, `describe`, `completion` |
| [Annotation and search](#annotation-and-search) | `domainate`, `domain_search`, `gene_call`, `filter_domains` |
| [Selection and extraction](#selection-and-extraction) | `select_by_cds`, `select_by_contig`, `select_by_coord`, `extract_domains`, `extract_peptides`, `extract_unannotated`, `trim_contigs`, `sort_contigs`, `clean_sequences`, `deduplicate_genbank` |
| [Color](#color) | `color_genbank`, `color_table_to_legend` |
| [Matrices and networks](#matrices-and-networks) | `seq_dist`, `compare_contigs`, `transform_matrix`, `build_ssn`, `build_ssn_viewer` |
| [Reports](#reports) | `enum_report`, `summary_report`, `matrix_report`, `ssn_navigator` |
| [HMM profiles](#hmm-profiles) | `hmmer_build`, `hmmer_select`, `hmmer_search`, `hmmer_compare`, `hmmer_report` |
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

---

## Annotation and search

### `domainate`

**Annotate sequences with hits to references, keeping every input record.** The workhorse
annotator: protein HMMs and protein FASTA are searched against CDSs (or against protein
records directly), nucleotide FASTA and nucleotide HMMs against whole contigs on both
strands, wrapping the origin on circular contigs. Pass `-r` more than once to annotate
against several databases in one pass.

Key flags: `-e/--evalue` (default 0.001) and `--min_evalue`; `-Z/--z` (database size for
E-value calculation, default 1000); `--max_overlap` (default 1.0, i.e. disabled) to drop
annotations overlapping a better hit, with `--overlap_by_db` to apply that per database;
`--max_domains` and `--max_mode` to cap annotations per CDS; `--hits_only` to keep only
records that hit something; `--no_annotations` to score without writing features;
`--fasta_type` (default `protein`) to say how to read FASTA input; `--cpu` to
parallelize independent reference profiles, including profiles in one HMM file;
taxonomy filters. `--gene_call all|unannotated` calls genes first, exactly as
[`gene_call`](#gene_call) does in its default meta mode, so a bare genome can be annotated
in one step; give `-Z` with it, since the database size is fixed before genes are called.

```bash
domainatoredux domainate -i contigs.gb -r pfam-A.hmm -r rebase_gold.fasta \
  -o annotated.gb -e 1e-5 --cpu 0
```

### `domain_search`

**Keep the records (or the neighborhoods) that hit a reference.** Where `domainate`
annotates everything, `domain_search` selects. It reads possibly sharded, BGZF-compressed
and `.didx`-indexed databases (see [`format_db`](#format_db)), so it is the tool for
searching something large.

It also does **neighborhood extraction**: rather than the hit CDS alone, emit the
surrounding region. `--whole_contig` for the entire contig; `--cds_range`/`--cds_up`/
`--cds_down` for a window measured in genes; `--kb_range`/`--kb_up`/`--kb_down` for a
window in kilobases. `--strand f|r` restricts by strand, `--keep_direction` preserves
orientation, `--pad` pads to equal lengths, `--translate` emits protein instead of
nucleotide, and `--max_region_overlap` deduplicates overlapping windows.

Other key flags: `--add_annotations` to also write `Domainator` features (not just the
`Domain_Search` best hit); `--max_hits` and `--max_hits_per_contig` to cap results
(ranked by the focus CDS's own hit score); `--decoys`/`--decoys_file` to reject hits that
score better against a decoy; `--batch_size` (default 10000); taxonomy filters;
`--gene_call all|unannotated`, as on `domainate`.

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
ends; `--mask` to forbid genes across runs of `N`; `--fasta_type` (default `nucleotide`);
`--bin_path` to name the `orphos` binary.

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

Key flags: `-e/--evalue` and `-m/--max_overlap` thresholds; `--domains`/`--domains_file`
to keep only named domains; `--databases_keep` and `--databases_filter` to act per
database; `--annotation_type` to choose `Domainator`, `Domain_Search` or all.

```bash
domainatoredux filter_domains -i annotated.gb -o clean.gb -e 1e-10 --max_overlap 0.5
```

---

## Selection and extraction

### `select_by_cds`

Extract contig regions around selected CDSs. Same neighborhood geometry as
`domain_search`, but selecting on what is *already annotated* rather than by running a
search.

Select CDSs by `--cds`/`--cds_file` (name), `--domains`/`--domains_file` (domain content),
`--domain_expr` (a boolean expression such as `'Sod_Fe_N and not Sod_Fe_C'`),
`--search_hits` (CDSs carrying a `Domain_Search` feature), or `--unannotated`. Then set
the window with `--whole_contig`, `--cds_range`/`--cds_up`/`--cds_down` or
`--kb_range`/`--kb_up`/`--kb_down`. `--invert` selects the complement,
`--include_nucleic_acids` keeps nucleotide records, `--skip_deduplicate` keeps
overlapping windows separate.

```bash
domainatoredux select_by_cds -i annotated.gb -o operons.gb \
  --domain_expr 'Cas9 or Cas12a' --cds_up 3 --cds_down 3
```

### `select_by_contig`

Select whole contigs, no extraction. Filter by name (`--contigs`, `--contigs_file`,
`--contigs_regex`), description (`--definition_regex`), sequence (`--sequence_regex`),
domain content (`--domains`, `--domain_expr`, `--domain_type domain|search|both`),
taxonomy, size (`--length_lb`/`--length_ub`, `--cds_count_lb`/`--cds_count_ub`), or search
score (`--search_score_lb`/`--search_score_ub`). `--first` takes the first N,
`--invert` complements, `--fasta_out` writes FASTA.

```bash
domainatoredux select_by_contig -i all.gb -o big_hits.gb \
  --domains Cas9 --length_lb 5000 --cds_count_lb 3
```

### `select_by_coord`

Extract regions at explicit 1-based coordinates listed in a TSV/CSV table.

```bash
domainatoredux select_by_coord -i contigs.gb -c regions.tsv -o regions.gb
```

### `extract_domains`

Extract just the domain-annotated regions as their own records. `--pad_up`/`--pad_down`
extend the boundaries (negative values trim), `--combine` merges adjacent domains into one
record, `--splice` joins non-adjacent ones, `--keep_direction` and `--keep_name` control
orientation and naming, `--fasta_out` writes FASTA. Filter by `-e/--evalue`, `--score`,
`--domains`, `--databases` or `--search_hits`.

```bash
domainatoredux extract_domains -i annotated.gb -o domains.fasta \
  --domains Sod_Fe_N --pad_up 10 --pad_down 10 --fasta_out
```

### `extract_peptides`

Extract the peptide sequence of selected CDSs. Select with `--cdss`/`--cdss_file` (note
the double `s`, unlike `select_by_cds`), `--domains`, `--databases`, `--search_hits`,
`--unannotated` or `--strand`; `--name_field` chooses the qualifier used for names.

```bash
domainatoredux extract_peptides -i annotated.gb -o proteins.fasta --domains Cas9 --fasta_out
```

### `extract_unannotated`

Extract the parts of proteins that *aren't* covered by a domain annotation — for finding
novel regions. `--largest` keeps only the longest gap per protein, `--lb` sets a minimum
length.

```bash
domainatoredux extract_unannotated -i annotated.gb -o novel.fasta --lb 50 --largest --fasta_out
```

### `trim_contigs`

Trim contig ends, either by size (`--kb_both`, `--kb_left`, `--kb_right`), by gene count
(`--cds_both`, `--cds_left`, `--cds_right`), or by stripping end CDSs until one matches a
domain (`--domains`, `--domain_expr`, `--no_domain`).

```bash
domainatoredux trim_contigs -i neighborhoods.gb -o trimmed.gb --cds_both 1
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
`--filter_by_aa` drops records containing them.

```bash
domainatoredux clean_sequences -i proteins.fasta -o clean.fasta --clean_name --deduplicate_names
```

### `deduplicate_genbank`

Collapse redundant sequences, keeping one representative per cluster.

`--algorithm` selects the backend: **`cd-hit`** (the default), `diamond`, `usearch` — each
needing that external binary — or **`hash`**, which is pure Rust, needs nothing, and does
exact deduplication only (so it requires `--id 1`). `--id` is required and is the identity
threshold in `[0,1]`. `--bin_path` points at the binary; `--cluster_table` writes cluster
membership; `--prefix_count`/`--suffix_count` annotate representative names with the
cluster size; `--both_strands` for nucleotides; `--params` forwards extra arguments.

```bash
domainatoredux deduplicate_genbank -i proteins.fasta -o representatives.fasta \
  --algorithm hash --id 1
domainatoredux deduplicate_genbank -i proteins.fasta -o representatives.fasta \
  --id 0.9 --cluster_table clusters.tsv
```

---

## Color

### `color_genbank`

Assign hex colors to features via the `Color` qualifier, which viewers (including
Geneious and the [desktop app](gui.md)) honor. `--mode domains|cdss|domains_and_cdss`
chooses what gets colored; colors come from a `--color_table` or are assigned
automatically and can be saved with `--color_table_out` so runs stay consistent.
`--search_hit_color` highlights `Domain_Search` features; `--clear` strips existing
colors.

```bash
domainatoredux color_genbank -i annotated.gb -o colored.gb \
  --mode domains_and_cdss --color_table_out colors.tsv
```

### `color_table_to_legend`

Render a color table as an SVG legend to accompany a figure.

```bash
domainatoredux color_table_to_legend -i colors.tsv --svg legend.svg --title "Domains"
```

---

## Matrices and networks

### `seq_dist`

**Pairwise similarity by local alignment.** Omit `-r` for all-vs-all. Write the matrix with
`--dense` (HDF5), `--sparse` (CSR HDF5) or `--dense_text` (TSV) — at least one is
required.

`--algorithm` picks the engine: `diamond_f`, `diamond_d`, `diamond_mids`, `diamond_s`,
`diamond_mors`, `diamond_vs`, `diamond_us` (the default, ultra-sensitive) all need the
external `diamond` binary; **`hmmer`** (phmmer/hmmsearch) and `hmmer_compare` (HMM-vs-HMM,
selected automatically for two HMM inputs) are built in. `--mode` sets what the cells
contain (`score` by default); `--lb` zeroes small scores; `-k` caps entries per row;
`--mst_knn`/`--knn` prune the graph; `--params` forwards DIAMOND options.

```bash
domainatoredux seq_dist -i proteins.fasta --dense dist.hdf5 --mode score
domainatoredux seq_dist -i proteins.fasta --dense dist.hdf5 --algorithm hmmer --cpu 8
```

### `compare_contigs`

**Pairwise similarity by shared domain content** — the right comparison for gene
neighborhoods rather than single proteins. Scores by Jaccard index (`--ji`) and adjacency
index (`--ai`), weighted; at least one must be non-zero. Restrict to particular
`--databases`; same `--dense`/`--sparse`/`--dense_text` outputs and `--lb`/`-k`/`--mst_knn`
pruning as `seq_dist`.

```bash
domainatoredux compare_contigs -i neighborhoods.gb --dense similarity.hdf5 --ji 0.5 --ai 0.5
```

### `transform_matrix`

Normalize, convert or sparsify an existing matrix. `--mode` is one of `none`, `score`,
`bool`, `norm_score`, `row_norm_score`, `score_dist`, `efi_score`, `efi_score_dist`.
Convert between representations by choosing a different output flag from the input's.
`--lb` applies a lower bound; `--mst_knn` and `--knn` prune.

```bash
domainatoredux transform_matrix -i dist.hdf5 --dense normalized.hdf5 --mode efi_score
domainatoredux transform_matrix -i dense.hdf5 --sparse sparse.hdf5 --lb 50
```

### `build_ssn`

Build a **Cytoscape XGMML** network from a matrix. `--lb` is the score cutoff — the single
most important choice, so consult [`matrix_report`](#matrix_report) first. `--mst` keeps
only maximum-spanning-tree edges and `--mst_knn` keeps MST plus k nearest neighbors,
both of which keep a large network legible. `--cluster` assigns connected-component
cluster ids and `--cluster_tsv` writes them out. `--metadata` joins TSV columns onto
nodes, and `--color_by`/`--color_table`/`--color_table_out` color by one of them.

```bash
domainatoredux build_ssn -i dist.hdf5 --xgmml network.xgmml --lb 100 \
  --cluster --cluster_tsv clusters.tsv --metadata annotations.tsv
```

### `build_ssn_viewer`

Build a `.dsnv` bundle for the **interactive viewer** — no cutoff needed, because the
viewer lets you move the threshold instead. `--html` also writes a standalone HTML page,
and `--embed_data` puts the data inside it so the page is self-contained and shareable.
`--metadata` joins TSV columns, `--color_by`/`--label_by` choose which colors and labels
nodes, and `--categorical` marks columns that should be treated as categories rather than
numbers.

```bash
domainatoredux build_ssn_viewer -i dist.hdf5 -o network.dsnv --html network.html \
  --embed_data --metadata annotations.tsv --color_by phylum --categorical phylum
```

There is deliberately no `--max_merge_events` here: the bundle stores only nodes, the
spanning tree and the hierarchy, and everything else is derived when read, so the viewer
(and [`ssn_navigator`](#ssn_navigator)) chooses that cap at read time.

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

Outputs are `-o/--tsv_out` and `--json_out` (streaming NDJSON, one object per row); give
at least one. `--partial` filters rows by fragment status. The taxonomy columns read
`--ncbi_taxonomy_path` (extracted `.dmp` files or a `taxdump.tar.gz`), which is never
downloaded.

```bash
domainatoredux enum_report -i annotated.gb --by cds \
  --columns domains length molecular_weight isoelectric_point taxname:genus:species \
  -o table.tsv
```

HTML output (`--html_out`) and `--taxonomy_update` are not ported. A config file may
still name them at their defaults (`null`, `false`), so upstream's configs load.

### `summary_report`

Dataset-level statistics: contig and CDS counts, length distribution, domain frequency,
taxonomy. `--domains` and `--databases` restrict the rollup; `--taxonomy` adds a taxonomic
breakdown. `--json <path|->` emits one JSON object. The counts include partial CDSs and
protein fragments (`partial_cdss`, `fragment_proteins`), and `--partial` restricts what
is counted.

```bash
domainatoredux summary_report -i annotated.gb -o summary.txt
domainatoredux summary_report -i annotated.gb --json - | jq .domain_counts
```

Writes text to stdout if you request no output at all. HTML and the ASCII length histogram
are not ported; the [desktop app's Summary tab](gui.md#viewers) covers that visually.

### `matrix_report`

Summarize a similarity matrix: node and edge counts, score statistics, a histogram, and
**suggested clustering thresholds** — the score values where the network structure
changes. Run this before choosing `build_ssn --lb`.

`--max_merge_events` (default 500, `0` = all) caps the merge-event table;
`--merge_impact_metric min_child|product` changes how an event's significance is scored.
`--json` writes the whole report as one object. At least one of `-o` and `--json` is
required.

```bash
domainatoredux matrix_report -i dist.hdf5 -o report.txt
domainatoredux matrix_report -i dist.hdf5 --json report.json
```

### `ssn_navigator`

Query a built `.dsnv` bundle from a script — the non-interactive counterpart to the viewer.
Output is always compact JSON, to `-o` or stdout.

`--mode` selects the question: `overview` (default) for the shape of the network,
`thresholds` for the cluster-count-versus-cutoff table, `clusters` for the clusters at a
given `--threshold`, `cluster` for one cluster's contents (`--id`, `--members`), and
`node` for one node's context (`--node`). `--min_size` and `--top_n` filter and cap
cluster listings.

`--node` must match a node's label exactly, as it appears in the bundle — usually the full
sequence id, not a prefix. `--mode overview` lists some to copy from.

```bash
domainatoredux ssn_navigator -i network.dsnv --mode overview
domainatoredux ssn_navigator -i network.dsnv --mode clusters --threshold 120 --min_size 3
domainatoredux ssn_navigator -i network.dsnv --mode node \
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

Build a profile from a **Stockholm** multiple sequence alignment. `--name` is required.
`--acc` and `--desc` set the accession and description, `--alphabet` overrides alphabet
detection, and `--window_length` applies to nucleotide models.

```bash
domainatoredux hmmer_build -i alignment.sto -o profile.hmm --name MyDomain --desc "My domain"
```

Stockholm is the only input format, and the alignment must be a file (not stdin).

### `hmmer_select`

Pull a subset of profiles out of one or more `.hmm` files by text matching on their NAME,
DESC or ACC fields. `--regex`, `--exact` and `--contains` are the match modes, `--field`
restricts which field is matched, `--case_sensitive` and `--alphabet` refine further.

```bash
domainatoredux hmmer_select -i Pfam-A.hmm.gz -o nucleases.hmm --contains nuclease --field desc
```

### `hmmer_search`

Keep the query profiles that score well against a set of reference profiles — a profile
"search" rather than a sequence search. `--score_cutoff` and `--max_hits` control what is
kept.

```bash
domainatoredux hmmer_search -i candidates.hmm -r known.hmm -o related.hmm --score_cutoff 20
```

### `hmmer_compare`

Align profiles against profiles (hhsearch-like, by profile-vs-profile Viterbi) and write a
scores table. `--alignments` includes the alignments, `--score_cutoff` and `-k` limit
output.

```bash
domainatoredux hmmer_compare -i query.hmm -r pfam-A.hmm -o scores.tsv --alignments
```

To build a network *of* profiles, feed the same files to
[`seq_dist`](#seq_dist) — with two HMM inputs it selects `hmmer_compare` automatically —
and then to [`build_ssn_viewer`](#build_ssn_viewer).

### `hmmer_report`

Tabulate profile metadata. Like `enum_report`, `--columns` takes the columns in output
order: `source`, `acc`, `desc`, `length`, `consensus`, `alphabet`, and
`append:<name>:<str|int|float>:<value>`. Outputs are `-o/--tsv_out` and `--json_out`
(streaming NDJSON).

```bash
domainatoredux hmmer_report -i pfam-A.hmm --columns acc desc length -o profiles.tsv
```

HTML output is not ported.

---

## Database preparation

### `format_db`

Prepare a sequence collection for fast repeated searching by
[`domain_search`](#domain_search): split it into shards (`--shards`), BGZF-compress it
(`--compress`, also spelled `--bgzf`) so it stays seekable, and build a `.didx` index
(`--index`). `--output_dir` chooses where the result goes, `--name` names a single-input
database, `--level` sets the compression level (1–12, default 6), `--force` overwrites and
`--log` redirects the log.

```bash
domainatoredux format_db -i uniprot.fasta --output_dir uniprot_db \
  --shards 8 --compress --index
domainatoredux domain_search -i uniprot_db/ -r query.hmm -o hits.gb
```

Formatted databases are byte-compatible with the Python suite's, in both directions.
Plain (non-BGZF) gzip is rejected, because the whole seekable-I/O layer depends on block
compression. See [File formats](file-formats.md#formatted-databases).
