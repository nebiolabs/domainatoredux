//! Workspace / filesystem commands (LEFT tree + MIDDLE file table).

use std::path::Path;

use domainator_core::app_support::workspace::{
    list_files, open_or_init_manifest, scan_folder_tree, FileEntry, FolderNode, Settings,
};
use serde::Serialize;

/// Everything the UI needs after opening a workspace directory.
#[derive(Serialize)]
pub struct WorkspaceInfo {
    pub root: String,
    pub tree: FolderNode,
    pub settings: Settings,
}

/// Native folder picker (returns `None` if the user cancels).
#[tauri::command]
pub fn pick_directory() -> Result<Option<String>, String> {
    Ok(rfd::FileDialog::new()
        .pick_folder()
        .map(|p| p.to_string_lossy().into_owned()))
}

/// Open a directory as a workspace: load/create its `workspace.json` and scan the
/// folder tree. The tree is derived from the filesystem so files added out-of-band
/// (e.g. by the CLI) always show up.
#[tauri::command]
pub async fn open_workspace(root: String) -> Result<WorkspaceInfo, String> {
    // Off the UI thread: scanning the tree classifies each file (reading first
    // records), which can be slow on a large workspace. A synchronous command
    // would freeze all IPC while it runs.
    tauri::async_runtime::spawn_blocking(move || {
        let root_path = Path::new(&root);
        if !root_path.is_dir() {
            return Err(format!("not a directory: {root}"));
        }
        let manifest =
            open_or_init_manifest(root_path).map_err(|e| format!("workspace manifest: {e}"))?;
        let tree = scan_folder_tree(root_path).map_err(|e| format!("scanning folders: {e}"))?;
        Ok(WorkspaceInfo { root, tree, settings: manifest.settings })
    })
    .await
    .map_err(|e| format!("opening workspace failed: {e}"))?
}

/// List the recognized data files directly inside `dir`.
#[tauri::command]
pub async fn list_folder(dir: String) -> Result<Vec<FileEntry>, String> {
    // Off the UI thread: classifies each file (may read first records).
    tauri::async_runtime::spawn_blocking(move || {
        list_files(Path::new(&dir)).map_err(|e| format!("listing {dir}: {e}"))
    })
    .await
    .map_err(|e| format!("listing folder failed: {e}"))?
}

/// Re-scan the folder tree (after creating/removing directories).
#[tauri::command]
pub async fn rescan_tree(root: String) -> Result<FolderNode, String> {
    tauri::async_runtime::spawn_blocking(move || {
        scan_folder_tree(Path::new(&root)).map_err(|e| format!("scanning folders: {e}"))
    })
    .await
    .map_err(|e| format!("scanning folders failed: {e}"))?
}

/// Create a subdirectory `name` under `parent`; returns the new path.
#[tauri::command]
pub fn create_folder(parent: String, name: String) -> Result<String, String> {
    let clean = name.trim();
    if clean.is_empty() || clean == "." || clean == ".." || clean.contains('/') || clean.contains('\\') {
        return Err("invalid folder name".into());
    }
    let path = Path::new(&parent).join(clean);
    if path.exists() {
        return Err(format!("already exists: {clean}"));
    }
    std::fs::create_dir(&path).map_err(|e| format!("creating folder: {e}"))?;
    Ok(path.to_string_lossy().into_owned())
}
