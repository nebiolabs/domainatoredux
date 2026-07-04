//! Commands for the external-tool registry (download / status).

use serde::Serialize;
use tauri::AppHandle;

use crate::external_tools;

#[derive(Serialize)]
pub struct ExternalToolStatus {
    pub id: String,
    pub bin_name: String,
    pub version: String,
    pub description: String,
    pub installed: bool,
    pub path: Option<String>,
    /// Whether a prebuilt binary exists for this platform.
    pub available: bool,
    pub platform: String,
}

/// Status of every managed external tool (also wires up already-installed ones).
#[tauri::command]
pub fn list_external_tools(app: AppHandle) -> Vec<ExternalToolStatus> {
    external_tools::ensure_env(&app);
    let plat = external_tools::platform_key().to_string();
    external_tools::specs()
        .into_iter()
        .map(|s| {
            let path = external_tools::installed_path(&app, &s);
            ExternalToolStatus {
                id: s.id.into(),
                bin_name: s.bin_name.clone(),
                version: s.version.into(),
                description: s.description.into(),
                installed: path.is_some(),
                path: path.map(|p| p.to_string_lossy().into_owned()),
                available: s.asset.is_some(),
                platform: plat.clone(),
            }
        })
        .collect()
}

/// Download + install a managed tool by id (off the UI thread). Returns its path.
#[tauri::command]
pub async fn download_external_tool(app: AppHandle, id: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let specs = external_tools::specs();
        let spec = specs.iter().find(|s| s.id == id).ok_or_else(|| format!("unknown tool: {id}"))?;
        external_tools::download(&app, spec).map(|p| p.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| format!("download failed: {e}"))?
}
