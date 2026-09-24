//! Detached sequence-viewer windows (SSN node → open its source sequence).
//!
//! The new window loads the same frontend with `?view=contig`; it reads its own
//! label and asks for the (path, contig) via `standalone_params` — so paths never
//! need URL-encoding.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Parameters for pending standalone windows, keyed by window label.
#[derive(Default)]
pub struct PendingWindows(pub Mutex<HashMap<String, (String, usize)>>);

/// Lock that survives a poisoned mutex. A panic in one window's handler must not
/// disable pop-out windows for the rest of the session (the map is plain data —
/// there is no invariant a panic could have left half-applied).
fn lock_pending(p: &PendingWindows) -> MutexGuard<'_, HashMap<String, (String, usize)>> {
    p.0.lock().unwrap_or_else(|e| e.into_inner())
}

/// Sizing for a detached window.
struct WinSpec {
    url: &'static str,
    w: f64,
    h: f64,
    min_w: f64,
    min_h: f64,
}

/// Build a detached window. **Every caller must be an `async` command**, so that
/// this runs on the async runtime and NOT on the main thread.
///
/// That is load-bearing, not a style choice. `Context::send_user_message`
/// (tauri-runtime-wry) short-circuits to running a message *inline* whenever the
/// caller is already on the main thread, and only queues it on the event-loop
/// proxy otherwise:
///
/// ```text
/// if current_thread().id() == context.main_thread_id { handle_user_message(..) }
/// else { context.proxy.send_event(message) }
/// ```
///
/// Tauri v2 runs a synchronous `#[tauri::command]` on the main thread, so calling
/// `build()` from one — directly *or* via `run_on_main_thread`, which is just
/// `send_user_message(Message::Task(..))` and therefore also inlines — creates the
/// WebView2 window nested inside WebView2's own IPC dispatch. wry then spins a
/// nested Win32 message pump waiting for the WebView2 controller, and tao's event
/// loop never returns to the top: the window appears and paints (WebView2 renders
/// out of process) while every queued user event and **every IPC response is
/// buffered forever**. Symptom on Windows: the pop-out shows its loading state,
/// the main window still handles clicks, and nothing in the app ever loads again.
///
/// From the async runtime the proxy branch is taken, `Message::CreateWindow` is
/// handled exactly once at the top of the event loop, and `Context::create_window`
/// is fire-and-forget (it waits on no channel), so there is nothing to deadlock on.
fn spawn_window(app: &AppHandle, label: String, title: String, spec: WinSpec) -> Result<(), String> {
    let mut builder = WebviewWindowBuilder::new(app, &label, WebviewUrl::App(spec.url.into()))
        .title(title)
        .inner_size(spec.w, spec.h)
        .min_inner_size(spec.min_w, spec.min_h);

    // A reduced menu: workspace and tool commands belong to the main window's
    // state, and a detached viewer has none of it. On macOS the menu is app-wide
    // (`Window::set_menu` is unsupported there) so these windows share the main
    // menu -- the platform norm -- and the frontend dispatcher is the backstop,
    // no-oping workspace commands when it is running standalone.
    #[cfg(not(target_os = "macos"))]
    {
        builder = builder.menu(
            crate::app_menu::build_detached(app)
                .map_err(|e| format!("building the detached window menu: {e}"))?,
        );
    }

    let window = builder.build().map_err(|e| format!("opening detached window: {e}"))?;

    // Drop the window's parameters when it closes. Keeping them until then (rather
    // than removing them on the first `standalone_params` read) means a reload of
    // the detached window — an HMR refresh in dev, say — still finds them.
    let app = app.clone();
    window.on_window_event(move |e| {
        if matches!(e, WindowEvent::Destroyed) {
            lock_pending(&app.state::<PendingWindows>()).remove(&label);
        }
    });
    Ok(())
}

/// Open a detached window showing `path` focused on `contig_index`.
#[tauri::command]
pub async fn open_sequence_window(
    app: AppHandle,
    pending: tauri::State<'_, PendingWindows>,
    path: String,
    contig_index: usize,
    title: String,
) -> Result<(), String> {
    let label = format!("seq{}", COUNTER.fetch_add(1, Ordering::Relaxed));
    lock_pending(&pending).insert(label.clone(), (path, contig_index));
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
pub async fn open_file_window(
    app: AppHandle,
    pending: tauri::State<'_, PendingWindows>,
    path: String,
    title: String,
) -> Result<(), String> {
    let label = format!("file{}", COUNTER.fetch_add(1, Ordering::Relaxed));
    lock_pending(&pending).insert(label.clone(), (path, 0));
    spawn_window(
        &app,
        label,
        title,
        WinSpec { url: "index.html?view=file", w: 1200.0, h: 800.0, min_w: 700.0, min_h: 400.0 },
    )
}

/// Open a detached window showing the HMM logo for profile `index` of `path`.
#[tauri::command]
pub async fn open_hmm_logo_window(
    app: AppHandle,
    pending: tauri::State<'_, PendingWindows>,
    path: String,
    index: usize,
    title: String,
) -> Result<(), String> {
    let label = format!("hmm{}", COUNTER.fetch_add(1, Ordering::Relaxed));
    lock_pending(&pending).insert(label.clone(), (path, index));
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
    lock_pending(&pending).get(&label).cloned()
}

/// File → Quit: ask every window to close, detached viewers first and the main
/// window last.
///
/// `Window::close` raises the same close request as the title-bar ×, so each
/// window's `onCloseRequested` guard still gets to ask about unsaved edits, and a
/// window whose user picks Cancel simply stays open. `AppHandle::exit` (or the
/// predefined Quit item, which is `PostQuitMessage` on Windows) would end the
/// process without any window being asked. Once the last window is gone Tauri
/// exits on its own.
///
/// `async` so the closes are posted from off the main thread (see
/// [`spawn_window`]'s note on WebView2 re-entrancy).
#[tauri::command]
pub async fn quit_app(app: AppHandle) {
    let mut windows: Vec<_> = app.webview_windows().into_values().collect();
    windows.sort_by_key(|w| w.label() == "main");
    for w in windows {
        if let Err(e) = w.close() {
            eprintln!("could not close window {}: {e}", w.label());
        }
    }
}
