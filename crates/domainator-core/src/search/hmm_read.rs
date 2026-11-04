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
//!
//! The crate rejects a match line whose MAP column is not an integer, where HMMER
//! (and so pyhmmer, which upstream reads with) takes `atoi` of it: a stray token, as
//! a terminal escape pasted into a production library, reads as 0 and the model
//! loads. MAP is never scored, so such a record is retried with the column rewritten
//! as HMMER reads it, and a warning names where. Any error that remains is reported
//! with the file, line and model name, which the crate's messages lack.

use std::io::Read;
use std::path::Path;

use hmmer_pure_rs::errors::{HmmerError, HmmerResult};
/// The profile type every reader here returns, for callers outside the core.
pub use hmmer_pure_rs::hmm::Hmm;
use hmmer_pure_rs::hmm::{
    CUTOFF_UNSET, P7H_GA, P7H_NC, P7H_TC, P7_GA1, P7_GA2, P7_NC1, P7_NC2, P7_TC1, P7_TC2,
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
/// A parse error is reported with the file, line and model it is in. Mixed format
/// versions or alphabets, which the crate rejects only across a whole file, fall back
/// to [`read_hmm_file_serial`], so those errors read exactly as they always have.
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
    let mut next_line = 1;
    let mut hmms = Vec::new();
    let mut chunks = |reader: &mut _| {
        read_chunks(
            reader,
            records_per_chunk,
            batch_len,
            &mut format,
            &mut mixed_formats,
            &mut next_line,
        )
    };
    let mut batch = chunks(&mut reader).map_err(HmmerError::Io)?;
    while !batch.is_empty() {
        // Parse in the caller's pool (the global one outside any) while this thread
        // decompresses the next batch; a plain thread would always use the global pool.
        let mut parsed = Vec::new();
        let next = rayon::in_place_scope(|s| {
            s.spawn(|_| {
                use rayon::prelude::*;
                parsed = batch
                    .par_iter()
                    .map(|(first_line, chunk)| parse_records(chunk, path, *first_line))
                    .collect::<Vec<_>>();
            });
            chunks(&mut reader)
        });
        for chunk in parsed {
            match chunk {
                Ok(chunk) => hmms.extend(chunk),
                Err(ParseFailure::InRecord(e)) => return Err(e),
                Err(ParseFailure::AcrossRecords(_)) => return read_hmm_file_serial(path),
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
/// a `//` line (or at end of input) and paired with the 1-based line it starts on
/// (`next_line` carries the count across calls). Notes each record's `HMMER3/x`
/// header, since the crate's mixed-format check cannot see across chunks.
fn read_chunks(
    reader: &mut impl std::io::BufRead,
    records_per_chunk: usize,
    count: usize,
    format: &mut Option<Vec<u8>>,
    mixed_formats: &mut bool,
    next_line: &mut usize,
) -> std::io::Result<Vec<(usize, Vec<u8>)>> {
    let mut chunks = Vec::with_capacity(count);
    let mut line = Vec::new();
    while chunks.len() < count {
        let first_line = *next_line;
        let mut chunk = Vec::new();
        let mut records = 0;
        while records < records_per_chunk {
            line.clear();
            if reader.read_until(b'\n', &mut line)? == 0 {
                break;
            }
            *next_line += 1;
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
        chunks.push((first_line, chunk));
    }
    Ok(chunks)
}

/// Why [`parse_records`] failed: in one record, located by [`locate_error`], or only
/// across records (mixed format versions or alphabets), which only a whole-file read
/// reports as it always has.
enum ParseFailure {
    InRecord(HmmerError),
    AcrossRecords(HmmerError),
}

/// Parse whole records (`text`, starting on line `first_line` of `path`) and repair
/// their cutoffs from the same text. A record the crate rejects only for a MAP value
/// HMMER would accept is retried as HMMER reads it; any other error is located.
fn parse_records(text: &[u8], path: &Path, first_line: usize) -> Result<Vec<Hmm>, ParseFailure> {
    let parse = |text: &[u8]| -> HmmerResult<Vec<Hmm>> {
        let mut hmms = hmmer_pure_rs::hmmfile::read_hmms(std::io::BufReader::new(text))?;
        if let Ok(text) = std::str::from_utf8(text) {
            repair_cutoffs(text, &mut hmms);
        }
        Ok(hmms)
    };
    match parse(text) {
        Ok(hmms) => Ok(hmms),
        Err(HmmerError::Format(_)) => {
            let lenient = lenient_map_columns(text, path, first_line);
            let text = lenient.as_deref().unwrap_or(text);
            parse(text).map_err(|e| match locate_error(&e, text, path, first_line) {
                Some(located) => ParseFailure::InRecord(located),
                None => ParseFailure::AcrossRecords(e),
            })
        }
        Err(e) => Err(ParseFailure::InRecord(e)),
    }
}

/// [`read_hmm_file`] in one piece: the whole file decompressed into memory, parsed
/// serially, and the cutoffs repaired from that one copy. The path for files the
/// crate must see whole (mixed format versions or alphabets).
pub fn read_hmm_file_serial(path: &Path) -> HmmerResult<Vec<Hmm>> {
    let mut bytes = Vec::new();
    crate::io::open_decompressed(path)
        .and_then(|mut r| r.read_to_end(&mut bytes))
        .map_err(HmmerError::Io)?;
    parse_records(&bytes, path, 1).map_err(|failure| match failure {
        ParseFailure::InRecord(e) => e,
        ParseFailure::AcrossRecords(HmmerError::Format(m)) => {
            HmmerError::Format(format!("{}: {}", path.display(), escape_control(&m)))
        }
        ParseFailure::AcrossRecords(e) => e,
    })
}

/// The first profile in `path` (plain, gzip or BGZF), or `None` for an empty file.
///
/// Reads only as far as the first `//`, for callers that want the alphabet a
/// library is locked to without parsing all of Pfam.
pub fn read_first_hmm(path: &Path) -> HmmerResult<Option<Hmm>> {
    use std::io::BufRead;
    let mut reader =
        std::io::BufReader::new(crate::io::open_decompressed(path).map_err(HmmerError::Io)?);
    let mut text = Vec::new();
    loop {
        let start = text.len();
        if reader
            .read_until(b'\n', &mut text)
            .map_err(HmmerError::Io)?
            == 0
            || text[start..].starts_with(b"//")
        {
            break;
        }
    }
    if text.iter().all(u8::is_ascii_whitespace) {
        return Ok(None);
    }
    match parse_records(&text, path, 1) {
        Ok(hmms) => Ok(hmms.into_iter().next()),
        Err(ParseFailure::InRecord(e) | ParseFailure::AcrossRecords(e)) => Err(e),
    }
}

/// C's `atoi`: an optional sign and the leading digits, 0 when there are none.
fn atoi(token: &str) -> i64 {
    let (sign, rest) = match token.as_bytes().first() {
        Some(b'-') => (-1, &token[1..]),
        Some(b'+') => (1, &token[1..]),
        _ => (1, token),
    };
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    sign * rest[..digits].parse::<i64>().unwrap_or(0)
}

/// The byte ranges of a line's whitespace-separated tokens, as HMMER splits them
/// (spaces and tabs; a line's own `\r\n` ends the last token).
fn token_ranges(line: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, b) in line.bytes().enumerate() {
        let space = matches!(b, b' ' | b'\t' | b'\r' | b'\n');
        match (space, start) {
            (true, Some(s)) => {
                out.push((s, i));
                start = None;
            }
            (false, None) => start = Some(i),
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((s, line.len()));
    }
    out
}

/// `text` with every non-integer MAP value (in a `MAP yes` record) replaced by its
/// `atoi`, as HMMER's reader stores it, warning once per record with where it was;
/// `None` when there is none to replace. The MAP column follows the node number and
/// the K match emissions, K being 20 for amino and 4 for nucleotide alphabets.
fn lenient_map_columns(text: &[u8], path: &Path, first_line: usize) -> Option<Vec<u8>> {
    let text = std::str::from_utf8(text).ok()?;
    let mut out = String::with_capacity(text.len());
    let mut changed = false;
    let (mut k, mut map_yes, mut in_body) = (20usize, false, false);
    let mut name = String::new();
    // (line, token) of each value replaced in the current record.
    let mut fixed: Vec<(usize, String)> = Vec::new();
    let warn = |name: &str, fixed: &mut Vec<(usize, String)>| {
        if let Some((line, token)) = fixed.first() {
            let more = match fixed.len() {
                1 => String::new(),
                n => format!(" ({n} such lines in this model)"),
            };
            crate::log::warn(format_args!(
                "{}:{line} (model {name}): MAP value '{}' is not a number; read as {}, as \
                 HMMER reads it{more}",
                path.display(),
                escape_control(token),
                atoi(token),
            ));
        }
        fixed.clear();
    };
    for (i, line) in text.split_inclusive('\n').enumerate() {
        let tokens = token_ranges(line);
        let word = |j: usize| tokens.get(j).map(|&(a, b)| &line[a..b]);
        match word(0) {
            Some("//") => {
                warn(&name, &mut fixed);
                (k, map_yes, in_body) = (20, false, false);
                name.clear();
            }
            Some("NAME") if !in_body => name = word(1).unwrap_or("").to_string(),
            Some("ALPH") if !in_body => {
                let alphabet = word(1).map(str::to_ascii_lowercase);
                k = if matches!(alphabet.as_deref(), Some("dna" | "rna")) {
                    4
                } else {
                    20
                };
            }
            Some("MAP") if !in_body => {
                map_yes = word(1).is_some_and(|v| v.eq_ignore_ascii_case("yes"));
            }
            Some("HMM") => in_body = true,
            Some(node) if in_body && map_yes && node.parse::<usize>().is_ok_and(|n| n > 0) => {
                if let Some(&(a, b)) = tokens.get(k + 1) {
                    let token = &line[a..b];
                    if token.parse::<i64>().is_err() {
                        out.push_str(&line[..a]);
                        out.push_str(&atoi(token).to_string());
                        out.push_str(&line[b..]);
                        fixed.push((first_line + i, token.to_string()));
                        changed = true;
                        continue;
                    }
                }
            }
            _ => {}
        }
        out.push_str(line);
    }
    warn(&name, &mut fixed);
    changed.then(|| out.into_bytes())
}

/// `s` with control characters (a stray escape sequence, say) spelled as escapes, so
/// a message never writes them to the terminal.
fn escape_control(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_control() {
            out.extend(c.escape_default());
        } else {
            out.push(c);
        }
    }
    out
}

/// The crate's `error` for `text` (records starting on line `first_line` of `path`),
/// saying where it is: the failing record is found by parsing the records one at a
/// time, and the line within it by the token the message quotes, when it quotes one
/// that is on a line of the record (else the record's first line). For example
/// `refs.hmm:5119428 (model spbK): Bad MAP value: \u{1b}[O`. `None` when every record
/// parses alone, so the error lies across records.
fn locate_error(
    error: &HmmerError,
    text: &[u8],
    path: &Path,
    first_line: usize,
) -> Option<HmmerError> {
    let HmmerError::Format(_) = error else {
        return None;
    };
    let text = std::str::from_utf8(text).ok()?;
    let mut record_start = 0;
    let mut record_line = first_line;
    let mut offset = 0;
    for (i, line) in text.split_inclusive('\n').enumerate() {
        offset += line.len();
        if !line.starts_with("//") && offset != text.len() {
            continue;
        }
        let record = &text[record_start..offset];
        if let Err(HmmerError::Format(m)) =
            hmmer_pure_rs::hmmfile::read_hmms(std::io::BufReader::new(record.as_bytes()))
        {
            let name = record
                .lines()
                .find_map(|l| l.strip_prefix("NAME").map(|n| n.trim().to_string()))
                .unwrap_or_else(|| "?".into());
            let quoted = m.rsplit_once(": ").map(|(_, t)| t.trim());
            let line = quoted
                .and_then(|t| {
                    record
                        .lines()
                        .position(|l| token_ranges(l).iter().any(|&(a, b)| &l[a..b] == t))
                })
                .map_or(record_line, |j| record_line + j);
            return Some(HmmerError::Format(format!(
                "{}:{line} (model {name}): {}",
                path.display(),
                escape_control(&m)
            )));
        }
        record_start = offset;
        record_line = first_line + i + 1;
    }
    None
}

/// A reading error from this module as one line that names the file: a located
/// parse error already does (`refs.hmm:12 (model a): ...`), anything else is prefixed
/// with the path. Use it instead of the error's `Debug` form, which escapes and
/// wraps the message (`Format("...")`) and never says which file.
pub fn error_message(path: &Path, error: &HmmerError) -> String {
    let shown = path.display().to_string();
    match error {
        HmmerError::Format(m) if m.starts_with(&shown) => m.clone(),
        HmmerError::Format(m) => format!("{shown}: {}", escape_control(m)),
        HmmerError::Io(e) => format!("{shown}: {e}"),
        other => format!("{shown}: {other}"),
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
    /// `pdonr_hmms.hmm` with line `line` (1-based) passed through `edit`, written to a
    /// scratch file named for the test.
    fn edited_pdonr(name: &str, line: usize, edit: impl Fn(&str) -> String) -> std::path::PathBuf {
        let text: String = fixture("pdonr_hmms.hmm")
            .split_inclusive('\n')
            .enumerate()
            .map(|(i, l)| {
                if i + 1 == line {
                    edit(l)
                } else {
                    l.to_string()
                }
            })
            .collect();
        let path = std::env::temp_dir().join(format!(
            "domainator-hmmread-{}-{name}.hmm",
            std::process::id()
        ));
        std::fs::write(&path, text).unwrap();
        path
    }

    /// PADLOC v2 has a terminal escape pasted before one MAP value. HMMER reads MAP
    /// with `atoi` (so 0) and the rest of the line shifts left, CONS taking the first
    /// character of the real MAP value; the crate rejected the whole file.
    #[test]
    fn a_non_numeric_map_value_reads_as_hmmer_reads_it() {
        // Line 1050 is node 100 of the second model, APH: `... 222 e - - H`.
        let clean = read_hmm_file(&edited_pdonr("map-clean", 0, str::to_string)).unwrap();
        let path = edited_pdonr("map-escape", 1050, |l| {
            l.replace(" 222 e", " \u{1b}[O 222 e")
        });
        for hmms in [
            read_hmm_file_chunked(&path, 1, 2).unwrap(),
            read_hmm_file_serial(&path).unwrap(),
        ] {
            assert_eq!(hmms.len(), clean.len());
            assert_eq!(hmms[1].name, "APH");
            assert_eq!(hmms[1].map.as_ref().unwrap()[100], 0);
            assert_eq!(hmms[1].consensus.as_ref().unwrap()[100], b'2');
            assert_eq!(hmms[1].mat, clean[1].mat);
            assert_eq!(format!("{:?}", hmms[2]), format!("{:?}", clean[2]));
        }
        assert_eq!(read_first_hmm(&path).unwrap().unwrap().name, clean[0].name);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn hmmer_atoi() {
        for (token, value) in [
            ("\u{1b}[O", 0),
            ("341", 341),
            ("12ab", 12),
            ("-3", -3),
            ("-", 0),
        ] {
            assert_eq!(atoi(token), value, "{token:?}");
        }
    }

    /// An error HMMER would report too names the file, the line and the model, in
    /// whichever chunk it lies, with no control characters and no `Debug` wrapping.
    #[test]
    fn a_parse_error_says_where_it_is() {
        let path = edited_pdonr("bad-emission", 1050, |l| {
            l.replacen("2.31196", "a\u{7}c", 1)
        });
        let expected = format!(
            "{}:1050 (model APH): Bad HMM probability value: a\\u{{7}}c",
            path.display()
        );
        for result in [
            read_hmm_file_chunked(&path, 1, 2),
            read_hmm_file_serial(&path),
        ] {
            let error = result.unwrap_err();
            assert_eq!(error_message(&path, &error), expected);
        }
        std::fs::remove_file(&path).ok();
    }
}
