//! Databases panel commands.
//!
//! **One registry for every workspace**, kept in the app data directory
//! ([`central_registry_path`]). A database is registered by path and never copied:
//! the files stay where they are, and adding one makes it available to every
//! workspace's tool dialogs. Workspaces used to keep their own list; opening one
//! folds that list in once ([`list_databases`]).
//!
//! Managed reference downloads (UniProt / Pfam, `cmd_refdb`) live in this same
//! registry. They are recognized by their manifest entry, so Rename is refused for
//! them (the manifest tracks the file name) and Delete goes through
//! `reference_dbs::uninstall`, which keeps the manifest in step.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Manager};

use domainator_core::app_support::registry::{
    self, classify_database, database_files, index_status, load_registry_file,
    migrate_workspace_registry, save_registry_file, DatabaseEntry, DatabaseRegistry, IndexStatus,
    REGISTRY_NAME,
};

/// Serializes every read-modify-write of the registry file. Registering from the
/// panel and from a finishing reference download can otherwise interleave and
/// drop one of the two entries.
static REGISTRY_LOCK: Mutex<()> = Mutex::new(());

/// `<app data>/databases.json`.
pub fn central_registry_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app.path().app_data_dir().map_err(|e| format!("app data dir: {e}"))?.join(REGISTRY_NAME))
}

/// Load, change and save the central registry under [`REGISTRY_LOCK`].
pub fn update_registry<R>(
    app: &AppHandle,
    f: impl FnOnce(&mut DatabaseRegistry) -> R,
) -> Result<R, String> {
    let path = central_registry_path(app)?;
    let _guard = REGISTRY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut reg = load_registry_file(&path);
    let out = f(&mut reg);
    save_registry_file(&path, &reg).map_err(|e| format!("saving the database registry: {e}"))?;
    Ok(out)
}

fn load_central(app: &AppHandle) -> Result<DatabaseRegistry, String> {
    let path = central_registry_path(app)?;
    let _guard = REGISTRY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    Ok(load_registry_file(&path))
}

/// Classify `path` and add it to the central registry (replacing `replaces`).
pub fn register(app: &AppHandle, path: &str, replaces: &[String]) -> Result<DatabaseEntry, String> {
    let entry = classify_database(path)?;
    update_registry(app, |reg| reg.upsert(entry.clone(), replaces))?;
    Ok(entry)
}

/// The id of the managed reference database stored at `path`, if it is one.
fn managed_id(app: &AppHandle, path: &str) -> Option<String> {
    crate::reference_dbs::load_manifest(app)
        .databases
        .into_iter()
        .find(|d| d.path == path)
        .map(|d| d.id)
}

/// One row of the Databases panel: the registry entry plus what is true of it on
/// disk right now.
#[derive(Serialize)]
pub struct DatabaseRow {
    #[serde(flatten)]
    pub entry: DatabaseEntry,
    pub index: IndexStatus,
    /// A managed reference download rather than a database the user added.
    pub managed: bool,
    /// The managed download's id (`reference_dbs`), which a tool spec's
    /// `select: database:<id>` names.
    pub managed_id: Option<String>,
    /// Whether any of its files still exist (a database on an unplugged drive
    /// stays listed, marked missing, rather than silently disappearing).
    pub on_disk: bool,
}

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

/// The central database list. With `workspace_root`, first folds that
/// workspace's legacy list into it (once; see `migrate_workspace_registry`).
#[tauri::command]
pub async fn list_databases(
    app: AppHandle,
    workspace_root: Option<String>,
) -> Result<Vec<DatabaseRow>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        if let Some(root) = workspace_root.as_deref().filter(|r| !r.is_empty()) {
            let central = central_registry_path(&app)?;
            let _guard = REGISTRY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            migrate_workspace_registry(&central, Path::new(root))
                .map_err(|e| format!("moving this workspace's databases to the shared list: {e}"))?;
        }
        let managed: HashMap<String, String> = crate::reference_dbs::load_manifest(&app)
            .databases
            .into_iter()
            .map(|d| (d.path, d.id))
            .collect();
        Ok(load_central(&app)?
            .databases
            .into_iter()
            .map(|entry| {
                let p = Path::new(&entry.path);
                DatabaseRow {
                    index: index_status(p, &entry.format),
                    managed: managed.contains_key(&entry.path),
                    managed_id: managed.get(&entry.path).cloned(),
                    on_disk: !database_files(p).is_empty(),
                    entry,
                }
            })
            .collect())
    })
    .await
    .map_err(|e| format!("listing databases failed: {e}"))?
}

/// Classify `path` and add/replace it in the shared registry.
#[tauri::command]
pub async fn register_database(app: AppHandle, path: String) -> Result<DatabaseEntry, String> {
    // Off the UI thread: classification reads the file (first record / shard
    // resolution), which shouldn't block Tauri's main thread / other IPC.
    tauri::async_runtime::spawn_blocking(move || register(&app, &path, &[]))
        .await
        .map_err(|e| format!("registering database failed: {e}"))?
}

/// Remove a database from the list. Its files are left alone.
#[tauri::command]
pub async fn unregister_database(app: AppHandle, path: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || update_registry(&app, |reg| reg.remove(&path)).map(|_| ()))
        .await
        .map_err(|e| format!("unregistering database failed: {e}"))?
}

/// The files that make up a database (data or shards, plus `.didx` indexes), so
/// Delete can say exactly what it is about to remove.
#[tauri::command]
pub async fn list_database_files(path: String) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        database_files(Path::new(&path)).into_iter().map(|p| p.to_string_lossy().into_owned()).collect()
    })
    .await
    .map_err(|e| format!("listing database files failed: {e}"))
}

/// Rename a database on disk (shards and indexes included) and update its entry.
/// Refused for managed reference downloads, whose file name the manifest owns.
#[tauri::command]
pub async fn rename_database(app: AppHandle, path: String, new_name: String) -> Result<DatabaseEntry, String> {
    tauri::async_runtime::spawn_blocking(move || {
        if managed_id(&app, &path).is_some() {
            return Err("a managed reference database cannot be renamed".into());
        }
        let new_path = registry::rename_database(Path::new(&path), &new_name)?;
        let new_path = new_path.to_string_lossy().into_owned();
        register(&app, &new_path, &[path])
    })
    .await
    .map_err(|e| format!("renaming database failed: {e}"))?
}

/// Delete a database's files from disk and remove it from the list.
#[tauri::command]
pub async fn delete_database(app: AppHandle, path: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        match managed_id(&app, &path) {
            Some(id) => {
                crate::reference_dbs::uninstall(&app, &id)?;
            }
            None => {
                registry::delete_database_files(Path::new(&path))?;
            }
        }
        update_registry(&app, |reg| reg.remove(&path)).map(|_| ())
    })
    .await
    .map_err(|e| format!("deleting database failed: {e}"))?
}

/// Build the `.didx` index for each of a database's data files, in place. Can
/// take minutes on a large database; it runs on the blocking pool.
#[tauri::command]
pub async fn build_database_index(path: String) -> Result<usize, String> {
    tauri::async_runtime::spawn_blocking(move || registry::build_database_index(Path::new(&path)))
        .await
        .map_err(|e| format!("indexing failed: {e}"))?
}

/// Show a database in the OS file manager. A sharded database's logical path is
/// not a file, so its first data file is what gets selected. Only registered
/// databases: the path came from the frontend.
#[tauri::command]
pub async fn reveal_database(app: AppHandle, path: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let registered = load_central(&app)?.databases.iter().any(|d| d.path == path);
    if !registered {
        return Err(format!("refusing to reveal {path}: not a registered database"));
    }
    let first = database_files(Path::new(&path))
        .into_iter()
        .next()
        .ok_or_else(|| format!("{path} is not on disk"))?;
    app.opener()
        .reveal_item_in_dir(&first)
        .map_err(|e| format!("could not open a file manager for {}: {e}", first.display()))
}
