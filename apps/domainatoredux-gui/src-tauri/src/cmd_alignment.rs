//! The alignment view's commands: an aligned FASTA file summarized, read a window at
//! a time, and rows exported, over `app_support::alignment`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use domainator_core::app_support::alignment::{rows_as_records, summarize, window, AlignmentInfo};
use domainator_core::io::alignment::{read_alignment, Alignment};

use crate::cmd_files::{notify_files_changed, validated_target};

/// The last alignment read, keyed by its path, size and modification time: the view
/// asks for a window on every scroll, and re-reading the file each time would be the
/// whole cost of scrolling.
type Cached = (PathBuf, u64, Option<SystemTime>, Arc<Alignment>);
static CACHE: Mutex<Option<Cached>> = Mutex::new(None);

fn load(path: &str) -> Result<Arc<Alignment>, String> {
    let p = Path::new(path);
    let meta = std::fs::metadata(p).map_err(|e| format!("reading {path}: {e}"))?;
    let (len, mtime) = (meta.len(), meta.modified().ok());
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((cp, cl, cm, al)) = cache.as_ref() {
        if cp == p && *cl == len && *cm == mtime {
            return Ok(al.clone());
        }
    }
    let alignment = Arc::new(read_alignment(p).map_err(|e| e.to_string())?);
    *cache = Some((p.to_path_buf(), len, mtime, alignment.clone()));
    Ok(alignment)
}

/// The row names, size, alphabet and per-column tracks of an aligned FASTA file, or
/// why it is not an alignment.
#[tauri::command]
pub async fn alignment_info(path: String) -> Result<AlignmentInfo, String> {
    tauri::async_runtime::spawn_blocking(move || Ok(summarize(&*load(&path)?)))
        .await
        .map_err(|e| format!("reading the alignment failed: {e}"))?
}

/// The residues of a block of rows and columns.
#[tauri::command]
pub async fn alignment_window(
    path: String,
    row_start: usize,
    row_count: usize,
    col_start: usize,
    col_count: usize,
) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        Ok(window(
            &*load(&path)?,
            row_start,
            row_count,
            col_start,
            col_count,
        ))
    })
    .await
    .map_err(|e| format!("reading the alignment failed: {e}"))?
}

/// Write the rows named by `ids` to a new FASTA file `dir/name`, with their gaps or
/// without, and return its path.
#[tauri::command]
pub async fn export_alignment_rows(
    app: tauri::AppHandle,
    path: String,
    dir: String,
    name: String,
    ids: Vec<String>,
    gapped: bool,
) -> Result<String, String> {
    let dir_c = dir.clone();
    let out = tauri::async_runtime::spawn_blocking(move || {
        let target = validated_target(&dir, &name)?;
        let records = rows_as_records(&*load(&path)?, &ids, gapped);
        if records.is_empty() {
            return Err("none of the selected rows are in the alignment".to_string());
        }
        let ts = target.to_string_lossy().into_owned();
        domainator_core::io::fasta::write_fasta(&ts, &records)
            .map_err(|e| format!("writing FASTA: {e}"))?;
        Ok(ts)
    })
    .await
    .map_err(|e| format!("export failed: {e}"))??;
    notify_files_changed(&app, &dir_c);
    Ok(out)
}
