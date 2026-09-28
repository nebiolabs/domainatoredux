//! Translation and sequence extraction (lifted from the prototype `parser.rs`).
//!
//! Codon-by-codon translation with the NCBI standard table (table 1), matching
//! Biopython's behavior so peptide lengths — and therefore hit coordinates —
//! stay consistent.

use super::location::Location;
use super::SeqFeature;

/// Translate one codon (NCBI table 1). Ambiguous/unknown base -> `X`; `*` is stop.
pub fn translate_codon(c0: u8, c1: u8, c2: u8) -> u8 {
    fn idx(b: u8) -> Option<usize> {
        match b.to_ascii_uppercase() {
            b'T' | b'U' => Some(0),
            b'C' => Some(1),
            b'A' => Some(2),
            b'G' => Some(3),
            _ => None,
        }
    }
    // TCAG-ordered standard genetic code.
    const TABLE: &[u8; 64] = b"FFLLSSSSYY**CC*WLLLLPPPPHHQQRRRRIIIMTTTTNNKKSSRRVVVVAAAADDEEGGGG";
    match (idx(c0), idx(c1), idx(c2)) {
        (Some(a), Some(b), Some(c)) => TABLE[a * 16 + b * 4 + c],
        _ => b'X',
    }
}

/// IUPAC reverse-complement, preserving case (matches Biopython's complement).
pub fn revcomp(seq: &[u8]) -> Vec<u8> {
    seq.iter()
        .rev()
        .map(|&b| match b {
            b'A' => b'T',
            b'T' => b'A',
            b'U' => b'A',
            b'G' => b'C',
            b'C' => b'G',
            b'a' => b't',
            b't' => b'a',
            b'u' => b'a',
            b'g' => b'c',
            b'c' => b'g',
            b'R' => b'Y',
            b'Y' => b'R',
            b'S' => b'S',
            b'W' => b'W',
            b'K' => b'M',
            b'M' => b'K',
            b'r' => b'y',
            b'y' => b'r',
            b's' => b's',
            b'w' => b'w',
            b'k' => b'm',
            b'm' => b'k',
            b'B' => b'V',
            b'V' => b'B',
            b'D' => b'H',
            b'H' => b'D',
            b'b' => b'v',
            b'v' => b'b',
            b'd' => b'h',
            b'h' => b'd',
            b'N' => b'N',
            b'n' => b'n',
            other => other,
        })
        .collect()
}

/// IUPAC complement (no reversal), preserving case (matches Biopython's
/// `Seq.complement`). Used by `deduplicate`'s both-strands hashing.
pub fn complement(seq: &[u8]) -> Vec<u8> {
    seq.iter()
        .map(|&b| match b {
            b'A' => b'T',
            b'T' => b'A',
            b'U' => b'A',
            b'G' => b'C',
            b'C' => b'G',
            b'a' => b't',
            b't' => b'a',
            b'u' => b'a',
            b'g' => b'c',
            b'c' => b'g',
            b'R' => b'Y',
            b'Y' => b'R',
            b'S' => b'S',
            b'W' => b'W',
            b'K' => b'M',
            b'M' => b'K',
            b'r' => b'y',
            b'y' => b'r',
            b's' => b's',
            b'w' => b'w',
            b'k' => b'm',
            b'm' => b'k',
            b'B' => b'V',
            b'V' => b'B',
            b'D' => b'H',
            b'H' => b'D',
            b'b' => b'v',
            b'v' => b'b',
            b'd' => b'h',
            b'h' => b'd',
            b'N' => b'N',
            b'n' => b'n',
            other => other,
        })
        .collect()
}

/// Translate a nucleotide sequence to a peptide string (no trailing partial codon).
pub fn translate_dna(seq: &[u8]) -> String {
    let mut out = Vec::with_capacity(seq.len() / 3);
    let mut i = 0;
    while i + 3 <= seq.len() {
        out.push(translate_codon(seq[i], seq[i + 1], seq[i + 2]));
        i += 3;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Extract the spliced nucleotide sequence for `loc` from `contig`, honoring
/// per-part strand (reverse-complementing minus parts) and join order.
pub fn extract_location_seq(loc: &Location, contig: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for &(start, end, strand, _b, _a) in &loc.parts {
        let s = start.max(0) as usize;
        let e = (end.max(0) as usize).min(contig.len());
        if s >= e {
            continue;
        }
        let mut piece = contig[s..e].to_vec();
        if strand == -1 {
            piece = revcomp(&piece);
        }
        out.extend_from_slice(&piece);
    }
    out
}

/// A CDS feature is searchable if it is a non-pseudo CDS.
pub fn is_searchable_cds(feature: &SeqFeature) -> bool {
    feature.kind == "CDS" && !feature.has("pseudo") && !feature.has("pseudogene")
}

/// Alnum-filtered search peptide for a CDS: its `/translation` if present, else the
/// translated spliced nucleotide sequence (matches Python's `get_prot_list`).
pub fn cds_peptide(feature: &SeqFeature, seq: &[u8]) -> String {
    let raw = match feature.get_first("translation").filter(|s| !s.is_empty()) {
        Some(tr) => tr.to_string(),
        None => translate_dna(&extract_location_seq(&feature.location, seq)),
    };
    raw.chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seq::Location;

    #[test]
    fn codon_and_translation() {
        assert_eq!(translate_codon(b'A', b'T', b'G'), b'M');
        assert_eq!(translate_codon(b'T', b'A', b'A'), b'*');
        assert_eq!(translate_codon(b'G', b'G', b'G'), b'G');
        assert_eq!(translate_dna(b"ATGGGGTAA"), "MG*");
        // Trailing partial codon is dropped.
        assert_eq!(translate_dna(b"ATGG"), "M");
    }

    #[test]
    fn revcomp_basic() {
        assert_eq!(revcomp(b"ATGC"), b"GCAT");
        assert_eq!(revcomp(b"AACCCG"), b"CGGGTT");
        // Reverse-complement is an involution.
        assert_eq!(revcomp(&revcomp(b"ACGTACGTA")), b"ACGTACGTA");
    }

    #[test]
    fn extract_location_seq_forward_and_reverse() {
        let contig = b"TTTTAACCCGTTTTTT";
        // Forward [4,10) -> "AACCCG".
        let fwd = Location::range(4, 10, 1);
        assert_eq!(extract_location_seq(&fwd, contig), b"AACCCG");
        // Reverse [4,10) -> revcomp("AACCCG") = "CGGGTT".
        let rev = Location::range(4, 10, -1);
        assert_eq!(extract_location_seq(&rev, contig), b"CGGGTT");
    }
}
