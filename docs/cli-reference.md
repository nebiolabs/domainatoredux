# Command-line reference

```
domainator <tool> [options]
```

`domainator --help` lists all 34 tools; `domainator <tool> --help` is the exhaustive,
always-current list of that tool's flags. This page gives each tool's purpose, the flags
that matter in practice, and a worked example. Filenames in the examples are
placeholders.

## Conventions that apply everywhere

These trip people up, so read them once:

**Subcommands use hyphens, flags use underscores.** The tool is
`domainator domain-search`; its flags are `--max_overlap`, `--dense_text`,
`--add_annotations`. This mirrors the Python suite's flag names exactly.

**There are no global flags.** `--config`, `--print_config`, `--cpu`, `--json` and
`--max_output_gb` are declared per tool, so they must come *after* the tool name, and a
tool that doesn't declare one doesn't accept it.

```bash
domainator domainate --cpu 8 -i in.gb -r db.hmm -o out.gb   # correct
domainator --cpu 8 domainate -i in.gb -r db.hmm -o out.gb   # error
```

**Required arguments are checked after the config file is merged.** A missing input is
reported as `missing required 'input' (-i / --input or config)` rather than a usage error,
because it might legitimately have come from `--config`. See
[Config files](config-and-agents.md).

**`-i` is the file being edited, `-r` is what you're editing it by.** See
[Concepts](concepts.md#the--i---r-convention).

**`--cpu 0` means all cores**, and 0 is the default. Only `domainate`, `domain-search`,
`deduplicate`, `seq-dist`, `compare-contigs`, `hmmer-compare` and `hmmer-search` are
threaded; output is deterministic regardless of thread count.

**stdin and stdout.** Tools that accept multiple inputs read GenBank from stdin when `-i`
is omitted, and most tools write to stdout when `-o` is omitted, so they pipe:

```bash
domainator select-by-contig -i in.gb --domains Sod_Fe_N | domainator enum-report --domains
```

A stdin input is spilled to a temporary file under your system temp directory, so a
read-only or tiny `TMPDIR` will cause failures on large piped inputs.

**`--max_output_gb`** (default 25, `0` disables) aborts a run whose projected matrix or
network output would exceed that size, before it writes. On `domain-search`, `seq-dist`,
`compare-contigs`, `transform-matrix`, `build-ssn`, `build-ssn-viewer`, `hmmer-compare`
and `hmmer-search`.

**Taxonomy flags** (`--include_taxids`, `--exclude_taxids`, `--taxonomy_expr`) need an
NCBI taxdump directory, given by `--ncbi_taxonomy_path`; it defaults to
`/tmp/ncbi_taxonomy_$USER` and is only loaded if you actually use a taxonomy filter or
column. Both extracted `.dmp` files and a `taxdump.tar.gz` are accepted.

## The tools at a glance

| Role | Tools |
| --- | --- |
| [I/O and diagnostics](#io-and-diagnostics) | `inspect`, `convert`, `genbank-to-fasta`, `schema` |
| [Annotation and search](#annotation-and-search) | `domainate`, `domain-search`, `filter-domains` |
| [Selection and extraction](#selection-and-extraction) | `select-by-cds`, `select-by-contig`, `select-by-coord`, `extract-domains`, `extract-peptides`, `extract-unannotated`, `trim-contigs`, `sort-contigs`, `clean-sequences`, `deduplicate` |
| [Color](#color) | `color-genbank`, `color-table-to-legend` |
| [Matrices and networks](#matrices-and-networks) | `seq-dist`, `compare-contigs`, `transform-matrix`, `build-ssn`, `build-ssn-viewer` |
| [Reports](#reports) | `enum-report`, `summary-report`, `matrix-report`, `ssn-navigator` |
| [HMM profiles](#hmm-profiles) | `hmmer-build`, `hmmer-select`, `hmmer-search`, `hmmer-compare`, `hmmer-report` |
| [Database preparation](#database-preparation) | `format-db` |

---

## I/O and diagnostics

### `inspect`

Parse a sequence file and report a per-record summary. The fastest way to check that a
file is readable and see what is in it. Takes a positional path, not `-i`.

```bash
domainator inspect contigs.gb
domainator inspect contigs.gb --verbose
```

### `convert`

Read GenBank or FASTA and write it back out as GenBank. Two positional arguments, no
flags. Useful for normalizing a file or checking round-trip fidelity.

```bash
domainator convert input.fasta output.gb
```

### `genbank-to-fasta`

Write the sequences of GenBank records as FASTA, dropping annotations.

```bash
domainator genbank-to-fasta -i annotated.gb -o sequences.fasta
```

### `schema`

Print the draft-07 JSON Schema for a tool's `--config` file. Accepts hyphen or underscore
spellings. Covered for the 30 config-backed tools — not `inspect`, `convert` or `schema`
itself.

```bash
domainator schema domain-search
domainator schema seq_dist | jq '.properties | keys'
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
`--fasta_type` (default `protein`) to say how to read FASTA input; `--cpu`; taxonomy
filters.

```bash
domainator domainate -i contigs.gb -r pfam-A.hmm -r rebase_gold.fasta \
  -o annotated.gb -e 1e-5 --cpu 0
```

### `domain-search`

**Keep the records (or the neighborhoods) that hit a reference.** Where `domainate`
annotates everything, `domain-search` selects. It reads possibly sharded, BGZF-compressed
and `.didx`-indexed databases (see [`format-db`](#format-db)), so it is the tool for
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
score better against a decoy; `--batch_size` (default 10000); taxonomy filters.

```bash
# every gene within 10 kb of a Cas9 hit, across a formatted database
domainator domain-search -i genomes_db/ -r cas9.hmm -o neighborhoods.gb \
  --kb_range 10 --add_annotations -e 1e-10
```

### `filter-domains`

Remove domain annotations without touching the sequences — the cleanup pass after
annotating generously.

Key flags: `-e/--evalue` and `-m/--max_overlap` thresholds; `--domains`/`--domains_file`
to keep only named domains; `--databases_keep` and `--databases_filter` to act per
database; `--annotation_type` to choose `Domainator`, `Domain_Search` or all.

```bash
domainator filter-domains -i annotated.gb -o clean.gb -e 1e-10 --max_overlap 0.5
```

---

## Selection and extraction

### `select-by-cds`

Extract contig regions around selected CDSs. Same neighborhood geometry as
`domain-search`, but selecting on what is *already annotated* rather than by running a
search.

Select CDSs by `--cds`/`--cds_file` (name), `--domains`/`--domains_file` (domain content),
`--domain_expr` (a boolean expression such as `'Sod_Fe_N and not Sod_Fe_C'`),
`--search_hits` (CDSs carrying a `Domain_Search` feature), or `--unannotated`. Then set
the window with `--whole_contig`, `--cds_range`/`--cds_up`/`--cds_down` or
`--kb_range`/`--kb_up`/`--kb_down`. `--invert` selects the complement,
`--include_nucleic_acids` keeps nucleotide records, `--skip_deduplicate` keeps
overlapping windows separate.

```bash
domainator select-by-cds -i annotated.gb -o operons.gb \
  --domain_expr 'Cas9 or Cas12a' --cds_up 3 --cds_down 3
```

### `select-by-contig`

Select whole contigs, no extraction. Filter by name (`--contigs`, `--contigs_file`,
`--contigs_regex`), description (`--definition_regex`), sequence (`--sequence_regex`),
domain content (`--domains`, `--domain_expr`, `--domain_type domain|search|both`),
taxonomy, size (`--length_lb`/`--length_ub`, `--cds_count_lb`/`--cds_count_ub`), or search
score (`--search_score_lb`/`--search_score_ub`). `--first` takes the first N,
`--invert` complements, `--fasta_out` writes FASTA.

```bash
domainator select-by-contig -i all.gb -o big_hits.gb \
  --domains Cas9 --length_lb 5000 --cds_count_lb 3
```

### `select-by-coord`

Extract regions at explicit 1-based coordinates listed in a TSV/CSV table.

```bash
domainator select-by-coord -i contigs.gb -c regions.tsv -o regions.gb
```

### `extract-domains`

Extract just the domain-annotated regions as their own records. `--pad_up`/`--pad_down`
extend the boundaries (negative values trim), `--combine` merges adjacent domains into one
record, `--splice` joins non-adjacent ones, `--keep_direction` and `--keep_name` control
orientation and naming, `--fasta_out` writes FASTA. Filter by `-e/--evalue`, `--score`,
`--domains`, `--databases` or `--search_hits`.

```bash
domainator extract-domains -i annotated.gb -o domains.fasta \
  --domains Sod_Fe_N --pad_up 10 --pad_down 10 --fasta_out
```

### `extract-peptides`

Extract the peptide sequence of selected CDSs. Select with `--cdss`/`--cdss_file` (note
the double `s`, unlike `select-by-cds`), `--domains`, `--databases`, `--search_hits`,
`--unannotated` or `--strand`; `--name_field` chooses the qualifier used for names.

```bash
domainator extract-peptides -i annotated.gb -o proteins.fasta --domains Cas9 --fasta_out
```

### `extract-unannotated`

Extract the parts of proteins that *aren't* covered by a domain annotation — for finding
novel regions. `--largest` keeps only the longest gap per protein, `--lb` sets a minimum
length.

```bash
domainator extract-unannotated -i annotated.gb -o novel.fasta --lb 50 --largest --fasta_out
```

### `trim-contigs`

Trim contig ends, either by size (`--kb_both`, `--kb_left`, `--kb_right`), by gene count
(`--cds_both`, `--cds_left`, `--cds_right`), or by stripping end CDSs until one matches a
domain (`--domains`, `--domain_expr`, `--no_domain`).

```bash
domainator trim-contigs -i neighborhoods.gb -o trimmed.gb --cds_both 1
```

### `sort-contigs`

Sort contigs by their best `Domain_Search` score, or by a selected `Domainator`
annotation (`--domains`, `--databases`). `--max_hits` truncates to the best N.

```bash
domainator sort-contigs -i hits.gb -o sorted.gb --max_hits 100
```

### `clean-sequences`

Housekeeping before phylogenetics or alignment: `--clean_name` makes names newick-safe,
`--deduplicate_names` disambiguates collisions, `--strip` removes unwanted residues and
`--filter_by_aa` drops records containing them.

```bash
domainator clean-sequences -i proteins.fasta -o clean.fasta --clean_name --deduplicate_names
```

### `deduplicate`

Collapse redundant sequences, keeping one representative per cluster.

`--algorithm` selects the backend: **`cd-hit`** (the default), `diamond`, `usearch` — each
needing that external binary — or **`hash`**, which is pure Rust, needs nothing, and does
exact deduplication only (so it requires `--id 1`). `--id` is required and is the identity
threshold in `[0,1]`. `--bin_path` points at the binary; `--cluster_table` writes cluster
membership; `--prefix_count`/`--suffix_count` annotate representative names with the
cluster size; `--both_strands` for nucleotides; `--params` forwards extra arguments.

```bash
domainator deduplicate -i proteins.fasta -o representatives.fasta \
  --algorithm hash --id 1
domainator deduplicate -i proteins.fasta -o representatives.fasta \
  --id 0.9 --cluster_table clusters.tsv
```

---

## Color

### `color-genbank`

Assign hex colors to features via the `Color` qualifier, which viewers (including
Geneious and the [desktop app](gui.md)) honor. `--mode domains|cdss|domains_and_cdss`
chooses what gets colored; colors come from a `--color_table` or are assigned
automatically and can be saved with `--color_table_out` so runs stay consistent.
`--search_hit_color` highlights `Domain_Search` features; `--clear` strips existing
colors.

```bash
domainator color-genbank -i annotated.gb -o colored.gb \
  --mode domains_and_cdss --color_table_out colors.tsv
```

### `color-table-to-legend`

Render a color table as an SVG legend to accompany a figure.

```bash
domainator color-table-to-legend -i colors.tsv --svg legend.svg --title "Domains"
```

---

## Matrices and networks

### `seq-dist`

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
domainator seq-dist -i proteins.fasta --dense dist.hdf5 --mode score
domainator seq-dist -i proteins.fasta --dense dist.hdf5 --algorithm hmmer --cpu 8
```

### `compare-contigs`

**Pairwise similarity by shared domain content** — the right comparison for gene
neighborhoods rather than single proteins. Scores by Jaccard index (`--ji`) and adjacency
index (`--ai`), weighted; at least one must be non-zero. Restrict to particular
`--databases`; same `--dense`/`--sparse`/`--dense_text` outputs and `--lb`/`-k`/`--mst_knn`
pruning as `seq-dist`.

```bash
domainator compare-contigs -i neighborhoods.gb --dense similarity.hdf5 --ji 0.5 --ai 0.5
```

### `transform-matrix`

Normalize, convert or sparsify an existing matrix. `--mode` is one of `none`, `score`,
`bool`, `norm_score`, `row_norm_score`, `score_dist`, `efi_score`, `efi_score_dist`.
Convert between representations by choosing a different output flag from the input's.
`--lb` applies a lower bound; `--mst_knn` and `--knn` prune.

```bash
domainator transform-matrix -i dist.hdf5 --dense normalized.hdf5 --mode efi_score
domainator transform-matrix -i dense.hdf5 --sparse sparse.hdf5 --lb 50
```

### `build-ssn`

Build a **Cytoscape XGMML** network from a matrix. `--lb` is the score cutoff — the single
most important choice, so consult [`matrix-report`](#matrix-report) first. `--mst` keeps
only maximum-spanning-tree edges and `--mst_knn` keeps MST plus k nearest neighbors,
both of which keep a large network legible. `--cluster` assigns connected-component
cluster ids and `--cluster_tsv` writes them out. `--metadata` joins TSV columns onto
nodes, and `--color_by`/`--color_table`/`--color_table_out` color by one of them.

```bash
domainator build-ssn -i dist.hdf5 --xgmml network.xgmml --lb 100 \
  --cluster --cluster_tsv clusters.tsv --metadata annotations.tsv
```

### `build-ssn-viewer`

Build a `.dsnv` bundle for the **interactive viewer** — no cutoff needed, because the
viewer lets you move the threshold instead. `--html` also writes a standalone HTML page,
and `--embed_data` puts the data inside it so the page is self-contained and shareable.
`--metadata` joins TSV columns, `--color_by`/`--label_by` choose which colors and labels
nodes, and `--categorical` marks columns that should be treated as categories rather than
numbers.

```bash
domainator build-ssn-viewer -i dist.hdf5 -o network.dsnv --html network.html \
  --embed_data --metadata annotations.tsv --color_by phylum --categorical phylum
```

There is deliberately no `--max_merge_events` here: the bundle stores only nodes, the
spanning tree and the hierarchy, and everything else is derived when read, so the viewer
(and [`ssn-navigator`](#ssn-navigator)) chooses that cap at read time.

---

## Reports

### `enum-report`

**One row per contig, CDS or domain**, chosen with `--by contig|cds|domain`. Column flags
are positional: **the order you pass them is the order of the output columns.** That is
why this tool parses its own arguments and why `--help` shows only a passthrough.

Bare column flags: `--domains`, `--domain_descriptions`, `--sequence`, `--start`, `--end`,
`--strand`, `--length`, `--topology`, `--architecture`, `--architecture_detailed`,
`--score`, `--identity`, `--definition`, `--filename`, `--cds_count`, `--domain_count`,
`--domain_search`, `--taxid_lineage`, `--taxname_lineage`, `--rank_lineage`,
`--isoelectric_point`, `--molecular_weight`, `--starts_with`.

Column flags that take values: `--taxid [ranks...]`, `--taxname [ranks...]`,
`--qualifier <feature> <qualifier>`, `--feature_count <feature>`,
`--motif_count <prosite-pattern>`, `--net_charge <pH>`, `--repeat_count <min_length>`,
`--named_domain_count <name>`, `--append <name> <value_type> <value>` (a constant column).

`--json <path|->` writes streaming NDJSON — one object per row — independently of `-o`, so
a single run can emit both.

```bash
domainator enum-report -i annotated.gb --by cds \
  --domains --length --molecular_weight --isoelectric_point \
  --taxname genus species -o table.tsv
```

HTML output is not ported; use TSV or `--json`.

### `summary-report`

Dataset-level statistics: contig and CDS counts, length distribution, domain frequency,
taxonomy. `--domains` and `--databases` restrict the rollup; `--taxonomy` adds a taxonomic
breakdown. `--json <path|->` emits one JSON object.

```bash
domainator summary-report -i annotated.gb -o summary.txt
domainator summary-report -i annotated.gb --json - | jq .domain_counts
```

Writes text to stdout if you request no output at all. HTML and the ASCII length histogram
are not ported; the [desktop app's Summary tab](gui.md#viewers) covers that visually.

### `matrix-report`

Summarize a similarity matrix: node and edge counts, score statistics, a histogram, and
**suggested clustering thresholds** — the score values where the network structure
changes. Run this before choosing `build-ssn --lb`.

`--max_merge_events` (default 500, `0` = all) caps the merge-event table;
`--merge_impact_metric min_child|product` changes how an event's significance is scored.
`--json` writes the whole report as one object. At least one of `-o` and `--json` is
required.

```bash
domainator matrix-report -i dist.hdf5 -o report.txt
domainator matrix-report -i dist.hdf5 --json report.json
```

### `ssn-navigator`

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
domainator ssn-navigator -i network.dsnv --mode overview
domainator ssn-navigator -i network.dsnv --mode clusters --threshold 120 --min_size 3
domainator ssn-navigator -i network.dsnv --mode node \
  --node "FeSOD_A0A1F4ZT98|unreviewed|Superoxide"
```

---

## HMM profiles

Domainator treats HMM profiles as first-class data: you can build them, subset them,
compare them to each other, and build similarity networks *of profiles*.

### `hmmer-build`

Build a profile from a **Stockholm** multiple sequence alignment. `--name` is required.
`--acc` and `--desc` set the accession and description, `--alphabet` overrides alphabet
detection, and `--window_length` applies to nucleotide models.

```bash
domainator hmmer-build -i alignment.sto -o profile.hmm --name MyDomain --desc "My domain"
```

Stockholm is the only input format, and the alignment must be a file (not stdin).

### `hmmer-select`

Pull a subset of profiles out of one or more `.hmm` files by text matching on their NAME,
DESC or ACC fields. `--regex`, `--exact` and `--contains` are the match modes, `--field`
restricts which field is matched, `--case_sensitive` and `--alphabet` refine further.

```bash
domainator hmmer-select -i pfam-A.hmm -o nucleases.hmm --contains nuclease --field desc
```

### `hmmer-search`

Keep the query profiles that score well against a set of reference profiles — a profile
"search" rather than a sequence search. `--score_cutoff` and `--max_hits` control what is
kept.

```bash
domainator hmmer-search -i candidates.hmm -r known.hmm -o related.hmm --score_cutoff 20
```

### `hmmer-compare`

Align profiles against profiles (hhsearch-like, by profile-vs-profile Viterbi) and write a
scores table. `--alignments` includes the alignments, `--score_cutoff` and `-k` limit
output.

```bash
domainator hmmer-compare -i query.hmm -r pfam-A.hmm -o scores.tsv --alignments
```

To build a network *of* profiles, feed the same files to
[`seq-dist`](#seq-dist) — with two HMM inputs it selects `hmmer_compare` automatically —
and then to [`build-ssn-viewer`](#build-ssn-viewer).

### `hmmer-report`

Tabulate profile metadata. Like `enum-report`, **column flag order is column order**.
Columns: `--source`, `--acc`, `--desc`, `--length`, `--consensus`, `--alphabet`, plus
`--append <name> <value_type> <value>`. `--json <path|->` writes streaming NDJSON.

```bash
domainator hmmer-report -i pfam-A.hmm --acc --desc --length -o profiles.tsv
```

HTML output is not ported.

---

## Database preparation

### `format-db`

Prepare a sequence collection for fast repeated searching by
[`domain-search`](#domain-search): split it into shards (`--shards`), BGZF-compress it
(`--compress`, also spelled `--bgzf`) so it stays seekable, and build a `.didx` index
(`--index`). `--output_dir` chooses where the result goes, `--name` names a single-input
database, `--level` sets the compression level (1–12, default 6), `--force` overwrites and
`--log` redirects the log.

```bash
domainator format-db -i uniprot.fasta --output_dir uniprot_db \
  --shards 8 --compress --index
domainator domain-search -i uniprot_db/ -r query.hmm -o hits.gb
```

Formatted databases are byte-compatible with the Python suite's, in both directions.
Plain (non-BGZF) gzip is rejected, because the whole seekable-I/O layer depends on block
compression. See [File formats](file-formats.md#formatted-databases).
