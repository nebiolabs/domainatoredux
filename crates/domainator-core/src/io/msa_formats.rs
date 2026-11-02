//! Multiple sequence alignment input as Easel reads it, and so as pyhmmer's `MSAFile`
//! and `hmmbuild` do: `esl_msafile_GuessFileFormat`'s format guess, and readers for
//! Clustal, PHYLIP, SELEX and PSI-BLAST that follow `esl_msafile_<format>.c` rule for
//! rule. Stockholm, aligned FASTA and A2M are read by hmmer-pure-rs, whose readers for
//! those agree with Easel; its readers for the other four split lines on whitespace,
//! which misreads PHYLIP's fixed-width names and SELEX's column layout.
//!
//! A reader returns text rows that `Msa::digitize` maps the way Easel's per-format
//! input map does in digital mode: characters the format ignores are dropped,
//! characters it remaps are rewritten (PHYLIP's `?` to `~`, SELEX's space to `-`), and
//! characters it refuses are errors. Where an Easel check has an oddity that decides
//! the outcome (the sequential-PHYLIP check never advancing its line counter, say), the
//! port keeps it, so the same file is read, or refused, the same way.

use std::path::Path;

use hmmer_pure_rs::msa::{
    read_a2m_from_reader, read_afa_from_reader, read_stockholm_from_reader, Msa,
};

/// An MSA file format, named as pyhmmer names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MsaFormat {
    Stockholm,
    Pfam,
    Afa,
    A2m,
    Clustal,
    ClustalLike,
    /// Interleaved PHYLIP, with the width of its name field.
    Phylip(usize),
    /// Sequential PHYLIP, with the width of its name field.
    Phylips(usize),
    Selex,
    Psiblast,
}

impl MsaFormat {
    /// pyhmmer's name for the format (`MSAFile.format`).
    pub fn name(self) -> &'static str {
        match self {
            MsaFormat::Stockholm => "stockholm",
            MsaFormat::Pfam => "pfam",
            MsaFormat::Afa => "afa",
            MsaFormat::A2m => "a2m",
            MsaFormat::Clustal => "clustal",
            MsaFormat::ClustalLike => "clustallike",
            MsaFormat::Phylip(_) => "phylip",
            MsaFormat::Phylips(_) => "phylips",
            MsaFormat::Selex => "selex",
            MsaFormat::Psiblast => "psiblast",
        }
    }
}

/// The residue symbols PHYLIP format checks count (`eslMSAFILE_PHYLIP_LEGALSYMS`).
const PHYLIP_LEGAL: &[u8] = b"-ABCDEFGHIJKLMNOPQRSTUVWZYX*?.";

/// One input line, as `esl_buffer_GetLine` returns it (no newline, no `\r` before
/// it), with its offset in the input.
#[derive(Clone, Copy)]
struct Line<'a> {
    bytes: &'a [u8],
    start: usize,
}

fn split_lines(text: &[u8]) -> Vec<Line<'_>> {
    let mut lines = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let end = text[start..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(text.len(), |i| start + i);
        let mut bytes = &text[start..end];
        if let [rest @ .., b'\r'] = bytes {
            bytes = rest;
        }
        lines.push(Line { bytes, start });
        start = end + 1;
    }
    lines
}

/// Easel's "blank line" idiom: nothing but spaces and tabs.
fn is_blank(line: &[u8]) -> bool {
    line.iter().all(|&b| b == b' ' || b == b'\t')
}

/// C's `isspace`.
fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

fn is_phylip_legal(b: u8) -> bool {
    PHYLIP_LEGAL.contains(&b)
}

/// `esl_memtok`: the next token of `s` delimited by any of `delim`, and the rest of `s`
/// after the token and the delimiters following it.
fn memtok<'a>(s: &'a [u8], delim: &[u8]) -> Option<(&'a [u8], &'a [u8])> {
    let so = s.iter().position(|b| !delim.contains(b))?;
    let xo = s[so..]
        .iter()
        .position(|b| delim.contains(b))
        .map_or(s.len(), |i| so + i);
    let eo = s[xo..]
        .iter()
        .position(|b| !delim.contains(b))
        .map_or(s.len(), |i| xo + i);
    Some((&s[so..xo], &s[eo..]))
}

/// `esl_mem_strtoi32` with base 0: leading whitespace and `-` allowed, `0x` hex, a
/// leading `0` octal, stopping at the first character that is not a digit of the base.
/// `None` when no digit was read or the value overflows an `i32`.
fn strtoi32(s: &[u8]) -> Option<i64> {
    let mut i = s.iter().take_while(|&&b| is_space(b)).count();
    let negative = s.get(i) == Some(&b'-');
    if negative {
        i += 1;
    }
    let mut ndigits = 0;
    let base: i64 = if s.get(i) == Some(&b'0') && s.get(i + 1) == Some(&b'x') && i + 1 < s.len() {
        i += 2;
        16
    } else if s.get(i) == Some(&b'0') {
        i += 1;
        ndigits += 1;
        8
    } else {
        10
    };
    let mut value: i64 = 0;
    for &b in &s[i.min(s.len())..] {
        let digit = match b {
            b'0'..=b'9' => i64::from(b - b'0'),
            b'A'..=b'Z' => 10 + i64::from(b - b'A'),
            b'a'..=b'z' => 10 + i64::from(b - b'a'),
            _ => break,
        };
        if digit >= base {
            break;
        }
        value = value * base + digit;
        if value > i64::from(i32::MAX) + i64::from(negative) {
            return None; // eslERANGE
        }
        ndigits += 1;
    }
    (ndigits > 0).then_some(if negative { -value } else { value })
}

/// The format a filename suffix names, as `esl_msafile_GuessFileFormat` reads it: a
/// trailing `.gz` is looked past, and the comparison is case-sensitive.
fn format_by_suffix(file_name: &str) -> Option<MsaFormat> {
    let base = Path::new(file_name).file_name()?.to_str()?;
    let extension = |name: &str| name.rfind('.').map(|i| name[i..].to_string());
    let mut suffix = extension(base)?;
    if suffix == ".gz" {
        suffix = extension(&base[..base.len() - 3])?;
    }
    Some(match suffix.as_str() {
        ".sto" | ".sth" | ".stk" => MsaFormat::Stockholm,
        ".afa" | ".afasta" => MsaFormat::Afa,
        ".pfam" => MsaFormat::Pfam,
        ".a2m" => MsaFormat::A2m,
        ".slx" | ".selex" => MsaFormat::Selex,
        ".pb" => MsaFormat::Psiblast,
        ".ph" | ".phy" | ".phyi" => MsaFormat::Phylip(10),
        ".phys" => MsaFormat::Phylips(10),
        _ => return None,
    })
}

/// Guess the format of the alignment in `text`, as `esl_msafile_GuessFileFormat` does:
/// from its first non-blank line, with the suffix of `file_name` (if any) deciding what
/// the content cannot (A2M from aligned FASTA, Pfam from Stockholm, sequential from
/// interleaved PHYLIP, PSI-BLAST from SELEX).
pub fn guess_format(text: &[u8], file_name: Option<&str>) -> Result<MsaFormat, String> {
    let by_suffix = file_name.and_then(format_by_suffix);
    let lines = split_lines(text);
    let Some(first) = lines.iter().find(|l| !is_blank(l.bytes)) else {
        return Err("can't guess alignment input format: empty file/no data".into());
    };
    let first = first.bytes;
    let contains = |needle: &[u8]| first.windows(needle.len()).any(|w| w == needle);
    let two_numbers = || {
        let all_digits = |t: &[u8]| t.iter().all(u8::is_ascii_digit);
        memtok(first, b" \t").is_some_and(|(tok, rest)| {
            all_digits(tok) && memtok(rest, b" \t").is_some_and(|(tok, _)| all_digits(tok))
        })
    };
    if first.starts_with(b"# STOCKHOLM") {
        Ok(match by_suffix {
            Some(MsaFormat::Pfam) => MsaFormat::Pfam,
            _ => MsaFormat::Stockholm,
        })
    } else if first.starts_with(b">") {
        // A2M needs a suffix to be told from aligned FASTA.
        Ok(match by_suffix {
            Some(MsaFormat::A2m) => MsaFormat::A2m,
            _ => MsaFormat::Afa,
        })
    } else if first.starts_with(b"CLUSTAL") {
        Ok(MsaFormat::Clustal)
    } else if contains(b"multiple sequence alignment") {
        Ok(MsaFormat::ClustalLike)
    } else if two_numbers() {
        match by_suffix {
            Some(format @ (MsaFormat::Phylip(_) | MsaFormat::Phylips(_))) => Ok(format),
            _ => phylip_check_file_format(text, &lines),
        }
    } else if by_suffix == Some(MsaFormat::Selex) {
        Ok(MsaFormat::Selex)
    } else if check_selex(&lines) {
        Ok(match by_suffix {
            Some(MsaFormat::Psiblast) => MsaFormat::Psiblast,
            _ => MsaFormat::Selex,
        })
    } else {
        Err("couldn't guess alignment input format - doesn't even look like selex".into())
    }
}

/// `msafile_check_selex`: two whitespace-separated fields on every data line, the same
/// sequence length within a block, and each block starting with the same name.
fn check_selex(lines: &[Line<'_>]) -> bool {
    let (mut block_nseq, mut nseq, mut block_nres) = (0usize, 0usize, 0usize);
    let mut first_name: &[u8] = &[];
    let (mut block_idx, mut in_block) = (0usize, false);
    for line in lines {
        let p = line.bytes;
        if [b"#=RF", b"#=CS", b"#=SS", b"#=SA"]
            .iter()
            .any(|tag| p.starts_with(*tag))
        {
            return true;
        }
        if p.starts_with(b"#") {
            continue;
        }
        if is_blank(p) {
            if block_nseq != 0 && block_nseq != nseq {
                return false;
            }
            if in_block {
                block_idx += 1;
            }
            if block_idx >= 3 {
                return true;
            }
            in_block = false;
            block_nres = 0;
            block_nseq = nseq;
            nseq = 0;
            continue;
        }
        in_block = true;
        let Some((name, rest)) = memtok(p, b" \t") else {
            return false;
        };
        if nseq == 0 {
            if block_idx == 0 {
                first_name = name;
            } else if name != first_name {
                return false;
            }
        }
        let Some((seq, rest)) = memtok(rest, b" \t") else {
            return false;
        };
        if block_nres != 0 && seq.len() != block_nres {
            return false;
        }
        block_nres = seq.len();
        if memtok(rest, b" \t").is_some() {
            return false;
        }
        nseq += 1;
    }
    if in_block {
        block_idx += 1;
    }
    block_idx > 0
}

// --------------------------------------------------------------------------
// PHYLIP format detection (esl_msafile_phylip.c, section 4)
// --------------------------------------------------------------------------

/// `phylip_parse_header`: `<nseq> <alen>` on the first non-blank line, and the index
/// of the first non-blank line after it.
fn phylip_parse_header(lines: &[Line<'_>]) -> Option<(i64, i64, usize)> {
    let header = lines.iter().position(|l| !is_blank(l.bytes))?;
    let (tok, rest) = memtok(lines[header].bytes, b" \t")?;
    let nseq = strtoi32(tok)?;
    let (tok, _) = memtok(rest, b" \t")?;
    let alen = strtoi32(tok)?;
    let first = (header + 1..lines.len()).find(|&i| !is_blank(lines[i].bytes))?;
    Some((nseq, alen, first))
}

/// `esl_msafile_phylip_CheckFileFormat`: whether the file is interleaved or
/// sequential PHYLIP, and its name width.
fn phylip_check_file_format(text: &[u8], lines: &[Line<'_>]) -> Result<MsaFormat, String> {
    let interleaved = phylip_check_interleaved(lines);
    if let Some((1, width)) = interleaved {
        return Ok(MsaFormat::Phylip(width));
    }
    let sequential = if phylip_check_sequential_known(lines, 10) {
        Some(10)
    } else {
        phylip_check_sequential_unknown(text, lines)
    };
    match (interleaved, sequential) {
        (Some(_), Some(_)) => {
            Err("can't guess format: it's consistent w/ both phylip, phylips.".into())
        }
        (Some((_, width)), None) => Ok(MsaFormat::Phylip(width)),
        (None, Some(width)) => Ok(MsaFormat::Phylips(width)),
        (None, None) => Err("format unrecognized, though it looks phylip-like".into()),
    }
}

/// `phylip_check_interleaved`: the number of blocks and the name width, when the file
/// reads as interleaved PHYLIP. Each column of a block is classed by `collate_colcodes`;
/// the name width is where the first block's alignment columns must start for the
/// blocks to add up to the stated length.
fn phylip_check_interleaved(lines: &[Line<'_>]) -> Option<(usize, usize)> {
    let (nseq, alen, first) = phylip_parse_header(lines)?;
    if nseq <= 0 {
        return None; // Easel would read the same line forever
    }
    let mut next = first;
    let mut current = Some(lines[first].bytes);
    let get_line = |next: &mut usize| {
        *next += 1;
        lines.get(*next).map(|l| l.bytes)
    };
    let (mut nblocks, mut nres1, mut nres2) = (0usize, 0i64, 0i64);
    let mut colcodes0: Vec<u8> = Vec::new();
    while let Some(block_start) = current {
        let mut colcodes = vec![b'?'; block_start.len()];
        for _ in 0..nseq {
            let p = current?; // EOF inside a block
            if !collate_colcodes(p, &mut colcodes) {
                return None;
            }
            current = get_line(&mut next);
        }
        if nblocks == 0 {
            // Speculate that it is strict PHYLIP (name width 10): then the blocks can
            // be stopped at the stated length.
            nres1 = colcodes.iter().skip(10).filter(|&&c| c != b'.').count() as i64;
            colcodes0 = colcodes;
        } else {
            for &c in &colcodes {
                match c {
                    b'x' => nres2 += 1,
                    b'n' => return None,
                    _ => {}
                }
            }
        }
        nblocks += 1;
        if nres1 + nres2 == alen {
            break;
        }
        // Easel skips only lines of tabs between blocks here (its `memspn(p, n, "\t")`),
        // so a line of spaces starts the next block.
        while let Some(p) = current {
            if !p.iter().all(|&b| b == b'\t') {
                break;
            }
            current = get_line(&mut next);
        }
    }
    let width = if nres1 + nres2 == alen {
        10
    } else {
        phylip_deduce_namewidth(&colcodes0, alen, nres2)?
    };
    Some((nblocks, width))
}

/// `phylip_collate_colcodes`: update each column's class with this line. `x` is
/// residues only, `.` spaces only, `o` spaces or other symbols, `n` residues mixed with
/// either (so a name column).
fn collate_colcodes(p: &[u8], colcodes: &mut [u8]) -> bool {
    let ncols = colcodes.len();
    for (c, &b) in p.iter().enumerate().take(ncols) {
        let code = &mut colcodes[c];
        if is_phylip_legal(b) {
            *code = match *code {
                b'?' => b'x',
                b'.' => b'n',
                other => other,
            };
        } else if b == b' ' {
            *code = match *code {
                b'?' => b'.',
                b'x' => b'n',
                other => other,
            };
        } else if b.is_ascii_graphic() {
            *code = match *code {
                b'?' | b'.' => b'o',
                b'x' => b'n',
                other => other,
            };
        } else {
            return false;
        }
    }
    !p.iter().skip(ncols).any(|&b| is_phylip_legal(b))
}

/// `phylip_deduce_namewidth`: the first block holds the columns the later blocks do
/// not, so its last `alen - nres2` residue columns are alignment and what precedes them
/// is the name (10 wide if that fits, else as narrow as the name allows).
fn phylip_deduce_namewidth(colcodes0: &[u8], alen: i64, nres2: i64) -> Option<usize> {
    let mut nres1 = alen - nres2;
    if nres1 <= 0 {
        return None;
    }
    // Walk left from the end; `c` is one past the column looked at next.
    let mut c = colcodes0.len();
    while nres1 > 0 && c > 0 {
        if colcodes0[c - 1] == b'x' {
            nres1 -= 1;
        }
        c -= 1;
    }
    if nres1 > 0 {
        return None;
    }
    let widest = c;
    while c > 0 && colcodes0[c - 1] == b'.' {
        c -= 1;
    }
    let narrowest = c;
    Some(if narrowest <= 10 && widest >= 10 {
        10
    } else {
        narrowest
    })
}

/// `phylip_check_sequential_known`: whether each sequence's lines hold exactly the
/// stated number of residues after `width` name columns. Easel never advances the line
/// counter this check keys on, so the name columns are skipped on every line, a
/// continuation line's included; ported as it is.
fn phylip_check_sequential_known(lines: &[Line<'_>], width: usize) -> bool {
    let Some((nseq, alen, first)) = phylip_parse_header(lines) else {
        return false;
    };
    let mut next = first;
    for _ in 0..nseq {
        let mut nres = 0i64;
        while nres < alen {
            let Some(line) = lines.get(next) else {
                return false;
            };
            nres += line
                .bytes
                .iter()
                .skip(width)
                .filter(|&&b| is_phylip_legal(b))
                .count() as i64;
            next += 1;
        }
        if nres != alen {
            return false;
        }
        while lines
            .get(next)
            .is_some_and(|l| l.bytes.iter().all(|&b| b == b'\t'))
        {
            next += 1;
        }
    }
    true
}

/// `phylip_check_sequential_unknown`: sequential PHYLIP with a name width other than
/// 10. The first sequence fixes the width (its lines past the first hold `b` residues,
/// so its first line must hold `alen - b`), and up to 100 more are checked against it.
fn phylip_check_sequential_unknown(text: &[u8], lines: &[Line<'_>]) -> Option<usize> {
    // Easel indexes a line past its end here, reading on into the buffer.
    let at = |line: &Line<'_>, i: usize| text.get(line.start + i).copied().unwrap_or(0);
    let count_legal = |bytes: &[u8]| bytes.iter().filter(|&&b| is_phylip_legal(b)).count() as i64;

    let nlines = lines.iter().filter(|l| !is_blank(l.bytes)).count() as i64 - 1;
    let (nseq, alen, first) = phylip_parse_header(lines)?;
    if nseq <= 0 || nlines % nseq != 0 {
        return None;
    }
    let nblocks = nlines / nseq;
    let line1 = lines[first];
    let l1 = line1.bytes.len();
    let mut rth = vec![0i64; l1];
    let mut r = 0i64;
    for i in (0..l1).rev() {
        if is_phylip_legal(line1.bytes[i]) {
            r += 1;
            rth[i] = r;
        }
    }
    let mut next = first;
    let next_data_line = |next: &mut usize| -> Option<Line<'_>> {
        *next += 1;
        while lines.get(*next).is_some_and(|l| is_blank(l.bytes)) {
            *next += 1;
        }
        lines.get(*next).copied()
    };
    let mut current = line1;
    let mut b = 0i64;
    for _ in 1..nblocks {
        current = next_data_line(&mut next)?;
        b += count_legal(current.bytes);
    }
    let a = alen - b;
    if a > r {
        return None;
    }
    let width = rth.iter().position(|&n| n == a)?;
    // Easel checks for a name on the line it read last, not the first line.
    if (0..width).all(|i| is_space(at(&current, i))) {
        return None;
    }
    for _ in 1..nseq.min(100) {
        current = next_data_line(&mut next)?;
        if !is_phylip_legal(at(&current, width)) || (0..width).all(|i| is_space(at(&current, i))) {
            return None;
        }
        let mut len2 = count_legal(current.bytes.get(width..).unwrap_or_default());
        for _ in 1..nblocks {
            current = next_data_line(&mut next)?;
            len2 += count_legal(current.bytes);
        }
        if len2 != alen {
            return None;
        }
    }
    Some(width)
}

// --------------------------------------------------------------------------
// Readers
// --------------------------------------------------------------------------

/// What a reader collects: rows of text, in file order, and the column annotations the
/// profile builder carries into the model.
#[derive(Default)]
struct Rows {
    names: Vec<String>,
    seqs: Vec<Vec<u8>>,
    rf: Option<Vec<u8>>,
    mm: Option<Vec<u8>>,
    ss_cons: Option<Vec<u8>>,
}

impl Rows {
    fn into_msa(self) -> Result<Msa, String> {
        let nseq = self.seqs.len();
        if nseq == 0 {
            return Err("the input alignment has no sequences".into());
        }
        let alen = self.seqs[0].len();
        if let Some(i) = self.seqs.iter().position(|s| s.len() != alen) {
            return Err(format!(
                "sequence {} has aligned length {}, expected {alen}",
                self.names[i],
                self.seqs[i].len()
            ));
        }
        Ok(Msa {
            name: "msa".into(),
            acc: None,
            desc: None,
            author: None,
            sqname: self.names,
            sqacc: vec![String::new(); nseq],
            sqdesc: vec![String::new(); nseq],
            weights: None,
            aseq: self.seqs,
            pp: vec![None; nseq],
            nseq,
            alen,
            rf: self.rf,
            mm: self.mm,
            ss_cons: self.ss_cons,
            sa_cons: None,
            pp_cons: None,
        })
    }
}

/// Read the first alignment in `text`, in `format`.
pub fn read_msa(text: &[u8], format: MsaFormat) -> Result<Msa, String> {
    let first = |msas: Vec<Msa>| {
        msas.into_iter()
            .next()
            .ok_or_else(|| "no alignment found in the input MSA".to_string())
    };
    let crate_error = |e: hmmer_pure_rs::errors::HmmerError| format!("{e}");
    let lines = split_lines(text);
    match format {
        MsaFormat::Stockholm | MsaFormat::Pfam => {
            first(read_stockholm_from_reader(std::io::BufReader::new(text)).map_err(crate_error)?)
        }
        MsaFormat::Afa => {
            first(read_afa_from_reader(&mut &text[..], "msa".into()).map_err(crate_error)?)
        }
        MsaFormat::A2m => {
            first(read_a2m_from_reader(&mut &text[..], "msa".into()).map_err(crate_error)?)
        }
        MsaFormat::Clustal | MsaFormat::ClustalLike => {
            read_clustal(&lines, format == MsaFormat::Clustal)?.into_msa()
        }
        MsaFormat::Phylip(width) => read_phylip(&lines, width, false)?.into_msa(),
        MsaFormat::Phylips(width) => read_phylip(&lines, width, true)?.into_msa(),
        MsaFormat::Selex => read_selex(&lines)?.into_msa(),
        MsaFormat::Psiblast => read_psiblast(&lines)?.into_msa(),
    }
}

/// The part of reading that waits on the alphabet: PHYLIP reads `O` as a gap in a
/// nucleotide alignment (`esl_msafile_phylip_SetInmap`).
pub fn apply_alphabet(msa: &mut Msa, format: MsaFormat, nucleic: bool) {
    if nucleic && matches!(format, MsaFormat::Phylip(_) | MsaFormat::Phylips(_)) {
        for seq in &mut msa.aseq {
            for b in seq.iter_mut().filter(|b| **b == b'O') {
                *b = b'-';
            }
        }
    }
}

fn utf8(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// A cursor over the input lines, as `esl_msafile_GetLine` walks them.
struct Cursor<'a, 'b> {
    lines: &'b [Line<'a>],
    next: usize,
}

impl<'a> Cursor<'a, '_> {
    fn get(&mut self) -> Option<&'a [u8]> {
        let line = self.lines.get(self.next)?;
        self.next += 1;
        Some(line.bytes)
    }

    /// The next non-blank line.
    fn get_nonblank(&mut self) -> Option<&'a [u8]> {
        loop {
            let line = self.get()?;
            if !is_blank(line) {
                return Some(line);
            }
        }
    }
}

/// `esl_msafile_clustal_Read`: blocks of `name sequence [count]` lines, each ended by a
/// consensus line (only ` .:*`, which an empty line also is).
fn read_clustal(lines: &[Line<'_>], strict: bool) -> Result<Rows, String> {
    let mut cursor = Cursor { lines, next: 0 };
    let header = cursor.get_nonblank().ok_or("the input MSA is empty")?;
    let missing = || "missing CLUSTAL header".to_string();
    let (tok, rest) = memtok(header, b" \t").ok_or_else(missing)?;
    if (strict && !tok.starts_with(b"CLUSTAL")) || !rest.windows(9).any(|w| w == b"alignment") {
        return Err(missing());
    }
    let mut p = cursor
        .get_nonblank()
        .ok_or("no alignment data following header")?;
    let mut rows = Rows::default();
    let mut nblocks = 0;
    loop {
        let mut idx = 0;
        let (mut block_start, mut block_len) = (0, 0);
        loop {
            let n = p.len();
            let mut pos = p.iter().position(|&b| !is_space(b)).unwrap_or(n);
            let name_start = pos;
            pos = (pos + 1..n)
                .find(|&i| is_space(p[i]))
                .unwrap_or(n.max(pos + 1));
            let name = &p[name_start.min(n)..pos.min(n)];
            pos = (pos + 1..n)
                .find(|&i| !is_space(p[i]))
                .unwrap_or(n.max(pos + 1));
            let seq_start = pos;
            if pos >= n {
                return Err("invalid alignment line".into());
            }
            pos = (pos + 1..n).find(|&i| is_space(p[i])).unwrap_or(n);
            let seq = &p[seq_start..pos];
            if idx == 0 {
                (block_start, block_len) = (seq_start, seq.len());
            } else if seq_start != block_start {
                return Err("sequence start is misaligned".into());
            } else if seq.len() != block_len {
                return Err("sequence end is misaligned".into());
            }
            if nblocks == 0 {
                rows.names.push(utf8(name));
                rows.seqs.push(seq.to_vec());
            } else {
                match rows.names.get(idx) {
                    Some(expected) if expected.as_bytes() == name => {}
                    expected => {
                        return Err(format!(
                            "expected sequence {} on this line, but saw {}",
                            expected.map_or("<none>", String::as_str),
                            utf8(name)
                        ))
                    }
                }
                rows.seqs[idx].extend_from_slice(seq);
            }
            p = cursor
                .get()
                .ok_or("alignment block did not end with consensus line")?;
            idx += 1;
            if p.iter().all(|b| b" .:*".contains(b)) {
                break;
            }
        }
        if idx != rows.names.len() {
            return Err("last block didn't contain same # of seqs as earlier blocks".into());
        }
        nblocks += 1;
        match cursor.get_nonblank() {
            Some(line) => p = line,
            None => return Ok(rows),
        }
    }
}

/// `phylip_rectify_input_name`: leading and trailing spaces dropped, inner ones
/// turned into `_`; a tab or other non-printing character is an error.
fn phylip_name(field: &[u8]) -> Result<String, String> {
    let end = field.iter().rposition(|&b| b != b' ').unwrap_or(0);
    let start = field.iter().position(|&b| b != b' ').unwrap_or(field.len());
    let mut name = Vec::new();
    for &b in field.iter().take(end + 1).skip(start) {
        if !b.is_ascii_graphic() && b != b' ' {
            return Err("invalid character(s) in sequence name".into());
        }
        name.push(if b == b' ' { b'_' } else { b });
    }
    Ok(utf8(&name))
}

/// Append `p`'s residues to `row` through PHYLIP's input map: spaces, tabs and the
/// digits `0`-`8` are ignored (Easel's loop stops short of `9`), `?` is missing data,
/// and `~` and `_` are refused. Returns how many were appended.
fn phylip_append(row: &mut Vec<u8>, p: &[u8]) -> Result<usize, String> {
    let before = row.len();
    for &b in p {
        match b {
            b' ' | b'\t' | b'0'..=b'8' => {}
            b'?' => row.push(b'~'),
            b'~' | b'_' => return Err("one or more invalid sequence characters".into()),
            _ => row.push(b),
        }
    }
    Ok(row.len() - before)
}

/// `esl_msafile_phylip_Read`: the `<nseq> <alen>` header, then the sequences
/// interleaved in blocks or one after another, names in the first `width` columns.
fn read_phylip(lines: &[Line<'_>], width: usize, sequential: bool) -> Result<Rows, String> {
    let mut cursor = Cursor { lines, next: 0 };
    let header = cursor.get_nonblank().ok_or("the input MSA is empty")?;
    let bad_header = |what: &str| format!("first PHYLIP line should be <nseq> <alen>: {what}");
    let (tok, rest) =
        memtok(header, b" \t").ok_or_else(|| bad_header("first field isn't an integer"))?;
    let nseq = strtoi32(tok).ok_or_else(|| bad_header("first field isn't an integer"))?;
    let (tok, _) = memtok(rest, b" \t").ok_or_else(|| bad_header("only one field found"))?;
    let alen_stated = strtoi32(tok).ok_or_else(|| bad_header("second field isn't an integer"))?;
    let Ok(nseq @ 1..) = usize::try_from(nseq) else {
        return Err("the input alignment has no sequences".into());
    };
    let first = cursor
        .get_nonblank()
        .ok_or("no alignment data following PHYLIP header")?;
    let mut rows = Rows {
        names: vec![String::new(); nseq],
        seqs: vec![Vec::new(); nseq],
        ..Rows::default()
    };
    let take_name = |p: &[u8]| -> Result<String, String> {
        if p.len() < width {
            return Err("PHYLIP line too short to find sequence name".into());
        }
        phylip_name(&p[..width])
    };
    let mut current = Some(first);
    if sequential {
        for idx in 0..nseq {
            let mut alen = 0i64;
            while let Some(mut p) = current.filter(|_| alen < alen_stated) {
                if alen == 0 {
                    rows.names[idx] = take_name(p)?;
                    p = &p[width..];
                }
                alen += phylip_append(&mut rows.seqs[idx], p)? as i64;
                current = cursor.get();
            }
            while current.is_some_and(is_blank) {
                current = cursor.get();
            }
            match current {
                None if idx + 1 < nseq => {
                    return Err(format!(
                        "premature end of file: header said to expect {nseq} sequences"
                    ))
                }
                None => {}
                Some(_) if alen != alen_stated => {
                    return Err(format!(
                        "aligned length of sequence disagrees with header: header says \
                         {alen_stated}, parsed {alen}"
                    ))
                }
                Some(_) => {}
            }
        }
        return Ok(rows);
    }
    let (mut alen, mut nblocks) = (0i64, 0);
    loop {
        let mut idx = 0;
        let mut block_alen = 0;
        while let Some(mut p) = current {
            if nblocks == 0 {
                rows.names[idx] = take_name(p)?;
                p = &p[width..];
            }
            let added = phylip_append(&mut rows.seqs[idx], p)? as i64;
            if idx == 0 {
                block_alen = added;
            } else if added != block_alen {
                return Err("number of residues on line differs from previous seqs in \
                            alignment block"
                    .into());
            }
            idx += 1;
            current = cursor.get();
            if idx >= nseq || current.is_none_or(is_blank) {
                break;
            }
        }
        if idx != nseq {
            return Err(format!(
                "unexpected number of sequences in block (saw {idx}, expected {nseq})"
            ));
        }
        nblocks += 1;
        alen += block_alen;
        while current.is_some_and(is_blank) {
            current = cursor.get();
        }
        if current.is_none() || alen >= alen_stated {
            break;
        }
    }
    // A line after the last block starts another alignment (a seqboot file).
    if current.is_none() && alen != alen_stated {
        return Err(format!(
            "alignment length disagrees with header: header said {alen_stated}, parsed {alen}"
        ));
    }
    Ok(rows)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SelexLine {
    Rf,
    Mm,
    Cs,
    Ss,
    Sa,
    Seq,
}

fn selex_line_type(p: &[u8]) -> SelexLine {
    match p.get(..4) {
        Some(b"#=RF") => SelexLine::Rf,
        Some(b"#=MM") => SelexLine::Mm,
        Some(b"#=CS") => SelexLine::Cs,
        Some(b"#=SS") => SelexLine::Ss,
        Some(b"#=SA") => SelexLine::Sa,
        _ => SelexLine::Seq,
    }
}

/// `esl_msafile_selex_Read`: blocks of `name sequence` lines, laid out by column, so a
/// space inside a sequence is a gap and a short line is padded with gaps. `#` lines are
/// comments, except the `#=RF`, `#=MM`, `#=CS`, `#=SS` and `#=SA` annotations.
fn read_selex(lines: &[Line<'_>]) -> Result<Rows, String> {
    let is_comment = |p: &[u8]| p.starts_with(b"#") && !p.starts_with(b"#=");
    let mut rows = Rows::default();
    let mut types: Vec<SelexLine> = Vec::new();
    let mut alen = 0usize;
    let mut i = 0;
    loop {
        while i < lines.len() && (is_blank(lines[i].bytes) || is_comment(lines[i].bytes)) {
            i += 1;
        }
        if i >= lines.len() {
            break;
        }
        let mut block: Vec<&[u8]> = Vec::new();
        while i < lines.len() && !is_blank(lines[i].bytes) {
            if !is_comment(lines[i].bytes) {
                block.push(lines[i].bytes);
            }
            i += 1;
        }
        let first_block = types.is_empty();
        if !first_block && block.len() != types.len() {
            return Err(format!(
                "expected {} lines in block, saw {}",
                types.len(),
                block.len()
            ));
        }
        let mut lpos: Vec<Option<usize>> = Vec::with_capacity(block.len());
        let mut seqi = 0;
        let (mut nrf, mut ncs, mut nss, mut nsa, mut nseq) = (0, 0, 0, 0, 0);
        for (idx, &p) in block.iter().enumerate() {
            let kind = selex_line_type(p);
            if first_block {
                match kind {
                    SelexLine::Rf => nrf += 1,
                    SelexLine::Cs => ncs += 1,
                    SelexLine::Ss => nss += 1,
                    SelexLine::Sa => nsa += 1,
                    SelexLine::Mm => {}
                    SelexLine::Seq => {
                        nseq += 1;
                        nss = 0;
                        nsa = 0;
                    }
                }
                if nss > 0 && nseq == 0 {
                    return Err("#=SS must follow a sequence".into());
                }
                if nsa > 0 && nseq == 0 {
                    return Err("#=SA must follow a sequence".into());
                }
                if nrf > 1 {
                    return Err("Too many #=RF lines for block".into());
                }
                if ncs > 1 {
                    return Err("Too many #=CS lines for block".into());
                }
                if nss > 1 {
                    return Err("Too many #=SS lines for seq".into());
                }
                if nsa > 1 {
                    return Err("Too many #=SA lines for seq".into());
                }
                types.push(kind);
            } else if types[idx] != kind {
                return Err(match kind {
                    SelexLine::Seq => "sequence line isn't in expected order in block".into(),
                    _ => format!("{} line isn't in expected order in block", utf8(&p[..4])),
                });
            }
            let (tok, rest) = memtok(p, b" \t").ok_or("blank line in a SELEX block")?;
            if kind == SelexLine::Seq {
                if first_block {
                    rows.names.push(utf8(tok));
                    rows.seqs.push(Vec::new());
                } else if rows.names[seqi].as_bytes() != tok {
                    return Err(format!(
                        "expected sequence {} at this line of block",
                        rows.names[seqi]
                    ));
                }
                seqi += 1;
            }
            lpos.push((!rest.is_empty()).then(|| p.len() - rest.len()));
        }

        // selex_append_block. A line's data runs from `lpos` to its last non-space
        // character (`rpos`; for a line with no data, that is the end of its name, which
        // Easel counts too). Easel seeds the block's left edge with the first line's
        // `lpos` even when that line has no data (-1, so every line shifts one column
        // right); ported as it is, with `None` for -1.
        let rpos: Vec<Option<usize>> = block
            .iter()
            .map(|p| p.iter().rposition(|&b| !is_space(b)))
            .collect();
        let leftmost = lpos[0].map(|first| lpos.iter().flatten().fold(first, |a, &b| a.min(b)));
        let Some(rightmost) = rpos.iter().flatten().copied().max() else {
            continue; // no sequence or annotation data in this block at all
        };
        let left_of = |l: usize| leftmost.map_or(l + 1, |m| l - m);
        let nadd = match leftmost {
            Some(m) => rightmost + 1 - m,
            None => rightmost + 2,
        };
        let mut seqi = 0;
        for (idx, &p) in block.iter().enumerate() {
            let (nleft, text) = match (lpos[idx], rpos[idx]) {
                (Some(l), Some(r)) => (left_of(l), &p[l..=r]),
                _ => (nadd, &p[..0]),
            };
            let target = match types[idx] {
                SelexLine::Seq => {
                    let row = &mut rows.seqs[seqi];
                    seqi += 1;
                    row.resize(alen + nleft, b'-');
                    for &b in text {
                        match b {
                            b' ' => row.push(b'-'),
                            b'\t' => return Err("illegal residue(s) in sequence line".into()),
                            _ => row.push(b),
                        }
                    }
                    row.resize(alen + nadd, b'-');
                    continue;
                }
                SelexLine::Rf => rows.rf.get_or_insert_with(Vec::new),
                SelexLine::Mm => rows.mm.get_or_insert_with(Vec::new),
                SelexLine::Cs => rows.ss_cons.get_or_insert_with(Vec::new),
                // Per-sequence structure and accessibility do not reach a profile.
                SelexLine::Ss | SelexLine::Sa => continue,
            };
            target.resize(alen + nleft, b'.');
            target.extend_from_slice(text);
            target.resize(alen + nadd, b'.');
        }
        alen += nadd;
    }
    if types.is_empty() {
        return Err("no alignment found in the input MSA".into());
    }
    Ok(rows)
}

/// `esl_msafile_psiblast_Read`: blocks of `name sequence` lines with `-` gaps, where
/// case marks the columns: upper case consensus, lower case insert. That becomes the
/// alignment's RF line.
fn read_psiblast(lines: &[Line<'_>]) -> Result<Rows, String> {
    let mut cursor = Cursor { lines, next: 0 };
    let mut p = cursor.get_nonblank().ok_or("the input MSA is empty")?;
    let mut rows = Rows::default();
    let mut rf: Vec<u8> = Vec::new();
    let (mut alen, mut nblocks) = (0usize, 0);
    loop {
        let mut idx = 0;
        let (mut block_start, mut block_len) = (0, 0);
        loop {
            let n = p.len();
            let mut pos = p.iter().position(|&b| !is_space(b)).unwrap_or(n);
            let name_start = pos;
            pos = (pos + 1..n)
                .find(|&i| is_space(p[i]))
                .unwrap_or(n.max(pos + 1));
            let name = &p[name_start.min(n)..pos.min(n)];
            pos = (pos + 1..n)
                .find(|&i| !is_space(p[i]))
                .unwrap_or(n.max(pos + 1));
            let seq_start = pos;
            if pos >= n {
                return Err("invalid alignment line".into());
            }
            let seq_end = (1..n).rev().find(|&i| !is_space(p[i])).unwrap_or(0);
            let seq = &p[seq_start..=seq_end.max(seq_start)];
            if idx == 0 {
                (block_start, block_len) = (seq_start, seq.len());
                rf.resize(alen + seq.len(), b'-');
            } else if seq_start != block_start {
                return Err("sequence start is misaligned".into());
            } else if seq.len() != block_len {
                return Err("sequence end is misaligned".into());
            }
            for (i, &b) in seq.iter().enumerate() {
                let column = &mut rf[alen + i];
                if b.is_ascii_uppercase() {
                    if *column == b'.' {
                        return Err(format!(
                            "unexpected upper case residue (#{} on line)",
                            i + 1
                        ));
                    }
                    *column = b'x';
                } else if b.is_ascii_lowercase() {
                    if *column == b'x' {
                        return Err(format!(
                            "unexpected lower case residue (#{} on line)",
                            i + 1
                        ));
                    }
                    *column = b'.';
                }
            }
            if seq
                .iter()
                .any(|b| b".-_*~Oo".contains(b) && *b != b'-' || is_space(*b))
            {
                return Err("one or more invalid sequence characters".into());
            }
            if nblocks == 0 {
                rows.names.push(utf8(name));
                rows.seqs.push(seq.to_vec());
            } else {
                match rows.names.get(idx) {
                    Some(expected) if expected.as_bytes() == name => {}
                    expected => {
                        return Err(format!(
                            "expected sequence {} on this line, but saw {}",
                            expected.map_or("<none>", String::as_str),
                            utf8(name)
                        ))
                    }
                }
                rows.seqs[idx].extend_from_slice(seq);
            }
            idx += 1;
            match cursor.get() {
                Some(line) if !is_blank(line) => p = line,
                _ => break,
            }
        }
        if nblocks > 0 && idx != rows.names.len() {
            return Err("last block didn't contain same # of seqs as earlier blocks".into());
        }
        alen += block_len;
        nblocks += 1;
        match cursor.get_nonblank() {
            Some(line) => p = line,
            None => break,
        }
    }
    rows.rf = Some(rf);
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guess(text: &str, name: Option<&str>) -> Result<MsaFormat, String> {
        guess_format(text.as_bytes(), name)
    }

    fn read(text: &str, name: Option<&str>) -> Msa {
        let format = guess(text, name).unwrap();
        read_msa(text.as_bytes(), format).unwrap()
    }

    fn rows(msa: &Msa) -> Vec<String> {
        msa.aseq.iter().map(|s| utf8(s)).collect()
    }

    #[test]
    fn the_first_line_decides_and_the_suffix_breaks_ties() {
        let afa = ">a\nAC-GT\n>b\nACAGT\n";
        assert_eq!(guess(afa, None), Ok(MsaFormat::Afa));
        assert_eq!(guess(afa, Some("x.afa")), Ok(MsaFormat::Afa));
        assert_eq!(guess(afa, Some("dir/x.a2m")), Ok(MsaFormat::A2m));
        assert_eq!(guess(afa, Some("x.a2m.gz")), Ok(MsaFormat::A2m));
        assert_eq!(guess(afa, Some("x.A2M")), Ok(MsaFormat::Afa));
        // The suffix never overrides the first line.
        assert_eq!(guess(afa, Some("x.phy")), Ok(MsaFormat::Afa));
        let sto = "# STOCKHOLM 1.0\na AC\n//\n";
        assert_eq!(guess(sto, Some("x.pfam")), Ok(MsaFormat::Pfam));
        assert_eq!(guess(sto, Some("x.txt")), Ok(MsaFormat::Stockholm));
        assert_eq!(
            guess("\n  \nCLUSTAL W (1.83) multiple sequence alignment\n", None),
            Ok(MsaFormat::Clustal)
        );
        assert_eq!(
            guess("MUSCLE (3.8) multiple sequence alignment\n", None),
            Ok(MsaFormat::ClustalLike)
        );
        assert_eq!(
            guess("2 4\nx     ACGT\ny     ACGA\n", Some("a.phys")),
            Ok(MsaFormat::Phylips(10))
        );
        let slx = "a ACGT\nb ACGA\n";
        assert_eq!(guess(slx, None), Ok(MsaFormat::Selex));
        assert_eq!(guess(slx, Some("x.pb")), Ok(MsaFormat::Psiblast));
        assert!(guess("\n \t\n", None).unwrap_err().contains("empty"));
        assert!(guess("hello world foo\nbar\n", None)
            .unwrap_err()
            .contains("doesn't even look like selex"));
        // A .pb suffix does not rescue content that is not SELEX-like.
        assert!(guess("hello world foo\n", Some("x.pb")).is_err());
    }

    #[test]
    fn phylip_names_are_ten_columns_and_may_hold_spaces() {
        let text =
            "  3    20\nTurkey    AAGCTNGGGC\nSalmo gairAAGCCTTGGC\nH. SapiensACCGGTTGGC\n\n\
                    GAGCCCGGGC\nGAGCCGTGGC\nACAGGTTGGC\n";
        assert_eq!(guess(text, None), Ok(MsaFormat::Phylip(10)));
        let msa = read(text, None);
        assert_eq!(msa.sqname, ["Turkey", "Salmo_gair", "H._Sapiens"]);
        assert_eq!(rows(&msa)[1], "AAGCCTTGGCGAGCCGTGGC");
    }

    #[test]
    fn a_wider_phylip_name_field_is_deduced() {
        let text = "3 12\nalpha_long_name ACGTACGTACGT\nbeta_long_name2 ACGTACGAACGT\n\
                    gamma_long_id_3 ACGTTCGAACGT\n";
        assert_eq!(guess(text, None), Ok(MsaFormat::Phylip(15)));
        let msa = read(text, None);
        assert_eq!(msa.sqname[2], "gamma_long_id_3");
        assert_eq!(rows(&msa)[2], "ACGTTCGAACGT");
    }

    #[test]
    fn sequential_phylip_is_told_from_interleaved() {
        let text = "  2    24\nTurkey    AAGCTNGGGC AT\nGAGCCCGGGC AA\nSalmo gairAAGCCTTGGC AG\n\
                    GAGCCGTGGC CG\n";
        assert_eq!(guess(text, None), Ok(MsaFormat::Phylips(10)));
        let msa = read(text, None);
        assert_eq!(
            rows(&msa),
            ["AAGCTNGGGCATGAGCCCGGGCAA", "AAGCCTTGGCAGGAGCCGTGGCCG"]
        );
    }

    #[test]
    fn phylip_maps_missing_data_and_drops_digits() {
        let mut msa = read("2 5\nx         AC?GT 5\ny         ACOGT 5\n", None);
        assert_eq!(rows(&msa), ["AC~GT", "ACOGT"]);
        apply_alphabet(&mut msa, MsaFormat::Phylip(10), true);
        assert_eq!(rows(&msa)[1], "AC-GT");
        let err = read_msa(
            b"2 5\nx         AC_GT\ny         ACAGT\n",
            MsaFormat::Phylip(10),
        );
        assert!(err.unwrap_err().contains("invalid sequence characters"));
    }

    #[test]
    fn selex_reads_by_column() {
        let text =
            "#=RF      xxx.xxxx\nseqA      MKV.LLAG\nseqB      MKVALL.G\nseqC      MRV LLAG\n\n\
                    # a comment\n#=RF      xxxx\nseqA      WWHK\nseqB      WWHK\nseqC      WF\n";
        let msa = read(text, None);
        assert_eq!(rows(&msa), ["MKV.LLAGWWHK", "MKVALL.GWWHK", "MRV-LLAGWF--"]);
        assert_eq!(msa.rf.as_deref(), Some(&b"xxx.xxxxxxxx"[..]));
    }

    #[test]
    fn selex_blocks_must_match() {
        let err = read_msa(b"a ACGT\nb ACGT\n\na AC\n", MsaFormat::Selex).unwrap_err();
        assert!(err.contains("expected 2 lines in block, saw 1"), "{err}");
        let err = read_msa(b"a ACGT\nb ACGT\n\nb AC\na AC\n", MsaFormat::Selex).unwrap_err();
        assert!(err.contains("expected sequence a"), "{err}");
    }

    #[test]
    fn clustal_blocks_end_at_a_consensus_line() {
        let text = "CLUSTAL W (1.83) multiple sequence alignment\n\n\nseqA      MKV-LLAG 7\n\
                    seqB      MKVALL-G 7\n          *.* ** *\n\nseqA      WWHK 11\nseqB      WFHK 11\n\n";
        let msa = read(text, None);
        assert_eq!(msa.sqname, ["seqA", "seqB"]);
        assert_eq!(rows(&msa), ["MKV-LLAGWWHK", "MKVALL-GWFHK"]);
        // The last block needs its consensus (or blank) line.
        let err = read_msa(
            b"MUSCLE (3.8) multiple sequence alignment\n\na MKV\nb MKV\n",
            MsaFormat::ClustalLike,
        )
        .unwrap_err();
        assert!(err.contains("did not end with consensus line"), "{err}");
        let err = read_msa(b"MUSCLE alignment\n\na MKV\n\n", MsaFormat::Clustal).unwrap_err();
        assert!(err.contains("missing CLUSTAL header"), "{err}");
    }

    #[test]
    fn psiblast_case_becomes_the_rf_line() {
        let msa = read_msa(
            b"a  MKv-LL\nb  MKva-L\n\na  WW\nb  W-\n",
            MsaFormat::Psiblast,
        )
        .unwrap();
        assert_eq!(rows(&msa), ["MKv-LLWW", "MKva-LW-"]);
        assert_eq!(msa.rf.as_deref(), Some(&b"xx..xxxx"[..]));
        let err = read_msa(b"a  MKV\nb  mKV\n", MsaFormat::Psiblast).unwrap_err();
        assert!(err.contains("unexpected lower case residue (#1"), "{err}");
        let err = read_msa(b"a  MK.\nb  MKV\n", MsaFormat::Psiblast).unwrap_err();
        assert!(err.contains("invalid sequence characters"), "{err}");
    }

    #[test]
    fn strtoi32_follows_easel() {
        assert_eq!(strtoi32(b"20"), Some(20));
        assert_eq!(strtoi32(b"020"), Some(16));
        assert_eq!(strtoi32(b"0x1A"), Some(26));
        assert_eq!(strtoi32(b"12abc"), Some(12));
        assert_eq!(strtoi32(b"abc"), None);
    }
}
