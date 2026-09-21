# Domainator (Rust)

Domainator annotates sequences with protein domains, extracts the genome neighborhoods
around them, and turns collections of sequences into **sequence similarity networks
(SSNs)** you can explore interactively.

This is a from-scratch Rust implementation of the
[Python Domainator suite](https://github.com/nebiolabs/domainator). It ships as:

- **one command-line executable** — `domainator <tool> [options]`, 34 tools, no
  interpreter or conda environment required;
- **one desktop app** — a workspace browser with a contig viewer, an interactive SSN
  viewer, HMM logos and dialogs for running the tools;
- **files that interoperate** with the Python suite in both directions.

HMMER3 is built in as pure Rust, so a basic install needs nothing else at all.

## Install

Download from the [Releases page](https://github.com/nebiolabs/domainatoredux/releases):

| Platform | Desktop app | Command-line tool |
| --- | --- | --- |
| macOS (Apple silicon) | `.dmg` | `domainator` |
| Windows (x64) | NSIS installer | `domainator.exe` |
| Linux (x64) | `.deb`, `.AppImage` | `domainator` |

Or build it — you need [Rust](https://rustup.rs/) 1.89+ and nothing else:

```bash
cargo build --release -p domainator-cli
./target/release/domainator --help
```

Full details, including the three optional external programs, are in
**[Installation](docs/installation.md)**.

## Quickstart

Annotate 20 proteins, score them all against each other, and build an interactive
network — using two files from `test/data/`:

```bash
domainator domainate -i FeSOD_20.gb -r FeSOD_pfam.hmm -o annotated.gb
#> wrote 20 records to annotated.gb

domainator seq-dist -i annotated.gb --dense dist.hdf5 --algorithm hmmer --mode score
#> wrote 20x20 matrix

domainator build-ssn-viewer -i dist.hdf5 -o network.dsnv --html network.html --embed_data
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

`domainator --help` lists them all; `domainator <tool> --help` documents one. Flags use
the Python suite's underscore spellings (`--max_overlap`), while subcommands use hyphens.

| | |
| --- | --- |
| **Annotation and search** | `domainate` annotate with hits to references · `domain-search` keep the records or neighborhoods that hit · `filter-domains` prune weak annotations |
| **Selection and extraction** | `select-by-cds` · `select-by-contig` · `select-by-coord` · `extract-domains` · `extract-peptides` · `extract-unannotated` · `trim-contigs` · `sort-contigs` · `clean-sequences` · `deduplicate` |
| **Matrices and networks** | `seq-dist` similarity by alignment · `compare-contigs` similarity by shared domain content · `transform-matrix` · `build-ssn` (Cytoscape XGMML) · `build-ssn-viewer` (interactive) |
| **Reports** | `enum-report` per contig/CDS/domain tables · `summary-report` · `matrix-report` score distribution and suggested cutoffs · `ssn-navigator` query a network as JSON |
| **HMM profiles** | `hmmer-build` · `hmmer-select` · `hmmer-search` · `hmmer-compare` · `hmmer-report` |
| **Colour** | `color-genbank` · `color-table-to-legend` |
| **I/O and database prep** | `format-db` shard/compress/index for fast searching · `genbank-to-fasta` · `convert` · `inspect` · `schema` |

Every tool documented with examples in the
**[command-line reference](docs/cli-reference.md)**; complete pipelines in
**[example workflows](docs/examples.md)**.

## The desktop app

Point it at a folder and it shows you what's in it — no import step, so files you make on
the command line just appear, and vice versa.

- **Contig viewer** — scroll and zoom gene maps, with circular origin-wrapping drawn
  correctly, e-value and database filters, sequence selection and copy, and per-feature
  editing and recolouring.
- **SSN viewer** — the interactive network: move the threshold to split clusters, switch
  layouts, chart and edit metadata, and jump from selected nodes back to their sequences.
- **Enum Report, Summary, Matrix Report, HMM Profiles and Logo** tabs for the other file
  types, plus table, text and SVG viewers.
- **Tools menu** — 28 of the tools, with dialogs generated from the same JSON Schema the
  CLI publishes, so the two cannot drift. Outputs land next to your inputs and chain by
  file type.
- **Jobs, Databases and External tools panels** — run history with provenance, a reusable
  registry of reference databases, and one-click download of `diamond`, `cdhit` and
  `usearch`.

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

Not ported: anything depending on torch, Infernal, or gene calling with prodigal. The
`structure_*` subsystem and HTML report output are out of scope for now; report tools offer
`--json` instead.

## Credits

A Rust port of [Domainator](https://github.com/nebiolabs/domainator) by New England
Biolabs — please cite the original if you use this.

Released under the [MIT License](LICENSE). The optional external programs `cd-hit` and
`usearch` are GPL licensed and are neither bundled nor linked — Domainator downloads them
only at your request and runs them as separate processes.

Questions and bug reports: <https://github.com/nebiolabs/domainatoredux/issues>.
