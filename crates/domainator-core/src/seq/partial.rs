//! Partial features and fragments: the GenBank `<`/`>` markers read as 5'/3'
//! partial ends, the reading-frame offset from `/codon_start`, the on-contig
//! trimming of a partial CDS's translation, and the fragment status that the
//! `--partial include|exclude|only` filters select on.
//!
//! Ports of upstream `utils.py`'s `location_partial_ends`, `partial_ends_to_status`,
//! `get_fragment_status`, `description_is_fragment`, `contig_has_fragment`,
//! `cds_fragment_status`, `fragment_status_allowed`, `codon_offset`,
//! `on_contig_translation` and `normalize_cds_translation` (`8bf3c42`, `bdc74bb`).

use std::str::FromStr;

use super::location::Location;
use super::translate::{extract_location_seq, translate_dna};
use super::{SeqFeature, SeqRecord};

/// UniProt FASTA headers flag partial proteins with one of these tokens.
pub const FRAGMENT_TOKENS: [&str; 2] = [" (Fragment)", " (Fragments)"];
/// UniProt text (`.dat`) entries flag them in a `Flags:` DE line (e.g. `Flags:
/// Precursor; Fragments;`), which ends up in the DEFINITION of a converted GenBank file.
pub const FRAGMENT_FLAGS_PREFIX: &str = "Flags:";
/// UniProt marks a missing terminus with a `NON_TER` feature on the first or last residue.
pub const NON_TERMINAL_FEATURE_NAME: &str = "NON_TER";
/// The values `--partial` accepts.
pub const PARTIAL_CHOICES: [&str; 3] = ["include", "exclude", "only"];

/// The `--partial` selection: which proteins/CDSs pass, by fragment status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Partial {
    /// Everything (the default).
    #[default]
    Include,
    /// Complete proteins/CDSs only.
    Exclude,
    /// Fragments only.
    Only,
}

impl Partial {
    /// Whether a protein/CDS with fragment status `status` (`None` = complete)
    /// passes (`utils.fragment_status_allowed`).
    pub fn allows(self, status: Option<&str>) -> bool {
        match self {
            Partial::Include => true,
            Partial::Exclude => status.is_none(),
            Partial::Only => status.is_some(),
        }
    }

    /// True for the default, which filters nothing (lets callers skip the work).
    pub fn is_include(self) -> bool {
        self == Partial::Include
    }
}

impl FromStr for Partial {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "include" => Ok(Partial::Include),
            "exclude" => Ok(Partial::Exclude),
            "only" => Ok(Partial::Only),
            other => Err(format!(
                "partial must be one of {}, not {other}",
                PARTIAL_CHOICES.join(", ")
            )),
        }
    }
}

/// `(five_prime_partial, three_prime_partial)` in transcript orientation: a `<`/`>`
/// on the outer end of the first part (5') or last part (3').
///
/// `before`/`after` always mark the low/high coordinate, so on the minus strand
/// the 5' end is the first part's `after` and the 3' end the last part's `before`.
pub fn location_partial_ends(location: &Location) -> (bool, bool) {
    let (Some(first), Some(last)) = (location.parts.first(), location.parts.last()) else {
        return (false, false);
    };
    let five = if first.2 == -1 { first.4 } else { first.3 };
    let three = if last.2 == -1 { last.3 } else { last.4 };
    (five, three)
}

/// Partial end flags as a fragment status: `None` (complete), `N`, `C` or `NC`.
pub fn partial_ends_to_status(five_prime: bool, three_prime: bool) -> Option<&'static str> {
    match (five_prime, three_prime) {
        (false, false) => None,
        (true, false) => Some("N"),
        (false, true) => Some("C"),
        (true, true) => Some("NC"),
    }
}

/// Fragment status of a feature, from its location's partial ends; failing those, a
/// legacy bare `/partial` (no value) gives `?`: partial, missing end unknown
/// (upstream `02a31c1`).
pub fn feature_fragment_status(feature: &SeqFeature) -> Option<&'static str> {
    let (five, three) = location_partial_ends(&feature.location);
    partial_ends_to_status(five, three).or_else(|| has_bare_partial(feature).then_some("?"))
}

/// The legacy INSDC `/partial` qualifier with no value (`lean_record.has_bare_partial`).
fn has_bare_partial(feature: &SeqFeature) -> bool {
    feature
        .get_first(PARTIAL_QUALIFIER)
        .is_some_and(|v| v.trim().trim_matches('"').is_empty())
}

/// True if a description carries a UniProt fragment marker: ` (Fragment)` or
/// ` (Fragments)` (FASTA headers), or a `Flags:` list naming `Fragment(s)` (text entries).
pub fn description_is_fragment(description: &str) -> bool {
    if FRAGMENT_TOKENS.iter().any(|t| description.contains(t)) {
        return true;
    }
    description
        .find(FRAGMENT_FLAGS_PREFIX)
        .is_some_and(|i| description[i..].contains("Fragment"))
}

/// Fragment status of a protein record (`utils.get_fragment_status` on a record):
///
/// 1. a `CDS` or `Protein` feature spanning the record decides, from its markers
///    (what `domain_search --translate` writes and GenPept carries);
/// 2. otherwise `NON_TER` features on the first/last residue give `N`/`C`/`NC`;
/// 3. otherwise a fragment marker in the description gives `?`.
///
/// `None` for a complete protein and for every nucleotide record, whose status is per CDS.
pub fn record_fragment_status(record: &SeqRecord) -> Option<&'static str> {
    if !record.is_protein() {
        return None;
    }
    let len = record.seq.len() as i64;
    for f in &record.features {
        if (f.kind == "CDS" || f.kind == "Protein")
            && f.location.start() == 0
            && f.location.end() >= len
        {
            return feature_fragment_status(f);
        }
    }
    let (mut missing_n, mut missing_c) = (false, false);
    for f in &record.features {
        if f.kind == NON_TERMINAL_FEATURE_NAME {
            missing_n |= f.location.start() == 0;
            missing_c |= f.location.end() >= len;
        }
    }
    if missing_n || missing_c {
        return partial_ends_to_status(missing_n, missing_c);
    }
    if description_is_fragment(&record.description) {
        return Some("?");
    }
    None
}

/// The fragment status of a CDS on `record`: the record's own status for a protein
/// record (or when there is no feature), otherwise the CDS feature's.
pub fn cds_fragment_status(
    record: &SeqRecord,
    feature: Option<&SeqFeature>,
) -> Option<&'static str> {
    match feature {
        Some(f) if !record.is_protein() => feature_fragment_status(f),
        _ => record_fragment_status(record),
    }
}

/// Contig-level fragment test (`utils.contig_has_fragment`): a protein record by its
/// own status, a nucleotide contig if any of its CDSs has a partial end.
pub fn contig_has_fragment(record: &SeqRecord) -> bool {
    if record.is_protein() {
        return record_fragment_status(record).is_some();
    }
    record
        .features
        .iter()
        .any(|f| f.kind == "CDS" && feature_fragment_status(f).is_some())
}

/// The 0-based reading-frame offset from `/codon_start` (0 if absent or invalid).
pub fn codon_offset(feature: &SeqFeature) -> i64 {
    match feature
        .get_first("codon_start")
        .map(|v| v.trim().parse::<i64>())
    {
        Some(Ok(n)) if (1..=3).contains(&n) => n - 1,
        _ => 0,
    }
}

/// Python's `s[..end]` for a possibly negative `end`, on chars.
fn py_prefix(chars: &[char], end: i64) -> String {
    let n = chars.len() as i64;
    let end = if end < 0 {
        (n + end).max(0)
    } else {
        end.min(n)
    };
    chars[..usize::try_from(end).unwrap_or(0)].iter().collect()
}

/// Restrict a partial CDS's translation to the residues encoded on the contig.
///
/// Some files give a partial CDS the translation of the full protein. If the
/// translation is longer than the on-contig coding sequence allows, the excess is
/// removed from the partial end. A complete 3' end is assumed to end in a stop codon,
/// which GenBank translations omit and Domainator's own include as `*`. `None` when
/// the excess cannot be placed because both ends are partial.
pub fn on_contig_translation(
    translation: &str,
    location_length: i64,
    offset: i64,
    five_prime_partial: bool,
    three_prime_partial: bool,
) -> Option<String> {
    let chars: Vec<char> = translation.chars().collect();
    let mut expected = (location_length - offset).div_euclid(3);
    if !three_prime_partial && !translation.ends_with('*') {
        expected -= 1; // the stop codon is not in the translation
    }
    let excess = chars.len() as i64 - expected;
    if excess <= 0 || !(five_prime_partial || three_prime_partial) {
        return Some(translation.to_string());
    }
    if five_prime_partial && !three_prime_partial {
        let excess = usize::try_from(excess).unwrap_or(0);
        return Some(chars[excess..].iter().collect());
    }
    if three_prime_partial && !five_prime_partial {
        return Some(py_prefix(&chars, expected));
    }
    None
}

/// Translate a CDS from the contig, starting at its `/codon_start` frame (Biopython's
/// `SeqFeature.translate(cds=False)`, which honors the qualifier).
pub fn translate_feature(feature: &SeqFeature, contig_seq: &[u8]) -> String {
    let nt = extract_location_seq(&feature.location, contig_seq);
    let offset = usize::try_from(codon_offset(feature))
        .unwrap_or(0)
        .min(nt.len());
    translate_dna(&nt[offset..])
}

/// The on-contig translation of a CDS (`utils.normalize_cds_translation` without the
/// write-back): the existing `/translation` trimmed by [`on_contig_translation`], or a
/// fresh translation from `contig_seq` when there is none, it is empty, or it cannot
/// be trimmed. The second value is true when the both-ends-partial warning applies.
pub fn on_contig_cds_translation(feature: &SeqFeature, contig_seq: &[u8]) -> (String, bool) {
    let mut warned = false;
    let mut translation = feature.get_first("translation").unwrap_or("").to_string();
    if !translation.is_empty() {
        let (five, three) = location_partial_ends(&feature.location);
        match on_contig_translation(
            &translation,
            feature.location.len(),
            codon_offset(feature),
            five,
            three,
        ) {
            Some(t) => translation = t,
            None => {
                warned = true;
                translation.clear();
            }
        }
    }
    if translation.is_empty() {
        translation = translate_feature(feature, contig_seq);
    }
    (translation, warned)
}

/// The qualifier gene callers (prodigal, GeneMark) put their partial code in.
pub const PARTIAL_QUALIFIER: &str = "partial";

/// `(left_partial, right_partial)` from a prodigal-style `/partial` code like `10`
/// (`lean_record.partial_code_ends`): the first digit is the low-coordinate end, the
/// second the high one, whatever the strand. `None` without such a code -- including
/// the legacy bare `/partial`, which says nothing about which end is missing.
fn partial_code_ends(feature: &SeqFeature) -> Option<(bool, bool)> {
    let value = feature.get_first(PARTIAL_QUALIFIER)?;
    let code = value.trim().trim_matches('"').as_bytes();
    if code.len() != 2 || !code.iter().all(|c| matches!(c, b'0' | b'1')) {
        return None;
    }
    Some((code[0] == b'1', code[1] == b'1'))
}

/// `/codon_start` after adding `five_prime_extension` bases to a CDS's 5' end
/// (`lean_record.new_codon_start`); `None` when it stays 1 and was absent.
fn shifted_codon_start(feature: &SeqFeature, five_prime_extension: i64) -> Option<String> {
    let new_offset = (codon_offset(feature) + five_prime_extension).rem_euclid(3);
    if new_offset == 0 && !feature.has("codon_start") {
        return None;
    }
    Some((new_offset + 1).to_string())
}

/// Convert prodigal-style `/partial="10"` codes to `<`/`>` markers and remove them
/// (upstream `0a8ba4c`, `utils.normalize_partial_codes`, run on every GenBank record
/// read). The code is tied to the coordinates, so it would be wrong after a reverse
/// complement. A partial end within 2 bases of the contig edge is extended to the
/// edge (gene callers stop at the last whole codon), and an extended 5' end moves
/// `/codon_start`; a partial end elsewhere (a gap) is marked where it is. Markers
/// already on the location win, and the code is just dropped.
pub fn normalize_partial_codes(record: &mut SeqRecord) {
    let contig_len = record.seq.len() as i64;
    for f in &mut record.features {
        let Some((left, right)) = partial_code_ends(f) else {
            continue;
        };
        // The native parser (the default) leaves between-bases and empty locations
        // alone, code and all.
        if f.location.parts.is_empty() || f.location.between {
            continue;
        }
        f.qualifiers.retain(|(k, _)| k != PARTIAL_QUALIFIER);
        let (five, three) = location_partial_ends(&f.location);
        if five || three || !(left || right) {
            continue;
        }
        let parts = &mut f.location.parts;
        // First minimum / first maximum, like Python's `min`/`max` with a key.
        let mut leftmost = 0;
        let mut rightmost = 0;
        for (i, p) in parts.iter().enumerate() {
            if p.0 < parts[leftmost].0 {
                leftmost = i;
            }
            if p.1 > parts[rightmost].1 {
                rightmost = i;
            }
        }
        let (mut left_extension, mut right_extension) = (0, 0);
        if left {
            let p = &mut parts[leftmost];
            if p.0 < 3 {
                left_extension = p.0;
                p.0 = 0;
            }
            p.3 = true;
        }
        if right {
            let p = &mut parts[rightmost];
            if contig_len - p.1 < 3 {
                right_extension = contig_len - p.1;
                p.1 = contig_len;
            }
            p.4 = true;
        }
        // The 5' end is the start of the first part on the plus strand, its end on
        // the minus strand.
        let five_prime_extension = if parts[0].2 == -1 {
            if rightmost == 0 {
                right_extension
            } else {
                0
            }
        } else if leftmost == 0 {
            left_extension
        } else {
            0
        };
        if f.kind == "CDS" {
            if let Some(cs) = shifted_codon_start(f, five_prime_extension) {
                f.set("codon_start", cs);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seq::location::{parse_location, Part};

    fn cds(loc: &str) -> SeqFeature {
        SeqFeature::new("CDS", parse_location(loc))
    }

    fn protein(description: &str, features: Vec<SeqFeature>) -> SeqRecord {
        SeqRecord {
            id: "p".into(),
            name: "p".into(),
            description: description.into(),
            seq: b"MKRFSLAILALV".to_vec(),
            molecule_type: Some("protein".into()),
            topology: Default::default(),
            annotations: Default::default(),
            features,
        }
    }

    /// `test_get_fragment_status_feature`.
    #[test]
    fn fragment_status_from_feature_markers() {
        for (loc, status) in [
            ("1..100", None),
            ("<1..100", Some("N")),
            ("1..>100", Some("C")),
            ("<1..>100", Some("NC")),
            ("complement(1..>100)", Some("N")),
            ("complement(<1..100)", Some("C")),
            ("join(<1..50,60..100)", Some("N")),
            ("join(1..50,60..>100)", Some("C")),
            ("complement(join(1..50,60..>100))", Some("N")),
            ("join(1..>50,60..100)", None),
        ] {
            assert_eq!(feature_fragment_status(&cds(loc)), status, "{loc}");
        }
    }

    /// `test_get_fragment_status_protein_description` and the UniProt text entries.
    #[test]
    fn fragment_status_from_description() {
        for (desc, status) in [
            (
                "tr|A0A0|A0A0_ECOLI Superoxide dismutase OS=Escherichia coli OX=562",
                None,
            ),
            (
                "tr|A0A0|A0A0_ECOLI Superoxide dismutase (Fragment) OS=Escherichia coli",
                Some("?"),
            ),
            (
                "tr|A0A0|A0A0_ECOLI Superoxide dismutase (Fragments) OS=Escherichia coli",
                Some("?"),
            ),
            (
                "RecName: Full=Superoxide dismutase; Flags: Fragment;",
                Some("?"),
            ),
            (
                "RecName: Full=Superoxide dismutase; Flags: Precursor; Fragments;",
                Some("?"),
            ),
            (
                "RecName: Full=Superoxide dismutase; Flags: Precursor;",
                None,
            ),
            ("Fragment of something", None),
        ] {
            assert_eq!(
                record_fragment_status(&protein(desc, vec![])),
                status,
                "{desc}"
            );
        }
    }

    #[test]
    fn protein_status_precedence() {
        // A spanning CDS/Protein feature decides, even over a fragment description.
        let rec = protein("x (Fragment)", vec![cds("1..>12")]);
        assert_eq!(record_fragment_status(&rec), Some("C"));
        let rec = protein("x (Fragment)", vec![cds("1..12")]);
        assert_eq!(record_fragment_status(&rec), None);
        // NON_TER on the first and last residues.
        let nonter = |loc| SeqFeature::new(NON_TERMINAL_FEATURE_NAME, parse_location(loc));
        let rec = protein("x", vec![nonter("1"), nonter("12")]);
        assert_eq!(record_fragment_status(&rec), Some("NC"));
        let rec = protein("x (Fragment)", vec![nonter("12")]);
        assert_eq!(record_fragment_status(&rec), Some("C"));
        // A nucleotide record has no record-level status.
        let mut nuc = protein("x (Fragment)", vec![cds("<1..12")]);
        nuc.molecule_type = Some("DNA".into());
        assert_eq!(record_fragment_status(&nuc), None);
        assert!(contig_has_fragment(&nuc));
        assert_eq!(cds_fragment_status(&nuc, Some(&nuc.features[0])), Some("N"));
    }

    /// `test_on_contig_translation`.
    #[test]
    fn on_contig_translation_cases() {
        // 30 nt, frame 0: 10 codons, the last a stop for a complete 3' end.
        assert_eq!(
            on_contig_translation("MAAAAAAAA", 30, 0, false, false).as_deref(),
            Some("MAAAAAAAA")
        );
        // Too long, 5' partial: trim from the front.
        assert_eq!(
            on_contig_translation("XXMAAAAAAAA", 30, 0, true, false).as_deref(),
            Some("MAAAAAAAA")
        );
        // Own translation with a '*' counts the stop codon.
        assert_eq!(
            on_contig_translation("XMAAAAAAAA*", 30, 0, true, false).as_deref(),
            Some("MAAAAAAAA*")
        );
        // Too long, 3' partial: trim from the back (no stop codon expected).
        assert_eq!(
            on_contig_translation("MAAAAAAAAAXX", 30, 0, false, true).as_deref(),
            Some("MAAAAAAAAA")
        );
        // codon_start 3 shortens the on-contig part.
        assert_eq!(
            on_contig_translation("MAAAAAAAAAXX", 30, 2, false, true).as_deref(),
            Some("MAAAAAAAA")
        );
        // Both ends partial and too long: can't place the excess.
        assert_eq!(
            on_contig_translation("MAAAAAAAAAXX", 30, 0, true, true),
            None
        );
        // Complete CDS: never trimmed.
        assert_eq!(
            on_contig_translation("MAAAAAAAAAXX", 30, 0, false, false).as_deref(),
            Some("MAAAAAAAAAXX")
        );
    }

    /// Upstream's own `test_on_contig_translation` table: 14 nt in frame 2, 4 codons.
    #[test]
    fn on_contig_translation_upstream_table() {
        // Upstream's table, with one residue changed so the spell checker passes it.
        for (translation, five, three, expected) in [
            ("MSKV", false, false, Some("MSKV")),
            ("XXMSKV", false, false, Some("XXMSKV")), // not partial, left as is
            ("XXMSKV", true, false, Some("SKV")),     // a complete 3' end lacks its stop codon
            ("XXMSK*", true, false, Some("MSK*")),
            ("MSKVXX", false, true, Some("MSKV")),
            ("XXMSKVXX", true, true, None),
            ("MSK", true, true, Some("MSK")), // shorter than the location, left as is
        ] {
            assert_eq!(
                on_contig_translation(translation, 14, 2, five, three).as_deref(),
                expected,
                "{translation} {five} {three}"
            );
        }
    }

    fn coded(loc: &str, code: &str) -> SeqFeature {
        let mut f = cds(loc);
        f.set(PARTIAL_QUALIFIER, code);
        f
    }

    fn normalized(features: Vec<SeqFeature>, len: usize) -> Vec<SeqFeature> {
        let mut rec = protein("", features);
        rec.molecule_type = Some("DNA".into());
        rec.seq = vec![b'A'; len];
        normalize_partial_codes(&mut rec);
        rec.features
    }

    /// Upstream's `test_normalize_partial_codes` table (a 60 nt record).
    #[test]
    fn prodigal_partial_codes_become_markers() {
        type Case = (&'static str, &'static str, Part, Option<&'static str>);
        let cases: [Case; 8] = [
            // The left end runs off the sequence; the gene caller stopped 1 base short.
            ("2..30", "10", (0, 30, 1, true, false), Some("2")), // plus: left is 5'
            ("complement(2..30)", "10", (0, 30, -1, true, false), None), // minus: left is 3'
            (
                "complement(31..58)",
                "01",
                (30, 60, -1, false, true),
                Some("3"),
            ), // minus: right is 5'
            ("31..58", "01", (30, 60, 1, false, true), None),
            ("1..60", "11", (0, 60, 1, true, true), None),
            // Runs into a gap, not the edge: marked where it is.
            ("11..30", "10", (10, 30, 1, true, false), None),
            ("2..30", "00", (1, 30, 1, false, false), None),
            // Existing markers win.
            ("<1..30", "01", (0, 30, 1, true, false), None),
        ];
        for (loc, code, expected, codon_start) in cases {
            let f = &normalized(vec![coded(loc, code)], 60)[0];
            assert_eq!(f.location.parts, vec![expected], "{loc} {code}");
            assert_eq!(f.get_first("codon_start"), codon_start, "{loc} {code}");
            assert!(!f.has(PARTIAL_QUALIFIER), "{loc} {code}");
        }
    }

    /// `test_normalize_partial_codes_keeps_bare_partial_and_combines_codon_start`.
    #[test]
    fn bare_partial_stays_and_codon_start_combines() {
        let f = &normalized(vec![coded("2..30", "")], 60)[0];
        assert_eq!(f.get_first(PARTIAL_QUALIFIER), Some(""));
        assert_eq!(f.location.parts, vec![(1, 30, 1, false, false)]);
        let mut f = coded("2..30", "10");
        f.set("codon_start", "2");
        let f = &normalized(vec![f], 60)[0];
        assert_eq!(f.get_first("codon_start"), Some("3"));
    }

    /// A bare `/partial` makes a CDS (and a protein spanned by one) a `?` fragment,
    /// survives reading, and loses to markers (upstream `test_utils.py`,
    /// `test_bare_partial_qualifier_is_a_fragment` and its neighbor).
    #[test]
    fn bare_partial_is_an_unknown_fragment() {
        let f = &normalized(vec![coded("2..30", "")], 60)[0];
        assert_eq!(feature_fragment_status(f), Some("?"));
        let f = &normalized(vec![coded("<1..30", "")], 60)[0];
        assert_eq!(feature_fragment_status(f), Some("N"));
        let mut contig = protein("", normalized(vec![coded("2..30", "")], 60));
        contig.molecule_type = Some("DNA".into());
        assert!(contig_has_fragment(&contig));
        let rec = protein("x", vec![coded("1..12", "")]);
        assert_eq!(record_fragment_status(&rec), Some("?"));
    }

    /// `test_fragment_status_allowed`.
    #[test]
    fn partial_filter() {
        for (status, include, exclude, only) in [
            (None, true, true, false),
            (Some("N"), true, false, true),
            (Some("?"), true, false, true),
        ] {
            assert_eq!(Partial::Include.allows(status), include);
            assert_eq!(Partial::Exclude.allows(status), exclude);
            assert_eq!(Partial::Only.allows(status), only);
        }
        assert_eq!("only".parse::<Partial>(), Ok(Partial::Only));
        assert!("some".parse::<Partial>().is_err());
    }

    #[test]
    fn codon_offset_and_translate_feature() {
        let mut f = cds("1..9");
        assert_eq!(codon_offset(&f), 0);
        f.set("codon_start", "3");
        assert_eq!(codon_offset(&f), 2);
        assert_eq!(translate_feature(&f, b"AAATGGGGT"), "MG");
        f.set("codon_start", "4");
        assert_eq!(codon_offset(&f), 0);
        f.set("codon_start", "x");
        assert_eq!(codon_offset(&f), 0);
    }
}
