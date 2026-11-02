#!/usr/bin/env python3
"""Write domainator-plm's TMbed assets: the CNN weights, and goldens for its tests.

TMbed's five CNN models (`tmbed/models/cnn/cv_*.pt`, Apache-2.0, at the commit upstream
domainator pins) are converted to one safetensors file, each model's state dict under
`cv_<n>.`, which the crate embeds. The goldens are TMbed's own CNN ensemble and Viterbi
decoder run on embeddings both sides can build exactly (a small integer formula), so
`cargo test` checks the Rust CNN and decoder against torch without the 2.4 GB encoder.

Run with a Python that has TMbed installed (its pinned commit) and torch:

    python scripts/tmbed/make_assets.py           # rewrite the assets
    python scripts/tmbed/make_assets.py --check   # exit 1 if they are out of date
"""
import argparse
import hashlib
import json
import pathlib
import sys
import tempfile

import torch
from safetensors.torch import save_file

import tmbed
from tmbed.model import Predictor
from tmbed.tmbed import predict_sequences
from tmbed.utils import make_mask
from tmbed.viterbi import Decoder

REPO = pathlib.Path(__file__).resolve().parents[2]
ASSETS = REPO / "crates" / "domainator-plm" / "assets" / "tmbed"
GOLDENS = REPO / "crates" / "domainator-plm" / "tests" / "data" / "tmbed_goldens.json"

# The models' sha256 at TMbed 8cee893523eb655bc9485c00c65336d27a236191.
CNN_SHA256 = {
    "cv_0.pt": "0d9f0737fb877f3eee1544372a22ec4c97ed0394c03d7ef2560c2bc50c84c41f",
    "cv_1.pt": "4d260420d75c995f525e821b2c62520e2ac9242071b85798029fb7f71d153d9b",
    "cv_2.pt": "0f31dc2000e29afe9c0488f0b1f208a3a676ca4a40ac4fafee6a8b9fdd9b44e7",
    "cv_3.pt": "fc54fcbda70cb24af52f8395303c02593663d9b7781e128af3e24d0b15f93650",
    "cv_4.pt": "e24fb30841dfeeb4e5ce54ea3f6c19384f4cc688a8b1e7b2dcccf2a56788d87a",
}

# Lengths of the golden "proteins": short, around the kernel widths, and long enough for
# the decoder to find segments. Batched together, padded to the longest, as TMbed runs.
GOLDEN_LENGTHS = [1, 4, 9, 23, 61, 150]


def golden_embedding(length, scale):
    """Row n, column c: ((n * 31 + c * 17 + 7 * scale) % 97) / 48.5 - 1, exact in f32."""
    n = torch.arange(length).view(-1, 1)
    c = torch.arange(1024).view(1, -1)
    return (((n * 31 + c * 17 + 7 * scale) % 97).float() / 48.5 - 1.0)


def load_models():
    root = pathlib.Path(tmbed.__file__).parent / "models" / "cnn"
    states = {}
    for name, sha in CNN_SHA256.items():
        data = (root / name).read_bytes()
        got = hashlib.sha256(data).hexdigest()
        if got != sha:
            sys.exit(f"{name}: sha256 {got}, expected {sha} -- is TMbed at the pinned commit?")
        states[name] = torch.load(root / name, map_location="cpu")["model"]
    return states


def write_weights(states, path):
    tensors = {}
    for i, name in enumerate(sorted(states)):
        for key, value in states[name].items():
            tensors[f"cv_{i}.{key}"] = value.contiguous()
    # One metadata key: safetensors writes a metadata mapping in no fixed order.
    save_file(tensors, str(path), metadata={
        "source": "TMbed 8cee893523eb655bc9485c00c65336d27a236191 tmbed/models/cnn/cv_*.pt "
                  "(Apache-2.0)",
    })


def goldens(states):
    models = []
    for name in sorted(states):
        model = Predictor()
        model.load_state_dict(states[name])
        models.append(model.eval().requires_grad_(False))
    embeddings = [golden_embedding(n, i) for i, n in enumerate(GOLDEN_LENGTHS)]
    padded = torch.nn.utils.rnn.pad_sequence(embeddings, batch_first=True)
    # TMbed's embeddings carry one more position (the </s> token), masked out.
    padded = torch.cat([padded, torch.zeros(len(embeddings), 1, 1024)], dim=1)
    mask = make_mask(padded, GOLDEN_LENGTHS)
    with torch.no_grad():
        probabilities = predict_sequences(models, padded, mask)
        labels = Decoder()(probabilities, mask).byte()
    probabilities = probabilities.permute(0, 2, 1)
    return {
        "formula": "((n * 31 + c * 17 + 7 * index) % 97) / 48.5 - 1",
        "viterbi": viterbi_goldens(),
        "proteins": [
            {
                "length": n,
                "probabilities": probabilities[i, :n].tolist(),
                "labels": labels[i, :n].tolist(),
            }
            for i, n in enumerate(GOLDEN_LENGTHS)
        ],
    }


# Class blocks (B, H, S, i, o as 0-4) and lengths: the decoder's segments, under noise.
VITERBI_LAYOUTS = [
    [(3, 30), (1, 22), (4, 15), (1, 20), (3, 10), (0, 9), (4, 20)],
    [(2, 18), (4, 40), (1, 3), (4, 12), (0, 11), (3, 25)],
    [(4, 6), (1, 25), (3, 4), (1, 21), (4, 30)],
]


def viterbi_goldens():
    """TMbed's decoder on class probabilities with segments and noise in them."""
    generator = torch.Generator().manual_seed(101)
    cases = []
    for layout in VITERBI_LAYOUTS:
        classes = torch.cat([torch.full((n,), c) for c, n in layout])
        logits = torch.randn(len(classes), 5, generator=generator) * 1.5
        logits[torch.arange(len(classes)), classes] += 2.0
        probabilities = torch.softmax(logits, dim=1)
        mask = torch.ones(1, len(classes))
        labels = Decoder()(probabilities.T.unsqueeze(0), mask).byte()[0]
        cases.append({"probabilities": probabilities.tolist(), "labels": labels.tolist()})
    return cases


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--check", action="store_true")
    args = ap.parse_args(argv)
    torch.use_deterministic_algorithms(True)
    states = load_models()
    weights = ASSETS / "cnn.safetensors"
    golden_text = json.dumps(goldens(states)) + "\n"
    if args.check:
        with tempfile.TemporaryDirectory() as tmp:
            fresh = pathlib.Path(tmp) / "cnn.safetensors"
            write_weights(states, fresh)
            stale = [p for p, ok in ((weights, fresh.read_bytes() == weights.read_bytes()),
                                     (GOLDENS, GOLDENS.read_text() == golden_text)) if not ok]
        if stale:
            sys.exit("out of date: " + ", ".join(str(p.relative_to(REPO)) for p in stale))
        print("TMbed assets are up to date")
        return 0
    ASSETS.mkdir(parents=True, exist_ok=True)
    GOLDENS.parent.mkdir(parents=True, exist_ok=True)
    write_weights(states, weights)
    GOLDENS.write_text(golden_text)
    print(f"wrote {weights.relative_to(REPO)} and {GOLDENS.relative_to(REPO)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
