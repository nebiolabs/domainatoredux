//! `hmmer_build`: build an HMM profile from a multiple sequence alignment, with
//! user-specified NAME/ACC/DESC. Port of `domainator.hmmer_build`.
//!
//! The MSA is read as pyhmmer's `MSAFile` reads one from a path (`io::msa_formats`):
//! every format hmmbuild reads, guessed from the content, with the file's suffix
//! deciding what the content cannot. The profile is built by hmmer-pure-rs with
//! pyhmmer's `Builder` defaults, and matches pyhmmer's apart from the `DATE` and `COM`
//! lines and, on an occasional alignment, the last printed digit of a COMPO or STATS
//! value (a rounding near-tie).

use std::io::Write;

use hmmer_pure_rs::msa::Msa;
use hmmer_pure_rs::{Alphabet, Bg, Hmm};

use crate::io::msa_formats::{apply_alphabet, guess_format, read_msa};

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
/// [`hmmer_build_from_bytes`]). A gzip- or BGZF-compressed file is read decompressed.
pub fn hmmer_build(path: &std::path::Path, opts: &HmmerBuildOptions) -> Result<Hmm, String> {
    let read_error = |e: std::io::Error| format!("reading {}: {e}", path.display());
    let mut text = Vec::new();
    std::io::Read::read_to_end(
        &mut crate::io::open_decompressed(path).map_err(read_error)?,
        &mut text,
    )
    .map_err(read_error)?;
    let name = path.to_string_lossy();
    hmmer_build_from_bytes(&text, Some(&name), opts)
}

/// Build an HMM from the first alignment in `text`. Its format is guessed as Easel
/// guesses it, with `file_name`'s suffix deciding what the content cannot; standard
/// input has no name, so its format comes from the content alone.
pub fn hmmer_build_from_bytes(
    text: &[u8],
    file_name: Option<&str>,
    opts: &HmmerBuildOptions,
) -> Result<Hmm, String> {
    if text.is_empty() {
        return Err("the input MSA is empty".into());
    }
    let format = guess_format(text, file_name).map_err(|e| {
        format!(
            "could not determine the format of the input MSA ({e}). Acceptable formats are \
             those of hmmbuild: Stockholm, Pfam, aligned FASTA, A2M, Clustal, PHYLIP, SELEX \
             and PSI-BLAST"
        )
    })?;
    let mut msa = read_msa(text, format)
        .map_err(|e| format!("reading the input MSA as {}: {e}", format.name()))?;
    if msa.nseq == 0 {
        return Err("the input alignment has no sequences".into());
    }
    let abc = match &opts.alphabet {
        Some(a) => alphabet_from_name(a)?,
        None => infer_alphabet(&msa),
    };
    apply_alphabet(
        &mut msa,
        format,
        crate::search::hmm_compare::is_nucleic_acid_alphabet(abc.abc_type),
    );
    let bg = Bg::new(&abc);
    // Fast model construction (symfrac 0.5), no hand architecture — pyhmmer's
    // Builder defaults.
    let mut hmm = build_from(&msa, &abc, &bg).map_err(|e| format!("building the profile: {e}"))?;

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
    if let Some(window_beta) = opts.window_beta {
        if !(0.0..=1.0).contains(&window_beta) {
            return Err(format!(
                "--window_beta must be between 0 and 1, got {window_beta}"
            ));
        }
        // C's p7_Builder (p7_builder.c:512-516): a zero tail mass means 4*M, anything
        // else is p7_Builder_MaxLength.
        if window_beta == 0.0 {
            hmm.max_length = i32::try_from(hmm.m * 4).unwrap_or(i32::MAX);
        } else {
            hmmer_pure_rs::builder::set_max_length_from_beta(&mut hmm, window_beta);
        }
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
        let from_stockholm = hmmer_build_from_bytes(stockholm, None, &options()).unwrap();
        let from_afa = hmmer_build_from_bytes(afa, None, &options()).unwrap();
        assert_eq!(from_stockholm.m, 8);
        assert_eq!(from_afa.m, from_stockholm.m);
        assert_eq!(from_afa.abc_type, from_stockholm.abc_type);
        assert_eq!(from_afa.mat, from_stockholm.mat);
    }

    /// MAXL from a tail mass, against pyhmmer 0.12's `Builder(window_beta=...)` on the
    /// same alignment: 60 at 1e-3, 4*M at 0, and 101 (C's default beta) without one.
    #[test]
    fn window_beta_sets_maxl_as_pyhmmer_does() {
        let afa = b">s1\nACGTACGTTAGCATCGATCGGATCCATGCA\n\
                    >s2\nACGTACGATAGCATCGTTCGGATCCATGCA\n\
                    >s3\nACGAACGTTAGCATCGATCGGATGCATGCA\n";
        let maxl = |window_beta| {
            let opts = HmmerBuildOptions {
                alphabet: Some("dna".into()),
                window_beta,
                ..options()
            };
            hmmer_build_from_bytes(afa, None, &opts).unwrap().max_length
        };
        assert_eq!(maxl(Some(1e-3)), 60);
        assert_eq!(maxl(Some(0.0)), 120);
        assert_eq!(maxl(None), 101);
        for bad in [-1.0, 2.0] {
            let opts = HmmerBuildOptions {
                alphabet: Some("dna".into()),
                window_beta: Some(bad),
                ..options()
            };
            let err = hmmer_build_from_bytes(afa, None, &opts).unwrap_err();
            assert!(err.contains("between 0 and 1"), "{err}");
        }
    }

    #[test]
    fn an_unknown_msa_format_is_refused() {
        let err = hmmer_build_from_bytes(b"hello world foo\nbar\n", None, &options()).unwrap_err();
        assert!(err.contains("could not determine the format"), "{err}");
        let err = hmmer_build_from_bytes(b"\n \n", None, &options()).unwrap_err();
        assert!(err.contains("empty file/no data"), "{err}");
        let err = hmmer_build_from_bytes(b"", None, &options()).unwrap_err();
        assert!(err.contains("empty"), "{err}");
    }

    #[test]
    fn every_format_of_one_alignment_builds_the_same_profile() {
        let afa = b">s1\nMKV-LLAGWWHK\n>s2\nMKVALL-GWWHK\n>s3\nMRV-LLAGWFHK\n";
        let reference = hmmer_build_from_bytes(afa, None, &options()).unwrap();
        for (text, name) in [
            (&b"# STOCKHOLM 1.0\ns1 MKV-LLAGWWHK\ns2 MKVALL-GWWHK\ns3 MRV-LLAGWFHK\n//\n"[..], None),
            (b"CLUSTAL W (1.83) multiple sequence alignment\n\ns1   MKV-LLAG\ns2   MKVALL-G\n\
               s3   MRV-LLAG\n     *.* ** *\n\ns1   WWHK\ns2   WWHK\ns3   WFHK\n\n", None),
            (b" 3 12\ns1        MKV-LLAGWW HK\ns2        MKVALL-GWW HK\ns3        MRV-LLAGWF HK\n",
             None),
            (b"s1 MKV-LLAGWWHK\ns2 MKVALL-GWWHK\ns3 MRV-LLAGWFHK\n", None),
            (b"s1 MKV-LLAGWWHK\ns2 MKVALL-GWWHK\ns3 MRV-LLAGWFHK\n", Some("x.slx")),
        ] {
            let hmm = hmmer_build_from_bytes(text, name, &options()).unwrap();
            assert_eq!(hmm.mat, reference.mat, "{}", String::from_utf8_lossy(text));
            assert_eq!(hmm.checksum, reference.checksum, "{}", String::from_utf8_lossy(text));
        }
    }
}
