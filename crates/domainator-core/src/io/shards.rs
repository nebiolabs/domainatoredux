//! Database shard naming and resolution — a port of `db_index.py`'s shard logic.
//!
//! A logical name `mydb.gb` resolves to `mydb.0.gb`, `mydb.1.gb`, … (the `.N` is
//! inserted before the format and compression suffixes). Both an unsharded file
//! and shards present is an error (refuse to guess); a leading-zero collision is
//! an error; a non-contiguous `0..N` run warns but proceeds.

use std::path::{Path, PathBuf};

const COMPRESSION_EXTS: [&str; 3] = ["gz", "bgz", "bgzf"];

/// `(dir, stem, fmt_ext, comp_ext)` split of a database path. `fmt_ext` is the
/// sequence-format extension; `comp_ext` is a trailing compression suffix or "".
pub(crate) fn split_db_name(db_path: &Path) -> (PathBuf, String, String, String) {
    let directory = db_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default();
    let base = db_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    let mut parts: Vec<&str> = base.split('.').collect();
    let mut comp = String::new();
    if parts.len() >= 2
        && COMPRESSION_EXTS.contains(&parts.last().unwrap().to_ascii_lowercase().as_str())
    {
        comp = parts.pop().unwrap().to_string();
    }
    let (fmt, stem) = if parts.len() >= 2 {
        let fmt = parts.pop().unwrap().to_string();
        (fmt, parts.join("."))
    } else {
        (
            String::new(),
            parts.first().copied().unwrap_or("").to_string(),
        )
    };
    (directory, stem, fmt, comp)
}

/// Build a shard path from `db_path`. `shard_index = None` means no shard number.
/// `compress`: `Some(true)` -> `.bgz`, `Some(false)` -> none, `None` -> keep input's.
pub fn shard_path(db_path: &Path, shard_index: Option<usize>, compress: Option<bool>) -> PathBuf {
    let (dir, stem, fmt, mut comp) = split_db_name(db_path);
    match compress {
        Some(true) => comp = "bgz".to_string(),
        Some(false) => comp = String::new(),
        None => {}
    }
    let mut name = match shard_index {
        Some(i) if !fmt.is_empty() => format!("{stem}.{i}.{fmt}"),
        Some(i) => format!("{stem}.{i}"),
        None if !fmt.is_empty() => format!("{stem}.{fmt}"),
        None => stem,
    };
    if !comp.is_empty() {
        name = format!("{name}.{comp}");
    }
    if dir.as_os_str().is_empty() {
        PathBuf::from(name)
    } else {
        dir.join(name)
    }
}

/// If `name` is a shard of `(stem, fmt)`, return its shard number. Matches
/// `^{stem}\.(\d+)\.{fmt}(\.(gz|bgz|bgzf))?$`.
fn match_shard(name: &str, stem: &str, fmt: &str) -> Option<u64> {
    let rest = name.strip_prefix(stem)?.strip_prefix('.')?;
    // digits
    let digits_end = rest.find(|c: char| !c.is_ascii_digit())?;
    if digits_end == 0 {
        return None;
    }
    let num: u64 = rest[..digits_end].parse().ok()?;
    let after = &rest[digits_end..];
    let after = after.strip_prefix('.')?;
    let after = after.strip_prefix(fmt)?;
    if after.is_empty() {
        return Some(num);
    }
    let comp = after.strip_prefix('.')?;
    if COMPRESSION_EXTS.contains(&comp.to_ascii_lowercase().as_str()) {
        Some(num)
    } else {
        None
    }
}

/// Expand a logical database path to the ordered list of files to read.
///
/// - only the unsharded file exists -> `[db_path]`
/// - only shards exist -> shard paths sorted numerically
/// - both exist -> `Err` (ambiguous)
/// - neither exists -> `[db_path]`
///
/// A leading-zero collision (`mydb.0.gb` and `mydb.00.gb`) is an error. A
/// non-contiguous `0..N` run is reported via `warnings` (printed to stderr).
pub fn resolve_database_shards(db_path: &Path) -> Result<Vec<PathBuf>, String> {
    let base_exists = db_path.exists();
    let (dir, stem, fmt, _comp) = split_db_name(db_path);

    let mut shards_by_num: std::collections::BTreeMap<u64, PathBuf> =
        std::collections::BTreeMap::new();
    if !fmt.is_empty() {
        let search_dir = if dir.as_os_str().is_empty() {
            PathBuf::from(".")
        } else {
            dir.clone()
        };
        if let Ok(entries) = std::fs::read_dir(&search_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = match name.to_str() {
                    Some(n) => n,
                    None => continue,
                };
                if let Some(num) = match_shard(name, &stem, &fmt) {
                    let full = if dir.as_os_str().is_empty() {
                        PathBuf::from(name)
                    } else {
                        dir.join(name)
                    };
                    if let Some(existing) = shards_by_num.get(&num) {
                        return Err(format!(
                            "Ambiguous shards for '{}': {} and {} both resolve to shard {} (leading-zero collision).",
                            db_path.display(), existing.display(), full.display(), num
                        ));
                    }
                    shards_by_num.insert(num, full);
                }
            }
        }
    }

    if base_exists && !shards_by_num.is_empty() {
        let example = shards_by_num.values().next().unwrap();
        return Err(format!(
            "Both an unsharded database '{}' and shard(s) (e.g. '{}') exist; refusing to guess. Remove one set.",
            db_path.display(), example.display()
        ));
    }

    if !shards_by_num.is_empty() {
        let nums: Vec<u64> = shards_by_num.keys().copied().collect();
        if nums[0] != 0 || nums[nums.len() - 1] != nums.len() as u64 - 1 {
            eprintln!(
                "warning: database shards for '{}' are not a contiguous 0..N sequence (found {:?}); proceeding.",
                db_path.display(), nums
            );
        }
        return Ok(shards_by_num.into_values().collect());
    }

    Ok(vec![db_path.to_path_buf()])
}
