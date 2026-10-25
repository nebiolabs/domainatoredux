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
mod cmd_refdb;
mod cmd_tools;
mod cmd_update;
mod cmd_windows;
mod cmd_workspace;
mod external_tools;
mod job_io;
mod menu_ids;
mod reference_dbs;
mod taxonomy_cache;
mod tool_spec_types;
mod tool_specs;
pub mod worker;

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
        .manage(cmd_refdb::RefDbRegistry::default())
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
            cmd_workspace::pick_save_path,
            cmd_workspace::export_files,
            cmd_workspace::rename_file,
            cmd_workspace::open_workspace,
            cmd_workspace::list_folder,
            cmd_workspace::rescan_tree,
            cmd_workspace::get_enum_report_view,
            cmd_workspace::set_enum_report_view,
            cmd_workspace::create_folder,
            cmd_workspace::delete_file,
            cmd_workspace::move_files,
            cmd_files::load_contig_view,
            cmd_files::load_matrix_report,
            cmd_files::load_hmm_logo,
            cmd_files::list_hmm_profiles,
            cmd_files::get_contig_sequences,
            cmd_files::read_text_file,
            cmd_files::write_text_file,
            cmd_files::extract_selection,
            cmd_files::extract_hmms,
            cmd_files::create_text_file,
            cmd_files::count_records,
            cmd_files::save_text_file,
            cmd_files::run_enum_report,
            cmd_files::save_binary_file,
            cmd_files::run_summary_stats,
            cmd_files::resolve_records,
            cmd_files::trace_network_sources,
            cmd_files::render_ssn_html,
            cmd_files::extract_records,
            cmd_tools::list_tools,
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
            cmd_db::list_database_files,
            cmd_db::rename_database,
            cmd_db::delete_database,
            cmd_db::build_database_index,
            cmd_db::cancel_database_index,
            cmd_db::reveal_database,
            cmd_edit::set_feature_color,
            cmd_edit::set_color_by_name,
            cmd_edit::remove_feature,
            cmd_edit::add_feature,
            cmd_windows::open_sequence_window,
            cmd_windows::open_file_window,
            cmd_windows::open_hmm_logo_window,
            cmd_windows::standalone_params,
            cmd_windows::quit_app,
            cmd_external::list_external_tools,
            cmd_external::download_external_tool,
            cmd_refdb::list_reference_dbs,
            cmd_refdb::check_reference_db_version,
            cmd_refdb::download_reference_db,
            cmd_refdb::cancel_reference_db,
            cmd_refdb::remove_reference_db,
            cmd_refdb::sync_reference_dbs,
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

    /// Two frontend modules whose paths differ only by case are a Linux-only build.
    ///
    /// macOS (APFS) and Windows (NTFS) are case-insensitive, so there
    /// `import ... from "./contextmenu.svelte"` resolves to the sibling component
    /// `ContextMenu.svelte` rather than to `contextmenu.svelte.ts` — and the
    /// component ends up importing itself. Rollup reports it as
    /// `"ctx" is not exported by "src/lib/ContextMenu.svelte", imported by
    /// "src/lib/ContextMenu.svelte"`, which says nothing about case and is
    /// unreproducible on the machine this is developed on. It shipped exactly once,
    /// as a green Linux build and two red cross-compiles.
    ///
    /// The collision is between *module specifiers*, not filenames, so the extension
    /// is stripped first: `Foo.svelte` and `foo.svelte.ts` both answer to
    /// `./foo.svelte`. This walks the tree rather than `include_str!`-ing a list,
    /// because the failure arrives with a *new* file; `cargo test` re-runs the test
    /// binary on every invocation, so the scan is not cached away.
    #[test]
    fn no_two_frontend_modules_differ_only_by_case() {
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).expect("frontend src/ is readable") {
                let path = entry.expect("readable dir entry").path();
                if path.is_dir() {
                    walk(&path, out);
                } else {
                    out.push(path);
                }
            }
        }

        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src");
        let mut files = Vec::new();
        walk(&root, &mut files);

        let mut seen: std::collections::HashMap<String, std::path::PathBuf> =
            std::collections::HashMap::new();
        for path in files {
            let rel = path.strip_prefix(&root).unwrap().to_owned();
            let name = rel.to_string_lossy().to_string();
            // Strip what a bare `./foo` specifier may omit, longest suffix first.
            let stem = [".svelte.ts", ".svelte", ".ts", ".js"]
                .iter()
                .find_map(|ext| name.strip_suffix(ext))
                .unwrap_or(&name);
            let key = stem.to_lowercase();
            if let Some(other) = seen.insert(key, rel.clone()) {
                panic!(
                    "`src/{}` and `src/{}` differ only by case (and extension). On \
                     macOS and Windows one import specifier resolves to whichever the \
                     filesystem hands back, so the build breaks there and nowhere \
                     else. Rename one of them.",
                    other.display(),
                    rel.display()
                );
            }
        }
    }
}
