//! Workspace model for the GUI: in-place folder browsing with sidecar metadata.
//!
//! The user points the app at a real directory. The folder tree (LEFT panel) is
//! derived by scanning subdirectories, and the file table (MIDDLE panel) lists the
//! recognized data files in a directory. Data files are never rewritten to carry
//! GUI metadata — per-file description/tags/provenance live in a sibling
//! `<name>.dom.json`, and workspace-wide settings live in `workspace.json` at the
//! root. This keeps every `.gb`/`.hdf5`/`.hmm` file untouched and Python-readable.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

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
        "ssnv" | "ssnview" => "ssn_viewer",
        _ => return None,
    })
}

/// GUI metadata stored beside a data file as `<name>.dom.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileMeta {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Id of the job (`jobs/<id>/job.json`) that produced this file, if any.
    #[serde(default)]
    pub created_by_job: Option<String>,
}

impl FileMeta {
    /// Sidecar path for a data file: `foo.gb` -> `foo.gb.dom.json`.
    pub fn sidecar_path(data_file: &Path) -> PathBuf {
        let mut s = data_file.as_os_str().to_os_string();
        s.push(".dom.json");
        PathBuf::from(s)
    }

    pub fn load(data_file: &Path) -> Self {
        let sidecar = Self::sidecar_path(data_file);
        fs::read(&sidecar)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, data_file: &Path) -> std::io::Result<()> {
        let sidecar = Self::sidecar_path(data_file);
        let json = serde_json::to_vec_pretty(self)?;
        write_atomic(&sidecar, &json)
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

/// Directory names that are workspace machinery, not user folders.
fn is_hidden_or_machinery(name: &str) -> bool {
    name.starts_with('.') || name == "jobs"
}

/// Load the workspace manifest, creating a default one on disk if absent.
pub fn open_or_init_manifest(root: &Path) -> std::io::Result<WorkspaceManifest> {
    let path = root.join(MANIFEST_NAME);
    if let Ok(bytes) = fs::read(&path) {
        if let Ok(m) = serde_json::from_slice::<WorkspaceManifest>(&bytes) {
            return Ok(m);
        }
    }
    let manifest = WorkspaceManifest { schema_version: SCHEMA_VERSION, settings: Settings::default() };
    save_manifest(root, &manifest)?;
    Ok(manifest)
}

pub fn save_manifest(root: &Path, manifest: &WorkspaceManifest) -> std::io::Result<()> {
    let json = serde_json::to_vec_pretty(manifest)?;
    write_atomic(&root.join(MANIFEST_NAME), &json)
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
/// metadata folded in. Sidecar `.dom.json` files and unrecognized files are skipped.
pub fn list_files(dir: &Path) -> std::io::Result<Vec<FileEntry>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let path = entry.path();
        let Some(file_type) = classify_file(&path) else { continue };
        let name = entry.file_name().to_string_lossy().into_owned();
        let meta = FileMeta::load(&path);
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
    fn sidecar_path_appends() {
        assert_eq!(
            FileMeta::sidecar_path(Path::new("/x/foo.gb")),
            PathBuf::from("/x/foo.gb.dom.json")
        );
    }
}
