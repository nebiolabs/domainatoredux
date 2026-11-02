//! TMbed's CNN (`tmbed/model.py`, `Predictor`) and its five-model ensemble
//! (`predict_sequences`), for one protein's embeddings.
//!
//! The network is small enough to run as plain loops: a 1x1 convolution from 1024
//! embedding dimensions to 64 channels, two depthwise convolutions (widths 9 and 21) over
//! its output, each block normalized over the whole sequence (`SeqNorm`) and passed
//! through a ReLU, then a 1x1 convolution of all 192 channels to the five classes and a
//! Gaussian smoothing along the sequence. TMbed pads a batch to its longest protein and
//! masks the padding to zero, which is what zero padding at the protein's ends gives, so
//! one protein at a time reproduces the batched result.

use std::collections::HashMap;

use candle_core::{Device, Tensor};

/// The weights of the five models, from TMbed's `tmbed/models/cnn/cv_*.pt` at the commit
/// upstream domainator pins (Apache-2.0; see `assets/tmbed/NOTICE`), converted by
/// `scripts/tmbed/make_assets.py`.
const WEIGHTS: &[u8] = include_bytes!("../../assets/tmbed/cnn.safetensors");

/// The number of models in the ensemble.
const MODELS: usize = 5;
/// Channels per block (`Predictor(channels=64)`).
const CHANNELS: usize = 64;
/// Embedding dimensions (ProtT5's `d_model`).
pub const EMBEDDING_DIM: usize = 1024;
/// Classes: B, H, S, i, o.
pub const CLASSES: usize = 5;

/// `Conv` with its `SeqNorm`: a bias-free convolution along the sequence (one input
/// channel per output channel when depthwise), then normalization, affine and ReLU.
struct ConvBlock {
    /// `(out, in, width)`, `in` being 1 when depthwise.
    weight: Vec<f32>,
    width: usize,
    depthwise: bool,
    norm_weight: Vec<f32>,
    norm_bias: Vec<f32>,
    eps: f32,
}

struct Model {
    input: ConvBlock,
    dwc1: ConvBlock,
    dwc2: ConvBlock,
    /// `(5, 192)`
    output_weight: Vec<f32>,
    output_bias: Vec<f32>,
    /// The smoothing kernel (7 taps), as stored with the model.
    filter: Vec<f32>,
}

/// TMbed's CNN ensemble.
pub struct Ensemble {
    models: Vec<Model>,
}

fn flat(tensors: &HashMap<String, Tensor>, name: &str) -> candle_core::Result<Vec<f32>> {
    tensors
        .get(name)
        .ok_or_else(|| candle_core::Error::Msg(format!("TMbed CNN weights lack {name}")))?
        .flatten_all()?
        .to_vec1::<f32>()
}

impl ConvBlock {
    fn load(
        tensors: &HashMap<String, Tensor>,
        prefix: &str,
        width: usize,
        depthwise: bool,
    ) -> candle_core::Result<Self> {
        Ok(ConvBlock {
            weight: flat(tensors, &format!("{prefix}.conv.weight"))?,
            width,
            depthwise,
            norm_weight: flat(tensors, &format!("{prefix}.norm.weight"))?,
            norm_bias: flat(tensors, &format!("{prefix}.norm.bias"))?,
            eps: flat(tensors, &format!("{prefix}.norm.eps"))?[0],
        })
    }

    /// `SeqNorm` then ReLU, in place on `(CHANNELS, len)`: mean and variance over every
    /// channel and position, as TMbed computes them (sums times a reciprocal).
    fn norm_relu(&self, x: &mut [f32], len: usize) {
        let rsum = 1.0 / (len as f32 * CHANNELS as f32);
        let mean = (x.iter().map(|&v| f64::from(v)).sum::<f64>() as f32) * rsum;
        for v in x.iter_mut() {
            *v -= mean;
        }
        let var = (x.iter().map(|&v| f64::from(v * v)).sum::<f64>() as f32) * rsum;
        let scale = 1.0 / (var + self.eps).sqrt();
        for (c, row) in x.chunks_mut(len).enumerate() {
            for v in row {
                *v = (*v * scale * self.norm_weight[c] + self.norm_bias[c]).max(0.0);
            }
        }
    }

    /// The input block, from embeddings `(len, EMBEDDING_DIM)` row-major.
    fn forward_input(&self, embeddings: &[f32], len: usize) -> Vec<f32> {
        let mut out = vec![0f32; CHANNELS * len];
        for (o, row) in out.chunks_mut(len).enumerate() {
            let w = &self.weight[o * EMBEDDING_DIM..(o + 1) * EMBEDDING_DIM];
            for (n, v) in row.iter_mut().enumerate() {
                let e = &embeddings[n * EMBEDDING_DIM..(n + 1) * EMBEDDING_DIM];
                *v = w.iter().zip(e).map(|(a, b)| a * b).sum();
            }
        }
        self.norm_relu(&mut out, len);
        out
    }

    /// A depthwise block over `(CHANNELS, len)`, zero-padded by half its width.
    fn forward_depthwise(&self, x: &[f32], len: usize) -> Vec<f32> {
        debug_assert!(self.depthwise);
        let pad = self.width / 2;
        let mut out = vec![0f32; CHANNELS * len];
        for c in 0..CHANNELS {
            let w = &self.weight[c * self.width..(c + 1) * self.width];
            let src = &x[c * len..(c + 1) * len];
            for n in 0..len {
                let mut acc = 0f32;
                for (t, &wt) in w.iter().enumerate() {
                    let pos = n + t;
                    if pos >= pad && pos - pad < len {
                        acc += wt * src[pos - pad];
                    }
                }
                out[c * len + n] = acc;
            }
        }
        self.norm_relu(&mut out, len);
        out
    }
}

impl Model {
    fn load(tensors: &HashMap<String, Tensor>, prefix: &str) -> candle_core::Result<Self> {
        let block = |name: &str, width, depthwise| {
            ConvBlock::load(tensors, &format!("{prefix}.model.{name}"), width, depthwise)
        };
        Ok(Model {
            input: block("input", 1, false)?,
            dwc1: block("dwc1", 9, true)?,
            dwc2: block("dwc2", 21, true)?,
            output_weight: flat(tensors, &format!("{prefix}.model.output.weight"))?,
            output_bias: flat(tensors, &format!("{prefix}.model.output.bias"))?,
            filter: flat(tensors, &format!("{prefix}.filter_kernel"))?,
        })
    }

    /// The smoothed class scores `(CLASSES, len)` (`Predictor.forward`).
    fn forward(&self, embeddings: &[f32], len: usize) -> Vec<f32> {
        let x = self.input.forward_input(embeddings, len);
        let z1 = self.dwc1.forward_depthwise(&x, len);
        let z2 = self.dwc2.forward_depthwise(&x, len);
        let concat = [x, z1, z2].concat();
        let all = 3 * CHANNELS;
        let mut scores = vec![0f32; CLASSES * len];
        for o in 0..CLASSES {
            let w = &self.output_weight[o * all..(o + 1) * all];
            for n in 0..len {
                let mut acc = 0f32;
                for (c, &wc) in w.iter().enumerate() {
                    acc += wc * concat[c * len + n];
                }
                scores[o * len + n] = acc + self.output_bias[o];
            }
        }
        // Smoothing: each position the kernel-weighted sum of its 7-wide window,
        // zero beyond the ends.
        let half = self.filter.len() / 2;
        let mut smoothed = vec![0f32; CLASSES * len];
        for o in 0..CLASSES {
            let row = &scores[o * len..(o + 1) * len];
            for n in 0..len {
                let mut acc = 0f32;
                for (m, &k) in self.filter.iter().enumerate() {
                    let pos = n + m;
                    if pos >= half && pos - half < len {
                        acc += row[pos - half] * k;
                    }
                }
                smoothed[o * len + n] = acc;
            }
        }
        smoothed
    }
}

impl Ensemble {
    /// The five models, from the weights built into this crate.
    pub fn load() -> candle_core::Result<Self> {
        let tensors = candle_core::safetensors::load_buffer(WEIGHTS, &Device::Cpu)?;
        let models = (0..MODELS)
            .map(|i| Model::load(&tensors, &format!("cv_{i}")))
            .collect::<candle_core::Result<Vec<_>>>()?;
        Ok(Ensemble { models })
    }

    /// Per residue, the class probabilities (B, H, S, i, o) averaged over the models,
    /// from one protein's embeddings `(len, EMBEDDING_DIM)`, row-major.
    pub fn predict(&self, embeddings: &[f32]) -> Vec<[f32; CLASSES]> {
        let len = embeddings.len() / EMBEDDING_DIM;
        let mut sum = vec![[0f32; CLASSES]; len];
        for model in &self.models {
            let scores = model.forward(embeddings, len);
            for (n, total) in sum.iter_mut().enumerate() {
                let column: [f32; CLASSES] = std::array::from_fn(|o| scores[o * len + n]);
                let max = column.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                let exp = column.map(|v| (v - max).exp());
                let denominator: f32 = exp.iter().sum();
                for (t, e) in total.iter_mut().zip(exp) {
                    *t += e / denominator;
                }
            }
        }
        let count = self.models.len() as f32;
        sum.into_iter().map(|p| p.map(|v| v / count)).collect()
    }
}
