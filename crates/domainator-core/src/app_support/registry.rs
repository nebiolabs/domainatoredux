//! The database registry (the "Databases" panel).
//!
//! Databases are formatted sequence/HMM sets — usually large — that stay where
//! they are on disk and are registered by path. **One registry serves every
//! workspace**: the GUI keeps it in the app data directory ([`load_registry_file`]
//! / [`save_registry_file`] take its path), so a database added once is offered
//! by every workspace's tool dialogs. Each entry records its `format`
//! (fasta | genbank | hmm) and `molecule` (protein | nucleic), so the UI can show
//! the six categories from the spec, plus the resolved shard count.
//!
//! Workspaces used to keep their own list in `.domainatoredux/databases.json`;
//! [`migrate_workspace_registry`] folds one of those into the central list the
//! first time the workspace is opened, then retires it.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::app_support::workspace::{classify_file, dot_dir};
use crate::io::didx::index_path_for;
use crate::io::shards::{resolve_database_shards, split_db_name};
use crate::io::{detect_compression, detect_filetype, Compression, FileType};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseEntry {
    /// Logical database path (a single file or a shard base like `db.gb`).
    pub path: String,
    pub name: String,
    /// `fasta` | `genbank` | `hmm` | `taxonomy` (an NCBI taxdump archive).
    pub format: String,
    /// `protein` | `nucleic` | `unknown`.
    pub molecule: String,
    /// Number of resolved shard files (1 for an unsharded db).
    pub shards: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DatabaseRegistry {
    #[serde(default)]
    pub databases: Vec<DatabaseEntry>,
}

impl DatabaseRegistry {
    /// Add `entry`, replacing any entry at the same path and any at `replaces`.
    pub fn upsert(&mut self, entry: DatabaseEntry, replaces: &[String]) {
        self.databases
            .retain(|d| d.path != entry.path && !replaces.contains(&d.path));
        self.databases.push(entry);
    }

    /// Drop the entry at `path`; whether there was one.
    pub fn remove(&mut self, path: &str) -> bool {
        let before = self.databases.len();
        self.databases.retain(|d| d.path != path);
        self.databases.len() != before
    }
}

/// The file name of a registry, central or (legacy) per-workspace.
pub const REGISTRY_NAME: &str = "databases.json";

/// What a workspace's own registry is renamed to once it has been merged into
/// the central one: kept for the record, never read again (so a database removed
/// centrally does not come back the next time that workspace opens).
const MIGRATED_NAME: &str = "databases.json.migrated";

fn workspace_registry_path(root: &Path) -> PathBuf {
    dot_dir(root).join(REGISTRY_NAME)
}

/// Read a registry file. A missing or corrupt file reads as empty rather than
/// failing the panel.
pub fn load_registry_file(file: &Path) -> DatabaseRegistry {
    fs::read(file)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Write a registry file, write-then-rename so a crash mid-write cannot leave
/// every workspace with an empty database list.
pub fn save_registry_file(file: &Path, reg: &DatabaseRegistry) -> std::io::Result<()> {
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_vec_pretty(reg)?;
    let tmp = file.with_extension("json.tmp");
    fs::write(&tmp, json)?;
    fs::rename(&tmp, file)
}

/// Fold the workspace at `root`'s legacy registry into the central one at
/// `central`, then retire it. Returns how many entries were new to the central
/// list; `Ok(0)` for a workspace that has no legacy registry.
pub fn migrate_workspace_registry(central: &Path, root: &Path) -> std::io::Result<usize> {
    let legacy = workspace_registry_path(root);
    if !legacy.is_file() {
        return Ok(0);
    }
    let old = load_registry_file(&legacy);
    let mut reg = load_registry_file(central);
    let mut added = 0;
    for entry in old.databases {
        if !reg.databases.iter().any(|d| d.path == entry.path) {
            reg.databases.push(entry);
            added += 1;
        }
    }
    if added > 0 {
        save_registry_file(central, &reg)?;
    }
    fs::rename(&legacy, dot_dir(root).join(MIGRATED_NAME))?;
    Ok(added)
}

/// Every file on disk that makes up the database at `path`: its data file, or
/// each resolved shard, followed by that file's `.didx` index when one exists.
/// Empty when nothing is there.
pub fn database_files(path: &Path) -> Vec<PathBuf> {
    let shards = resolve_database_shards(path).unwrap_or_else(|_| vec![path.to_path_buf()]);
    let mut out = Vec::new();
    for shard in shards.into_iter().filter(|s| s.is_file()) {
        let index = index_path_for(&shard);
        out.push(shard);
        if index.is_file() {
            out.push(index);
        }
    }
    out
}

/// Rename the database at `path` to `new_name` (a bare file name in the same
/// directory), shards and `.didx` indexes included. Returns the new logical path.
///
/// The format and compression suffixes must stay the same: they are what shard
/// resolution and the readers key on, so `db.fasta` can become `nr.fasta` but not
/// `nr.gb`. An index stays valid across the rename because it fingerprints the
/// data file's size and mtime, which a rename keeps. Every target is checked
/// before anything moves, and a failure part-way moves the finished ones back.
pub fn rename_database(path: &Path, new_name: &str) -> Result<PathBuf, String> {
    let clean = new_name.trim();
    if clean.is_empty() || clean.contains(['/', '\\']) || clean == "." || clean == ".." {
        return Err("invalid file name".into());
    }
    let (dir, old_stem, old_fmt, old_comp) = split_db_name(path);
    let new_path = dir.join(clean);
    let (_, new_stem, new_fmt, new_comp) = split_db_name(&new_path);
    if !new_fmt.eq_ignore_ascii_case(&old_fmt) || !new_comp.eq_ignore_ascii_case(&old_comp) {
        let suffix = if old_comp.is_empty() {
            format!(".{old_fmt}")
        } else {
            format!(".{old_fmt}.{old_comp}")
        };
        return Err(format!(
            "a database keeps its format when renamed: the new name must end in {suffix}"
        ));
    }
    if new_stem.is_empty() {
        return Err("invalid file name".into());
    }

    let files = database_files(path);
    if files.is_empty() {
        return Err(format!("{} is not on disk", path.display()));
    }
    let mut moves = Vec::with_capacity(files.len());
    for from in files {
        let name = from
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let rest = name
            .strip_prefix(old_stem.as_str())
            .ok_or_else(|| format!("{} does not start with {old_stem}", from.display()))?;
        let to = dir.join(format!("{new_stem}{rest}"));
        if to.exists() {
            return Err(format!("already exists: {}", to.display()));
        }
        moves.push((from, to));
    }
    for (i, (from, to)) in moves.iter().enumerate() {
        if let Err(e) = fs::rename(from, to) {
            for (back_from, back_to) in moves[..i].iter().rev() {
                let _ = fs::rename(back_to, back_from);
            }
            return Err(format!("renaming {}: {e}", from.display()));
        }
    }
    Ok(new_path)
}

/// Whether a database has the `.didx` offset index that lets `domain_search`
/// seek into it (and read `-Z 0`'s target count without a scan).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexStatus {
    /// Every data file has a fresh index.
    Indexed,
    /// An index exists for some file but no longer matches it (the data changed
    /// since it was built), or some files are indexed and others are not.
    Stale,
    /// No index at all.
    Missing,
    /// Plain (non-BGZF) gzip: not seekable, so it cannot carry an index until it
    /// is recompressed as BGZF (`format_db --compress`).
    Unindexable,
    /// HMM libraries are read front to back; there is nothing to index.
    NotApplicable,
}

fn seq_filetype(format: &str) -> Option<FileType> {
    match format {
        "fasta" => Some(FileType::Fasta),
        "genbank" => Some(FileType::Genbank),
        _ => None,
    }
}

/// The index status of the database at `path` with registry `format`. Reads only
/// each shard's first bytes and its index header, so it is cheap on any size.
pub fn index_status(path: &Path, format: &str) -> IndexStatus {
    let Some(filetype) = seq_filetype(format) else {
        return IndexStatus::NotApplicable;
    };
    let shards: Vec<PathBuf> = resolve_database_shards(path)
        .unwrap_or_else(|_| vec![path.to_path_buf()])
        .into_iter()
        .filter(|s| s.is_file())
        .collect();
    if shards.is_empty() {
        return IndexStatus::Missing;
    }
    let (mut fresh, mut present) = (0, 0);
    for shard in &shards {
        let compression = detect_compression(shard);
        if compression == Compression::Gzip {
            return IndexStatus::Unindexable;
        }
        if index_path_for(shard).is_file() {
            present += 1;
        }
        if matches!(
            crate::io::didx::read_total_cds(shard, filetype, compression),
            Ok(Some(_))
        ) {
            fresh += 1;
        }
    }
    if fresh == shards.len() {
        IndexStatus::Indexed
    } else if present == 0 {
        IndexStatus::Missing
    } else {
        IndexStatus::Stale
    }
}

/// Write a fresh `.didx` beside each data file of the database at `path`,
/// through `format_db`'s index-only path, which indexes a file in place rather
/// than copying it (a 40 GB FASTA stays one file). Returns how many were written.
pub fn build_database_index(path: &Path) -> Result<usize, String> {
    let shards = index_targets(path)?;
    let opts = crate::tools::format_db::FormatDbOptions {
        index: true,
        ..Default::default()
    };
    for shard in &shards {
        crate::tools::format_db::format_db(
            std::slice::from_ref(shard),
            &opts,
            &mut std::io::sink(),
        )
        .map_err(|e| format!("indexing {shard}: {e}"))?;
    }
    Ok(shards.len())
}

/// The data files [`build_database_index`] would index, after the checks it makes
/// first: the database is on disk, and none of it is plain gzip. The GUI runs the
/// indexing itself as `format_db --index` over these, in a worker process it can
/// kill.
pub fn index_targets(path: &Path) -> Result<Vec<String>, String> {
    let shards: Vec<String> = resolve_database_shards(path)?
        .into_iter()
        .filter(|s| s.is_file())
        .map(|s| s.to_string_lossy().into_owned())
        .collect();
    if shards.is_empty() {
        return Err(format!("{} is not on disk", path.display()));
    }
    if shards
        .iter()
        .any(|s| detect_compression(Path::new(s)) == Compression::Gzip)
    {
        return Err(
            "plain gzip cannot be indexed; recompress it as BGZF with format_db --compress".into(),
        );
    }
    Ok(shards)
}

/// Delete every file of the database at `path` ([`database_files`]). Returns
/// what was removed.
pub fn delete_database_files(path: &Path) -> Result<Vec<PathBuf>, String> {
    let files = database_files(path);
    for f in &files {
        fs::remove_file(f).map_err(|e| format!("deleting {}: {e}", f.display()))?;
    }
    Ok(files)
}

/// Sniff whether a residue string looks like nucleic acid (mostly A/C/G/T/U/N).
fn looks_nucleic(seq: &[u8]) -> bool {
    let mut n = 0usize;
    let mut total = 0usize;
    for &b in seq.iter().take(2000) {
        if b.is_ascii_alphabetic() {
            total += 1;
            if matches!(
                b.to_ascii_uppercase(),
                b'A' | b'C' | b'G' | b'T' | b'U' | b'N'
            ) {
                n += 1;
            }
        }
    }
    total > 0 && (n as f64 / total as f64) > 0.9
}

/// First HMM's alphabet: `protein` (amino) or `nucleic` (dna/rna).
///
/// Reads the first profile's header lines only, never the emission matrices: Pfam-A
/// is ~30k models and 1.6 GB, and parsing all of them to learn that the first one
/// says `ALPH amino` froze the Databases panel for minutes. HMMER's own default is
/// amino, so a well-formed profile with no `ALPH` line is protein; a file we cannot
/// open, or one that never presents a `HMMER3` header, stays `unknown`.
///
/// A gzipped library is read through the decompressor (the managed Pfam download
/// stays `.hmm.gz`); only the header block is ever inflated.
fn hmm_molecule(path: &str) -> String {
    let Ok(fh) = crate::io::open_decompressed(Path::new(path)) else {
        return "unknown".into();
    };
    let mut reader = std::io::BufReader::new(fh);
    let mut line = String::new();
    let mut saw_header = false;
    // The header block ends at `HMM` (the match-column key line); ALPH is always
    // above it, so there is no reason to read further.
    while {
        line.clear();
        std::io::BufRead::read_line(&mut reader, &mut line).unwrap_or(0) > 0
    } {
        let trimmed = line.trim_end();
        if trimmed.starts_with("HMMER3") {
            saw_header = true;
            continue;
        }
        if !saw_header {
            return "unknown".into();
        }
        if let Some(val) = trimmed.strip_prefix("ALPH") {
            return match val.trim().to_ascii_lowercase().as_str() {
                "dna" | "rna" => "nucleic".into(),
                "amino" => "protein".into(),
                _ => "unknown".into(),
            };
        }
        if trimmed.split_whitespace().next() == Some("HMM") {
            break;
        }
    }
    if saw_header {
        "protein".into()
    } else {
        "unknown".into()
    }
}

/// Whether `path` names an NCBI taxdump archive (`taxdump.tar.gz`, as NCBI
/// publishes it and as the GUI's managed download stores it), which
/// `NCBITaxonomy::load` reads directly. By name alone: the archive is ~65 MB and
/// only a full decompression could see inside it.
pub fn is_taxdump_archive(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    name.starts_with("taxdump") && (name.ends_with(".tar.gz") || name.ends_with(".tgz"))
}

/// Classify a database path into a [`DatabaseEntry`] (reads the first shard's
/// first record to determine molecule type).
pub fn classify_database(path: &str) -> Result<DatabaseEntry, String> {
    let p = Path::new(path);
    // Not a sequence or profile database, but registered like one so tools can
    // take it as their `ncbi_taxonomy_path`; nothing in it has a molecule type.
    if is_taxdump_archive(p) {
        return Ok(DatabaseEntry {
            path: path.to_string(),
            name: "NCBI taxonomy".into(),
            format: "taxonomy".into(),
            molecule: "unknown".into(),
            shards: 1,
        });
    }
    // The name annotations are labeled with, so `Pfam-A.hmm.gz` is `Pfam-A`.
    let name = crate::io::db_name_from_path(path);

    // hmm files aren't handled by detect_filetype (which is genbank/fasta only),
    // so classify format from the extension first.
    let format = classify_file(p).ok_or_else(|| format!("unrecognized database type: {path}"))?;

    let shards = resolve_database_shards(p).unwrap_or_else(|_| vec![p.to_path_buf()]);
    let first = shards
        .first()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string());

    // Only ever read the FIRST record, and only through a reader that understands
    // the file's compression. A formatted database is routinely tens of gigabytes
    // (UniProtKB/TrEMBL is ~45 GB as BGZF), so reading it whole to look at record
    // one is not a slow path, it is a hang; and feeding BGZF bytes to the
    // uncompressed reader sniffs the gzip container rather than the sequence, which
    // silently mislabels every compressed protein database as the wrong molecule.
    //
    // `None` is plain (non-BGZF) gzip: not seekable, so no reader here can open it
    // at all, and `unknown` is the honest answer rather than a sniff of the
    // compressed bytes. It is still a database -- `format` is decided by the name.
    let compressed: Option<bool> = match detect_compression(Path::new(&first)) {
        Compression::Bgzf => Some(true),
        Compression::None => Some(false),
        Compression::Gzip => None,
    };

    let (format, molecule) = match format {
        // Every hmm reader decompresses gzip and BGZF alike (nothing seeks in a
        // profile file), so compression does not matter here.
        "hmm" => ("hmm".to_string(), hmm_molecule(&first)),
        "genbank" => {
            let mol = compressed
                .filter(|_| detect_filetype(Path::new(&first)) == Some(FileType::Genbank))
                .and_then(|c| crate::io::genbank::GenbankReader::open(&first, 0, c, None).ok())
                .and_then(|mut recs| recs.next())
                .and_then(|r| r.ok())
                .map(|r| {
                    if r.is_protein() {
                        "protein".to_string()
                    } else {
                        "nucleic".to_string()
                    }
                })
                .unwrap_or_else(|| "unknown".into());
            ("genbank".to_string(), mol)
        }
        "fasta" => {
            let mol = compressed
                .and_then(|c| crate::io::fasta::read_fasta_partition(&first, 0, c, 1, None).ok())
                .and_then(|recs| recs.into_iter().next())
                .map(|r| {
                    if looks_nucleic(&r.seq) {
                        "nucleic".to_string()
                    } else {
                        "protein".to_string()
                    }
                })
                .unwrap_or_else(|| "unknown".into());
            ("fasta".to_string(), mol)
        }
        other => return Err(format!("{other} files are not databases")),
    };

    Ok(DatabaseEntry {
        path: path.to_string(),
        name,
        format,
        molecule,
        shards: shards.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("domainator-registry-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn write(root: &Path, name: &str, body: &str) -> String {
        let p = root.join(name);
        fs::write(&p, body).unwrap();
        p.to_string_lossy().into_owned()
    }

    /// `looks_nucleic` decides whether a FASTA database is protein or nucleic, and
    /// that drives which reference dropdowns it appears in. The 90% threshold is
    /// what lets a nucleotide sequence carry ambiguity codes while still keeping a
    /// protein sequence that happens to be A/C/G/T-rich on the protein side.
    #[test]
    fn nucleic_is_sniffed_from_residue_composition() {
        assert!(looks_nucleic(b"ACGTACGTACGT"));
        assert!(looks_nucleic(b"acguacguNNNN"));
        // Gaps and newlines are ignored; only letters count.
        assert!(looks_nucleic(b"ACGT\n--ACGT\n"));
        assert!(!looks_nucleic(b"MKVLAWIYPQRSTHDEF"));
        // Exactly at the boundary: 9 of 10 letters is not *more* than 90%.
        assert!(!looks_nucleic(b"ACGTACGTAW"));
        assert!(looks_nucleic(b"ACGTACGTACGTACGTACGW"));
        // Nothing to go on.
        assert!(!looks_nucleic(b""));
        assert!(!looks_nucleic(b"----"));
    }

    #[test]
    fn a_fasta_database_is_classified_by_its_first_record() {
        let root = workspace("fasta");
        let dna = write(&root, "d.fasta", ">a\nACGTACGTACGT\n");
        let entry = classify_database(&dna).unwrap();
        assert_eq!(
            (entry.format.as_str(), entry.molecule.as_str()),
            ("fasta", "nucleic")
        );
        assert_eq!(entry.name, "d");
        assert_eq!(entry.shards, 1);

        let prot = write(&root, "p.faa", ">a\nMKVLAWIYPQRSTHDEF\n");
        let entry = classify_database(&prot).unwrap();
        assert_eq!(
            (entry.format.as_str(), entry.molecule.as_str()),
            ("fasta", "protein")
        );
    }

    /// A formatted database is normally BGZF, and the molecule sniff has to read it
    /// *as* BGZF. Handing compressed bytes to the uncompressed reader does not fail
    /// -- it "reads" the gzip container, which is mostly non-letters, and lands on
    /// `protein` for everything. Every database the app downloads is BGZF, so this
    /// is the common case, not the exotic one.
    #[test]
    fn a_bgzf_database_is_read_through_its_compression() {
        let root = workspace("bgzf");
        let path = root.join("d.fasta.bgz");
        let out = fs::File::create(&path).unwrap();
        let mut w = bgzf::Writer::new(
            std::io::BufWriter::new(out),
            bgzf::CompressionLevel::new(6).unwrap(),
        );
        std::io::Write::write_all(&mut w, b">a desc\nACGTACGTACGTACGT\n").unwrap();
        std::io::Write::flush(&mut w.finish().unwrap()).unwrap();

        let entry = classify_database(path.to_str().unwrap()).unwrap();
        assert_eq!(
            (entry.format.as_str(), entry.molecule.as_str()),
            ("fasta", "nucleic")
        );

        // Plain (non-BGZF) gzip is not seekable, so nothing can read record one --
        // but it is still a fasta database by name, and must classify as one. The
        // body here is deliberately *not* gzip: `.gz` without the BGZF subfield is
        // exactly what `detect_compression` calls plain gzip, and the point is that
        // the molecule is reported unknown rather than sniffed out of the bytes.
        let gz = write(&root, "g.fasta.gz", ">a\nACGTACGTACGT\n");
        let entry = classify_database(&gz).unwrap();
        assert_eq!(
            (entry.format.as_str(), entry.molecule.as_str()),
            ("fasta", "unknown")
        );
    }

    /// The alphabet comes out of the first profile's header block alone. Pfam-A is
    /// ~30k models / 1.6 GB, and the previous full parse made registering it look
    /// like the app had hung. The `HMM` key line is the stop: anything below it is
    /// emission matrices.
    #[test]
    fn an_hmm_database_is_classified_from_the_first_header_alone() {
        let root = workspace("hmm");

        let amino = write(
            &root,
            "p.hmm",
            "HMMER3/f [3.4]\nNAME  x\nLENG  10\nALPH  amino\nHMM  A C D\n",
        );
        assert_eq!(classify_database(&amino).unwrap().molecule, "protein");

        let dna = write(
            &root,
            "n.hmm",
            "HMMER3/f [3.4]\nNAME  x\nALPH  DNA\nHMM  A C G T\n",
        );
        assert_eq!(classify_database(&dna).unwrap().molecule, "nucleic");

        // HMMER defaults to amino when a profile declares no ALPH.
        let bare = write(
            &root,
            "b.hmm",
            "HMMER3/f [3.4]\nNAME  x\nHMM  A C D\nsomething\n",
        );
        assert_eq!(classify_database(&bare).unwrap().molecule, "protein");

        // Not an HMM at all, and a file that does not exist: both `unknown`, never
        // a guess and never a panic.
        let junk = write(&root, "j.hmm", "this is not a profile\n");
        assert_eq!(classify_database(&junk).unwrap().molecule, "unknown");
        assert_eq!(
            classify_database("/nonexistent/x.hmm").unwrap().molecule,
            "unknown"
        );
    }

    #[test]
    fn a_taxdump_archive_is_a_taxonomy_database() {
        for name in ["taxdump.tar.gz", "taxdump_2026-09.tgz", "TAXDUMP.TAR.GZ"] {
            let entry = classify_database(&format!("/somewhere/{name}")).unwrap();
            assert_eq!(entry.format, "taxonomy", "{name}");
            assert_eq!(
                index_status(Path::new(&entry.path), &entry.format),
                IndexStatus::NotApplicable
            );
        }
        // Any other tarball is not one.
        assert!(!is_taxdump_archive(Path::new("/x/backup.tar.gz")));
    }

    /// Two different refusals, both of which must be errors rather than a guess:
    /// an extension nothing recognizes, and one that is recognized but is not a
    /// sequence database (a matrix, a color table, a notes file).
    #[test]
    fn non_databases_are_refused_rather_than_guessed() {
        let root = workspace("unknown");
        let mystery = write(&root, "db.zzz", "hello");
        assert!(
            classify_database(&mystery)
                .unwrap_err()
                .contains("unrecognized database type"),
            "an unknown extension should say so"
        );

        for (name, kind) in [
            ("notes.txt", "text"),
            ("d.hdf5", "hdf5"),
            ("c.tsv", "table"),
        ] {
            let p = write(&root, name, "x");
            let err = classify_database(&p).unwrap_err();
            assert!(
                err.contains(kind) && err.contains("not databases"),
                "{name}: {err}"
            );
        }

        // A recognized extension that does not exist on disk still classifies --
        // the molecule is simply unknown. It must not panic.
        let entry = classify_database("/nonexistent/x.fasta").unwrap();
        assert_eq!(
            (entry.format.as_str(), entry.molecule.as_str()),
            ("fasta", "unknown")
        );
    }

    /// The central registry must survive a round trip; a missing or corrupt file
    /// reads as empty rather than failing the panel.
    #[test]
    fn the_registry_round_trips_and_tolerates_a_missing_file() {
        let root = workspace("round-trip");
        let file = root.join("central").join(REGISTRY_NAME);
        assert!(load_registry_file(&file).databases.is_empty());

        let mut reg = DatabaseRegistry::default();
        reg.upsert(entry("/db/pfam.hmm", 4), &[]);
        save_registry_file(&file, &reg).unwrap();
        let loaded = load_registry_file(&file);
        assert_eq!(loaded.databases.len(), 1);
        assert_eq!(loaded.databases[0].shards, 4);
        assert_eq!(loaded.databases[0].format, "hmm");

        fs::write(&file, b"{ not json").unwrap();
        assert!(
            load_registry_file(&file).databases.is_empty(),
            "corrupt registry reads as empty"
        );
    }

    fn entry(path: &str, shards: usize) -> DatabaseEntry {
        DatabaseEntry {
            path: path.into(),
            name: "x".into(),
            format: "hmm".into(),
            molecule: "protein".into(),
            shards,
        }
    }

    #[test]
    fn upsert_replaces_by_path_and_by_the_superseded_list() {
        let mut reg = DatabaseRegistry::default();
        reg.upsert(entry("/a.hmm", 1), &[]);
        reg.upsert(entry("/old.hmm", 1), &[]);
        reg.upsert(entry("/a.hmm", 2), &["/old.hmm".into()]);
        assert_eq!(reg.databases.len(), 1);
        assert_eq!(reg.databases[0].shards, 2);
        assert!(reg.remove("/a.hmm"));
        assert!(!reg.remove("/a.hmm"));
    }

    /// A workspace's old list is merged once, without duplicating what the
    /// central list already has, and is then retired -- so a database the user
    /// removes centrally does not come back the next time that workspace opens.
    #[test]
    fn a_workspace_registry_is_merged_once_then_retired() {
        let root = workspace("migrate");
        let central = root.join("appdata").join(REGISTRY_NAME);
        let mut c = DatabaseRegistry::default();
        c.upsert(entry("/shared.hmm", 1), &[]);
        save_registry_file(&central, &c).unwrap();

        let mut legacy = DatabaseRegistry::default();
        legacy.upsert(entry("/shared.hmm", 1), &[]);
        legacy.upsert(entry("/only-here.hmm", 1), &[]);
        save_registry_file(&workspace_registry_path(&root), &legacy).unwrap();

        assert_eq!(migrate_workspace_registry(&central, &root).unwrap(), 1);
        let paths: Vec<String> = load_registry_file(&central)
            .databases
            .into_iter()
            .map(|d| d.path)
            .collect();
        assert_eq!(paths, vec!["/shared.hmm", "/only-here.hmm"]);
        assert!(
            !workspace_registry_path(&root).exists(),
            "the legacy list must be retired"
        );
        assert!(
            dot_dir(&root).join(MIGRATED_NAME).exists(),
            "and kept for the record"
        );

        let mut c = load_registry_file(&central);
        c.remove("/only-here.hmm");
        save_registry_file(&central, &c).unwrap();
        assert_eq!(migrate_workspace_registry(&central, &root).unwrap(), 0);
        assert_eq!(
            load_registry_file(&central).databases.len(),
            1,
            "a removed database came back"
        );
    }

    /// Renaming moves every shard and every index together and keeps the shard
    /// numbering; a rename that would change the format, or collide, moves nothing.
    #[test]
    fn renaming_a_sharded_database_moves_its_shards_and_indexes() {
        let root = workspace("rename-shards");
        for n in ["db.0.fasta", "db.1.fasta", "db.0.fasta.didx", "dbx.fasta"] {
            fs::write(root.join(n), ">a\nMK\n").unwrap();
        }
        let base = root.join("db.fasta");
        assert_eq!(database_files(&base).len(), 3);

        let err = rename_database(&base, "nr.gb").unwrap_err();
        assert!(err.contains(".fasta"), "{err}");
        // A collision with any target file refuses the whole rename.
        let root2 = workspace("rename-collide");
        for n in ["db.0.fasta", "db.1.fasta", "nr.1.fasta"] {
            fs::write(root2.join(n), ">a\nMK\n").unwrap();
        }
        let err = rename_database(&root2.join("db.fasta"), "nr.fasta").unwrap_err();
        assert!(err.contains("already exists"), "{err}");
        assert!(
            root2.join("db.0.fasta").exists(),
            "a refused rename must move nothing"
        );

        let root3 = workspace("rename-ok");
        for n in ["db.0.fasta", "db.1.fasta", "db.1.fasta.didx", "dbx.fasta"] {
            fs::write(root3.join(n), ">a\nMK\n").unwrap();
        }
        let new = rename_database(&root3.join("db.fasta"), "nr.fasta").unwrap();
        assert_eq!(new, root3.join("nr.fasta"));
        for n in ["nr.0.fasta", "nr.1.fasta", "nr.1.fasta.didx", "dbx.fasta"] {
            assert!(root3.join(n).exists(), "{n} missing after rename");
        }
        assert!(!root3.join("db.0.fasta").exists());
    }

    /// Built in place, the index reads back as fresh; touching the data makes it
    /// stale; plain gzip and HMMs are reported for what they are, not "missing".
    #[test]
    fn an_index_is_built_in_place_and_goes_stale_when_the_data_changes() {
        let root = workspace("index");
        let db = root.join("prot.fasta");
        fs::write(&db, ">a\nMKV\n>b\nMKL\n").unwrap();
        assert_eq!(index_status(&db, "fasta"), IndexStatus::Missing);

        assert_eq!(build_database_index(&db).unwrap(), 1);
        assert_eq!(index_status(&db, "fasta"), IndexStatus::Indexed);
        assert_eq!(
            database_files(&db).len(),
            2,
            "the index is a sidecar, not a copy"
        );

        fs::write(&db, ">a\nMKV\n>b\nMKL\n>c\nMKA\n").unwrap();
        assert_eq!(index_status(&db, "fasta"), IndexStatus::Stale);

        let gz = root.join("plain.fasta.gz");
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut enc, b">a\nMK\n").unwrap();
        fs::write(&gz, enc.finish().unwrap()).unwrap();
        assert_eq!(index_status(&gz, "fasta"), IndexStatus::Unindexable);
        assert!(build_database_index(&gz).unwrap_err().contains("BGZF"));

        assert_eq!(
            index_status(&root.join("p.hmm"), "hmm"),
            IndexStatus::NotApplicable
        );
    }

    #[test]
    fn deleting_a_database_removes_its_index_but_not_its_neighbors() {
        let root = workspace("delete");
        for n in ["p.hmm", "p.hmm.didx", "p2.hmm"] {
            fs::write(root.join(n), "x").unwrap();
        }
        let removed = delete_database_files(&root.join("p.hmm")).unwrap();
        assert_eq!(removed.len(), 2);
        assert!(!root.join("p.hmm.didx").exists());
        assert!(root.join("p2.hmm").exists());
    }
}
