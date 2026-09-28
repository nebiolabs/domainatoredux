//! Handing a URL or a path to the operating system.
//!
//! The plugin is registered in `lib.rs` but deliberately granted **no `opener:`
//! permission** in `capabilities/default.json`. Capabilities gate the JS→Rust IPC
//! surface, not Rust-side calls, so the webview cannot invoke
//! `plugin:opener|open_url` at all — including the SSN viewer's generated HTML,
//! which runs in a sandboxed iframe in this same app. The only way out is these
//! two commands, and both validate first.
//!
//! That is why this is a plugin rather than a hand-rolled subprocess, too:
//! `cmd /c start <url>` treats `&`, `^` and quotes as syntax (an injection
//! surface, and it swallows the first quoted argument as a window title), and
//! there is no "select this file in its folder" on Windows without COM.

use domainator_core::app_support::{about, workspace};
use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;

/// Open one of the app's own URLs in the user's browser.
///
/// Refuses anything not on the allow-list, so the frontend can ask for *a* link
/// but never name an arbitrary destination.
#[tauri::command]
pub async fn open_external_url(app: AppHandle, url: String) -> Result<(), String> {
    if !about::is_allowed_url(&url) {
        return Err(format!(
            "refusing to open {url}: not a Domainator project link"
        ));
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| format!("opening link: {e}"))
}

/// Show `path` in the OS file manager, selected.
///
/// Bounded to the open workspace or the app's own data directory: those are the
/// only places the GUI has any business pointing at, and the path reaching here
/// came from the frontend.
#[tauri::command]
pub async fn reveal_in_file_manager(
    app: AppHandle,
    workspace_root: Option<String>,
    path: String,
) -> Result<(), String> {
    let target = std::path::Path::new(&path);
    if !target.exists() {
        return Err(format!("{path} no longer exists"));
    }

    let app_data = app.path().app_data_dir().ok();
    let in_workspace = workspace_root
        .as_deref()
        .map(std::path::Path::new)
        .is_some_and(|root| workspace::is_within(root, target));
    let in_app_data = app_data
        .as_deref()
        .is_some_and(|d| workspace::is_within(d, target));

    if !in_workspace && !in_app_data {
        return Err(format!("refusing to reveal {path}: outside the workspace"));
    }

    // Fails on a bare WSLg session (no org.freedesktop.FileManager1 provider), so
    // the message has to be readable rather than a bare DBus error.
    app.opener()
        .reveal_item_in_dir(target)
        .map_err(|e| format!("could not open a file manager for {path}: {e}"))
}

/// Reveal the app's own data directory (Preferences → Locations, About).
#[tauri::command]
pub async fn open_app_data_dir(app: AppHandle) -> Result<(), String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    app.opener()
        .reveal_item_in_dir(&dir)
        .map_err(|e| format!("could not open a file manager for {}: {e}", dir.display()))
}
