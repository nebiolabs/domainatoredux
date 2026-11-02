//! TMbed's predictions for a protein FASTA, as JSON (the Rust twin of
//! `scripts/tmbed/tmbed_probe.py`; compare them with `scripts/tmbed/compare_probes.py`).
//!
//! `cargo run -p domainator-plm --release --example tmbed_probe -- MODEL_DIR FASTA [BATCH_SIZE [DEVICE]]`

use std::path::Path;

use domainator_plm::tmbed::{Tmbed, LABEL_LETTERS};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (model_dir, fasta) = (&args[1], &args[2]);
    let batch_size: i64 = args
        .get(3)
        .map_or(4000, |s| s.parse().expect("a batch size"));
    let (device, warning) = domainator_plm::select_device(args.get(4).map(String::as_str));
    if let Some(w) = warning {
        eprintln!("{w}");
    }
    let text = std::fs::read_to_string(fasta).expect("a readable FASTA");
    let mut records: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        if let Some(header) = line.strip_prefix('>') {
            let id = header.split_whitespace().next().unwrap_or_default();
            records.push((id.to_string(), String::new()));
        } else if let Some((_, seq)) = records.last_mut() {
            seq.extend(line.split_whitespace());
        }
    }
    let start = std::time::Instant::now();
    let tmbed = Tmbed::load(Path::new(model_dir), &device).expect("ProtT5 loads");
    eprintln!("loaded in {:.1?}", start.elapsed());
    let start = std::time::Instant::now();
    let sequences: Vec<&str> = records.iter().map(|(_, s)| s.as_str()).collect();
    let predictions = tmbed
        .predict(&sequences, batch_size)
        .expect("prediction runs");
    eprintln!("predicted in {:.1?}", start.elapsed());
    let out: Vec<String> = records
        .iter()
        .zip(&predictions)
        .map(|((id, _), p)| {
            let labels: String = p
                .labels
                .iter()
                .map(|&l| LABEL_LETTERS[l as usize])
                .collect();
            let probabilities: Vec<String> = p
                .probabilities
                .iter()
                .map(|r| format!("[{}]", r.map(|v| v.to_string()).join(", ")))
                .collect();
            format!(
                "{{\"id\": \"{id}\", \"labels\": \"{labels}\", \"probabilities\": [{}]}}",
                probabilities.join(", ")
            )
        })
        .collect();
    println!("[{}]", out.join(", "));
}
