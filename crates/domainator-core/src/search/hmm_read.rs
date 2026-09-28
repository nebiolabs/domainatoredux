//! Reading HMMER3 profiles, with the Pfam cutoff lines repaired.
//!
//! `hmmer_pure_rs`'s parser splits `GA`/`TC`/`NC` on whitespace and parses each
//! half with `unwrap_or(<the first cutoff>)`. Pfam ends those lines with a
//! semicolon (`TC    28.60 28.80;`), so the *second* value fails to parse and
//! silently becomes a copy of the first — `28.60 28.80;` reads back as
//! `28.60 28.60`. It is invisible whenever the two halves are equal, which they
//! usually are, and `test/data/CcdB.hmm` is where it is not.
//!
//! Nothing in this port scores against the cutoffs, but [`super::hmm_write`]
//! writes them out, and emitting a wrong Pfam threshold is worse than emitting
//! none. So re-read the three lines from the file and correct the parsed model.
//! Reported upstream; drop this once the crate's parser strips the semicolon.
//!
//! Compression follows `utils.open_hmm_file`: gzip and BGZF are detected from
//! the **magic bytes**, not the name. The crate's own opener dispatches on a
//! `.gz` extension with a single-member `GzDecoder`, which fails on an
//! uncompressed file named `.gz` and reads only the first 64 KiB block of a
//! BGZF file (silently dropping every later profile). `hmmpress` sidecars
//! (`.h3f`/`.h3i`/`.h3m`/`.h3p`) are never consulted: they are keyed to the
//! `.hmm` only by name, so a stale set would yield the old profiles.

use std::io::Read;
use std::path::Path;

use hmmer_pure_rs::errors::{HmmerError, HmmerResult};
use hmmer_pure_rs::hmm::{
    Hmm, CUTOFF_UNSET, P7H_GA, P7H_NC, P7H_TC, P7_GA1, P7_GA2, P7_NC1, P7_NC2, P7_TC1, P7_TC2,
};

/// Read every profile in `path`, with `GA`/`TC`/`NC` taken from the raw text.
///
/// The file may be plain, gzip or BGZF. It is decompressed once into memory and
/// both the parse and the cutoff repair read that one copy.
pub fn read_hmm_file(path: &Path) -> HmmerResult<Vec<Hmm>> {
    let mut bytes = Vec::new();
    crate::io::open_decompressed(path)
        .and_then(|mut r| r.read_to_end(&mut bytes))
        .map_err(HmmerError::Io)?;
    let mut hmms = hmmer_pure_rs::hmmfile::read_hmms(std::io::BufReader::new(bytes.as_slice()))?;
    if let Ok(text) = std::str::from_utf8(&bytes) {
        repair_cutoffs(text, &mut hmms);
    }
    Ok(hmms)
}

/// The first profile in `path` (plain, gzip or BGZF), or `None` for an empty file.
///
/// Reads only as far as the first `//`, for callers that want the alphabet a
/// library is locked to without parsing all of Pfam. Cutoffs are not repaired.
pub fn read_first_hmm(path: &Path) -> HmmerResult<Option<Hmm>> {
    let reader = crate::io::open_decompressed(path).map_err(HmmerError::Io)?;
    match hmmer_pure_rs::hmmfile::read_first_hmm(std::io::BufReader::new(reader)) {
        Ok(h) => Ok(Some(h)),
        Err(HmmerError::Format(m)) if m.contains("No HMM") => Ok(None),
        Err(e) => Err(e),
    }
}

/// Overwrite each profile's cutoffs with the values spelled in `text`.
///
/// Profiles are matched positionally: the parser and this scan walk the same
/// `//`-delimited records in the same order, so the n-th `GA`/`TC`/`NC` block
/// belongs to the n-th model.
fn repair_cutoffs(text: &str, hmms: &mut [Hmm]) {
    let mut index = 0;
    for line in text.lines() {
        if index >= hmms.len() {
            break;
        }
        if line.starts_with("//") {
            index += 1;
            continue;
        }
        let Some((key, rest)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let (bit, lo, hi) = match key {
            "GA" => (P7H_GA, P7_GA1, P7_GA2),
            "TC" => (P7H_TC, P7_TC1, P7_TC2),
            "NC" => (P7H_NC, P7_NC1, P7_NC2),
            _ => continue,
        };
        // Pfam's trailing `;` is the whole point of this function.
        let mut values = rest
            .trim()
            .trim_end_matches(';')
            .split_whitespace()
            .map(|v| v.trim_end_matches(';').parse::<f32>());
        let Some(Ok(first)) = values.next() else {
            continue;
        };
        let second = match values.next() {
            Some(Ok(v)) => v,
            // A one-value line means both cutoffs are that value.
            None => first,
            Some(Err(_)) => continue,
        };
        let hmm = &mut hmms[index];
        hmm.cutoff[lo] = first;
        hmm.cutoff[hi] = second;
        hmm.flags |= bit;
    }
    // A model whose cutoffs never parsed at all must not claim to have them.
    for hmm in hmms.iter_mut() {
        for (bit, lo) in [(P7H_GA, P7_GA1), (P7H_TC, P7_TC1), (P7H_NC, P7_NC1)] {
            if hmm.cutoff[lo] == CUTOFF_UNSET {
                hmm.flags &= !bit;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hmmer_pure_rs::alphabet::AlphabetType;

    fn model() -> Hmm {
        Hmm::new(1, AlphabetType::Amino, 20)
    }

    #[test]
    fn pfam_trailing_semicolon_keeps_the_second_cutoff() {
        // Verbatim from test/data/CcdB.hmm, where the two halves differ.
        let text = "GA    28.60 28.60;\nTC    28.60 28.80;\nNC    28.50 28.40;\n//\n";
        let mut hmms = vec![model()];
        repair_cutoffs(text, &mut hmms);
        assert_eq!(hmms[0].cutoff[P7_GA1], 28.60);
        assert_eq!(hmms[0].cutoff[P7_GA2], 28.60);
        assert_eq!(hmms[0].cutoff[P7_TC1], 28.60);
        assert_eq!(hmms[0].cutoff[P7_TC2], 28.80);
        assert_eq!(hmms[0].cutoff[P7_NC1], 28.50);
        assert_eq!(hmms[0].cutoff[P7_NC2], 28.40);
        assert_eq!(
            hmms[0].flags & (P7H_GA | P7H_TC | P7H_NC),
            P7H_GA | P7H_TC | P7H_NC
        );
    }

    #[test]
    fn cutoffs_land_on_the_model_that_declared_them() {
        let text = "NAME  a\nGA    1.00 2.00\n//\nNAME  b\nGA    3.00 4.00\n//\n";
        let mut hmms = vec![model(), model()];
        repair_cutoffs(text, &mut hmms);
        assert_eq!(
            (hmms[0].cutoff[P7_GA1], hmms[0].cutoff[P7_GA2]),
            (1.00, 2.00)
        );
        assert_eq!(
            (hmms[1].cutoff[P7_GA1], hmms[1].cutoff[P7_GA2]),
            (3.00, 4.00)
        );
    }

    #[test]
    fn a_profile_without_cutoffs_does_not_claim_them() {
        let mut hmms = vec![model()];
        repair_cutoffs("NAME  a\n//\n", &mut hmms);
        assert_eq!(hmms[0].flags & (P7H_GA | P7H_TC | P7H_NC), 0);
    }

    #[test]
    fn a_single_value_line_fills_both_halves() {
        let mut hmms = vec![model()];
        repair_cutoffs("TC    12.50\n//\n", &mut hmms);
        assert_eq!(
            (hmms[0].cutoff[P7_TC1], hmms[0].cutoff[P7_TC2]),
            (12.50, 12.50)
        );
    }
}
