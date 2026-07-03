//! Databases panel commands (workspace-global registry).

use std::path::Path;

use domainator_core::app_support::registry::{
    classify_database, load_registry, save_registry, DatabaseEntry,
};

/// Single-file picker (used for "Add database…").
#[tauri::command]
pub fn pick_file() -> Result<Option<String>, String> {
    Ok(rfd::FileDialog::new().pick_file().map(|p| p.to_string_lossy().into_owned()))
}

#[tauri::command]
pub fn list_databases(workspace_root: String) -> Vec<DatabaseEntry> {
    load_registry(Path::new(&workspace_root)).databases
}

/// Classify `path` and add/replace it in the workspace registry.
#[tauri::command]
pub fn register_database(workspace_root: String, path: String) -> Result<DatabaseEntry, String> {
    let entry = classify_database(&path)?;
    let root = Path::new(&workspace_root);
    let mut reg = load_registry(root);
    reg.databases.retain(|d| d.path != entry.path);
    reg.databases.push(entry.clone());
    save_registry(root, &reg).map_err(|e| format!("saving registry: {e}"))?;
    Ok(entry)
}

#[tauri::command]
pub fn unregister_database(workspace_root: String, path: String) -> Result<(), String> {
    let root = Path::new(&workspace_root);
    let mut reg = load_registry(root);
    reg.databases.retain(|d| d.path != path);
    save_registry(root, &reg).map_err(|e| format!("saving registry: {e}"))
}
