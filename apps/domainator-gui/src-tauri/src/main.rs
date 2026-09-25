#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Must stay the first thing main does: nothing before it may touch Tauri,
    // GTK, WebView2 or the window-state plugin, or every worker opens a window.
    // Guarded by `tests/worker.rs::worker_mode_never_starts_tauri`.
    if let Some(code) = domainator_gui::worker::maybe_run_worker() {
        std::process::exit(code);
    }
    domainator_gui::run()
}
