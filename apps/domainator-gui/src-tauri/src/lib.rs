//! Domainator GUI — Tauri backend. Thin `#[tauri::command]`s delegating to the
//! reusable `domainator_core::app_support` seam (Phase 1: workspace browsing +
//! read-only contig viewer).

mod app_menu;
mod cmd_about;
mod cmd_db;
mod cmd_edit;
mod cmd_external;
mod cmd_files;
mod cmd_open;
mod cmd_prefs;
mod cmd_update;
mod cmd_tools;
mod cmd_windows;
mod cmd_workspace;
mod menu_ids;
mod external_tools;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Remember the main window's size/position/maximized across launches.
        // Detached viewer windows get generated labels (`hmm0`, `file3`, …), so
        // persisting their geometry is meaningless and would just accumulate dead
        // entries in the state file — the filter keeps them out.
        // Registered so the Rust side can hand URLs and paths to the OS. Note
        // there is intentionally NO `opener:` permission in
        // `capabilities/default.json`: the webview must go through
        // `cmd_open`'s validating commands, never the plugin directly.
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_filter(|label| label == "main")
                .build(),
        )
        .manage(app_menu::MenuRegistry::default())
        .manage(cmd_tools::JobRegistry::default())
        .manage(cmd_windows::PendingWindows::default())
        .manage(cmd_files::SeqCacheState::default())
        .manage(cmd_files::RecordCountState::default())
        .manage(cmd_files::HmmIndexState::default())
        .manage(cmd_files::RecordIdIndexState::default())
        .setup(|app| {
            // Wire up any already-downloaded external tools (sets their env vars).
            external_tools::ensure_env(app.handle());
            // Built in `setup` rather than via `Builder::menu` so the registry is
            // already managed and can record the items for later enable/disable.
            let menu = app_menu::build_main(app.handle())
                .expect("building the application menu (a bad accelerator?)");
            app.set_menu(menu).expect("installing the application menu");
            Ok(())
        })
        .on_menu_event(app_menu::route)
        .invoke_handler(tauri::generate_handler![
            cmd_workspace::pick_directory,
            cmd_workspace::pick_files,
            cmd_workspace::import_files,
            cmd_workspace::rename_file,
            cmd_workspace::open_workspace,
            cmd_workspace::list_folder,
            cmd_workspace::rescan_tree,
            cmd_workspace::create_folder,
            cmd_workspace::delete_file,
            cmd_files::load_contig_view,
            cmd_files::load_matrix_report,
            cmd_files::load_hmm_logo,
            cmd_files::list_hmm_profiles,
            cmd_files::get_contig_sequences,
            cmd_files::read_text_file,
            cmd_files::write_text_file,
            cmd_files::extract_contigs,
            cmd_files::extract_region,
            cmd_files::extract_hmms,
            cmd_files::create_text_file,
            cmd_files::count_records,
            cmd_files::save_text_file,
            cmd_files::run_enum_report,
            cmd_files::run_summary_stats,
            cmd_files::resolve_records,
            cmd_files::trace_network_sources,
            cmd_files::render_ssn_html,
            cmd_files::extract_records,
            cmd_tools::list_tools,
            cmd_tools::tool_schema,
            cmd_tools::run_tool,
            cmd_tools::job_status,
            cmd_tools::list_jobs,
            cmd_tools::load_jobs,
            cmd_tools::delete_job,
            cmd_tools::clear_jobs,
            cmd_tools::cancel_job,
            cmd_db::pick_file,
            cmd_db::list_databases,
            cmd_db::register_database,
            cmd_db::unregister_database,
            cmd_edit::set_feature_color,
            cmd_edit::set_color_by_name,
            cmd_edit::remove_feature,
            cmd_edit::add_feature,
            cmd_windows::open_sequence_window,
            cmd_windows::open_file_window,
            cmd_windows::open_hmm_logo_window,
            cmd_windows::standalone_params,
            cmd_external::list_external_tools,
            cmd_external::download_external_tool,
            app_menu::set_menu_state,
            cmd_about::app_diagnostics,
            cmd_open::open_external_url,
            cmd_open::reveal_in_file_manager,
            cmd_open::open_app_data_dir,
            cmd_prefs::get_prefs,
            cmd_prefs::startup_info,
            cmd_prefs::forget_recent_workspace,
            cmd_prefs::set_prefs,
            cmd_update::check_for_updates,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Domainator GUI");
}

#[cfg(test)]
mod tests {
    /// The app version must have exactly one source: `workspace.package.version` in
    /// the root `Cargo.toml`, which reaches the binary as `CARGO_PKG_VERSION` and the
    /// bundle through Tauri's "no `version` in the config → read it from Cargo"
    /// fallback.
    ///
    /// It used to live in three places — here, `package.json`, and the workspace —
    /// which is two chances to ship an installer whose version disagrees with the
    /// `engine_version` this same binary stamps into every job's provenance
    /// (`cmd_tools.rs`). Re-adding the key is the one way to recreate that, so this
    /// guard is simply "the list of extra copies stays empty".
    #[test]
    fn the_app_version_is_not_pinned_in_the_tauri_config() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert!(
            conf.get("version").is_none(),
            "tauri.conf.json pins a `version` again. Remove it: with the key absent \
             the bundler reads workspace.package.version from Cargo.toml, which is \
             also what CARGO_PKG_VERSION (and therefore the About dialog and job \
             provenance) resolves to. Pinning it here re-creates the drift."
        );
    }
}
