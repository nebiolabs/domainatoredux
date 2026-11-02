//! The encoder half of T5, as HF `transformers`' `T5EncoderModel` runs it: enough for
//! ProtT5 (TMbed's embeddings). Written from `modeling_t5.py` (transformers 4.x) rather
//! than taken from candle-transformers, whose T5 has no attention mask, so padded batches
//! could not match HF's.
//!
//! The pieces, per HF: RMS layer norms with no bias or mean subtraction, unscaled
//! dot-product attention with a learned relative-position bias (computed by the first
//! layer and shared by all of them), a ReLU feed-forward, and a final layer norm.

use candle_core::{DType, Device, Module, Result, Tensor, D};
use candle_nn::{Embedding, VarBuilder};

/// The T5 encoder's shape (HF `T5Config`, the parts an encoder uses).
#[derive(Clone, Debug)]
pub struct T5Config {
    pub vocab_size: usize,
    pub d_model: usize,
    pub d_kv: usize,
    pub d_ff: usize,
    pub num_heads: usize,
    pub num_layers: usize,
    pub relative_attention_num_buckets: usize,
    pub relative_attention_max_distance: usize,
    pub layer_norm_epsilon: f64,
}

impl T5Config {
    /// `Rostlab/prot_t5_xl_half_uniref50-enc` (ProtT5-XL-U50, encoder only).
    pub fn prot_t5_xl() -> Self {
        T5Config {
            vocab_size: 128,
            d_model: 1024,
            d_kv: 128,
            d_ff: 16384,
            num_heads: 32,
            num_layers: 24,
            relative_attention_num_buckets: 32,
            relative_attention_max_distance: 128,
            layer_norm_epsilon: 1e-6,
        }
    }
}

/// A bias-free linear layer: `x @ weight.T`, over the last dimension. The weight is
/// kept as stored, `(out, in)`, and multiplied through a transposed view: copying a
/// transpose of every weight at load time took longer than a prediction.
struct Linear {
    weight: Tensor,
}

impl Linear {
    fn new(vb: VarBuilder<'_>, d_in: usize, d_out: usize) -> Result<Self> {
        Ok(Linear {
            weight: vb.get((d_out, d_in), "weight")?,
        })
    }

    /// `x` is cast to the weight's dtype first, as HF casts into a kept-f32 `wo`.
    fn forward(&self, x: &Tensor) -> Result<Tensor> {
        let dims = x.dims().to_vec();
        let d_in = dims[dims.len() - 1];
        let rows = x.elem_count() / d_in;
        let out = x
            .to_dtype(self.weight.dtype())?
            .reshape((rows, d_in))?
            .matmul(&self.weight.t()?)?;
        let mut out_dims = dims;
        *out_dims.last_mut().expect("a tensor with dimensions") = self.weight.dim(0)?;
        out.reshape(out_dims)
    }
}

/// HF `T5LayerNorm`: scale by the root mean square, computed in f32.
struct LayerNorm {
    weight: Tensor,
    eps: f64,
}

impl LayerNorm {
    fn new(vb: VarBuilder<'_>, size: usize, eps: f64) -> Result<Self> {
        Ok(LayerNorm {
            weight: vb.get(size, "weight")?,
            eps,
        })
    }

    /// The output takes the weight's dtype, as HF casts to a half-precision weight.
    fn forward(&self, x: &Tensor) -> Result<Tensor> {
        let x = x.to_dtype(DType::F32)?;
        let variance = x.sqr()?.mean_keepdim(D::Minus1)?;
        let x = x.broadcast_div(&(variance + self.eps)?.sqrt()?)?;
        x.to_dtype(self.weight.dtype())?.broadcast_mul(&self.weight)
    }
}

/// HF `T5Attention._relative_position_bucket` for a bidirectional (encoder) model: the
/// bucket of `relative_position` (key position minus query position). Half the buckets
/// are for keys after the query; within each half, distances below a quarter of the
/// buckets are exact and longer ones are log-spaced up to `max_distance`. The log is
/// taken in f32, as torch does, so a distance on a bucket boundary lands where HF's does.
pub fn relative_position_bucket(
    relative_position: i64,
    num_buckets: usize,
    max_distance: usize,
) -> usize {
    let half = num_buckets / 2;
    let mut bucket = if relative_position > 0 { half } else { 0 };
    let distance = relative_position.unsigned_abs() as usize;
    let max_exact = half / 2;
    if distance < max_exact {
        bucket += distance;
    } else {
        let scale = (max_distance as f64 / max_exact as f64).ln() as f32;
        // Never negative: distance >= max_exact, so the log is >= 0. The cast truncates,
        // as torch's `.to(torch.long)` does.
        #[allow(clippy::cast_sign_loss)]
        let large = ((distance as f32 / max_exact as f32).ln() / scale * (half - max_exact) as f32)
            as usize;
        bucket += (max_exact + large).min(half - 1);
    }
    bucket
}

struct Attention {
    q: Linear,
    k: Linear,
    v: Linear,
    o: Linear,
    relative_attention_bias: Option<Embedding>,
    num_heads: usize,
    d_kv: usize,
    num_buckets: usize,
    max_distance: usize,
}

impl Attention {
    fn new(vb: VarBuilder<'_>, cfg: &T5Config, has_relative_bias: bool) -> Result<Self> {
        let inner = cfg.num_heads * cfg.d_kv;
        let relative_attention_bias = if has_relative_bias {
            Some(candle_nn::embedding(
                cfg.relative_attention_num_buckets,
                cfg.num_heads,
                vb.pp("relative_attention_bias"),
            )?)
        } else {
            None
        };
        Ok(Attention {
            q: Linear::new(vb.pp("q"), cfg.d_model, inner)?,
            k: Linear::new(vb.pp("k"), cfg.d_model, inner)?,
            v: Linear::new(vb.pp("v"), cfg.d_model, inner)?,
            o: Linear::new(vb.pp("o"), inner, cfg.d_model)?,
            relative_attention_bias,
            num_heads: cfg.num_heads,
            d_kv: cfg.d_kv,
            num_buckets: cfg.relative_attention_num_buckets,
            max_distance: cfg.relative_attention_max_distance,
        })
    }

    /// The `(1, heads, len, len)` position bias, from the first layer's embedding.
    fn position_bias(&self, len: usize, device: &Device) -> Result<Tensor> {
        let embedding = self
            .relative_attention_bias
            .as_ref()
            .expect("only the first layer computes the position bias");
        let buckets: Vec<u32> = (0..len)
            .flat_map(|q| {
                (0..len).map(move |k| {
                    relative_position_bucket(
                        k as i64 - q as i64,
                        self.num_buckets,
                        self.max_distance,
                    ) as u32
                })
            })
            .collect();
        let buckets = Tensor::from_vec(buckets, (len, len), device)?;
        // (len, len, heads) -> (1, heads, len, len)
        embedding
            .forward(&buckets)?
            .permute((2, 0, 1))?
            .unsqueeze(0)
    }

    /// `x` is `(batch, len, d_model)`; `bias` is the position bias plus the mask.
    fn forward(&self, x: &Tensor, bias: &Tensor) -> Result<Tensor> {
        let (batch, len, _) = x.dims3()?;
        let heads = |t: Tensor| -> Result<Tensor> {
            t.reshape((batch, len, self.num_heads, self.d_kv))?
                .transpose(1, 2)?
                .contiguous()
        };
        let q = heads(self.q.forward(x)?)?;
        let k = heads(self.k.forward(x)?)?;
        let v = heads(self.v.forward(x)?)?;
        // T5 does not scale the scores by 1/sqrt(d_kv): it is folded into the weights.
        let scores = q
            .matmul(&k.t()?.contiguous()?)?
            .broadcast_add(&bias.to_dtype(q.dtype())?)?;
        let weights =
            candle_nn::ops::softmax_last_dim(&scores.to_dtype(DType::F32)?)?.to_dtype(q.dtype())?;
        let out = weights.matmul(&v)?.transpose(1, 2)?.reshape((
            batch,
            len,
            self.num_heads * self.d_kv,
        ))?;
        self.o.forward(&out)
    }
}

struct Block {
    attention_norm: LayerNorm,
    attention: Attention,
    ff_norm: LayerNorm,
    wi: Linear,
    wo: Linear,
}

impl Block {
    fn new(vb: VarBuilder<'_>, cfg: &T5Config, first: bool) -> Result<Self> {
        let attn = vb.pp("layer.0");
        let ff = vb.pp("layer.1");
        Ok(Block {
            attention_norm: LayerNorm::new(
                attn.pp("layer_norm"),
                cfg.d_model,
                cfg.layer_norm_epsilon,
            )?,
            attention: Attention::new(attn.pp("SelfAttention"), cfg, first)?,
            ff_norm: LayerNorm::new(ff.pp("layer_norm"), cfg.d_model, cfg.layer_norm_epsilon)?,
            wi: Linear::new(ff.pp("DenseReluDense.wi"), cfg.d_model, cfg.d_ff)?,
            // HF keeps `wo` in f32 in a half-precision model (`_keep_in_fp32_modules`).
            wo: Linear::new(
                ff.pp("DenseReluDense.wo").set_dtype(DType::F32),
                cfg.d_ff,
                cfg.d_model,
            )?,
        })
    }

    /// In half precision the residual stream turns f32 at the first f32 `wo` output, as
    /// torch's type promotion makes it in HF; the sublayers themselves stay half.
    fn forward(&self, x: &Tensor, bias: &Tensor) -> Result<Tensor> {
        let attention = self
            .attention
            .forward(&self.attention_norm.forward(x)?, bias)?;
        let x = clamp_f16(add_promoting(x, &attention)?)?;
        let ff = self
            .wo
            .forward(&self.wi.forward(&self.ff_norm.forward(&x)?)?.relu()?)?;
        clamp_f16(add_promoting(&x, &ff)?)
    }
}

/// `a + b`, in f32 when their dtypes differ (torch's promotion of f16 + f32).
fn add_promoting(a: &Tensor, b: &Tensor) -> Result<Tensor> {
    if a.dtype() == b.dtype() {
        a + b
    } else {
        a.to_dtype(DType::F32)? + b.to_dtype(DType::F32)?
    }
}

/// HF clamps half-precision hidden states just inside the f16 range, so an overflow
/// does not turn into infinities and then NaNs.
fn clamp_f16(x: Tensor) -> Result<Tensor> {
    if x.dtype() == DType::F16 {
        let limit = 65504.0 - 1000.0;
        x.clamp(-limit, limit)
    } else {
        Ok(x)
    }
}

/// A T5 encoder: token ids in, the last hidden states (after the final layer norm) out.
pub struct T5Encoder {
    embed_tokens: Embedding,
    blocks: Vec<Block>,
    final_layer_norm: LayerNorm,
}

impl T5Encoder {
    /// Load from a `VarBuilder` rooted where HF's `T5EncoderModel` keeps its weights:
    /// `shared.weight` and `encoder.*`.
    pub fn new(vb: VarBuilder<'_>, cfg: &T5Config) -> Result<Self> {
        let embed_tokens = candle_nn::embedding(cfg.vocab_size, cfg.d_model, vb.pp("shared"))?;
        let encoder = vb.pp("encoder");
        let blocks = (0..cfg.num_layers)
            .map(|i| Block::new(encoder.pp(format!("block.{i}")), cfg, i == 0))
            .collect::<Result<Vec<_>>>()?;
        let final_layer_norm = LayerNorm::new(
            encoder.pp("final_layer_norm"),
            cfg.d_model,
            cfg.layer_norm_epsilon,
        )?;
        Ok(T5Encoder {
            embed_tokens,
            blocks,
            final_layer_norm,
        })
    }

    /// `input_ids` and `attention_mask` are `(batch, len)` (u32; the mask 1 for a token,
    /// 0 for padding); returns `(batch, len, d_model)`.
    pub fn forward(&self, input_ids: &Tensor, attention_mask: &Tensor) -> Result<Tensor> {
        let (_, len) = input_ids.dims2()?;
        let mut x = self.embed_tokens.forward(input_ids)?;
        let dtype = x.dtype();
        // HF's extended mask: 0 where attended, the dtype's minimum where padded.
        let min = match dtype {
            DType::F16 => -65504.0,
            _ => f64::from(f32::MIN),
        };
        let mask = ((attention_mask.to_dtype(DType::F32)? - 1.0)? * -min)?
            .unsqueeze(1)?
            .unsqueeze(1)?;
        let position_bias = self.blocks[0]
            .attention
            .position_bias(len, input_ids.device())?
            .to_dtype(DType::F32)?;
        let bias = position_bias.broadcast_add(&mask)?;
        for block in &self.blocks {
            x = block.forward(&x, &bias)?;
        }
        self.final_layer_norm.forward(&x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_position_buckets_match_hf() {
        // HF `_relative_position_bucket(positions, bidirectional=True, 32, 128)`.
        let bucket = |p| relative_position_bucket(p, 32, 128);
        assert_eq!(bucket(0), 0);
        assert_eq!(bucket(-7), 7);
        assert_eq!(bucket(7), 23);
        assert_eq!(bucket(-8), 8);
        assert_eq!(bucket(-16), 10);
        assert_eq!(bucket(-127), 15);
        assert_eq!(bucket(-10_000), 15);
        assert_eq!(bucket(10_000), 31);
    }
}
