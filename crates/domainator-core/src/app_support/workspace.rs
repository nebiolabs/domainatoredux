//! Workspace model for the GUI: in-place folder browsing with sidecar metadata.
//!
//! The user points the app at a real directory. The folder tree (LEFT panel) is
//! derived by scanning subdirectories, and the file table (MIDDLE panel) lists the
//! recognized data files in a directory. Data files are never rewritten to carry
//! GUI metadata — everything the app writes lives under a single reserved hidden
//! directory, [`DOT_DIR`] (`.domainatoredux/`): the manifest, the database registry,
//! job provenance, per-file metadata sidecars, and cached payloads. This keeps
//! every `.gb`/`.hdf5`/`.hmm` file untouched and Python-readable, and means
//! `.domainatoredux` is the ONLY name the app reserves — no clashes with user files.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The single reserved directory that holds all app machinery for a workspace.
/// Users may not create a file or folder with this name.
pub const DOT_DIR: &str = ".domainatoredux";

/// `<root>/.domainatoredux` (not created here; writers call [`ensure_dot_subdir`]).
pub fn dot_dir(root: &Path) -> PathBuf {
    root.join(DOT_DIR)
}

/// Create `<root>/.domainatoredux/<sub>` and return it.
pub fn ensure_dot_subdir(root: &Path, sub: &str) -> std::io::Result<PathBuf> {
    let d = dot_dir(root).join(sub);
    fs::create_dir_all(&d)?;
    Ok(d)
}

/// True if `name` is the reserved machinery directory (rejected for user create/rename).
pub fn is_reserved_name(name: &str) -> bool {
    name.eq_ignore_ascii_case(DOT_DIR)
}

/// Find the workspace root containing `data_file` by walking up its ancestors for
/// the first directory that holds a `.domainatoredux/` machinery dir. Lets provenance
/// be located from a file path alone (e.g. in a detached window that doesn't know
/// the workspace root). Returns `None` if the file isn't inside a workspace.
/// Whether `candidate` lies inside `root`.
///
/// Both sides are canonicalized first, so `..` segments and symlinks cannot walk
/// out of the workspace and then appear to be inside it. Comparison is
/// **component-wise**, not string-prefix: `/data/workspace2` is not inside
/// `/data/workspace`, though its path string starts with it.
///
/// Used to bound "reveal this in the file manager" — the one place the GUI hands
/// a user-supplied path to the OS.
pub fn is_within(root: &Path, candidate: &Path) -> bool {
    // A path that cannot be canonicalized (deleted, or a broken symlink) is not
    // something to reveal, so failing closed is the right answer.
    let (Ok(root), Ok(candidate)) = (root.canonicalize(), candidate.canonicalize()) else {
        return false;
    };
    candidate.starts_with(&root)
}

pub fn find_workspace_root(data_file: &Path) -> Option<PathBuf> {
    let mut cur = data_file.parent();
    while let Some(dir) = cur {
        if dir.join(DOT_DIR).is_dir() {
            return Some(dir.to_path_buf());
        }
        cur = dir.parent();
    }
    None
}

/// Move legacy top-level machinery (`workspace.json`, `databases.json`, `jobs/`)
/// into `.domainatoredux/` for workspaces created before the restructure. Best-effort:
/// ignores individual failures so an old workspace still opens.
pub fn migrate_legacy_layout(root: &Path) {
    let dot = dot_dir(root);
    if dot.exists() {
        return; // already migrated (or a fresh workspace)
    }
    let moves = [
        ("workspace.json", false),
        ("databases.json", false),
        ("jobs", true),
    ];
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
/// Python `EXTENSION_TO_TYPE` map plus the GUI-only types: `viewer_bundle` (Domainator
/// Viewer bundles), `alignment` (aligned FASTA) and `newick` (trees).
/// `SystemTime` → epoch milliseconds (None if before the epoch / unavailable).
fn sys_time_ms(t: Option<std::time::SystemTime>) -> Option<u64> {
    t?.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as u64)
}

/// Cheap record count for a data file, by streaming lines (no full parse):
/// GenBank `LOCUS` lines, FASTA `>` headers, HMMER `HMMER3` profile headers,
/// through gzip or BGZF when the file is compressed.
/// Returns None for types without a meaningful record count (matrices, tables, …)
/// or on read error.
pub fn count_records(path: &Path) -> Option<u64> {
    use std::io::{BufRead, BufReader};
    let prefix = match classify_file(path)? {
        "genbank" => "LOCUS",
        "fasta" | "alignment" => ">",
        "hmm" => "HMMER3",
        _ => return None,
    };
    // Count the text, not the container: a `.gz` is decompressed by content.
    let file = crate::io::open_decompressed(path).ok()?;
    let mut reader = BufReader::new(file);
    let mut n: u64 = 0;
    let mut line = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) => break,
            Ok(_) => {
                if line.starts_with(prefix.as_bytes()) {
                    n += 1;
                }
            }
            Err(_) => return None,
        }
    }
    Some(n)
}

/// Human-readable list of the extensions [`classify_file`] recognizes, for error
/// messages when a user names a new/renamed file with an unrecognized type.
pub fn accepted_types_hint() -> &'static str {
    "Accepted: .gb/.gbk/.genbank/.gbff, .fasta/.fa/.fna/.faa/.pep, .afa (aligned FASTA), \
     .hmm, .hdf5/.h5, .cm, .sto/.stockholm, .tsv/.csv/.tab, .txt, .svg, .nwk/.newick, \
     .dvb/.dsnv (optionally with a .gz suffix)."
}

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
        "txt" | "text" | "list" => "text",
        "svg" => "svg",
        "afa" | "afasta" => "alignment",
        "nwk" | "newick" => "newick",
        // `.dsnv` is the older network bundle, and `.ssnv` its pre-0.11 spelling; both
        // are still read, never written.
        "dvb" | "dsnv" | "ssnv" | "ssnview" => "viewer_bundle",
        _ => return None,
    })
}

/// GUI metadata for a data file, stored under `<root>/.domainatoredux/meta/` mirroring
/// the file's path relative to the workspace root (so data files stay clean).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileMeta {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Id of the job (`.domainatoredux/jobs/<id>/job.json`) that produced this file.
    #[serde(default)]
    pub created_by_job: Option<String>,
    /// The GUI's Enum Report tab as last left for this file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enum_report: Option<EnumReportView>,
}

/// What the Enum Report tab remembers for one file: the granularity last shown
/// (`contig` | `cds` | `domain`) and, per granularity, the columns last applied.
/// Columns are `--columns` tokens (`qualifier:CDS:product`), kept as text so this
/// module needs no knowledge of them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EnumReportView {
    #[serde(default)]
    pub by: Option<String>,
    #[serde(default)]
    pub columns: std::collections::BTreeMap<String, Vec<String>>,
}

impl FileMeta {
    /// Sidecar path: `<root>/.domainatoredux/meta/<rel>.dom.json`, where `<rel>` is the
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

    /// Carry a data file's sidecar (if any) along after the file moved from `old`
    /// to `new`. Best-effort: losing a description is not worth failing a move for.
    pub fn follow(root: &Path, old: &Path, new: &Path) {
        let old_sc = Self::sidecar_path(root, old);
        if !old_sc.is_file() {
            return;
        }
        let new_sc = Self::sidecar_path(root, new);
        if let Some(parent) = new_sc.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::rename(&old_sc, &new_sc);
    }
}

/// True if `path` is (or is inside) the reserved machinery directory.
pub fn is_in_dot_dir(root: &Path, path: &Path) -> bool {
    path.starts_with(dot_dir(root)) || path.components().any(|c| c.as_os_str() == DOT_DIR)
}

/// Move `src` to `dest`: a rename, or copy + remove when the two are on different
/// filesystems (a workspace subfolder can be a mount). The copy keeps the source's
/// mtime, because a `.didx` index judges staleness by size + mtime.
fn move_path(src: &Path, dest: &Path) -> std::io::Result<()> {
    if fs::rename(src, dest).is_ok() {
        return Ok(());
    }
    let modified = fs::metadata(src)?.modified()?;
    fs::copy(src, dest)?;
    let copied = fs::OpenOptions::new()
        .write(true)
        .open(dest)
        .and_then(|f| f.set_modified(modified));
    if let Err(e) = copied.and_then(|()| fs::remove_file(src)) {
        let _ = fs::remove_file(dest);
        return Err(e);
    }
    Ok(())
}

/// Move data file `src` into workspace folder `dest_dir`, keeping its name, and
/// bring along what belongs to it: the metadata sidecar under `.domainatoredux/meta/`
/// and a `<name>.didx` index beside it (the file table never lists the index, so
/// leaving it behind would strand it invisibly). Returns the new path; moving a
/// file into the folder it is already in returns `src` unchanged.
///
/// Never overwrites: a name already taken in `dest_dir` (by the file or by its
/// index) is an error and nothing moves.
pub fn move_data_file(root: &Path, src: &Path, dest_dir: &Path) -> std::io::Result<PathBuf> {
    use std::io::{Error, ErrorKind};
    let invalid = |msg: String| Error::new(ErrorKind::InvalidInput, msg);
    if is_in_dot_dir(root, src) || is_in_dot_dir(root, dest_dir) {
        return Err(invalid(format!(
            "refusing to move into or out of the {DOT_DIR} directory"
        )));
    }
    if !dest_dir.is_dir() {
        return Err(invalid(format!("not a folder: {}", dest_dir.display())));
    }
    if !is_within(root, dest_dir) {
        return Err(invalid(format!(
            "not inside the workspace: {}",
            dest_dir.display()
        )));
    }
    if !src.is_file() {
        return Err(invalid(format!("not a file: {}", src.display())));
    }
    let name = src
        .file_name()
        .ok_or_else(|| invalid(format!("not a file: {}", src.display())))?;
    if let (Ok(from), Ok(to)) = (
        src.parent().unwrap_or(src).canonicalize(),
        dest_dir.canonicalize(),
    ) {
        if from == to {
            return Ok(src.to_path_buf());
        }
    }
    let dest = dest_dir.join(name);
    let src_index = crate::io::didx::index_path_for(src);
    let dest_index = crate::io::didx::index_path_for(&dest);
    let has_index = src_index.is_file();
    if dest.exists() || (has_index && dest_index.exists()) {
        return Err(Error::new(
            ErrorKind::AlreadyExists,
            format!(
                "{} already exists in {}",
                name.to_string_lossy(),
                dest_dir.display()
            ),
        ));
    }
    move_path(src, &dest)?;
    if has_index {
        // The data file already moved; an index that cannot follow is only a
        // stale-index rebuild later, not a reason to report the move as failed.
        let _ = move_path(&src_index, &dest_index);
    }
    FileMeta::follow(root, src, &dest);
    Ok(dest)
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
    /// File creation / last-modified time, epoch millis (None if unavailable).
    pub created_ms: Option<u64>,
    pub modified_ms: Option<u64>,
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    /// Worker threads for jobs launched from the GUI; `0` (the default) = all cores.
    #[serde(default)]
    pub default_cpu: usize,
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

/// Directory names hidden from the folder tree (dotdirs, incl. `.domainatoredux`, plus
/// the legacy top-level `jobs/` dir from pre-restructure workspaces).
fn is_hidden_or_machinery(name: &str) -> bool {
    name.starts_with('.') || name == "jobs"
}

/// Load the workspace manifest, creating a default one on disk if absent. Also
/// migrates any legacy top-level machinery into `.domainatoredux/`.
pub fn open_or_init_manifest(root: &Path) -> std::io::Result<WorkspaceManifest> {
    migrate_legacy_layout(root);
    if let Ok(bytes) = fs::read(manifest_path(root)) {
        if let Ok(m) = serde_json::from_slice::<WorkspaceManifest>(&bytes) {
            return Ok(m);
        }
    }
    let manifest = WorkspaceManifest {
        schema_version: SCHEMA_VERSION,
        settings: Settings::default(),
    };
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
            children.push(FolderNode {
                path: path.to_string_lossy().into_owned(),
                name,
                children: walk(&path)?,
            });
        }
        children.sort_by_cached_key(|c| c.name.to_ascii_lowercase());
        Ok(children)
    }

    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.to_string_lossy().into_owned());
    Ok(FolderNode {
        path: root.to_string_lossy().into_owned(),
        name,
        children: walk(root)?,
    })
}

/// List recognized data files directly inside `dir` (non-recursive), with sidecar
/// metadata (from `<root>/.domainatoredux/meta`) folded in. Unrecognized files skipped.
pub fn list_files(root: &Path, dir: &Path) -> std::io::Result<Vec<FileEntry>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let path = entry.path();
        let Some(file_type) = classify_file(&path) else {
            continue;
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        let meta = FileMeta::load(root, &path);
        let md = entry.metadata().ok();
        let size = md.as_ref().map(|m| m.len()).unwrap_or(0);
        let created_ms = md.as_ref().and_then(|m| sys_time_ms(m.created().ok()));
        let modified_ms = md.as_ref().and_then(|m| sys_time_ms(m.modified().ok()));
        out.push(FileEntry {
            path: path.to_string_lossy().into_owned(),
            name,
            file_type: file_type.to_string(),
            description: meta.description,
            size,
            created_ms,
            modified_ms,
            created_by_job: meta.created_by_job,
        });
    }
    out.sort_by_cached_key(|e| e.name.to_ascii_lowercase());
    Ok(out)
}

/// Write `bytes` to `path` atomically (temp file in the same dir, then rename).
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp = dir.join(format!(
        ".{}.tmp",
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "ws".into())
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
        assert_eq!(classify_file(Path::new("net.dvb")), Some("viewer_bundle"));
        // The older network bundle, and its pre-0.11 spelling, still open.
        assert_eq!(classify_file(Path::new("net.dsnv")), Some("viewer_bundle"));
        assert_eq!(classify_file(Path::new("net.ssnv")), Some("viewer_bundle"));
        assert_eq!(classify_file(Path::new("msa.afa")), Some("alignment"));
        assert_eq!(classify_file(Path::new("tree.nwk")), Some("newick"));
        assert_eq!(classify_file(Path::new("m.hdf5")), Some("hdf5"));
        assert_eq!(classify_file(Path::new("readme.txt")), Some("text"));
        assert_eq!(classify_file(Path::new("logo.svg")), Some("svg"));
        assert_eq!(classify_file(Path::new("notes.md")), None);
        assert_eq!(classify_file(Path::new("a.gb.dom.json")), None);
    }

    /// The sibling-with-a-shared-prefix case is the one a string `starts_with`
    /// on the *path text* gets wrong, and it is the difference between bounding a
    /// reveal to the workspace and not bounding it at all.
    #[test]
    fn is_within_bounds_a_path_to_its_workspace() {
        let base = std::env::temp_dir().join(format!("domainator-within-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let root = base.join("workspace");
        let nested = root.join("sub");
        let sibling = base.join("workspace2");
        fs::create_dir_all(&nested).unwrap();
        fs::create_dir_all(&sibling).unwrap();
        let file = nested.join("f.gb");
        fs::write(&file, b"x").unwrap();

        assert!(is_within(&root, &root), "a root contains itself");
        assert!(is_within(&root, &nested));
        assert!(is_within(&root, &file));
        assert!(
            !is_within(&root, &sibling),
            "shared name prefix is not containment"
        );
        assert!(
            !is_within(&root, &base),
            "the parent is not inside the child"
        );
        // `..` must not be able to escape and come back looking legitimate.
        assert!(!is_within(&root, &root.join("..").join("workspace2")));
        // Nonexistent paths fail closed.
        assert!(!is_within(&root, &root.join("nope").join("gone.gb")));

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn sidecar_path_mirrors_under_dot_dir() {
        assert_eq!(
            FileMeta::sidecar_path(Path::new("/ws"), Path::new("/ws/sub/foo.gb")),
            PathBuf::from("/ws/.domainatoredux/meta/sub/foo.gb.dom.json")
        );
    }

    fn move_workspace(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("domainator-move-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("a")).unwrap();
        fs::create_dir_all(root.join("b")).unwrap();
        root
    }

    /// The index and sidecar are invisible in the file table, so a move that
    /// leaves them behind looks complete and silently loses both.
    #[test]
    fn move_brings_the_sidecar_and_index_along() {
        let root = move_workspace("along");
        let src = root.join("a").join("x.gb");
        fs::write(&src, b"LOCUS").unwrap();
        fs::write(crate::io::didx::index_path_for(&src), b"idx").unwrap();
        let view = EnumReportView {
            by: Some("domain".into()),
            columns: [("domain".to_string(), vec!["score".to_string()])].into(),
        };
        FileMeta {
            description: "kept".into(),
            enum_report: Some(view.clone()),
            ..Default::default()
        }
        .save(&root, &src)
        .unwrap();
        let mtime = fs::metadata(&src).unwrap().modified().unwrap();

        let dest = move_data_file(&root, &src, &root.join("b")).unwrap();
        assert_eq!(dest, root.join("b").join("x.gb"));
        assert!(!src.exists());
        assert_eq!(fs::read(&dest).unwrap(), b"LOCUS");
        assert_eq!(
            fs::metadata(&dest).unwrap().modified().unwrap(),
            mtime,
            "the index's staleness check reads mtime"
        );
        assert!(crate::io::didx::index_path_for(&dest).is_file());
        assert!(!crate::io::didx::index_path_for(&src).exists());
        assert_eq!(FileMeta::load(&root, &dest).description, "kept");
        assert_eq!(
            FileMeta::load(&root, &dest).enum_report,
            Some(view),
            "the Enum Report columns too"
        );
        assert!(!FileMeta::sidecar_path(&root, &src).exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn move_never_overwrites() {
        let root = move_workspace("clash");
        let src = root.join("a").join("x.gb");
        fs::write(&src, b"new").unwrap();
        fs::write(root.join("b").join("x.gb"), b"old").unwrap();
        let err = move_data_file(&root, &src, &root.join("b")).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&src).unwrap(), b"new", "the source stays put");
        assert_eq!(fs::read(root.join("b").join("x.gb")).unwrap(), b"old");

        // A clash on the index alone refuses too, rather than clobbering it.
        let other = root.join("a").join("y.gb");
        fs::write(&other, b"y").unwrap();
        fs::write(crate::io::didx::index_path_for(&other), b"idx").unwrap();
        fs::write(root.join("b").join("y.gb.didx"), b"someone else's").unwrap();
        assert!(move_data_file(&root, &other, &root.join("b")).is_err());
        assert!(other.is_file());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn move_stays_inside_the_workspace() {
        let root = move_workspace("bounds");
        let src = root.join("a").join("x.gb");
        fs::write(&src, b"x").unwrap();
        fs::create_dir_all(dot_dir(&root).join("meta")).unwrap();
        assert!(move_data_file(&root, &src, &dot_dir(&root).join("meta")).is_err());
        let outside =
            std::env::temp_dir().join(format!("domainator-move-{}-outside", std::process::id()));
        fs::create_dir_all(&outside).unwrap();
        assert!(move_data_file(&root, &src, &outside).is_err());
        assert!(move_data_file(&root, &root.join("a").join("gone.gb"), &root.join("b")).is_err());
        assert!(src.is_file());

        // Into its own folder is a no-op, not an "already exists" error.
        assert_eq!(move_data_file(&root, &src, &root.join("a")).unwrap(), src);
        assert!(src.is_file());
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&outside);
    }

    #[test]
    fn reserved_name() {
        assert!(is_reserved_name(".domainatoredux"));
        assert!(!is_reserved_name("genomes"));
    }
}
