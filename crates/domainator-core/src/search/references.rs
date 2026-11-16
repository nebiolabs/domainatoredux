//! Reference-file classification (port of `domainate.read_references`).
//!
//! Each `-r` file is classified by extension + alphabet into a search group:
//! - protein `.hmm`  -> hmmsearch  (protein peptides)
//! - protein FASTA   -> phmmer     (protein peptides; single-seq profile per ref)
//! - nucleotide `.hmm` / FASTA -> nhmmer (whole nucleotide contigs, both strands)
//! - `.cm` (infernal) -> cmsearch (whole nucleotide contigs, both strands)
//!
//! Nucleotide or CM references combined with protein input is an error, matching
//! `validate_input_molecule_types`.

use std::path::Path;

use super::hmmer::{load_hmm_dbs, load_phmmer_db, HmmDb};
use super::infernal::{load_cm_db, CmDb};
use super::nhmmer::NuclDb;

/// Classified references: protein dbs (hmmsearch + phmmer), nucleotide dbs
/// (nhmmer) and covariance models (cmsearch). Empty is legal (no references).
#[derive(Default)]
pub struct References {
    pub protein: Vec<HmmDb>,
    pub nucleotide: Vec<NuclDb>,
    pub infernal: Vec<CmDb>,
}

impl References {
    /// `true` if any nucleic-acid (nhmmer or cmsearch) reference was loaded —
    /// protein input with these is an error (`has_nucleic_acid_references`).
    pub fn has_nucleotide(&self) -> bool {
        !self.nucleotide.is_empty() || !self.infernal.is_empty()
    }
}

/// File "type" by extension, mirroring `utils.get_file_type` for reference files.
enum RefFileType {
    Hmm,
    Fasta,
    Cm,
}

fn ref_file_type(file: &str) -> Result<RefFileType, String> {
    // One compression suffix is looked through, as `get_file_type` does, so
    // `refs.hmm.gz` is an hmm reference; the readers decompress by content.
    let path = Path::new(file);
    let unwrapped = if crate::io::is_compressed_path(path) {
        path.file_stem().map(Path::new).unwrap_or(path)
    } else {
        path
    };
    let ext = unwrapped
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "hmm" | "hmmer" => Ok(RefFileType::Hmm),
        "fasta" | "fa" | "fna" | "pep" | "faa" => Ok(RefFileType::Fasta),
        "cm" => Ok(RefFileType::Cm),
        _ if crate::tools::structure_lib::is_structure_path(file) => Err(format!(
            "'{file}' is a protein structure file, which cannot be used as a domainate \
             reference. Use structure_domainate to annotate with reference structures."
        )),
        _ => Err(format!(
            "Reference files must be fasta, hmmer, or infernal cm (got {file:?})"
        )),
    }
}

/// Easel's alphabet guess for one sequence (`esl_abc_GuessAlphabet`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EaselGuess {
    Dna,
    Rna,
    Amino,
}

/// Letters Easel treats as amino-acid only.
const AMINO_ONLY: &[u8] = b"EFIJLOPQZ";
/// Every letter the DNA alphabet reads as a nucleotide: the canonical bases,
/// `U`, the IUPAC degeneracy codes, and `X` (an alias of `N`).
const NUCLEOTIDE_IUPAC: &[u8] = b"ACGTURYSWKMBDHVNX";
/// Easel guesses from the first `4000` residues of a sequence
/// (`sqascii_ReadWindow(sqfp, 0, 4000, sq)`).
const EASEL_GUESS_WINDOW: usize = 4000;

/// A..Z histogram of `seq`, case-folded, counting at most `window` non-whitespace
/// symbols (`esl_sq_GuessAlphabet` over the window `sqascii_ReadWindow` reads).
fn letter_counts(seq: &[u8], window: usize) -> [i64; 26] {
    let mut counts = [0i64; 26];
    for &c in seq.iter().filter(|c| !c.is_ascii_whitespace()).take(window) {
        let upper = c.to_ascii_uppercase();
        if upper.is_ascii_uppercase() {
            counts[usize::from(upper - b'A')] += 1;
        }
    }
    counts
}

fn count_of(ct: &[i64; 26], letters: &[u8]) -> i64 {
    letters.iter().map(|&c| ct[usize::from(c - b'A')]).sum()
}

/// Port of Easel's `esl_abc_GuessAlphabet` (`esl_alphabet.c`), which is what
/// pyhmmer's `SequenceFile(digital=True)` runs on a reference FASTA's first
/// sequence. `None` is Easel's `eslENOALPHABET` ("Could not determine alphabet").
/// The same port lives, private, in hmmer-pure-rs's `makehmmerdb`.
fn easel_guess_alphabet(ct: &[i64; 26]) -> Option<EaselGuess> {
    let idx = |c: u8| usize::from(c - b'A');
    // (residue count, number of distinct letters seen) over a letter set.
    let tally = |letters: &[u8]| {
        letters.iter().fold((0i64, 0i32), |(n, x), &c| {
            let k = ct[idx(c)];
            (n + k, x + i32::from(k > 0))
        })
    };
    let (n1, x1) = tally(AMINO_ONLY);
    let (n2, x2) = tally(b"ACG");
    let (n3, x3) = tally(b"DHKMRSVWY");
    let n: i64 = ct.iter().sum();
    let nt = ct[idx(b'T')];
    let xt = i32::from(nt > 0);
    let nu = ct[idx(b'U')];
    let xu = i32::from(nu > 0);
    let nx = ct[idx(b'X')];
    let nn = ct[idx(b'N')];
    let xn = i32::from(nn > 0);
    // `n * 0.02` as Easel computes it, in floating point.
    let at_most_2_percent = |other: i64| other as f64 <= 0.02 * n as f64;

    if n <= 10 {
        None
    } else if n > 2000 && nn == n {
        Some(EaselGuess::Dna)
    } else if n1 > 0 {
        Some(EaselGuess::Amino)
    } else if at_most_2_percent(n - (n2 + nt + nn)) && x2 + xt == 4 {
        Some(EaselGuess::Dna)
    } else if at_most_2_percent(n - (n2 + nu + nn)) && x2 + xu == 4 {
        Some(EaselGuess::Rna)
    } else if at_most_2_percent(n - (n1 + n2 + n3 + nn + nt + nx))
        && n3 > n2
        && x1 + x2 + x3 + xn + xt >= 15
    {
        Some(EaselGuess::Amino)
    } else {
        None
    }
}

/// What one reference record looks like.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RecordAlphabet {
    Nucleotide,
    Protein,
    /// Overwhelmingly nucleotide, but with these amino-only letters in it.
    NucleotideWithAminoLetters(String),
    /// Too short, or too odd, to say.
    Unknown,
}

/// Classify one reference sequence.
///
/// Easel's guess decides wherever it can, so every file Python accepts is
/// read the way Python reads it. Two port-only rules cover the cases where
/// Easel fails or silently goes wrong. Easel gives up on DNA that is more than 2%
/// IUPAC ambiguity codes, so a sequence of more than 10 letters that are all
/// nucleotide codes, at least 75% of them `ACGTUN`, counts as nucleotide. And
/// Easel calls DNA with a stray amino-only letter (an `E`) protein, so a
/// sequence that is at least 90% `ACGTUN` but has one is reported, not
/// searched as protein.
fn classify_record(seq: &[u8]) -> RecordAlphabet {
    let full = letter_counts(seq, usize::MAX);
    let n: i64 = full.iter().sum();
    let canonical = count_of(&full, b"ACGTUN");
    match easel_guess_alphabet(&letter_counts(seq, EASEL_GUESS_WINDOW)) {
        Some(EaselGuess::Dna | EaselGuess::Rna) => RecordAlphabet::Nucleotide,
        Some(EaselGuess::Amino) => {
            let stray: String = AMINO_ONLY
                .iter()
                .filter(|&&c| full[usize::from(c - b'A')] > 0)
                .map(|&c| char::from(c))
                .collect();
            if !stray.is_empty() && canonical as f64 >= 0.9 * n as f64 {
                RecordAlphabet::NucleotideWithAminoLetters(stray)
            } else {
                RecordAlphabet::Protein
            }
        }
        None => {
            if n > 10
                && count_of(&full, NUCLEOTIDE_IUPAC) == n
                && canonical as f64 >= 0.75 * n as f64
            {
                RecordAlphabet::Nucleotide
            } else {
                RecordAlphabet::Unknown
            }
        }
    }
}

/// Whether a FASTA reference is nucleic acid.
///
/// As in Python's `_fasta_is_nucleic_acid`, the first record decides (an empty
/// file is protein). An undecidable first record is an error, as it is in
/// Python. Unlike Python, every later record is checked too: a record that is
/// confidently the other alphabet, or DNA with amino-only letters, is an error
/// rather than a search that silently finds nothing.
fn fasta_is_nucleic_acid(file: &str) -> std::io::Result<bool> {
    let invalid = |msg: String| std::io::Error::new(std::io::ErrorKind::InvalidData, msg);
    let recs = crate::io::fasta::read_fasta_decompressed(file, Some("protein"))?;
    let Some(first) = recs.first() else {
        return Ok(false);
    };
    let is_nucleotide = match classify_record(&first.seq) {
        RecordAlphabet::Nucleotide => true,
        RecordAlphabet::Protein => false,
        RecordAlphabet::NucleotideWithAminoLetters(letters) => {
            return Err(stray_letters_error(file, &first.id, &letters));
        }
        RecordAlphabet::Unknown => {
            return Err(invalid(format!(
                "Could not determine alphabet of reference '{file}': its first record \
                 '{}' is neither clearly nucleotide nor clearly protein",
                first.id
            )));
        }
    };
    for rec in &recs[1..] {
        let other_is_nucleotide = match classify_record(&rec.seq) {
            RecordAlphabet::Nucleotide => true,
            RecordAlphabet::Protein => false,
            RecordAlphabet::NucleotideWithAminoLetters(letters) => {
                return Err(stray_letters_error(file, &rec.id, &letters));
            }
            RecordAlphabet::Unknown => continue,
        };
        if other_is_nucleotide != is_nucleotide {
            let (nucl, prot) = if is_nucleotide {
                (&first.id, &rec.id)
            } else {
                (&rec.id, &first.id)
            };
            return Err(invalid(format!(
                "reference '{file}' mixes nucleotide (record '{nucl}') and protein (record \
                 '{prot}') sequences; put them in separate files"
            )));
        }
    }
    Ok(is_nucleotide)
}

fn stray_letters_error(file: &str, id: &str, letters: &str) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!(
            "reference '{file}': record '{id}' looks like a nucleotide sequence but contains \
             amino-acid-only letters ({letters}); fix the sequence, or put it in a protein \
             reference if it is one"
        ),
    )
}

/// Whether an `.hmm` file's first profile is a DNA/RNA model.
fn hmm_is_nucleic_acid(file: &str) -> std::io::Result<bool> {
    use hmmer_pure_rs::alphabet::AlphabetType;
    let path = Path::new(file);
    let first = crate::search::hmm_read::read_first_hmm(path).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            crate::search::hmm_read::error_message(path, &e),
        )
    })?;
    Ok(first.is_some_and(|h| matches!(h.abc_type, AlphabetType::Dna | AlphabetType::Rna)))
}

/// Load a nucleotide `.hmm` file as an nhmmer db (file stem = db name).
fn load_nucl_hmm(file: &str) -> std::io::Result<NuclDb> {
    let path = Path::new(file);
    let hmms = crate::search::hmm_read::read_hmm_file(path).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            crate::search::hmm_read::error_message(path, &e),
        )
    })?;
    Ok(NuclDb {
        name: file_stem(file),
        hmms,
    })
}

/// pyhmmer's nucleotide gap-open / gap-extend defaults (C `p7_builder.c`'s
/// nucleotide branch); the amino pair is `0.02`/`0.4`.
const NUCL_POPEN: f32 = 0.03125;
const NUCL_PEXTEND: f32 = 0.75;

/// Build a nucleotide FASTA nhmmer db: one single-sequence DNA profile per ref
/// sequence (mirrors `read_pyhmmer_fastas` + `pyhmmer.hmmer.nhmmer`).
///
/// Python hands each query sequence to pyhmmer's default `Builder`, i.e. C's
/// `p7_SingleBuilder`: a score-matrix conversion (DNA1 for nucleotide, not
/// BLOSUM62) plus the nucleotide-only window length that nhmmer's E-values are
/// computed against. `popen`/`pextend` are pyhmmer's nucleotide defaults
/// (`0.03125`/`0.75`, C's `p7_builder.c` nucleotide branch), and the window
/// length is derived from the default tail mass because we pass neither
/// `--w_len` nor `--w_beta`.
///
/// `build_single_seq_hmm` calibrates internally, so there is no separate
/// `calibrate()` call here -- a second pass would redraw the Gumbel fit.
fn load_nucl_fasta(file: &str) -> std::io::Result<NuclDb> {
    use hmmer_pure_rs::{Alphabet, Bg};
    let recs = crate::io::fasta::read_fasta_decompressed(file, Some("nucleotide"))?;
    let abc = Alphabet::dna();
    let bg = Bg::new(&abc);
    let mut hmms = Vec::new();
    for rec in &recs {
        let residues: Vec<u8> = rec
            .seq
            .iter()
            .copied()
            .filter(|b| b.is_ascii_alphabetic())
            .map(|b| b.to_ascii_uppercase())
            .collect();
        if residues.is_empty() {
            continue;
        }
        let (id, desc) = super::hmmer::fasta_description(&rec.description);
        // Never drop a symbol silently: the model length below must be the
        // digitized length, and a letter the DNA alphabet cannot read is the
        // user's to fix, not ours to skip.
        let dsq = abc.digitize_checked(&residues).map_err(|e| {
            let msg = match e {
                hmmer_pure_rs::errors::HmmerError::Format(m) => m,
                other => other.to_string(),
            };
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("nucleotide reference '{file}', record '{id}': {msg}"),
            )
        })?;
        let mut hmm = hmmer_pure_rs::seqmodel::build_single_seq_hmm(
            &id,
            &dsq,
            dsq.len() - 2, // strip the two sentinels
            &abc,
            &bg,
            NUCL_POPEN,
            NUCL_PEXTEND,
        );
        hmm.desc = desc;
        hmms.push(hmm);
    }
    Ok(NuclDb {
        name: file_stem(file),
        hmms,
    })
}

fn file_stem(f: &str) -> String {
    crate::io::db_name_from_path(f)
}

/// Classify and load all reference files (port of `read_references`).
pub fn read_references(files: &[String]) -> std::io::Result<References> {
    let mut hmm_files: Vec<String> = Vec::new();
    let mut fasta_files: Vec<String> = Vec::new();
    let mut refs = References::default();

    for file in files {
        let ftype = ref_file_type(file)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        match ftype {
            RefFileType::Cm => {
                let db = load_cm_db(file)?;
                // `read_infernal_cms` keys its databases by name and refuses a repeat.
                if refs.infernal.iter().any(|d| d.name == db.name) {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "Multiple cm files with the same name, please combine the models into \
                         a single file, or rename one of the files.",
                    ));
                }
                refs.infernal.push(db);
            }
            RefFileType::Hmm => {
                if hmm_is_nucleic_acid(file)? {
                    refs.nucleotide.push(load_nucl_hmm(file)?);
                } else {
                    hmm_files.push(file.clone());
                }
            }
            RefFileType::Fasta => {
                if fasta_is_nucleic_acid(file)? {
                    refs.nucleotide.push(load_nucl_fasta(file)?);
                } else {
                    fasta_files.push(file.clone());
                }
            }
        }
    }

    // Protein HMM files: one hmmsearch db per file (matches load_hmm_dbs).
    if !hmm_files.is_empty() {
        refs.protein.extend(load_hmm_dbs(&hmm_files)?);
    }
    // Protein FASTA files: one phmmer db per file.
    for f in &fasta_files {
        refs.protein.push(load_phmmer_db(f)?);
    }

    Ok(refs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_protein_vs_nucleotide_by_extension_and_alphabet() {
        assert!(matches!(ref_file_type("x.hmm"), Ok(RefFileType::Hmm)));
        assert!(matches!(ref_file_type("x.hmmer"), Ok(RefFileType::Hmm)));
        assert!(matches!(ref_file_type("x.fasta"), Ok(RefFileType::Fasta)));
        assert!(matches!(ref_file_type("x.fna"), Ok(RefFileType::Fasta)));
        assert!(matches!(ref_file_type("x.faa"), Ok(RefFileType::Fasta)));
        assert!(matches!(ref_file_type("x.cm"), Ok(RefFileType::Cm)));
        assert!(ref_file_type("x.gb").is_err());
        // A compression suffix is looked through, never mistaken for the type.
        assert!(matches!(ref_file_type("x.hmm.gz"), Ok(RefFileType::Hmm)));
        assert!(matches!(ref_file_type("x.HMM.BGZ"), Ok(RefFileType::Hmm)));
        assert!(matches!(
            ref_file_type("x.fasta.gz"),
            Ok(RefFileType::Fasta)
        ));
        assert!(ref_file_type("x.gz").is_err());
    }

    /// A deterministic DNA sequence of `len` bases, all four of them present.
    fn dna(len: usize) -> Vec<u8> {
        (0..len)
            .map(|i| b"ATGCGTACCTGA"[(i * 7 + i / 5) % 12])
            .collect()
    }

    /// `dna(len)` with every `step`th base turned into an `R` or `Y`.
    fn ambiguous_dna(len: usize, step: usize) -> Vec<u8> {
        let mut s = dna(len);
        for (k, i) in (0..len).step_by(step).enumerate() {
            s[i] = if k % 2 == 0 { b'R' } else { b'Y' };
        }
        s
    }

    const PROTEIN: &[u8] = b"MSKGEELFTGVVPILVELDGDVNGHKFSVSGEGEGDATYGKLTLKFICTTGKLPVPWPTLVTTF";

    #[test]
    fn easel_guess_matches_easel_on_the_report_cases() {
        let guess = |s: &[u8]| easel_guess_alphabet(&letter_counts(s, EASEL_GUESS_WINDOW));
        assert_eq!(guess(&dna(642)), Some(EaselGuess::Dna));
        assert_eq!(guess(&dna(642).to_ascii_lowercase()), Some(EaselGuess::Dna));
        assert_eq!(guess(PROTEIN), Some(EaselGuess::Amino));
        // Easel gives up on DNA that is ~5% ambiguity codes, and on <= 10 residues.
        assert_eq!(guess(&ambiguous_dna(642, 20)), None);
        assert_eq!(guess(b"ACGTACGTAC"), None);
        // An all-N run longer than 2000 is DNA.
        assert_eq!(guess(&[b'N'; 2001]), Some(EaselGuess::Dna));
        let rna: Vec<u8> = dna(300)
            .iter()
            .map(|&c| if c == b'T' { b'U' } else { c })
            .collect();
        assert_eq!(guess(&rna), Some(EaselGuess::Rna));
    }

    #[test]
    fn iupac_dna_is_nucleotide_and_never_protein() {
        assert_eq!(classify_record(&dna(642)), RecordAlphabet::Nucleotide);
        // ~5% and ~15% R/Y: Easel cannot decide, the IUPAC fallback can.
        assert_eq!(
            classify_record(&ambiguous_dna(642, 20)),
            RecordAlphabet::Nucleotide
        );
        assert_eq!(
            classify_record(&ambiguous_dna(642, 7)),
            RecordAlphabet::Nucleotide
        );
        assert_eq!(classify_record(PROTEIN), RecordAlphabet::Protein);
        assert_eq!(classify_record(b"MKV"), RecordAlphabet::Unknown);
    }

    #[test]
    fn dna_with_amino_only_letters_is_reported() {
        let mut one = dna(642);
        one[300] = b'E';
        assert_eq!(
            classify_record(&one),
            RecordAlphabet::NucleotideWithAminoLetters("E".into())
        );
        let mut two = dna(642);
        two[300] = b'E';
        two[301] = b'Q';
        assert_eq!(
            classify_record(&two),
            RecordAlphabet::NucleotideWithAminoLetters("EQ".into())
        );
    }

    /// Scratch directory for file-level tests, keyed by pid and test name.
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "domainatoredux-references-{}-{name}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_fasta(dir: &Path, name: &str, records: &[(&str, &[u8])]) -> String {
        let mut text = Vec::new();
        for (id, seq) in records {
            text.extend_from_slice(format!(">{id}\n").as_bytes());
            text.extend_from_slice(seq);
            text.push(b'\n');
        }
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        path.to_string_lossy().into_owned()
    }

    fn error_of(r: std::io::Result<References>) -> String {
        match r {
            Ok(_) => panic!("expected an error"),
            Err(e) => e.to_string(),
        }
    }

    #[test]
    fn reference_files_are_classified_or_rejected_clearly() {
        let dir = scratch("classify");
        let dna_seq = dna(642);
        let iupac = ambiguous_dna(642, 7);

        let refs = read_references(&[write_fasta(&dir, "d.fasta", &[("d", &dna_seq)])]).unwrap();
        assert_eq!((refs.nucleotide.len(), refs.protein.len()), (1, 0));
        // The model has one node per base: nothing was dropped.
        assert_eq!(refs.nucleotide[0].hmms[0].m, 642);

        let refs = read_references(&[write_fasta(&dir, "i.fasta", &[("i", &iupac)])]).unwrap();
        assert_eq!((refs.nucleotide.len(), refs.protein.len()), (1, 0));

        let refs = read_references(&[write_fasta(&dir, "p.fasta", &[("p", PROTEIN)])]).unwrap();
        assert_eq!((refs.nucleotide.len(), refs.protein.len()), (0, 1));

        for (name, records) in [
            ("pd.fasta", [("p", PROTEIN), ("d", &dna_seq[..])]),
            ("dp.fasta", [("d", &dna_seq[..]), ("p", PROTEIN)]),
        ] {
            let msg = error_of(read_references(&[write_fasta(&dir, name, &records)]));
            assert!(
                msg.contains("mixes nucleotide (record 'd') and protein (record 'p')"),
                "{msg}"
            );
        }

        for letters in [&b"E"[..], b"EE"] {
            let mut s = dna(642);
            s.splice(300..300, letters.iter().copied());
            let msg = error_of(read_references(&[write_fasta(
                &dir,
                "e.fasta",
                &[("e", &s)],
            )]));
            assert!(msg.contains("amino-acid-only letters (E)"), "{msg}");
        }

        let msg = error_of(read_references(&[write_fasta(
            &dir,
            "s.fasta",
            &[("s", b"ACGT")],
        )]));
        assert!(msg.contains("Could not determine alphabet"), "{msg}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_letter_the_dna_alphabet_cannot_read_is_an_error_not_a_panic() {
        // Easel reads the window as DNA; the `J`s past it are illegal there.
        let dir = scratch("illegal");
        let mut s = dna(5000);
        s[4500] = b'J';
        s[4600] = b'J';
        let file = write_fasta(&dir, "j.fasta", &[("j", &s)]);
        let msg = match load_nucl_fasta(&file) {
            Ok(_) => panic!("expected an error"),
            Err(e) => e.to_string(),
        };
        assert!(msg.contains("record 'j'") && msg.contains("'J'"), "{msg}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
