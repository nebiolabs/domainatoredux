# Domainator (Rust)

Domainator annotates sequences with protein domains, pulls out the genome neighborhoods
around them, and turns collections of sequences into **sequence similarity networks
(SSNs)** you can explore interactively.

This is a from-scratch Rust implementation of the
[Python Domainator suite](https://github.com/nebiolabs/domainator). It ships as:

- **one command-line executable** — `domainatoredux <tool> [options]`, 36 tools;
- **one desktop app** — a Geneious-inspired workspace browser with a contig viewer, an
  interactive SSN viewer, HMM logos, and dialogs for running the tools.

It reads and writes the Python suite's files, so you can move between the two
implementations mid-workflow.

## Where to start

| If you want to… | Read |
| --- | --- |
| install it | [Installation](installation.md) |
| run your first analysis | [Getting started](getting-started.md) |
| understand how the tools fit together | [Concepts](concepts.md) |
| look up a tool or a flag | [Command-line reference](cli-reference.md) |
| copy a complete pipeline | [Example workflows](examples.md) |
| use the desktop app | [The desktop app](gui.md) |
| know what a file contains | [File formats](file-formats.md) |
| drive it from a script or an agent | [Config files and agent output](config-and-agents.md) |
| fix an error message | [Troubleshooting](troubleshooting.md) |

Developers: see [Developing Domainator](development.md), [Architecture](architecture.md),
[Porting status](porting-status.md) and [the parity harness](parity.md).
