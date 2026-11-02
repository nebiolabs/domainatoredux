# Config files and agent output

Domainator is designed to be driven by scripts and agents as well as by hand: every tool
can take its parameters from a file, `describe` lists every tool's parameters as JSON,
and the report tools can emit JSON instead of (or as well as) human-readable output.

## Config files

Every tool except `inspect` and `convert` accepts `--config`, pointing at a
JSON or YAML file:

```bash
domainatoredux domainate --config annotate.yml
```

```yaml
# annotate.yml
input:
  - contigs.gb
reference:
  - pfam-A.hmm
  - rebase_gold.fasta
genbank_out: annotated.gb
evalue: 1.0e-5
cpu: 0
```

The key names are the long flag names without the dashes, and the format is
**interchangeable with the Python suite's**: an object keyed by option name, exactly what
`--print_config` prints here (and what `python -m domainator <tool> --print_config`
prints), so a file written for one implementation works with the other. An option the
Python suite has and this port does not is accepted at its upstream default (or `null`),
and is an error that names it otherwise.

### How a config file and the command line combine

The file is turned into command-line options and parsed with the rest, so every check
that applies on the command line (types, bounds, choices) applies to the file too:

- an option given on the command line **replaces** the file's value, lists included;
- a switch is `true` or `false`, and `false` turns it off even if it defaults on;
- `null` or a missing key leaves the default;
- an unknown key is an error that names it.

```bash
# override just the output, keep everything else from the file
domainatoredux sort_contigs --config sort.yml -o other.gb
```

### Seeing the resolved parameters

`--print_config` prints every option with the value it will have, defaults included, and
exits without doing any work. It does not require the required options, so it can also
capture a partial config to finish later:

```bash
domainatoredux sort_contigs -i hits.gb -o sorted.gb --max_hits 100 --print_config > sort.yml
```

## Discovering a tool's parameters

`domainatoredux describe` prints the tool manifest: every tool and every option, with its
flags, type, bounds, default, choices, and whether it is a file to read (`role: input`) or
write (`role: output`), in the format of upstream's `domainator describe`. It is built
from the command-line parser itself, so it cannot disagree with it:

```bash
domainatoredux describe --tool sort_contigs --indent 1
```

A report's `columns` option also lists its columns, each with its argument names
(`{"name": "qualifier", "args": ["feature", "qualifier"], ...}`).

The header says what the manifest describes: `manifest_version` (1, upstream's format),
`domainator_version` (the Python release this build reproduces), `domainatoredux_version`
(this build), and `invocation`, which says how to run a tool (`["domainatoredux",
"{tool}"]`), that `--config` takes an object keyed by parameter name, and that `-` means
stdin or stdout.

This is the interface to use when generating invocations programmatically: the desktop
app renders its tool dialogs from the same manifest, so anything an agent can discover
here is exactly what the GUI exposes.

### How it differs from upstream's

A test compares this manifest with the one upstream's `domainator describe` prints, so
the two differ only where a difference is deliberate and listed. An agent written against
upstream's manifest can expect the same tool names, parameter names, types, defaults and
choices here, with these exceptions:

- tools upstream has and this port does not (`db_download`;
  see [Porting status](porting-status.md));
- options of ported tools that are not ported. None remain: `domainate
  --esm2_3di_weights`, which needs PyTorch, is in the manifest, and domainate refuses it
  when it is used;
- `gene_call`, `inspect` and `convert`, which upstream does not have (`inspect` and
  `convert` take no config and are not in the manifest).

`domainatoredux doctor` reports which optional external programs (diamond, cd-hit,
usearch, Orphos, cmbuild, foldseek, reseek) this installation can find, and which tools need each one.

## JSON output

Four tools write machine-readable output on request with `--json_out`: `enum_report`,
`hmmer_report`, `matrix_report` and `summary_report`. In every case the
option **takes a path, `-` means stdout, and it is independent of `-o`** — so one run can
produce a human-readable report and a JSON one at the same time. `ssn_navigator` always
writes JSON.

| Tool | Shape |
| --- | --- |
| [`enum_report`](cli-reference.md#enum_report) | **streaming NDJSON** — one compact object per row |
| [`hmmer_report`](cli-reference.md#hmmer_report) | **streaming NDJSON** — one object per profile |
| [`matrix_report`](cli-reference.md#matrix_report) | one JSON object, stable shape |
| [`summary_report`](cli-reference.md#summary_report) | one JSON object, stable shape |
| [`ssn_navigator`](cli-reference.md#ssn_navigator) | always JSON — it has no other mode |

NDJSON streams, so it works on datasets far larger than memory:

```bash
domainatoredux enum_report -i annotated.gb --columns domains length --json_out -
```

```
{"contig":"FeSOD_A0A1F4ZT98|unreviewed|Superoxide","domains":"Sod_Fe_C; Sod_Fe_N","length":196}
{"contig":"FeSOD_A0A067LT26|unreviewed|Superoxide","domains":"Sod_Fe_C; Sod_Fe_N","length":208}
```

Both at once:

```bash
domainatoredux enum_report -i annotated.gb --columns domains length -o table.tsv --json_out rows.ndjson
```

The object keys are the column names, in the order you asked for the columns.

HTML output is **not** available for any report tool; `--html_out` is an error. Use TSV
or JSON. The [desktop app](gui.md) covers the visual case.

## Querying a built network

[`ssn_navigator`](cli-reference.md#ssn_navigator) is the scriptable counterpart to the
viewer — it answers questions about the network in a `.dvb` bundle (`build_ssn --dvb_out`,
or an older `.dsnv`) as JSON, with no display:

```bash
# how is the network shaped?
domainatoredux ssn_navigator -i network.dvb -o - --query overview

# where are the interesting cutoffs?
domainatoredux ssn_navigator -i network.dvb -o - --query thresholds

# what clusters exist at a cutoff, biggest first?
domainatoredux ssn_navigator -i network.dvb -o - --query clusters --threshold 120 --min_size 3

# what's in one cluster?
domainatoredux ssn_navigator -i network.dvb -o - --query cluster --cluster 4 --members

# what's around one node?
domainatoredux ssn_navigator -i network.dvb -o - --query node \
  --node "FeSOD_A0A1F4ZT98|unreviewed|Superoxide"
```

Because the bundle stores the spanning tree and hierarchy rather than a fixed network, you
can ask about any threshold without rebuilding anything.

## A worked agent loop

A reasonable shape for automated analysis:

```bash
# 1. discover what the tool takes
domainatoredux describe --tool domain_search > domain_search.json

# 2. run it from a generated config
domainatoredux domain_search --config search.json

# 3. read the results as data
domainatoredux summary_report -i hits.gb --json_out - > summary.json
domainatoredux enum_report -i hits.gb --by cds --columns domains score --json_out rows.ndjson

# 4. build a network and explore it without a GUI
domainatoredux seq_dist -i hits.gb -o dist.hdf5 --algorithm hmmer
domainatoredux build_ssn -i dist.hdf5 --dvb_out network.dvb
domainatoredux ssn_navigator -i network.dvb -o - --query thresholds
```

Every step is deterministic: tools that use multiple threads still produce identical
output regardless of the thread count.
