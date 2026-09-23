//! Line-ending detection and conversion for the GUI's text and table editors.
//!
//! The editors work in LF internally — a `<textarea>` normalizes its value to LF
//! whatever you put in it (HTML spec, "API value"), and the table editor splits on
//! newlines — so a file's original terminator has to be recorded on read and put
//! back on write. Without that, opening a CRLF file and pressing Save silently
//! rewrote every line ending in it.
//!
//! Pure functions, kept here rather than in the Tauri crate so `cargo test` covers
//! them: the GUI layer has no upstream to compare against, so a unit test is its
//! only gate.

use std::borrow::Cow;

/// The line terminator a text file uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Eol {
    /// `\n` — Unix, and the default for a file with no terminator at all.
    #[default]
    Lf,
    /// `\r\n` — Windows.
    CrLf,
    /// `\r` — classic Mac. Vanishingly rare, but the table editor already split on
    /// it, so a file using it would otherwise be rewritten as LF.
    Cr,
}

impl Eol {
    pub fn as_str(self) -> &'static str {
        match self {
            Eol::Lf => "\n",
            Eol::CrLf => "\r\n",
            Eol::Cr => "\r",
        }
    }

    /// Parse the wire form (the string `as_str` produces). Anything else is LF,
    /// so a caller that omits or garbles it gets today's behavior rather than an
    /// error in the middle of a save.
    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "\r\n" => Eol::CrLf,
            "\r" => Eol::Cr,
            _ => Eol::Lf,
        }
    }
}

/// Detect from the **first** terminator in `text`, which is what Git and most
/// editors do. A mixed file is therefore normalized to whichever came first, which
/// is the point: the file should be internally consistent after a save.
///
/// A file with no terminator has nothing to preserve and reports [`Eol::Lf`].
pub fn detect_eol(text: &str) -> Eol {
    match text.find(['\n', '\r']) {
        None => Eol::Lf,
        Some(i) => {
            if text.as_bytes()[i] == b'\n' {
                Eol::Lf
            } else if text.as_bytes().get(i + 1) == Some(&b'\n') {
                Eol::CrLf
            } else {
                Eol::Cr
            }
        }
    }
}

/// Normalize every terminator to LF. Borrows when there is nothing to change,
/// which is the common case (an LF file).
pub fn to_lf(text: &str) -> Cow<'_, str> {
    if !text.contains('\r') {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            // Swallow the LF of a CRLF pair so it does not become a blank line.
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    Cow::Owned(out)
}

/// Rewrite LF-only `text` with `eol`. The input is expected to have already been
/// through [`to_lf`]; a stray `\r` is left alone rather than being doubled.
pub fn from_lf(text: &str, eol: Eol) -> Cow<'_, str> {
    if eol == Eol::Lf || !text.contains('\n') {
        return Cow::Borrowed(text);
    }
    Cow::Owned(text.replace('\n', eol.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_the_first_terminator() {
        assert_eq!(detect_eol("a\nb\n"), Eol::Lf);
        assert_eq!(detect_eol("a\r\nb\r\n"), Eol::CrLf);
        assert_eq!(detect_eol("a\rb\r"), Eol::Cr);
        // No terminator at all: nothing to preserve.
        assert_eq!(detect_eol(""), Eol::Lf);
        assert_eq!(detect_eol("one line"), Eol::Lf);
        // A lone CR at the very end must not be read past the end of the buffer.
        assert_eq!(detect_eol("a\r"), Eol::Cr);
        // Mixed: the first one wins, and a save then makes the file consistent.
        assert_eq!(detect_eol("a\r\nb\nc"), Eol::CrLf);
        assert_eq!(detect_eol("a\nb\r\nc"), Eol::Lf);
    }

    #[test]
    fn to_lf_collapses_every_flavor() {
        assert_eq!(to_lf("a\r\nb\r\n"), "a\nb\n");
        assert_eq!(to_lf("a\rb\r"), "a\nb\n");
        assert_eq!(to_lf("a\nb\n"), "a\nb\n");
        assert_eq!(to_lf("a\r\nb\rc\nd"), "a\nb\nc\nd");
        // An LF-only string is returned borrowed, not rebuilt.
        assert!(matches!(to_lf("a\nb"), Cow::Borrowed(_)));
    }

    #[test]
    fn round_trip_preserves_the_original_terminator() {
        // The whole point: read then write must reproduce the input byte for byte
        // when nothing was edited.
        for original in ["a\nb\nc\n", "a\r\nb\r\nc\r\n", "a\rb\rc\r", "no newline", ""] {
            let eol = detect_eol(original);
            let lf = to_lf(original);
            let back = from_lf(&lf, eol);
            assert_eq!(back, original, "round trip failed for {original:?}");
        }
    }

    #[test]
    fn from_lf_leaves_a_stray_cr_alone() {
        // `from_lf` takes LF-normalized input; if a CR survives anyway, expanding
        // the LFs must not turn it into `\r\r\n`.
        assert_eq!(from_lf("a\nb", Eol::CrLf), "a\r\nb");
        assert_eq!(from_lf("a\nb", Eol::Lf), "a\nb");
        assert_eq!(from_lf("no newlines", Eol::CrLf), "no newlines");
    }

    #[test]
    fn multibyte_text_survives() {
        let s = "café\r\nnaïve\r\n🧬\r\n";
        assert_eq!(detect_eol(s), Eol::CrLf);
        let lf = to_lf(s);
        assert_eq!(lf, "café\nnaïve\n🧬\n");
        assert_eq!(from_lf(&lf, Eol::CrLf), s);
    }

    /// The round trip as the Tauri commands actually compose it, over a real file:
    /// `read_text_file` does `detect_eol` + `to_lf`, `write_text_file` does
    /// `from_lf`. Reading a file and saving it unedited must reproduce it byte for
    /// byte -- that is the whole contract, and it is the one the old code broke.
    #[test]
    fn a_file_read_then_saved_unedited_is_byte_identical() {
        let dir = std::env::temp_dir().join(format!("domainator-eol-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cases: [&[u8]; 7] = [
            b"col_a\tcol_b\r\n1\t2\r\n",    // CRLF table, the case that regressed
            b"col_a\tcol_b\n1\t2\n",        // LF table
            b"a\rb\rc\r",                   // classic Mac
            b"no trailing newline\r\nlast", // CRLF, no final terminator
            b"single line",                 // no terminator at all
            b"",                            // empty
            "caf\u{e9}\r\nna\u{ef}ve\r\n".as_bytes(), // multi-byte
        ];
        for (i, original) in cases.iter().enumerate() {
            let path = dir.join(format!("case{i}.tsv"));
            std::fs::write(&path, original).unwrap();

            // read_text_file
            let raw = std::fs::read_to_string(&path).unwrap();
            let eol = detect_eol(&raw);
            let buffer = to_lf(&raw).into_owned();
            // The editors split and rejoin on LF; nothing else may reach the buffer.
            assert!(!buffer.contains('\r'), "case {i}: CR survived into the editor buffer");

            // write_text_file, with nothing edited
            std::fs::write(&path, from_lf(&buffer, eol).as_bytes()).unwrap();
            assert_eq!(
                std::fs::read(&path).unwrap(),
                original.to_vec(),
                "case {i}: unedited save changed the file"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wire_form_round_trips() {
        for e in [Eol::Lf, Eol::CrLf, Eol::Cr] {
            assert_eq!(Eol::from_str_lossy(e.as_str()), e);
        }
        // An unknown or missing value degrades to LF rather than failing a save.
        assert_eq!(Eol::from_str_lossy("nonsense"), Eol::Lf);
        assert_eq!(Eol::from_str_lossy(""), Eol::Lf);
    }
}
