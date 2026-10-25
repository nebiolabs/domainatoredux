# Getting started

This walks through a complete analysis: annotate 20 superoxide dismutase proteins with
HMM profiles, tabulate what was found, score every sequence against every other, and
build an interactive similarity network.

Every command below is real and runnable. It uses two files from the repository's
`test/data/` directory, so clone the repo (or copy those two files out of it) and work in
a scratch directory:

```bash
mkdir ~/domainator-demo && cd ~/domainator-demo
cp /path/to/domainatoredux/test/data/FeSOD_20.gb .
cp /path/to/domainatoredux/test/data/FeSOD_pfam.hmm .
```

> `test/data/` is a machine-maintained fixture set (see [the parity
> harness](parity.md)). Copy files out of it; don't edit them in place.

## 1. Look at what you have

```bash
domainatoredux inspect FeSOD_20.gb
```

```
20 records
```

`inspect` is the quick "does this parse, and what's in it" tool. Add `--verbose` for a
per-record summary.

## 2. Annotate the sequences

`domainate` adds domain annotations to sequences. The file being annotated is `-i`, and
the profiles to annotate it *with* are `-r`:

```bash
domainatoredux domainate -i FeSOD_20.gb -r FeSOD_pfam.hmm -o annotated.gb
```

```
wrote 20 records to annotated.gb
```

`annotated.gb` is the same 20 records, now carrying `Domainator` features where a profile
hit. The input is untouched — see [Concepts](concepts.md#the--i---r-convention) for why
the tools are shaped this way.

## 3. Tabulate the annotations

`enum_report` writes a table, one row per contig, CDS or domain. **The order of
`--columns` is the order of the output columns:**

```bash
domainatoredux enum_report -i annotated.gb --columns domains length -o report.tsv
```

```
contig                                          domains              length
FeSOD_A0A1F4ZT98|unreviewed|Superoxide          Sod_Fe_C; Sod_Fe_N   196
FeSOD_A0A067LT26|unreviewed|Superoxide          Sod_Fe_C; Sod_Fe_N   208
```

Each sequence has an N-terminal and a C-terminal iron/manganese SOD domain. There are
~20 more columns available (taxonomy, molecular weight, isoelectric point, PROSITE motif
counts, architecture); see [the reference](cli-reference.md#enum_report).

For a dataset-level overview rather than a per-record table:

```bash
domainatoredux summary_report -i annotated.gb -o summary.txt
```

```
Contig Stats

contigs: 20
CDSs: 0
CDSs per 10 kb: 0.00
```

`CDSs: 0` is correct here — these records are bare proteins, with no CDS features. Run
this on annotated genome contigs and you get CDS and domain frequency tables too.

## 4. Score everything against everything

`seq_dist` builds a similarity matrix. Leaving `-r` off means all-vs-all:

```bash
domainatoredux seq_dist -i annotated.gb --dense dist.hdf5 --algorithm hmmer --mode score
```

```
wrote 20x20 matrix
```

`--algorithm hmmer` uses the built-in HMMER3 implementation and needs nothing installed.
The default algorithm is `diamond_us`, which is much faster on large datasets but
requires the external `diamond` binary — see
[Installation](installation.md#optional-external-programs).

Check the score distribution before you pick a network cutoff:

```bash
domainatoredux matrix_report -i dist.hdf5 -o matrix.txt
```

```
Matrix Report
Nodes: 20
Non-zero edges: 190
Mean: 164.2
Median: 152.0
Min: 62.5
Max: 309.2

Edge counts by score:
     62.48 | ### 2
     74.81 | ### 2
     87.15 | ###### 4
     99.48 | ######################### 15
```

The report also suggests clustering thresholds — the score values where the network
breaks into meaningfully different groups.

## 5. Build the network

```bash
domainatoredux build_ssn_viewer -i dist.hdf5 -o network.dsnv --html network.html --embed_data
```

```
wrote 20-node .dsnv bundle to network.dsnv
wrote HTML viewer to network.html
```

You now have two things:

- **`network.dsnv`** — a compact bundle. Open it in the desktop app's SSN viewer tab, or
  query it from a script with [`ssn_navigator`](cli-reference.md#ssn_navigator).
- **`network.html`** — a standalone page with the data embedded (`--embed_data`). Open it
  in any browser; drag the threshold slider to split the network into clusters. Nothing
  else is required to view or share it.

If you'd rather work in Cytoscape, use [`build_ssn`](cli-reference.md#build_ssn) to write
XGMML instead.

## Where to go next

- [Concepts](concepts.md) — the `-i`/`-r` convention and how the tool categories compose.
- [Example workflows](examples.md) — genome mining around a domain of interest, HMM
  profile comparison, and more.
- [The desktop app](gui.md) — the same pipeline with dialogs, plus viewers for every file
  produced above.
