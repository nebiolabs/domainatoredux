//! Aligned FASTA: a multiple sequence alignment, one gapped sequence per record.
//!
//! Read the way upstream's `build_tree.read_msa` reads one (Biopython's FASTA parser,
//! gaps kept), and shared by `build_tree --msa` and the GUI's alignment view.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::Path;

use crate::io::fasta::read_fasta_decompressed;

/// Residues other than these (and gaps) in at least this fraction of an alignment's
/// residues make it protein rather than nucleotide.
const NUCLEOTIDE_LETTERS: &[u8] = b"ACGTUN";
const NUCLEOTIDE_MIN_FRACTION: f64 = 0.9;

/// Whether `b` is a gap character (`-` or `.`).
pub fn is_gap(b: u8) -> bool {
    matches!(b, b'-' | b'.')
}

/// A multiple sequence alignment: every sequence has the same length.
#[derive(Clone, Debug, PartialEq)]
pub struct Alignment {
    /// Each sequence's id (the first word of its FASTA title), in file order.
    pub ids: Vec<String>,
    /// Each aligned sequence, gaps included, in file order.
    pub sequences: Vec<Vec<u8>>,
    /// The number of columns.
    pub length: usize,
}

fn invalid(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

/// Read an aligned FASTA file (plain or gzip-compressed). Refuses a file with no
/// sequences, a repeated id, or sequences of different lengths.
pub fn read_alignment(path: &Path) -> io::Result<Alignment> {
    let display = path.display();
    let records = read_fasta_decompressed(&path.to_string_lossy(), None)?;
    let mut seen = HashSet::new();
    for record in &records {
        if !seen.insert(record.id.as_str()) {
            return Err(invalid(format!(
                "The alignment {display} has more than one sequence named {}.",
                record.id
            )));
        }
    }
    if records.is_empty() {
        return Err(invalid(format!(
            "The alignment {display} has no sequences."
        )));
    }
    let lengths: Vec<usize> = records.iter().map(|r| r.seq.len()).collect();
    let (min, max) = (
        *lengths.iter().min().unwrap_or(&0),
        *lengths.iter().max().unwrap_or(&0),
    );
    if min != max {
        return Err(invalid(format!(
            "The sequences in {display} are not all the same length ({min} to {max}), so \
             they are not an alignment."
        )));
    }
    let (ids, sequences) = records.into_iter().map(|r| (r.id, r.seq)).unzip();
    Ok(Alignment {
        ids,
        sequences,
        length: max,
    })
}

/// `"nucleotide"` when almost every residue is A, C, G, T, U or N, otherwise
/// `"protein"` (`_guess_alphabet`).
pub fn guess_alphabet<'a>(sequences: impl IntoIterator<Item = &'a [u8]>) -> &'static str {
    let mut residues = 0usize;
    let mut nucleotides = 0usize;
    for sequence in sequences {
        for &b in sequence {
            if is_gap(b) {
                continue;
            }
            residues += 1;
            nucleotides += usize::from(NUCLEOTIDE_LETTERS.contains(&b.to_ascii_uppercase()));
        }
    }
    if residues > 0 && nucleotides as f64 >= NUCLEOTIDE_MIN_FRACTION * residues as f64 {
        "nucleotide"
    } else {
        "protein"
    }
}

/// The alignment's sequences for `names`, in that order: `None` for a name it lacks.
/// Also returns warnings about leaves without a sequence and sequences that are not
/// leaves, as upstream's `read_msa` warns.
pub fn sequences_for(
    alignment: &Alignment,
    names: &[String],
    display: &str,
) -> (Vec<Option<String>>, Vec<String>) {
    let by_id: HashMap<&str, &[u8]> = alignment
        .ids
        .iter()
        .map(String::as_str)
        .zip(alignment.sequences.iter().map(Vec::as_slice))
        .collect();
    let wanted: HashSet<&str> = names.iter().map(String::as_str).collect();
    let missing: Vec<&String> = names
        .iter()
        .filter(|n| !by_id.contains_key(n.as_str()))
        .collect();
    let extra: Vec<&String> = alignment
        .ids
        .iter()
        .filter(|id| !wanted.contains(id.as_str()))
        .collect();
    let mut warnings = Vec::new();
    if let Some(first) = missing.first() {
        warnings.push(format!(
            "{} of the tree's {} leaves have no sequence in {display}, for example {first}.",
            missing.len(),
            names.len()
        ));
    }
    if let Some(first) = extra.first() {
        warnings.push(format!(
            "{} sequences in {display} are not leaves of the tree and are left out, for \
             example {first}.",
            extra.len()
        ));
    }
    let sequences = names
        .iter()
        .map(|n| {
            by_id
                .get(n.as_str())
                .map(|s| String::from_utf8_lossy(s).into_owned())
        })
        .collect();
    (sequences, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str, text: &str) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("alignment_{}_{name}.afa", std::process::id()));
        std::fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn reads_an_alignment_and_refuses_what_is_not_one() {
        let path = scratch("ok", ">a desc\nAC-G\nT\n>b\n.ACGT\n");
        let alignment = read_alignment(&path).unwrap();
        assert_eq!(alignment.ids, ["a", "b"]);
        assert_eq!(alignment.sequences[0], b"AC-GT");
        assert_eq!(alignment.length, 5);
        std::fs::remove_file(&path).unwrap();

        for (name, text, message) in [
            ("dup", ">a\nAC\n>a\nAG\n", "more than one sequence named a"),
            (
                "ragged",
                ">a\nAC\n>b\nACG\n",
                "not all the same length (2 to 3)",
            ),
            ("empty", "", "has no sequences"),
        ] {
            let path = scratch(name, text);
            let err = read_alignment(&path).unwrap_err().to_string();
            std::fs::remove_file(&path).unwrap();
            assert!(err.contains(message), "{name}: {err}");
        }
    }

    #[test]
    fn the_alphabet_is_guessed_from_the_residues_not_the_gaps() {
        assert_eq!(guess_alphabet([b"ACGT--acgu".as_slice()]), "nucleotide");
        // 9 of 10 residues are nucleotide letters: still nucleotide.
        assert_eq!(guess_alphabet([b"ACGTACGTAE".as_slice()]), "nucleotide");
        assert_eq!(guess_alphabet([b"ACGTACGTEE".as_slice()]), "protein");
        assert_eq!(guess_alphabet([b"----".as_slice()]), "protein");
    }

    #[test]
    fn sequences_follow_the_leaves_and_report_what_does_not_match() {
        let alignment = Alignment {
            ids: vec!["a".into(), "x".into()],
            sequences: vec![b"A-C".to_vec(), b"AGC".to_vec()],
            length: 3,
        };
        let (sequences, warnings) = sequences_for(&alignment, &["b".into(), "a".into()], "msa.afa");
        assert_eq!(sequences, [None, Some("A-C".to_string())]);
        assert_eq!(warnings.len(), 2);
        assert!(warnings[0].starts_with("1 of the tree's 2 leaves"));
        assert!(warnings[1].contains("for example x"));
    }
}
