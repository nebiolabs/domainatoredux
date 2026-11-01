//! UniProt flat files (`uniprot_sprot.dat`, `uniprot_trembl.dat`), read the way
//! Biopython's `swiss` format reads them, for `db_download`'s `_gb` databases.
//!
//! A port of `Bio.SwissProt._read` and `Bio.SeqIO.SwissIO`, rule for rule wherever the
//! result reaches the GenBank file upstream writes: the entry is the record
//! `SwissIterator` builds, and [`SwissEntry::to_record`] fills our record's fields
//! so that our GenBank writer produces the bytes Biopython's does. Fields upstream
//! never writes (gene names, organelle, host, protein existence, sequence versions)
//! are skipped. Biopython's quirks are kept on purpose, among them: a keyword's
//! evidence tag survives when it wraps onto the next `KW` line, an `RC` line without
//! its closing `;` is carried onto whatever line comes next, and `DR` cross
//! references become `DBLINK` lines.

use std::io::{self, BufRead};

use crate::seq::location::Location;
use crate::seq::{Annotations, Reference, SeqFeature, SeqRecord, Topology};

/// One position of a UniProt feature (Biopython's `Position.fromstring`); `?n`, an
/// uncertain position, is exact as far as the GenBank writer is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    Exact(i64),
    Before(i64),
    After(i64),
    Unknown,
}

impl Position {
    fn parse(text: &str, offset: i64) -> Result<Position, String> {
        let number = |t: &str| {
            py_int(t)
                .map(|n| n + offset)
                .ok_or_else(|| format!("invalid feature position {text:?}"))
        };
        Ok(if text == "?" {
            Position::Unknown
        } else if let Some(t) = text.strip_prefix('?') {
            Position::Exact(number(t)?)
        } else if let Some(t) = text.strip_prefix('<') {
            Position::Before(number(t)?)
        } else if let Some(t) = text.strip_prefix('>') {
            Position::After(number(t)?)
        } else {
            Position::Exact(number(text)?)
        })
    }

    fn value(self) -> Option<i64> {
        match self {
            Position::Exact(n) | Position::Before(n) | Position::After(n) => Some(n),
            Position::Unknown => None,
        }
    }

    /// `Position + 1`, which keeps the kind.
    fn plus_one(self) -> Position {
        match self {
            Position::Exact(n) => Position::Exact(n + 1),
            Position::Before(n) => Position::Before(n + 1),
            Position::After(n) => Position::After(n + 1),
            Position::Unknown => Position::Unknown,
        }
    }
}

/// A feature-table (`FT`) entry.
#[derive(Debug, Clone, PartialEq)]
pub struct SwissFeature {
    pub kind: String,
    /// 0-based start and end, as Biopython's `SimpleLocation` holds them.
    pub start: Position,
    pub end: Position,
    /// The isoform a feature is on (`P68250-2:1`), Biopython's location `ref`.
    pub isoform: Option<String>,
    pub qualifiers: Vec<(String, String)>,
}

impl SwissFeature {
    /// The feature as a GenBank feature, or `None` when a position is `?`, which
    /// upstream's `db_download` drops (it has no GenBank form).
    pub fn to_feature(&self) -> Option<SeqFeature> {
        let (start, end) = (self.start.value()?, self.end.value()?);
        // A `>` start or a `<` end has no place in our location model. UniProt writes
        // neither, so they are read as exact positions.
        let before = matches!(self.start, Position::Before(_));
        let after = matches!(self.end, Position::After(_));
        let mut location = Location::range(start, end, 1);
        location.parts[0].3 = before;
        location.parts[0].4 = after;
        location.reference.clone_from(&self.isoform);
        Some(SeqFeature {
            kind: self.kind.clone(),
            location,
            qualifiers: self
                .qualifiers
                .iter()
                .map(|(k, v)| (k.clone(), vec![v.clone()]))
                .collect(),
        })
    }
}

/// A literature reference (`RN` to `RL`), finished at `//`.
#[derive(Debug, Clone, Default, PartialEq)]
struct SwissReference {
    authors: Vec<String>,
    title: Vec<String>,
    location: Vec<String>,
    /// `RC` tokens, `(token, text)`; a continuation without `=` makes the entry
    /// unwritable for Biopython, which is reported when the record is built.
    comments: Vec<Result<(String, String), String>>,
    /// `RX` cross references, `(database, id)`.
    references: Vec<(String, String)>,
}

/// One UniProt entry, as far as upstream's GenBank output needs it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SwissEntry {
    pub entry_name: String,
    pub accessions: Vec<String>,
    /// The date the entry was integrated into UniProtKB (`created`).
    pub created: Option<String>,
    pub description: String,
    pub organism: String,
    pub organism_classification: Vec<String>,
    pub taxonomy_id: Vec<String>,
    pub comments: Vec<String>,
    pub cross_references: Vec<Vec<String>>,
    pub keywords: Vec<String>,
    references: Vec<SwissReference>,
    pub features: Vec<SwissFeature>,
    pub sequence: String,
}

/// The entries of a UniProt flat-file stream, one at a time.
pub struct SwissReader<R> {
    reader: R,
    line_no: usize,
}

impl<R: BufRead> SwissReader<R> {
    pub fn new(reader: R) -> Self {
        SwissReader { reader, line_no: 0 }
    }

    /// The next line with its line ending, normalized to `\n` (Python's universal
    /// newlines), or `None` at the end.
    fn next_line(&mut self) -> io::Result<Option<String>> {
        let mut line = String::new();
        if self.reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        self.line_no += 1;
        if line.ends_with("\r\n") {
            line.truncate(line.len() - 2);
            line.push('\n');
        }
        Ok(Some(line))
    }

    fn error(&self, message: impl std::fmt::Display) -> io::Error {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("UniProt flat file, line {}: {message}", self.line_no),
        )
    }

    fn read_entry(&mut self) -> io::Result<Option<SwissEntry>> {
        // Blank lines between entries are skipped (Biopython fails on them).
        let first = loop {
            match self.next_line()? {
                None => return Ok(None),
                Some(line) if line.trim().is_empty() => continue,
                Some(line) => break line,
            }
        };
        if slice(&first, 0, Some(2)) != "ID" {
            return Err(self.error("Failed to find ID in first line"));
        }
        let mut entry = SwissEntry {
            entry_name: slice(&first, 5, None)
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_string(),
            ..SwissEntry::default()
        };
        let mut description: Vec<String> = Vec::new();
        let mut organism: Vec<String> = Vec::new();
        let mut sequence = String::new();
        let mut unread = String::new();
        while let Some(line) = self.next_line()? {
            let key = slice(&line, 0, Some(2));
            let mut value = slice(&line, 5, None).trim_end().to_string();
            if !unread.is_empty() {
                value = format!("{} {value}", std::mem::take(&mut unread));
            }
            let reference = |entry: &mut SwissEntry| entry.references.last_mut().is_some();
            match key {
                "AC" => entry
                    .accessions
                    .extend(value.trim_end_matches(';').split("; ").map(str::to_string)),
                "DT" => read_dt(&mut entry, &line).map_err(|e| self.error(e))?,
                "DE" => description.push(value.trim().to_string()),
                "OS" => organism.push(value),
                "OC" => entry.organism_classification.extend(
                    value
                        .trim_end_matches([';', '.'])
                        .split("; ")
                        .map(str::to_string),
                ),
                "OX" => read_ox(&mut entry, &line).map_err(|e| self.error(e))?,
                "RN" => entry.references.push(SwissReference::default()),
                "RP" | "RC" | "RX" | "RL" | "RA" | "RG" | "RT" if !reference(&mut entry) => {
                    return Err(self.error(format!("{key}: missing RN")));
                }
                "RC" => {
                    let r = entry.references.last_mut().expect("checked above");
                    unread = read_rc(r, &value);
                }
                "RX" => read_rx(entry.references.last_mut().expect("checked above"), &value),
                "RL" => entry
                    .references
                    .last_mut()
                    .expect("checked above")
                    .location
                    .push(value),
                "RA" | "RG" => entry
                    .references
                    .last_mut()
                    .expect("checked above")
                    .authors
                    .push(value),
                "RT" => entry
                    .references
                    .last_mut()
                    .expect("checked above")
                    .title
                    .push(value),
                "CC" => read_cc(&mut entry, &line),
                "DR" => entry.cross_references.push(
                    value
                        .trim_end_matches('.')
                        .split("; ")
                        .map(str::to_string)
                        .collect(),
                ),
                "KW" => read_kw(&mut entry, &value),
                "FT" => read_ft(&mut entry, &line).map_err(|e| self.error(e))?,
                "  " => sequence.push_str(value.replace(' ', "").trim_end()),
                "//" => {
                    entry.description = description.join(" ");
                    entry.organism = organism.join(" ");
                    entry.sequence = sequence;
                    return Ok(Some(entry));
                }
                // Read by Biopython but never written by upstream.
                "GN" | "OG" | "OH" | "RP" | "PE" | "SQ" | "**" => {}
                _ => return Err(self.error(format!("Unknown keyword {key:?} found"))),
            }
        }
        Err(self.error("Unexpected end of stream."))
    }
}

impl<R: BufRead> Iterator for SwissReader<R> {
    type Item = io::Result<SwissEntry>;

    fn next(&mut self) -> Option<io::Result<SwissEntry>> {
        self.read_entry().transpose()
    }
}

impl SwissEntry {
    /// The NCBI taxid upstream files the entry under (`ncbi_taxid[0]`).
    pub fn taxid(&self) -> io::Result<i64> {
        self.taxonomy_id
            .first()
            .and_then(|t| py_int(t))
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("UniProt entry {} has no NCBI taxid", self.entry_name),
                )
            })
    }

    /// The record `SwissIterator` builds, in our model: `id` is the primary accession
    /// and `name` the entry name, which is how they come back from the GenBank file
    /// upstream writes (LOCUS and VERSION). Features keep their `?` positions out.
    pub fn to_record(&self) -> io::Result<SeqRecord> {
        let invalid = |m: String| io::Error::new(io::ErrorKind::InvalidData, m);
        let id = self.accessions.first().cloned().ok_or_else(|| {
            invalid(format!(
                "UniProt entry {} has no accession",
                self.entry_name
            ))
        })?;
        let mut dbxrefs: Vec<String> = Vec::new();
        for cross_reference in &self.cross_references {
            if let [database, accession, ..] = cross_reference.as_slice() {
                let dbxref = format!("{database}:{accession}");
                if !dbxrefs.contains(&dbxref) {
                    dbxrefs.push(dbxref);
                }
            }
        }
        let mut references = Vec::new();
        for r in &self.references {
            let mut reference = Reference::default();
            let mut comment = Vec::new();
            for c in &r.comments {
                match c {
                    Ok((token, text)) => comment.push(format!("{token}={text};")),
                    Err(line) => {
                        return Err(invalid(format!(
                            "UniProt entry {}: cannot read the RC line {line:?}",
                            self.entry_name
                        )))
                    }
                }
            }
            reference.remark = comment.join(" ");
            for (key, value) in &r.references {
                match key.as_str() {
                    "PubMed" => reference.pubmed_id.clone_from(value),
                    "MEDLINE" => reference.medline_id.clone_from(value),
                    "DOI" | "AGRICOLA" => {}
                    _ => {
                        return Err(invalid(format!(
                            "UniProt entry {}: Unknown key {key} found in references",
                            self.entry_name
                        )))
                    }
                }
            }
            reference.authors = r.authors.join(" ").trim_end_matches(';').to_string();
            reference.title = joined_title(&r.title);
            reference.journal = r.location.join(" ");
            references.push(reference);
        }
        let annotations = Annotations {
            date: self.created.as_deref().map(genbank_date),
            keywords: self.keywords.clone(),
            organism: Some(self.organism.trim_end_matches('.').to_string()),
            taxonomy: self.organism_classification.clone(),
            // Biopython's writer puts ": " after each database name, unless one is
            // already there.
            dblink: dbxrefs
                .iter()
                .map(|x| {
                    if x.contains(": ") {
                        x.clone()
                    } else {
                        x.replace(':', ": ")
                    }
                })
                .collect(),
            references,
            comment: (!self.comments.is_empty()).then(|| self.comments.join("\n")),
            ..Annotations::default()
        };
        Ok(SeqRecord {
            id,
            name: self.entry_name.clone(),
            description: self.description.clone(),
            seq: self.sequence.clone().into_bytes(),
            molecule_type: Some("protein".to_string()),
            topology: Topology::Linear,
            annotations,
            features: self
                .features
                .iter()
                .filter_map(SwissFeature::to_feature)
                .collect(),
        })
    }
}

/// `line[a:b]` in characters, as Python slices a `str` (out-of-range ends clamp).
fn slice(line: &str, a: usize, b: Option<usize>) -> &str {
    if line.is_ascii() {
        let end = b.unwrap_or(line.len()).min(line.len());
        return line.get(a.min(end)..end).unwrap_or("");
    }
    let byte = |n: usize| line.char_indices().nth(n).map_or(line.len(), |(i, _)| i);
    let end = b.map_or(line.len(), byte);
    let start = byte(a).min(end);
    &line[start..end]
}

/// Python's `int(text)`: surrounding whitespace, a sign and `_` digit separators.
fn py_int(text: &str) -> Option<i64> {
    let t = text.trim();
    let (sign, digits) = match t.strip_prefix('-') {
        Some(d) => (-1, d),
        None => (1, t.strip_prefix('+').unwrap_or(t)),
    };
    if digits.is_empty()
        || digits.starts_with('_')
        || digits.ends_with('_')
        || digits.contains("__")
        || !digits.chars().all(|c| c.is_ascii_digit() || c == '_')
    {
        return None;
    }
    digits
        .replace('_', "")
        .parse::<i64>()
        .ok()
        .map(|n| sign * n)
}

/// `_read_dt`: only the date the entry was created reaches the output.
fn read_dt(entry: &mut SwissEntry, line: &str) -> Result<(), String> {
    let value = slice(line, 5, None);
    let upper = value.to_uppercase();
    let cols: Vec<&str> = value.split_whitespace().collect();
    let first = cols.first().copied().unwrap_or("");
    if upper.contains("CREATED")
        || upper.contains("LAST SEQUENCE UPDATE")
        || upper.contains("LAST ANNOTATION UPDATE")
    {
        if !upper.split_whitespace().any(|c| c.contains("REL.")) {
            return Err(format!(
                "Could not find Rel. in DT line: {}",
                line.trim_end()
            ));
        }
        if upper.contains("CREATED") {
            entry.created = Some(first.to_string());
        }
    } else if upper.contains("INTEGRATED INTO")
        || upper.contains("SEQUENCE VERSION")
        || upper.contains("ENTRY VERSION")
    {
        if upper.contains("INTEGRATED") {
            entry.created = Some(first.trim_end_matches(',').to_string());
        }
    } else {
        return Err("Failed to parse DT (DaTe) line".to_string());
    }
    Ok(())
}

/// `_read_ox`: evidence tags (`{...}`) are dropped; the first `OX` line names the
/// database.
fn read_ox(entry: &mut SwissEntry, line: &str) -> Result<(), String> {
    let line = line.split('{').next().unwrap_or("");
    let text = slice(line, 5, None).trim_end().trim_end_matches(';');
    let ids = if entry.taxonomy_id.is_empty() {
        let mut pieces = text.split('=');
        match (pieces.next(), pieces.next(), pieces.next()) {
            (Some("NCBI_TaxID"), Some(ids), None) => ids,
            (Some(descr), Some(_), None) => {
                return Err(format!("Unexpected taxonomy type {descr}"))
            }
            _ => return Err(format!("cannot read the OX line {:?}", line.trim_end())),
        }
    } else {
        text
    };
    entry
        .taxonomy_id
        .extend(ids.split(", ").map(str::to_string));
    Ok(())
}

/// `_read_rc`: `token=text;` pairs; a value without its closing `;` hands its last
/// piece to the next line. Returns that piece.
fn read_rc(reference: &mut SwissReference, value: &str) -> String {
    let mut cols: Vec<&str> = value.split(';').collect();
    let unread = if value.ends_with(';') {
        String::new()
    } else {
        cols.pop().unwrap_or("").to_string()
    };
    for col in cols {
        if col.is_empty() {
            // Python returns here without the carried piece.
            return String::new();
        }
        match col.find('=') {
            Some(i) => reference.comments.push(Ok((
                col[..i].trim_start().to_string(),
                col[i + 1..].to_string(),
            ))),
            None => {
                let previous = reference.comments.pop();
                reference.comments.push(Err(match previous {
                    Some(Ok((token, text))) => format!("{token}={text} {col}"),
                    Some(Err(text)) => format!("{text} {col}"),
                    None => col.to_string(),
                }));
            }
        }
    }
    unread
}

/// `_read_rx`: `PubMed=...; DOI=...;` pairs (or the old `MEDLINE; 123.` form). A
/// malformed pair ends the line, as after Biopython's warning.
fn read_rx(reference: &mut SwissReference, value: &str) {
    let value = value.replace(" [NCBI, ExPASy, Israel, Japan]", "");
    if value.contains('=') {
        for col in value.split("; ").map(str::trim).filter(|c| !c.is_empty()) {
            let pieces: Vec<&str> = col.split('=').collect();
            if pieces.len() != 2 {
                break;
            }
            reference.references.push((
                pieces[0].to_string(),
                pieces[1].trim_end_matches(';').to_string(),
            ));
        }
    } else {
        let cols: Vec<&str> = value.split("; ").collect();
        if let [database, id] = cols.as_slice() {
            reference.references.push((
                database.trim_end_matches(';').to_string(),
                id.trim_end_matches('.').to_string(),
            ));
        }
    }
}

/// `_read_cc`: `-!-` starts a comment and an indented line continues it; the
/// copyright block's lines are neither.
fn read_cc(entry: &mut SwissEntry, line: &str) {
    let value = slice(line, 9, None).trim_end();
    match slice(line, 5, Some(8)) {
        "-!-" => entry.comments.push(value.to_string()),
        "   " => match entry.comments.last_mut() {
            Some(last) => {
                last.push(' ');
                last.push_str(value);
            }
            None => entry.comments.push(value.to_string()),
        },
        _ => {}
    }
}

/// `_read_kw`: a keyword's evidence tag is dropped when it ends on the same line.
fn read_kw(entry: &mut SwissEntry, value: &str) {
    for keyword in value.trim_end_matches([';', '.']).split("; ") {
        let keyword = if keyword.ends_with('}') {
            keyword.rsplit_once('{').map_or(keyword, |(k, _)| k)
        } else {
            keyword
        };
        entry.keywords.push(keyword.trim().to_string());
    }
}

/// Biopython's VARSPLIC/VAR_SEQ rewrite: the spaces wrapping put inside the two
/// sequences of `A B -> C D (extra)` are removed.
fn var_seq_description(description: String) -> String {
    let mut halves = description.split(" -> ");
    let (Some(first), Some(second), None) = (halves.next(), halves.next(), halves.next()) else {
        return description;
    };
    let (second, extra) = match second.find(" (") {
        Some(i) => second.split_at(i),
        None => (second, ""),
    };
    format!(
        "{} -> {}{extra}",
        first.replace(' ', ""),
        second.replace(' ', "")
    )
}

/// `_read_ft`, both the current layout (location at column 22, `/qualifier="..."`
/// lines) and the pre-2019 one (from/to/description columns).
fn read_ft(entry: &mut SwissEntry, line: &str) -> Result<(), String> {
    let name = slice(line, 5, Some(13)).trim_end();
    if !name.is_empty() {
        let (isoform, from, to, qualifiers) = if slice(line, 13, Some(21)) == "        " {
            let location = slice(line, 21, Some(80)).trim_end();
            let mut pieces = location.split(':');
            let (isoform, location) = match (pieces.next(), pieces.next(), pieces.next()) {
                (Some(isoform), Some(location), None) => (Some(isoform.to_string()), location),
                _ => (None, location),
            };
            let mut ends = location.split("..");
            let (from, to) = match (ends.next(), ends.next(), ends.next()) {
                (Some(from), Some(to), None) => (from, to),
                _ => (location, ""),
            };
            (isoform, from, to, Vec::new())
        } else {
            let description = slice(line, 34, Some(75)).trim_end().to_string();
            (
                None,
                slice(line, 14, Some(20)).trim_start(),
                slice(line, 21, Some(27)).trim_start(),
                vec![("description".to_string(), description)],
            )
        };
        let start = Position::parse(from, -1)?;
        let end = if to.is_empty() {
            start.plus_one()
        } else {
            Position::parse(to, 0)?
        };
        if let (Some(s), Some(e)) = (start.value(), end.value()) {
            if e < s {
                return Err(format!("feature {name} ends before it starts"));
            }
        }
        entry.features.push(SwissFeature {
            kind: name.to_string(),
            start,
            end,
            isoform,
            qualifiers,
        });
        return Ok(());
    }
    let Some(feature) = entry.features.last_mut() else {
        return Err("FT continuation line without a feature".to_string());
    };
    if slice(line, 5, Some(34)) == " ".repeat(29) {
        // Pre-2019 continuation of the description column.
        let description = slice(line, 34, Some(75)).trim_end();
        if description.starts_with("/FTId=") {
            return Ok(());
        }
        let Some((_, old)) = feature
            .qualifiers
            .iter_mut()
            .find(|(k, _)| k == "description")
        else {
            return Err("FT continuation line without a description".to_string());
        };
        let mut joined = if old.ends_with('-') {
            format!("{old}{description}")
        } else {
            format!("{old} {description}")
        };
        if feature.kind == "VARSPLIC" || feature.kind == "VAR_SEQ" {
            joined = var_seq_description(joined);
        }
        *old = joined;
        return Ok(());
    }
    let value = slice(line, 21, None).trim_end();
    if let Some(rest) = value.strip_prefix('/') {
        let key_len = rest
            .bytes()
            .take_while(|b| b.is_ascii_lowercase() || *b == b'_')
            .count();
        if key_len > 0 && rest[key_len..].starts_with('=') {
            let key = &rest[..key_len];
            let text = &rest[key_len + 1..];
            let Some(text) = text.strip_prefix('"') else {
                return Err("Missing starting quote in feature".to_string());
            };
            if key == "id" {
                // Biopython keeps the feature id off the qualifiers.
                if !text.ends_with('"') {
                    return Err("Missing closing quote for id".to_string());
                }
                return Ok(());
            }
            if feature.qualifiers.iter().any(|(k, _)| k == key) {
                return Err(format!(
                    "Feature qualifier '{key}' already exists for feature"
                ));
            }
            // A value that does not end with its quote continues on the next line.
            let text = text.strip_suffix('"').unwrap_or(text);
            feature.qualifiers.push((key.to_string(), text.to_string()));
            return Ok(());
        }
    }
    let kind = feature.kind.clone();
    let Some((key, old)) = feature.qualifiers.last_mut() else {
        return Err("FT continuation line without a qualifier".to_string());
    };
    let description = value.trim_end_matches('"');
    let mut joined = if key == "evidence" || old.ends_with('-') {
        format!("{old}{description}")
    } else {
        format!("{old} {description}")
    };
    if kind == "VAR_SEQ" {
        joined = var_seq_description(joined);
    }
    *old = joined;
    Ok(())
}

/// A reference title from its `RT` lines: joined with a space unless a line ends in
/// a hyphen, then the trailing `;` and the surrounding quotes removed.
fn joined_title(lines: &[String]) -> String {
    let Some((first, rest)) = lines.split_first() else {
        return String::new();
    };
    let mut title = first.clone();
    for fragment in rest {
        if !title.ends_with('-') {
            title.push(' ');
        }
        title.push_str(fragment);
    }
    let title = title.trim_end_matches(';');
    if title.starts_with('"') && title.ends_with('"') {
        title.get(1..title.len() - 1).unwrap_or("").to_string()
    } else {
        title.to_string()
    }
}

/// The LOCUS date Biopython writes for a date annotation: the date itself when it
/// is a valid `DD-MON-YYYY`, else `01-JAN-1980`.
fn genbank_date(date: &str) -> String {
    const MONTHS: [&str; 12] = [
        "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
    ];
    let valid = || -> Option<()> {
        if date.len() != 11 || !date.is_ascii() || &date[2..3] != "-" || &date[6..7] != "-" {
            return None;
        }
        let day: u32 = date[..2].trim_start().parse().ok()?;
        let month = MONTHS
            .iter()
            .position(|m| m.eq_ignore_ascii_case(&date[3..6]))?;
        let year_text = &date[7..];
        if !year_text.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let year: i64 = year_text.parse().ok()?;
        let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
        let days = [
            31,
            if leap { 29 } else { 28 },
            31,
            30,
            31,
            30,
            31,
            31,
            30,
            31,
            30,
            31,
        ];
        (1..=days[month]).contains(&day).then_some(())
    };
    match valid() {
        Some(()) => date.to_string(),
        None => "01-JAN-1980".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENTRY: &str = "\
ID   TEST_ECOLI              Reviewed;          12 AA.
AC   P00001; P00002;
DT   21-JUL-1986, integrated into UniProtKB/Swiss-Prot.
DT   21-JUL-1986, sequence version 1.
DE   RecName: Full=Test protein;
DE            Short=TP;
DE   Flags: Fragment;
GN   Name=tst;
OS   Escherichia coli (strain
OS   K12).
OC   Bacteria; Pseudomonadota;
OC   Escherichia.
OX   NCBI_TaxID=83333 {ECO:0000313|EMBL:X};
RN   [1]
RP   NUCLEOTIDE SEQUENCE.
RC   STRAIN=K12 / MG1655 / ATCC
RC   47076; TISSUE=Liver;
RX   PubMed=123; DOI=10.1/x;
RA   Doe J., Roe R.;
RT   \"A long title that wraps at a hyph-
RT   en and goes on.\";
RL   J. Test 1:1-2(2000).
CC   -!- FUNCTION: Does
CC       things.
CC   ---------------------------------------------------------------------------
CC   Copyrighted by the UniProt Consortium
DR   EMBL; X1; AAA1.1; -; Genomic_DNA.
DR   GO; GO:0005737; C:cytoplasm; IDA:EcoCyc.
DR   EMBL; X1; AAA2.1; -; Genomic_DNA.
PE   1: Evidence at protein level;
KW   Hydrolase {ECO:0000256|ARBA:ARBA1,
KW   ECO:0000256|HAMAP-Rule:MF_1}; Nuclease.
FT   NON_TER         1
FT   CHAIN           ?2..12
FT                   /note=\"A note that\"
FT   VAR_SEQ         3..4
FT                   /note=\"AB -> C
FT                   D (in isoform 2)\"
FT                   /id=\"VSP_1\"
FT   MOD_RES         P00001-2:5
FT   SITE            ?..7
SQ   SEQUENCE   12 AA;  1000 MW;  0000000000000000 CRC64;
     MKRFSLAILA LG
//
";

    fn entry() -> SwissEntry {
        SwissReader::new(ENTRY.as_bytes()).next().unwrap().unwrap()
    }

    #[test]
    fn header_fields_follow_biopython() {
        let e = entry();
        assert_eq!(e.entry_name, "TEST_ECOLI");
        assert_eq!(e.accessions, ["P00001", "P00002"]);
        assert_eq!(e.created.as_deref(), Some("21-JUL-1986"));
        assert_eq!(
            e.description,
            "RecName: Full=Test protein; Short=TP; Flags: Fragment;"
        );
        assert_eq!(e.organism, "Escherichia coli (strain K12).");
        assert_eq!(
            e.organism_classification,
            ["Bacteria", "Pseudomonadota", "Escherichia"]
        );
        assert_eq!(e.taxid().unwrap(), 83333);
        assert_eq!(e.comments, ["FUNCTION: Does things."]);
        // The evidence tag that wraps onto the next line survives, split at "; ".
        assert_eq!(
            e.keywords,
            [
                "Hydrolase {ECO:0000256|ARBA:ARBA1,",
                "ECO:0000256|HAMAP-Rule:MF_1}",
                "Nuclease"
            ]
        );
        assert_eq!(e.sequence, "MKRFSLAILALG");
    }

    #[test]
    fn references_join_like_biopython() {
        let rec = entry().to_record().unwrap();
        let r = &rec.annotations.references[0];
        assert_eq!(r.authors, "Doe J., Roe R.");
        assert_eq!(r.title, "A long title that wraps at a hyph-en and goes on.");
        assert_eq!(r.journal, "J. Test 1:1-2(2000).");
        assert_eq!(r.pubmed_id, "123");
        // The RC value without its ';' is carried onto the next line.
        assert_eq!(r.remark, "STRAIN=K12 / MG1655 / ATCC 47076; TISSUE=Liver;");
    }

    #[test]
    fn the_record_is_what_swissio_builds() {
        let rec = entry().to_record().unwrap();
        assert_eq!(
            (rec.id.as_str(), rec.name.as_str()),
            ("P00001", "TEST_ECOLI")
        );
        assert_eq!(rec.annotations.date.as_deref(), Some("21-JUL-1986"));
        assert_eq!(
            rec.annotations.organism.as_deref(),
            Some("Escherichia coli (strain K12)")
        );
        assert_eq!(rec.annotations.dblink, ["EMBL: X1", "GO: GO: 0005737"]);
        // The `?..7` feature has no GenBank form.
        let kinds: Vec<&str> = rec.features.iter().map(|f| f.kind.as_str()).collect();
        assert_eq!(kinds, ["NON_TER", "CHAIN", "VAR_SEQ", "MOD_RES"]);
        assert_eq!(
            rec.features[1].location.parts,
            vec![(1, 12, 1, false, false)]
        );
        assert_eq!(
            rec.features[2].get_first("note"),
            Some("AB -> CD (in isoform 2)")
        );
        assert_eq!(
            rec.features[3].location.reference.as_deref(),
            Some("P00001-2")
        );
        assert_eq!(
            crate::seq::partial::record_fragment_status(&rec),
            Some("N"),
            "a NON_TER feature on the first residue"
        );
    }

    #[test]
    fn dates_biopython_cannot_read_become_its_default() {
        assert_eq!(genbank_date("29-FEB-2000"), "29-FEB-2000");
        assert_eq!(genbank_date("29-FEB-1900"), "01-JAN-1980");
        assert_eq!(genbank_date("1-JAN-2000"), "01-JAN-1980");
        assert_eq!(genbank_date("01-Jan-2000"), "01-Jan-2000");
    }

    #[test]
    fn python_int_rules() {
        assert_eq!(py_int(" 12 "), Some(12));
        assert_eq!(py_int("1_000"), Some(1000));
        assert_eq!(py_int("+5"), Some(5));
        assert_eq!(py_int("_5"), None);
        assert_eq!(py_int(""), None);
    }
}
