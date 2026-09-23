//! The About box's data, assembled in one place.
//!
//! Deliberately a command rather than client-side assembly. `getVersion()` is
//! reachable from JS, but the webview version, the OS/arch and the resolved app
//! paths are not — and the diagnostics blob has to have the *same* shape in every
//! bug report, which it cannot if half of it is composed in the frontend. Keeping
//! it here also means the version on screen and the `engine_version` stamped into
//! job provenance come from the same `CARGO_PKG_VERSION`.

use domainator_core::app_support::about::{render_diagnostics, Diagnostics, ToolLine};
use tauri::{AppHandle, Manager};

use crate::{cmd_prefs, external_tools};

/// [`Diagnostics`] plus the rendered blob, so the dialog never re-derives it.
#[derive(serde::Serialize)]
pub struct AppDiagnostics {
    #[serde(flatten)]
    pub info: Diagnostics,
    pub diagnostics: String,
    pub project_url: String,
    pub issues_url: String,
    pub license: String,
}

/// Everything the About dialog shows.
///
/// `async` so its filesystem probes stay off Tauri's main thread, matching
/// `list_external_tools`.
#[tauri::command(async)]
pub fn app_diagnostics(app: AppHandle) -> Result<AppDiagnostics, String> {
    let pkg = app.package_info();
    let show = |p: Result<std::path::PathBuf, String>| {
        p.map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|e| format!("unavailable ({e})"))
    };

    let info = Diagnostics {
        product: pkg.name.clone(),
        version: pkg.version.to_string(),
        upstream_version: domainator_core::version::UPSTREAM_VERSION.to_string(),
        upstream_commit: domainator_core::version::UPSTREAM_COMMIT.to_string(),
        tauri_version: tauri::VERSION.to_string(),
        // Never fail the whole command over a version probe -- an About box that
        // errors is worse than one missing a line.
        webview_version: tauri::webview_version().unwrap_or_else(|_| "unknown".into()),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        profile: if cfg!(debug_assertions) { "debug" } else { "release" }.to_string(),
        app_data_dir: show(
            app.path().app_data_dir().map_err(|e| format!("app data dir: {e}")),
        ),
        prefs_path: show(cmd_prefs::prefs_path(&app)),
        managed_tools_dir: show(external_tools::managed_dir(&app)),
        // The same source `list_external_tools` reads, so the External tools panel
        // and a pasted bug report can never disagree about what is installed.
        external_tools: external_tools::specs()
            .into_iter()
            .map(|s| ToolLine {
                installed: external_tools::installed_path(&app, &s).is_some(),
                name: s.bin_name.clone(),
                version: s.version.to_string(),
            })
            .collect(),
    };

    Ok(AppDiagnostics {
        diagnostics: render_diagnostics(&info),
        project_url: domainator_core::app_support::about::PROJECT_URL.to_string(),
        issues_url: domainator_core::app_support::about::ISSUES_URL.to_string(),
        license: domainator_core::app_support::about::LICENSE.to_string(),
        info,
    })
}
