//! Workspace / filesystem commands (LEFT tree + MIDDLE file table).

use std::path::{Path, PathBuf};

use domainator_core::app_support::export::{self, ExportOptions, ExportOutcome, Packaging};
use domainator_core::app_support::workspace::{
    accepted_types_hint, classify_file, find_workspace_root, is_in_dot_dir, is_reserved_name,
    list_files, move_data_file, open_or_init_manifest, scan_folder_tree, EnumReportView, FileEntry,
    FileMeta, FolderNode, Settings, DOT_DIR,
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
///
/// `async` + `AsyncFileDialog` is required, not cosmetic: a synchronous
/// `#[tauri::command]` runs on Tauri's main thread, and rfd's *blocking* API shows
/// the dialog on the calling thread (`IFileDialog::Show` on Windows), spinning a
/// nested modal message loop inside WebView2's IPC dispatch — the same re-entrancy
/// documented at length in `cmd_windows::spawn_window`. The async variants run the
/// dialog on their own thread instead. `set_parent` also makes the dialog owned by
/// the app window, so Windows can't raise it *behind* the app.
#[tauri::command]
pub async fn pick_directory(window: tauri::WebviewWindow) -> Result<Option<String>, String> {
    Ok(rfd::AsyncFileDialog::new()
        .set_parent(&window)
        .pick_folder()
        .await
        .map(|h| h.path().to_string_lossy().into_owned()))
}

/// Native multi-file picker for import (returns `None` if the user cancels).
#[tauri::command]
pub async fn pick_files(window: tauri::WebviewWindow) -> Result<Option<Vec<String>>, String> {
    Ok(rfd::AsyncFileDialog::new()
        .set_parent(&window)
        .pick_files()
        .await
        .map(|handles| {
            handles
                .into_iter()
                .map(|h| h.path().to_string_lossy().into_owned())
                .collect()
        }))
}

/// Pick a collision-free destination path for `name` inside `dir`: if it already
/// exists, insert " (2)", " (3)", … before the final extension.
fn unique_dest(dir: &Path, name: &std::ffi::OsStr) -> PathBuf {
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
/// actually written. Refuses the reserved `.domainatoredux/` machinery directory.
#[tauri::command]
pub async fn import_files(
    workspace_root: String,
    dir: String,
    sources: Vec<String>,
) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = Path::new(&workspace_root);
        let target = Path::new(&dir);
        if is_in_dot_dir(root, target) {
            return Err("cannot import into the .domainatoredux directory".to_string());
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
            std::fs::copy(sp, &dest).map_err(|e| format!("copying {}: {e}", sp.display()))?;
            imported.push(dest.to_string_lossy().into_owned());
        }
        Ok(imported)
    })
    .await
    .map_err(|e| format!("import failed: {e}"))?
}

/// Native "Save As" picker that only chooses a path (`None` if the user cancels);
/// the caller does the writing. Used to name an export's zip archive. Async for the
/// reason given on [`pick_directory`].
#[tauri::command]
pub async fn pick_save_path(
    window: tauri::WebviewWindow,
    default_name: String,
    filter_name: String,
    extensions: Vec<String>,
) -> Result<Option<String>, String> {
    let exts: Vec<&str> = extensions.iter().map(String::as_str).collect();
    Ok(rfd::AsyncFileDialog::new()
        .set_parent(&window)
        .set_file_name(&default_name)
        .add_filter(&filter_name, &exts)
        .save_file()
        .await
        .map(|h| h.path().to_string_lossy().into_owned()))
}

/// Export `paths` to `dest` (a directory, or the zip archive to create); see
/// [`export::export_files`]. Without `overwrite`, an export that would replace
/// existing files writes nothing and returns their names for the user to confirm.
/// Sources may live outside the workspace (registered databases do).
#[tauri::command]
pub async fn export_files(
    app: tauri::AppHandle,
    paths: Vec<String>,
    options: ExportOptions,
    dest: String,
    overwrite: bool,
) -> Result<ExportOutcome, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let sources: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
        let dest = Path::new(&dest);
        let outcome = export::export_files(&sources, &options, dest, overwrite)?;
        if matches!(outcome, ExportOutcome::Done { .. }) {
            // Refreshes a file browser only if it is showing the destination.
            let dir = if options.packaging == Packaging::Zip {
                dest.parent()
            } else {
                Some(dest)
            };
            if let Some(dir) = dir {
                crate::cmd_files::notify_files_changed(&app, &dir.to_string_lossy());
            }
        }
        Ok(outcome)
    })
    .await
    .map_err(|e| format!("export failed: {e}"))?
}

/// Open a directory as a workspace: load/create its `workspace.json` and scan the
/// folder tree. The tree is derived from the filesystem so files added out-of-band
/// (e.g. by the CLI) always show up.
#[tauri::command]
pub async fn open_workspace(app: tauri::AppHandle, root: String) -> Result<WorkspaceInfo, String> {
    // Off the UI thread: scanning the tree classifies each file (reading first
    // records), which can be slow on a large workspace. A synchronous command
    // would freeze all IPC while it runs.
    let info = tauri::async_runtime::spawn_blocking(move || {
        let root_path = Path::new(&root);
        if !root_path.is_dir() {
            return Err(format!("not a directory: {root}"));
        }
        let manifest =
            open_or_init_manifest(root_path).map_err(|e| format!("workspace manifest: {e}"))?;
        let tree = scan_folder_tree(root_path).map_err(|e| format!("scanning folders: {e}"))?;
        Ok(WorkspaceInfo {
            root,
            tree,
            settings: manifest.settings,
        })
    })
    .await
    .map_err(|e| format!("opening workspace failed: {e}"))??;
    // Only a workspace that actually opened goes in the recent list.
    crate::cmd_prefs::remember_workspace(&app, &info.root);
    Ok(info)
}

/// List the recognized data files directly inside `dir` (sidecar metadata is read
/// from `<workspace_root>/.domainatoredux/meta`).
#[tauri::command]
pub async fn list_folder(workspace_root: String, dir: String) -> Result<Vec<FileEntry>, String> {
    // Off the UI thread: classifies each file (may read first records).
    tauri::async_runtime::spawn_blocking(move || {
        list_files(Path::new(&workspace_root), Path::new(&dir))
            .map_err(|e| format!("listing {dir}: {e}"))
    })
    .await
    .map_err(|e| format!("listing folder failed: {e}"))?
}

/// The Enum Report tab's remembered granularity and columns for `path`, from the
/// file's sidecar in the workspace that holds it (`None` when nothing was saved,
/// or the file is in no workspace).
#[tauri::command(async)]
pub fn get_enum_report_view(path: String) -> Option<EnumReportView> {
    let p = Path::new(&path);
    FileMeta::load(&find_workspace_root(p)?, p).enum_report
}

/// Remember the Enum Report tab's granularity and columns for `path` in its
/// sidecar, which follows the file through renames and moves. A file in no
/// workspace has no sidecar, so nothing is saved for it.
#[tauri::command(async)]
pub fn set_enum_report_view(path: String, view: EnumReportView) -> Result<(), String> {
    let p = Path::new(&path);
    let Some(root) = find_workspace_root(p) else {
        return Ok(());
    };
    let mut meta = FileMeta::load(&root, p);
    meta.enum_report = Some(view);
    meta.save(&root, p)
        .map_err(|e| format!("saving the report columns: {e}"))
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
    if clean.is_empty()
        || clean == "."
        || clean == ".."
        || clean.contains('/')
        || clean.contains('\\')
    {
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
/// Refuses the reserved `.domainatoredux/` dir, invalid/reserved names, and collisions.
#[tauri::command]
pub async fn rename_file(
    workspace_root: String,
    path: String,
    new_name: String,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = Path::new(&workspace_root);
        let clean = new_name.trim();
        if clean.is_empty()
            || clean.contains('/')
            || clean.contains('\\')
            || clean == "."
            || clean == ".."
        {
            return Err("invalid file name".into());
        }
        if is_reserved_name(clean) {
            return Err(format!("'{clean}' is reserved"));
        }
        let old = Path::new(&path);
        if is_in_dot_dir(root, old) {
            return Err("refusing to rename inside the .domainatoredux directory".into());
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
        FileMeta::follow(root, old, &new_path);
        Ok(new_path.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| format!("rename failed: {e}"))?
}

/// Delete a data file and its metadata sidecar. Refuses anything inside the
/// reserved `.domainatoredux/` machinery directory.
#[tauri::command]
pub async fn delete_file(workspace_root: String, path: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = Path::new(&workspace_root);
        let p = Path::new(&path);
        if is_in_dot_dir(root, p) {
            return Err("refusing to delete inside the .domainatoredux directory".to_string());
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

/// What a batch move did: `(old, new)` for each file that moved, `(path, reason)`
/// for each that did not. One bad file never stops the rest.
#[derive(Serialize)]
pub struct MoveReport {
    pub moved: Vec<(String, String)>,
    pub failed: Vec<(String, String)>,
}

/// Move data files into workspace folder `dest_dir`, each with its metadata
/// sidecar and `.didx` index (`workspace::move_data_file`). Never overwrites.
#[tauri::command]
pub async fn move_files(
    workspace_root: String,
    paths: Vec<String>,
    dest_dir: String,
) -> Result<MoveReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = Path::new(&workspace_root);
        let dest = Path::new(&dest_dir);
        let mut report = MoveReport {
            moved: Vec::new(),
            failed: Vec::new(),
        };
        for p in paths {
            match move_data_file(root, Path::new(&p), dest) {
                Ok(np) => report.moved.push((p, np.to_string_lossy().into_owned())),
                Err(e) => report.failed.push((p, e.to_string())),
            }
        }
        report
    })
    .await
    .map_err(|e| format!("move failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The columns are kept per file in the workspace that holds it; a file in no
    /// workspace is simply not remembered, never an error.
    #[test]
    fn enum_report_view_round_trips_through_the_sidecar() {
        let root = std::env::temp_dir().join(format!(
            "domainatoredux-gui-{}-enum-view",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(DOT_DIR)).unwrap();
        let file = root.join("a.gb").to_string_lossy().into_owned();
        assert_eq!(get_enum_report_view(file.clone()), None);

        let view = EnumReportView {
            by: Some("contig".into()),
            columns: [("contig".to_string(), vec!["length".to_string()])].into(),
        };
        set_enum_report_view(file.clone(), view.clone()).unwrap();
        assert_eq!(get_enum_report_view(file), Some(view.clone()));

        let outside = std::env::temp_dir().join("no-workspace-here").join("b.gb");
        let outside = outside.to_string_lossy().into_owned();
        set_enum_report_view(outside.clone(), view).unwrap();
        assert_eq!(get_enum_report_view(outside), None);
        let _ = std::fs::remove_dir_all(&root);
    }
}
