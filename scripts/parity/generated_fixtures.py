#!/usr/bin/env python3
"""Build the fixtures that upstream's tests generate at test time.

Some upstream tests do not read their inputs from `test/data` -- they build them
first: a single-sequence HMM from a CDS's translation, a FASTA with a UniProt
` (Fragment)` token added to one header, a GenBank file annotated by `domainate`.
The harvester can resolve their `main([...])` argument lists, but the files those
arguments name never exist, so Python rejects the invocation and the sweep counts
it `skipped`.

This module builds those files, once per sweep, into a directory that
`differential.py` substitutes for the `@GEN@` placeholder. The invocations that
use them live in `extra_invocations.tsv`. Where upstream's `test/helpers.py` has a
builder, it is called rather than copied, so the fixture cannot drift from the
test it stands in for.

Usage (normally called by differential.py):
    generated_fixtures.py --data_dir test/data --test_dir external/domainator/test -o OUT
"""

import argparse
import os
import pathlib
import sys


def _single_sequence_hmm(sequence, path, name):
    import pyhmmer

    alphabet = pyhmmer.easel.Alphabet.amino()
    digital = pyhmmer.easel.TextSequence(name=name, sequence=sequence).digitize(alphabet)
    hmm, _, _ = pyhmmer.plan7.Builder(alphabet).build(digital, pyhmmer.plan7.Background(alphabet))
    with open(path, "wb") as handle:
        hmm.write(handle)


def generate(data_dir, test_dir, out_dir):
    """Write every generated fixture into `out_dir`."""
    sys.path.insert(0, str(test_dir))
    import helpers
    from Bio import SeqIO

    data = pathlib.Path(data_dir)
    out = pathlib.Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)

    # One HMM per CDS of the partial fixture, named by locus_tag, and the fixture
    # annotated with them (helpers.domainate_partial_fixture writes both).
    helpers.domainate_partial_fixture(data, str(out))

    # A single-sequence HMM from the 5'-partial CDS alone (test_domainate.py's
    # _write_partial_cds_hmm).
    record = next(SeqIO.parse(str(data / helpers.PARTIAL_FIXTURE), "genbank"))
    partial_cds = [f for f in record.features
                   if f.type == "CDS" and f.qualifiers.get("locus_tag") == [helpers.PARTIAL_CDS_LOCUS_TAG]][0]
    on_contig = partial_cds.qualifiers["translation"][0]
    _single_sequence_hmm(on_contig, str(out / "partial_cds.hmm"), b"partial_cds")

    # The partial fixture with residues off the edge of the contig prepended to the
    # 5'-partial CDS's translation (test_domainate_trims_overlong_partial_translation).
    partial_cds.qualifiers["translation"] = ["MKRFSLAIL" + on_contig]
    SeqIO.write([record], str(out / "overlong.gb"), "genbank")

    # The partial fixture with a legacy bare /partial on its complete CDS
    # (test_utils.py's test_bare_partial_qualifier_is_a_fragment).
    text = (data / helpers.PARTIAL_FIXTURE).read_text()
    text = text.replace("     CDS             complement(280..780)\n",
                        "     CDS             complement(280..780)\n                     /partial\n", 1)
    (out / "bare_partial.gb").write_text(text)

    # swissprot_CuSOD_subset.fasta with ' (Fragment)' on the first header, and an
    # HMM built from that first sequence.
    helpers.write_uniprot_fragment_fasta(data, str(out / "fragments.fasta"))
    first = next(SeqIO.parse(str(data / "swissprot_CuSOD_subset.fasta"), "fasta"))
    _single_sequence_hmm(str(first.seq), str(out / "sod.hmm"), b"sod")

    _rotate_contigs_fixtures(data, out)
    _compare_contigs_fixtures(out)
    _infernal_fixtures(data, out)
    _selection_fixtures(data, out)
    _split_by_cluster_fixtures(data, out)

    # test_find_features.py's test_motif_integration writes this protein to a
    # temporary file.
    (out / "motif_test.fasta").write_text(">test\nMKFLDDHSAXXPXXXXCILLFNILCLFPVLA\n")


def _compare_contigs_fixtures(out):
    """The inputs test_compare_contigs.py builds with its own record helper."""
    from domainator.seqio import write_genbank
    from test_compare_contigs import _make_compare_contig_record as contig

    # test_compare_contigs_name_by_order_reorders_clustered_records
    with open(out / "compare_reorder.gb", "w") as handle:
        write_genbank([contig("different", ["C"]), contig("similar_1", ["A", "B"]),
                       contig("similar_2", ["A", "B"])], handle)
    # test_compare_contigs_sorts_with_max_hits_per_query
    with open(out / "compare_max_hits.gb", "w") as handle:
        write_genbank([contig(f"rec{i}", doms) for i, doms in enumerate(
            [["A", "B"], ["C", "D"], ["A", "B", "E"], ["C", "D", "F"], ["A", "E"], ["C", "F"]])], handle)


def _infernal_fixtures(data, out):
    """Inputs for the Infernal invocations: test_infernal_build.py's alignments, and
    CopA (RF00042) sites cut or wrapped so cmsearch's truncated passes run."""
    from Bio import SeqIO
    from Bio.Seq import Seq
    from Bio.SeqRecord import SeqRecord
    from test_infernal_build import FASTA_CONTENT, STOCKHOLM_CONTENT

    (out / "infernal_build.sto").write_bytes(STOCKHOLM_CONTENT)
    (out / "infernal_build.afa").write_bytes(FASTA_CONTENT)

    record = next(SeqIO.parse(str(data / "pANT_R100.fa"), "fasta"))
    # test_domainator_infernal_skips_duplicate_hits: the site cut near one end, where
    # the truncated pass finds it twice.
    SeqIO.write([SeqRecord(record.seq[48590:48730], id="fragment", description="")],
                str(out / "copa_fragment.fa"), "fasta")
    # The site (48665..48755, minus strand) cut at either end, several ways.
    windows = [(48560, 48700), (48590, 48730), (48690, 48790), (48700, 48800), (48720, 48820)]
    SeqIO.write([SeqRecord(record.seq[a:b], id=f"copa_{a}_{b}", description="")
                 for a, b in windows], str(out / "copa_fragments.fa"), "fasta")
    # The whole plasmid as a circular contig whose origin falls inside the site.
    circular = SeqRecord(record.seq[48710:] + record.seq[:48710], id="pANT_rotated",
                         name="pANT_rotated", description="pANT_R100 rotated into CopA")
    circular.annotations["molecule_type"] = "DNA"
    circular.annotations["topology"] = "circular"
    SeqIO.write([circular], str(out / "copa_circular.gb"), "genbank")
    # One model twice: the second replaces the first, with a warning.
    model = (data / "RF00042.cm").read_text()
    (out / "copa_twice.cm").write_text(model + model)


def _split_by_cluster_fixtures(data, out):
    """The compressed inputs test_split_by_cluster.py writes with helpers.gzip_file and
    helpers.bgzip_file, and the two-model .cm file (with its matrix) of
    test_split_by_cluster_cm_keeps_each_filter_hmm_with_its_model."""
    import helpers
    from test_split_by_cluster import _write_test_matrix

    helpers.gzip_file(data / "pdonr_hmms.hmm", out / "clusters.hmm.gz")
    helpers.bgzip_file(data / "FeSOD_20.gb", out / "in.gb.bgz")
    copa = (data / "RF00042.cm").read_text()
    (out / "two.cm").write_text(copa + copa.replace("CopA", "CopB"))
    _write_test_matrix(out / "two_cm.matrix.tsv", ["CopA", "CopB"])


def _rotate_contigs_fixtures(data, out):
    """The inputs test_rotate_contigs.py builds at test time."""
    from Bio import SeqIO
    from Bio.Seq import Seq
    from Bio.SeqRecord import SeqRecord
    from domainator.domain_search import main as domain_search_main

    # test_rotate_search_hits: whole-contig domain_search hits to rotate on.
    domain_search_main(["-i", str(data / "bacillus_phage_SPR.gb"), "-r", str(data / "SPR.hmm"),
                        "--whole_contig", "-Z", "1000", "--cpu", "1", "-o", str(out / "spr_search_hits.gb")])
    # test_rotate_domains_file
    (out / "dhfr_domains.txt").write_text("DHFR_1\n")
    # test_rotate_linear_genbank_skipped: the annotated contig marked linear.
    record = next(SeqIO.parse(str(data / "bacillus_phage_SPR_with_annotations.gb"), "genbank"))
    record.annotations["topology"] = "linear"
    SeqIO.write([record], str(out / "spr_linear.gb"), "genbank")
    # test_rotate_fasta_gene_call: the contig as FASTA.
    record = next(SeqIO.parse(str(data / "bacillus_phage_SPR.gb"), "genbank"))
    (out / "spr.fasta").write_text(f">{record.id}\n{record.seq}\n")
    # test_rotate_nucleotide_query_spanning_origin: a circular contig and a nucleotide
    # query across its origin.
    sequence = "GCTAACCGTTAGCGATCGTACGATCGATGCTAGTCCGATTAACCGGTTAGGCTTACCGATGG"
    circular = SeqRecord(Seq(sequence), id="circular_test", name="circular_test", description="circular test")
    circular.annotations["molecule_type"] = "DNA"
    circular.annotations["topology"] = "circular"
    SeqIO.write([circular], str(out / "circular_test.gb"), "genbank")
    (out / "origin_query.fna").write_text(f">origin_query\n{sequence[-18:] + sequence[:18]}\n")


def _selection_fixtures(data, out):
    """Inputs for the shared selection contract (upstream 1.0's taxonomy filters,
    --dedup, --has_search_hit), whose tests call the tools as library functions or
    through `getattr`, so the harvester sees none of them."""
    from Bio.Seq import Seq
    from Bio import SeqIO
    from domainator.seqio import write_genbank
    import test_taxonomy_filters as tax

    # The builders leave `name` unset, which Biopython writes as an ACCESSION of
    # "<unknown name>"; name each record by its id, as a real file would.
    def named(record):
        record.name = record.id
        return record

    # The mixed-source contig: CDSs under E. coli (562), under B. subtilis (1423), and
    # one straddling the two sources (so no source covers it).
    with open(out / "mixed.gb", "w") as handle:
        write_genbank([named(tax.mixed())], handle)
    # Proteins under a known, a missing, a deleted and a merged taxid.
    proteins = [tax.protein(562, "coli"), tax.protein(None, "unlabeled"), tax.protein(1423, "subtilis"),
                tax.protein(1985417, "deleted"), tax.protein(17, "merged")]
    with open(out / "tax_proteins.gb", "w") as handle:
        write_genbank([named(p) for p in proteins], handle)
    # test_search_worker_unknown_policy: a protein with no taxid beside labeled ones.
    records = list(SeqIO.parse(str(data / "swissprot_CuSOD_subset.fasta"), "fasta"))
    lines = [f">{r.description}\n{r.seq}\n" for r in records]
    lines.append(f">unlabeled unlabeled\n{records[0].seq}\n")
    (out / "sod_unlabeled.fasta").write_text("".join(lines))
    # test_deduplicate_genbank_hash_both_strands: a sequence, its reverse complement,
    # its complement and its reverse.
    seq = "ACGTAAGT"
    (out / "strands.fasta").write_text(
        f">seq1\n{seq}\n>seq2\n{Seq(seq).reverse_complement()}\n"
        f">complement_only\n{Seq(seq).complement()}\n>reverse_only\n{seq[::-1]}\n")
    # A nucleotide target, an exact copy and its reverse complement, for --dedup.
    target = next(SeqIO.parse(str(data / "simple_dna_target.fna"), "fasta"))
    (out / "rc_pair.fna").write_text(
        f">{target.id}\n{target.seq}\n>target_copy\n{target.seq}\n"
        f">target_rc\n{target.seq.reverse_complement()}\n")


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--data_dir", required=True)
    ap.add_argument("--test_dir", required=True)
    ap.add_argument("-o", "--out_dir", required=True)
    cfg = ap.parse_args(argv)
    generate(cfg.data_dir, cfg.test_dir, cfg.out_dir)
    for name in sorted(os.listdir(cfg.out_dir)):
        print(name)


if __name__ == "__main__":
    main()
