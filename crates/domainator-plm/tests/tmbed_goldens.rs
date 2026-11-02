//! The CNN ensemble and Viterbi decoder against TMbed's own, run by
//! `scripts/tmbed/make_assets.py` on inputs both sides can build exactly.

use domainator_plm::tmbed::{cnn, viterbi};
use serde_json::Value;

fn goldens() -> Value {
    let text = include_str!("data/tmbed_goldens.json");
    serde_json::from_str(text).expect("goldens are JSON")
}

fn probabilities(value: &Value) -> Vec<[f32; 5]> {
    value
        .as_array()
        .expect("a list of residues")
        .iter()
        .map(|row| std::array::from_fn(|c| row[c].as_f64().expect("a number") as f32))
        .collect()
}

fn labels(value: &Value) -> Vec<u8> {
    value
        .as_array()
        .expect("a list of labels")
        .iter()
        .map(|l| l.as_u64().expect("a label") as u8)
        .collect()
}

#[test]
fn viterbi_decodes_as_tmbed_does() {
    let goldens = goldens();
    for case in goldens["viterbi"].as_array().unwrap() {
        let p = probabilities(&case["probabilities"]);
        assert_eq!(viterbi::decode(&p), labels(&case["labels"]));
    }
}

#[test]
fn the_cnn_ensemble_matches_tmbed() {
    let goldens = goldens();
    let ensemble = cnn::Ensemble::load().unwrap();
    for (index, protein) in goldens["proteins"].as_array().unwrap().iter().enumerate() {
        let len = protein["length"].as_u64().unwrap() as usize;
        // The goldens' embedding formula, in f32 as torch computes it.
        let embeddings: Vec<f32> = (0..len)
            .flat_map(|n| {
                (0..cnn::EMBEDDING_DIM)
                    .map(move |c| ((n * 31 + c * 17 + 7 * index) % 97) as f32 / 48.5 - 1.0)
            })
            .collect();
        let got = ensemble.predict(&embeddings);
        let want = probabilities(&protein["probabilities"]);
        assert_eq!(got.len(), want.len());
        for (n, (g, w)) in got.iter().zip(&want).enumerate() {
            for c in 0..5 {
                assert!(
                    (g[c] - w[c]).abs() < 1e-5,
                    "protein {index}, residue {n}, class {c}: {} vs {}",
                    g[c],
                    w[c]
                );
            }
        }
        assert_eq!(
            viterbi::decode(&got),
            labels(&protein["labels"]),
            "protein {index}"
        );
    }
}
