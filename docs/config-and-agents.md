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

The key names are the long flag names without the dashes.

The tools are moving, one at a time, to the Domainator 1.0 interface, whose config files
are **interchangeable with the Python suite's** (`domainate` and the selection,
extraction and editing tools have moved; [porting status](porting-status.md) has the
list). The rest of this section describes the older, native format of the tools not yet
moved, which is *not* compatible with the Python suite's files. See
[Tools on the 1.0 interface](#tools-on-the-10-interface) for the new one.

### How a config file and the command line combine

The config file is the base, and command-line flags override it:

| Kind of option | Behavior |
| --- | --- |
| scalars (`algorithm`, `cpu`, `dense`) | the CLI value replaces the file's |
| lists (`references`, `domains`) | the CLI value replaces the file's, when non-empty |
| booleans (`hits_only`, `max_mode`) | **OR'd together** |

```bash
# override just the algorithm, keep everything else from the file
domainatoredux seq_dist --config dist.yml --algorithm hmmer_compare
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
domainatoredux seq_dist -i a.gb -r b.gb --dense d.hdf5 --print_config
```

```yaml
input: a.gb
reference: b.gb
dense: d.hdf5
dense_text: null
sparse: null
algorithm: null
mode: null
...
```

`null` means "not set, so the tool's built-in default applies" — it is *not* the effective
value. Use it to check that a config file and your flags combined the way you expected,
and `--help` to see what each default actually is.

The output is a valid config file, so this is also how you capture a working invocation:

```bash
domainatoredux seq_dist -i a.gb -r b.gb --dense d.hdf5 --print_config > dist.yml
```

### Tools on the 1.0 interface

A migrated tool reads upstream's format: an object keyed by option name, exactly what
`--print_config` prints (and what `python -m domainator <tool> --print_config` prints),
so a file written for one implementation works with the other.

```yaml
# sort.yml
input:
- hits.gb
genbank_out: sorted.gb
max_hits: 100
```

The file is turned into command-line options and parsed with the rest, so every check
that applies on the command line (types, bounds, choices) applies to the file too:

- an option given on the command line **replaces** the file's value, lists included;
- a switch is `true` or `false`, and `false` turns it off even if it defaults on;
- `null` or a missing key leaves the default;
- an unknown key is an error that names it.

`--print_config` prints every option with the value it will have, defaults included, and
does not require the required ones, so it can capture a partial config to finish later.

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

This is the interface to use when generating invocations programmatically: the desktop
app renders its tool dialogs from the same manifest, so anything an agent can discover
here is exactly what the GUI exposes. For a tool not yet on the 1.0 interface, its entry
shows its current options, and its config keys are the option names it lists.

## JSON output

Four tools write machine-readable output on request: `enum_report` and `hmmer_report`
with `--json_out`, `matrix_report` and `summary_report` with `--json`. In every case the
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

[`ssn_navigator`](cli-reference.md#ssn_navigator) is the scriptable counterpart to the SSN
viewer — it answers questions about a `.dsnv` bundle as JSON, with no display:

```bash
# how is the network shaped?
domainatoredux ssn_navigator -i network.dsnv --mode overview

# where are the interesting cutoffs?
domainatoredux ssn_navigator -i network.dsnv --mode thresholds

# what clusters exist at a cutoff, biggest first?
domainatoredux ssn_navigator -i network.dsnv --mode clusters --threshold 120 --min_size 3

# what's in one cluster?
domainatoredux ssn_navigator -i network.dsnv --mode cluster --id 4 --members

# what's around one node?
domainatoredux ssn_navigator -i network.dsnv --mode node \
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
domainatoredux summary_report -i hits.gb --json - > summary.json
domainatoredux enum_report -i hits.gb --by cds --columns domains score --json_out rows.ndjson

# 4. build a network and explore it without a GUI
domainatoredux seq_dist -i hits.gb --dense dist.hdf5 --algorithm hmmer
domainatoredux build_ssn_viewer -i dist.hdf5 -o network.dsnv
domainatoredux ssn_navigator -i network.dsnv --mode thresholds
```

Every step is deterministic: tools that use multiple threads still produce identical
output regardless of the thread count.
