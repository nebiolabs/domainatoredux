//! Application preferences: the recent-workspace list and the reopen-on-launch
//! setting.
//!
//! These are per *app*, not per workspace, so they can't live under a workspace's
//! `.domainatoredux/` directory — they go in Tauri's per-app data dir, next to the
//! managed external-tool binaries (`external_tools::managed_dir`). The format and
//! the list semantics live in `domainator_core::app_support::prefs`, which is
//! where their tests are; this file only resolves the path and exposes commands.

use std::path::PathBuf;

use domainator_core::app_support::prefs::{
    apply_patch, load_prefs, update_prefs, Prefs, PrefsPatch,
};
use tauri::{AppHandle, Manager};

const PREFS_NAME: &str = "prefs.json";

/// `<app_data>/prefs.json`.
pub fn prefs_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir: {e}"))?
        .join(PREFS_NAME))
}

/// The current preferences (defaults if the file is missing or unreadable).
#[tauri::command(async)]
pub fn get_prefs(app: AppHandle) -> Result<Prefs, String> {
    Ok(load_prefs(&prefs_path(&app)?))
}

/// The recent workspaces that still exist on disk, most recent first, plus the
/// one to reopen at launch (`None` if reopening is off or nothing is available).
#[derive(serde::Serialize)]
pub struct StartupInfo {
    pub recent: Vec<PathBuf>,
    pub reopen: Option<PathBuf>,
}

/// What the shell needs on launch, in one round trip.
#[tauri::command(async)]
pub fn startup_info(app: AppHandle) -> Result<StartupInfo, String> {
    let prefs = load_prefs(&prefs_path(&app)?);
    Ok(StartupInfo {
        recent: prefs.existing_recent(),
        reopen: prefs.workspace_to_reopen(),
    })
}

/// Drop one entry from the recent list.
#[tauri::command(async)]
pub fn forget_recent_workspace(app: AppHandle, root: String) -> Result<Vec<PathBuf>, String> {
    let path = prefs_path(&app)?;
    let prefs = update_prefs(&path, |p| p.forget_recent(&root))
        .map_err(|e| format!("saving preferences: {e}"))?;
    Ok(prefs.existing_recent())
}

/// Apply a partial settings update and return the saved preferences.
///
/// One command rather than one per field. This replaces the old
/// `set_reopen_last_workspace`, which had a frontend wrapper but no caller —
/// keeping both would have left two paths to the same field.
#[tauri::command(async)]
pub fn set_prefs(app: AppHandle, patch: PrefsPatch) -> Result<Prefs, String> {
    let path = prefs_path(&app)?;
    update_prefs(&path, |p| apply_patch(p, &patch)).map_err(|e| format!("saving preferences: {e}"))
}

/// Record `root` as the most recently opened workspace. Best-effort: failing to
/// write the recent list must never fail the workspace open itself.
pub fn remember_workspace(app: &AppHandle, root: &str) {
    let Ok(path) = prefs_path(app) else { return };
    if let Err(e) = update_prefs(&path, |p| p.push_recent(root)) {
        eprintln!("could not record recent workspace: {e}");
    }
}
