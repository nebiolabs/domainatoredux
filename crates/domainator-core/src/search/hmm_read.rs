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

use std::path::Path;

use hmmer_pure_rs::errors::HmmerResult;
use hmmer_pure_rs::hmm::{
    Hmm, CUTOFF_UNSET, P7H_GA, P7H_NC, P7H_TC, P7_GA1, P7_GA2, P7_NC1, P7_NC2, P7_TC1, P7_TC2,
};

/// Read every profile in `path`, with `GA`/`TC`/`NC` taken from the raw text.
pub fn read_hmm_file(path: &Path) -> HmmerResult<Vec<Hmm>> {
    let mut hmms = hmmer_pure_rs::hmmfile::read_hmm_file(path)?;
    if let Ok(text) = std::fs::read_to_string(path) {
        repair_cutoffs(&text, &mut hmms);
    }
    Ok(hmms)
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
        assert_eq!(hmms[0].flags & (P7H_GA | P7H_TC | P7H_NC), P7H_GA | P7H_TC | P7H_NC);
    }

    #[test]
    fn cutoffs_land_on_the_model_that_declared_them() {
        let text = "NAME  a\nGA    1.00 2.00\n//\nNAME  b\nGA    3.00 4.00\n//\n";
        let mut hmms = vec![model(), model()];
        repair_cutoffs(text, &mut hmms);
        assert_eq!((hmms[0].cutoff[P7_GA1], hmms[0].cutoff[P7_GA2]), (1.00, 2.00));
        assert_eq!((hmms[1].cutoff[P7_GA1], hmms[1].cutoff[P7_GA2]), (3.00, 4.00));
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
        assert_eq!((hmms[0].cutoff[P7_TC1], hmms[0].cutoff[P7_TC2]), (12.50, 12.50));
    }
}
