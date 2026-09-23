use std::fs;

use domainator_core::app_support::genbank_summary::{
    parse_domain_names, summarize_records_tsv, SummaryGroupBy, SummaryOptions,
};
use domainator_core::io::genbank::read_genbank;

#[tauri::command]
fn select_genbank_file() -> Result<Option<String>, String> {
    Ok(rfd::FileDialog::new()
        .add_filter("GenBank", &["gb", "gbk", "genbank"])
        .pick_file()
        .map(|path| path.to_string_lossy().into_owned()))
}

#[tauri::command]
fn summarize_genbank(
    input_path: String,
    group_by: String,
    domain_names_text: String,
    evalue_threshold: f64,
) -> Result<String, String> {
    if !evalue_threshold.is_finite() || evalue_threshold < 0.0 {
        return Err("evalue threshold must be a finite non-negative number".into());
    }

    let group_by = match group_by.as_str() {
        "taxid" => SummaryGroupBy::Taxid,
        "definition" => SummaryGroupBy::Definition,
        other => return Err(format!("unknown grouping: {other}")),
    };
    let records = read_genbank(&input_path).map_err(|e| format!("reading GenBank: {e}"))?;
    let opts = SummaryOptions {
        group_by,
        domain_names: parse_domain_names(&domain_names_text),
        evalue: evalue_threshold,
    };
    summarize_records_tsv(&records, &opts)
}

#[tauri::command]
fn save_tsv_file(contents: String) -> Result<Option<String>, String> {
    let Some(path) = rfd::FileDialog::new()
        .add_filter("TSV", &["tsv", "txt"])
        .set_file_name("genbank_summary.tsv")
        .save_file()
    else {
        return Ok(None);
    };

    fs::write(&path, contents).map_err(|e| format!("writing TSV: {e}"))?;
    Ok(Some(path.to_string_lossy().into_owned()))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            select_genbank_file,
            summarize_genbank,
            save_tsv_file
        ])
        .run(tauri::generate_context!())
        .expect("error while running Domainator GenBank Summary");
}

#[cfg(test)]
mod tests {
    /// Same guard as domainator-gui's: with no `version` key the bundler reads
    /// `workspace.package.version`, so the installer cannot disagree with the
    /// workspace release it was built from.
    #[test]
    fn the_app_version_is_not_pinned_in_the_tauri_config() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert!(
            conf.get("version").is_none(),
            "tauri.conf.json pins a `version` again; remove it so the bundle takes \
             workspace.package.version from Cargo.toml"
        );
    }
}
