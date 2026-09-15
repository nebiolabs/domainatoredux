//! Workspace-global database registry (the "Databases" panel).
//!
//! Databases are formatted sequence/HMM sets shared across all folders. They are
//! tracked in `databases.json` at the workspace root (separate from the folder
//! tree). Each entry records its `format` (fasta | genbank | hmmer) and `molecule`
//! (protein | nucleic), so the UI can show the six categories from the spec
//! (protein/nucleic × fasta/genbank/hmmer), plus the resolved shard count.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::app_support::workspace::{classify_file, dot_dir};
use crate::io::shards::resolve_database_shards;
use crate::io::{detect_filetype, FileType};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseEntry {
    /// Logical database path (a single file or a shard base like `db.gb`).
    pub path: String,
    pub name: String,
    /// `fasta` | `genbank` | `hmm`.
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

const REGISTRY_NAME: &str = "databases.json";

fn registry_path(root: &Path) -> std::path::PathBuf {
    dot_dir(root).join(REGISTRY_NAME)
}

pub fn load_registry(root: &Path) -> DatabaseRegistry {
    fs::read(registry_path(root))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_registry(root: &Path, reg: &DatabaseRegistry) -> std::io::Result<()> {
    fs::create_dir_all(dot_dir(root))?;
    let json = serde_json::to_vec_pretty(reg)?;
    fs::write(registry_path(root), json)
}

/// Sniff whether a residue string looks like nucleic acid (mostly A/C/G/T/U/N).
fn looks_nucleic(seq: &[u8]) -> bool {
    let mut n = 0usize;
    let mut total = 0usize;
    for &b in seq.iter().take(2000) {
        if b.is_ascii_alphabetic() {
            total += 1;
            if matches!(b.to_ascii_uppercase(), b'A' | b'C' | b'G' | b'T' | b'U' | b'N') {
                n += 1;
            }
        }
    }
    total > 0 && (n as f64 / total as f64) > 0.9
}

/// First HMM's alphabet: `protein` (amino) or `nucleic` (dna/rna).
fn hmm_molecule(path: &str) -> String {
    use hmmer_pure_rs::alphabet::AlphabetType;
    match crate::search::hmm_read::read_hmm_file(Path::new(path)).ok().and_then(|h| h.into_iter().next()) {
        Some(h) => match h.abc_type {
            AlphabetType::Dna | AlphabetType::Rna => "nucleic".into(),
            AlphabetType::Amino => "protein".into(),
            _ => "unknown".into(),
        },
        None => "unknown".into(),
    }
}

/// Classify a database path into a [`DatabaseEntry`] (reads the first shard's
/// first record to determine molecule type).
pub fn classify_database(path: &str) -> Result<DatabaseEntry, String> {
    let p = Path::new(path);
    let name = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string());

    // hmm files aren't handled by detect_filetype (which is genbank/fasta only),
    // so classify format from the extension first.
    let format = classify_file(p).ok_or_else(|| format!("unrecognized database type: {path}"))?;

    let shards = resolve_database_shards(p).unwrap_or_else(|_| vec![p.to_path_buf()]);
    let first = shards.first().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string());

    let (format, molecule) = match format {
        "hmm" => ("hmm".to_string(), hmm_molecule(&first)),
        "genbank" => {
            let mol = detect_filetype(Path::new(&first))
                .filter(|k| *k == FileType::Genbank)
                .and_then(|_| crate::io::genbank::read_genbank(&first).ok())
                .and_then(|recs| recs.into_iter().next())
                .map(|r| if r.is_protein() { "protein".to_string() } else { "nucleic".to_string() })
                .unwrap_or_else(|| "unknown".into());
            ("genbank".to_string(), mol)
        }
        "fasta" => {
            let mol = crate::io::fasta::read_fasta(&first, None)
                .ok()
                .and_then(|recs| recs.into_iter().next())
                .map(|r| if looks_nucleic(&r.seq) { "nucleic".to_string() } else { "protein".to_string() })
                .unwrap_or_else(|| "unknown".into());
            ("fasta".to_string(), mol)
        }
        other => return Err(format!("{other} files are not databases")),
    };

    Ok(DatabaseEntry { path: path.to_string(), name, format, molecule, shards: shards.len() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(name: &str) -> std::path::PathBuf {
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
        assert_eq!((entry.format.as_str(), entry.molecule.as_str()), ("fasta", "nucleic"));
        assert_eq!(entry.name, "d");
        assert_eq!(entry.shards, 1);

        let prot = write(&root, "p.faa", ">a\nMKVLAWIYPQRSTHDEF\n");
        let entry = classify_database(&prot).unwrap();
        assert_eq!((entry.format.as_str(), entry.molecule.as_str()), ("fasta", "protein"));
    }

    /// Two different refusals, both of which must be errors rather than a guess:
    /// an extension nothing recognizes, and one that is recognized but is not a
    /// sequence database (a matrix, a colour table, a notes file).
    #[test]
    fn non_databases_are_refused_rather_than_guessed() {
        let root = workspace("unknown");
        let mystery = write(&root, "db.zzz", "hello");
        assert!(
            classify_database(&mystery).unwrap_err().contains("unrecognized database type"),
            "an unknown extension should say so"
        );

        for (name, kind) in [("notes.txt", "text"), ("d.hdf5", "hdf5"), ("c.tsv", "table")] {
            let p = write(&root, name, "x");
            let err = classify_database(&p).unwrap_err();
            assert!(err.contains(kind) && err.contains("not databases"), "{name}: {err}");
        }

        // A recognized extension that does not exist on disk still classifies --
        // the molecule is simply unknown. It must not panic.
        let entry = classify_database("/nonexistent/x.fasta").unwrap();
        assert_eq!((entry.format.as_str(), entry.molecule.as_str()), ("fasta", "unknown"));
    }

    /// The registry lives at the workspace root and must survive a round trip;
    /// a missing or corrupt file reads as empty rather than failing the panel.
    #[test]
    fn the_registry_round_trips_and_tolerates_a_missing_file() {
        let root = workspace("round-trip");
        assert!(load_registry(&root).databases.is_empty());

        let reg = DatabaseRegistry {
            databases: vec![DatabaseEntry {
                path: "/db/pfam.hmm".into(),
                name: "pfam".into(),
                format: "hmm".into(),
                molecule: "protein".into(),
                shards: 4,
            }],
        };
        save_registry(&root, &reg).unwrap();
        let loaded = load_registry(&root);
        assert_eq!(loaded.databases.len(), 1);
        assert_eq!(loaded.databases[0].shards, 4);
        assert_eq!(loaded.databases[0].format, "hmm");

        fs::write(registry_path(&root), b"{ not json").unwrap();
        assert!(load_registry(&root).databases.is_empty(), "corrupt registry reads as empty");
    }
}
