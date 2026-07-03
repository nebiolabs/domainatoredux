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
pub fn open_workspace(root: String) -> Result<WorkspaceInfo, String> {
    let root_path = Path::new(&root);
    if !root_path.is_dir() {
        return Err(format!("not a directory: {root}"));
    }
    let manifest = open_or_init_manifest(root_path).map_err(|e| format!("workspace manifest: {e}"))?;
    let tree = scan_folder_tree(root_path).map_err(|e| format!("scanning folders: {e}"))?;
    Ok(WorkspaceInfo { root, tree, settings: manifest.settings })
}

/// List the recognized data files directly inside `dir`.
#[tauri::command]
pub fn list_folder(dir: String) -> Result<Vec<FileEntry>, String> {
    list_files(Path::new(&dir)).map_err(|e| format!("listing {dir}: {e}"))
}
