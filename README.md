# Domainator (Rust)

Domainator annotates sequences with protein domains, extracts the genome neighborhoods
around them, and turns collections of sequences into **sequence similarity networks
(SSNs)** you can explore interactively.

This is a from-scratch Rust implementation of the
[Python Domainator suite](https://github.com/nebiolabs/domainator). It ships as:

- **one command-line executable** — `domainatoredux <tool> [options]`, 35 tools, no
  interpreter or conda environment required;
- **one desktop app** — a workspace browser with a contig viewer, an interactive SSN
  viewer, HMM logos and dialogs for running the tools;
- **files that interoperate** with the Python suite in both directions.

HMMER3 is built in as pure Rust, so a basic install needs nothing else at all.

## Install

Download from the [Releases page](https://github.com/nebiolabs/domainatoredux/releases):

| Platform | Desktop app | Command-line tool |
| --- | --- | --- |
| macOS (Apple silicon) | `.dmg` | `domainatoredux` |
| Windows (x64) | NSIS installer | `domainatoredux.exe` |
| Linux (x64) | `.deb`, `.AppImage` | `domainatoredux` |

Or build it — you need [Rust](https://rustup.rs/) 1.89+ and nothing else:

```bash
cargo build --release -p domainator-cli
./target/release/domainatoredux --help
```

Full details, including the three optional external programs, are in
**[Installation](docs/installation.md)**.

## Quickstart

Annotate 20 proteins, score them all against each other, and build an interactive
network — using two files from `test/data/`:

```bash
domainatoredux domainate -i FeSOD_20.gb -r FeSOD_pfam.hmm -o annotated.gb
#> wrote 20 records to annotated.gb

domainatoredux seq_dist -i annotated.gb -o dist.hdf5 --algorithm hmmer --mode score
#> wrote 20x20 matrix

domainatoredux build_ssn_viewer -i dist.hdf5 -o network.dsnv --html_out network.html --embed_data
#> wrote 20-node .dsnv bundle to network.dsnv
#> wrote HTML viewer to network.html
```

`network.html` is self-contained: open it in any browser and drag the threshold slider to
split the network into clusters.

Walked through step by step in **[Getting started](docs/getting-started.md)**.

> One convention to know up front: **`-i` is the file being edited, `-r` is the criteria
> you're editing it by.** So searching a database for hits to your query puts the
> *database* in `-i`. See [Concepts](docs/concepts.md).

## The tools

`domainatoredux --help` lists them all; `domainatoredux <tool> --help` documents one. Flags use
the Python suite's underscore spellings (`--max_overlap`), and so do subcommands
(`domain_search`). `domainatoredux <tool>` takes the same tool names as Python's
`domainator <tool>`; the different executable name lets both be installed side by side.

| | |
| --- | --- |
| **Annotation and search** | `domainate` annotate with hits to references · `domain_search` keep the records or neighborhoods that hit · `filter_domains` prune weak annotations |
| **Selection and extraction** | `select_by_cds` · `select_by_contig` · `select_by_coord` · `extract_domains` · `extract_peptides` · `extract_unannotated` · `trim_contigs` · `sort_contigs` · `clean_sequences` · `deduplicate_genbank` |
| **Matrices and networks** | `seq_dist` similarity by alignment · `compare_contigs` similarity by shared domain content · `transform_matrix` · `build_ssn` (Cytoscape XGMML) · `build_ssn_viewer` (interactive) |
| **Reports** | `enum_report` per contig/CDS/domain tables · `summary_report` · `matrix_report` score distribution and suggested cutoffs · `ssn_navigator` query a network as JSON |
| **HMM profiles** | `hmmer_build` · `hmmer_select` · `hmmer_search` · `hmmer_compare` · `hmmer_report` |
| **Color** | `color_genbank` · `color_table_to_legend` |
| **I/O and database prep** | `format_db` shard/compress/index for fast searching · `genbank_to_fasta` · `convert` · `inspect` · `schema` |

Every tool documented with examples in the
**[command-line reference](docs/cli-reference.md)**; complete pipelines in
**[example workflows](docs/examples.md)**.

## The desktop app

Point it at a folder and it shows you what's in it — no import step, so files you make on
the command line just appear, and vice versa.

- **Contig viewer** — scroll and zoom gene maps, with circular origin-wrapping drawn
  correctly, e-value and database filters, sequence selection and copy, and per-feature
  editing and recoloring.
- **SSN viewer** — the interactive network: move the threshold to split clusters, switch
  layouts, chart and edit metadata, and jump from selected nodes back to their sequences.
- **Enum Report, Summary, Matrix Report, HMM Profiles and Logo** tabs for the other file
  types, plus table, text and SVG viewers.
- **Tools menu** — 29 of the tools, with dialogs generated from the same JSON Schema the
  CLI publishes, so the two cannot drift. Outputs land next to your inputs and chain by
  file type.
- **Jobs, Databases and External tools panels** — run history with provenance, a reusable
  registry of reference databases, and one-click download of `diamond`, `cdhit`,
  `usearch` and `orphos`.

See **[the desktop app](docs/gui.md)**, including its current limitations.

## Documentation

- [Installation](docs/installation.md)
- [Getting started](docs/getting-started.md)
- [Concepts](docs/concepts.md) — how the tools compose, and the `-i`/`-r` convention
- [Command-line reference](docs/cli-reference.md)
- [Example workflows](docs/examples.md)
- [The desktop app](docs/gui.md)
- [File formats](docs/file-formats.md)
- [Config files and agent output](docs/config-and-agents.md) — `--config`, `schema`, JSON
- [Troubleshooting](docs/troubleshooting.md)

Developers: [Developing Domainator](docs/development.md) ·
[Architecture](docs/architecture.md) · [Porting status](docs/porting-status.md) ·
[The parity harness](docs/parity.md) · [Upstream sync](UPSTREAM.md)

The `docs/` directory is an [mdbook](https://rust-lang.github.io/mdBook/):
`mdbook serve` renders it locally.

## Interoperability with the Python suite

Maintaining on-disk compatibility with the original tool is a hard requirement, in both
directions — GenBank files, HDF5 matrices, `.dsnv` network bundles, HMM profiles, and
formatted databases (shards, BGZF, `.didx` indexes). You can hand a file to either
implementation mid-workflow.

The guarantee is **semantic, not byte-for-byte**: a file round-trips to an equal record
under the Python suite's own comparators, though bytes may differ where nothing is meant
by them. In practice most output is byte-identical anyway.

Fidelity is enforced by a differential parity harness that replays every invocation in
upstream's test suite through both implementations and diffs the results — see
[the parity harness](docs/parity.md) — and per-tool coverage, including what is
deliberately not ported, is tracked in [Porting status](docs/porting-status.md).

Not ported: anything depending on torch or Infernal. Gene calling runs Prodigal's
algorithm through the external [Orphos](https://github.com/FullHuman/orphos) binary. The
`structure_*` subsystem and HTML report output are out of scope for now; report tools offer
`--json` instead.

## Credits

A Rust port of [Domainator](https://github.com/nebiolabs/domainator) by New England
Biolabs — please cite the original if you use this.

Released under the [MIT License](LICENSE). The optional external programs `cd-hit`,
`usearch` and `orphos` are GPL licensed and are neither bundled nor linked — Domainator downloads them
only at your request and runs them as separate processes.

Questions and bug reports: <https://github.com/nebiolabs/domainatoredux/issues>.
