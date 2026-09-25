//! `clean_sequences`: clean/filter GenBank or FASTA records — make names
//! newick-safe, deduplicate names, strip non-canonical residues from the ends,
//! and/or drop sequences containing non-canonical residues. Port of
//! `domainator.clean_sequences.clean_sequences`.

use std::collections::HashMap;

use crate::seq::location::{CompoundOp, Location};
use crate::seq::{SeqFeature, SeqRecord};

/// The 20 canonical amino acids.
const CANONICAL_PROTEIN: &[u8] = b"ACDEFGHIKLMNPQRSTVWY";
/// Unambiguous nucleotides (DNA + RNA): A C G T U.
const CANONICAL_NUCLEOTIDE: &[u8] = b"ACGTU";

/// Statistics tracked during a cleaning run (mirrors `CleaningStats`).
#[derive(Default, Debug, Clone)]
pub struct CleaningStats {
    pub input_count: usize,
    pub output_count: usize,
    pub filtered_count: usize,
    pub empty_after_strip_count: usize,
    pub names_changed_count: usize,
    pub sequences_changed_count: usize,
}

/// Options for [`clean_sequences`].
#[derive(Default)]
pub struct CleanSequencesOptions {
    pub fasta_type: String,
    pub clean_name: bool,
    pub deduplicate_names: bool,
    pub strip: bool,
    pub filter_by_aa: bool,
}

fn is_canonical(b: u8, canonical: &[u8]) -> bool {
    let u = b.to_ascii_uppercase();
    u != b'*' && canonical.contains(&u)
}

/// Newick-incompatible characters replaced with `_` (` ;:,()'"`).
fn clean_name_for_newick(name: &str) -> String {
    const BAD: &[char] = &[' ', ';', ':', ',', '(', ')', '\'', '"'];
    name.chars().map(|c| if BAD.contains(&c) { '_' } else { c }).collect()
}

/// `[start, end)` bounds after trimming non-canonical residues (and `*`) from
/// both ends. `start == end` means the whole sequence was non-canonical.
fn strip_bounds(seq: &[u8], canonical: &[u8]) -> (usize, usize) {
    let mut start = 0;
    while start < seq.len() && !is_canonical(seq[start], canonical) {
        start += 1;
    }
    let mut end = seq.len();
    while end > start && !is_canonical(seq[end - 1], canonical) {
        end -= 1;
    }
    (start, end)
}

fn has_non_canonical(seq: &[u8], canonical: &[u8]) -> bool {
    seq.iter().any(|&b| !is_canonical(b, canonical))
}

/// Ensure `name` is unique. On a collision, append `_{count}` (incrementing the
/// per-base counter), matching `deduplicate_name`.
fn deduplicate_name(name: &str, seen: &mut HashMap<String, usize>) -> String {
    let mut candidate = name.to_string();
    loop {
        match seen.get(&candidate).copied() {
            None => {
                seen.insert(candidate.clone(), 1);
                return candidate;
            }
            Some(count) => {
                seen.insert(candidate.clone(), count + 1);
                candidate = format!("{candidate}_{count}");
            }
        }
    }
}

/// Linear slice `[start, end)` keeping (and truncating) all overlapping features,
/// shifting coordinates by `-start`. Mirrors `utils.slice_record` with
/// `truncate_features=True` for the simple forward-strand case used by `--strip`.
fn slice_linear(rec: &SeqRecord, start: i64, end: i64) -> SeqRecord {
    let mut features = Vec::new();
    for f in &rec.features {
        let mut parts: Vec<(i64, i64, i32, bool, bool)> = Vec::new();
        for &(s0, e0, strand, before, after) in &f.location.parts {
            let (s, e) = if s0 <= e0 { (s0, e0) } else { (e0, s0) };
            let ns = s.max(start);
            let ne = e.min(end);
            if ns < ne {
                parts.push((ns - start, ne - start, strand, before, after));
            }
        }
        if parts.is_empty() {
            continue;
        }
        parts.sort_by_key(|p| p.0);
        let operator = if parts.len() > 1 { f.location.operator.or(Some(CompoundOp::Join)) } else { None };
        features.push(SeqFeature {
            kind: f.kind.clone(),
            location: Location { operator, between: false, parts },
            qualifiers: f.qualifiers.clone(),
        });
    }
    SeqRecord {
        seq: rec.seq[start as usize..end as usize].to_vec(),
        features,
        ..rec.clone()
    }
}

/// Clean/filter `records`, updating `stats`. Returns the surviving records.
pub fn clean_sequences(
    records: Vec<SeqRecord>,
    opts: &CleanSequencesOptions,
    stats: &mut CleaningStats,
) -> Vec<SeqRecord> {
    let canonical: &[u8] =
        if opts.fasta_type == "protein" { CANONICAL_PROTEIN } else { CANONICAL_NUCLEOTIDE };
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut out = Vec::new();

    for mut contig in records {
        stats.input_count += 1;
        let original_name = contig.id.clone();
        let original_seq = contig.seq.clone();

        if opts.strip {
            let (s, e) = strip_bounds(&contig.seq, canonical);
            if s != 0 || e != contig.seq.len() {
                contig = slice_linear(&contig, s as i64, e as i64);
            }
        }

        if opts.filter_by_aa && has_non_canonical(&contig.seq, canonical) {
            stats.filtered_count += 1;
            continue;
        }

        if contig.seq.is_empty() {
            stats.empty_after_strip_count += 1;
            continue;
        }

        if opts.clean_name {
            contig.name = clean_name_for_newick(&contig.name);
            contig.id = clean_name_for_newick(&contig.id);
        }

        if opts.deduplicate_names {
            let new_name = deduplicate_name(&contig.id, &mut seen);
            if new_name != contig.id {
                contig.name = new_name.clone();
                contig.id = new_name;
            }
        }

        if contig.id != original_name {
            stats.names_changed_count += 1;
        }
        if contig.seq != original_seq {
            stats.sequences_changed_count += 1;
        }
        stats.output_count += 1;
        out.push(contig);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seq::{Annotations, Topology};

    fn rec(name: &str, seq: &[u8]) -> SeqRecord {
        SeqRecord {
            id: name.into(),
            name: name.into(),
            description: String::new(),
            seq: seq.to_vec(),
            molecule_type: Some("protein".into()),
            topology: Topology::Linear,
            annotations: Annotations::default(),
            features: Vec::new(),
        }
    }

    #[test]
    fn strip_and_filter() {
        let recs = vec![rec("a", b"**ACDEFXX"), rec("b", b"XXXX")];
        let opts = CleanSequencesOptions { fasta_type: "protein".into(), strip: true, ..Default::default() };
        let mut st = CleaningStats::default();
        let out = clean_sequences(recs, &opts, &mut st);
        // "a" strips to "ACDEF"; "b" is all non-canonical -> empty -> dropped.
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].seq, b"ACDEF");
        assert_eq!(st.empty_after_strip_count, 1);
    }

    #[test]
    fn filter_by_aa_drops_noncanonical() {
        let recs = vec![rec("a", b"ACDEF"), rec("b", b"ACXEF")];
        let opts =
            CleanSequencesOptions { fasta_type: "protein".into(), filter_by_aa: true, ..Default::default() };
        let mut st = CleaningStats::default();
        let out = clean_sequences(recs, &opts, &mut st);
        assert_eq!(out.len(), 1);
        assert_eq!(st.filtered_count, 1);
    }

    #[test]
    fn clean_and_dedup_names() {
        let recs = vec![rec("a b(c)", b"ACDEF"), rec("x", b"ACDEF"), rec("x", b"GGGGG")];
        let opts = CleanSequencesOptions {
            fasta_type: "protein".into(),
            clean_name: true,
            deduplicate_names: true,
            ..Default::default()
        };
        let mut st = CleaningStats::default();
        let out = clean_sequences(recs, &opts, &mut st);
        assert_eq!(out[0].name, "a_b_c_");
        assert_eq!(out[1].name, "x");
        assert_eq!(out[2].name, "x_1");
    }
}
