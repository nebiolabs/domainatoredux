//! `hmmer_build`: build an HMM profile from a multiple sequence alignment, with
//! user-specified NAME/ACC/DESC. Port of `domainator.hmmer_build`.
//!
//! Limitations vs. the Python tool (which wraps pyhmmer's `MSAFile` + `Builder`):
//! - Input MSA must be **Stockholm** format (hmmer-pure-rs only parses Stockholm;
//!   aligned-FASTA/Clustal are not yet supported upstream).
//! - The profile is built by hmmer-pure-rs's `build_hmm_from_msa` (fast model
//!   construction, symfrac 0.5) + its E-value calibration, so the emitted HMM is
//!   a valid, re-readable HMMER3 profile but is **not byte-identical** to
//!   pyhmmer/`hmmbuild` output (different builder/calibration implementation).

use std::io::Write;

use hmmer_pure_rs::msa::{read_afa_from_reader, read_stockholm_from_reader, Msa};
use hmmer_pure_rs::{Alphabet, Bg, Hmm};

/// Options for [`hmmer_build`].
pub struct HmmerBuildOptions {
    /// Profile NAME (required; sanitized).
    pub name: String,
    /// Optional ACC (sanitized).
    pub acc: Option<String>,
    /// Optional DESC (sanitized).
    pub desc: Option<String>,
    /// Force the alphabet (`amino` | `dna` | `rna`); inferred when `None`.
    pub alphabet: Option<String>,
    /// Max expected hit length — the profile's `MAXL` field, which nhmmer uses for
    /// windowed E-values. Nucleotide alphabets only.
    pub window_length: Option<i32>,
    /// Tail mass to derive the window length from. Nucleotide alphabets only, and
    /// mutually exclusive with `window_length`.
    pub window_beta: Option<f64>,
}

/// Replace characters outside `[\w\d_\-.;: ]` with `_` (mirrors `sanitize_string`).
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | ';' | ':' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Guess the alphabet from the aligned residues: nucleotide when the non-gap
/// symbols are overwhelmingly `ACGTUN`, RNA if a `U` appears (and no `T`), else
/// amino (mirrors easel's alphabet guess closely enough for build purposes).
fn infer_alphabet(msa: &Msa) -> Alphabet {
    let (mut acgt, mut total, mut has_u, mut has_t) = (0usize, 0usize, false, false);
    for seq in &msa.aseq {
        for &b in seq {
            let c = b.to_ascii_uppercase();
            if c == b'-' || c == b'.' {
                continue;
            }
            total += 1;
            match c {
                b'A' | b'C' | b'G' | b'N' => acgt += 1,
                b'T' => {
                    acgt += 1;
                    has_t = true;
                }
                b'U' => {
                    acgt += 1;
                    has_u = true;
                }
                _ => {}
            }
        }
    }
    if total > 0 && (acgt as f64 / total as f64) >= 0.9 {
        if has_u && !has_t {
            Alphabet::rna()
        } else {
            Alphabet::dna()
        }
    } else {
        Alphabet::amino()
    }
}

fn alphabet_from_name(name: &str) -> Result<Alphabet, String> {
    match name {
        "amino" => Ok(Alphabet::amino()),
        "dna" => Ok(Alphabet::dna()),
        "rna" => Ok(Alphabet::rna()),
        other => Err(format!(
            "--alphabet must be amino, dna, or rna, got {other:?}"
        )),
    }
}

/// Build an HMM from the first alignment in the file at `path` (see
/// [`hmmer_build_from_bytes`]).
pub fn hmmer_build(path: &std::path::Path, opts: &HmmerBuildOptions) -> Result<Hmm, String> {
    let text = std::fs::read(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    hmmer_build_from_bytes(&text, &path.display().to_string(), opts)
}

/// Build an HMM from the first alignment in `text`, a Stockholm or aligned-FASTA MSA
/// (told apart by the first line, as easel's format sniffing does); `source` names it
/// in errors.
pub fn hmmer_build_from_bytes(
    text: &[u8],
    source: &str,
    opts: &HmmerBuildOptions,
) -> Result<Hmm, String> {
    let first_line = text
        .split(|&b| b == b'\n')
        .map(<[u8]>::trim_ascii)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    let msas = if first_line.starts_with(b"# STOCKHOLM") {
        read_stockholm_from_reader(std::io::BufReader::new(text))
    } else if first_line.starts_with(b">") {
        read_afa_from_reader(&mut &text[..], "msa".to_string())
    } else {
        return Err(format!(
            "reading MSA {source}: not a Stockholm or aligned FASTA alignment (the formats \
             this port reads)"
        ));
    }
    .map_err(|e| format!("reading MSA {source}: {e:?}"))?;
    let msa = msas
        .into_iter()
        .next()
        .ok_or("no alignment found in the input MSA")?;
    if msa.nseq == 0 {
        return Err("the input alignment has no sequences".into());
    }
    let abc = match &opts.alphabet {
        Some(a) => alphabet_from_name(a)?,
        None => infer_alphabet(&msa),
    };
    let bg = Bg::new(&abc);
    // Fast model construction (symfrac 0.5), no hand architecture — pyhmmer's
    // Builder defaults.
    let mut hmm = build_from(&msa, &abc, &bg)
        .map_err(|e| format!("building the profile from {source}: {e}"))?;

    // MAXL / nhmmer search window. Both options are nucleotide-only and mutually
    // exclusive, matching Python.
    if opts.window_length.is_some() && opts.window_beta.is_some() {
        return Err(
            "only one of --window_length and --window_beta may be given (--window_length \
             takes precedence in HMMER)"
                .into(),
        );
    }
    if (opts.window_length.is_some() || opts.window_beta.is_some())
        && !crate::search::hmm_compare::is_nucleic_acid_alphabet(hmm.abc_type)
    {
        return Err(format!(
            "--window_length/--window_beta set the nhmmer search window and apply to \
             nucleotide profiles only; this alignment is {}",
            crate::search::hmm_compare::alphabet_name(hmm.abc_type)
        ));
    }
    if let Some(window_length) = opts.window_length {
        if window_length < 1 {
            return Err("--window_length must be >= 1".into());
        }
        hmm.max_length = window_length;
    }
    if opts.window_beta.is_some() {
        // Deriving MAXL from a tail mass needs the builder's length-model calibration,
        // which hmmer-pure-rs does not expose. Refusing is better than silently
        // ignoring it and writing a profile whose window is not what was asked for.
        return Err(
            "--window_beta is not supported by this port (deriving MAXL from a tail mass \
             needs calibration hmmer-pure-rs does not expose); pass --window_length \
             instead, or build the profile with hmmbuild"
                .into(),
        );
    }

    hmm.name = sanitize(&opts.name);
    hmm.acc = opts.acc.as_deref().map(sanitize);
    hmm.desc = opts.desc.as_deref().map(sanitize);
    Ok(hmm)
}

/// The builder fails, as C's `hmmbuild` does, when E-value calibration cannot
/// fit its distributions.
fn build_from(msa: &Msa, abc: &Alphabet, bg: &Bg) -> Result<Hmm, String> {
    hmmer_pure_rs::builder::build_hmm_from_msa(
        msa,
        abc,
        bg,
        0.5,
        0.5,
        false,
        hmmer_pure_rs::builder::RelativeWeighting::PositionBased,
        hmmer_pure_rs::builder::EffectiveSeqNumber::Entropy {
            target_re: None,
            target_sigma: None,
        },
        hmmer_pure_rs::builder::DEFAULT_BUILDER_SEED,
    )
}

/// Write `hmm` as an HMMER3 profile to `out`.
pub fn write_hmm_out<W: Write>(hmm: &Hmm, out: &mut W) -> Result<(), String> {
    crate::search::hmm_write::write_hmm(out, hmm).map_err(|e| format!("{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_replaces_bad_chars() {
        assert_eq!(sanitize("my/prot|name (1)"), "my_prot_name _1_");
        assert_eq!(sanitize("keep_-.;:x9 y"), "keep_-.;:x9 y");
    }

    fn options() -> HmmerBuildOptions {
        HmmerBuildOptions {
            name: "p".into(),
            acc: None,
            desc: None,
            alphabet: None,
            window_length: None,
            window_beta: None,
        }
    }

    #[test]
    fn stockholm_and_aligned_fasta_build_the_same_profile() {
        let stockholm = b"# STOCKHOLM 1.0\nseq1 ACGTACGT\nseq2 ACGTACGA\n//\n";
        let afa = b"\n>seq1\nACGTACGT\n>seq2\nACGTACGA\n";
        let from_stockholm = hmmer_build_from_bytes(stockholm, "s", &options()).unwrap();
        let from_afa = hmmer_build_from_bytes(afa, "a", &options()).unwrap();
        assert_eq!(from_stockholm.m, 8);
        assert_eq!(from_afa.m, from_stockholm.m);
        assert_eq!(from_afa.abc_type, from_stockholm.abc_type);
        assert_eq!(from_afa.mat, from_stockholm.mat);
    }

    #[test]
    fn an_unknown_msa_format_is_refused() {
        let err = hmmer_build_from_bytes(b"CLUSTAL W\n", "x.aln", &options()).unwrap_err();
        assert!(err.contains("not a Stockholm or aligned FASTA"), "{err}");
    }
}
