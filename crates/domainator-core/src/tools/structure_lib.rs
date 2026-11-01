//! Shared plumbing for the structure tools (upstream `structure_lib.py`):
//! `structure_to_genbank`, `structure_domainate`, `structure_search` and
//! `structure_dist`.
//!
//! Three things live here:
//!
//! * **Input resolution.** An `-i`/`-r` value is a prebuilt aligner database, a
//!   directory of structures (searched recursively), a glob, or structure files;
//!   [`resolve_structure_inputs`] turns the values into one database or one list of
//!   files. Paths are spelled the way Python's `pathlib`/`glob` spell them, because they
//!   reach the output: a record's DEFINITION is `structure=<path>`.
//! * **The backends.** [`Aligner`] drives the external `foldseek` or `reseek` binary
//!   as a subprocess (both are GPL, so they are never linked or bundled), declaring what
//!   each can do so an unsupported request fails with a clear message before any work.
//! * **Translation into annotations.** Hits become [`SearchResult`]s keyed by input
//!   record, and database entries become protein [`SeqRecord`]s, so the sequence tools'
//!   annotation writers are reused unchanged.
//!
//! Alignment direction is fixed: references are the QUERY and the database being
//! searched is the TARGET, so E-values are computed against the database, as they are
//! everywhere else. `structure_dist` runs the other way round (inputs as the query).
//!
//! Hits sorted by target are sorted here, stably on the target column's bytes, which is
//! what upstream's `LC_ALL=C sort -s` does. Coreutils `sort` is not on Windows, and the
//! sort spills to disk the same way, so memory does not grow with the hit table.

use std::collections::{BTreeMap, BinaryHeap, HashMap, HashSet};
use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::search::results::SearchResult;
use crate::seq::{Annotations, Location, SeqFeature, SeqRecord, Topology};
use crate::tools::external::{hide_console, ExternalProgram, TempDir, FOLDSEEK, RESEEK};

/// The foldseek executable's name, and the environment variable naming its path.
pub const FOLDSEEK_BIN_NAME: &str = "foldseek";
pub const FOLDSEEK_ENV_VAR: &str = "DOMAINATOREDUX_FOLDSEEK";
/// The reseek executable's name, and the environment variable naming its path.
pub const RESEEK_BIN_NAME: &str = "reseek";
pub const RESEEK_ENV_VAR: &str = "DOMAINATOREDUX_RESEEK";

/// Structure file extensions (upstream's `EXTENSION_TO_TYPE` entries of type
/// `structure`), longest first: the order [`strip_structure_extension`] tries them in.
pub const STRUCTURE_EXTENSIONS: [&str; 6] = ["mmcif", "pdb1", "bcif", "pdb", "ent", "cif"];

/// Compression suffixes a structure path may carry (upstream `COMPRESSION_EXTENSIONS`).
const COMPRESSION_EXTENSIONS: [&str; 3] = ["gz", "bgz", "bgzf"];

/// Optional per-hit metrics a backend may be asked for; the same-named optional fields
/// of [`SearchResult`].
pub const METRIC_CHOICES: [&str; 4] = ["tmscore", "lddt", "rmsd", "prob"];

/// Metrics that need C-alpha coordinates in the databases compared.
const COORDINATE_METRICS: [&str; 3] = ["tmscore", "lddt", "rmsd"];

/// foldseek `--alignment-type` values that need C-alpha coordinates.
const COORDINATE_ALIGNMENT_TYPES: [i64; 1] = [1];

/// reseek database magic numbers (`BCA_MAGIC`/`BCB_MAGIC` in reseek's `bcadata.h`). The
/// header is `uint32 magic; uint64 chain_count`.
const RESEEK_DB_MAGICS: [u32; 2] = [0x00BC_ABC2, 0x00BC_BBC2];

/// reseek silently drops chains shorter than this when building a database.
const RESEEK_MIN_CHAIN_LENGTH: usize = 32;

fn input_error(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn other_error(message: impl Into<String>) -> io::Error {
    io::Error::other(message.into())
}

// ---------------------------------------------------------------- Python path rules

/// `PurePath(path).name`: the last component, trailing separators ignored.
pub(crate) fn py_name(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    let name = trimmed.rsplit('/').next().unwrap_or("");
    if name == "." {
        ""
    } else {
        name
    }
}

/// `PurePath(name).suffix` of a bare name: from the last dot, unless the dot is the
/// first or last character.
fn py_suffix(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) if i > 0 && i < name.len() - 1 => &name[i..],
        _ => "",
    }
}

/// `PurePath(name).stem` of a bare name.
fn py_stem(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) if i > 0 && i < name.len() - 1 => &name[..i],
        _ => name,
    }
}

/// `PurePath(path).suffixes`, lower-cased and without their dots.
fn py_suffixes(path: &str) -> Vec<String> {
    let name = py_name(path);
    if name.ends_with('.') {
        return Vec::new();
    }
    name.trim_start_matches('.')
        .split('.')
        .skip(1)
        .map(str::to_ascii_lowercase)
        .collect()
}

fn is_compression_suffix(suffix: &str) -> bool {
    COMPRESSION_EXTENSIONS.contains(&suffix.to_ascii_lowercase().as_str())
}

/// Whether `path` looks like a structure file, ignoring one compression suffix
/// (upstream `is_structure_path`, i.e. `get_file_type(path) == "structure"`).
pub fn is_structure_path(path: &str) -> bool {
    let mut suffixes = py_suffixes(path);
    if suffixes.last().is_some_and(|s| is_compression_suffix(s)) {
        suffixes.pop();
    }
    suffixes
        .last()
        .is_some_and(|s| STRUCTURE_EXTENSIONS.contains(&s.as_str()))
}

/// `str(Path(value))`: repeated separators and `.` components dropped, no trailing
/// separator, `.` for an empty path.
fn py_path_str(value: &str) -> String {
    let absolute = value.starts_with('/');
    let parts: Vec<&str> = value
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    let joined = parts.join("/");
    match (absolute, joined.is_empty()) {
        (true, _) => format!("/{joined}"),
        (false, true) => ".".to_string(),
        (false, false) => joined,
    }
}

/// `os.path.abspath(path)`: joined to the working directory and normalized, `..`
/// included, without resolving symlinks.
fn py_abspath(path: &str) -> String {
    let joined = if path.starts_with('/') {
        path.to_string()
    } else {
        let cwd = std::env::current_dir()
            .map(|d| d.to_string_lossy().into_owned())
            .unwrap_or_default();
        format!("{cwd}/{path}")
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in joined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    format!("/{}", parts.join("/"))
}

// ---------------------------------------------------------------- input resolution

/// Whether `value` is the prefix of an existing foldseek/mmseqs-style database: both
/// `<value>.dbtype` and `<value>.index` exist.
pub fn is_aligner_db(value: &str) -> bool {
    Path::new(&format!("{value}.dbtype")).is_file()
        && Path::new(&format!("{value}.index")).is_file()
}

/// `(magic, chain_count)` from a reseek database file, or `None` if it is not one.
fn read_reseek_db_header(path: &str) -> Option<(u32, u64)> {
    let mut header = [0u8; 12];
    File::open(path).ok()?.read_exact(&mut header).ok()?;
    let magic = u32::from_le_bytes(header[0..4].try_into().ok()?);
    RESEEK_DB_MAGICS.contains(&magic).then(|| {
        (
            magic,
            u64::from_le_bytes(header[4..12].try_into().unwrap_or_default()),
        )
    })
}

/// Whether `value` is a reseek `.bcb`/`.bca` database, told by its magic number.
pub fn is_reseek_db(value: &str) -> bool {
    Path::new(value).is_file() && read_reseek_db_header(value).is_some()
}

fn reseek_db_size(path: &str) -> io::Result<u64> {
    read_reseek_db_header(path)
        .map(|(_, count)| count)
        .ok_or_else(|| other_error(format!("'{path}' is not a readable reseek database.")))
}

/// One resolved `-i`/`-r` argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    /// A prebuilt database, by prefix (foldseek) or file (reseek).
    Db(String),
    /// Structure files, in order.
    Structures(Vec<String>),
}

fn recognized_extensions() -> String {
    let mut extensions = STRUCTURE_EXTENSIONS.to_vec();
    extensions.sort_unstable();
    extensions.join(", ")
}

/// Every file under `dir`, as `rglob("*")` spells them: `base/relative/path`.
fn walk_files(dir: &Path, base: &str, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let shown = if base == "." {
            name.clone()
        } else {
            format!("{base}/{name}")
        };
        let path = entry.path();
        // pathlib's rglob does not descend into symlinked directories.
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir {
            walk_files(&path, &shown, out);
        } else if path.is_file() {
            out.push(shown);
        }
    }
}

/// Interpret one `-i`/`-r` value (upstream `resolve_structure_input`): a database
/// prefix, a directory (searched recursively), an existing structure file, or a glob.
pub fn resolve_structure_input(value: &str) -> io::Result<Resolved> {
    if is_aligner_db(value) || is_reseek_db(value) {
        return Ok(Resolved::Db(value.to_string()));
    }
    let path = Path::new(value);
    if path.is_dir() {
        let mut found = Vec::new();
        walk_files(path, &py_path_str(value), &mut found);
        found.retain(|p| is_structure_path(p));
        found.sort();
        if found.is_empty() {
            return Err(input_error(format!(
                "No structure files found under directory '{value}'. Recognized extensions: {} \
                 (optionally .gz).",
                recognized_extensions()
            )));
        }
        return Ok(Resolved::Structures(found));
    }
    if path.is_file() {
        if !is_structure_path(value) {
            return Err(input_error(format!(
                "'{value}' is not a recognized structure file. Recognized extensions: {} \
                 (optionally .gz).",
                recognized_extensions()
            )));
        }
        return Ok(Resolved::Structures(vec![value.to_string()]));
    }
    if value.contains(['*', '?', '[']) {
        // Python's glob: `*` does not match a leading dot, `**` spans directories.
        let options = glob::MatchOptions {
            case_sensitive: true,
            require_literal_separator: true,
            require_literal_leading_dot: true,
        };
        let mut found: Vec<String> = glob::glob_with(value, options)
            .map_err(|e| input_error(format!("Glob '{value}' is not valid: {e}")))?
            .flatten()
            .filter(|p| p.is_file())
            .map(|p| p.to_string_lossy().into_owned())
            .filter(|p| is_structure_path(p))
            .collect();
        found.sort();
        if found.is_empty() {
            return Err(input_error(format!(
                "Glob '{value}' matched no structure files."
            )));
        }
        return Ok(Resolved::Structures(found));
    }
    Err(input_error(format!(
        "Could not interpret '{value}'. It is not an aligner database prefix ('{value}.dbtype' \
         and '{value}.index' do not both exist), not a directory, not an existing structure \
         file, and not a glob that matched anything."
    )))
}

/// Resolve several `-i`/`-r` values into one database or one list of structures
/// (upstream `resolve_structure_inputs`). A database mixed with structure files, or two
/// databases, is refused; duplicate files are dropped, first occurrence kept.
pub fn resolve_structure_inputs(values: &[String]) -> io::Result<Resolved> {
    let mut structures: Vec<String> = Vec::new();
    let mut dbs: Vec<String> = Vec::new();
    for value in values {
        match resolve_structure_input(value)? {
            Resolved::Db(prefix) => dbs.push(prefix),
            Resolved::Structures(files) => structures.extend(files),
        }
    }
    if !dbs.is_empty() && !structures.is_empty() {
        return Err(input_error(format!(
            "Cannot mix a prebuilt aligner database with structure files in the same \
             argument. Got database(s) {} and {} structure file(s).",
            py_list(&dbs),
            structures.len()
        )));
    }
    if dbs.len() > 1 {
        return Err(input_error(format!(
            "Only one aligner database can be given per argument, got {}.",
            py_list(&dbs)
        )));
    }
    if let Some(db) = dbs.pop() {
        return Ok(Resolved::Db(db));
    }
    let mut seen = HashSet::new();
    structures.retain(|s| seen.insert(s.clone()));
    Ok(Resolved::Structures(structures))
}

/// A list of strings the way Python prints one: `['a', 'b']`.
fn py_list(items: &[String]) -> String {
    let quoted: Vec<String> = items.iter().map(|s| format!("'{s}'")).collect();
    format!("[{}]", quoted.join(", "))
}

/// The `/database` label for a database prefix or structure file (upstream
/// `database_label`): its name, less a compression suffix and then one structure
/// suffix, so `ref.pdb.gz` is `ref` but the prefix `refs.v1` stays `refs.v1`.
pub fn database_label(prefix: &str) -> String {
    let mut name = py_name(prefix).to_string();
    for _ in 0..2 {
        let suffix = py_suffix(&name);
        if is_structure_path(&name) || (!suffix.is_empty() && is_compression_suffix(&suffix[1..])) {
            name = py_stem(&name).to_string();
        }
    }
    name
}

/// A human-meaningful name for a resolved `-i`/`-r` argument (upstream `input_label`),
/// used for the `/database` qualifier.
pub fn input_label(values: &[String], resolved: &Resolved) -> String {
    let files = match resolved {
        Resolved::Db(prefix) => return database_label(prefix),
        Resolved::Structures(files) => files,
    };
    if values.len() == 1 && Path::new(&values[0]).is_dir() {
        return non_empty_or_references(py_name(&values[0]));
    }
    if files.len() == 1 {
        return database_label(&files[0]);
    }
    let parents: HashSet<String> = files.iter().map(|p| py_parent(p)).collect();
    if parents.len() == 1 {
        let parent = parents.into_iter().next().unwrap_or_default();
        return non_empty_or_references(py_name(&parent));
    }
    "references".to_string()
}

fn non_empty_or_references(name: &str) -> String {
    if name.is_empty() {
        "references".to_string()
    } else {
        name.to_string()
    }
}

/// `str(Path(path).parent)`.
fn py_parent(path: &str) -> String {
    let normalized = py_path_str(path);
    match normalized.rfind('/') {
        Some(0) => "/".to_string(),
        Some(i) => normalized[..i].to_string(),
        None => ".".to_string(),
    }
}

/// `1abc.pdb_A` -> `1abc_A`, `1abc.pdb` -> `1abc` (upstream
/// `strip_structure_extension`). foldseek names entries `<file basename>_<chain>`, so a
/// file extension can land in the middle of a reference's name.
pub fn strip_structure_extension(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    for extension in STRUCTURE_EXTENSIONS {
        let needle = format!(".{extension}_");
        if let Some(index) = lower.rfind(&needle) {
            return format!("{}_{}", &name[..index], &name[index + needle.len()..]);
        }
        let suffix = format!(".{extension}");
        if lower.ends_with(&suffix) {
            return name[..name.len() - suffix.len()].to_string();
        }
    }
    name.to_string()
}

// ---------------------------------------------------------------- hits and databases

/// One alignment, backend-neutral. `query` is a reference and `target` an input record,
/// except in `structure_dist`, which aligns the other way round.
#[derive(Debug, Clone, Default)]
pub struct StructureHit {
    pub query: String,
    pub target: String,
    pub qstart: i64,
    pub qend: i64,
    pub qlen: i64,
    pub tstart: i64,
    pub tend: i64,
    pub tlen: i64,
    pub evalue: f64,
    pub bits: f64,
    pub fident: f64,
    pub alnlen: i64,
    pub tseq: String,
    pub qheader: String,
    pub tmscore: Option<f64>,
    pub lddt: Option<f64>,
    pub rmsd: Option<f64>,
    pub prob: Option<f64>,
}

impl StructureHit {
    /// The value of a metric by name: `tmscore`, `lddt`, `rmsd`, `prob`, `fident` or
    /// `bits`.
    pub fn field(&self, name: &str) -> Option<f64> {
        match name {
            "tmscore" => self.tmscore,
            "lddt" => self.lddt,
            "rmsd" => self.rmsd,
            "prob" => self.prob,
            "fident" => Some(self.fident),
            _ => Some(self.bits),
        }
    }
}

/// A structural database on disk: its prefix (a foldseek prefix or a reseek file), and
/// the input paths it was built from, empty for a prebuilt one (used to recover each
/// entry's source file).
#[derive(Debug, Clone, Default)]
pub struct StructureDb {
    pub prefix: String,
    pub source_paths: Vec<String>,
}

impl StructureDb {
    pub fn prebuilt(prefix: &str) -> StructureDb {
        StructureDb {
            prefix: prefix.to_string(),
            source_paths: Vec::new(),
        }
    }

    /// Whether a C-alpha sub-database is present. Databases built from amino acid + 3Di
    /// FASTA have none, so they cannot do TM-alignment or coordinate metrics.
    fn has_coordinates(&self) -> bool {
        Path::new(&format!("{}_ca", self.prefix)).is_file()
    }
}

/// A stream of hits from a backend's result table.
pub type Hits = Box<dyn Iterator<Item = io::Result<StructureHit>>>;

/// Lines of a result table, skipping empty ones, parsed one at a time.
fn hits_from_lines(
    lines: impl Iterator<Item = io::Result<String>> + 'static,
    parse: impl Fn(&str) -> io::Result<StructureHit> + 'static,
) -> Hits {
    Box::new(lines.filter_map(move |line| match line {
        Ok(line) if line.is_empty() => None,
        Ok(line) => Some(parse(&line)),
        Err(e) => Some(Err(e)),
    }))
}

fn parse_int(value: &str, line: &str) -> io::Result<i64> {
    value.trim().parse().map_err(|_| {
        other_error(format!(
            "Unexpected aligner output: '{value}' is not an integer in line: {}",
            head(line)
        ))
    })
}

fn parse_float(value: &str, line: &str) -> io::Result<f64> {
    value.trim().parse().map_err(|_| {
        other_error(format!(
            "Unexpected aligner output: '{value}' is not a number in line: {}",
            head(line)
        ))
    })
}

/// The first 200 characters of a line, for an error message.
fn head(line: &str) -> &str {
    match line.char_indices().nth(200) {
        Some((i, _)) => &line[..i],
        None => line,
    }
}

/// `-log10(value)`, with a finite ceiling so an underflowed p-value stays sortable.
fn neg_log10(value: f64) -> f64 {
    if value <= 0.0 {
        999.0
    } else {
        -value.log10()
    }
}

// ---------------------------------------------------------------- running programs

/// Run an external command (upstream `external.run_command`). A nonzero exit is not a
/// failure when `expect` names a file that was produced: foldseek's workflows exit
/// nonzero during cleanup even when the run succeeded. On failure, the error carries the
/// last 25 lines of the program's error output.
fn run_command(
    argv: &[String],
    description: &str,
    expect: Option<&str>,
    env: &[(&str, String)],
) -> io::Result<std::process::Output> {
    let mut command = Command::new(&argv[0]);
    command.args(&argv[1..]);
    for (key, value) in env {
        command.env(key, value);
    }
    hide_console(&mut command);
    crate::log::debug(format!("running: {}", argv.join(" ")));
    let output = command.output().map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            other_error(format!(
                "{description}: could not run '{}'; it is not an executable file.",
                argv[0]
            ))
        } else {
            e
        }
    })?;
    if output.status.success() || expect.is_some_and(|p| Path::new(p).exists()) {
        return Ok(output);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let text = if stderr.trim().is_empty() {
        stdout.trim()
    } else {
        stderr.trim()
    };
    let lines: Vec<&str> = text.split('\n').collect();
    let tail = lines[lines.len().saturating_sub(25)..].join("\n");
    let code = output
        .status
        .code()
        .map_or_else(|| "a signal".to_string(), |c| c.to_string());
    Err(other_error(format!(
        "{description} failed with exit code {code}.\nCommand: {}\n{tail}",
        argv.join(" ")
    )))
}

/// `argv` followed by the user's extra arguments, refusing an option `argv` already
/// sets (upstream `append_extra_args`).
fn append_extra_args(
    mut argv: Vec<String>,
    extra: &[String],
    option: &str,
) -> io::Result<Vec<String>> {
    let set: HashSet<&str> = argv
        .iter()
        .filter(|t| t.starts_with('-'))
        .map(String::as_str)
        .collect();
    let mut conflicts: Vec<&str> = extra
        .iter()
        .map(String::as_str)
        .filter(|t| t.starts_with('-') && set.contains(t))
        .collect();
    conflicts.sort_unstable();
    conflicts.dedup();
    if !conflicts.is_empty() {
        return Err(input_error(format!(
            "{option} cannot set {}: Domainator sets {} itself.",
            conflicts.join(", "),
            if conflicts.len() == 1 { "it" } else { "them" }
        )));
    }
    argv.extend(extra.iter().cloned());
    Ok(argv)
}

/// Sort a TSV file stably on one 0-based column, comparing that field's bytes, as
/// `LC_ALL=C sort -s -t '\t' -kN,N` does. Lines are sorted in memory a chunk at a time
/// and the sorted runs merged from disk, so a hit table larger than memory still sorts.
pub(crate) fn sort_by_column(
    path: &Path,
    column: usize,
    sorted: &Path,
    work_dir: &Path,
) -> io::Result<()> {
    sort_by_column_in_chunks(path, column, sorted, work_dir, 256 << 20)
}

/// [`sort_by_column`], sorting `chunk_bytes` of lines at a time in memory.
fn sort_by_column_in_chunks(
    path: &Path,
    column: usize,
    sorted: &Path,
    work_dir: &Path,
    chunk_bytes_max: usize,
) -> io::Result<()> {
    fn key(line: &[u8], column: usize) -> &[u8] {
        line.split(|&b| b == b'\t').nth(column).unwrap_or(b"")
    }
    let mut reader = BufReader::new(File::open(path)?);
    let mut runs: Vec<PathBuf> = Vec::new();
    let mut chunk: Vec<Vec<u8>> = Vec::new();
    let mut chunk_bytes = 0usize;
    let mut finished = false;
    let mut single: Option<Vec<Vec<u8>>> = None;
    while !finished {
        let mut line = Vec::new();
        if reader.read_until(b'\n', &mut line)? == 0 {
            finished = true;
        } else {
            if line.last() == Some(&b'\n') {
                line.pop();
            }
            chunk_bytes += line.len() + 1;
            chunk.push(line);
        }
        if (finished || chunk_bytes >= chunk_bytes_max) && !chunk.is_empty() {
            chunk.sort_by(|a, b| key(a, column).cmp(key(b, column)));
            if finished && runs.is_empty() {
                single = Some(std::mem::take(&mut chunk));
                break;
            }
            let run = work_dir.join(format!("sort_run_{}.tsv", runs.len()));
            let mut w = BufWriter::new(File::create(&run)?);
            for line in chunk.drain(..) {
                w.write_all(&line)?;
                w.write_all(b"\n")?;
            }
            w.flush()?;
            runs.push(run);
            chunk_bytes = 0;
        }
    }
    let mut out = BufWriter::new(File::create(sorted)?);
    if let Some(lines) = single {
        for line in lines {
            out.write_all(&line)?;
            out.write_all(b"\n")?;
        }
        return out.flush();
    }
    // k-way merge; ties go to the earlier run, which holds the earlier lines.
    let mut readers: Vec<BufReader<File>> = runs
        .iter()
        .map(|r| File::open(r).map(BufReader::new))
        .collect::<io::Result<_>>()?;
    let mut heap = BinaryHeap::new();
    let next = |reader: &mut BufReader<File>| -> io::Result<Option<Vec<u8>>> {
        let mut line = Vec::new();
        if reader.read_until(b'\n', &mut line)? == 0 {
            return Ok(None);
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        Ok(Some(line))
    };
    for (i, reader) in readers.iter_mut().enumerate() {
        if let Some(line) = next(reader)? {
            heap.push(std::cmp::Reverse((key(&line, column).to_vec(), i, line)));
        }
    }
    while let Some(std::cmp::Reverse((_, i, line))) = heap.pop() {
        out.write_all(&line)?;
        out.write_all(b"\n")?;
        if let Some(line) = next(&mut readers[i])? {
            heap.push(std::cmp::Reverse((key(&line, column).to_vec(), i, line)));
        }
    }
    for run in &runs {
        let _ = std::fs::remove_file(run);
    }
    out.flush()
}

/// The executable for `program` (upstream `resolve_binary`): the user's explicit path,
/// which must be an executable file or a name on PATH, or else the program found the
/// usual way (environment variable, bundled copy, PATH).
fn resolve_binary(
    program: &ExternalProgram,
    path: Option<&str>,
    hint: &str,
) -> io::Result<PathBuf> {
    if let Some(path) = path {
        if is_executable_file(Path::new(path)) || on_path(path).is_some() {
            return Ok(PathBuf::from(path));
        }
        return Err(other_error(format!("'{path}' is not an executable file.")));
    }
    program.locate().ok_or_else(|| {
        other_error(format!(
            "Could not find '{name}' on PATH. {hint} Alternatively pass an explicit path with \
             --{name}_path.",
            name = program.name
        ))
    })
}

/// A file the user may run: on Unix, one with an execute bit set.
fn is_executable_file(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

fn on_path(name: &str) -> Option<PathBuf> {
    if name.contains('/') {
        return None;
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

fn py_float(x: f64) -> String {
    crate::matrix::text::py_float_str(x)
}

// ---------------------------------------------------------------- the backends

/// A structural aligner (upstream `ALIGNERS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Foldseek,
    Reseek,
}

impl Backend {
    pub fn name(self) -> &'static str {
        match self {
            Backend::Foldseek => "foldseek",
            Backend::Reseek => "reseek",
        }
    }
}

impl std::str::FromStr for Backend {
    type Err = String;
    fn from_str(s: &str) -> Result<Backend, String> {
        match s {
            "foldseek" => Ok(Backend::Foldseek),
            "reseek" => Ok(Backend::Reseek),
            other => Err(format!("Unknown structure alignment backend: {other}")),
        }
    }
}

/// The backend options every structure tool shares (upstream `add_backend_arguments`).
#[derive(Debug, Clone)]
pub struct BackendOptions {
    pub algorithm: Backend,
    pub foldseek_path: Option<String>,
    pub reseek_path: Option<String>,
    pub foldseek_args: Option<String>,
    pub reseek_args: Option<String>,
    /// Threads for the backend; 0 lets it decide.
    pub cpu: i64,
    /// `cpu`, or a CUDA device (`cuda`, `cuda:1`); foldseek only.
    pub device: Option<String>,
    pub tmp_dir: Option<String>,
    pub reseek_stats: String,
    pub reseek_sensitivity: String,
}

impl Default for BackendOptions {
    fn default() -> Self {
        BackendOptions {
            algorithm: Backend::Foldseek,
            foldseek_path: None,
            reseek_path: None,
            foldseek_args: None,
            reseek_args: None,
            cpu: 0,
            device: None,
            tmp_dir: None,
            reseek_stats: "superfamily".to_string(),
            reseek_sensitivity: "sensitive".to_string(),
        }
    }
}

/// One search request (upstream `StructureAligner.search`'s keywords).
#[derive(Debug, Clone, Default)]
pub struct SearchRequest<'a> {
    pub evalue: f64,
    pub max_seqs: Option<usize>,
    pub want_tseq: bool,
    pub alignment_type: Option<i64>,
    pub metrics: &'a [String],
    /// Yield hits grouped by target, so a caller can stream one record at a time.
    pub sort_by_target: bool,
}

/// A configured backend with its executable resolved (upstream `StructureAligner`).
#[derive(Debug, Clone)]
pub struct Aligner {
    pub backend: Backend,
    bin: PathBuf,
    cpu: i64,
    device: Option<String>,
    extra_args: Vec<String>,
    pub tmp_dir: Option<String>,
    reseek_stats: String,
    reseek_sensitivity: String,
}

/// foldseek `convertalis` columns read for every hit.
const FOLDSEEK_BASE_COLUMNS: [&str; 13] = [
    "query", "target", "qheader", "fident", "alnlen", "qstart", "qend", "qlen", "tstart", "tend",
    "tlen", "evalue", "bits",
];

/// foldseek's `convertalis` column for each metric.
fn foldseek_metric_column(metric: &str) -> &'static str {
    match metric {
        "tmscore" => "alntmscore",
        "lddt" => "lddt",
        "rmsd" => "rmsd",
        _ => "prob",
    }
}

/// reseek's `-columns`.
const RESEEK_COLUMNS: [&str; 10] = [
    "query", "target", "qlo", "qhi", "ql", "tlo", "thi", "tl", "pctid", "pvalue",
];

impl Aligner {
    /// Instantiate the selected backend (upstream `build_aligner`), resolving its
    /// executable before any work is done.
    pub fn new(opts: &BackendOptions) -> io::Result<Aligner> {
        let algorithm = opts.algorithm;
        for (backend, args) in [
            (Backend::Foldseek, &opts.foldseek_args),
            (Backend::Reseek, &opts.reseek_args),
        ] {
            if backend != algorithm && args.as_deref().is_some_and(|a| !a.is_empty()) {
                return Err(input_error(format!(
                    "--{}_args was given, but the search backend is {} (--algorithm).",
                    backend.name(),
                    algorithm.name()
                )));
            }
        }
        let (path, extra) = match algorithm {
            Backend::Foldseek => (&opts.foldseek_path, &opts.foldseek_args),
            Backend::Reseek => (&opts.reseek_path, &opts.reseek_args),
        };
        let extra_args = match extra.as_deref().filter(|a| !a.is_empty()) {
            Some(text) => shlex::split(text).ok_or_else(|| {
                input_error(format!(
                    "--{}_args could not be split: {text}",
                    algorithm.name()
                ))
            })?,
            None => Vec::new(),
        };
        let bin = match algorithm {
            Backend::Foldseek => resolve_binary(
                &FOLDSEEK,
                path.as_deref(),
                "Install it with 'conda install -c conda-forge -c bioconda foldseek'.",
            )?,
            Backend::Reseek => resolve_binary(
                &RESEEK,
                path.as_deref(),
                "Build it from https://github.com/rcedgar/reseek (the v3.01 release binary \
                 requires AVX-512).",
            )?,
        };
        Ok(Aligner {
            backend: algorithm,
            bin,
            cpu: opts.cpu,
            device: opts.device.clone(),
            extra_args,
            tmp_dir: opts.tmp_dir.clone(),
            reseek_stats: opts.reseek_stats.clone(),
            reseek_sensitivity: opts.reseek_sensitivity.clone(),
        })
    }

    pub fn name(&self) -> &'static str {
        self.backend.name()
    }

    fn supports_alignment_types(&self) -> &'static [i64] {
        match self.backend {
            Backend::Foldseek => &[0, 1, 2],
            Backend::Reseek => &[],
        }
    }

    /// The metrics this backend can compute.
    pub fn supports_metrics(&self) -> &'static [&'static str] {
        match self.backend {
            Backend::Foldseek => &METRIC_CHOICES,
            Backend::Reseek => &[],
        }
    }

    /// Whether the backend exposes a per-residue 3Di alphabet.
    pub fn supports_3di(&self) -> bool {
        self.backend == Backend::Foldseek
    }

    /// Whether search output carries each target's full sequence.
    pub fn provides_target_sequence(&self) -> bool {
        self.backend == Backend::Foldseek
    }

    fn default_alignment_type(&self) -> Option<i64> {
        match self.backend {
            Backend::Foldseek => Some(2),
            Backend::Reseek => None,
        }
    }

    /// The raw hit tables this backend leaves in the work directory, most processed
    /// first.
    fn results_filenames(&self) -> &'static [&'static str] {
        match self.backend {
            Backend::Foldseek => &["results.by_target.tsv", "results.tsv"],
            Backend::Reseek => &["reseek_hits.by_target.tsv", "reseek_hits.tsv"],
        }
    }

    fn resolve_alignment_type(&self, alignment_type: Option<i64>) -> Option<i64> {
        alignment_type.or(self.default_alignment_type())
    }

    /// The per-reference hit cap in force: the request, or the backend's default (1000
    /// for foldseek, none for reseek).
    pub fn effective_max_seqs(&self, max_seqs: Option<usize>) -> Option<usize> {
        match (max_seqs, self.backend) {
            (Some(n), _) => Some(n),
            (None, Backend::Foldseek) => Some(1000),
            (None, Backend::Reseek) => None,
        }
    }

    fn database_has_coordinates(&self, db: &StructureDb) -> bool {
        match self.backend {
            Backend::Foldseek => db.has_coordinates(),
            // A reseek database is C-alpha coordinates by construction.
            Backend::Reseek => true,
        }
    }

    /// Refuse a database this backend cannot read.
    pub fn validate_database(&self, db: &StructureDb) -> io::Result<()> {
        let prefix = &db.prefix;
        match self.backend {
            Backend::Foldseek => {
                if is_reseek_db(prefix) {
                    return Err(input_error(format!(
                        "'{prefix}' is a reseek database, which foldseek cannot read. Use \
                         --algorithm reseek, or point --input/--reference at structure files \
                         or a foldseek database."
                    )));
                }
                if !is_aligner_db(prefix) {
                    return Err(input_error(format!(
                        "'{prefix}' is not a foldseek database ('{prefix}.dbtype' and \
                         '{prefix}.index' do not both exist)."
                    )));
                }
            }
            Backend::Reseek => {
                if !is_reseek_db(prefix) {
                    let hint = if is_aligner_db(prefix) {
                        " It looks like a foldseek database; use --algorithm foldseek, or \
                         rebuild it for reseek from the original structures."
                    } else {
                        ""
                    };
                    return Err(input_error(format!(
                        "'{prefix}' is not a reseek database (.bcb/.bca).{hint}"
                    )));
                }
            }
        }
        Ok(())
    }

    /// Fail early on an unsupported or impossible request (upstream
    /// `check_capabilities`).
    pub fn check_capabilities(
        &self,
        alignment_type: Option<i64>,
        metrics: &[String],
        databases: &[&StructureDb],
    ) -> io::Result<()> {
        if let Some(t) = alignment_type {
            let supported = self.supports_alignment_types();
            if !supported.contains(&t) {
                let detail = if supported.is_empty() {
                    "It has no alignment-type option at all.".to_string()
                } else {
                    format!("Supported: {supported:?}.")
                };
                return Err(input_error(format!(
                    "Backend '{}' does not support alignment type {t}. {detail}",
                    self.name()
                )));
            }
        }
        let alignment_type = self.resolve_alignment_type(alignment_type);
        let mut unsupported: Vec<&str> = metrics
            .iter()
            .map(String::as_str)
            .filter(|m| !self.supports_metrics().contains(m))
            .collect();
        unsupported.sort_unstable();
        unsupported.dedup();
        if !unsupported.is_empty() {
            let mut supported = self.supports_metrics().to_vec();
            supported.sort_unstable();
            return Err(input_error(format!(
                "Backend '{}' cannot compute {}. Supported metrics: {}.",
                self.name(),
                py_str_list(&unsupported),
                py_str_list(&supported)
            )));
        }
        let mut needs: Vec<String> = metrics
            .iter()
            .filter(|m| COORDINATE_METRICS.contains(&m.as_str()))
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        if let Some(t) = alignment_type.filter(|t| COORDINATE_ALIGNMENT_TYPES.contains(t)) {
            needs.push(format!("alignment type {t}"));
        }
        if needs.is_empty() {
            return Ok(());
        }
        for db in databases {
            if !self.database_has_coordinates(db) {
                let verb = if needs.len() == 1 {
                    "requires"
                } else {
                    "require"
                };
                return Err(input_error(format!(
                    "{} {verb} C-alpha coordinates, but the database '{}' has no '{}_ca' \
                     file. Databases built from amino acid and 3Di sequences (rather than \
                     from structure files) carry no coordinates. Rebuild it from the \
                     original structures, or drop the coordinate-dependent options.",
                    needs.join(", "),
                    db.prefix,
                    py_name(&db.prefix)
                )));
            }
        }
        Ok(())
    }

    fn thread_args(&self) -> Vec<String> {
        if self.cpu <= 0 {
            return Vec::new();
        }
        let flag = match self.backend {
            Backend::Foldseek => "--threads",
            Backend::Reseek => "-threads",
        };
        vec![flag.to_string(), self.cpu.to_string()]
    }

    fn bin_str(&self) -> String {
        self.bin.to_string_lossy().into_owned()
    }

    /// Build a database from structure files (foldseek `createdb` driven by a list file,
    /// or reseek `-convert`).
    pub fn build_db(&self, inputs: &[String], out_prefix: &str) -> io::Result<StructureDb> {
        if inputs.is_empty() {
            return Err(input_error("No structure files to build a database from."));
        }
        match self.backend {
            Backend::Foldseek => {
                // A list file has no ARG_MAX limit, sidesteps "only one directory can be
                // given", and fixes the file numbering `.lookup` refers to.
                let list_path = format!("{out_prefix}.inputs.tsv");
                write_path_list(&list_path, inputs)?;
                let mut argv = vec![
                    self.bin_str(),
                    "createdb".into(),
                    list_path,
                    out_prefix.into(),
                    "--chain-name-mode".into(),
                    "1".into(),
                    "--write-lookup".into(),
                    "1".into(),
                    "-v".into(),
                    "1".into(),
                ];
                argv.extend(self.thread_args());
                let dbtype = format!("{out_prefix}.dbtype");
                run_command(&argv, "foldseek createdb", Some(&dbtype), &[])?;
                if !Path::new(&dbtype).is_file() {
                    return Err(other_error(format!(
                        "foldseek createdb produced no database at '{out_prefix}'. Check that \
                         the input files are readable structures."
                    )));
                }
                Ok(StructureDb {
                    prefix: out_prefix.to_string(),
                    source_paths: inputs.to_vec(),
                })
            }
            Backend::Reseek => {
                let db_path = if out_prefix.ends_with(".bcb") {
                    out_prefix.to_string()
                } else {
                    format!("{out_prefix}.bcb")
                };
                let list_path = format!("{db_path}.files");
                write_path_list(&list_path, inputs)?;
                // One thread: with several, reseek stores chains in completion order,
                // so record (and matrix row) order would change from run to run.
                let argv = vec![
                    self.bin_str(),
                    "-convert".into(),
                    list_path,
                    "-bcb".into(),
                    db_path.clone(),
                    "-threads".into(),
                    "1".into(),
                ];
                let output = run_command(&argv, "reseek -convert", Some(&db_path), &[])?;
                if !Path::new(&db_path).is_file() {
                    return Err(other_error(format!(
                        "reseek -convert produced no database at '{db_path}'. Check that the \
                         input files are readable structures."
                    )));
                }
                warn_on_dropped_chains(&output, &db_path);
                Ok(StructureDb {
                    prefix: db_path,
                    source_paths: inputs.to_vec(),
                })
            }
        }
    }

    /// Align every entry of `query_db` against `target_db`, in `work_dir`.
    pub fn search(
        &self,
        query_db: &StructureDb,
        target_db: &StructureDb,
        request: &SearchRequest<'_>,
        work_dir: &Path,
    ) -> io::Result<Hits> {
        match self.backend {
            Backend::Foldseek => self.foldseek_search(query_db, target_db, request, work_dir),
            Backend::Reseek => self.reseek_search(query_db, target_db, request, work_dir),
        }
    }

    fn foldseek_search(
        &self,
        query_db: &StructureDb,
        target_db: &StructureDb,
        request: &SearchRequest<'_>,
        work_dir: &Path,
    ) -> io::Result<Hits> {
        let alignment_type = self
            .resolve_alignment_type(request.alignment_type)
            .unwrap_or(2);
        let max_seqs = self.effective_max_seqs(request.max_seqs).unwrap_or(1000);
        let mut columns: Vec<String> = FOLDSEEK_BASE_COLUMNS
            .iter()
            .map(|c| c.to_string())
            .collect();
        if request.want_tseq {
            columns.push("tseq".into());
        }
        let metric_columns: Vec<String> = request
            .metrics
            .iter()
            .map(|m| foldseek_metric_column(m).to_string())
            .collect();
        columns.extend(metric_columns.iter().cloned());

        let path = |name: &str| work_dir.join(name).to_string_lossy().into_owned();
        let aln_path = path("aln");
        let foldseek_tmp = path("foldseek_tmp");
        let results_path = path("results.tsv");
        std::fs::create_dir_all(&foldseek_tmp)?;

        // foldseek selects a GPU through CUDA_VISIBLE_DEVICES, not a flag.
        let mut gpu_args: Vec<String> = Vec::new();
        let mut env: Vec<(&str, String)> = Vec::new();
        if let Some(device) = self.device.as_deref().filter(|d| *d != "cpu") {
            gpu_args = vec!["--gpu".into(), "1".into()];
            if let Some((_, index)) = device.split_once(':') {
                env.push(("CUDA_VISIBLE_DEVICES", index.to_string()));
            }
        }
        let mut argv = vec![
            self.bin_str(),
            "search".into(),
            query_db.prefix.clone(),
            target_db.prefix.clone(),
            aln_path.clone(),
            foldseek_tmp,
            "-e".into(),
            py_float(request.evalue),
            "--max-seqs".into(),
            max_seqs.to_string(),
            "--alignment-type".into(),
            alignment_type.to_string(),
        ];
        if !metric_columns.is_empty() {
            // lddt/rmsd/alntmscore come from the alignment backtrace, which foldseek
            // search only stores with -a.
            argv.push("-a".into());
        }
        argv.extend(self.thread_args());
        argv.extend(gpu_args);
        let argv = append_extra_args(argv, &self.extra_args, "--foldseek_args")?;
        run_command(
            &argv,
            "foldseek search",
            Some(&format!("{aln_path}.dbtype")),
            &env,
        )?;

        let mut convert = vec![
            self.bin_str(),
            "convertalis".into(),
            query_db.prefix.clone(),
            target_db.prefix.clone(),
            aln_path,
            results_path.clone(),
            "--format-mode".into(),
            "0".into(),
            "--format-output".into(),
            columns.join(","),
        ];
        convert.extend(self.thread_args());
        run_command(&convert, "foldseek convertalis", Some(&results_path), &[])?;

        let mut results = PathBuf::from(&results_path);
        if request.sort_by_target {
            let sorted = work_dir.join("results.by_target.tsv");
            let target = columns.iter().position(|c| c == "target").unwrap_or(1);
            sort_by_column(&results, target, &sorted, work_dir)?;
            results = sorted;
        }
        let lines = BufReader::new(File::open(&results)?).lines();
        Ok(hits_from_lines(lines, move |line| {
            parse_foldseek_line(line, &columns)
        }))
    }

    fn reseek_search(
        &self,
        query_db: &StructureDb,
        target_db: &StructureDb,
        request: &SearchRequest<'_>,
        work_dir: &Path,
    ) -> io::Result<Hits> {
        if !request.metrics.is_empty() {
            let mut metrics: Vec<&str> = request.metrics.iter().map(String::as_str).collect();
            metrics.sort_unstable();
            return Err(input_error(format!(
                "Backend 'reseek' cannot compute {}.",
                py_str_list(&metrics)
            )));
        }
        let max_seqs = self.effective_max_seqs(request.max_seqs);
        // reseek reports a per-comparison p-value; -e is an E-value against the
        // database searched, so for N entries the threshold converts as p = E / N.
        let db_size = reseek_db_size(&target_db.prefix)?.max(1);
        let pvalue = (request.evalue / db_size as f64).min(1.0);
        let results_path = work_dir.join("reseek_hits.tsv");
        let argv = vec![
            self.bin_str(),
            "-search".into(),
            query_db.prefix.clone(),
            "-db".into(),
            target_db.prefix.clone(),
            "-output".into(),
            results_path.to_string_lossy().into_owned(),
            format!("-{}", self.reseek_sensitivity),
            "-pvalue".into(),
            py_float(pvalue),
            "-stats".into(),
            self.reseek_stats.clone(),
            "-columns".into(),
            RESEEK_COLUMNS.join("+"),
        ];
        let mut argv = argv;
        argv.extend(self.thread_args());
        let argv = append_extra_args(argv, &self.extra_args, "--reseek_args")?;
        run_command(
            &argv,
            "reseek -search",
            Some(&results_path.to_string_lossy()),
            &[],
        )?;

        let db_size = db_size as f64;
        if let Some(max_seqs) = max_seqs.filter(|&n| n > 0) {
            // Capped by significance rather than file position: reseek's output is not
            // sorted, so its first lines are not its best.
            let mut lines = reseek_best_per_query(&results_path, max_seqs)?;
            if request.sort_by_target {
                lines.sort_by(|a, b| {
                    let key = |l: &str| l.split('\t').nth(1).unwrap_or("").to_string();
                    key(a).cmp(&key(b))
                });
            }
            return Ok(hits_from_lines(lines.into_iter().map(Ok), move |line| {
                parse_reseek_line(line, db_size)
            }));
        }
        let mut results = results_path;
        if request.sort_by_target {
            let sorted = work_dir.join("reseek_hits.by_target.tsv");
            sort_by_column(&results, 1, &sorted, work_dir)?;
            results = sorted;
        }
        let lines = BufReader::new(File::open(&results)?).lines();
        Ok(hits_from_lines(lines, move |line| {
            parse_reseek_line(line, db_size)
        }))
    }

    /// `(name, amino acid sequence, source file)` for every entry of `db`, in database
    /// order. reseek records no source files.
    pub fn iter_sequences(
        &self,
        db: &StructureDb,
    ) -> io::Result<Vec<(String, String, Option<String>)>> {
        match self.backend {
            Backend::Foldseek => {
                let sources = foldseek_lookup_sources(db)?;
                let names = foldseek_read_names(&db.prefix)?;
                Ok(read_flat_db(&db.prefix)?
                    .into_iter()
                    .map(|(key, sequence)| {
                        let name = names.get(&key).cloned().unwrap_or_else(|| key.to_string());
                        let source = sources.get(&name).cloned();
                        (name, sequence, source)
                    })
                    .collect())
            }
            Backend::Reseek => {
                let scratch = TempDir::new_in(
                    self.tmp_dir.as_deref().map(Path::new),
                    "domainatoredux_reseek",
                )?;
                let fasta_path = scratch
                    .path()
                    .join("sequences.fasta")
                    .to_string_lossy()
                    .into_owned();
                let argv = vec![
                    self.bin_str(),
                    "-convert".into(),
                    db.prefix.clone(),
                    "-fasta".into(),
                    fasta_path.clone(),
                ];
                run_command(&argv, "reseek -convert -fasta", Some(&fasta_path), &[])?;
                read_reseek_fasta(Path::new(&fasta_path))
            }
        }
    }

    /// `(name, 3Di sequence)` for every entry, from foldseek's `_ss` sub-database.
    pub fn iter_3di(&self, db: &StructureDb) -> io::Result<Vec<(String, String)>> {
        if self.backend != Backend::Foldseek {
            return Err(input_error(format!(
                "{} does not expose a 3Di alphabet.",
                self.name()
            )));
        }
        let ss = format!("{}_ss", db.prefix);
        if !Path::new(&ss).is_file() {
            return Err(other_error(format!(
                "Database '{}' has no 3Di sub-database ('{}_ss').",
                db.prefix,
                py_name(&db.prefix)
            )));
        }
        let names = foldseek_read_names(&db.prefix)?;
        Ok(read_flat_db(&ss)?
            .into_iter()
            .map(|(key, sequence)| {
                (
                    names.get(&key).cloned().unwrap_or_else(|| key.to_string()),
                    sequence,
                )
            })
            .collect())
    }

    /// Copy the backend's raw hit table out of the work directory (upstream
    /// `copy_hits_tsv`), the most processed one that exists.
    pub fn copy_hits_tsv(&self, work_dir: &Path, destination: &str) -> io::Result<()> {
        for candidate in self.results_filenames() {
            let source = work_dir.join(candidate);
            if source.is_file() {
                std::fs::copy(&source, destination)?;
                return Ok(());
            }
        }
        crate::log::warn(format!(
            "No raw hit table was found in '{}' for backend '{}'; --hits_tsv_out not written.",
            work_dir.display(),
            self.name()
        ));
        Ok(())
    }
}

/// A list of strings as Python's `sorted(...)` prints it.
fn py_str_list(items: &[&str]) -> String {
    let quoted: Vec<String> = items.iter().map(|s| format!("'{s}'")).collect();
    format!("[{}]", quoted.join(", "))
}

/// One absolute input path per line, the list foldseek and reseek build databases from.
fn write_path_list(list_path: &str, inputs: &[String]) -> io::Result<()> {
    if let Some(parent) = Path::new(list_path)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let mut w = BufWriter::new(File::create(list_path)?);
    for path in inputs {
        writeln!(w, "{}", py_abspath(path))?;
    }
    w.flush()
}

/// Report reseek's minimum-length filter, which would otherwise lose records silently.
fn warn_on_dropped_chains(output: &std::process::Output, db_path: &str) {
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let re = regex::Regex::new(r"(\d+) too short").expect("a valid regex");
    if let Some(count) = re.captures(&text).and_then(|c| c[1].parse::<u64>().ok()) {
        if count > 0 {
            crate::log::warn(format!(
                "reseek skipped {count} chain(s) shorter than {RESEEK_MIN_CHAIN_LENGTH} residues \
                 while building '{db_path}'; those structures will be absent from the results."
            ));
        }
    }
}

fn parse_foldseek_line(line: &str, columns: &[String]) -> io::Result<StructureHit> {
    let fields: Vec<&str> = line.split('\t').collect();
    if fields.len() != columns.len() {
        return Err(other_error(format!(
            "Unexpected foldseek output: got {} columns, expected {} ({}) in line: {}",
            fields.len(),
            columns.len(),
            columns.join(","),
            head(line)
        )));
    }
    let row: HashMap<&str, &str> = columns.iter().map(String::as_str).zip(fields).collect();
    let get = |c: &str| row.get(c).copied().unwrap_or("");
    // A metric foldseek could not compute is not a number; it is then absent.
    let metric = |c: &str| row.get(c).and_then(|v| v.trim().parse::<f64>().ok());
    Ok(StructureHit {
        query: get("query").to_string(),
        target: get("target").to_string(),
        qstart: parse_int(get("qstart"), line)?,
        qend: parse_int(get("qend"), line)?,
        qlen: parse_int(get("qlen"), line)?,
        tstart: parse_int(get("tstart"), line)?,
        tend: parse_int(get("tend"), line)?,
        tlen: parse_int(get("tlen"), line)?,
        evalue: parse_float(get("evalue"), line)?,
        bits: parse_float(get("bits"), line)?,
        fident: parse_float(get("fident"), line)?,
        alnlen: parse_int(get("alnlen"), line)?,
        tseq: get("tseq").to_string(),
        qheader: get("qheader").to_string(),
        tmscore: metric("alntmscore"),
        lddt: metric("lddt"),
        rmsd: metric("rmsd"),
        prob: metric("prob"),
    })
}

fn reseek_fields(line: &str) -> io::Result<Vec<&str>> {
    let fields: Vec<&str> = line.split('\t').collect();
    if fields.len() != RESEEK_COLUMNS.len() {
        return Err(other_error(format!(
            "Unexpected reseek output: got {} columns, expected {} ({}) in line: {}",
            fields.len(),
            RESEEK_COLUMNS.len(),
            RESEEK_COLUMNS.join("+"),
            head(line)
        )));
    }
    Ok(fields)
}

fn parse_reseek_line(line: &str, db_size: f64) -> io::Result<StructureHit> {
    let f = reseek_fields(line)?;
    let pvalue = parse_float(f[9], line)?;
    let (tlo, thi) = (parse_int(f[5], line)?, parse_int(f[6], line)?);
    Ok(StructureHit {
        query: f[0].to_string(),
        target: f[1].to_string(),
        qstart: parse_int(f[2], line)?,
        qend: parse_int(f[3], line)?,
        qlen: parse_int(f[4], line)?,
        tstart: tlo,
        tend: thi,
        tlen: parse_int(f[7], line)?,
        // E = p * N, so -e keeps its meaning across backends.
        evalue: pvalue * db_size,
        // No bit score: -log10(p) is monotonic in significance and positive for
        // significant hits, which is what the overlap filter needs.
        bits: neg_log10(pvalue),
        // pctid is a percentage, foldseek's fident a fraction.
        fident: parse_float(f[8], line)? / 100.0,
        alnlen: (thi - tlo).abs() + 1,
        ..StructureHit::default()
    })
}

/// The best `max_seqs` hit lines per query by p-value, ties in file order; queries in
/// the order they first appear (upstream `ReseekAligner._apply_max_seqs`).
fn reseek_best_per_query(path: &Path, max_seqs: usize) -> io::Result<Vec<String>> {
    let mut order: Vec<String> = Vec::new();
    let mut by_query: HashMap<String, Vec<(f64, String)>> = HashMap::new();
    let trim = |kept: &mut Vec<(f64, String)>, cap: usize| {
        kept.sort_by(|a, b| a.0.total_cmp(&b.0));
        kept.truncate(cap);
    };
    for line in BufReader::new(File::open(path)?).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let fields = reseek_fields(&line)?;
        let pvalue = parse_float(fields[9], &line)?;
        let query = fields[0].to_string();
        let kept = by_query.entry(query.clone()).or_insert_with(|| {
            order.push(query);
            Vec::new()
        });
        kept.push((pvalue, line));
        if kept.len() > 2 * max_seqs {
            trim(kept, max_seqs);
        }
    }
    let mut out = Vec::new();
    for query in order {
        if let Some(mut kept) = by_query.remove(&query) {
            trim(&mut kept, max_seqs);
            out.extend(kept.into_iter().map(|(_, line)| line));
        }
    }
    Ok(out)
}

/// `(key, sequence)` from a foldseek flat database: `<prefix>.index` rows are
/// `key<TAB>offset<TAB>length` into the data file, whose records end `\n\0`. Read
/// directly because the `_ss` sub-database has no header database, so `convert2fasta`
/// refuses it.
fn read_flat_db(prefix: &str) -> io::Result<Vec<(u64, String)>> {
    let index_path = format!("{prefix}.index");
    if !Path::new(&index_path).is_file() || !Path::new(prefix).is_file() {
        return Err(other_error(format!(
            "'{prefix}' is not a readable foldseek database."
        )));
    }
    let mut data = File::open(prefix)?;
    let mut out = Vec::new();
    for line in BufReader::new(File::open(&index_path)?).lines() {
        let line = line?;
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 3 {
            continue;
        }
        let bad = || other_error(format!("Bad row in '{index_path}': {line}"));
        let key: u64 = fields[0].parse().map_err(|_| bad())?;
        let offset: u64 = fields[1].parse().map_err(|_| bad())?;
        let length: usize = fields[2].parse().map_err(|_| bad())?;
        use std::io::Seek;
        data.seek(io::SeekFrom::Start(offset))?;
        let mut record = vec![0u8; length];
        let read = data.read(&mut record)?;
        record.truncate(read);
        while record.last() == Some(&0) {
            record.pop();
        }
        while record.last() == Some(&b'\n') {
            record.pop();
        }
        out.push((key, String::from_utf8_lossy(&record).into_owned()));
    }
    Ok(out)
}

/// `{key: entry name}` from `<prefix>.lookup`.
fn foldseek_read_names(prefix: &str) -> io::Result<HashMap<u64, String>> {
    let lookup = format!("{prefix}.lookup");
    if !Path::new(&lookup).is_file() {
        return Err(other_error(format!(
            "Database '{prefix}' has no .lookup file, so entry names cannot be recovered. \
             Rebuild it with 'foldseek createdb --write-lookup 1'."
        )));
    }
    let mut names = HashMap::new();
    for line in BufReader::new(File::open(&lookup)?).lines() {
        let line = line?;
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() >= 2 {
            let key = fields[0]
                .trim()
                .parse()
                .map_err(|_| other_error(format!("Bad row in '{lookup}': {line}")))?;
            names.insert(key, fields[1].to_string());
        }
    }
    Ok(names)
}

/// The name foldseek records in `<prefix>.source` for an input file: less a compression
/// suffix, then two more suffixes (`some.long.name.pdb.gz` -> `some.long`).
fn foldseek_source_key(path: &str) -> String {
    let mut name = py_name(path).to_string();
    let suffix = py_suffix(&name);
    if !suffix.is_empty() && is_compression_suffix(&suffix[1..]) {
        name = py_stem(&name).to_string();
    }
    for _ in 0..2 {
        name = py_stem(&name).to_string();
    }
    name
}

/// Entry name -> the input file it came from, via `.lookup` (key -> name, file index)
/// and `.source` (file index -> truncated file name). An ambiguous truncated name is
/// dropped rather than guessed at: provenance is cosmetic.
fn foldseek_lookup_sources(db: &StructureDb) -> io::Result<HashMap<String, String>> {
    let lookup = format!("{}.lookup", db.prefix);
    let source = format!("{}.source", db.prefix);
    if !Path::new(&lookup).is_file() || db.source_paths.is_empty() {
        return Ok(HashMap::new());
    }
    let mut by_key: BTreeMap<String, Vec<&String>> = BTreeMap::new();
    for path in &db.source_paths {
        by_key
            .entry(foldseek_source_key(path))
            .or_default()
            .push(path);
    }
    let mut index_to_path: HashMap<i64, String> = HashMap::new();
    if Path::new(&source).is_file() {
        for line in BufReader::new(File::open(&source)?).lines() {
            let line = line?;
            let fields: Vec<&str> = line.split('\t').collect();
            if fields.len() < 2 {
                continue;
            }
            let Ok(index) = fields[0].trim().parse::<i64>() else {
                continue;
            };
            if let Some([only]) = by_key.get(fields[1]).map(Vec::as_slice) {
                index_to_path.insert(index, (*only).clone());
            }
        }
    } else if db.source_paths.len() == 1 {
        index_to_path.insert(0, db.source_paths[0].clone());
    }
    let mut sources = HashMap::new();
    for line in BufReader::new(File::open(&lookup)?).lines() {
        let line = line?;
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 3 {
            continue;
        }
        let Ok(index) = fields[2].trim().parse::<i64>() else {
            continue;
        };
        if let Some(path) = index_to_path.get(&index) {
            sources.insert(fields[1].to_string(), path.clone());
        }
    }
    Ok(sources)
}

/// Records of reseek's `-convert -fasta` output: the first header word, and the
/// sequence lines joined.
fn read_reseek_fasta(path: &Path) -> io::Result<Vec<(String, String, Option<String>)>> {
    let mut out = Vec::new();
    let mut name: Option<String> = None;
    let mut chunks = String::new();
    for line in BufReader::new(File::open(path)?).lines() {
        let line = line?;
        if let Some(header) = line.strip_prefix('>') {
            if let Some(name) = name.take() {
                out.push((name, std::mem::take(&mut chunks), None));
            }
            name = Some(header.split_whitespace().next().unwrap_or("").to_string());
            chunks.clear();
        } else if !line.is_empty() {
            chunks.push_str(line.trim());
        }
    }
    if let Some(name) = name {
        out.push((name, chunks, None));
    }
    Ok(out)
}

// ---------------------------------------------------------------- annotations

/// One hit as an annotation on its input record, if its E-value is within
/// `[min_evalue, evalue]` (upstream `structure_hits_to_search_results`, one hit at a
/// time). Coordinates on the input come from the target columns, and on the reference
/// from the query columns.
pub fn hit_to_search_result(
    hit: &StructureHit,
    db_name: &str,
    evalue: f64,
    min_evalue: f64,
    program: &str,
) -> Option<SearchResult> {
    if !(hit.evalue <= evalue && hit.evalue >= min_evalue) {
        return None;
    }
    let desc = hit
        .qheader
        .split_once(' ')
        .map(|(_, rest)| rest.to_string())
        .unwrap_or_default();
    Some(SearchResult {
        name: strip_structure_extension(&hit.query),
        desc,
        acc: String::new(),
        evalue: hit.evalue,
        score: hit.bits,
        // min/max because a backend may report a descending range.
        start: hit.tstart.min(hit.tend) - 1,
        end: hit.tstart.max(hit.tend),
        database: db_name.to_string(),
        identity: hit.fident * 100.0,
        rstart: hit.qstart.min(hit.qend),
        rend: hit.qstart.max(hit.qend),
        rlen: hit.qlen,
        program: program.to_string(),
        strand: 1,
        tmscore: hit.tmscore,
        lddt: hit.lddt,
        rmsd: hit.rmsd,
        prob: hit.prob,
    })
}

/// A protein record for one structure chain (upstream `build_protein_record`), with its
/// source file in the DEFINITION and, given one of the same length, its 3Di string on a
/// full-length `misc_feature`.
pub fn build_protein_record(
    name: &str,
    sequence: &str,
    source_path: Option<&str>,
    threedi: Option<&str>,
) -> SeqRecord {
    let mut record = SeqRecord {
        id: name.to_string(),
        name: name.to_string(),
        description: source_path
            .map(|p| format!("structure={p}"))
            .unwrap_or_default(),
        seq: sequence.as_bytes().to_vec(),
        molecule_type: Some("protein".to_string()),
        topology: Topology::Linear,
        annotations: Annotations::default(),
        features: Vec::new(),
    };
    if let Some(threedi) = threedi {
        let (n3, ns) = (threedi.chars().count(), sequence.chars().count());
        if n3 == ns {
            let mut feature = SeqFeature::new("misc_feature", Location::range(0, ns as i64, 1));
            feature.push("threedi", threedi);
            record.features.push(feature);
        } else {
            crate::log::warn(format!(
                "3Di length ({n3}) does not match sequence length ({ns}) for {name}; not \
                 storing 3Di."
            ));
        }
    }
    record
}

/// Warn when a query returned at least `max_seqs` hits, so was likely truncated
/// (upstream `warn_on_saturation`). `subject` names what the counted keys are.
pub fn warn_on_saturation(
    hit_counts: &HashMap<String, usize>,
    max_seqs: Option<usize>,
    subject: &str,
) {
    let Some(max_seqs) = max_seqs else {
        return;
    };
    let mut saturated: Vec<&str> = hit_counts
        .iter()
        .filter(|(_, &count)| count >= max_seqs)
        .map(|(name, _)| name.as_str())
        .collect();
    if saturated.is_empty() {
        return;
    }
    saturated.sort_unstable();
    let more = if saturated.len() > 5 { " ..." } else { "" };
    crate::log::warn(format!(
        "--max_seqs ({max_seqs}) was reached by {} {subject}(s): {}{more}. Results for those \
         {subject}s are truncated; raise --max_seqs for full coverage.",
        saturated.len(),
        saturated[..saturated.len().min(5)].join(", ")
    ));
}

/// The databases and scratch space for one search (upstream `prepared_databases`).
/// Databases built here live in `work_dir`, removed on drop, except an input database
/// built at `keep_db`.
pub(crate) struct Prepared {
    pub input_db: StructureDb,
    pub reference_db: StructureDb,
    pub work_dir: TempDir,
    pub reference_label: String,
}

/// Resolve `-i` and `-r` into databases, building any that are not prebuilt.
/// `references` of `None` compares the input against itself, reusing its database.
pub(crate) fn prepare_databases(
    aligner: &Aligner,
    inputs: &[String],
    references: Option<&[String]>,
    keep_db: Option<&str>,
) -> io::Result<Prepared> {
    let input_resolved = resolve_structure_inputs(inputs)?;
    let reference_resolved = references.map(resolve_structure_inputs).transpose()?;
    if let (Resolved::Db(prefix), Some(_)) = (&input_resolved, keep_db) {
        return Err(input_error(format!(
            "--db_out writes the database built from input structures, but the input is \
             already a database ('{prefix}')."
        )));
    }
    let work_dir = TempDir::new_in(
        aligner.tmp_dir.as_deref().map(Path::new),
        "domainatoredux_structure",
    )?;
    let input_db = match &input_resolved {
        Resolved::Db(prefix) => {
            let db = StructureDb::prebuilt(prefix);
            aligner.validate_database(&db)?;
            db
        }
        Resolved::Structures(files) => {
            let prefix = match keep_db {
                Some(prefix) => prefix.to_string(),
                None => work_dir
                    .path()
                    .join("inputdb")
                    .to_string_lossy()
                    .into_owned(),
            };
            aligner.build_db(files, &prefix)?
        }
    };
    let reference_db = match &reference_resolved {
        None => input_db.clone(),
        Some(Resolved::Db(prefix)) => {
            let db = StructureDb::prebuilt(prefix);
            aligner.validate_database(&db)?;
            db
        }
        Some(Resolved::Structures(files)) => {
            let prefix = work_dir.path().join("refdb").to_string_lossy().into_owned();
            aligner.build_db(files, &prefix)?
        }
    };
    let reference_label = match (references, &reference_resolved) {
        (Some(values), Some(resolved)) => input_label(values, resolved),
        _ => input_label(inputs, &input_resolved),
    };
    Ok(Prepared {
        input_db,
        reference_db,
        work_dir,
        reference_label,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structure_paths_follow_the_shared_extension_map() {
        for (path, expected) in [
            ("a.pdb", true),
            ("a.cif", true),
            ("a.mmcif", true),
            ("a.pdb.gz", true),
            ("a.cif.gz", true),
            ("some.long.name.pdb.gz", true),
            ("/x/y/a.ent", true),
            ("x.pdb1", true),
            ("x.bcif", true),
            ("A.PDB", true),
            ("a.gb", false),
            ("a.fasta", false),
            ("a.hmm", false),
            ("a.gb.gz", false),
            ("noextension", false),
            ("x.bcb", false),
        ] {
            assert_eq!(is_structure_path(path), expected, "{path}");
        }
    }

    #[test]
    fn database_labels_keep_dotted_prefixes() {
        assert_eq!(database_label("/a/b/refs.v1"), "refs.v1");
        assert_eq!(database_label("/a/b/refs.v2"), "refs.v2");
        assert_eq!(database_label("/a/b/UBQref.pdb.gz"), "UBQref");
        assert_eq!(database_label("/a/b/UBQref.pdb"), "UBQref");
        assert_eq!(database_label("refs/"), "refs");
    }

    #[test]
    fn structure_extensions_are_stripped_from_entry_names() {
        for (name, expected) in [
            ("1abc.pdb_A", "1abc_A"),
            ("x.cif_B", "x_B"),
            ("1abc.pdb", "1abc"),
            ("plain_A", "plain_A"),
            ("some.long.name_A", "some.long.name_A"),
            ("x.mmcif_C", "x_C"),
        ] {
            assert_eq!(strip_structure_extension(name), expected, "{name}");
        }
    }

    #[test]
    fn source_keys_match_foldseek_truncation() {
        for (path, expected) in [
            ("x.pdb", "x"),
            ("x.pdb.gz", "x"),
            ("some.long.name.pdb", "some.long"),
            ("some.long.name.pdb.gz", "some.long"),
            ("/a/b/1UBQ.pdb.gz", "1UBQ"),
        ] {
            assert_eq!(foldseek_source_key(path), expected, "{path}");
        }
    }

    #[test]
    fn paths_are_spelled_as_pathlib_spells_them() {
        assert_eq!(py_path_str("./inputs/"), "inputs");
        assert_eq!(py_path_str("a//b/./c"), "a/b/c");
        assert_eq!(py_path_str("/x/"), "/x");
        assert_eq!(py_path_str("."), ".");
        assert_eq!(py_parent("a/b/c.pdb"), "a/b");
        assert_eq!(py_parent("c.pdb"), ".");
        assert!(py_abspath("a/../b/./c").ends_with("/b/c"));
    }

    #[test]
    fn hits_use_target_coordinates_on_the_input() {
        let hit = StructureHit {
            query: "REF_A".into(),
            target: "IN_A".into(),
            qstart: 5,
            qend: 40,
            qlen: 100,
            tstart: 11,
            tend: 60,
            tlen: 200,
            evalue: 1e-9,
            bits: 123.0,
            fident: 0.75,
            alnlen: 50,
            qheader: "REF_A some description".into(),
            ..StructureHit::default()
        };
        let r = hit_to_search_result(&hit, "mydb", 1.0, 0.0, "foldseek").expect("kept");
        assert_eq!((r.start, r.end), (10, 60));
        assert_eq!((r.rstart, r.rend, r.rlen), (5, 40, 100));
        assert_eq!(r.name, "REF_A");
        assert_eq!(r.desc, "some description");
        assert!((r.identity - 75.0).abs() < 1e-9);

        let reversed = StructureHit {
            tstart: 50,
            tend: 20,
            ..hit.clone()
        };
        let r = hit_to_search_result(&reversed, "db", 1.0, 0.0, "foldseek").expect("kept");
        assert_eq!((r.start, r.end), (19, 50));

        // Both E-value bounds apply, inclusive.
        for (e, kept) in [(1e-20, false), (1e-5, true), (10.0, false), (1.0, true)] {
            let h = StructureHit {
                evalue: e,
                ..hit.clone()
            };
            assert_eq!(
                hit_to_search_result(&h, "db", 1.0, 1e-10, "foldseek").is_some(),
                kept,
                "{e}"
            );
        }
    }

    #[test]
    fn a_3di_string_of_the_wrong_length_is_not_stored() {
        let record = build_protein_record("1UBQ_A", "MQIFV", Some("/x/1UBQ.pdb"), None);
        assert_eq!(record.description, "structure=/x/1UBQ.pdb");
        assert!(record.features.is_empty());
        let record = build_protein_record("x", "MQIFV", None, Some("DWEWE"));
        assert_eq!(record.features[0].kind, "misc_feature");
        assert_eq!(record.description, "");
        let record = build_protein_record("x", "MQIFV", None, Some("DW"));
        assert!(record.features.is_empty());
    }

    #[test]
    fn extra_args_cannot_override_what_domainator_sets() {
        let argv: Vec<String> = ["foldseek", "search", "-e", "0.001"]
            .map(String::from)
            .to_vec();
        let ok = append_extra_args(
            argv.clone(),
            &["--exhaustive-search".into(), "1".into()],
            "--foldseek_args",
        );
        assert_eq!(ok.expect("allowed").len(), 6);
        let err = append_extra_args(argv, &["-e".into(), "5".into()], "--foldseek_args")
            .expect_err("refused");
        assert!(
            err.to_string().contains("--foldseek_args cannot set -e"),
            "{err}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_explicit_path_must_be_an_executable_file() {
        let dir = TempDir::new("domainatoredux_bin_test").expect("a temp dir");
        let bogus = dir.path().join("notabinary");
        std::fs::write(&bogus, "hello").expect("written");
        let opts = BackendOptions {
            foldseek_path: Some(bogus.to_string_lossy().into_owned()),
            ..BackendOptions::default()
        };
        let err = Aligner::new(&opts).expect_err("refused");
        assert!(err.to_string().contains("not an executable file"), "{err}");
    }

    #[test]
    fn the_other_backends_args_are_refused() {
        let opts = BackendOptions {
            reseek_args: Some("-verysensitive".into()),
            ..BackendOptions::default()
        };
        let err = Aligner::new(&opts).expect_err("refused");
        assert!(
            err.to_string()
                .contains("--reseek_args was given, but the search backend is foldseek"),
            "{err}"
        );
    }

    #[test]
    fn hits_sort_stably_on_the_target_bytes() {
        let dir = TempDir::new("domainatoredux_sort_test").expect("a temp dir");
        let source = dir.path().join("hits.tsv");
        std::fs::write(
            &source,
            "r2\tb_1\t9\nr1\tB_1\t1\nr3\tb_1\t2\nr1\ta_1\t5\nr0\tb_1\t7",
        )
        .expect("written");
        let sorted = dir.path().join("sorted.tsv");
        sort_by_column(&source, 1, &sorted, dir.path()).expect("sorted");
        let expected = "r1\tB_1\t1\nr1\ta_1\t5\nr2\tb_1\t9\nr3\tb_1\t2\nr0\tb_1\t7\n";
        assert_eq!(std::fs::read_to_string(&sorted).expect("read"), expected);
        // Sorted a line or two at a time and merged from disk: still stable.
        for chunk in [1, 10, 20] {
            sort_by_column_in_chunks(&source, 1, &sorted, dir.path(), chunk).expect("sorted");
            assert_eq!(
                std::fs::read_to_string(&sorted).expect("read"),
                expected,
                "{chunk}"
            );
        }
    }

    #[test]
    fn reseek_keeps_the_best_hits_per_query_in_first_seen_order() {
        let dir = TempDir::new("domainatoredux_reseek_test").expect("a temp dir");
        let path = dir.path().join("hits.tsv");
        let row = |q: &str, t: &str, p: &str| format!("{q}\t{t}\t1\t2\t3\t1\t2\t3\t50\t{p}\n");
        let text = [
            row("q2", "a", "1e-3"),
            row("q1", "b", "1e-5"),
            row("q2", "c", "1e-9"),
            row("q2", "d", "1e-9"),
            row("q1", "e", "1e-1"),
        ]
        .concat();
        std::fs::write(&path, text).expect("written");
        let kept = reseek_best_per_query(&path, 2).expect("read");
        let targets: Vec<&str> = kept
            .iter()
            .map(|l| l.split('\t').nth(1).unwrap_or(""))
            .collect();
        assert_eq!(targets, ["c", "d", "b", "e"]);
    }
}
