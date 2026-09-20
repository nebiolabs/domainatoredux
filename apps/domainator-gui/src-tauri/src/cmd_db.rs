//! Databases panel commands (workspace-global registry).

use std::path::Path;

use domainator_core::app_support::registry::{
    classify_database, load_registry, save_registry, DatabaseEntry,
};

/// Single-file picker (used for "Add database…"). `async` + `AsyncFileDialog`:
/// see the note on `cmd_workspace::pick_directory`.
#[tauri::command]
pub async fn pick_file(window: tauri::WebviewWindow) -> Result<Option<String>, String> {
    Ok(rfd::AsyncFileDialog::new()
        .set_parent(&window)
        .pick_file()
        .await
        .map(|h| h.path().to_string_lossy().into_owned()))
}

#[tauri::command]
pub async fn list_databases(workspace_root: String) -> Result<Vec<DatabaseEntry>, String> {
    tauri::async_runtime::spawn_blocking(move || load_registry(Path::new(&workspace_root)).databases)
        .await
        .map_err(|e| format!("listing databases failed: {e}"))
}

/// Classify `path` and add/replace it in the workspace registry.
#[tauri::command]
pub async fn register_database(
    workspace_root: String,
    path: String,
) -> Result<DatabaseEntry, String> {
    // Off the UI thread: classification reads the file (first record / shard
    // resolution), which shouldn't block Tauri's main thread / other IPC.
    tauri::async_runtime::spawn_blocking(move || {
        let entry = classify_database(&path)?;
        let root = Path::new(&workspace_root);
        let mut reg = load_registry(root);
        reg.databases.retain(|d| d.path != entry.path);
        reg.databases.push(entry.clone());
        save_registry(root, &reg).map_err(|e| format!("saving registry: {e}"))?;
        Ok(entry)
    })
    .await
    .map_err(|e| format!("registering database failed: {e}"))?
}

#[tauri::command]
pub async fn unregister_database(workspace_root: String, path: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = Path::new(&workspace_root);
        let mut reg = load_registry(root);
        reg.databases.retain(|d| d.path != path);
        save_registry(root, &reg).map_err(|e| format!("saving registry: {e}"))
    })
    .await
    .map_err(|e| format!("unregistering database failed: {e}"))?
}
