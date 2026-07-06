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

/// Sizing for a detached window.
struct WinSpec {
    url: &'static str,
    w: f64,
    h: f64,
    min_w: f64,
    min_h: f64,
}

/// Build a detached window, DEFERRED to the next main-thread tick.
///
/// `WebviewWindowBuilder::build()` blocks until the new webview is created, and on
/// Windows/WebView2 that runs a nested message loop. Calling it directly here —
/// inside the synchronous IPC handler, which is itself dispatched on the main
/// thread — re-enters WebView2 and DEADLOCKS the whole app (symptom: pop-out
/// windows never populate and the program has to be killed). Scheduling the build
/// with `run_on_main_thread` lets this command return first (so the invoking
/// webview's IPC call resolves), and the window is then created at the top of the
/// event loop rather than nested inside IPC dispatch.
fn spawn_window(app: &AppHandle, label: String, title: String, spec: WinSpec) -> Result<(), String> {
    let app2 = app.clone();
    app.run_on_main_thread(move || {
        if let Err(e) = WebviewWindowBuilder::new(&app2, &label, WebviewUrl::App(spec.url.into()))
            .title(title)
            .inner_size(spec.w, spec.h)
            .min_inner_size(spec.min_w, spec.min_h)
            .build()
        {
            eprintln!("opening detached window failed: {e}");
        }
    })
    .map_err(|e| format!("scheduling window: {e}"))
}

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
    spawn_window(
        &app,
        label,
        title,
        WinSpec { url: "index.html?view=contig", w: 1100.0, h: 720.0, min_w: 700.0, min_h: 400.0 },
    )
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
    spawn_window(
        &app,
        label,
        title,
        WinSpec { url: "index.html?view=file", w: 1200.0, h: 800.0, min_w: 700.0, min_h: 400.0 },
    )
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
    spawn_window(
        &app,
        label,
        title,
        WinSpec { url: "index.html?view=hmmlogo", w: 1000.0, h: 520.0, min_w: 500.0, min_h: 300.0 },
    )
}

/// What a standalone window should show (called by that window on load).
#[tauri::command]
pub fn standalone_params(
    pending: tauri::State<'_, PendingWindows>,
    label: String,
) -> Option<(String, usize)> {
    pending.0.lock().unwrap().get(&label).cloned()
}
