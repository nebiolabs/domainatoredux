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
