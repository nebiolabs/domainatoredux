//! Application preferences — the small amount of state that belongs to the *app*
//! rather than to any one workspace, and therefore cannot live under a workspace's
//! `.domainator/` directory.
//!
//! Today that is the recent-workspace list, which is what lets the GUI reopen the
//! workspace you were last in instead of starting cold every launch. The caller
//! supplies the file path (the GUI uses Tauri's per-app data directory); this
//! module owns the format and the list semantics so both are testable without a
//! desktop environment.
//!
//! Everything here is best-effort: a missing or unreadable file reads as defaults
//! rather than an error, because losing your recent list must never be something
//! that stops the app from starting.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// How many recent workspaces to remember.
pub const MAX_RECENT: usize = 10;

/// Current on-disk schema. Bump when a field's meaning changes incompatibly.
const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prefs {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// Workspace roots, most recently opened first.
    #[serde(default)]
    pub recent_workspaces: Vec<PathBuf>,
    /// Whether to reopen the most recent workspace on launch.
    #[serde(default = "default_true")]
    pub reopen_last_workspace: bool,
}

fn default_schema_version() -> u32 {
    SCHEMA_VERSION
}

fn default_true() -> bool {
    true
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            recent_workspaces: Vec::new(),
            reopen_last_workspace: true,
        }
    }
}

impl Prefs {
    /// Record `root` as the most recently opened workspace.
    ///
    /// The list is a most-recent-first set: an existing entry is moved to the
    /// front rather than duplicated, and the tail past [`MAX_RECENT`] is dropped.
    pub fn push_recent(&mut self, root: impl AsRef<Path>) {
        let root = root.as_ref().to_path_buf();
        self.recent_workspaces.retain(|p| p != &root);
        self.recent_workspaces.insert(0, root);
        self.recent_workspaces.truncate(MAX_RECENT);
    }

    /// Forget one entry (the user removed it from the welcome list).
    pub fn forget_recent(&mut self, root: impl AsRef<Path>) {
        let root = root.as_ref();
        self.recent_workspaces.retain(|p| p != root);
    }

    /// The recent entries that still exist on disk, most recent first.
    ///
    /// Filtering on read rather than pruning on write is deliberate: a workspace on
    /// a network share or an unplugged external drive is *missing*, not *gone*, and
    /// deleting it from the list the one time you launched without that drive
    /// mounted would be the wrong call.
    pub fn existing_recent(&self) -> Vec<PathBuf> {
        self.recent_workspaces.iter().filter(|p| p.is_dir()).cloned().collect()
    }

    /// The workspace to reopen at launch, if any.
    pub fn workspace_to_reopen(&self) -> Option<PathBuf> {
        if !self.reopen_last_workspace {
            return None;
        }
        self.existing_recent().into_iter().next()
    }
}

/// Read the preferences at `path`. A missing, unreadable or malformed file yields
/// defaults — see the module note on best-effort behaviour.
pub fn load_prefs(path: &Path) -> Prefs {
    fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Write the preferences to `path`, creating the parent directory if needed.
pub fn save_prefs(path: &Path, prefs: &Prefs) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_vec_pretty(prefs)?;
    write_atomic(path, &json)
}

/// Load, mutate, and save in one step. Returns the saved value.
pub fn update_prefs(path: &Path, f: impl FnOnce(&mut Prefs)) -> std::io::Result<Prefs> {
    let mut prefs = load_prefs(path);
    f(&mut prefs);
    prefs.schema_version = SCHEMA_VERSION;
    save_prefs(path, &prefs)?;
    Ok(prefs)
}

/// Write `bytes` to `path` atomically (temp file in the same dir, then rename), so
/// a crash mid-write can't leave a truncated file that reads back as defaults.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp = dir.join(format!(
        ".{}.tmp",
        path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "prefs".into())
    ));
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("domainator-prefs-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn a_missing_file_reads_as_defaults() {
        let p = scratch("missing").join("prefs.json");
        let prefs = load_prefs(&p);
        assert!(prefs.recent_workspaces.is_empty());
        assert!(prefs.reopen_last_workspace);
    }

    /// A truncated or hand-mangled prefs file must not stop the app from starting.
    #[test]
    fn a_corrupt_file_reads_as_defaults() {
        let dir = scratch("corrupt");
        let p = dir.join("prefs.json");
        fs::write(&p, b"{not json").unwrap();
        assert!(load_prefs(&p).recent_workspaces.is_empty());
    }

    #[test]
    fn round_trips_through_the_file() {
        let dir = scratch("roundtrip");
        let p = dir.join("prefs.json");
        let mut prefs = Prefs::default();
        prefs.push_recent("/data/one");
        prefs.push_recent("/data/two");
        save_prefs(&p, &prefs).unwrap();
        assert_eq!(
            load_prefs(&p).recent_workspaces,
            vec![PathBuf::from("/data/two"), PathBuf::from("/data/one")]
        );
    }

    /// Reopening a workspace must move it to the front, not add a second copy —
    /// otherwise the list fills up with one directory.
    #[test]
    fn push_recent_moves_an_existing_entry_to_the_front() {
        let mut prefs = Prefs::default();
        prefs.push_recent("/a");
        prefs.push_recent("/b");
        prefs.push_recent("/a");
        assert_eq!(
            prefs.recent_workspaces,
            vec![PathBuf::from("/a"), PathBuf::from("/b")]
        );
    }

    #[test]
    fn push_recent_caps_the_list() {
        let mut prefs = Prefs::default();
        for i in 0..MAX_RECENT + 5 {
            prefs.push_recent(format!("/ws/{i}"));
        }
        assert_eq!(prefs.recent_workspaces.len(), MAX_RECENT);
        // Newest first: the last one pushed leads.
        assert_eq!(
            prefs.recent_workspaces[0],
            PathBuf::from(format!("/ws/{}", MAX_RECENT + 4))
        );
    }

    #[test]
    fn forget_recent_removes_one_entry() {
        let mut prefs = Prefs::default();
        prefs.push_recent("/a");
        prefs.push_recent("/b");
        prefs.forget_recent("/a");
        assert_eq!(prefs.recent_workspaces, vec![PathBuf::from("/b")]);
    }

    /// A directory that has since been deleted or unmounted is filtered out of the
    /// offered list, but stays in the file so it comes back when it reappears.
    #[test]
    fn existing_recent_skips_directories_that_are_gone() {
        let dir = scratch("existing");
        let live = dir.join("live");
        fs::create_dir_all(&live).unwrap();
        let mut prefs = Prefs::default();
        prefs.push_recent(dir.join("deleted"));
        prefs.push_recent(&live);
        assert_eq!(prefs.existing_recent(), vec![live.clone()]);
        assert_eq!(prefs.recent_workspaces.len(), 2);
        assert_eq!(prefs.workspace_to_reopen(), Some(live));
    }

    #[test]
    fn reopen_can_be_turned_off() {
        let dir = scratch("reopen-off");
        let live = dir.join("live");
        fs::create_dir_all(&live).unwrap();
        let mut prefs = Prefs::default();
        prefs.push_recent(&live);
        prefs.reopen_last_workspace = false;
        assert_eq!(prefs.workspace_to_reopen(), None);
    }

    #[test]
    fn update_prefs_reads_modifies_and_writes() {
        let dir = scratch("update");
        let p = dir.join("prefs.json");
        update_prefs(&p, |x| x.push_recent("/one")).unwrap();
        let after = update_prefs(&p, |x| x.push_recent("/two")).unwrap();
        assert_eq!(
            after.recent_workspaces,
            vec![PathBuf::from("/two"), PathBuf::from("/one")]
        );
        assert_eq!(after.schema_version, SCHEMA_VERSION);
        assert_eq!(load_prefs(&p).recent_workspaces, after.recent_workspaces);
    }
}
