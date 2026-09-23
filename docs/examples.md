# Example workflows

Complete pipelines. If you haven't run anything yet, start with
[Getting started](getting-started.md).

The first three workflows run as written against files in the repository's `test/data/`
directory, and show their real output. The rest use placeholder filenames — the shape of
the pipeline is what matters, so substitute your own data.

To follow the first three, copy the fixtures into a scratch directory:

```bash
mkdir ~/domainator-demo && cd ~/domainator-demo
cp /path/to/domainatoredux/test/data/* .
```

## 1. Genome mining: find a gene, then look at its neighborhood

The question is rarely "where is this gene" but "what sits around it". Annotate the
contigs, extract a window of genes around every hit, then compare those windows by the
domains they contain.

```bash
# annotate a set of plasmid contigs
domainator domainate -i pDONR201_multi_genemark.gb -r pdonr_hmms.hmm -o annotated.gb
#> wrote 4 records to annotated.gb

# what got annotated, gene by gene?
domainator enum-report -i annotated.gb --by cds --domains --length -o genes.tsv
```

```
contig        cds            domains   length
pDONR201_1    pDONR201_1               105
pDONR201_1    pDONR201_2     CcdB      306
pDONR201_1    pDONR201_3     CcdA      126
```

```bash
# pull one gene either side of every CcdB hit
domainator select-by-cds -i annotated.gb -o neighborhoods.gb \
  --domains CcdB --cds_up 1 --cds_down 1

# compare those neighborhoods by shared domain content, not by sequence
domainator compare-contigs -i neighborhoods.gb --dense similarity.hdf5 --ji 0.5 --ai 0.5
#> wrote 4x4 matrix

# cluster them and write a Cytoscape network
domainator build-ssn -i similarity.hdf5 --xgmml network.xgmml --lb 0 \
  --cluster --cluster_tsv clusters.tsv
#> wrote 4 nodes, 6 edges to network.xgmml
```

The extracted records are named for the region they came from:

```
LOCUS       pDONR201_1_1391:2rc     1390 bp    DNA
```

`_1391:2` is the coordinate window and `rc` marks a reverse-complemented,
origin-crossing slice — these are circular plasmids, so a window around a gene near
position 1 wraps the origin. See [Concepts](concepts.md).

Use [`compare-contigs`](cli-reference.md#compare-contigs) rather than
[`seq-dist`](cli-reference.md#seq-dist) whenever the unit of comparison is a
neighborhood: two operons with the same genes in the same order should score as similar
even if the sequences have diverged.

## 2. All-vs-all protein similarity network

The classic SSN. Score every protein against every other, inspect the score
distribution, then build a viewer.

```bash
domainator domainate -i FeSOD_20.gb -r FeSOD_pfam.hmm -o annotated.gb

domainator seq-dist -i annotated.gb --dense dist.hdf5 --algorithm hmmer --mode score
#> wrote 20x20 matrix

# look before you choose a cutoff
domainator matrix-report -i dist.hdf5 -o report.txt

domainator build-ssn-viewer -i dist.hdf5 -o network.dsnv \
  --html network.html --embed_data
```

On a large dataset, swap in DIAMOND and normalize the scores:

```bash
domainator seq-dist -i proteins.fasta --dense raw.hdf5 --algorithm diamond_us --cpu 0
domainator transform-matrix -i raw.hdf5 --dense efi.hdf5 --mode efi_score
domainator build-ssn-viewer -i efi.hdf5 -o network.dsnv --html network.html --embed_data
```

`--mode efi_score` applies the EFI-style normalization, which makes a cutoff comparable
across protein families of different lengths. If the network is too dense to lay out,
prune it while keeping it connected:

```bash
domainator transform-matrix -i efi.hdf5 --dense pruned.hdf5 --mst_knn 5
```

## 3. A similarity network of HMM profiles

Profiles are first-class data here, so you can build an SSN *of profiles* to see how a
domain family is organized. Two HMM inputs select the profile-vs-profile engine
automatically:

```bash
domainator seq-dist -i pdonr_hmms.hmm --dense hmm_dist.hdf5 --mode score
#> Warning: input and reference are both HMM files; using --algorithm hmmer_compare.
#> wrote 7x7 matrix

domainator build-ssn-viewer -i hmm_dist.hdf5 -o profiles.dsnv \
  --html profiles.html --embed_data
```

For the pairwise alignments themselves rather than a matrix, use
[`hmmer-compare`](cli-reference.md#hmmer-compare):

```bash
domainator hmmer-compare -i pdonr_hmms.hmm -r pdonr_hmms.hmm -o scores.tsv --alignments
```

## 4. Curate a reference HMM set

Pfam is large; most analyses want a slice of it, plus your own profiles.

```bash
# take the profiles whose description mentions nucleases
domainator hmmer-select -i pfam-A.hmm -o nucleases.hmm --contains nuclease --field desc

# build your own profile from an alignment and add it
domainator hmmer-build -i my_alignment.sto -o mine.hmm --name MyDomain --desc "My domain"

# see what you ended up with
domainator hmmer-report -i nucleases.hmm -i mine.hmm --acc --desc --length -o profiles.tsv

# keep only candidates related to something known
domainator hmmer-search -i candidates.hmm -r nucleases.hmm -o related.hmm --score_cutoff 20
```

`hmmer-select` reproduces its input byte for byte for the profiles it keeps, including the
Pfam `GA`/`TC`/`NC` cutoffs, so `--cut_ga` still works downstream.

## 5. Search a large database

Format the database once, then search it repeatedly. `format-db` shards it,
BGZF-compresses it so it stays seekable, and builds a `.didx` index:

```bash
domainator format-db -i uniprot.fasta --output_dir uniprot_db \
  --shards 8 --compress --index

domainator domain-search -i uniprot_db/ -r my_query.hmm -o hits.gb \
  -e 1e-20 --max_hits 1000
```

Remember the [`-i`/`-r` convention](concepts.md#the--i---r-convention): the *database* is
`-i`, because it is the file being filtered; your query is the criteria.

Then collapse near-identical hits before building a network:

```bash
domainator deduplicate -i hits.gb -o representatives.gb --id 0.9 \
  --cluster_table clusters.tsv
```

`deduplicate` defaults to the cd-hit backend, which needs an external binary. For exact
deduplication with nothing installed, use `--algorithm hash --id 1`.

## 6. Reporting for a paper or a spreadsheet

```bash
# per-domain table with chemistry and taxonomy
domainator enum-report -i annotated.gb --by domain \
  --domains --start --end --score --length \
  --molecular_weight --isoelectric_point --taxname genus species \
  -o domains.tsv

# dataset-level summary
domainator summary-report -i annotated.gb -o summary.txt

# color the features, and a matching legend for the figure
domainator color-genbank -i annotated.gb -o colored.gb \
  --mode domains_and_cdss --color_table_out colors.tsv
domainator color-table-to-legend -i colors.tsv --svg legend.svg --title Domains
```

`colored.gb` opens with its colors intact in the [desktop app's contig
viewer](gui.md#viewers), in Geneious, or in any GenBank viewer that honors the `Color`
qualifier.

## 7. Driving it from a script or an agent

Every report tool can emit JSON, and `ssn-navigator` answers questions about a built
network without a GUI:

```bash
domainator summary-report -i annotated.gb --json - | jq .domain_counts
domainator enum-report -i annotated.gb --by cds --domains --json - | head -1
domainator ssn-navigator -i network.dsnv --mode overview
domainator ssn-navigator -i network.dsnv --mode clusters --threshold 120 --min_size 3
```

See [Config files and agent output](config-and-agents.md) for the exact contracts and for
running tools from a config file instead of a long command line.
