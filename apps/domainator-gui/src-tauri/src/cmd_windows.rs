//! Detached sequence-viewer windows (SSN node → open its source sequence).
//!
//! The new window loads the same frontend with `?view=contig`; it reads its own
//! label and asks for the (path, contig) via `standalone_params` — so paths never
//! need URL-encoding.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use tauri::{AppHandle, WebviewUrl, WebviewWindowBuilder};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Parameters for pending standalone windows, keyed by window label.
#[derive(Default)]
pub struct PendingWindows(pub Mutex<HashMap<String, (String, usize)>>);

/// Open a detached window showing `path` focused on `contig_index`.
#[tauri::command]
pub fn open_sequence_window(
    app: AppHandle,
    pending: tauri::State<'_, PendingWindows>,
    path: String,
    contig_index: usize,
    title: String,
) -> Result<(), String> {
    let label = format!("seq{}", COUNTER.fetch_add(1, Ordering::Relaxed));
    pending.0.lock().unwrap().insert(label.clone(), (path, contig_index));
    WebviewWindowBuilder::new(&app, &label, WebviewUrl::App("index.html?view=contig".into()))
        .title(title)
        .inner_size(1100.0, 720.0)
        .min_inner_size(700.0, 400.0)
        .build()
        .map_err(|e| format!("opening window: {e}"))?;
    Ok(())
}

/// Pop a whole file out into its own window (the full viewer with its sub-tabs).
/// The window classifies the file by extension client-side, so only the path is
/// needed (contig index unused → 0).
#[tauri::command]
pub fn open_file_window(
    app: AppHandle,
    pending: tauri::State<'_, PendingWindows>,
    path: String,
    title: String,
) -> Result<(), String> {
    let label = format!("file{}", COUNTER.fetch_add(1, Ordering::Relaxed));
    pending.0.lock().unwrap().insert(label.clone(), (path, 0));
    WebviewWindowBuilder::new(&app, &label, WebviewUrl::App("index.html?view=file".into()))
        .title(title)
        .inner_size(1200.0, 800.0)
        .min_inner_size(700.0, 400.0)
        .build()
        .map_err(|e| format!("opening window: {e}"))?;
    Ok(())
}

/// Open a detached window showing the HMM logo for profile `index` of `path`.
#[tauri::command]
pub fn open_hmm_logo_window(
    app: AppHandle,
    pending: tauri::State<'_, PendingWindows>,
    path: String,
    index: usize,
    title: String,
) -> Result<(), String> {
    let label = format!("hmm{}", COUNTER.fetch_add(1, Ordering::Relaxed));
    pending.0.lock().unwrap().insert(label.clone(), (path, index));
    WebviewWindowBuilder::new(&app, &label, WebviewUrl::App("index.html?view=hmmlogo".into()))
        .title(title)
        .inner_size(1000.0, 520.0)
        .min_inner_size(500.0, 300.0)
        .build()
        .map_err(|e| format!("opening window: {e}"))?;
    Ok(())
}

/// What a standalone window should show (called by that window on load).
#[tauri::command]
pub fn standalone_params(
    pending: tauri::State<'_, PendingWindows>,
    label: String,
) -> Option<(String, usize)> {
    pending.0.lock().unwrap().get(&label).cloned()
}
