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

/// Today's date as GenBank writes it, `DD-MON-YYYY` upper-cased.
///
/// Python uses `date.today()`, i.e. *local* time; we have no timezone database and
/// no date dependency, so this is UTC. The two disagree only for a run that
/// straddles local midnight, and every parity comparator skips `date`.
fn today_genbank() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    const MONTHS: [&str; 12] = [
        "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
    ];
    format!("{d:02}-{}-{y:04}", MONTHS[(m - 1) as usize])
}

/// Days since the Unix epoch → `(year, month, day)`. Howard Hinnant's
/// `civil_from_days`, which is exact for the whole proleptic Gregorian range.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
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

/// Write one feature qualifier, wrapping only `/translation`.
///
/// This is upstream's patched Biopython writer (`Bio/SeqIO/InsdcIO.py`,
/// `_write_feature_qualifier`): it wraps `/translation` at 80 columns and gives
/// every other qualifier a width of `10000000000000`, i.e. one unwrapped line no
/// matter how long. Wrapping the rest as stock Biopython does produces files that
/// still *read back* equal — which is why every parity gate was blind to it — but
/// differ byte for byte from the reference, and it is what makes breaking a long
/// spaceless token hazardous, since the reader rejoins the halves with a space.
///
/// The width counts the 21-space indent, so a wrapped translation line is exactly
/// 80 characters and the last one is shorter.
fn write_qualifier<W: Write>(w: &mut W, key: &str, value: &str, has_value: bool) -> io::Result<()> {
    const INDENT: &str = "                     "; // 21 spaces
    // Upstream leaves a qualifier unquoted only for this fixed list (its
    // `FTQUAL_NO_QUOTE`) or for a value that is a Python `int`. Our qualifier
    // values are always strings, so the list is the whole rule -- guessing from
    // the value instead strips the quotes off `/gc="49"`, `/length="105"` and
    // every other numeric-looking annotation these tools write.
    const FTQUAL_NO_QUOTE: [&str; 13] = [
        "anticodon",
        "citation",
        "codon_start",
        "compare",
        "direction",
        "estimated_length",
        "mod_base",
        "number",
        "rpt_type",
        "rpt_unit_range",
        "tag_peptide",
        "transl_except",
        "transl_table",
    ];
    let line = if !has_value {
        format!("{INDENT}/{key}")
    } else if FTQUAL_NO_QUOTE.contains(&key) {
        format!("{INDENT}/{key}={value}")
    } else {
        // GenBank escapes a literal `"` by doubling it.
        format!("{INDENT}/{key}=\"{}\"", value.replace('"', "\"\""))
    };
    if key != "translation" {
        return writeln!(w, "{line}");
    }

    const MAX_WIDTH: usize = 80;
    let mut line = line;
    while !line.trim_start().is_empty() {
        if line.len() <= MAX_WIDTH {
            return writeln!(w, "{line}");
        }
        // Scan back for a space to break on, never past the indent. A protein
        // translation has none, so this normally falls through to a hard cut at
        // exactly MAX_WIDTH.
        let bytes = line.as_bytes();
        let cut = (INDENT.len() + 2..=MAX_WIDTH.min(line.len() - 1))
            .rev()
            .find(|&i| bytes[i] == b' ')
            .filter(|&i| line.is_char_boundary(i))
            .unwrap_or(MAX_WIDTH);
        writeln!(w, "{}", &line[..cut])?;
        line = format!("{INDENT}{}", line[cut..].trim_start());
    }
    Ok(())
}

fn write_feature<W: Write>(w: &mut W, feature: &SeqFeature) -> io::Result<()> {
    writeln!(w, "     {:<15} {}", feature.kind, fmt_location(&feature.location))?;
    for (key, values) in &feature.qualifiers {
        if values.is_empty() {
            write_qualifier(w, key, "", false)?;
        } else {
            for v in values {
                // Biopython writes a bare `/key` only for a qualifier whose value is
                // `None`; a present-but-empty value (which is what its parser makes of
                // a valueless `/environmental_sample`) is written back as `/key=""`.
                // So a GenBank round trip through either implementation turns `/key`
                // into `/key=""`, and guessing from the emptiness of the value here
                // would keep the bare form and differ on the second pass.
                write_qualifier(w, key, v, true)?;
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
/// Rebuild each CDS feature's `domainator_<db>` rollup qualifier from the
/// `Domainator` features (in their current order, grouped by database), and drop
/// stale `domainator_*` qualifiers. Mirrors `utils.write_genbank`'s
/// `add_domain_cds_annotations` (and `tools::domainate::add_domain_cds_annotations`),
/// so pass-through tools reproduce the original's rollup ordering.
fn regenerate_domain_rollups(features: &mut [SeqFeature]) {
    use std::collections::HashMap;
    let mut by_cds: HashMap<String, Vec<(String, Vec<String>)>> = HashMap::new();
    for f in features.iter() {
        if f.kind != crate::DOMAIN_FEATURE_NAME {
            continue;
        }
        let (cds_id, db, name, eval, score, desc) = match (
            f.get_first("cds_id"), f.get_first("database"), f.get_first("name"),
            f.get_first("evalue"), f.get_first("score"), f.get_first("description"),
        ) {
            (Some(c), Some(db), Some(n), Some(e), Some(s), Some(d)) => (c, db, n, e, s, d),
            _ => continue,
        };
        let s = format!("{name} ({desc}, {eval}, {score})");
        let buckets = by_cds.entry(cds_id.to_string()).or_default();
        match buckets.iter_mut().find(|(d, _)| d == db) {
            Some((_, v)) => v.push(s),
            None => buckets.push((db.to_string(), vec![s])),
        }
    }
    for f in features.iter_mut() {
        if f.kind != "CDS" {
            continue;
        }
        let mut seen: Vec<String> = Vec::new();
        if let Some(cds_id) = f.get_first("cds_id").map(str::to_string) {
            if let Some(buckets) = by_cds.get(&cds_id) {
                for (db, strings) in buckets {
                    let key = format!("domainator_{db}");
                    f.set(&key, strings.join(" & "));
                    seen.push(key);
                }
            }
        }
        f.qualifiers.retain(|(k, _)| !k.starts_with("domainator_") || seen.contains(k));
    }
}

pub fn write_record<W: Write>(w: &mut W, rec: &SeqRecord) -> io::Result<()> {
    // Match Biopython's GenBank LOCUS column layout exactly (InsdcIO
    // `_write_the_first_line`): protein records have empty molecule type and `aa`
    // units; the 28-char name+length field has the name left-justified and the
    // length right-justified.
    let locus = if !rec.id.is_empty() {
        rec.id.clone()
    } else if !rec.name.is_empty() {
        rec.name.clone()
    } else {
        rec.annotations.accessions.first().cloned().unwrap_or_default()
    };
    let mol_type = match rec.molecule_type.as_deref() {
        Some("protein") | Some("PROTEIN") => "",
        // "nucleotide" is the internal molecule type for a nucleotide FASTA input
        // (from `--fasta_type nucleotide`); the GenBank LOCUS uses "DNA" (mirrors
        // Python's write_genbank, whose Biopython writer emits DNA).
        Some("nucleotide") => "DNA",
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
    // Biopython writes `annotations["topology"]` and leaves the column blank when
    // there is none, so a record whose LOCUS line never declared one round-trips
    // blank. A tool that *made* the record circular still says so, even though it
    // set no annotation.
    let declared = rec.annotations.topology.or(match rec.topology {
        Topology::Circular => Some(Topology::Circular),
        Topology::Linear => None,
    });
    let topology = if mol_type.is_empty() {
        "        " // 8 blanks
    } else {
        match declared {
            Some(Topology::Circular) => "circular",
            Some(Topology::Linear) => "linear  ",
            None => "        ",
        }
    };
    let division = rec.annotations.division.as_deref().unwrap_or("UNK");
    // `utils.write_genbank` stamps `date.today()` on any record that has none, so a
    // record a tool just built is dated when it was made. Biopython's own
    // `01-JAN-1980` fallback never fires for tool output, and emitting it instead is
    // both visibly wrong and a byte difference on every freshly created record.
    let stamped;
    let date = match rec.annotations.date.as_deref() {
        Some(d) => d,
        None => {
            stamped = today_genbank();
            &stamped
        }
    };
    // "LOCUS       %s %s    %s %s %s %s"
    writeln!(
        w,
        "LOCUS       {} {}    {:<7} {} {} {}",
        name_length, units, mol_type, topology, division, date,
    )?;
    // Biopython writes DEFINITION with a trailing '.' (or just '.' when empty).
    let definition =
        if rec.description.is_empty() { ".".to_string() } else { format!("{}.", rec.description) };
    write_wrapped(w, "DEFINITION", &definition)?;
    let ann = &rec.annotations;
    // ACCESSION / VERSION: derive a default accession from the retained version
    // token (stripping a trailing `.<version>`), prefer the
    // `accessions` annotation, and always emit both lines.
    let version = if rec.name.is_empty() { &rec.id } else { &rec.name };
    let default_acc = match version.split_once('.') {
        Some((base, ver)) if !ver.is_empty() && ver.chars().all(|c| c.is_ascii_digit()) => base,
        _ => version.as_str(),
    };
    let accession = ann.accessions.first().map(String::as_str).unwrap_or(default_acc);
    let acc_with_version = if let Some(rest) = version.strip_prefix(&format!("{accession}.")) {
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
    // Regenerate the `domainator_<db>` CDS rollup from the current feature order
    // and then sort by start — matching `utils.write_genbank`, which rebuilds the
    // rollup (`add_domain_cds_annotations`) before sorting on every write. Done on
    // a clone so the caller's record is not mutated.
    let mut feats: Vec<SeqFeature> = rec.features.clone();
    regenerate_domain_rollups(&mut feats);
    feats.sort_by_key(|f| f.location.start());
    for f in &feats {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seq::Annotations;

    fn qualifier(key: &str, value: &str) -> String {
        let mut out = Vec::new();
        write_qualifier(&mut out, key, value, true).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn only_translation_wraps() {
        // Upstream gives every non-translation qualifier a width of 1e13, i.e. one
        // line however long. Values taken from test/data/simple_genpept.gb, whose
        // description is 74 characters -- comfortably past the 79-column rule stock
        // Biopython would apply, and the case that exposed this.
        let long = "2-oxoacid dehydrogenases acyltransferase (catalytic domain)";
        let out = qualifier("description", long);
        assert_eq!(out.lines().count(), 1);
        assert!(out.len() > 80, "the line is {} chars, so it really is past the old cut", out.len());
        assert!(out.ends_with(&format!("{long}\"\n")));
    }

    #[test]
    fn translation_wraps_at_eighty_columns_including_the_indent() {
        let peptide = "M".repeat(200);
        let out = qualifier("translation", &peptide);
        let lines: Vec<&str> = out.lines().collect();
        // A protein has no spaces, so every line but the last is a hard cut at 80.
        for line in &lines[..lines.len() - 1] {
            assert_eq!(line.len(), 80, "wrapped line: {line:?}");
        }
        assert!(lines.last().unwrap().len() <= 80);
        for line in &lines[1..] {
            assert!(line.starts_with("                     "), "continuation indent");
        }
        // Nothing is lost or duplicated on the way through.
        let rejoined: String = lines.iter().map(|l| l.trim_start()).collect::<Vec<_>>().join("");
        assert_eq!(rejoined, format!("/translation=\"{peptide}\""));
    }

    fn locus_line(rec: &SeqRecord) -> String {
        let mut out = Vec::new();
        write_record(&mut out, rec).unwrap();
        String::from_utf8(out).unwrap().lines().next().unwrap().to_string()
    }

    /// Biopython writes `annotations["topology"]` and leaves the column blank when
    /// the LOCUS line never declared one -- which is most tool-written records.
    #[test]
    fn locus_topology_is_blank_unless_it_was_declared() {
        let mut r = rec(vec![], "d");
        assert!(locus_line(&r).contains("DNA              "), "{}", locus_line(&r));

        r.annotations.topology = Some(Topology::Linear);
        assert!(locus_line(&r).contains("DNA     linear   "), "{}", locus_line(&r));

        // A tool that made the record circular still says so, annotation or not.
        let mut r = rec(vec![], "d");
        r.topology = Topology::Circular;
        assert!(locus_line(&r).contains("DNA     circular "), "{}", locus_line(&r));
    }

    #[test]
    fn numeric_looking_values_keep_their_quotes() {
        // These are all strings in the model, so only the FTQUAL_NO_QUOTE list
        // goes unquoted. Guessing from the value drops the quotes off every
        // numeric annotation these tools write.
        assert_eq!(qualifier("gc", "49"), "                     /gc=\"49\"\n");
        assert_eq!(qualifier("length", "105"), "                     /length=\"105\"\n");
        assert_eq!(qualifier("phase", "0"), "                     /phase=\"0\"\n");
        assert_eq!(
            qualifier("codon_start", "1"),
            "                     /codon_start=1\n"
        );
        assert_eq!(
            qualifier("transl_table", "11"),
            "                     /transl_table=11\n"
        );
    }

    #[test]
    fn a_short_translation_stays_on_one_line() {
        let out = qualifier("translation", "MKV");
        assert_eq!(out, "                     /translation=\"MKV\"\n");
    }

    fn dom(name: &str, start: i64, end: i64, db: &str) -> SeqFeature {
        let mut f = SeqFeature::new(crate::DOMAIN_FEATURE_NAME, Location::range(start, end, 1));
        f.set("cds_id", "C1");
        f.set("name", name);
        f.set("database", db);
        f.set("evalue", "1e-5");
        f.set("score", "10.0");
        f.set("description", format!("desc{name}"));
        f
    }

    fn rec(features: Vec<SeqFeature>, description: &str) -> SeqRecord {
        SeqRecord {
            id: "loc1".into(),
            name: "loc1".into(),
            description: description.into(),
            seq: vec![b'A'; 500],
            molecule_type: Some("DNA".into()),
            topology: Topology::Linear,
            annotations: Annotations::default(),
            features,
        }
    }

    #[test]
    fn rollup_regenerates_in_feature_order_and_drops_stale() {
        let mut cds = SeqFeature::new("CDS", Location::range(100, 400, 1));
        cds.set("cds_id", "C1");
        cds.set("domainator_OLD", "stale"); // a db with no surviving domains
        // Domains added out of start order (B at 250 before A at 150).
        let mut feats = vec![cds, dom("Bdom", 250, 300, "DB"), dom("Adom", 150, 200, "DB")];
        regenerate_domain_rollups(&mut feats);
        let cds = feats.iter().find(|f| f.kind == "CDS").unwrap();
        // Rollup follows feature order (B then A), grouped by database.
        assert_eq!(
            cds.get_first("domainator_DB"),
            Some("Bdom (descBdom, 1e-5, 10.0) & Adom (descAdom, 1e-5, 10.0)")
        );
        // The stale domainator_OLD qualifier is removed.
        assert!(cds.get_first("domainator_OLD").is_none());
    }

    #[test]
    fn writes_definition_with_trailing_period_and_sorts_features() {
        let cds = {
            let mut c = SeqFeature::new("CDS", Location::range(100, 400, 1));
            c.set("cds_id", "C1");
            c
        };
        let r = rec(vec![cds, dom("Bdom", 250, 300, "DB"), dom("Adom", 150, 200, "DB")], "My definition");
        let mut buf = Vec::new();
        write_record(&mut buf, &r).unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(text.contains("DEFINITION  My definition."), "{text}");
        // Features are emitted in ascending start order: Adom (150) before Bdom (250).
        let a = text.find("/name=\"Adom\"").unwrap();
        let b = text.find("/name=\"Bdom\"").unwrap();
        assert!(a < b, "Adom should be written before Bdom");
    }

    #[test]
    fn empty_definition_is_a_lone_period() {
        let r = rec(vec![], "");
        let mut buf = Vec::new();
        write_record(&mut buf, &r).unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(text.lines().any(|l| l.trim_end() == "DEFINITION  ."), "{text}");
    }

    #[test]
    fn genbank_round_trip_preserves_record() {
        use crate::io::genbank::read_genbank;
        let mut cds = SeqFeature::new("CDS", Location::range(100, 400, 1));
        cds.set("cds_id", "C1");
        cds.set("gene_id", "geneA");
        let mut original = rec(vec![cds, dom("Adom", 150, 200, "DB")], "A sample definition");
        original.id = "locus_101:700".into();
        original.name = "ACC001.2".into();
        original.annotations.accessions = vec!["ACC001".into()];
        original.annotations.sequence_version = Some(2);
        original.molecule_type = Some("DNA".into());
        original.seq = b"ACGTACGTACGT".repeat(50); // 600 bp, valid bases
        let dir = std::env::temp_dir();
        let path = dir.join(format!("dom_rt_{}.gb", std::process::id()));
        let ps = path.to_str().unwrap();
        write_genbank(ps, std::slice::from_ref(&original)).unwrap();
        let back = read_genbank(ps).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(back.len(), 1);
        let b = &back[0];
        assert_eq!(b.id, original.id);
        assert_eq!(b.name, original.name);
        assert_eq!(b.annotations.accessions, original.annotations.accessions);
        assert_eq!(b.description, original.description); // period stripped on read
        assert_eq!(b.seq, original.seq);
        // CDS + Domainator features survive the round trip.
        assert_eq!(b.features.iter().filter(|f| f.kind == "CDS").count(), 1);
        let dcds = b.features.iter().find(|f| f.kind == "CDS").unwrap();
        assert_eq!(dcds.get_first("gene_id"), Some("geneA"));
        assert!(dcds.get_first("domainator_DB").is_some());
    }
}
