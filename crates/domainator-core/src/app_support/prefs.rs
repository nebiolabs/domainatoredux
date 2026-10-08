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

/// How many Tools-menu jobs may run at once unless the user says otherwise. Each
/// job is its own worker process and defaults to every core, so more than this
/// mostly makes them slower together; two keeps a quick job from waiting behind
/// a long one.
pub const DEFAULT_MAX_CONCURRENT_JOBS: usize = 2;

/// Current on-disk schema. Bump when a field's meaning changes incompatibly.
const SCHEMA_VERSION: u32 = 1;

/// Which color scheme to use. `System` follows `prefers-color-scheme`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

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
    /// Color scheme.
    #[serde(default)]
    pub theme: Theme,
    /// Default thread count for tools that accept one. `0` means all cores.
    ///
    /// App-level rather than per-workspace on purpose: it is a property of the
    /// *machine*. A workspace copied from a 64-core node to a laptop must not
    /// bring `default_cpu = 64` with it.
    #[serde(default)]
    pub default_cpu: usize,
    /// How many Tools-menu jobs run at once; the rest wait in a queue. `0` reads
    /// as `1`.
    #[serde(default = "default_max_concurrent_jobs")]
    pub max_concurrent_jobs: usize,
    /// Whether to check for a newer release at launch.
    ///
    /// Defaults to **off**: a lab machine may be air-gapped or on a restricted
    /// network, and a scientific tool should not phone home unasked.
    #[serde(default)]
    pub check_updates_on_launch: bool,
    /// When the last update check ran (ms since the epoch), so a failing endpoint
    /// backs off instead of being retried every launch.
    #[serde(default)]
    pub last_update_check_ms: Option<u64>,
    /// The newest release tag already shown to the user, so the same version is
    /// not announced twice.
    #[serde(default)]
    pub last_seen_release: Option<String>,
    /// The columns the Enum Report tab starts with, per granularity, for a file
    /// that has none of its own remembered.
    #[serde(default)]
    pub enum_report_columns: EnumReportColumns,
}

/// Default Enum Report columns per granularity: column objects as the config's
/// `ColumnSpec` reads them. `None` means the GUI's built-in set.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EnumReportColumns {
    #[serde(default)]
    pub contig: Option<Vec<serde_json::Value>>,
    #[serde(default)]
    pub cds: Option<Vec<serde_json::Value>>,
    #[serde(default)]
    pub domain: Option<Vec<serde_json::Value>>,
}

fn default_schema_version() -> u32 {
    SCHEMA_VERSION
}

fn default_max_concurrent_jobs() -> usize {
    DEFAULT_MAX_CONCURRENT_JOBS
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
            theme: Theme::System,
            default_cpu: 0,
            max_concurrent_jobs: DEFAULT_MAX_CONCURRENT_JOBS,
            check_updates_on_launch: false,
            last_update_check_ms: None,
            last_seen_release: None,
            enum_report_columns: EnumReportColumns::default(),
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
        self.recent_workspaces
            .iter()
            .filter(|p| p.is_dir())
            .cloned()
            .collect()
    }

    /// The workspace to reopen at launch, if any.
    pub fn workspace_to_reopen(&self) -> Option<PathBuf> {
        if !self.reopen_last_workspace {
            return None;
        }
        self.existing_recent().into_iter().next()
    }
}

/// A partial update from the Preferences dialog.
///
/// One command instead of one per field, and `None` means "leave alone" — so a
/// dialog that only knows about some fields can never clear the others. Notably
/// it cannot touch `recent_workspaces`.
#[derive(Debug, Default, Deserialize)]
pub struct PrefsPatch {
    pub reopen_last_workspace: Option<bool>,
    pub theme: Option<Theme>,
    pub default_cpu: Option<usize>,
    pub max_concurrent_jobs: Option<usize>,
    pub check_updates_on_launch: Option<bool>,
    /// Replaces all three granularities' defaults at once.
    pub enum_report_columns: Option<EnumReportColumns>,
}

/// Apply a patch in place.
pub fn apply_patch(p: &mut Prefs, patch: &PrefsPatch) {
    if let Some(v) = patch.reopen_last_workspace {
        p.reopen_last_workspace = v;
    }
    if let Some(v) = patch.theme {
        p.theme = v;
    }
    if let Some(v) = patch.default_cpu {
        p.default_cpu = v;
    }
    if let Some(v) = patch.max_concurrent_jobs {
        p.max_concurrent_jobs = v.max(1);
    }
    if let Some(v) = patch.check_updates_on_launch {
        p.check_updates_on_launch = v;
    }
    if let Some(v) = &patch.enum_report_columns {
        p.enum_report_columns = v.clone();
    }
}

/// Read the preferences at `path`. A missing, unreadable or malformed file yields
/// defaults — see the module note on best-effort behavior.
pub fn load_prefs(path: &Path) -> Prefs {
    fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
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
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "prefs".into())
    ));
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {

    /// The new fields are all `#[serde(default)]`, which is exactly why
    /// `SCHEMA_VERSION` stays at 1 (the module's own rule: bump only when a
    /// field's *meaning* changes incompatibly). This proves the claim rather than
    /// asserting it in a comment: a file written by the previous build still
    /// loads, keeps its recent list, and picks up defaults for the rest.
    #[test]
    fn a_prefs_file_from_before_these_fields_still_loads() {
        let dir = std::env::temp_dir().join(format!("domainator-prefs-old-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("prefs.json");
        fs::write(
            &path,
            br#"{"schema_version":1,"recent_workspaces":["/tmp"],"reopen_last_workspace":false}"#,
        )
        .unwrap();

        let p = load_prefs(&path);
        assert_eq!(
            p.recent_workspaces,
            vec![PathBuf::from("/tmp")],
            "recent list survived"
        );
        assert!(!p.reopen_last_workspace, "existing field kept its value");
        assert_eq!(p.theme, Theme::System);
        assert_eq!(p.default_cpu, 0);
        assert_eq!(p.max_concurrent_jobs, DEFAULT_MAX_CONCURRENT_JOBS);
        assert!(!p.check_updates_on_launch);
        assert_eq!(p.last_update_check_ms, None);
        assert_eq!(p.enum_report_columns, EnumReportColumns::default());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_patch_changes_nothing() {
        let mut p = Prefs {
            default_cpu: 8,
            theme: Theme::Dark,
            ..Default::default()
        };
        p.push_recent("/a");
        let before = format!("{p:?}");
        apply_patch(&mut p, &PrefsPatch::default());
        assert_eq!(format!("{p:?}"), before);
    }

    #[test]
    fn a_patch_sets_only_the_fields_it_names() {
        let mut p = Prefs {
            default_cpu: 8,
            ..Default::default()
        };
        apply_patch(
            &mut p,
            &PrefsPatch {
                theme: Some(Theme::Dark),
                ..Default::default()
            },
        );
        assert_eq!(p.theme, Theme::Dark);
        assert_eq!(p.default_cpu, 8, "an unnamed field is untouched");
        assert!(
            p.reopen_last_workspace,
            "an unnamed bool is not reset to false"
        );
    }

    /// The Enum Report defaults are replaced together, and a patch that does not
    /// name them leaves them alone.
    #[test]
    fn a_patch_replaces_the_enum_report_defaults_whole() {
        let mut p = Prefs::default();
        let cols = EnumReportColumns {
            cds: Some(vec![serde_json::json!({"type": "length"})]),
            ..Default::default()
        };
        apply_patch(
            &mut p,
            &PrefsPatch {
                enum_report_columns: Some(cols.clone()),
                ..Default::default()
            },
        );
        assert_eq!(p.enum_report_columns, cols);
        apply_patch(
            &mut p,
            &PrefsPatch {
                default_cpu: Some(4),
                ..Default::default()
            },
        );
        assert_eq!(p.enum_report_columns, cols);
    }

    /// The patch has no field for it, but it is worth pinning: losing the recent
    /// list to a settings change would be a bad surprise.
    #[test]
    fn a_patch_never_clears_the_recent_list() {
        let mut p = Prefs::default();
        p.push_recent("/a");
        p.push_recent("/b");
        apply_patch(
            &mut p,
            &PrefsPatch {
                reopen_last_workspace: Some(false),
                theme: Some(Theme::Light),
                default_cpu: Some(4),
                max_concurrent_jobs: Some(3),
                check_updates_on_launch: Some(true),
                enum_report_columns: Some(EnumReportColumns::default()),
            },
        );
        assert_eq!(p.recent_workspaces.len(), 2);
    }

    /// Zero jobs at a time would queue everything forever.
    #[test]
    fn max_concurrent_jobs_is_at_least_one() {
        let mut p = Prefs::default();
        apply_patch(
            &mut p,
            &PrefsPatch {
                max_concurrent_jobs: Some(0),
                ..Default::default()
            },
        );
        assert_eq!(p.max_concurrent_jobs, 1);
    }

    #[test]
    fn theme_round_trips_as_lowercase_json() {
        assert_eq!(serde_json::to_string(&Theme::Dark).unwrap(), "\"dark\"");
        assert_eq!(
            serde_json::from_str::<Theme>("\"system\"").unwrap(),
            Theme::System
        );
    }
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
