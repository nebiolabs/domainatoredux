#!/usr/bin/env python3
"""TMbed's predictions for a protein FASTA, as JSON: the Python twin of
`cargo run -p domainator-plm --release --example tmbed_probe -- MODEL_DIR FASTA`.

Both print `[{"id", "labels", "probabilities"}]` in input order, labels as TMbed's
letters (B b H h S i o). This one runs upstream domainator's own `run_tmbed_batch` on the
CPU, so the two can be diffed with `compare_probes.py`.

    PYTHONPATH=/path/to/domainator/src python scripts/tmbed/tmbed_probe.py MODEL_DIR FASTA [BATCH_SIZE]
"""
import json
import sys

from domainator.find_features import get_tmbed_predictor, run_tmbed_batch
from tmbed.utils import get_md5, read_fasta


def main(model_dir, fasta, batch_size=4000):
    proteins = read_fasta(fasta)
    encoder, models, decoder, device, _ = get_tmbed_predictor(None, 0, model_dir)
    predictions = run_tmbed_batch([(p.seq_hash, p.sequence) for p in proteins],
                                  encoder, models, decoder, device, batch_size=int(batch_size))
    out = []
    for p in proteins:
        labels, probabilities = predictions[get_md5(p.sequence)]
        out.append({"id": p.header[1:].split()[0],
                    "labels": "".join("BbHhSio"[i] for i in labels.tolist()),
                    "probabilities": probabilities.tolist()})
    json.dump(out, sys.stdout)
    print()


if __name__ == "__main__":
    main(*sys.argv[1:])
