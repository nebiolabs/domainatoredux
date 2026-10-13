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

/// Profiles per parse chunk. Pfam records average ~54 KB of text, so a chunk is a
/// few MB and a batch of `2 * threads` chunks stays well under the 1.6 GB that
/// holding all of Pfam-A's text would cost.
const RECORDS_PER_CHUNK: usize = 64;

/// Read every profile in `path`, with `GA`/`TC`/`NC` taken from the raw text.
///
/// The file may be plain, gzip or BGZF. It is streamed in `//`-delimited chunks
/// that are parsed in parallel while the next batch is decompressed: parsing
/// Pfam-A serially took four times as long as decompressing it, and keeping its
/// whole text in memory beside the parsed models doubled peak memory. Each chunk
/// is a run of complete records, so its parse and cutoff repair are the same as
/// the whole file's, and chunks are concatenated in file order.
///
/// Anything unusual (a parse error, mixed format versions or alphabets, which the
/// crate rejects only across a whole file) falls back to [`read_hmm_file_serial`],
/// so errors read exactly as they always have.
pub fn read_hmm_file(path: &Path) -> HmmerResult<Vec<Hmm>> {
    read_hmm_file_chunked(path, RECORDS_PER_CHUNK, 2 * rayon::current_num_threads())
}

/// [`read_hmm_file`] with the chunk shape spelled out, so a test can cross chunk
/// and batch boundaries without thousands of records.
fn read_hmm_file_chunked(
    path: &Path,
    records_per_chunk: usize,
    batch_len: usize,
) -> HmmerResult<Vec<Hmm>> {
    let mut reader =
        std::io::BufReader::new(crate::io::open_decompressed(path).map_err(HmmerError::Io)?);
    let mut format: Option<Vec<u8>> = None;
    let mut mixed_formats = false;
    let mut hmms = Vec::new();
    let mut chunks = |reader: &mut _| {
        read_chunks(
            reader,
            records_per_chunk,
            batch_len,
            &mut format,
            &mut mixed_formats,
        )
    };
    let mut batch = chunks(&mut reader).map_err(HmmerError::Io)?;
    while !batch.is_empty() {
        let (parsed, next) = std::thread::scope(|s| {
            let parser = s.spawn(|| {
                use rayon::prelude::*;
                batch
                    .par_iter()
                    .map(|chunk| parse_chunk(chunk))
                    .collect::<Vec<_>>()
            });
            let next = chunks(&mut reader);
            let parsed = parser
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
            (parsed, next)
        });
        for chunk in parsed {
            match chunk {
                Ok(chunk) => hmms.extend(chunk),
                Err(_) => return read_hmm_file_serial(path),
            }
        }
        batch = next.map_err(HmmerError::Io)?;
    }
    let mixed_alphabets = hmms.windows(2).any(|w| w[0].abc_type != w[1].abc_type);
    if mixed_formats || mixed_alphabets {
        return read_hmm_file_serial(path);
    }
    Ok(hmms)
}

/// Up to `count` chunks of `records_per_chunk` whole records each, each ending on
/// a `//` line (or at end of input). Notes each record's `HMMER3/x` header, since
/// the crate's mixed-format check cannot see across chunks.
fn read_chunks(
    reader: &mut impl std::io::BufRead,
    records_per_chunk: usize,
    count: usize,
    format: &mut Option<Vec<u8>>,
    mixed_formats: &mut bool,
) -> std::io::Result<Vec<Vec<u8>>> {
    let mut chunks = Vec::with_capacity(count);
    let mut line = Vec::new();
    while chunks.len() < count {
        let mut chunk = Vec::new();
        let mut records = 0;
        while records < records_per_chunk {
            line.clear();
            if reader.read_until(b'\n', &mut line)? == 0 {
                break;
            }
            if line.starts_with(b"//") {
                records += 1;
            } else if line.starts_with(b"HMMER3/") {
                let magic = line
                    .split(|b| b.is_ascii_whitespace())
                    .next()
                    .unwrap_or(&[]);
                match format {
                    Some(first) => *mixed_formats |= first.as_slice() != magic,
                    None => *format = Some(magic.to_vec()),
                }
            }
            chunk.extend_from_slice(&line);
        }
        if chunk.is_empty() {
            break;
        }
        chunks.push(chunk);
    }
    Ok(chunks)
}

/// Parse one chunk of whole records and repair its cutoffs from the same text.
fn parse_chunk(text: &[u8]) -> HmmerResult<Vec<Hmm>> {
    let mut hmms = hmmer_pure_rs::hmmfile::read_hmms(std::io::BufReader::new(text))?;
    if let Ok(text) = std::str::from_utf8(text) {
        repair_cutoffs(text, &mut hmms);
    }
    Ok(hmms)
}

/// [`read_hmm_file`] in one piece: the whole file decompressed into memory, parsed
/// serially, and the cutoffs repaired from that one copy. Kept as the path that
/// reports errors, because the crate's messages number records and lines from
/// the start of the file.
pub fn read_hmm_file_serial(path: &Path) -> HmmerResult<Vec<Hmm>> {
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

    fn fixture(name: &str) -> String {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../test/data/");
        std::fs::read_to_string(format!("{path}{name}")).unwrap()
    }

    /// `text` written to a scratch file, read by both readers.
    fn read_both(name: &str, text: &str) -> (String, String) {
        let path = std::env::temp_dir().join(format!(
            "domainator-hmmread-{}-{name}.hmm",
            std::process::id()
        ));
        std::fs::write(&path, text).unwrap();
        // Two records per chunk, three chunks per batch.
        let chunked = format!("{:?}", read_hmm_file_chunked(&path, 2, 3));
        let serial = format!("{:?}", read_hmm_file_serial(&path));
        std::fs::remove_file(&path).unwrap();
        (chunked, serial)
    }

    #[test]
    fn chunked_read_matches_the_serial_read() {
        // Enough records for several chunks and more than one batch, with CcdB's
        // unequal cutoffs recurring so a repair landing on the wrong model shows.
        let one = fixture("CcdB.hmm") + &fixture("pdonr_hmms.hmm");
        let text = one.repeat(5);
        let (chunked, serial) = read_both("many", &text);
        assert!(
            serial.starts_with("Ok("),
            "{}",
            &serial[..200.min(serial.len())]
        );
        assert_eq!(chunked, serial);
    }

    #[test]
    fn a_mixed_alphabet_file_fails_as_it_always_has() {
        // A DNA library first, a protein one far enough on to be in another chunk:
        // only the whole-file check sees the two side by side.
        let text = fixture("dna_profiles.hmm") + &fixture("pdonr_hmms.hmm").repeat(2);
        let (chunked, serial) = read_both("mixed", &text);
        assert!(serial.starts_with("Err("), "{serial}");
        assert_eq!(chunked, serial);
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
