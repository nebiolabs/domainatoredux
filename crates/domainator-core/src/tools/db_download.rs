//! `db_download` (upstream `db_download.py`): UniProt as FASTA or GenBank, and NCBI
//! genome assemblies as GenBank.
//!
//! UniProt's release files are streamed from UniProt (or a local mirror,
//! `--uniprot_dir`), filtered by taxonomy and fragment status, and appended to the
//! output as they are read: FASTA records unchanged, flat-file entries converted to
//! GenBank by [`crate::io::swissprot`]. NCBI genomes are chosen from the assembly
//! summary (downloaded, or a local file whose `ftp_path` may name local directories)
//! and fetched by one of two backends: `direct`, over HTTPS, or `datasets`, which
//! drives the NCBI Datasets CLI. Either way each genome is validated, and gene-called
//! when asked, by a worker into a file of its own, and the genomes are appended to the
//! output in summary order, whatever order the workers finish in, so the output and
//! the genomes `--max_records` keeps do not depend on timing.
//!
//! The output is written in place and flushed after each genome, and the success log
//! line follows, so a run that stops leaves a database and a log that agree: the log
//! is what a later run's `--exclude_accessions_file` skips.

use std::collections::HashSet;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};

use super::domainate::clean_rec_nucleotide;
use super::external::TempDir;
use super::gene_call::{call_genes, clear_cds_annotations, GeneCall, GeneCallScope};
use crate::io::fasta::{fasta_title, FastaReader};
use crate::io::genbank::writer::write_record;
use crate::io::swissprot::SwissReader;
use crate::seq::location::Location;
use crate::seq::partial::{record_fragment_status, Partial};
use crate::seq::taxid::record_taxid;
use crate::seq::taxonomy::NCBITaxonomy;
use crate::seq::SeqFeature;
use crate::{DOMAIN_FEATURE_NAME, DOMAIN_SEARCH_BEST_HIT_NAME};

/// UniProt's release directory, which holds the `uniprot_<part>.<fasta|dat>.gz` files.
pub const UNIPROT_URL: &str =
    "https://ftp.uniprot.org/pub/databases/uniprot/current_release/knowledgebase/complete";
/// NCBI's GenBank assembly summary.
pub const ASSEMBLY_SUMMARY_URL: &str =
    "https://ftp.ncbi.nlm.nih.gov/genomes/genbank/assembly_summary_genbank.txt";
/// The UniProt databases, which `--partial` and `--uniprot_dir` apply to.
pub const UNIPROT_DBS: [&str; 6] = [
    "swissprot",
    "swissprot_gb",
    "trembl",
    "trembl_gb",
    "uniprot",
    "uniprot_gb",
];
/// The databases downloaded as FASTA; every other one is GenBank.
pub const FASTA_DBS: [&str; 3] = ["swissprot", "trembl", "uniprot"];
/// The `_proks` databases leave out Eukaryota.
const EUKARYOTA_TAXID: i64 = 2759;
/// Lineages whose genomes the nonredundant databases keep however many share a taxid:
/// prokaryotic environmental samples and metagenomes.
const ALWAYS_KEPT_TAXIDS: [i64; 2] = [81490, 408169];

/// `--db` names a UniProt database.
pub fn is_uniprot(db: &str) -> bool {
    UNIPROT_DBS.contains(&db)
}

/// `--db` is written as FASTA.
pub fn is_fasta(db: &str) -> bool {
    FASTA_DBS.contains(&db)
}

/// The release files a UniProt database is made of: `uniprot` and `uniprot_gb` are
/// SwissProt followed by TrEMBL, in one output.
fn uniprot_parts(db: &str) -> &'static [&'static str] {
    match db.trim_end_matches("_gb") {
        "swissprot" => &["sprot"],
        "trembl" => &["trembl"],
        _ => &["sprot", "trembl"],
    }
}

/// How an NCBI database chooses genomes from the assembly summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NcbiSelection {
    /// Leave out Eukaryota.
    pub proks: bool,
    /// One genome per taxid, best `refseq_category` first.
    pub unique: bool,
    /// `assembly_level` "Complete Genome" only.
    pub complete_genome: bool,
    /// Representative and reference genomes only (`refseq_category` other than `na`).
    pub no_na: bool,
}

/// The selection an `ncbi_*` database makes; `None` for a UniProt database.
pub fn ncbi_selection(db: &str) -> Option<NcbiSelection> {
    let s = |proks, unique, complete_genome, no_na| {
        Some(NcbiSelection {
            proks,
            unique,
            complete_genome,
            no_na,
        })
    };
    match db {
        "ncbi_complete_genome_proks" => s(true, false, true, false),
        "ncbi_complete_genome_nonredundant_proks" => s(true, true, true, false),
        "ncbi_representative_proks" => s(true, false, false, true),
        "ncbi_representative_all" => s(false, false, false, true),
        "ncbi_nonredundant_proks" => s(true, true, false, false),
        "ncbi_nonredundant_all" => s(false, true, false, false),
        "ncbi_all" => s(false, false, false, false),
        _ => None,
    }
}

/// Whether a database needs the NCBI taxonomy: for a taxonomy filter, to leave out
/// Eukaryota, or to compare lineages for one genome per taxid.
pub fn needs_taxonomy(db: &str, filtering: bool) -> bool {
    filtering || ncbi_selection(db).is_some_and(|s| s.proks || s.unique)
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn is_url(location: &str) -> bool {
    ["http://", "https://", "ftp://"]
        .iter()
        .any(|scheme| location.starts_with(scheme))
}

// ---------------------------------------------------------------- HTTP

/// The HTTP client, or an error naming the missing feature in a build without one.
#[cfg(feature = "network")]
mod http {
    use std::io;
    use std::time::Duration;

    /// Upstream's `requests` retry policy for genome downloads: up to 10 tries,
    /// backing off 2 s, 4 s, ... up to 2 minutes, on connection errors and on 429 and
    /// 5xx answers, honoring `Retry-After`.
    const RETRIES: u32 = 10;
    const RETRY_STATUSES: [u16; 5] = [429, 500, 502, 503, 504];

    fn agent() -> ureq::Agent {
        ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(60))
            .timeout_read(Duration::from_secs(300))
            .build()
    }

    /// A streaming GET of `url`, retried as above when `retry` is set.
    pub fn get(
        url: &str,
        user_agent: Option<&str>,
        retry: bool,
    ) -> io::Result<Box<dyn io::Read + Send>> {
        let agent = agent();
        let mut attempt = 0;
        loop {
            attempt += 1;
            let mut request = agent.get(url);
            if let Some(user_agent) = user_agent {
                request = request.set("User-Agent", user_agent);
            }
            let wait = match request.call() {
                Ok(response) => return Ok(Box::new(response.into_reader())),
                Err(ureq::Error::Status(code, response)) => {
                    if !retry || attempt >= RETRIES || !RETRY_STATUSES.contains(&code) {
                        return Err(io::Error::other(format!(
                            "{code} {} for url: {url}",
                            response.status_text()
                        )));
                    }
                    response
                        .header("Retry-After")
                        .and_then(|s| s.trim().parse::<u64>().ok())
                        .map(Duration::from_secs)
                }
                Err(e) => {
                    if !retry || attempt >= RETRIES {
                        return Err(io::Error::other(format!("{url}: {e}")));
                    }
                    None
                }
            };
            let backoff = Duration::from_secs((2u64 << (attempt - 1).min(6)).min(120));
            std::thread::sleep(wait.unwrap_or(backoff));
        }
    }
}

#[cfg(not(feature = "network"))]
mod http {
    use std::io;

    pub fn get(
        url: &str,
        _user_agent: Option<&str>,
        _retry: bool,
    ) -> io::Result<Box<dyn io::Read + Send>> {
        Err(io::Error::other(format!(
            "cannot download {url}: this build has no network support; use a local copy"
        )))
    }
}

/// A gzipped file, a URL or a local path, decompressed (every gzip member).
fn open_gzipped(location: &str) -> io::Result<Box<dyn BufRead>> {
    let raw: Box<dyn Read> = if is_url(location) {
        http::get(location, None, false)?
    } else {
        Box::new(std::fs::File::open(location)?)
    };
    Ok(Box::new(BufReader::with_capacity(
        1 << 20,
        flate2::read::MultiGzDecoder::new(BufReader::new(raw)),
    )))
}

// ---------------------------------------------------------------- UniProt

/// What a UniProt record must pass: a taxonomy filter (the allowed taxids) and
/// `--partial`.
#[derive(Debug, Clone, Copy, Default)]
pub struct UniprotFilter<'a> {
    pub allowed: Option<&'a HashSet<i64>>,
    pub partial: Partial,
}

/// The release files `db` is read from, in order: under `uniprot_dir` when given
/// (each must be there), else on UniProt's server.
pub fn uniprot_sources(db: &str, uniprot_dir: Option<&str>) -> io::Result<Vec<String>> {
    let extension = if is_fasta(db) { "fasta" } else { "dat" };
    uniprot_parts(db)
        .iter()
        .map(|part| {
            let file_name = format!("uniprot_{part}.{extension}.gz");
            match uniprot_dir {
                None => Ok(format!("{UNIPROT_URL}/{file_name}")),
                Some(dir) => {
                    let path = Path::new(dir).join(&file_name);
                    if path.is_file() {
                        Ok(path.to_string_lossy().into_owned())
                    } else {
                        Err(invalid(format!(
                            "{} not found: --uniprot_dir must hold the UniProt release files \
                             for --db {db}.",
                            path.display()
                        )))
                    }
                }
            }
        })
        .collect()
}

/// Append the records of the gzipped FASTA at `source` that pass `filter` to `out`,
/// at most `limit` of them. Returns how many were written.
pub fn append_uniprot_fasta(
    source: &str,
    out: &mut dyn Write,
    filter: UniprotFilter<'_>,
    limit: Option<usize>,
) -> io::Result<usize> {
    let progress = crate::progress::Progress::counter(true, file_name(source));
    let mut written = 0;
    for record in FastaReader::new(open_gzipped(source)?, Some("protein")) {
        let record = record?;
        progress.inc(1);
        if filter
            .allowed
            .is_some_and(|allowed| !allowed.contains(&record_taxid(&record)))
        {
            continue;
        }
        if !filter.partial.is_include() && !filter.partial.allows(record_fragment_status(&record)) {
            continue;
        }
        // Biopython's FASTA writer: the title, then the sequence in lines of 60.
        writeln!(out, ">{}", fasta_title(&record.id, &record.description))?;
        for chunk in record.seq.chunks(60) {
            out.write_all(chunk)?;
            out.write_all(b"\n")?;
        }
        written += 1;
        if limit.is_some_and(|limit| written >= limit) {
            break;
        }
    }
    Ok(written)
}

/// Append the entries of the gzipped UniProt flat file at `source` that pass
/// `filter` to `out` as GenBank, at most `limit` of them. Each gets a `source` feature
/// carrying its taxid, and loses the features with a `?` position. Returns how many
/// were written.
pub fn append_uniprot_genbank(
    source: &str,
    mut out: &mut dyn Write,
    filter: UniprotFilter<'_>,
    limit: Option<usize>,
) -> io::Result<usize> {
    let progress = crate::progress::Progress::counter(true, file_name(source));
    let mut written = 0;
    for entry in SwissReader::new(open_gzipped(source)?) {
        let entry = entry?;
        progress.inc(1);
        if let Some(allowed) = filter.allowed {
            if !allowed.contains(&entry.taxid()?) {
                continue;
            }
        }
        let mut record = entry.to_record()?;
        if !filter.partial.is_include() && !filter.partial.allows(record_fragment_status(&record)) {
            continue;
        }
        let taxid = entry.taxonomy_id.first().cloned().unwrap_or_default();
        let mut source_feature =
            SeqFeature::new("source", Location::range(0, record.seq.len() as i64, 1));
        source_feature.set("db_xref", format!("taxon:{taxid}"));
        record.features.insert(0, source_feature);
        write_record(&mut out, &record)?;
        written += 1;
        if limit.is_some_and(|limit| written >= limit) {
            break;
        }
    }
    Ok(written)
}

fn file_name(location: &str) -> &str {
    location.rsplit(['/', '\\']).next().unwrap_or(location)
}

// ---------------------------------------------------------------- assembly summary

/// One row of the assembly summary, keyed by its column names.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Assembly {
    fields: Vec<(String, String)>,
}

impl Assembly {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    fn field(&self, key: &str) -> io::Result<&str> {
        self.get(key).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("the assembly summary has no {key} for {}", self.label()),
            )
        })
    }

    fn set(&mut self, key: &str, value: String) {
        match self.fields.iter_mut().find(|(k, _)| k == key) {
            Some((_, v)) => *v = value,
            None => self.fields.push((key.to_string(), value)),
        }
    }

    /// The accession without its version, which the success log lists.
    pub fn versionless_accession(&self) -> &str {
        self.get("versionless_accession").unwrap_or("")
    }

    fn label(&self) -> &str {
        self.get("assembly_accession").unwrap_or("an assembly")
    }

    fn taxid(&self) -> io::Result<i64> {
        let text = self.field("taxid")?;
        text.trim().parse().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid taxid {text:?} for {}", self.label()),
            )
        })
    }
}

/// Parse an assembly summary: a comment line, a `#`-prefixed header, then one
/// tab-separated row per assembly (`load_ncbi_assembly_summary`). A row's surrounding
/// whitespace is stripped first, so trailing empty columns are absent, as in
/// upstream's dict. With `base_dir` (a local summary's directory), an `ftp_path` that
/// is not a URL is a directory relative to it.
pub fn read_assembly_summary(
    reader: impl BufRead,
    base_dir: Option<&Path>,
) -> io::Result<Vec<Assembly>> {
    let mut lines = reader.lines();
    let missing = || {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "the assembly summary has no header",
        )
    };
    lines.next().ok_or_else(missing)??;
    let header_line = lines.next().ok_or_else(missing)??;
    let mut header: Vec<String> = header_line.trim().split('\t').map(str::to_string).collect();
    header[0] = header[0].trim_matches(['#', ' ']).to_string();
    let mut rows = Vec::new();
    for line in lines {
        let line = line?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut row = Assembly {
            fields: header
                .iter()
                .zip(line.split('\t'))
                .map(|(k, v)| (k.clone(), v.to_string()))
                .collect(),
        };
        let versionless = row
            .field("assembly_accession")?
            .split('.')
            .next()
            .unwrap_or("")
            .to_string();
        row.set("versionless_accession", versionless);
        if let Some(base) = base_dir {
            let ftp_path = row.get("ftp_path").unwrap_or("").trim().to_string();
            if !ftp_path.is_empty() && ftp_path != "na" && !is_url(&ftp_path) {
                row.set(
                    "ftp_path",
                    base.join(ftp_path).to_string_lossy().into_owned(),
                );
            }
        }
        rows.push(row);
    }
    Ok(rows)
}

/// The assembly summary at `path`, or NCBI's when `None`.
pub fn load_assembly_summary(path: Option<&str>) -> io::Result<Vec<Assembly>> {
    crate::log::info(format_args!("Loading assembly_summary_genbank.txt"));
    match path {
        Some(path) => {
            let base = std::path::absolute(path)?
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default();
            read_assembly_summary(BufReader::new(std::fs::File::open(path)?), Some(&base))
        }
        None => read_assembly_summary(
            BufReader::new(http::get(ASSEMBLY_SUMMARY_URL, None, false)?),
            None,
        ),
    }
}

/// A calendar date, compared as `(year, month, day)`.
pub type Date = (i32, u32, u32);

/// A `YYYY<sep>MM<sep>DD` date as Python's `strptime` reads it: one- or two-digit
/// month and day, a four-digit year, and a day that exists.
pub fn parse_date(text: &str, sep: char) -> Option<Date> {
    let mut pieces = text.split(sep);
    let (y, m, d) = (pieces.next()?, pieces.next()?, pieces.next()?);
    if pieces.next().is_some()
        || y.len() != 4
        || !(1..=2).contains(&m.len())
        || !(1..=2).contains(&d.len())
    {
        return None;
    }
    if ![y, m, d]
        .iter()
        .all(|p| p.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let (year, month, day): (i32, u32, u32) = (y.parse().ok()?, m.parse().ok()?, d.parse().ok()?);
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
    let in_month = *days.get(month.checked_sub(1)? as usize)?;
    (year >= 1 && (1..=in_month).contains(&day)).then_some((year, month, day))
}

/// How the assemblies of a summary are narrowed down, in upstream's order.
#[derive(Clone, Default)]
pub struct AssemblyFilters<'a> {
    pub exclude_accessions: Option<&'a HashSet<String>>,
    pub before: Option<Date>,
    pub after: Option<Date>,
    /// The taxids a taxonomy filter allows.
    pub allowed: Option<&'a HashSet<i64>>,
    /// One genome per taxid, judged by this taxonomy's lineages.
    pub unique: Option<&'a NCBITaxonomy>,
    pub complete_genome: bool,
    pub no_na: bool,
}

/// Narrow the summary's assemblies down: those with genome files, then the
/// exclusions, dates and taxonomy, then one per taxid (which puts representative and
/// reference genomes first), the assembly level and the RefSeq category.
pub fn filter_assemblies(
    assemblies: Vec<Assembly>,
    filters: &AssemblyFilters<'_>,
) -> io::Result<Vec<Assembly>> {
    let info = |n: usize, what: &str| crate::log::info(format_args!("{n} accessions {what}"));
    let mut rows = Vec::new();
    for r in assemblies {
        if r.field("ftp_path")?.trim() != "na" {
            rows.push(r);
        }
    }
    info(rows.len(), "in assembly_summary_genbank.txt");
    if let Some(excluded) = filters.exclude_accessions {
        rows.retain(|r| !excluded.contains(r.versionless_accession()));
        info(rows.len(), "after filtering by accession exclusion");
    }
    let release_date = |r: &Assembly| -> io::Result<Date> {
        let text = r.field("seq_rel_date")?;
        parse_date(text, '/').ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("time data {text:?} does not match format '%Y/%m/%d'"),
            )
        })
    };
    for (bound, before) in [(filters.before, true), (filters.after, false)] {
        let Some(bound) = bound else { continue };
        let mut kept = Vec::new();
        for r in rows {
            let date = release_date(&r)?;
            if (before && date < bound) || (!before && date > bound) {
                kept.push(r);
            }
        }
        rows = kept;
        let (y, m, d) = bound;
        info(
            rows.len(),
            &format!(
                "{} {y:04}-{m:02}-{d:02}",
                if before { "before" } else { "after" }
            ),
        );
    }
    if let Some(allowed) = filters.allowed {
        let mut kept = Vec::new();
        for r in rows {
            if allowed.contains(&r.taxid()?) {
                kept.push(r);
            }
        }
        rows = kept;
        info(
            rows.len(),
            "in assembly_summary_genbank.txt after filtering by taxonomy",
        );
    }
    if let Some(taxonomy) = filters.unique {
        // Python's `sort(reverse=True)`, which is stable: representative genomes, then
        // reference genomes, then the rest, each in summary order.
        rows.sort_by(|a, b| b.get("refseq_category").cmp(&a.get("refseq_category")));
        let mut seen = HashSet::new();
        let mut kept = Vec::new();
        for r in rows {
            let taxid = r.taxid()?;
            let lineage = taxonomy.lineage(taxid);
            if seen.insert(taxid) || ALWAYS_KEPT_TAXIDS.iter().any(|t| lineage.contains(t)) {
                kept.push(r);
            }
        }
        rows = kept;
        info(
            rows.len(),
            "in assembly_summary_genbank.txt after filtering by unique taxid",
        );
    }
    if filters.complete_genome {
        rows.retain(|r| r.get("assembly_level") == Some("Complete Genome"));
        info(
            rows.len(),
            "in assembly_summary_genbank.txt after filtering by assembly_level in {'Complete Genome'}",
        );
    }
    if filters.no_na {
        rows.retain(|r| r.get("refseq_category") != Some("na"));
        info(
            rows.len(),
            "in assembly_summary_genbank.txt after filtering by refseq_category != na",
        );
    }
    Ok(rows)
}

// ---------------------------------------------------------------- genomes

/// Copy the text of `gbff` (plain or gzipped) to `out`, with Python's universal
/// newlines (`\r\n` and `\r` become `\n`), after checking it is GenBank: valid UTF-8
/// beginning with a `LOCUS` line, readable to its end (`gbff_is_valid`). `Ok(false)`
/// for a file that is not.
fn copy_valid_gbff(gbff: &Path, mut out: Option<&mut dyn Write>) -> io::Result<bool> {
    let mut reader = crate::io::open_decompressed(gbff)?;
    let mut buf = vec![0u8; 1 << 20];
    let mut pending: Vec<u8> = Vec::new(); // an incomplete UTF-8 sequence
    let mut head = String::new();
    let mut head_checked = false;
    let mut after_cr = false;
    loop {
        let n = match reader.read(&mut buf) {
            Ok(n) => n,
            // A truncated or corrupt gzip stream.
            Err(e) if e.kind() != io::ErrorKind::Interrupted => return Ok(false),
            Err(_) => continue,
        };
        if n == 0 {
            break;
        }
        pending.extend_from_slice(&buf[..n]);
        let valid_len = match std::str::from_utf8(&pending) {
            Ok(_) => pending.len(),
            Err(e) if e.error_len().is_none() => e.valid_up_to(),
            Err(_) => return Ok(false),
        };
        let chunk = std::str::from_utf8(&pending[..valid_len]).expect("checked");
        if !head_checked {
            head.extend(chunk.chars().take(4096 - head.chars().count().min(4096)));
            if head.chars().count() >= 4096 {
                if !head.trim_start().starts_with("LOCUS") {
                    return Ok(false);
                }
                head_checked = true;
            }
        }
        if let Some(out) = out.as_deref_mut() {
            let mut text = Vec::with_capacity(chunk.len());
            for &b in chunk.as_bytes() {
                match b {
                    b'\r' => {
                        text.push(b'\n');
                        after_cr = true;
                        continue;
                    }
                    b'\n' if after_cr => {}
                    _ => text.push(b),
                }
                after_cr = false;
            }
            out.write_all(&text)?;
        }
        pending.drain(..valid_len);
    }
    if !pending.is_empty() {
        return Ok(false);
    }
    Ok(head_checked || head.trim_start().starts_with("LOCUS"))
}

/// Append the records of one local `.gbff(.gz)` to `staged` (`process_local_gbff`):
/// unchanged, or with `gene_call` cleaned and gene-called like `domainate --gene_call`
/// (CDS features normalized; `all` replaces every CDS, `unannotated` calls genes on
/// the contigs with none). `false`, with a warning, for a file that is not GenBank or
/// that fails.
pub fn process_local_gbff(gbff: &Path, staged: &Path, gene_call: Option<&GeneCall>) -> bool {
    let result = (|| -> io::Result<bool> {
        let mut out = io::BufWriter::new(
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(staged)?,
        );
        let Some(gene_call) = gene_call else {
            let ok = copy_valid_gbff(gbff, Some(&mut out))?;
            out.flush()?;
            return Ok(ok);
        };
        if !copy_valid_gbff(gbff, None)? {
            return Ok(false);
        }
        let mut records = crate::io::genbank::read_genbank(&gbff.to_string_lossy())?;
        for rec in &mut records {
            rec.features
                .retain(|f| f.kind != DOMAIN_SEARCH_BEST_HIT_NAME && f.kind != DOMAIN_FEATURE_NAME);
        }
        if gene_call.scope == GeneCallScope::All {
            clear_cds_annotations(&mut records);
        }
        let mut unannotated = Vec::new();
        for (i, rec) in records.iter_mut().enumerate() {
            if rec.is_protein() || clean_rec_nucleotide(rec) == 0 {
                unannotated.push(i);
            }
        }
        call_genes(&mut records, &unannotated, &gene_call.options)?;
        for rec in &records {
            write_record(&mut out, rec)?;
        }
        out.flush()?;
        Ok(true)
    })();
    match result {
        Ok(true) => true,
        Ok(false) => {
            crate::log::warn(format_args!(
                "gbff validation failed for {}, skipping",
                gbff.display()
            ));
            false
        }
        Err(e) => {
            crate::log::warn(format_args!("error processing {}: {e}", gbff.display()));
            false
        }
    }
}

/// Run `task` over `items` on `workers` threads, handing the results to `consume` in
/// item order (upstream's `imap_bounded`): at most `2 * workers` items are started
/// ahead of the one being consumed. `consume` returning `Break` stops the run; tasks
/// see that through the flag they are given, and items not yet started are skipped.
fn ordered_parallel<T: Sync, R: Send>(
    items: &[T],
    workers: usize,
    task: impl Fn(usize, &T, &AtomicBool) -> R + Sync,
    mut consume: impl FnMut(usize, R) -> io::Result<ControlFlow<()>>,
) -> io::Result<()> {
    let workers = workers.max(1);
    let max_pending = 2 * workers;
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let consumed = Mutex::new(0usize);
    let advanced = Condvar::new();
    let (tx, rx) = std::sync::mpsc::channel::<(usize, R)>();
    std::thread::scope(|scope| {
        for _ in 0..workers {
            let tx = tx.clone();
            let (next, stop, consumed, advanced, task) =
                (&next, &stop, &consumed, &advanced, &task);
            scope.spawn(move || loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                if i >= items.len() || stop.load(Ordering::SeqCst) {
                    break;
                }
                let mut done = consumed.lock().unwrap_or_else(|e| e.into_inner());
                while i >= *done + max_pending && !stop.load(Ordering::SeqCst) {
                    done = advanced.wait(done).unwrap_or_else(|e| e.into_inner());
                }
                drop(done);
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                if tx.send((i, task(i, &items[i], stop))).is_err() {
                    break;
                }
            });
        }
        drop(tx);
        let halt = || {
            stop.store(true, Ordering::SeqCst);
            let _guard = consumed.lock().unwrap_or_else(|e| e.into_inner());
            advanced.notify_all();
        };
        let mut waiting = std::collections::BTreeMap::new();
        let mut expected = 0;
        for (i, result) in rx {
            waiting.insert(i, result);
            while let Some(result) = waiting.remove(&expected) {
                let flow = consume(expected, result);
                expected += 1;
                {
                    let mut done = consumed.lock().unwrap_or_else(|e| e.into_inner());
                    *done = expected;
                    advanced.notify_all();
                }
                match flow {
                    Ok(ControlFlow::Continue(())) => {}
                    Ok(ControlFlow::Break(())) => {
                        halt();
                        return Ok(());
                    }
                    Err(e) => {
                        halt();
                        return Err(e);
                    }
                }
            }
        }
        Ok(())
    })
}

/// Where the genomes go: the database, flushed after each genome, and the success
/// log, which lists each genome's accession once it is written.
pub struct GenomeSink<'a> {
    pub out: &'a mut dyn Write,
    pub success_log: Option<&'a mut dyn Write>,
}

impl GenomeSink<'_> {
    /// Append a worker's file to the database, log `accession`, and delete the file.
    fn append(&mut self, staged: &Path, accession: &str) -> io::Result<()> {
        io::copy(&mut std::fs::File::open(staged)?, self.out)?;
        self.out.flush()?;
        if let Some(log) = self.success_log.as_deref_mut() {
            writeln!(log, "{accession}")?;
            log.flush()?;
        }
        let _ = std::fs::remove_file(staged);
        Ok(())
    }
}

/// How many genomes were appended, could not be fetched or processed, and (datasets
/// backend) were requested but never arrived.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DownloadSummary {
    pub appended: usize,
    pub failed: usize,
    pub missing: usize,
}

/// The direct backend's options.
#[derive(Debug, Clone, Default)]
pub struct DirectOptions {
    pub gene_call: Option<GeneCall>,
    /// At most this many genomes are written.
    pub max_records: Option<usize>,
    /// Simultaneous downloads.
    pub workers: usize,
    pub user_agent: Option<String>,
    /// Where downloads and processed genomes wait (the system temp directory when
    /// `None`).
    pub tmp_dir: Option<PathBuf>,
}

/// Fetch one assembly's `<name>_genomic.gbff.gz` into `staging` and process it to
/// `staging/<index>.gb` (`worker_process`). An `ftp_path` that is not a URL is a local
/// directory, read in place.
fn fetch_genome(
    index: usize,
    assembly: &Assembly,
    staging: &Path,
    options: &DirectOptions,
    stop: &AtomicBool,
) -> Option<PathBuf> {
    let ftp_path = assembly.get("ftp_path").unwrap_or("").trim_end_matches('/');
    let staged = staging.join(format!("{index}.gb"));
    let name = ftp_path.rsplit('/').next().unwrap_or(ftp_path);
    if !is_url(ftp_path) {
        let local_name = Path::new(ftp_path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let local = Path::new(ftp_path).join(format!("{local_name}_genomic.gbff.gz"));
        if !local.is_file() {
            crate::log::warn(format_args!("{} not found, skipping", local.display()));
            return None;
        }
        return process_local_gbff(&local, &staged, options.gene_call.as_ref()).then_some(staged);
    }
    let remote = format!("{ftp_path}/{name}_genomic.gbff.gz");
    let download = staging.join(format!("{index}.gbff.gz"));
    let fetched = (|| -> io::Result<bool> {
        let mut response = http::get(&remote, options.user_agent.as_deref(), true)?;
        let mut file = io::BufWriter::new(std::fs::File::create(&download)?);
        let mut buf = vec![0u8; 1 << 20];
        loop {
            if stop.load(Ordering::SeqCst) {
                return Ok(false);
            }
            let n = response.read(&mut buf)?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])?;
        }
        file.flush()?;
        Ok(true)
    })();
    let result = match fetched {
        Ok(true) => {
            process_local_gbff(&download, &staged, options.gene_call.as_ref()).then_some(staged)
        }
        Ok(false) => None,
        Err(e) => {
            crate::log::warn(format_args!("error downloading {remote}: {e}"));
            None
        }
    };
    let _ = std::fs::remove_file(&download);
    result
}

/// The direct backend (`process_genbank_accessions`): download each assembly's
/// genome over HTTPS, `options.workers` at a time, and append them in order.
pub fn process_direct(
    assemblies: &[Assembly],
    sink: &mut GenomeSink<'_>,
    options: &DirectOptions,
) -> io::Result<DownloadSummary> {
    let staging = TempDir::new_in(options.tmp_dir.as_deref(), "domainatoredux_db_download")?;
    let progress = crate::progress::Progress::new(true, assemblies.len(), "GenBank genomes");
    let mut summary = DownloadSummary::default();
    ordered_parallel(
        assemblies,
        options.workers,
        |i, assembly, stop| fetch_genome(i, assembly, staging.path(), options, stop),
        |i, staged| {
            progress.inc(1);
            let Some(staged) = staged else {
                summary.failed += 1;
                return Ok(ControlFlow::Continue(()));
            };
            sink.append(&staged, assemblies[i].versionless_accession())?;
            summary.appended += 1;
            Ok(
                if options
                    .max_records
                    .is_some_and(|max| summary.appended >= max)
                {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                },
            )
        },
    )?;
    Ok(summary)
}

/// The datasets backend's options.
#[derive(Debug, Clone, Default)]
pub struct DatasetsOptions {
    pub gene_call: Option<GeneCall>,
    /// At most this many genomes are requested.
    pub max_records: Option<usize>,
    /// Threads for the local processing.
    pub cpu: usize,
    /// `--datasets_path`: an explicit `datasets` executable, else it is looked up the
    /// usual way. Resolved by [`process_datasets`], so a selection with nothing to
    /// download succeeds without `datasets` installed, as upstream's does.
    pub datasets_path: Option<String>,
    pub workdir: PathBuf,
    pub include: String,
    pub max_workers: i64,
    pub api_key: Option<String>,
    pub gzip: bool,
}

/// Run an external command with inherited output, `retries` times at most, waiting
/// 30 s, 60 s, ... between tries. With `check`, a command that never succeeds is an
/// error; without, its last exit status is returned.
fn run_retried(argv: &[String], retries: u32, check: bool) -> io::Result<std::process::ExitStatus> {
    let shown = argv.join(" ");
    let mut last = None;
    for attempt in 1..=retries {
        crate::log::info(format_args!("+ {shown}"));
        let mut command = std::process::Command::new(&argv[0]);
        command.args(&argv[1..]);
        super::external::hide_console(&mut command);
        let status = command
            .status()
            .map_err(|e| io::Error::other(format!("running {}: {e}", argv[0])))?;
        if status.success() {
            return Ok(status);
        }
        let code = status
            .code()
            .map_or("signal".to_string(), |c| c.to_string());
        if attempt < retries {
            let wait = 30 * u64::from(attempt);
            crate::log::warn(format_args!(
                "command failed (rc={code}); retrying in {wait}s (attempt {attempt}/{} of retries used)",
                retries - 1
            ));
            std::thread::sleep(std::time::Duration::from_secs(wait));
        }
        last = Some((status, code));
    }
    let (status, code) = last.expect("at least one attempt");
    let message = format!("command failed (rc={code}) after {retries} attempt(s): {shown}");
    if check {
        return Err(io::Error::other(message));
    }
    crate::log::warn(format_args!("{message}"));
    Ok(status)
}

/// Extract a zip archive into `dir` (what upstream runs `unzip -o` for).
fn extract_zip(zip_path: &Path, dir: &Path) -> io::Result<()> {
    crate::log::info(format_args!(
        "+ unzip -o {} -d {}",
        zip_path.display(),
        dir.display()
    ));
    let mut archive = zip::ZipArchive::new(std::fs::File::open(zip_path)?)
        .map_err(|e| io::Error::other(format!("reading {}: {e}", zip_path.display())))?;
    archive
        .extract(dir)
        .map_err(|e| io::Error::other(format!("extracting {}: {e}", zip_path.display())))
}

/// The rehydrated genome files: `ncbi_dataset/data/*/*.gbff`, then `*.gbff.gz`, each
/// sorted by path.
fn rehydrated_files(extract_dir: &Path) -> io::Result<Vec<PathBuf>> {
    let data = extract_dir.join("ncbi_dataset").join("data");
    let mut plain = Vec::new();
    let mut gzipped = Vec::new();
    if data.is_dir() {
        for dir in std::fs::read_dir(&data)? {
            let dir = dir?.path();
            if !dir.is_dir() {
                continue;
            }
            for file in std::fs::read_dir(&dir)? {
                let file = file?.path();
                let name = file
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if name.ends_with(".gbff") {
                    plain.push(file);
                } else if name.ends_with(".gbff.gz") {
                    gzipped.push(file);
                }
            }
        }
    }
    plain.sort();
    gzipped.sort();
    plain.extend(gzipped);
    Ok(plain)
}

/// The accession a rehydrated file belongs to: its directory's name, versionless.
fn rehydrated_accession(path: &Path) -> String {
    path.parent()
        .and_then(Path::file_name)
        .map(|n| {
            n.to_string_lossy()
                .split('.')
                .next()
                .unwrap_or("")
                .to_string()
        })
        .unwrap_or_default()
}

/// The datasets backend (`process_via_datasets`): `datasets download genome
/// accession --dehydrated`, extract, `datasets rehydrate`, then process the genome
/// files, `options.cpu` at a time, appending them in path order.
pub fn process_datasets(
    assemblies: &[Assembly],
    sink: &mut GenomeSink<'_>,
    options: &DatasetsOptions,
) -> io::Result<DownloadSummary> {
    // First, as upstream's process_via_datasets does: `--max_records 0` still needs it.
    let datasets = super::external::resolve_binary(
        &super::external::DATASETS,
        options.datasets_path.as_deref(),
        "Install the NCBI Datasets CLI (e.g. `conda install -c conda-forge ncbi-datasets-cli`).",
    )?;
    let mut accessions: Vec<String> = assemblies
        .iter()
        .map(|a| a.field("assembly_accession").map(str::to_string))
        .collect::<io::Result<_>>()?;
    if let Some(max) = options.max_records {
        // Cap the download, not just the appending, so --max_records is a cheap test.
        accessions.truncate(max);
    }
    if accessions.is_empty() {
        // `datasets` rejects an empty accession file (and would be retried for a
        // minute first); nothing to fetch is a successful, empty download.
        crate::log::info("0 assemblies to download");
        return Ok(DownloadSummary::default());
    }
    let workdir = &options.workdir;
    std::fs::create_dir_all(workdir)?;
    let accession_file = workdir.join("accessions.txt");
    std::fs::write(
        &accession_file,
        accessions
            .iter()
            .map(|a| format!("{a}\n"))
            .collect::<String>(),
    )?;
    let zip_path = workdir.join("ncbi_dataset.zip");
    let extract_dir = workdir.join("extracted");
    std::fs::create_dir_all(&extract_dir)?;
    let datasets = datasets.to_string_lossy().into_owned();
    let path = |p: &Path| p.to_string_lossy().into_owned();

    let mut download = vec![
        datasets.clone(),
        "download".into(),
        "genome".into(),
        "accession".into(),
        "--inputfile".into(),
        path(&accession_file),
        "--include".into(),
        options.include.clone(),
        "--dehydrated".into(),
        "--filename".into(),
        path(&zip_path),
    ];
    if let Some(key) = &options.api_key {
        download.extend(["--api-key".into(), key.clone()]);
    }
    run_retried(&download, 3, true)?;
    extract_zip(&zip_path, &extract_dir)?;
    let mut rehydrate = vec![
        datasets,
        "rehydrate".into(),
        "--directory".into(),
        path(&extract_dir),
        "--max-workers".into(),
        options.max_workers.to_string(),
    ];
    if options.gzip {
        rehydrate.push("--gzip".into());
    }
    if let Some(key) = &options.api_key {
        rehydrate.extend(["--api-key".into(), key.clone()]);
    }
    // `datasets rehydrate` fails if any one assembly is unavailable; that must not
    // abort a large batch, so only a rehydrate that produced nothing is an error.
    let status = run_retried(&rehydrate, 3, false)?;

    let files = rehydrated_files(&extract_dir)?;
    if files.is_empty() {
        return Err(io::Error::other(
            "rehydrate produced no genome files; every requested assembly was unavailable or the \
             dehydrated package was empty. See the datasets errors above.",
        ));
    }
    let found: HashSet<String> = files.iter().map(|f| rehydrated_accession(f)).collect();
    let requested: HashSet<&str> = accessions
        .iter()
        .map(|a| a.split('.').next().unwrap_or(""))
        .collect();
    let missing = requested.iter().filter(|a| !found.contains(**a)).count();
    if !status.success() {
        crate::log::warn(format_args!(
            "`datasets rehydrate` exited rc={} (some assemblies unavailable); proceeding with the \
             {} genome(s) that rehydrated.",
            status
                .code()
                .map_or("signal".to_string(), |c| c.to_string()),
            files.len()
        ));
    }
    if missing > 0 {
        crate::log::warn(format_args!(
            "{missing} requested assemblies have no .gbff.gz after rehydrate"
        ));
    }

    let staging = TempDir::new_in(Some(workdir), "domainatoredux_db_download")?;
    let progress = crate::progress::Progress::new(true, files.len(), "Datasets genomes");
    let mut summary = DownloadSummary {
        missing,
        ..DownloadSummary::default()
    };
    ordered_parallel(
        &files,
        options.cpu,
        |i, file, _| {
            let staged = staging.path().join(format!("{i}.gb"));
            process_local_gbff(file, &staged, options.gene_call.as_ref()).then_some(staged)
        },
        |i, staged| {
            progress.inc(1);
            match staged {
                Some(staged) => {
                    sink.append(&staged, &rehydrated_accession(&files[i]))?;
                    summary.appended += 1;
                }
                None => summary.failed += 1,
            }
            Ok(ControlFlow::Continue(()))
        },
    )?;
    Ok(summary)
}

// ---------------------------------------------------------------- the tool

/// Which NCBI backend fetches genomes.
#[derive(Debug, Clone)]
pub enum Backend {
    Direct(DirectOptions),
    Datasets(DatasetsOptions),
}

/// `db_download`'s options, named like its command line.
#[derive(Debug, Clone, Default)]
pub struct DbDownloadOptions {
    pub db: String,
    pub include_taxids: Option<HashSet<i64>>,
    pub exclude_taxids: Option<HashSet<i64>>,
    pub taxonomy_expr: Option<String>,
    pub partial: Partial,
    pub uniprot_dir: Option<String>,
    /// UniProt records, or NCBI genomes, at most.
    pub max_records: Option<usize>,
    pub ncbi_summary: Option<String>,
    pub exclude_accessions: Option<HashSet<String>>,
    pub before: Option<Date>,
    pub after: Option<Date>,
}

/// The taxids a taxonomy filter allows (`compile_taxonomy_allowed`), `None` without
/// criteria.
fn allowed_taxids(
    taxonomy: Option<&NCBITaxonomy>,
    include: Option<&HashSet<i64>>,
    exclude: Option<&HashSet<i64>>,
    expression: Option<&str>,
) -> io::Result<Option<HashSet<i64>>> {
    let expression = expression.filter(|e| !e.is_empty());
    let active = expression.is_some()
        || include.is_some_and(|s| !s.is_empty())
        || exclude.is_some_and(|s| !s.is_empty());
    if !active {
        return Ok(None);
    }
    let taxonomy = taxonomy
        .ok_or_else(|| invalid("Taxonomy filtering requires an NCBI taxonomy database."))?;
    match expression {
        Some(expression) => taxonomy
            .compile_taxonomy_expression(expression)
            .map_err(invalid),
        None => Ok(taxonomy.compile_taxonomy_filter(include, exclude)),
    }
}

/// Download `options.db` and append it to `out` (upstream's `db_download`): UniProt
/// records, or NCBI genomes through `backend` (needed for an NCBI database), whose
/// accessions are also written to `success_log`. `taxonomy` must be given for a
/// taxonomy filter and for the databases [`needs_taxonomy`] names.
pub fn db_download<'a>(
    options: &DbDownloadOptions,
    taxonomy: Option<&NCBITaxonomy>,
    backend: Option<&Backend>,
    out: &'a mut dyn Write,
    success_log: Option<&'a mut dyn Write>,
) -> io::Result<()> {
    let db = options.db.as_str();
    let uniprot_list = UNIPROT_DBS.join(", ");
    if options
        .taxonomy_expr
        .as_deref()
        .is_some_and(|e| !e.is_empty())
        && (options.include_taxids.is_some() || options.exclude_taxids.is_some())
    {
        return Err(invalid(
            "taxonomy_expr is mutually exclusive with include_taxids/exclude_taxids.",
        ));
    }
    if !options.partial.is_include() && !is_uniprot(db) {
        return Err(invalid(format!(
            "partial only applies to the UniProt databases ({uniprot_list}), not {db}."
        )));
    }
    if options.uniprot_dir.is_some() && !is_uniprot(db) {
        return Err(invalid(format!(
            "uniprot_dir only applies to the UniProt databases ({uniprot_list}), not {db}."
        )));
    }

    if is_uniprot(db) {
        let allowed = allowed_taxids(
            taxonomy,
            options.include_taxids.as_ref(),
            options.exclude_taxids.as_ref(),
            options.taxonomy_expr.as_deref(),
        )?;
        let filter = UniprotFilter {
            allowed: allowed.as_ref(),
            partial: options.partial,
        };
        let mut remaining = options.max_records;
        for source in uniprot_sources(db, options.uniprot_dir.as_deref())? {
            if remaining == Some(0) {
                break;
            }
            let written = if is_fasta(db) {
                append_uniprot_fasta(&source, out, filter, remaining)?
            } else {
                append_uniprot_genbank(&source, out, filter, remaining)?
            };
            remaining = remaining.map(|r| r - written);
        }
        return Ok(());
    }

    let Some(selection) = ncbi_selection(db) else {
        return Err(invalid(format!("unknown database {db}")));
    };
    // The _proks databases always leave out Eukaryota, on top of the other criteria.
    let (include, exclude, expression) = if selection.proks {
        match options.taxonomy_expr.as_deref().filter(|e| !e.is_empty()) {
            Some(expression) => (
                None,
                None,
                Some(format!("({expression}) & ~{EUKARYOTA_TAXID}")),
            ),
            None => {
                let mut exclude = options.exclude_taxids.clone().unwrap_or_default();
                exclude.insert(EUKARYOTA_TAXID);
                (options.include_taxids.clone(), Some(exclude), None)
            }
        }
    } else {
        (
            options.include_taxids.clone(),
            options.exclude_taxids.clone(),
            options.taxonomy_expr.clone(),
        )
    };
    if selection.unique && taxonomy.is_none() {
        return Err(invalid(
            "if include_taxa, exclude_taxa, taxonomy_expr, or filter_by_uniqueness are specified, \
             then taxonomy_database must also be supplied",
        ));
    }
    let allowed = allowed_taxids(
        taxonomy,
        include.as_ref(),
        exclude.as_ref(),
        expression.as_deref(),
    )?;
    let assemblies = filter_assemblies(
        load_assembly_summary(options.ncbi_summary.as_deref())?,
        &AssemblyFilters {
            exclude_accessions: options.exclude_accessions.as_ref(),
            before: options.before,
            after: options.after,
            allowed: allowed.as_ref(),
            unique: taxonomy.filter(|_| selection.unique),
            complete_genome: selection.complete_genome,
            no_na: selection.no_na,
        },
    )?;
    if assemblies.is_empty() {
        // An incremental update with nothing new: leave the outputs empty and succeed,
        // rather than ask a backend for nothing (`datasets` refuses an empty list).
        crate::log::info("0 assemblies to download");
        return Ok(());
    }
    let mut sink = GenomeSink { out, success_log };
    let (name, summary) = match backend {
        Some(Backend::Direct(direct)) => {
            ("direct", process_direct(&assemblies, &mut sink, direct)?)
        }
        Some(Backend::Datasets(datasets)) => (
            "datasets",
            process_datasets(&assemblies, &mut sink, datasets)?,
        ),
        None => return Err(invalid("an NCBI database needs a download backend")),
    };
    crate::log::info(format_args!(
        "Download summary ({name} backend): targeted={}, appended={}, failed={}, missing={}",
        assemblies.len(),
        summary.appended,
        summary.failed,
        summary.missing
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("db_download_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const SUMMARY: &str = "#   See the README\n\
#assembly_accession\trefseq_category\ttaxid\tassembly_level\tseq_rel_date\tftp_path\texcluded_from_refseq\n\
GCA_1.1\tna\t562\tComplete Genome\t2020/01/15\tgenomes/GCA_1.1_A\t\n\
GCA_2.1\treference genome\t562\tScaffold\t2021/06/01\thttps://ftp.example/GCA_2.1_B\n\
GCA_3.1\trepresentative genome\t9\tComplete Genome\t2019/03/10\tna\n";

    #[test]
    fn the_summary_is_read_like_upstream() {
        let rows = read_assembly_summary(SUMMARY.as_bytes(), Some(Path::new("/mirror"))).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].versionless_accession(), "GCA_1");
        // The trailing empty column is stripped with the line, so it is absent.
        assert_eq!(rows[0].get("excluded_from_refseq"), None);
        assert_eq!(rows[0].get("ftp_path"), Some("/mirror/genomes/GCA_1.1_A"));
        assert_eq!(
            rows[1].get("ftp_path"),
            Some("https://ftp.example/GCA_2.1_B")
        );
        assert_eq!(rows[2].get("ftp_path"), Some("na"));
    }

    #[test]
    fn filters_apply_in_upstream_order() {
        let rows = read_assembly_summary(SUMMARY.as_bytes(), None).unwrap();
        let ids = |rows: &[Assembly]| {
            rows.iter()
                .map(|r| r.versionless_accession().to_string())
                .collect::<Vec<_>>()
        };
        let all = filter_assemblies(rows.clone(), &AssemblyFilters::default()).unwrap();
        assert_eq!(ids(&all), ["GCA_1", "GCA_2"], "no ftp_path, no genome");
        let before = filter_assemblies(
            rows.clone(),
            &AssemblyFilters {
                before: Some((2021, 6, 1)),
                ..AssemblyFilters::default()
            },
        )
        .unwrap();
        assert_eq!(ids(&before), ["GCA_1"], "strictly before");
        let complete = filter_assemblies(
            rows.clone(),
            &AssemblyFilters {
                complete_genome: true,
                no_na: true,
                ..AssemblyFilters::default()
            },
        )
        .unwrap();
        assert!(complete.is_empty());
        let allowed: HashSet<i64> = [562].into();
        let excluded: HashSet<String> = ["GCA_1".to_string()].into();
        let kept = filter_assemblies(
            rows,
            &AssemblyFilters {
                allowed: Some(&allowed),
                exclude_accessions: Some(&excluded),
                ..AssemblyFilters::default()
            },
        )
        .unwrap();
        assert_eq!(ids(&kept), ["GCA_2"]);
    }

    #[test]
    fn dates_are_read_as_strptime_reads_them() {
        assert_eq!(parse_date("2021-6-1", '-'), Some((2021, 6, 1)));
        assert_eq!(parse_date("2020/02/29", '/'), Some((2020, 2, 29)));
        assert_eq!(parse_date("2021/02/29", '/'), None);
        assert_eq!(parse_date("21-06-01", '-'), None);
        assert_eq!(parse_date("2021-06-01-", '-'), None);
    }

    #[test]
    fn a_genome_is_validated_and_copied_with_universal_newlines() {
        let dir = scratch("copy");
        let good = dir.join("good.gbff");
        std::fs::write(&good, "\n  LOCUS       X\r\nORIGIN\r//\n").unwrap();
        let staged = dir.join("0.gb");
        assert!(process_local_gbff(&good, &staged, None));
        assert_eq!(
            std::fs::read_to_string(&staged).unwrap(),
            "\n  LOCUS       X\nORIGIN\n//\n"
        );

        let not_genbank = dir.join("page.html");
        std::fs::write(&not_genbank, "<html>error</html>").unwrap();
        assert!(!process_local_gbff(&not_genbank, &dir.join("1.gb"), None));

        // A truncated gzip download.
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(&"LOCUS       X\n".repeat(5000).into_bytes())
            .unwrap();
        let bytes = gz.finish().unwrap();
        let truncated = dir.join("cut.gbff.gz");
        std::fs::write(&truncated, &bytes[..bytes.len() / 2]).unwrap();
        assert!(!process_local_gbff(&truncated, &dir.join("2.gb"), None));
        let whole = dir.join("whole.gbff.gz");
        std::fs::write(&whole, &bytes).unwrap();
        assert!(process_local_gbff(&whole, &dir.join("3.gb"), None));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ordered_parallel_consumes_in_order_and_stops() {
        let items: Vec<usize> = (0..50).collect();
        let mut seen = Vec::new();
        ordered_parallel(
            &items,
            4,
            |i, _, _| {
                // Later items finish first.
                std::thread::sleep(std::time::Duration::from_millis((50 - i as u64) % 7));
                i * 10
            },
            |i, r| {
                assert_eq!(r, i * 10);
                seen.push(i);
                Ok(if i == 30 {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                })
            },
        )
        .unwrap();
        assert_eq!(seen, (0..=30).collect::<Vec<_>>());
    }

    #[test]
    fn rehydrated_files_are_plain_then_gzipped_each_sorted() {
        let dir = scratch("rehydrated");
        for (acc, name) in [
            ("GCA_2.1", "genomic.gbff"),
            ("GCA_1.1", "genomic.gbff.gz"),
            ("GCA_3.1", "genomic.gbff"),
        ] {
            let d = dir.join("ncbi_dataset").join("data").join(acc);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join(name), "").unwrap();
        }
        let accessions: Vec<String> = rehydrated_files(&dir)
            .unwrap()
            .iter()
            .map(|f| rehydrated_accession(f))
            .collect();
        assert_eq!(accessions, ["GCA_2", "GCA_3", "GCA_1"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Nothing selected (an incremental update with nothing new, or `--max_records 0`):
    /// `datasets` is never run, since it rejects an empty accession list, and the
    /// download succeeds with nothing appended.
    #[test]
    fn an_empty_selection_never_runs_datasets() {
        let dir = scratch("datasets_empty");
        let summary = "#\n#assembly_accession\tftp_path\nGCA_1.1\tx\n";
        let one = read_assembly_summary(summary.as_bytes(), None).unwrap();
        for (assemblies, max_records) in [(&one[..0], None), (&one[..], Some(0))] {
            let options = DatasetsOptions {
                max_records,
                // An executable that is never run: running it would fail the download.
                datasets_path: Some(std::env::current_exe().unwrap().to_string_lossy().into()),
                workdir: dir.join("work"),
                ..DatasetsOptions::default()
            };
            let (mut out, mut log) = (Vec::new(), Vec::new());
            let result = process_datasets(
                assemblies,
                &mut GenomeSink {
                    out: &mut out,
                    success_log: Some(&mut log),
                },
                &options,
            )
            .unwrap();
            assert_eq!(result, DownloadSummary::default());
            assert!(out.is_empty() && log.is_empty());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The datasets backend against a stand-in `datasets` that hands over a prepared
    /// package and "rehydrates" two of the three requested genomes.
    #[cfg(unix)]
    #[test]
    fn the_datasets_backend_requests_extracts_and_appends_in_path_order() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("datasets");
        let package = dir.join("package.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&package).unwrap());
        zip.start_file("README.md", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"dehydrated").unwrap();
        zip.finish().unwrap();
        let calls = dir.join("calls.txt");
        let fake = dir.join("datasets");
        std::fs::write(
            &fake,
            format!(
                "#!/bin/sh\n\
                 echo \"$@\" >> '{calls}'\n\
                 cmd=$1\n\
                 while [ $# -gt 0 ]; do\n\
                   case $1 in --filename) out=$2;; --directory) dest=$2;; esac\n\
                   shift\n\
                 done\n\
                 if [ \"$cmd\" = download ]; then cp '{package}' \"$out\"; exit 0; fi\n\
                 for acc in GCA_2.1 GCA_1.1; do\n\
                   mkdir -p \"$dest/ncbi_dataset/data/$acc\"\n\
                   printf 'LOCUS       %s\\n//\\n' $acc > \"$dest/ncbi_dataset/data/$acc/genomic.gbff\"\n\
                 done\n",
                calls = calls.display(),
                package = package.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();

        let summary = "#\n#assembly_accession\tftp_path\nGCA_1.1\tx\nGCA_2.1\tx\nGCA_3.1\tx\n";
        let assemblies = read_assembly_summary(summary.as_bytes(), None).unwrap();
        let options = DatasetsOptions {
            max_records: Some(2),
            cpu: 2,
            datasets_path: Some(fake.to_string_lossy().into()),
            workdir: dir.join("work"),
            include: "gbff".into(),
            max_workers: 10,
            gzip: true,
            ..DatasetsOptions::default()
        };
        let (mut out, mut log) = (Vec::new(), Vec::new());
        let result = process_datasets(
            &assemblies,
            &mut GenomeSink {
                out: &mut out,
                success_log: Some(&mut log),
            },
            &options,
        )
        .unwrap();
        assert_eq!(
            result,
            DownloadSummary {
                appended: 2,
                failed: 0,
                missing: 0
            }
        );
        // --max_records caps the request itself.
        let requested = std::fs::read_to_string(dir.join("work").join("accessions.txt")).unwrap();
        assert_eq!(requested, "GCA_1.1\nGCA_2.1\n");
        assert!(
            dir.join("work/extracted/README.md").is_file(),
            "the package was extracted"
        );
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "LOCUS       GCA_1.1\n//\nLOCUS       GCA_2.1\n//\n"
        );
        assert_eq!(String::from_utf8(log).unwrap(), "GCA_1\nGCA_2\n");
        let calls = std::fs::read_to_string(&calls).unwrap();
        let lines: Vec<&str> = calls.lines().collect();
        assert!(
            lines[0].starts_with("download genome accession --inputfile"),
            "{calls}"
        );
        assert!(
            lines[0].contains("--include gbff --dehydrated --filename"),
            "{calls}"
        );
        assert!(
            lines[1].starts_with("rehydrate --directory")
                && lines[1].ends_with("--max-workers 10 --gzip"),
            "{calls}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn database_tables() {
        assert!(is_fasta("trembl") && !is_fasta("trembl_gb"));
        assert_eq!(uniprot_parts("uniprot_gb"), ["sprot", "trembl"]);
        let representative_all = ncbi_selection("ncbi_representative_all").unwrap();
        assert!(representative_all.no_na && !representative_all.proks);
        assert!(needs_taxonomy("ncbi_nonredundant_all", false));
        assert!(!needs_taxonomy("ncbi_all", false) && !needs_taxonomy("swissprot", false));
    }
}
