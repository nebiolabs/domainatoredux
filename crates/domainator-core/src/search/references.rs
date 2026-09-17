//! Reference-file classification (port of `domainate.read_references`).
//!
//! Each `-r` file is classified by extension + alphabet into a search group:
//! - protein `.hmm`  -> hmmsearch  (protein peptides)
//! - protein FASTA   -> phmmer     (protein peptides; single-seq profile per ref)
//! - nucleotide `.hmm` / FASTA -> nhmmer (whole nucleotide contigs, both strands)
//! - `.cm` (infernal) -> error (deferred)
//!
//! Nucleotide references combined with protein input is an error, matching
//! `validate_input_molecule_types`.

use std::path::Path;

use super::hmmer::{load_hmm_dbs, load_phmmer_db, HmmDb};
use super::nhmmer::NuclDb;

/// Classified references: protein dbs (hmmsearch + phmmer) and nucleotide dbs
/// (nhmmer). Empty is legal (no references).
#[derive(Default)]
pub struct References {
    pub protein: Vec<HmmDb>,
    pub nucleotide: Vec<NuclDb>,
}

impl References {
    /// `true` if any nucleotide (nhmmer) reference was loaded — protein input with
    /// these is an error.
    pub fn has_nucleotide(&self) -> bool {
        !self.nucleotide.is_empty()
    }
}

/// File "type" by extension, mirroring `utils.get_file_type` for reference files.
enum RefFileType {
    Hmm,
    Fasta,
    Cm,
}

fn ref_file_type(file: &str) -> Result<RefFileType, String> {
    let ext = Path::new(file)
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "hmm" | "hmmer" => Ok(RefFileType::Hmm),
        "fasta" | "fa" | "fna" | "pep" | "faa" => Ok(RefFileType::Fasta),
        "cm" => Ok(RefFileType::Cm),
        _ => Err(format!(
            "Reference files must be fasta, hmmer, or infernal cm (got {file:?})"
        )),
    }
}

/// Whether a FASTA's first sequence is nucleic-acid (mirrors easel's alphabet
/// guess used by `_fasta_is_nucleic_acid`): the residues are overwhelmingly
/// A/C/G/T/U/N.
fn fasta_is_nucleic_acid(file: &str) -> std::io::Result<bool> {
    let recs = crate::io::fasta::read_fasta(file, Some("protein"))?;
    let Some(rec) = recs.first() else {
        return Ok(false);
    };
    let mut nuc = 0usize;
    let mut total = 0usize;
    for &b in &rec.seq {
        if !b.is_ascii_alphabetic() {
            continue;
        }
        total += 1;
        if matches!(b.to_ascii_uppercase(), b'A' | b'C' | b'G' | b'T' | b'U' | b'N') {
            nuc += 1;
        }
    }
    if total == 0 {
        return Ok(false);
    }
    // Easel guesses nucleic acid when the canonical DNA/RNA symbols dominate.
    Ok(nuc as f64 / total as f64 >= 0.9)
}

/// Whether an `.hmm` file's first profile is a DNA/RNA model.
fn hmm_is_nucleic_acid(file: &str) -> std::io::Result<bool> {
    use hmmer_pure_rs::alphabet::AlphabetType;
    let hmms = crate::search::hmm_read::read_hmm_file(Path::new(file))
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{e:?}")))?;
    Ok(hmms
        .first()
        .map(|h| matches!(h.abc_type, AlphabetType::Dna | AlphabetType::Rna))
        .unwrap_or(false))
}

/// Load a nucleotide `.hmm` file as an nhmmer db (file stem = db name).
fn load_nucl_hmm(file: &str) -> std::io::Result<NuclDb> {
    let hmms = crate::search::hmm_read::read_hmm_file(Path::new(file))
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{e:?}")))?;
    Ok(NuclDb { name: file_stem(file), hmms })
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
    let recs = crate::io::fasta::read_fasta(file, Some("nucleotide"))?;
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
        let dsq = abc.digitize(&residues);
        let mut hmm = hmmer_pure_rs::seqmodel::build_single_seq_hmm(
            &id,
            &dsq,
            residues.len(),
            &abc,
            &bg,
            NUCL_POPEN,
            NUCL_PEXTEND,
        );
        hmm.desc = desc;
        hmms.push(hmm);
    }
    Ok(NuclDb { name: file_stem(file), hmms })
}

fn file_stem(f: &str) -> String {
    Path::new(f)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("references")
        .to_string()
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
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("infernal .cm references are not yet supported ({file:?})"),
                ));
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
    }
}
