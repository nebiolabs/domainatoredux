//! Contig-annotation editing commands (GenBank write-back).
//!
//! Each edit reads, mutates, and rewrites the whole GenBank file, which can be
//! multiple megabytes — so every command runs off Tauri's main thread via
//! `spawn_blocking` (a synchronous command would freeze all IPC during the write).

use domainator_core::app_support::contig_edit;

#[tauri::command]
pub async fn set_feature_color(
    path: String,
    contig_index: usize,
    feat_index: usize,
    hex: String,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        contig_edit::set_feature_color(&path, contig_index, feat_index, &hex)
    })
    .await
    .map_err(|e| format!("edit failed: {e}"))?
}

#[tauri::command]
pub async fn set_color_by_name(path: String, name: String, hex: String) -> Result<usize, String> {
    tauri::async_runtime::spawn_blocking(move || contig_edit::set_color_by_name(&path, &name, &hex))
        .await
        .map_err(|e| format!("edit failed: {e}"))?
}

#[tauri::command]
pub async fn remove_feature(
    path: String,
    contig_index: usize,
    feat_index: usize,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        contig_edit::remove_feature(&path, contig_index, feat_index)
    })
    .await
    .map_err(|e| format!("edit failed: {e}"))?
}

#[tauri::command]
pub async fn add_feature(
    path: String,
    contig_index: usize,
    kind: String,
    start: i64,
    end: i64,
    strand: i32,
    qualifiers: Vec<(String, String)>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        contig_edit::add_feature(&path, contig_index, &kind, start, end, strand, &qualifiers)
    })
    .await
    .map_err(|e| format!("edit failed: {e}"))?
}
