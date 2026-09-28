//! Commands for the managed reference-database registry (UniProt / Pfam).
//!
//! The counterpart of `cmd_external` for data rather than binaries. Two things it
//! has to do that the tool downloader does not, both consequences of scale: report
//! progress as `refdb-progress` events (a TrEMBL download runs for hours), and let
//! the user stop one — the cancel flags live in [`RefDbRegistry`] so the blocking
//! worker can read them mid-stream rather than only after it finishes.
//!
//! On success a database is registered into the shared database registry too
//! (`cmd_db`), so it appears in the Databases panel and in every workspace's tool
//! dialogs. That is the whole point of downloading it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::reference_dbs::{self, InstalledDb};

/// Managed state: a cancel flag per in-flight download, keyed by database id.
#[derive(Default)]
pub struct RefDbRegistry {
    downloading: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

/// Lock that survives a poisoned mutex — see `cmd_files::lock_poison_tolerant`.
fn reg_lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// One row of the Reference databases panel.
#[derive(Serialize)]
pub struct ReferenceDbStatus {
    pub id: String,
    pub name: String,
    pub description: String,
    /// `fasta` | `hmm`.
    pub kind: String,
    pub file_name: String,
    pub url: String,
    pub download_bytes: u64,
    pub disk_bytes: u64,
    /// Free bytes the install actually needs — `disk_bytes`, plus the copy already
    /// installed when this is an update, since the old file is only replaced at the
    /// closing rename.
    pub required_bytes: u64,
    /// False only when the shortfall is certain; a filesystem whose free space
    /// cannot be read reports `true` rather than blocking the download.
    pub enough_space: bool,
    /// Whether the panel must confirm before starting this download.
    pub needs_confirmation: bool,
    /// Present once installed, with the release it was installed at.
    pub installed: Option<InstalledDb>,
    /// The newest release seen by a user-requested check, if one has ever run.
    pub latest_version: Option<String>,
    pub checked_at_ms: Option<String>,
    /// True when a check has found a release newer than the installed one.
    pub update_available: bool,
    pub downloading: bool,
}

/// Progress for one download, broadcast as the `refdb-progress` event.
#[derive(Serialize, Clone)]
pub struct RefDbProgress {
    pub id: String,
    /// `downloading` | `indexing` | `registering`.
    pub phase: String,
    pub done_bytes: u64,
    /// 0 when the server sent no `Content-Length`, or for the indexing phase.
    pub total_bytes: u64,
}

/// The panel's whole state: the databases, and the free space they have to fit in.
#[derive(Serialize)]
pub struct ReferenceDbList {
    pub databases: Vec<ReferenceDbStatus>,
    /// Free space on the volume holding the managed directory, or `null` when the
    /// platform will not say.
    pub free_bytes: Option<u64>,
    pub dir: String,
}

/// Every managed reference database and its state. No network access: this is what
/// the panel renders on open, and it must not stall on a slow FTP mirror.
#[tauri::command]
pub async fn list_reference_dbs(
    app: AppHandle,
    registry: tauri::State<'_, RefDbRegistry>,
) -> Result<ReferenceDbList, String> {
    let in_flight: Vec<String> = reg_lock(&registry.downloading).keys().cloned().collect();
    let manifest = reference_dbs::load_manifest(&app);
    let dir = reference_dbs::managed_dir(&app)?;
    let free_bytes = reference_dbs::free_space(&dir);
    let databases = reference_dbs::specs()
        .into_iter()
        .map(|s| {
            // Straight from the manifest would show a database the user has since
            // deleted from disk, which every tool dropdown would then offer and no
            // tool could open -- so the file has to still be there.
            let installed = manifest
                .databases
                .iter()
                .find(|d| d.id == s.id)
                .filter(|d| std::path::Path::new(&d.path).is_file())
                .cloned();
            let check = manifest.checks.iter().find(|c| c.id == s.id);
            let latest = check.map(|c| c.version.clone());
            let update_available = match (&installed, &latest) {
                // Deliberately "differs", not "is greater": UniProt releases are
                // `2026_03` and Pfam's are `38.2`, which share no ordering, and a
                // published release only ever moves forward.
                (Some(i), Some(l)) => &i.version != l,
                _ => false,
            };
            let required_bytes = reference_dbs::required_bytes(&s, &dir);
            ReferenceDbStatus {
                id: s.id.into(),
                name: s.name.into(),
                description: s.description.into(),
                kind: s.kind.as_str().into(),
                file_name: s.file_name.into(),
                url: s.url.into(),
                download_bytes: s.download_bytes,
                disk_bytes: s.disk_bytes,
                required_bytes,
                // An unreadable free-space figure must not block a download, so an
                // unknown counts as enough -- `install` re-checks and is the gate.
                enough_space: free_bytes.is_none_or(|free| free >= required_bytes),
                needs_confirmation: s.needs_confirmation(),
                installed,
                latest_version: latest,
                checked_at_ms: check.map(|c| c.checked_at_ms.clone()),
                update_available,
                downloading: in_flight.iter().any(|d| d == s.id),
            }
        })
        .collect();
    Ok(ReferenceDbList {
        databases,
        free_bytes,
        dir: dir.to_string_lossy().into_owned(),
    })
}

/// Fetch one database's current release from its publisher and remember it. This is
/// the only command that goes to the network on its own, and only when asked.
#[tauri::command]
pub async fn check_reference_db_version(app: AppHandle, id: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let spec = reference_dbs::spec(&id).ok_or_else(|| format!("unknown database: {id}"))?;
        let version = reference_dbs::remote_version(&spec)?;
        reference_dbs::record_check(&app, &id, &version)?;
        Ok(version)
    })
    .await
    .map_err(|e| format!("version check failed: {e}"))?
}

/// Download + install a reference database, then register it in the shared
/// database registry.
///
/// Used for a first install and for an update alike — an update is the same
/// download over the same file name, which is why the release is read from the
/// publisher's release file rather than from the (unchanging) URL.
#[tauri::command]
pub async fn download_reference_db(
    app: AppHandle,
    registry: tauri::State<'_, RefDbRegistry>,
    id: String,
) -> Result<InstalledDb, String> {
    let flag = Arc::new(AtomicBool::new(false));
    {
        let mut in_flight = reg_lock(&registry.downloading);
        if in_flight.contains_key(&id) {
            return Err(format!("{id} is already downloading"));
        }
        in_flight.insert(id.clone(), flag.clone());
    }

    let app_c = app.clone();
    let id_c = id.clone();
    let flag_c = flag.clone();
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        let spec = reference_dbs::spec(&id_c).ok_or_else(|| format!("unknown database: {id_c}"))?;
        let report = |phase: &str, done: u64, total: u64| {
            let _ = app_c.emit(
                "refdb-progress",
                RefDbProgress {
                    id: id_c.clone(),
                    phase: phase.to_string(),
                    done_bytes: done,
                    total_bytes: total,
                },
            );
        };
        let entry = reference_dbs::install(&app_c, &spec, &flag_c, &report)?;

        // Make it usable: without this the file is on disk and no tool offers it.
        report("registering", 0, 0);
        // `install` deleted any copy stored under a name this release no longer
        // uses; drop those from the registry too, or the panel lists a ghost.
        let dir = reference_dbs::managed_dir(&app_c)?;
        let superseded: Vec<String> = spec
            .superseded_file_names
            .iter()
            .map(|n| dir.join(n).to_string_lossy().into_owned())
            .collect();
        crate::cmd_db::register(&app_c, &entry.path, &superseded)?;
        Ok::<_, String>(entry)
    })
    .await
    .map_err(|e| format!("download failed: {e}"))?;

    reg_lock(&registry.downloading).remove(&id);
    // A cancel arrives as an error from the stream; say what actually happened.
    if outcome.is_err() && flag.load(Ordering::Relaxed) {
        let _ = app.emit(
            "refdb-progress",
            RefDbProgress {
                id,
                phase: "canceled".into(),
                done_bytes: 0,
                total_bytes: 0,
            },
        );
        return Err("canceled".into());
    }
    let _ = app.emit(
        "refdb-progress",
        RefDbProgress {
            id,
            phase: if outcome.is_ok() { "done" } else { "failed" }.into(),
            done_bytes: 0,
            total_bytes: 0,
        },
    );
    outcome
}

/// Ask an in-flight download to stop. Best-effort in the same sense as
/// `cancel_job`, but far more effective: the flag is read by the download stream
/// itself, so a canceled transfer stops at the next read rather than at the end.
#[tauri::command]
pub fn cancel_reference_db(registry: tauri::State<'_, RefDbRegistry>, id: String) {
    if let Some(flag) = reg_lock(&registry.downloading).get(&id) {
        flag.store(true, Ordering::Relaxed);
    }
}

/// Delete an installed database and unregister it.
#[tauri::command]
pub async fn remove_reference_db(app: AppHandle, id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        if let Some(path) = reference_dbs::uninstall(&app, &id)? {
            crate::cmd_db::update_registry(&app, |reg| reg.remove(&path))?;
        }
        Ok(())
    })
    .await
    .map_err(|e| format!("removing database failed: {e}"))?
}

/// Make sure every installed reference database is in the shared registry.
///
/// Downloads register themselves, so this only matters for copies installed
/// before the registry was shared (they were registered per workspace, and a
/// workspace that was never reopened would not bring them over). Called once at
/// startup. Entries already present are not reclassified.
#[tauri::command]
pub async fn sync_reference_dbs(app: AppHandle) -> Result<usize, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = crate::cmd_db::central_registry_path(&app)?;
        let known: Vec<String> = domainator_core::app_support::registry::load_registry_file(&path)
            .databases
            .into_iter()
            .map(|d| d.path)
            .collect();
        let mut added = 0usize;
        for entry in reference_dbs::load_manifest(&app).databases {
            if known.contains(&entry.path) || !std::path::Path::new(&entry.path).is_file() {
                continue;
            }
            // A classification failure on one database must not block the others.
            if crate::cmd_db::register(&app, &entry.path, &[]).is_ok() {
                added += 1;
            }
        }
        Ok(added)
    })
    .await
    .map_err(|e| format!("syncing databases failed: {e}"))?
}
