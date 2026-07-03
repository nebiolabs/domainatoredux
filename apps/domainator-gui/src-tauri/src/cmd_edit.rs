//! Contig-annotation editing commands (GenBank write-back).

use domainator_core::app_support::contig_edit;

#[tauri::command]
pub fn set_feature_color(
    path: String,
    contig_index: usize,
    feat_index: usize,
    hex: String,
) -> Result<(), String> {
    contig_edit::set_feature_color(&path, contig_index, feat_index, &hex)
}

#[tauri::command]
pub fn set_color_by_name(path: String, name: String, hex: String) -> Result<usize, String> {
    contig_edit::set_color_by_name(&path, &name, &hex)
}

#[tauri::command]
pub fn remove_feature(path: String, contig_index: usize, feat_index: usize) -> Result<(), String> {
    contig_edit::remove_feature(&path, contig_index, feat_index)
}

#[tauri::command]
pub fn add_feature(
    path: String,
    contig_index: usize,
    kind: String,
    start: i64,
    end: i64,
    strand: i32,
    qualifiers: Vec<(String, String)>,
) -> Result<(), String> {
    contig_edit::add_feature(&path, contig_index, &kind, start, end, strand, &qualifiers)
}
