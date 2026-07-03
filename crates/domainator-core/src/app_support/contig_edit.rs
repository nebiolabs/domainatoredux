//! In-place annotation editing for the contig viewer: set feature colors, add,
//! and remove features, persisted by rewriting the GenBank file through the
//! standard writer (which regenerates `domainator_<db>` rollups and sorts features
//! by start, exactly like every other pass-through tool). GenBank only — FASTA has
//! nowhere to store features/colors.

use crate::io::genbank::{read_genbank, write_genbank};
use crate::io::{detect_filetype, FileType};
use crate::seq::{Location, SeqFeature};

fn require_genbank(path: &str) -> Result<Vec<crate::seq::SeqRecord>, String> {
    match detect_filetype(std::path::Path::new(path)) {
        Some(FileType::Genbank) => read_genbank(path).map_err(|e| format!("reading GenBank: {e}")),
        _ => Err("annotation editing is only supported for GenBank files".into()),
    }
}

fn write_back(path: &str, records: &[crate::seq::SeqRecord]) -> Result<(), String> {
    write_genbank(path, records).map_err(|e| format!("writing GenBank: {e}"))
}

/// Normalize `(#?)([0-9A-Fa-f]{6})` to `#RRGGBB`.
fn normalize_hex(raw: &str) -> Result<String, String> {
    let s = raw.trim().trim_start_matches('#');
    if s.len() == 6 && s.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(format!("#{}", s.to_ascii_uppercase()))
    } else {
        Err(format!("invalid color hex: {raw:?}"))
    }
}

/// Set the `Color` qualifier of one feature (by contig + feature index).
pub fn set_feature_color(path: &str, contig_index: usize, feat_index: usize, hex: &str) -> Result<(), String> {
    let hex = normalize_hex(hex)?;
    let mut records = require_genbank(path)?;
    let rec = records.get_mut(contig_index).ok_or("contig index out of range")?;
    let feat = rec.features.get_mut(feat_index).ok_or("feature index out of range")?;
    feat.set("Color", hex);
    write_back(path, &records)
}

/// Set the `Color` qualifier on every feature whose `name` qualifier equals
/// `name` (across all contigs). Returns the number changed.
pub fn set_color_by_name(path: &str, name: &str, hex: &str) -> Result<usize, String> {
    let hex = normalize_hex(hex)?;
    let mut records = require_genbank(path)?;
    let mut n = 0;
    for rec in &mut records {
        for feat in &mut rec.features {
            if feat.get_first("name") == Some(name) {
                feat.set("Color", hex.clone());
                n += 1;
            }
        }
    }
    write_back(path, &records)?;
    Ok(n)
}

/// Remove one feature (by contig + feature index).
pub fn remove_feature(path: &str, contig_index: usize, feat_index: usize) -> Result<(), String> {
    let mut records = require_genbank(path)?;
    let rec = records.get_mut(contig_index).ok_or("contig index out of range")?;
    if feat_index >= rec.features.len() {
        return Err("feature index out of range".into());
    }
    rec.features.remove(feat_index);
    write_back(path, &records)
}

/// Add a feature to a contig. Coordinates are 0-based half-open; `strand` is
/// `1`/`-1`/`0`. `qualifiers` are appended in order.
pub fn add_feature(
    path: &str,
    contig_index: usize,
    kind: &str,
    start: i64,
    end: i64,
    strand: i32,
    qualifiers: &[(String, String)],
) -> Result<(), String> {
    if start < 0 || end < start {
        return Err(format!("invalid coordinates {start}..{end}"));
    }
    let mut records = require_genbank(path)?;
    let rec = records.get_mut(contig_index).ok_or("contig index out of range")?;
    let mut feat = SeqFeature::new(kind, Location::range(start, end, strand));
    for (k, v) in qualifiers {
        feat.push(k, v.clone());
    }
    rec.features.push(feat);
    write_back(path, &records)
}
