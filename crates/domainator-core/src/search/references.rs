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
    for rec in &recs {
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
        return Ok(nuc as f64 / total as f64 >= 0.9);
    }
    Ok(false)
}

/// Whether an `.hmm` file's first profile is a DNA/RNA model.
fn hmm_is_nucleic_acid(file: &str) -> std::io::Result<bool> {
    use hmmer_pure_rs::alphabet::AlphabetType;
    let hmms = hmmer_pure_rs::hmmfile::read_hmm_file(Path::new(file))
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{e:?}")))?;
    Ok(hmms
        .first()
        .map(|h| matches!(h.abc_type, AlphabetType::Dna | AlphabetType::Rna))
        .unwrap_or(false))
}

/// Load a nucleotide `.hmm` file as an nhmmer db (file stem = db name).
fn load_nucl_hmm(file: &str) -> std::io::Result<NuclDb> {
    let hmms = hmmer_pure_rs::hmmfile::read_hmm_file(Path::new(file))
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{e:?}")))?;
    Ok(NuclDb { name: file_stem(file), hmms })
}

/// Build a nucleotide FASTA nhmmer db: one single-sequence DNA profile per ref
/// sequence (mirrors `read_pyhmmer_fastas` + `pyhmmer.hmmer.nhmmer`).
///
/// The amino single-seq builder (BLOSUM62) can't build DNA models, so we build
/// each profile from a one-row MSA (the DNA build path) and calibrate it, then
/// carry the FASTA id/description as name/desc.
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
        let msa = single_row_msa(&id, &residues);
        let mut hmm = hmmer_pure_rs::builder::build_hmm_from_msa(&msa, &abc, &bg, 0.5, false);
        hmm.name = id;
        hmm.desc = desc;
        hmmer_pure_rs::calibrate::calibrate(&mut hmm, &abc, &bg);
        hmms.push(hmm);
    }
    Ok(NuclDb { name: file_stem(file), hmms })
}

/// A one-sequence, gap-free MSA (all columns become match states) for building a
/// single-sequence profile.
fn single_row_msa(name: &str, residues: &[u8]) -> hmmer_pure_rs::msa::Msa {
    hmmer_pure_rs::msa::Msa {
        name: name.to_string(),
        desc: None,
        author: None,
        sqname: vec![name.to_string()],
        sqdesc: vec![String::new()],
        aseq: vec![residues.to_vec()],
        pp: vec![None],
        nseq: 1,
        alen: residues.len(),
        rf: None,
        pp_cons: None,
    }
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
