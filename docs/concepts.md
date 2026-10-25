# Concepts

A handful of ideas explain why Domainator's tools are shaped the way they are. Reading
this first will save you time.

## GenBank carries both sequence and annotation

Domainator uses **GenBank as its working format**, not FASTA. A GenBank file holds the
sequence *and* every annotation made about it in one place: CDS locations, domain hits,
taxonomy, colors, descriptions. That means:

- a file is self-describing, and a pipeline of tools accumulates knowledge in it rather
  than scattering results across side files;
- you can stop at any step, open the file in the [contig viewer](gui.md) or Geneious, and
  see exactly what the previous tool did;
- you are not tied to a particular sequence source.

FASTA is accepted as input and offered as output (`--fasta_out` on many tools) but cannot
carry annotations.

## The `-i` / `-r` convention

This is the single most important convention, and it surprises people:

> **`-i` is the file being edited. `-r` is the criteria you are editing it by.**

So to annotate your contigs with Pfam, the contigs are `-i` and Pfam is `-r`:

```bash
domainatoredux domainate -i unannotated.gb -r pfam-A.hmm -o annotated.gb
```

And to search a big database for hits to your query, **the database is `-i`** and your
query is `-r`:

```bash
domainatoredux domain_search -i uniprot.fasta -r my_query.hmm -o hits.gb
```

That second one reads backwards if you are used to BLAST, where the query leads. Think of
it as "take this file, and keep/annotate the parts that match these references."

## What the tools do to a file

Tools fall into a few roles, and they compose:

**Editors** take a format and return the same format, so they chain arbitrarily. Each does
one simple thing. `domainate` adds annotations; `domain_search` keeps the subset of
records that hit a reference; `select_by_cds` extracts genome neighborhoods around
interesting genes; `filter_domains` strips weak annotations; `deduplicate_genbank` collapses
near-identical sequences. Most of a workflow is a chain of editors.

**Comparison tools** turn a set of sequences into a pairwise score matrix.
`seq_dist` scores by local alignment (DIAMOND or HMMER); `compare_contigs` scores by
shared domain content (Jaccard and adjacency indexes), which is what you want for
comparing gene neighborhoods rather than single proteins. `hmmer_compare` compares HMM
profiles to each other.

**Record-wise reports** write one row per contig, CDS or domain — `enum_report`,
`hmmer_report`. Use them to get data into Excel or R, and as intermediate files for other
Domainator tools that take a table.

**Summary reports** describe a whole dataset rather than its records —
`summary_report`, `matrix_report`.

**Network tools** turn a matrix into something you can look at: `build_ssn` writes
Cytoscape XGMML, `build_ssn_viewer` writes a self-contained interactive viewer, and
`ssn_navigator` queries a built network from a script.

The [command-line reference](cli-reference.md) is organized along these lines.

## Domain annotations: `Domainator` vs `Domain_Search`

Two different feature types appear in output files, and they mean different things:

- **`Domainator`** features are domain annotations — "this region matches this profile."
  `domainate` writes them. There can be many per CDS.
- **`Domain_Search`** features mark the *best* hit that caused a record to be selected.
  `domain_search` writes one per focus CDS.

The distinction matters for ranking. `domain_search --max_hits` ranks by the
`Domain_Search` score only, never by the `Domainator` annotations — otherwise an extracted
neighborhood would be scored by whatever strong gene happened to sit next to your weak
hit.

Several tools let you choose which to act on (`--annotation_type`, which is
`domainator`, `domain_search` or, on `select_by_contig`, `all`).

## The `domainator_<db>` rollup

Each CDS gets a summary qualifier per reference database it was annotated against, for
example:

```
/domainator_Pfam="Sod_Fe_N; Sod_Fe_C"
```

It is a convenience rollup of that CDS's `Domainator` features, regenerated every time a
file is written. You can read it in any GenBank viewer without parsing sub-features, and
tools like `select_by_contig --domains` match against it.

## Sequences, CDSs and domains are three levels

Most tools let you work at whichever level you mean:

- **contig / record** — a genome, a plasmid, a single protein;
- **CDS** — a gene within a contig (protein records have no CDS features, as you saw in
  [getting started](getting-started.md#3-tabulate-the-annotations));
- **domain** — a region within a CDS matching a profile.

`enum_report --by contig|cds|domain` is the clearest expression of this; `select_by_contig`
versus `select_by_cds` is the same distinction applied to selection.

## Protein and nucleotide references

Protein references are searched against CDSs (or against protein records directly).
Nucleotide references — FASTA or nucleotide HMMs — are searched against whole nucleotide
contigs, both strands, wrapping the origin on circular contigs. You can mix reference
types in one run: annotate contigs with Pfam profiles and a set of nucleotide motifs at
the same time by passing `-r` twice.

## Compatibility with the Python suite

Files written here are readable by the Python Domainator suite and vice versa. The
guarantee is **semantic, not byte-for-byte**: a file round-trips to an equal record, but
the bytes may differ (JSON key order, for instance). See
[File formats](file-formats.md) and [Porting status](porting-status.md).
