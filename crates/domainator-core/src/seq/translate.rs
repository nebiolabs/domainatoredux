//! Translation and sequence extraction (lifted from the prototype `parser.rs`).
//!
//! Codon-by-codon translation with the NCBI standard table (table 1), matching
//! Biopython's behavior so peptide lengths — and therefore hit coordinates —
//! stay consistent.

use super::location::Location;
use super::SeqFeature;

/// Translate one codon (NCBI table 1). Ambiguous/unknown base -> `X`; `*` is stop.
pub fn translate_codon(c0: u8, c1: u8, c2: u8) -> u8 {
    translate_codon_with(STANDARD_CODE, c0, c1, c2)
}

/// The TCAG-ordered standard genetic code (NCBI table 1, and table 11's amino acids).
const STANDARD_CODE: &[u8; 64] =
    b"FFLLSSSSYY**CC*WLLLLPPPPHHQQRRRRIIIMTTTTNNKKSSRRVVVVAAAADDEEGGGG";

/// The NCBI genetic codes Prodigal (and so Orphos) accepts: 1-6, 9-16 and 21-25.
/// Each is the 64 amino acids in TCAG codon order, `*` for stop -- NCBI's `ncbieaa`
/// string, generated from Biopython's `CodonTable.unambiguous_dna_by_id`.
const GENETIC_CODES: [(u8, &[u8; 64]); 19] = [
    (1, STANDARD_CODE),
    (
        2,
        b"FFLLSSSSYY**CCWWLLLLPPPPHHQQRRRRIIMMTTTTNNKKSS**VVVVAAAADDEEGGGG",
    ),
    (
        3,
        b"FFLLSSSSYY**CCWWTTTTPPPPHHQQRRRRIIMMTTTTNNKKSSRRVVVVAAAADDEEGGGG",
    ),
    (
        4,
        b"FFLLSSSSYY**CCWWLLLLPPPPHHQQRRRRIIIMTTTTNNKKSSRRVVVVAAAADDEEGGGG",
    ),
    (
        5,
        b"FFLLSSSSYY**CCWWLLLLPPPPHHQQRRRRIIMMTTTTNNKKSSSSVVVVAAAADDEEGGGG",
    ),
    (
        6,
        b"FFLLSSSSYYQQCC*WLLLLPPPPHHQQRRRRIIIMTTTTNNKKSSRRVVVVAAAADDEEGGGG",
    ),
    (
        9,
        b"FFLLSSSSYY**CCWWLLLLPPPPHHQQRRRRIIIMTTTTNNNKSSSSVVVVAAAADDEEGGGG",
    ),
    (
        10,
        b"FFLLSSSSYY**CCCWLLLLPPPPHHQQRRRRIIIMTTTTNNKKSSRRVVVVAAAADDEEGGGG",
    ),
    (11, STANDARD_CODE),
    (
        12,
        b"FFLLSSSSYY**CC*WLLLSPPPPHHQQRRRRIIIMTTTTNNKKSSRRVVVVAAAADDEEGGGG",
    ),
    (
        13,
        b"FFLLSSSSYY**CCWWLLLLPPPPHHQQRRRRIIMMTTTTNNKKSSGGVVVVAAAADDEEGGGG",
    ),
    (
        14,
        b"FFLLSSSSYYY*CCWWLLLLPPPPHHQQRRRRIIIMTTTTNNNKSSSSVVVVAAAADDEEGGGG",
    ),
    (
        15,
        b"FFLLSSSSYY*QCC*WLLLLPPPPHHQQRRRRIIIMTTTTNNKKSSRRVVVVAAAADDEEGGGG",
    ),
    (
        16,
        b"FFLLSSSSYY*LCC*WLLLLPPPPHHQQRRRRIIIMTTTTNNKKSSRRVVVVAAAADDEEGGGG",
    ),
    (
        21,
        b"FFLLSSSSYY**CCWWLLLLPPPPHHQQRRRRIIMMTTTTNNNKSSSSVVVVAAAADDEEGGGG",
    ),
    (
        22,
        b"FFLLSS*SYY*LCC*WLLLLPPPPHHQQRRRRIIIMTTTTNNKKSSRRVVVVAAAADDEEGGGG",
    ),
    (
        23,
        b"FF*LSSSSYY**CC*WLLLLPPPPHHQQRRRRIIIMTTTTNNKKSSRRVVVVAAAADDEEGGGG",
    ),
    (
        24,
        b"FFLLSSSSYY**CCWWLLLLPPPPHHQQRRRRIIIMTTTTNNKKSSSKVVVVAAAADDEEGGGG",
    ),
    (
        25,
        b"FFLLSSSSYY**CCGWLLLLPPPPHHQQRRRRIIIMTTTTNNKKSSRRVVVVAAAADDEEGGGG",
    ),
];

/// NCBI genetic code `table`, or `None` for an id Prodigal does not support.
pub fn genetic_code(table: u8) -> Option<&'static [u8; 64]> {
    GENETIC_CODES
        .iter()
        .find(|(id, _)| *id == table)
        .map(|(_, code)| *code)
}

/// Translate one codon with `code` (from [`genetic_code`]). Ambiguous/unknown base
/// -> `X`; `*` is stop.
pub fn translate_codon_with(code: &[u8; 64], c0: u8, c1: u8, c2: u8) -> u8 {
    fn idx(b: u8) -> Option<usize> {
        match b.to_ascii_uppercase() {
            b'T' | b'U' => Some(0),
            b'C' => Some(1),
            b'A' => Some(2),
            b'G' => Some(3),
            _ => None,
        }
    }
    match (idx(c0), idx(c1), idx(c2)) {
        (Some(a), Some(b), Some(c)) => code[a * 16 + b * 4 + c],
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
        // Clamped to the contig like a Python slice, so a malformed location (the
        // parser accepts `0..9` and ends past the sequence) yields fewer bases, not a
        // panic.
        let s = usize::try_from(start).unwrap_or(0);
        let e = usize::try_from(end).unwrap_or(0).min(contig.len());
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

/// Alnum-filtered search peptide for a CDS: the on-contig part of its
/// `/translation` if present, else the CDS translated from the contig at its
/// `/codon_start` frame (Python's `get_prot_list` after `clean_rec`, and the native
/// parser's `cds_peptide`). See [`super::partial::on_contig_cds_translation`].
pub fn cds_peptide(feature: &SeqFeature, seq: &[u8]) -> String {
    let (raw, _) = super::partial::on_contig_cds_translation(feature, seq);
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
    fn genetic_codes_differ_where_ncbi_says() {
        let tga = |t: u8| translate_codon_with(genetic_code(t).unwrap(), b'T', b'G', b'A');
        assert_eq!(tga(11), b'*');
        assert_eq!(tga(4), b'W'); // Mycoplasma: the table prodigal's meta mode can pick
        assert_eq!(tga(25), b'G');
        // Prodigal rejects the ids NCBI never assigned.
        for missing in [0, 7, 8, 17, 18, 19, 20, 26] {
            assert!(genetic_code(missing).is_none(), "table {missing}");
        }
        assert_eq!(
            translate_codon_with(genetic_code(11).unwrap(), b'A', b'N', b'G'),
            b'X'
        );
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
