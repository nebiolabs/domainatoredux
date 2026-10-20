# Config files and agent output

Domainator is designed to be driven by scripts and agents as well as by hand: every tool
can take its parameters from a file, describe its own parameters as a JSON Schema, and
the report tools can emit JSON instead of (or as well as) human-readable output.

## Config files

Every tool except `inspect`, `convert` and `schema` accepts `--config`, pointing at a
JSON or YAML file:

```bash
domainator domainate --config annotate.yml
```

```yaml
# annotate.yml
input: contigs.gb
references:
  - pfam-A.hmm
  - rebase_gold.fasta
output: annotated.gb
evalue: 1.0e-5
cpu: 0
```

The key names are the long flag names without the dashes. This is a **native Rust schema**
and is deliberately *not* compatible with the Python suite's jsonargparse config files.

### How a config file and the command line combine

The config file is the base, and command-line flags override it:

| Kind of option | Behavior |
| --- | --- |
| scalars (`evalue`, `cpu`, `output`) | the CLI value replaces the file's |
| lists (`references`, `domains`) | the CLI value replaces the file's, when non-empty |
| booleans (`hits_only`, `max_mode`) | **OR'd together** |
| report columns (`enum-report`, `hmmer-report`) | appended |

```bash
# override just the E-value, keep everything else from the file
domainator domainate --config annotate.yml -e 1e-10
```

Two things to watch:

- **A boolean set `true` in a config file cannot be turned off from the command line**,
  because booleans are OR'd. Keep flags you want to toggle out of the config file.
- **Unknown keys are an error**, not a warning. A typo is caught rather than silently
  ignored.

Because inputs may legitimately come from the file, required arguments are validated
*after* merging, so a missing input reports
`missing required 'input' (-i / --input or config)` rather than a usage error.

### Seeing the resolved parameters

`--print_config` dumps the merged parameter set as YAML and exits without doing any work:

```bash
domainator domainate -i contigs.gb -r pfam-A.hmm -o out.gb --print_config
```

```yaml
input: contigs.gb
references:
- pfam-A.hmm
output: out.gb
evalue: null
min_evalue: null
z: null
max_overlap: null
...
```

`null` means "not set, so the tool's built-in default applies" — it is *not* the effective
value. Use it to check that a config file and your flags combined the way you expected,
and `--help` to see what each default actually is.

The output is a valid config file, so this is also how you capture a working invocation:

```bash
domainator domainate -i contigs.gb -r pfam-A.hmm -o out.gb --print_config > annotate.yml
```

## Discovering a tool's parameters

`domainator schema <tool>` prints a draft-07 JSON Schema describing that tool's config
file — every option, its type, its allowed values, and a description:

```bash
domainator schema domain-search
domainator schema seq_dist | jq -r '.properties | keys[]'
```

Tool names are accepted with hyphens or underscores. The 32 config-backed tools are
covered; `inspect`, `convert` and `schema` have no config and no schema.

This is the interface to use when generating invocations programmatically: it is the same
schema the desktop app renders its tool dialogs from, so anything an agent can discover
here is exactly what the GUI exposes.

## JSON output

Four tools write machine-readable output with `--json`. In every case **`--json` takes a
path, `-` means stdout, and it is independent of `-o`** — so one run can produce a
human-readable report and a JSON one at the same time.

| Tool | Shape |
| --- | --- |
| [`enum-report`](cli-reference.md#enum-report) | **streaming NDJSON** — one compact object per row |
| [`hmmer-report`](cli-reference.md#hmmer-report) | **streaming NDJSON** — one object per profile |
| [`matrix-report`](cli-reference.md#matrix-report) | one JSON object, stable shape |
| [`summary-report`](cli-reference.md#summary-report) | one JSON object, stable shape |
| [`ssn-navigator`](cli-reference.md#ssn-navigator) | always JSON — it has no other mode |

NDJSON streams, so it works on datasets far larger than memory:

```bash
domainator enum-report -i annotated.gb --domains --length --json -
```

```
{"contig":"FeSOD_A0A1F4ZT98|unreviewed|Superoxide","domains":"Sod_Fe_C; Sod_Fe_N","length":196}
{"contig":"FeSOD_A0A067LT26|unreviewed|Superoxide","domains":"Sod_Fe_C; Sod_Fe_N","length":208}
```

Both at once:

```bash
domainator enum-report -i annotated.gb --domains --length -o table.tsv --json rows.ndjson
```

The object keys are the column names, in the order you asked for the columns.

HTML output is **not** available for any report tool; `--html` is a hard error. Use TSV or
JSON. The [desktop app](gui.md) covers the visual case.

## Querying a built network

[`ssn-navigator`](cli-reference.md#ssn-navigator) is the scriptable counterpart to the SSN
viewer — it answers questions about a `.dsnv` bundle as JSON, with no display:

```bash
# how is the network shaped?
domainator ssn-navigator -i network.dsnv --mode overview

# where are the interesting cutoffs?
domainator ssn-navigator -i network.dsnv --mode thresholds

# what clusters exist at a cutoff, biggest first?
domainator ssn-navigator -i network.dsnv --mode clusters --threshold 120 --min_size 3

# what's in one cluster?
domainator ssn-navigator -i network.dsnv --mode cluster --id 4 --members

# what's around one node?
domainator ssn-navigator -i network.dsnv --mode node \
  --node "FeSOD_A0A1F4ZT98|unreviewed|Superoxide"
```

Because the bundle stores the spanning tree and hierarchy rather than a fixed network, you
can ask about any threshold without rebuilding anything.

## A worked agent loop

A reasonable shape for automated analysis:

```bash
# 1. discover what the tool takes
domainator schema domain-search > schema.json

# 2. run it from a generated config
domainator domain-search --config search.json

# 3. read the results as data
domainator summary-report -i hits.gb --json - > summary.json
domainator enum-report -i hits.gb --by cds --domains --score --json rows.ndjson

# 4. build a network and explore it without a GUI
domainator seq-dist -i hits.gb --dense dist.hdf5 --algorithm hmmer
domainator build-ssn-viewer -i dist.hdf5 -o network.dsnv
domainator ssn-navigator -i network.dsnv --mode thresholds
```

Every step is deterministic: tools that use multiple threads still produce identical
output regardless of the thread count.
