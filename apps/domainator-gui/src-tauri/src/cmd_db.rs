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
use std::sync::{Arc, LazyLock, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Manager};

use domainator_core::app_support::registry::{
    self, classify_database, database_files, index_status, load_registry_file,
    migrate_workspace_registry, save_registry_file, DatabaseEntry, DatabaseRegistry, IndexStatus,
    REGISTRY_NAME,
};

use crate::cmd_tools::LiveJob;
use crate::job_io;

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

/// Indexing runs in progress, by database path.
static INDEXING: LazyLock<Mutex<HashMap<String, Arc<LiveJob>>>> = LazyLock::new(Default::default);

/// Build the `.didx` index for each of a database's data files, in place.
///
/// Can take minutes on a large database, so it runs as `format-db --index` in a
/// worker process (`worker.rs`), like a Tools-menu job: [`cancel_database_index`]
/// can stop it, and a crash cannot take the app down. Returns the number of data
/// files indexed.
#[tauri::command]
pub async fn build_database_index(path: String) -> Result<usize, String> {
    tauri::async_runtime::spawn_blocking(move || index_with(&path, crate::cmd_tools::launcher()))
        .await
        .map_err(|e| format!("indexing failed: {e}"))?
}

fn index_with(path: &str, launcher: Result<crate::cmd_tools::Launcher, String>) -> Result<usize, String> {
    // The checks (on disk, not plain gzip) are quick and give the better message.
    let shards = registry::index_targets(Path::new(path))?;
    let indexes: Vec<String> = shards
        .iter()
        .map(|s| domainator_core::io::didx::index_path_for(Path::new(s)).to_string_lossy().into_owned())
        .collect();
    let live = Arc::new(LiveJob::default());
    {
        let mut running = INDEXING.lock().unwrap_or_else(|e| e.into_inner());
        if running.contains_key(path) {
            return Err(format!("{path} is already being indexed"));
        }
        running.insert(path.to_string(), live.clone());
    }
    // A config file rather than `-i` arguments: a database with many shards could
    // pass Windows' 32K command-line limit.
    let config = std::env::temp_dir().join(format!(
        "domainator-index-{}-{}.json",
        std::process::id(),
        domainator_core::app_support::provenance::now_ms()
    ));
    let snapshot = job_io::OutputSnapshot::take(&indexes);
    let ran = std::fs::write(&config, serde_json::json!({"input": shards, "index": true}).to_string())
        .map_err(|e| format!("writing the indexing config: {e}"))
        .and(launcher)
        .and_then(|launcher| {
            let args = ["format-db".into(), "--config".into(), config.clone().into_os_string()];
            let cwd = Path::new(path).parent().unwrap_or(Path::new("."));
            crate::cmd_tools::run_cli_command(&launcher, &args, cwd, None, &live)
        });
    let _ = std::fs::remove_file(&config);
    INDEXING.lock().unwrap_or_else(|e| e.into_inner()).remove(path);
    if live.is_canceled() {
        // A killed indexer can leave a truncated `.didx`, and one that reads back
        // wrong is worse than none: the database just shows as unindexed again.
        snapshot.remove_changed();
        return Err(INDEXING_CANCELED.into());
    }
    ran.map(|_| shards.len())
}

/// The error a canceled [`build_database_index`] returns. The panel checks for it
/// so a cancel is not shown as a failure.
pub const INDEXING_CANCELED: &str = "indexing canceled";

/// Stop the indexing of `path`, if it is running, and remove the partial indexes.
#[tauri::command(async)]
pub fn cancel_database_index(path: String) {
    let live = INDEXING.lock().unwrap_or_else(|e| e.into_inner()).get(&path).cloned();
    if let Some(live) = live {
        live.cancel();
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Indexing goes through the CLI's `format-db --config` now, so the config it
    /// writes must be one `FormatDbConfig` accepts, and the run must leave the
    /// same `.didx` the in-process `registry::build_database_index` did.
    #[test]
    fn indexing_runs_format_db_and_writes_the_index() {
        let dir = std::env::temp_dir().join(format!("domainator-gui-index-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("db.fasta");
        std::fs::write(&db, ">a\nMKVLAAGIVG\n>b\nMSTNPKPQRK\n").unwrap();
        let path = db.to_string_lossy().into_owned();

        let n = index_with(&path, Ok(crate::cmd_tools::Launcher::InProcess)).unwrap();
        assert_eq!(n, 1);
        assert!(domainator_core::io::didx::index_path_for(&db).exists(), "no .didx written");
        assert!(INDEXING.lock().unwrap().is_empty(), "a finished run stayed registered");
    }
}
