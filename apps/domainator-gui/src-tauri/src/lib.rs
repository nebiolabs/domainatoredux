//! Domainator GUI — Tauri backend. Thin `#[tauri::command]`s delegating to the
//! reusable `domainator_core::app_support` seam (Phase 1: workspace browsing +
//! read-only contig viewer).

mod cmd_db;
mod cmd_edit;
mod cmd_external;
mod cmd_files;
mod cmd_tools;
mod cmd_windows;
mod cmd_workspace;
mod external_tools;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(cmd_tools::JobRegistry::default())
        .manage(cmd_windows::PendingWindows::default())
        .manage(cmd_files::SeqCacheState::default())
        .manage(cmd_files::RecordCountState::default())
        .setup(|app| {
            // Wire up any already-downloaded external tools (sets their env vars).
            external_tools::ensure_env(&app.handle());
            Ok(())
        })
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
            cmd_files::extract_hmms,
            cmd_files::create_text_file,
            cmd_files::count_records,
            cmd_files::save_text_file,
            cmd_files::run_enum_report,
            cmd_files::run_summary_stats,
            cmd_files::resolve_record,
            cmd_files::render_ssn_html,
            cmd_files::extract_records,
            cmd_tools::list_tools,
            cmd_tools::tool_schema,
            cmd_tools::run_tool,
            cmd_tools::job_status,
            cmd_tools::list_jobs,
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running Domainator GUI");
}
