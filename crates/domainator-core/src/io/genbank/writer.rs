//! GenBank writer: emits standard GenBank from our owned model.
//!
//! We write our own emitter rather than round-tripping through gb-io because our
//! lean location parts have lost gb-io's original nesting. The goal is *semantic*
//! re-readability (the original tool's parser reproduces the same records via
//! `helpers.compare_seqfiles`), not byte-identical output. Features are written in
//! ascending `location.start` order, matching `utils.write_genbank`.

use std::io::{self, Write};

use crate::seq::location::Part;
use crate::seq::{Location, SeqFeature, SeqRecord, Topology};

/// Format one location part as a 1-based inclusive GenBank span (`start..end`),
/// honoring the `<`/`>` fuzzy markers. Strand is handled by the caller.
fn fmt_part(part: &Part) -> String {
    let (start, end, _strand, before, after) = *part;
    let lo = start + 1;
    let lt = if before { "<" } else { "" };
    let gt = if after { ">" } else { "" };
    if lo == end && !before && !after {
        format!("{lo}")
    } else {
        format!("{lt}{lo}..{gt}{end}")
    }
}

/// Render a location to a GenBank location string.
fn fmt_location(loc: &Location) -> String {
    if loc.between {
        if let Some(p) = loc.parts.first() {
            return format!("{}^{}", p.0, p.0 + 1);
        }
    }
    match loc.operator {
        None => {
            let p = match loc.parts.first() {
                Some(p) => p,
                None => return String::new(),
            };
            let body = fmt_part(p);
            if p.2 == -1 {
                format!("complement({body})")
            } else {
                body
            }
        }
        Some(op) => {
            // All-minus compound was originally complement(join(...)) with parts
            // pre-reversed; undo that for a faithful, re-parseable string.
            let all_minus = loc.parts.iter().all(|p| p.2 == -1);
            let mut parts: Vec<&Part> = loc.parts.iter().collect();
            if all_minus {
                parts.reverse();
            }
            let inner: Vec<String> = parts.iter().map(|p| fmt_part(p)).collect();
            let joined = format!("{}({})", op.as_str(), inner.join(","));
            if all_minus {
                format!("complement({joined})")
            } else {
                joined
            }
        }
    }
}

const HEADER_INDENT: &str = "            "; // 12 spaces

/// Write a header line at the 12-col indent (continuation lines).
fn write_indented<W: Write>(w: &mut W, text: &str) -> io::Result<()> {
    writeln!(w, "{HEADER_INDENT}{text}")
}

/// Write a `TAG  value` header field, padding the tag to 12 cols and soft-wrapping
/// the value at 79 cols on whitespace (continuation indented 12). `tag` may carry a
/// leading indent (e.g. `  ORGANISM`). Continuation join is space, matching how the
/// reader and Biopython re-join these fields.
fn write_wrapped<W: Write>(w: &mut W, tag: &str, value: &str) -> io::Result<()> {
    let width = 79usize.saturating_sub(HEADER_INDENT.len());
    let mut first = true;
    let mut remaining = value;
    loop {
        let prefix = if first { format!("{:<12}", tag) } else { HEADER_INDENT.to_string() };
        if remaining.len() <= width {
            writeln!(w, "{prefix}{remaining}")?;
            break;
        }
        let mut cut = remaining[..width].rfind(' ').unwrap_or(width);
        if cut == 0 {
            cut = width;
        }
        writeln!(w, "{prefix}{}", &remaining[..cut])?;
        remaining = remaining[cut..].trim_start_matches(' ');
        first = false;
    }
    Ok(())
}

fn write_references<W: Write>(w: &mut W, refs: &[crate::seq::Reference]) -> io::Result<()> {
    for (i, r) in refs.iter().enumerate() {
        match &r.location_text {
            Some(loc) => writeln!(w, "REFERENCE   {}  {}", i + 1, loc)?,
            None => writeln!(w, "REFERENCE   {}", i + 1)?,
        }
        for (tag, val) in [
            ("  AUTHORS", &r.authors),
            ("  CONSRTM", &r.consrtm),
            ("  TITLE", &r.title),
            ("  JOURNAL", &r.journal),
            ("  MEDLINE", &r.medline_id),
            ("  PUBMED", &r.pubmed_id),
            ("  REMARK", &r.remark),
        ] {
            if !val.is_empty() {
                write_wrapped(w, tag, val)?;
            }
        }
    }
    Ok(())
}

fn write_comment<W: Write>(w: &mut W, ann: &crate::seq::Annotations) -> io::Result<()> {
    if ann.comment.is_none() && ann.structured_comment.is_empty() {
        return Ok(());
    }
    // Build the full comment body: free text (verbatim lines, re-joined by '\n' by
    // the parser) then each structured block as `##Name-START## .. ##Name-END##`.
    let mut body: Vec<String> = Vec::new();
    if let Some(c) = &ann.comment {
        for line in c.split('\n') {
            body.push(line.to_string());
        }
    }
    for (name, entries) in &ann.structured_comment {
        body.push(format!("##{name}-START##"));
        for (k, v) in entries {
            body.push(format!("{k} :: {v}"));
        }
        body.push(format!("##{name}-END##"));
    }
    for (i, line) in body.iter().enumerate() {
        if i == 0 {
            writeln!(w, "COMMENT     {line}")?;
        } else {
            write_indented(w, line)?;
        }
    }
    Ok(())
}

/// Wrap a feature qualifier across 79-column lines under the 21-space indent.
fn write_qualifier<W: Write>(w: &mut W, key: &str, value: &str, has_value: bool) -> io::Result<()> {
    const INDENT: &str = "                     "; // 21 spaces
    let line = if !has_value {
        format!("/{key}")
    } else if value.chars().all(|c| c.is_ascii_digit()) && !value.is_empty() {
        format!("/{key}={value}")
    } else {
        // GenBank escapes a literal `"` by doubling it.
        format!("/{key}=\"{}\"", value.replace('"', "\"\""))
    };
    // Wrap at word (space) boundaries only — never split a long token, matching
    // Biopython's `break_long_words=False`. Breaking mid-token would make the reader
    // rejoin the pieces with a spurious space.
    let width = 79 - INDENT.len();
    let mut cur = String::new();
    for word in line.split(' ') {
        if cur.is_empty() {
            cur.push_str(word);
        } else if cur.len() + 1 + word.len() <= width {
            cur.push(' ');
            cur.push_str(word);
        } else {
            writeln!(w, "{INDENT}{cur}")?;
            cur.clear();
            cur.push_str(word);
        }
    }
    writeln!(w, "{INDENT}{cur}")
}

fn write_feature<W: Write>(w: &mut W, feature: &SeqFeature) -> io::Result<()> {
    writeln!(w, "     {:<15} {}", feature.kind, fmt_location(&feature.location))?;
    for (key, values) in &feature.qualifiers {
        if values.is_empty() {
            write_qualifier(w, key, "", false)?;
        } else {
            for v in values {
                // A flag qualifier (e.g. /pseudo) is stored as one empty string.
                write_qualifier(w, key, v, !v.is_empty())?;
            }
        }
    }
    Ok(())
}

fn write_sequence<W: Write>(w: &mut W, seq: &[u8]) -> io::Result<()> {
    writeln!(w, "ORIGIN")?;
    let lower: Vec<u8> = seq.iter().map(|b| b.to_ascii_lowercase()).collect();
    let mut pos = 0;
    while pos < lower.len() {
        write!(w, "{:>9}", pos + 1)?;
        let line_end = (pos + 60).min(lower.len());
        let mut col = pos;
        while col < line_end {
            let chunk_end = (col + 10).min(line_end);
            write!(w, " {}", String::from_utf8_lossy(&lower[col..chunk_end]))?;
            col = chunk_end;
        }
        writeln!(w)?;
        pos += 60;
    }
    writeln!(w, "//")?;
    Ok(())
}

/// Write a single record in GenBank format.
pub fn write_record<W: Write>(w: &mut W, rec: &SeqRecord) -> io::Result<()> {
    // Match Biopython's GenBank LOCUS column layout exactly (InsdcIO
    // `_write_the_first_line`): protein records have empty molecule type and `aa`
    // units; the 28-char name+length field has the name left-justified and the
    // length right-justified.
    let locus = if !rec.name.is_empty() {
        rec.name.clone()
    } else if !rec.id.is_empty() {
        rec.id.clone()
    } else {
        rec.annotations.accessions.first().cloned().unwrap_or_default()
    };
    let mol_type = match rec.molecule_type.as_deref() {
        Some("protein") | Some("PROTEIN") => "",
        Some(m) => m,
        None => "DNA",
    };
    let units = if mol_type.is_empty() { "aa" } else { "bp" };
    let len_str = rec.len().to_string();
    // Match Biopython: long names steal from the length field but keep a space.
    let name_length = if locus.len() > 16 && len_str.len() > 11usize.saturating_sub(locus.len() - 16) {
        format!("{locus} {len_str}")
    } else {
        let padded = format!("{:>28}", len_str);
        format!("{}{}", locus, &padded[locus.len().min(padded.len())..])
    };
    // Protein (genpept) LOCUS lines carry no topology field; emitting one breaks
    // Biopython's column parser so it never sets molecule_type='protein'.
    let topology = if mol_type.is_empty() {
        "        " // 8 blanks
    } else {
        match rec.topology {
            Topology::Circular => "circular",
            Topology::Linear => "linear  ",
        }
    };
    let division = rec.annotations.division.as_deref().unwrap_or("UNK");
    let date = rec.annotations.date.as_deref().unwrap_or("01-JAN-1980");
    // "LOCUS       %s %s    %s %s %s %s"
    writeln!(
        w,
        "LOCUS       {} {}    {:<7} {} {} {}",
        name_length, units, mol_type, topology, division, date,
    )?;
    write_wrapped(w, "DEFINITION", if rec.description.is_empty() { "." } else { &rec.description })?;
    let ann = &rec.annotations;
    // ACCESSION / VERSION, replicating Biopython's GenBankWriter: derive a
    // default accession from `id` (stripping a trailing `.<version>`), prefer the
    // `accessions` annotation, and always emit both lines.
    let id = &rec.id;
    let default_acc = match id.split_once('.') {
        Some((base, ver)) if !ver.is_empty() && ver.chars().all(|c| c.is_ascii_digit()) => base,
        _ => id.as_str(),
    };
    let accession = ann.accessions.first().map(String::as_str).unwrap_or(default_acc);
    let acc_with_version = if let Some(rest) = id.strip_prefix(&format!("{accession}.")) {
        match rest.parse::<i64>() {
            Ok(v) => format!("{accession}.{v}"),
            Err(_) => accession.to_string(),
        }
    } else {
        accession.to_string()
    };
    writeln!(w, "ACCESSION   {accession}")?;
    match &ann.gi {
        Some(gi) => writeln!(w, "VERSION     {acc_with_version}  GI:{gi}")?,
        None => writeln!(w, "VERSION     {acc_with_version}")?,
    }
    if !ann.dblink.is_empty() {
        writeln!(w, "DBLINK      {}", ann.dblink.join("\n            "))?;
    }
    if let Some(dbs) = &ann.db_source {
        write_wrapped(w, "DBSOURCE", dbs)?;
    }
    // KEYWORDS: Biopython yields [""] for a `.`-only field.
    let kw = if ann.keywords.iter().all(|k| k.is_empty()) {
        ".".to_string()
    } else {
        format!("{}.", ann.keywords.join("; "))
    };
    write_wrapped(w, "KEYWORDS", &kw)?;
    // Biopython always emits SOURCE / ORGANISM / taxonomy, defaulting to ".".
    write_wrapped(w, "SOURCE", ann.source.as_deref().unwrap_or("."))?;
    let org = ann.organism.as_deref().unwrap_or(".");
    // ORGANISM must be a single line (continuations are taxonomy); truncate like Biopython.
    let org_trunc = if org.len() > 67 { format!("{}...", &org[..63]) } else { org.to_string() };
    writeln!(w, "{:<12}{}", "  ORGANISM", org_trunc)?;
    let taxonomy = if ann.taxonomy.is_empty() {
        ".".to_string()
    } else {
        format!("{}.", ann.taxonomy.join("; "))
    };
    write_wrapped(w, "", &taxonomy)?;
    write_references(w, &ann.references)?;
    write_comment(w, ann)?;
    writeln!(w, "FEATURES             Location/Qualifiers")?;
    let mut order: Vec<&SeqFeature> = rec.features.iter().collect();
    order.sort_by_key(|f| f.location.start());
    for f in order {
        write_feature(w, f)?;
    }
    write_sequence(w, &rec.seq)?;
    Ok(())
}

/// Write records to a GenBank file.
pub fn write_genbank(path: &str, records: &[SeqRecord]) -> io::Result<()> {
    let file = std::fs::File::create(path)?;
    let mut w = io::BufWriter::new(file);
    for rec in records {
        write_record(&mut w, rec)?;
    }
    w.flush()
}
