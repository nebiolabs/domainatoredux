//! Protein/nucleotide physico-chemical analyses used by `enum_report`:
//! molecular weight, isoelectric point and net charge (BioPython
//! `SeqUtils.molecular_weight` + `SeqUtils.IsoelectricPoint`/`ProtParam`), and
//! PROSITE-motif counting (`find_features.{prosite_to_regex,search_motif}`).

use regex::RegexBuilder;

/// IUPAC average residue weights (`IUPACData.protein_weights`), only the 20
/// standard letters used by the report.
fn protein_weight(aa: u8) -> Option<f64> {
    Some(match aa {
        b'A' => 89.0932,
        b'C' => 121.1582,
        b'D' => 133.1027,
        b'E' => 147.1293,
        b'F' => 165.1891,
        b'G' => 75.0666,
        b'H' => 155.1546,
        b'I' => 131.1729,
        b'K' => 146.1876,
        b'L' => 131.1729,
        b'M' => 149.2113,
        b'N' => 132.1179,
        b'O' => 255.3134,
        b'P' => 115.1305,
        b'Q' => 146.1445,
        b'R' => 174.201,
        b'S' => 105.0926,
        b'T' => 119.1192,
        b'U' => 168.0532,
        b'V' => 117.1463,
        b'W' => 204.2252,
        b'Y' => 181.1885,
        _ => return None,
    })
}

fn dna_weight(b: u8) -> Option<f64> {
    Some(match b {
        b'A' => 331.2218,
        b'C' => 307.1971,
        b'G' => 347.2212,
        b'T' => 322.2085,
        _ => return None,
    })
}

fn rna_weight(b: u8) -> Option<f64> {
    Some(match b {
        b'A' => 347.2212,
        b'C' => 323.1965,
        b'G' => 363.2206,
        b'U' => 324.1813,
        _ => return None,
    })
}

const WATER: f64 = 18.0153;

/// Sequence type for [`molecular_weight`].
#[derive(Clone, Copy, PartialEq)]
pub enum SeqType {
    Dna,
    Rna,
    Protein,
}

/// `SeqUtils.molecular_weight` (single-stranded, linear, average masses).
/// Returns `None` on any non-unambiguous letter (Python raises `ValueError`,
/// which `enum_report` swallows into an empty cell).
pub fn molecular_weight(seq: &[u8], seq_type: SeqType) -> Option<f64> {
    let table: fn(u8) -> Option<f64> = match seq_type {
        SeqType::Dna => dna_weight,
        SeqType::Rna => rna_weight,
        SeqType::Protein => protein_weight,
    };
    if seq.is_empty() {
        return Some(WATER); // matches Python: 0 - (0-1)*water = water
    }
    let mut sum = 0.0;
    for &b in seq {
        sum += table(b.to_ascii_uppercase())?;
    }
    Some(sum - (seq.len() as f64 - 1.0) * WATER)
}

// --- Isoelectric point / charge (Bjellqvist pKs) ---------------------------

const CHARGED_AAS: [u8; 7] = *b"KRHDECY";

/// Charged-residue counts (+ Nterm/Cterm) and seq-specific N-/C-terminal pKs.
struct ChargeState {
    counts: [(u8, f64); 9], // (aa or pseudo, count); pseudo Nterm=b'1', Cterm=b'2'
    pos_pks: Vec<(usize, f64)>,
    neg_pks: Vec<(usize, f64)>,
}

const NTERM: u8 = 1; // pseudo-residue markers
const CTERM: u8 = 2;

fn build_charge_state(seq: &[u8]) -> ChargeState {
    let mut counts = [(0u8, 0.0f64); 9];
    for (i, &aa) in CHARGED_AAS.iter().enumerate() {
        let c = seq.iter().filter(|&&b| b == aa).count() as f64;
        counts[i] = (aa, c);
    }
    counts[7] = (NTERM, 1.0);
    counts[8] = (CTERM, 1.0);

    // positive_pKs: Nterm 7.5, K 10.0, R 12.0, H 5.98
    let nterm = seq[0];
    let cterm = seq[seq.len() - 1];
    let nterm_pk = match nterm {
        b'A' => 7.59,
        b'M' => 7.0,
        b'S' => 6.93,
        b'P' => 8.36,
        b'T' => 6.82,
        b'V' => 7.44,
        b'E' => 7.7,
        _ => 7.5,
    };
    let cterm_pk = match cterm {
        b'D' => 4.55,
        b'E' => 4.75,
        _ => 3.55,
    };
    // index into `counts` for each charged species.
    let idx = |aa: u8| counts.iter().position(|&(a, _)| a == aa).unwrap();
    let pos_pks = vec![
        (idx(NTERM), nterm_pk),
        (idx(b'K'), 10.0),
        (idx(b'R'), 12.0),
        (idx(b'H'), 5.98),
    ];
    let neg_pks = vec![
        (idx(CTERM), cterm_pk),
        (idx(b'D'), 4.05),
        (idx(b'E'), 4.45),
        (idx(b'C'), 9.0),
        (idx(b'Y'), 10.0),
    ];
    ChargeState {
        counts,
        pos_pks,
        neg_pks,
    }
}

impl ChargeState {
    fn charge_at_ph(&self, ph: f64) -> f64 {
        let mut positive = 0.0;
        for &(i, pk) in &self.pos_pks {
            let partial = 1.0 / (10f64.powf(ph - pk) + 1.0);
            positive += self.counts[i].1 * partial;
        }
        let mut negative = 0.0;
        for &(i, pk) in &self.neg_pks {
            let partial = 1.0 / (10f64.powf(pk - ph) + 1.0);
            negative += self.counts[i].1 * partial;
        }
        positive - negative
    }
}

/// Net charge of a protein at `ph` (`ProtParam.charge_at_pH`). `None` for an
/// empty sequence.
pub fn charge_at_ph(seq: &[u8], ph: f64) -> Option<f64> {
    if seq.is_empty() {
        return None;
    }
    let up: Vec<u8> = seq.iter().map(|b| b.to_ascii_uppercase()).collect();
    Some(build_charge_state(&up).charge_at_ph(ph))
}

/// Isoelectric point (`IsoelectricPoint.pi`, bisection). `None` for empty input.
pub fn isoelectric_point(seq: &[u8]) -> Option<f64> {
    if seq.is_empty() {
        return None;
    }
    let up: Vec<u8> = seq.iter().map(|b| b.to_ascii_uppercase()).collect();
    let state = build_charge_state(&up);
    let (mut min, mut max) = (4.05f64, 12.0f64);
    let mut ph = 7.775f64;
    while max - min > 0.0001 {
        let charge = state.charge_at_ph(ph);
        if charge > 0.0 {
            min = ph;
        } else {
            max = ph;
        }
        ph = (min + max) / 2.0;
    }
    Some(ph)
}

// --- PROSITE motif counting ------------------------------------------------

/// `find_features.prosite_to_regex`: translate a PROSITE pattern to a regex.
pub fn prosite_to_regex(pattern: &str) -> String {
    let pattern: Vec<char> = pattern.trim().trim_end_matches('.').chars().collect();
    let mut result = String::new();
    let mut i = 0;
    while i < pattern.len() {
        let c = pattern[i];
        match c {
            '<' => {
                result.push('^');
                i += 1;
            }
            '>' => {
                result.push('$');
                i += 1;
            }
            '-' => i += 1,
            'x' | 'X' => {
                result.push('.');
                i += 1;
            }
            '[' => {
                if let Some(end) = pattern[i..].iter().position(|&ch| ch == ']') {
                    let end = i + end;
                    let cls: String = pattern[i..=end].iter().collect();
                    result.push_str(&cls.to_uppercase());
                    i = end + 1;
                } else {
                    i += 1;
                }
            }
            '{' => {
                if let Some(end) = pattern[i..].iter().position(|&ch| ch == '}') {
                    let end = i + end;
                    let chars: String = pattern[i + 1..end].iter().collect();
                    result.push_str(&format!("[^{}]", chars.to_uppercase()));
                    i = end + 1;
                } else {
                    i += 1;
                }
            }
            '(' => {
                if let Some(end) = pattern[i..].iter().position(|&ch| ch == ')') {
                    let end = i + end;
                    let rep: String = pattern[i + 1..end].iter().collect();
                    if let Some((n, m)) = rep.split_once(',') {
                        result.push_str(&format!("{{{},{}}}", n.trim(), m.trim()));
                    } else {
                        result.push_str(&format!("{{{}}}", rep.trim()));
                    }
                    i = end + 1;
                } else {
                    i += 1;
                }
            }
            c if c.is_alphabetic() => {
                result.push(c.to_ascii_uppercase());
                i += 1;
            }
            _ => i += 1,
        }
    }
    result
}

/// Count non-overlapping occurrences of a PROSITE motif in `sequence`
/// (`find_features.search_motif`). Trailing `*` stripped; case-insensitive.
/// Invalid patterns yield 0 (Python warns and returns no matches).
pub fn motif_count(sequence: &str, pattern: &str) -> i64 {
    let regex_pattern = prosite_to_regex(pattern);
    let re = match RegexBuilder::new(&regex_pattern)
        .case_insensitive(true)
        .build()
    {
        Ok(re) => re,
        Err(_) => return 0,
    };
    let seq = sequence.to_uppercase();
    let seq = seq.trim_end_matches('*');
    re.find_iter(seq).count() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() < tol
    }

    #[test]
    fn molecular_weight_matches_biopython() {
        // Values from BioPython's SeqUtils.molecular_weight docstring.
        assert!(close(
            molecular_weight(b"AGC", SeqType::Dna).unwrap(),
            949.61,
            0.01
        ));
        assert!(close(
            molecular_weight(b"AGC", SeqType::Rna).unwrap(),
            997.61,
            0.01
        ));
        assert!(close(
            molecular_weight(b"AGC", SeqType::Protein).unwrap(),
            249.29,
            0.01
        ));
        // Ambiguous letters are rejected (DNA table has no N).
        assert!(molecular_weight(b"AGCN", SeqType::Dna).is_none());
    }

    #[test]
    fn isoelectric_point_and_charge_match_biopython() {
        // BioPython IsoelectricPoint/ProtParam docstrings.
        assert!(close(isoelectric_point(b"INGAR").unwrap(), 9.75, 0.02));
        assert!(close(isoelectric_point(b"PETER").unwrap(), 4.53, 0.02));
        assert!(close(charge_at_ph(b"INGAR", 7.0).unwrap(), 0.76, 0.02));
        // At its pI a protein's net charge is ~0.
        let pi = isoelectric_point(b"PETER").unwrap();
        assert!(close(charge_at_ph(b"PETER", pi).unwrap(), 0.0, 0.01));
        assert!(isoelectric_point(b"").is_none());
    }

    #[test]
    fn prosite_translation_and_counting() {
        assert_eq!(prosite_to_regex("M-A"), "MA");
        assert_eq!(prosite_to_regex("[AG]-x-[DE]"), "[AG].[DE]");
        assert_eq!(prosite_to_regex("<A-x(2)-{P}>"), "^A.{2}[^P]$");
        // Non-overlapping, case-insensitive, trailing '*' stripped.
        assert_eq!(motif_count("MAMAMA", "M-A"), 3);
        assert_eq!(motif_count("mama*", "M-A"), 2);
        assert_eq!(motif_count("AADGE", "[AG]-x-[DE]"), 1);
    }
}
