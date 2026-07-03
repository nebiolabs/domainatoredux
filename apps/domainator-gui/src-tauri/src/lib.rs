//! Domainator GUI — Tauri backend. Thin `#[tauri::command]`s delegating to the
//! reusable `domainator_core::app_support` seam (Phase 1: workspace browsing +
//! read-only contig viewer).

mod cmd_db;
mod cmd_edit;
mod cmd_files;
mod cmd_tools;
mod cmd_workspace;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(cmd_tools::JobRegistry::default())
        .invoke_handler(tauri::generate_handler![
            cmd_workspace::pick_directory,
            cmd_workspace::open_workspace,
            cmd_workspace::list_folder,
            cmd_files::load_contig_view,
            cmd_files::load_matrix_report,
            cmd_files::load_hmm_logo,
            cmd_files::run_enum_report,
            cmd_files::resolve_record,
            cmd_files::render_ssn_html,
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running Domainator GUI");
}
