//! The GUI's alignment view: a multiple sequence alignment (aligned FASTA) summarized
//! for display, and read back a window at a time.
//!
//! The alignment is read by [`crate::io::alignment::read_alignment`], the same reader
//! `build_tree --msa` uses, so a file the view opens is one the tool accepts.

use serde::Serialize;

use crate::io::alignment::{guess_alphabet, is_gap, Alignment};
use crate::seq::{Annotations, SeqRecord, Topology};

/// What the view draws without fetching the residues: the row names, the size, and
/// one value per column for the tracks above the grid.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AlignmentInfo {
    /// Each sequence's id, in file order.
    pub ids: Vec<String>,
    /// The number of columns.
    pub length: usize,
    /// `"protein"` or `"nucleotide"`, as `build_tree` guesses it.
    pub alphabet: &'static str,
    /// Each column's most common residue (upper case), or `-` for an all-gap column.
    pub consensus: String,
    /// Each column's share of rows holding its consensus residue, in `0..=1`.
    pub conservation: Vec<f32>,
    /// Each column's share of rows holding a residue rather than a gap, in `0..=1`.
    pub occupancy: Vec<f32>,
}

/// The view's summary of `alignment`.
pub fn summarize(alignment: &Alignment) -> AlignmentInfo {
    let rows = alignment.sequences.len().max(1) as f32;
    let mut consensus = String::with_capacity(alignment.length);
    let mut conservation = Vec::with_capacity(alignment.length);
    let mut occupancy = Vec::with_capacity(alignment.length);
    let mut counts = [0usize; 256];
    for col in 0..alignment.length {
        counts.fill(0);
        let mut residues = 0usize;
        for seq in &alignment.sequences {
            let b = seq[col];
            if !is_gap(b) {
                residues += 1;
                counts[usize::from(b.to_ascii_uppercase())] += 1;
            }
        }
        // The most common residue; ties go to the earlier byte, so the choice is
        // stable.
        let (best, best_count) =
            counts
                .iter()
                .enumerate()
                .fold(
                    (b'-', 0usize),
                    |(bb, bc), (b, &c)| {
                        if c > bc {
                            (b as u8, c)
                        } else {
                            (bb, bc)
                        }
                    },
                );
        consensus.push(char::from(best));
        conservation.push(best_count as f32 / rows);
        occupancy.push(residues as f32 / rows);
    }
    AlignmentInfo {
        ids: alignment.ids.clone(),
        length: alignment.length,
        alphabet: guess_alphabet(alignment.sequences.iter().map(Vec::as_slice)),
        consensus,
        conservation,
        occupancy,
    }
}

/// The residues of rows `row_start..row_start + row_count` in columns
/// `col_start..col_start + col_count`, clipped to the alignment.
pub fn window(
    alignment: &Alignment,
    row_start: usize,
    row_count: usize,
    col_start: usize,
    col_count: usize,
) -> Vec<String> {
    let col_start = col_start.min(alignment.length);
    let col_end = col_start.saturating_add(col_count).min(alignment.length);
    alignment
        .sequences
        .iter()
        .skip(row_start)
        .take(row_count)
        .map(|seq| String::from_utf8_lossy(&seq[col_start..col_end]).into_owned())
        .collect()
}

/// The rows named by `ids`, in alignment order, as FASTA records: with their gaps, or
/// without them (the sequences themselves). Ids the alignment lacks are skipped.
pub fn rows_as_records(alignment: &Alignment, ids: &[String], gapped: bool) -> Vec<SeqRecord> {
    let wanted: std::collections::HashSet<&str> = ids.iter().map(String::as_str).collect();
    let molecule = if guess_alphabet(alignment.sequences.iter().map(Vec::as_slice)) == "nucleotide"
    {
        "DNA"
    } else {
        "protein"
    };
    alignment
        .ids
        .iter()
        .zip(&alignment.sequences)
        .filter(|(id, _)| wanted.contains(id.as_str()))
        .map(|(id, seq)| SeqRecord {
            id: id.clone(),
            name: id.clone(),
            description: id.clone(),
            seq: if gapped {
                seq.clone()
            } else {
                seq.iter().copied().filter(|&b| !is_gap(b)).collect()
            },
            molecule_type: Some(molecule.to_string()),
            topology: Topology::Linear,
            annotations: Annotations::default(),
            features: Vec::new(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alignment() -> Alignment {
        Alignment {
            ids: vec!["a".into(), "b".into(), "c".into(), "d".into()],
            sequences: vec![
                b"MK-L".to_vec(),
                b"MR-L".to_vec(),
                b"mK-I".to_vec(),
                b"-K-L".to_vec(),
            ],
            length: 4,
        }
    }

    #[test]
    fn tracks_count_residues_case_blind_and_skip_gaps() {
        let info = summarize(&alignment());
        assert_eq!(info.consensus, "MK-L");
        assert_eq!(info.conservation, [0.75, 0.75, 0.0, 0.75]);
        assert_eq!(info.occupancy, [0.75, 1.0, 0.0, 1.0]);
        assert_eq!(info.alphabet, "protein");
    }

    #[test]
    fn windows_are_clipped_to_the_alignment() {
        let al = alignment();
        assert_eq!(window(&al, 1, 2, 1, 2), ["R-", "K-"]);
        assert_eq!(window(&al, 3, 10, 2, 10), ["-L"]);
        assert!(window(&al, 9, 1, 0, 1).is_empty());
        assert_eq!(window(&al, 0, 1, 9, 1), [""]);
    }

    #[test]
    fn rows_export_with_or_without_gaps_in_alignment_order() {
        let al = alignment();
        let ids = ["d".to_string(), "a".to_string(), "zz".to_string()];
        let gapped = rows_as_records(&al, &ids, true);
        assert_eq!(gapped.len(), 2);
        assert_eq!(
            (gapped[0].id.as_str(), gapped[0].seq.as_slice()),
            ("a", b"MK-L".as_slice())
        );
        let plain = rows_as_records(&al, &ids, false);
        assert_eq!(plain[1].seq, b"KL");
    }
}
