//! TMbed (<https://github.com/BernhoferM/TMbed>, Apache-2.0): transmembrane beta strands
//! and alpha helices, and signal peptides, predicted per residue from ProtT5 embeddings.
//!
//! The pipeline is TMbed's (`tmbed.py`, at the commit upstream domainator pins): proteins
//! sorted by length are cut into batches ([`make_batches`]), each batch is embedded by
//! ProtT5 ([`crate::t5`]), five CNNs turn each protein's embeddings into class
//! probabilities ([`cnn`]), and a Viterbi decoder picks a consistent labeling
//! ([`viterbi`]). Precision follows TMbed: f32 on the CPU, f16 on a GPU.

pub mod cnn;
pub mod viterbi;

use std::path::{Path, PathBuf};

use candle_core::{DType, Device, Result, Tensor};
use candle_nn::VarBuilder;

use crate::t5::{T5Config, T5Encoder};

/// The ProtT5 model TMbed embeds with.
pub const PROT_T5_MODEL: &str = "Rostlab/prot_t5_xl_half_uniref50-enc";

/// What the decoder calls each label (0..7): TMbed's `pred_map` letters.
pub const LABEL_LETTERS: [char; 7] = ['B', 'b', 'H', 'h', 'S', 'i', 'o'];

/// One protein's prediction.
#[derive(Clone, Debug, PartialEq)]
pub struct Prediction {
    /// Per residue: 0 B, 1 b, 2 H, 3 h, 4 S, 5 i, 6 o ([`LABEL_LETTERS`]).
    pub labels: Vec<u8>,
    /// Per residue, the class probabilities in TMbed's order: B, H, S, i, o.
    pub probabilities: Vec<[f32; cnn::CLASSES]>,
}

/// ProtT5's token ids for a protein, as TMbed's tokenizer call makes them: uppercase,
/// B/J/O/U/Z read as X, one SentencePiece piece per residue (anything outside the
/// vocabulary is `<unk>`), then `</s>`.
pub fn tokenize(sequence: &str) -> Vec<u32> {
    const PIECES: &[u8] = b"ALGVSREDTIPKFQNYMHWCXBOUZ";
    const UNK: u32 = 2;
    const EOS: u32 = 1;
    let mut ids: Vec<u32> = sequence
        .chars()
        .map(|c| {
            let c = match c.to_ascii_uppercase() {
                'B' | 'J' | 'O' | 'U' | 'Z' => 'X',
                c => c,
            };
            u8::try_from(c)
                .ok()
                .and_then(|b| PIECES.iter().position(|&p| p == b))
                .map_or(UNK, |i| i as u32 + 3)
        })
        .collect();
    ids.push(EOS);
    ids
}

/// TMbed's `make_batches`: `[start, end)` ranges over proteins sorted by length, each
/// batch as long as its size (protein count times the longest length to the power 1.5)
/// stays within `batch_size` to the power 1.5.
pub fn make_batches(sorted_lengths: &[usize], batch_size: i64) -> Vec<(usize, usize)> {
    if sorted_lengths.is_empty() {
        return Vec::new();
    }
    let limit = (batch_size.unsigned_abs() as f64).powf(1.5);
    let mut batches = Vec::new();
    let mut num_prot = 1usize;
    let mut last = 0usize;
    let mut max_size = 0f64;
    for (idx, &length) in sorted_lengths.iter().enumerate() {
        let size = (length as f64).powf(1.5);
        max_size = max_size.max(size);
        if idx > 0 && num_prot as f64 * max_size > limit {
            batches.push((last, idx));
            num_prot = 1;
            last = idx;
            max_size = size;
        }
        num_prot += 1;
    }
    batches.push((last, sorted_lengths.len()));
    batches
}

/// The weights file in a ProtT5 model directory: what `save_pretrained` writes
/// (`model.safetensors`, or `pytorch_model.bin` from an older transformers).
pub fn model_weights(dir: &Path) -> Option<PathBuf> {
    ["model.safetensors", "pytorch_model.bin"]
        .iter()
        .map(|name| dir.join(name))
        .find(|p| p.is_file())
}

/// TMbed, loaded: the ProtT5 encoder and the CNN ensemble.
pub struct Tmbed {
    encoder: T5Encoder,
    ensemble: cnn::Ensemble,
    device: Device,
}

impl Tmbed {
    /// Load ProtT5 from `model_dir` (a `save_pretrained` directory, as TMbed's
    /// `--model-dir` holds it) onto `device`.
    pub fn load(model_dir: &Path, device: &Device) -> Result<Self> {
        let weights = model_weights(model_dir).ok_or_else(|| {
            candle_core::Error::Msg(format!(
                "no ProtT5 weights (model.safetensors or pytorch_model.bin) in {}",
                model_dir.display()
            ))
        })?;
        let dtype = if device.is_cpu() {
            DType::F32
        } else {
            DType::F16
        };
        let vb = if weights.extension().is_some_and(|e| e == "safetensors") {
            // SAFETY: the file is memory-mapped read-only for the encoder's lifetime;
            // like every mmap, it must not be truncated while in use, which is the
            // contract candle's mmap loader states.
            unsafe { VarBuilder::from_mmaped_safetensors(&[weights], dtype, device)? }
        } else {
            VarBuilder::from_pth(&weights, dtype, device)?
        };
        Ok(Tmbed {
            encoder: T5Encoder::new(vb, &T5Config::prot_t5_xl())?,
            ensemble: cnn::Ensemble::load()?,
            device: device.clone(),
        })
    }

    /// ProtT5's embeddings of a batch, `(len, 1024)` per protein, row-major f32.
    fn embed(&self, sequences: &[&str]) -> Result<Vec<Vec<f32>>> {
        let tokens: Vec<Vec<u32>> = sequences.iter().map(|s| tokenize(s)).collect();
        let width = tokens.iter().map(Vec::len).max().unwrap_or(0);
        let mut ids = Vec::with_capacity(tokens.len() * width);
        let mut mask = Vec::with_capacity(tokens.len() * width);
        for t in &tokens {
            ids.extend(
                t.iter()
                    .copied()
                    .chain(std::iter::repeat_n(0, width - t.len())),
            );
            mask.extend((0..width).map(|i| u32::from(i < t.len())));
        }
        let shape = (tokens.len(), width);
        let ids = Tensor::from_vec(ids, shape, &self.device)?;
        let mask = Tensor::from_vec(mask, shape, &self.device)?;
        let hidden = self
            .encoder
            .forward(&ids, &mask)?
            .to_dtype(DType::F32)?
            .to_device(&Device::Cpu)?;
        sequences
            .iter()
            .enumerate()
            .map(|(i, s)| {
                // Each residue is one token, so the protein is the first `len` rows;
                // `</s>` and the padding are dropped, as TMbed's mask drops them.
                let len = s.chars().count();
                hidden
                    .get(i)?
                    .narrow(0, 0, len)?
                    .flatten_all()?
                    .to_vec1::<f32>()
            })
            .collect()
    }

    /// Predict every sequence, returned in input order (TMbed's `run_tmbed_batch`:
    /// sorted by length, batched by `batch_size`). A batch is embedded together, its
    /// shorter proteins padded and masked, as TMbed does.
    pub fn predict(&self, sequences: &[&str], batch_size: i64) -> Result<Vec<Prediction>> {
        let mut order: Vec<usize> = (0..sequences.len()).collect();
        order.sort_by_key(|&i| sequences[i].chars().count());
        let lengths: Vec<usize> = order
            .iter()
            .map(|&i| sequences[i].chars().count())
            .collect();
        let mut out: Vec<Option<Prediction>> = vec![None; sequences.len()];
        for (start, end) in make_batches(&lengths, batch_size) {
            let batch: Vec<&str> = order[start..end].iter().map(|&i| sequences[i]).collect();
            let embeddings = self.embed(&batch)?;
            for (&i, embedding) in order[start..end].iter().zip(embeddings) {
                let probabilities = self.ensemble.predict(&embedding);
                let labels = viterbi::decode(&probabilities);
                out[i] = Some(Prediction {
                    labels,
                    probabilities,
                });
            }
        }
        Ok(out
            .into_iter()
            .map(|p| p.expect("every protein is in one batch"))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_matches_prot_t5s_tokenizer() {
        // T5Tokenizer('M K B J O U Z X A') and ('M K * A'), add_special_tokens=True.
        assert_eq!(
            tokenize("MKBJOUZXA"),
            [19, 14, 23, 23, 23, 23, 23, 23, 3, 1]
        );
        assert_eq!(tokenize("mk*a"), [19, 14, 2, 3, 1]);
        assert_eq!(tokenize(""), [1]);
    }

    #[test]
    fn make_batches_matches_tmbed() {
        assert!(make_batches(&[], 4000).is_empty());
        assert_eq!(make_batches(&[10], 4000), [(0, 1)]);
        // Two of length 3000 fit 4000^1.5 one at a time; the 100s go together.
        assert_eq!(
            make_batches(&[100, 100, 100, 3000, 3000], 4000),
            [(0, 3), (3, 4), (4, 5)]
        );
        // A protein longer than the budget still gets a batch of its own.
        assert_eq!(make_batches(&[5000], 10), [(0, 1)]);
    }
}
