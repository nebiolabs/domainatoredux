//! Workspace / filesystem commands (LEFT tree + MIDDLE file table).

use std::path::Path;

use domainator_core::app_support::workspace::{
    accepted_types_hint, classify_file, dot_dir, is_reserved_name, list_files, open_or_init_manifest,
    scan_folder_tree, FileEntry, FileMeta, FolderNode, Settings, DOT_DIR,
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

/// Native multi-file picker for import (returns `None` if the user cancels).
#[tauri::command]
pub fn pick_files() -> Result<Option<Vec<String>>, String> {
    Ok(rfd::FileDialog::new().pick_files().map(|paths| {
        paths
            .into_iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect()
    }))
}

/// Pick a collision-free destination path for `name` inside `dir`: if it already
/// exists, insert " (2)", " (3)", … before the final extension.
fn unique_dest(dir: &Path, name: &std::ffi::OsStr) -> std::path::PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let name = name.to_string_lossy();
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]), // keep the dot in ext
        _ => (name.as_ref(), ""),
    };
    for n in 2..10_000 {
        let cand = dir.join(format!("{stem} ({n}){ext}"));
        if !cand.exists() {
            return cand;
        }
    }
    first // pathological fallback
}

/// Import (copy) `sources` into workspace folder `dir`, renaming to avoid clobbering
/// existing files. Skips directories and non-existent paths. Returns the paths
/// actually written. Refuses the reserved `.domainator/` machinery directory.
#[tauri::command]
pub async fn import_files(
    workspace_root: String,
    dir: String,
    sources: Vec<String>,
) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = Path::new(&workspace_root);
        let target = Path::new(&dir);
        if target.starts_with(dot_dir(root)) || target.components().any(|c| c.as_os_str() == DOT_DIR)
        {
            return Err("cannot import into the .domainator directory".to_string());
        }
        if !target.is_dir() {
            return Err(format!("not a directory: {dir}"));
        }
        let mut imported = Vec::new();
        for src in &sources {
            let sp = Path::new(src);
            // Skip directories and vanished paths rather than erroring the batch.
            if !sp.is_file() {
                continue;
            }
            let name = match sp.file_name() {
                Some(n) => n,
                None => continue,
            };
            let dest = unique_dest(target, name);
            std::fs::copy(sp, &dest)
                .map_err(|e| format!("copying {}: {e}", sp.display()))?;
            imported.push(dest.to_string_lossy().into_owned());
        }
        Ok(imported)
    })
    .await
    .map_err(|e| format!("import failed: {e}"))?
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

/// List the recognized data files directly inside `dir` (sidecar metadata is read
/// from `<workspace_root>/.domainator/meta`).
#[tauri::command]
pub async fn list_folder(workspace_root: String, dir: String) -> Result<Vec<FileEntry>, String> {
    // Off the UI thread: classifies each file (may read first records).
    tauri::async_runtime::spawn_blocking(move || {
        list_files(Path::new(&workspace_root), Path::new(&dir)).map_err(|e| format!("listing {dir}: {e}"))
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
    if is_reserved_name(clean) {
        return Err(format!("'{DOT_DIR}' is reserved for the app"));
    }
    let path = Path::new(&parent).join(clean);
    if path.exists() {
        return Err(format!("already exists: {clean}"));
    }
    std::fs::create_dir(&path).map_err(|e| format!("creating folder: {e}"))?;
    Ok(path.to_string_lossy().into_owned())
}

/// Rename a data file in place (same folder), moving its metadata sidecar too.
/// Refuses the reserved `.domainator/` dir, invalid/reserved names, and collisions.
#[tauri::command]
pub async fn rename_file(
    workspace_root: String,
    path: String,
    new_name: String,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = Path::new(&workspace_root);
        let clean = new_name.trim();
        if clean.is_empty() || clean.contains('/') || clean.contains('\\') || clean == "." || clean == ".." {
            return Err("invalid file name".into());
        }
        if is_reserved_name(clean) {
            return Err(format!("'{clean}' is reserved"));
        }
        let old = Path::new(&path);
        if old.starts_with(dot_dir(root)) || old.components().any(|c| c.as_os_str() == DOT_DIR) {
            return Err("refusing to rename inside the .domainator directory".into());
        }
        if !old.is_file() {
            return Err(format!("not a file: {path}"));
        }
        let new_path = old.with_file_name(clean);
        if classify_file(&new_path).is_none() {
            return Err(format!(
                "unrecognized file type for '{clean}'. {}",
                accepted_types_hint()
            ));
        }
        if new_path.exists() {
            return Err(format!("already exists: {clean}"));
        }
        std::fs::rename(old, &new_path).map_err(|e| format!("renaming: {e}"))?;
        // Move the metadata sidecar if one exists.
        let old_sc = FileMeta::sidecar_path(root, old);
        if old_sc.is_file() {
            let new_sc = FileMeta::sidecar_path(root, &new_path);
            if let Some(parent) = new_sc.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::rename(&old_sc, &new_sc);
        }
        Ok(new_path.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| format!("rename failed: {e}"))?
}

/// Delete a data file and its metadata sidecar. Refuses anything inside the
/// reserved `.domainator/` machinery directory.
#[tauri::command]
pub async fn delete_file(workspace_root: String, path: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = Path::new(&workspace_root);
        let p = Path::new(&path);
        if p.starts_with(dot_dir(root)) || p.components().any(|c| c.as_os_str() == DOT_DIR) {
            return Err("refusing to delete inside the .domainator directory".to_string());
        }
        if !p.is_file() {
            return Err(format!("not a file: {path}"));
        }
        std::fs::remove_file(p).map_err(|e| format!("deleting {path}: {e}"))?;
        FileMeta::remove(root, p); // best-effort sidecar cleanup
        Ok(())
    })
    .await
    .map_err(|e| format!("delete failed: {e}"))?
}
