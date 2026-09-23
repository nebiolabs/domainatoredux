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

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch directory unique to this test binary + name, so the file-writing
    /// tests do not collide with each other or leave anything in the repo.
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("domainator-contig-edit-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Write a two-feature GenBank file and return its path.
    fn fixture(name: &str) -> String {
        let mut rec = crate::seq::SeqRecord {
            id: "c1".into(),
            name: "c1".into(),
            description: String::new(),
            seq: vec![b'a'; 60],
            molecule_type: Some("DNA".into()),
            topology: crate::seq::Topology::Linear,
            annotations: crate::seq::Annotations::default(),
            features: Vec::new(),
        };
        for (kind, dom) in [("CDS", "alpha"), ("CDS", "beta")] {
            let mut f = SeqFeature::new(kind, Location::range(0, 30, 1));
            f.set("name", dom);
            rec.features.push(f);
        }
        let path = scratch(name).join("c.gb").to_string_lossy().into_owned();
        write_genbank(&path, &[rec]).unwrap();
        path
    }

    fn features(path: &str) -> Vec<crate::seq::SeqFeature> {
        read_genbank(path).unwrap().into_iter().next().unwrap().features
    }

    #[test]
    fn hex_is_normalized_to_uppercase_with_a_leading_hash() {
        assert_eq!(normalize_hex("aabbcc").unwrap(), "#AABBCC");
        assert_eq!(normalize_hex("#AaBbCc").unwrap(), "#AABBCC");
        assert_eq!(normalize_hex("  #123abc ").unwrap(), "#123ABC");
        for bad in ["#12345", "#1234567", "gggggg", "", "#"] {
            assert!(normalize_hex(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn setting_one_features_color_persists_and_leaves_the_rest_alone() {
        let path = fixture("one-color");
        set_feature_color(&path, 0, 1, "ff0000").unwrap();
        let feats = features(&path);
        assert_eq!(feats[1].get_first("Color"), Some("#FF0000"));
        assert_eq!(feats[0].get_first("Color"), None);
    }

    #[test]
    fn coloring_by_name_reports_how_many_it_changed() {
        let path = fixture("by-name");
        assert_eq!(set_color_by_name(&path, "alpha", "#00ff00").unwrap(), 1);
        assert_eq!(set_color_by_name(&path, "nothing-called-this", "#00ff00").unwrap(), 0);
        let feats = features(&path);
        assert_eq!(feats[0].get_first("Color"), Some("#00FF00"));
        assert_eq!(feats[1].get_first("Color"), None);
    }

    #[test]
    fn features_can_be_added_and_removed() {
        let path = fixture("add-remove");
        add_feature(&path, 0, "misc_feature", 10, 20, -1, &[("note".into(), "hi".into())]).unwrap();
        let feats = features(&path);
        assert_eq!(feats.len(), 3);
        let added = feats.iter().find(|f| f.kind == "misc_feature").unwrap();
        assert_eq!(added.get_first("note"), Some("hi"));
        assert_eq!(added.location.parts[0], (10, 20, -1, false, false));

        remove_feature(&path, 0, 0).unwrap();
        assert_eq!(features(&path).len(), 2);
    }

    /// Every entry point takes raw indices from the viewer, so each has to reject
    /// an out-of-range one rather than panic in the middle of a file rewrite.
    #[test]
    fn out_of_range_indices_are_errors_not_panics() {
        let path = fixture("ranges");
        assert!(set_feature_color(&path, 9, 0, "#ffffff").is_err());
        assert!(set_feature_color(&path, 0, 9, "#ffffff").is_err());
        assert!(remove_feature(&path, 9, 0).is_err());
        assert!(remove_feature(&path, 0, 9).is_err());
        assert!(add_feature(&path, 9, "CDS", 0, 1, 1, &[]).is_err());
        // ...and the file is untouched by any of them.
        assert_eq!(features(&path).len(), 2);
    }

    #[test]
    fn backwards_or_negative_coordinates_are_rejected() {
        let path = fixture("coords");
        assert!(add_feature(&path, 0, "CDS", 20, 10, 1, &[]).is_err());
        assert!(add_feature(&path, 0, "CDS", -1, 10, 1, &[]).is_err());
        // A zero-length feature at a point is legal.
        assert!(add_feature(&path, 0, "CDS", 5, 5, 1, &[]).is_ok());
    }

    #[test]
    fn editing_is_genbank_only() {
        // FASTA has nowhere to put a feature or a color, so it is refused rather
        // than silently dropping the edit.
        let dir = scratch("fasta");
        let path = dir.join("x.fasta");
        std::fs::write(&path, ">a\nACGT\n").unwrap();
        let path = path.to_string_lossy().into_owned();
        let err = set_color_by_name(&path, "alpha", "#ffffff").unwrap_err();
        assert!(err.contains("only supported for GenBank"), "{err}");
        assert!(add_feature(&path, 0, "CDS", 0, 1, 1, &[]).is_err());
    }
}
