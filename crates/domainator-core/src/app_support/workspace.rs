//! Workspace model for the GUI: in-place folder browsing with sidecar metadata.
//!
//! The user points the app at a real directory. The folder tree (LEFT panel) is
//! derived by scanning subdirectories, and the file table (MIDDLE panel) lists the
//! recognized data files in a directory. Data files are never rewritten to carry
//! GUI metadata — everything the app writes lives under a single reserved hidden
//! directory, [`DOT_DIR`] (`.domainator/`): the manifest, the database registry,
//! job provenance, per-file metadata sidecars, and cached payloads. This keeps
//! every `.gb`/`.hdf5`/`.hmm` file untouched and Python-readable, and means
//! `.domainator` is the ONLY name the app reserves — no clashes with user files.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The single reserved directory that holds all app machinery for a workspace.
/// Users may not create a file or folder with this name.
pub const DOT_DIR: &str = ".domainator";

/// `<root>/.domainator` (not created here; writers call [`ensure_dot_dir`]).
pub fn dot_dir(root: &Path) -> PathBuf {
    root.join(DOT_DIR)
}

/// Create `<root>/.domainator/<sub>` and return it.
pub fn ensure_dot_subdir(root: &Path, sub: &str) -> std::io::Result<PathBuf> {
    let d = dot_dir(root).join(sub);
    fs::create_dir_all(&d)?;
    Ok(d)
}

/// True if `name` is the reserved machinery directory (rejected for user create/rename).
pub fn is_reserved_name(name: &str) -> bool {
    name.eq_ignore_ascii_case(DOT_DIR)
}

/// Move legacy top-level machinery (`workspace.json`, `databases.json`, `jobs/`)
/// into `.domainator/` for workspaces created before the restructure. Best-effort:
/// ignores individual failures so an old workspace still opens.
pub fn migrate_legacy_layout(root: &Path) {
    let dot = dot_dir(root);
    if dot.exists() {
        return; // already migrated (or a fresh workspace)
    }
    let moves = [("workspace.json", false), ("databases.json", false), ("jobs", true)];
    let any = moves.iter().any(|(n, _)| root.join(n).exists());
    if !any {
        return;
    }
    let _ = fs::create_dir_all(&dot);
    for (name, _is_dir) in moves {
        let src = root.join(name);
        if src.exists() {
            let _ = fs::rename(&src, dot.join(name));
        }
    }
}

/// Logical file type shown in the file table / used to pick viewers. Mirrors the
/// Python `EXTENSION_TO_TYPE` map plus the GUI-only `ssn_viewer` bundle type.
pub fn classify_file(path: &Path) -> Option<&'static str> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    // Strip one trailing compression suffix, matching Python get_file_type.
    let stem = ["gz", "bgz", "bgzf"]
        .iter()
        .find_map(|suf| name.strip_suffix(&format!(".{suf}")))
        .unwrap_or(&name);
    let ext = stem.rsplit_once('.').map(|(_, e)| e)?;
    Some(match ext {
        "gb" | "gbk" | "genbank" | "gbff" => "genbank",
        "fasta" | "fa" | "fna" | "pep" | "faa" => "fasta",
        "hmm" | "hmmer" => "hmm",
        "hdf5" | "hdf" | "h5" => "hdf5",
        "cm" => "cm",
        "sto" | "stockholm" | "sth" => "stockholm",
        "tsv" | "csv" | "tab" => "table",
        "ssnv" | "ssnview" => "ssn_viewer",
        _ => return None,
    })
}

/// GUI metadata for a data file, stored under `<root>/.domainator/meta/` mirroring
/// the file's path relative to the workspace root (so data files stay clean).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileMeta {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Id of the job (`.domainator/jobs/<id>/job.json`) that produced this file.
    #[serde(default)]
    pub created_by_job: Option<String>,
}

impl FileMeta {
    /// Sidecar path: `<root>/.domainator/meta/<rel>.dom.json`, where `<rel>` is the
    /// data file relative to `root` (falling back to the bare file name if the file
    /// lives outside the workspace, e.g. an external database).
    pub fn sidecar_path(root: &Path, data_file: &Path) -> PathBuf {
        let rel = data_file
            .strip_prefix(root)
            .ok()
            .map(|p| p.to_path_buf())
            .or_else(|| data_file.file_name().map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("unknown"));
        let mut s = dot_dir(root).join("meta").join(rel).into_os_string();
        s.push(".dom.json");
        PathBuf::from(s)
    }

    pub fn load(root: &Path, data_file: &Path) -> Self {
        fs::read(Self::sidecar_path(root, data_file))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, root: &Path, data_file: &Path) -> std::io::Result<()> {
        let sidecar = Self::sidecar_path(root, data_file);
        if let Some(parent) = sidecar.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_vec_pretty(self)?;
        write_atomic(&sidecar, &json)
    }

    /// Remove the sidecar for a data file (used when the file is deleted).
    pub fn remove(root: &Path, data_file: &Path) {
        let _ = fs::remove_file(Self::sidecar_path(root, data_file));
    }
}

/// A recognized data file in a folder (one MIDDLE-panel row).
#[derive(Debug, Clone, Serialize)]
pub struct FileEntry {
    /// Absolute path (stable id).
    pub path: String,
    pub name: String,
    /// Logical type from [`classify_file`].
    pub file_type: String,
    pub description: String,
    pub size: u64,
    pub created_by_job: Option<String>,
}

/// A directory node in the LEFT folder tree.
#[derive(Debug, Clone, Serialize)]
pub struct FolderNode {
    pub path: String,
    pub name: String,
    pub children: Vec<FolderNode>,
}

/// Workspace-wide settings persisted in `workspace.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub default_cpu: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { default_cpu: 0 }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkspaceManifest {
    pub schema_version: u32,
    #[serde(default)]
    pub settings: Settings,
}

const MANIFEST_NAME: &str = "workspace.json";
const SCHEMA_VERSION: u32 = 1;

fn manifest_path(root: &Path) -> PathBuf {
    dot_dir(root).join(MANIFEST_NAME)
}

/// Directory names hidden from the folder tree (dotdirs, incl. `.domainator`, plus
/// the legacy top-level `jobs/` dir from pre-restructure workspaces).
fn is_hidden_or_machinery(name: &str) -> bool {
    name.starts_with('.') || name == "jobs"
}

/// Load the workspace manifest, creating a default one on disk if absent. Also
/// migrates any legacy top-level machinery into `.domainator/`.
pub fn open_or_init_manifest(root: &Path) -> std::io::Result<WorkspaceManifest> {
    migrate_legacy_layout(root);
    if let Ok(bytes) = fs::read(manifest_path(root)) {
        if let Ok(m) = serde_json::from_slice::<WorkspaceManifest>(&bytes) {
            return Ok(m);
        }
    }
    let manifest = WorkspaceManifest { schema_version: SCHEMA_VERSION, settings: Settings::default() };
    save_manifest(root, &manifest)?;
    Ok(manifest)
}

pub fn save_manifest(root: &Path, manifest: &WorkspaceManifest) -> std::io::Result<()> {
    fs::create_dir_all(dot_dir(root))?;
    let json = serde_json::to_vec_pretty(manifest)?;
    write_atomic(&manifest_path(root), &json)
}

/// Build the folder tree rooted at `root` by scanning subdirectories (recursive,
/// skipping hidden dirs and the `jobs/` machinery dir). The root node is named
/// after the directory.
pub fn scan_folder_tree(root: &Path) -> std::io::Result<FolderNode> {
    fn walk(dir: &Path) -> std::io::Result<Vec<FolderNode>> {
        let mut children = Vec::new();
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if is_hidden_or_machinery(&name) {
                continue;
            }
            let path = entry.path();
            children.push(FolderNode { path: path.to_string_lossy().into_owned(), name, children: walk(&path)? });
        }
        children.sort_by(|a, b| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()));
        Ok(children)
    }

    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.to_string_lossy().into_owned());
    Ok(FolderNode { path: root.to_string_lossy().into_owned(), name, children: walk(root)? })
}

/// List recognized data files directly inside `dir` (non-recursive), with sidecar
/// metadata (from `<root>/.domainator/meta`) folded in. Unrecognized files skipped.
pub fn list_files(root: &Path, dir: &Path) -> std::io::Result<Vec<FileEntry>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let path = entry.path();
        let Some(file_type) = classify_file(&path) else { continue };
        let name = entry.file_name().to_string_lossy().into_owned();
        let meta = FileMeta::load(root, &path);
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        out.push(FileEntry {
            path: path.to_string_lossy().into_owned(),
            name,
            file_type: file_type.to_string(),
            description: meta.description,
            size,
            created_by_job: meta.created_by_job,
        });
    }
    out.sort_by(|a, b| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()));
    Ok(out)
}

/// Write `bytes` to `path` atomically (temp file in the same dir, then rename).
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp = dir.join(format!(
        ".{}.tmp",
        path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "ws".into())
    ));
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn classifies_extensions_and_compression() {
        assert_eq!(classify_file(Path::new("a.gb")), Some("genbank"));
        assert_eq!(classify_file(Path::new("a.gbff.gz")), Some("genbank"));
        assert_eq!(classify_file(Path::new("a.faa")), Some("fasta"));
        assert_eq!(classify_file(Path::new("a.hmm")), Some("hmm"));
        assert_eq!(classify_file(Path::new("net.ssnv")), Some("ssn_viewer"));
        assert_eq!(classify_file(Path::new("m.hdf5")), Some("hdf5"));
        assert_eq!(classify_file(Path::new("readme.txt")), None);
        assert_eq!(classify_file(Path::new("a.gb.dom.json")), None);
    }

    #[test]
    fn sidecar_path_mirrors_under_dot_dir() {
        assert_eq!(
            FileMeta::sidecar_path(Path::new("/ws"), Path::new("/ws/sub/foo.gb")),
            PathBuf::from("/ws/.domainator/meta/sub/foo.gb.dom.json")
        );
    }

    #[test]
    fn reserved_name() {
        assert!(is_reserved_name(".domainator"));
        assert!(!is_reserved_name("genomes"));
    }
}
