//! `color_genbank`: assign hex colors to domain and/or CDS features. Port of
//! `domainator.color_genbank`.
//!
//! Domains are colored by domain name (or database id with `--domain_database`).
//! CDSs are colored by their highest-scoring domain. Colors are stored in the
//! `Color` feature qualifier (matches Python's `feature.qualifiers['Color']`,
//! which Benchling/ApE read for feature coloring).
//!
//! Color source: either a user-supplied color table (exact parity) or an
//! auto-generated map. The auto-generated path mirrors Python's
//! `AutoColormap`, which uses `seaborn.color_palette('husl', n_colors=30)`
//! shuffled with `random.Random(12)`. The 30-color husl palette is
//! deterministic, so the shuffled list is hardcoded below (see `AUTO_PALETTE`)
//! — reproducing Python's output byte-for-byte for the seaborn/HSLuv math in the
//! reference environment. If seaborn ever changes its husl implementation this
//! constant would need regenerating; the color-table path is unaffected.

use std::collections::HashSet;

use crate::seq::SeqRecord;
use crate::{DOMAIN_FEATURE_NAME, DOMAIN_SEARCH_BEST_HIT_NAME};

/// What kinds of features to color.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ColorTargets {
    pub domains: bool,
    pub cdss: bool,
}

impl ColorTargets {
    /// Parse the `--mode` value into targets. `domains` | `cdss` | `domains_and_cdss`.
    pub fn parse(mode: &str) -> Result<Self, String> {
        match mode {
            "domains" => Ok(ColorTargets {
                domains: true,
                cdss: false,
            }),
            "cdss" => Ok(ColorTargets {
                domains: false,
                cdss: true,
            }),
            "domains_and_cdss" => Ok(ColorTargets {
                domains: true,
                cdss: true,
            }),
            _ => Err(format!("Invalid color mode: {mode}")),
        }
    }
}

/// The 30-color `husl` palette from `seaborn.color_palette('husl', n_colors=30)`,
/// upper-cased and shuffled with Python's `random.Random(12)` (Fisher-Yates over
/// MT19937). Hardcoded because the palette is a deterministic constant. See the
/// module docs for the parity caveat.
const AUTO_PALETTE: [&str; 30] = [
    "#3BA3EC", "#38A9C5", "#34AF8C", "#AE9D31", "#F77468", "#B287F4", "#39A7D5", "#DC8932",
    "#37AAB9", "#9591F4", "#BB9832", "#F560E4", "#F37A32", "#77AB31", "#50B131", "#E866F4",
    "#9FA131", "#35AE99", "#F66AB7", "#F66DA2", "#CC7AF4", "#F77189", "#33B07A", "#CA9232",
    "#32B25C", "#F565CC", "#36ACAE", "#6E9BF4", "#8EA631", "#36ADA4",
];

/// Normalize a hex color string. Accepts `(#?)([a-fA-F0-9]{6})`; returns
/// `#[A-F0-9]{6}`. Mirrors `color_genbank.normalize_color_hex`.
pub fn normalize_color_hex(color_hex: &str) -> Result<String, String> {
    let s = color_hex.trim().to_ascii_uppercase();
    let body = s.strip_prefix('#').unwrap_or(&s);
    if body.len() == 6 && body.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(format!("#{body}"))
    } else {
        Err(format!("Color code not recognized: {color_hex}"))
    }
}

/// A color source: either a fixed user table or the auto-generated palette. Both
/// preserve insertion order (for `--color_table_out`) and normalize lookups.
pub struct ColorMap {
    /// Ordered (key -> color) so `--color_table_out` writes in first-seen order.
    entries: Vec<(String, String)>,
    /// If true, unknown keys get auto-assigned from [`AUTO_PALETTE`] (Python's
    /// `AutoColormap.__contains__` always returns True). If false, only keys in
    /// the fixed table match (Python's plain dict).
    auto: bool,
    /// Next palette index for auto-assignment.
    next_color: usize,
}

impl ColorMap {
    /// A fixed table from `read_color_table` (color-table path).
    pub fn from_table(table: Vec<(String, String)>) -> Self {
        ColorMap {
            entries: table,
            auto: false,
            next_color: 0,
        }
    }

    /// The auto-generating map (`AutoColormap`).
    pub fn auto() -> Self {
        ColorMap {
            entries: Vec::new(),
            auto: false,
            next_color: 0,
        }
        .with_auto()
    }

    fn with_auto(mut self) -> Self {
        self.auto = true;
        self
    }

    fn find(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// Look up (and, for the auto map, lazily assign) a color for `key`. Returns
    /// `None` only for the fixed-table map when the key is absent.
    fn get(&mut self, key: &str) -> Option<String> {
        if let Some(c) = self.find(key) {
            return Some(c.to_string());
        }
        if self.auto {
            let color = AUTO_PALETTE[self.next_color % AUTO_PALETTE.len()].to_string();
            self.next_color = (self.next_color + 1) % AUTO_PALETTE.len();
            self.entries.push((key.to_string(), color.clone()));
            Some(color)
        } else {
            None
        }
    }

    /// The (key, color) pairs in insertion order (for `--color_table_out`).
    pub fn items(&self) -> &[(String, String)] {
        &self.entries
    }
}

/// Parse a color table (TSV, two columns, no header: `key\thex`). Mirrors
/// `read_color_table` — errors on wrong column count or duplicate keys.
pub fn read_color_table(text: &str) -> Result<Vec<(String, String)>, String> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() != 2 {
            return Err(format!("badly formatted line in color table: {line}"));
        }
        if seen.contains(parts[0]) {
            return Err(format!(
                "domain specified multiple times in color table: {}",
                parts[0]
            ));
        }
        seen.insert(parts[0].to_string());
        out.push((parts[0].to_string(), normalize_color_hex(parts[1])?));
    }
    Ok(out)
}

/// Options for [`color_genbank`].
pub struct ColorGenbankOptions {
    pub targets: ColorTargets,
    /// Optional color for `Domain_Search` best-hit features.
    pub search_hit_color: Option<String>,
    /// Remove all existing `Color` qualifiers before coloring.
    pub clear: bool,
    /// If set, only consider domains from these databases when coloring.
    pub databases: Option<HashSet<String>>,
    /// Qualifier used as the color key: `"name"` (default) or `"database"`.
    pub color_feature: String,
}

/// Color a single record in place using `color_map`. Mirrors
/// `color_seqrecord_by_domain`.
pub fn color_seqrecord_by_domain(
    record: &mut SeqRecord,
    color_map: &mut ColorMap,
    opts: &ColorGenbankOptions,
) {
    let ninf = f64::NEG_INFINITY;
    // cds_id -> (best_score, color); insertion order not needed here.
    let mut new_cds_bitscores: std::collections::HashMap<String, f64> =
        std::collections::HashMap::new();
    let mut new_cds_colors: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    let mut new_search_hit_colors: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

    for feature in &mut record.features {
        if opts.clear {
            feature.qualifiers.retain(|(k, _)| k != "Color");
        }
        if feature.kind == DOMAIN_FEATURE_NAME {
            if let Some(dbs) = &opts.databases {
                let db = feature.get_first("database").unwrap_or("");
                if !dbs.contains(db) {
                    continue;
                }
            }
            let color_key = match feature.get_first(&opts.color_feature) {
                Some(k) => k.to_string(),
                None => continue,
            };
            if let Some(color) = color_map.get(&color_key) {
                if opts.targets.domains {
                    feature.set("Color", color.clone());
                }
                if opts.targets.cdss {
                    if let Some(cds_id) = feature.get_first("cds_id") {
                        let cds_id = cds_id.to_string();
                        let score: f64 = feature
                            .get_first("score")
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(ninf);
                        if score > *new_cds_bitscores.get(&cds_id).unwrap_or(&ninf) {
                            new_cds_bitscores.insert(cds_id.clone(), score);
                            new_cds_colors.insert(cds_id, color);
                        }
                    }
                }
            }
        } else if feature.kind == DOMAIN_SEARCH_BEST_HIT_NAME {
            if let Some(shc) = &opts.search_hit_color {
                if opts.targets.domains {
                    feature.set("Color", shc.clone());
                }
                if opts.targets.cdss {
                    if let Some(cds_id) = feature.get_first("cds_id") {
                        new_search_hit_colors.insert(cds_id.to_string(), shc.clone());
                    }
                }
            }
        }
    }

    // Search-hit colors override domain-derived colors for the same CDS.
    for (cds, color) in new_search_hit_colors {
        new_cds_colors.insert(cds, color);
    }

    if opts.targets.cdss {
        for feature in &mut record.features {
            if feature.kind == "CDS" {
                if let Some(cds_id) = feature.get_first("cds_id") {
                    if let Some(color) = new_cds_colors.get(cds_id) {
                        let color = color.clone();
                        feature.set("Color", color);
                    }
                }
            }
        }
    }
}

/// Color all records in place, assigning colors from `color_map`. Mirrors
/// `color_genbank` (the generator). Returns the records.
pub fn color_genbank(
    mut records: Vec<SeqRecord>,
    color_map: &mut ColorMap,
    opts: &ColorGenbankOptions,
) -> Vec<SeqRecord> {
    for rec in &mut records {
        color_seqrecord_by_domain(rec, color_map, opts);
    }
    records
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seq::{Annotations, Location, SeqFeature, Topology};

    #[test]
    fn normalize_hex_variants() {
        assert_eq!(normalize_color_hex("cc0000").unwrap(), "#CC0000");
        assert_eq!(normalize_color_hex("#Ff00aB").unwrap(), "#FF00AB");
        assert_eq!(normalize_color_hex("  #abcdef ").unwrap(), "#ABCDEF");
        assert!(normalize_color_hex("xyz").is_err());
        assert!(normalize_color_hex("#12345").is_err());
        assert!(normalize_color_hex("1234567").is_err());
    }

    #[test]
    fn parse_color_table() {
        let t = read_color_table("CcdB\tcc0000\nAPH\t#00ff00\n\nCAT\t0000CC\n").unwrap();
        assert_eq!(
            t,
            vec![
                ("CcdB".into(), "#CC0000".into()),
                ("APH".into(), "#00FF00".into()),
                ("CAT".into(), "#0000CC".into()),
            ]
        );
    }

    #[test]
    fn color_table_rejects_dupes_and_bad_lines() {
        assert!(read_color_table("A\tcc0000\nA\t00ff00\n").is_err());
        assert!(read_color_table("A\tcc0000\textra\n").is_err());
    }

    fn dom(cds: &str, name: &str, score: f64, db: &str) -> SeqFeature {
        let mut f = SeqFeature::new(DOMAIN_FEATURE_NAME, Location::range(0, 30, 1));
        f.set("cds_id", cds);
        f.set("name", name);
        f.set("database", db);
        f.set("score", format!("{score}"));
        f
    }
    fn cds(cds: &str) -> SeqFeature {
        let mut f = SeqFeature::new("CDS", Location::range(0, 30, 1));
        f.set("cds_id", cds);
        f
    }
    fn rec(features: Vec<SeqFeature>) -> SeqRecord {
        SeqRecord {
            id: "c".into(),
            name: "c".into(),
            description: String::new(),
            seq: vec![b'A'; 60],
            molecule_type: Some("DNA".into()),
            topology: Topology::Linear,
            annotations: Annotations::default(),
            features,
        }
    }

    #[test]
    fn cds_colored_by_highest_scoring_domain() {
        let mut r = rec(vec![
            cds("C1"),
            dom("C1", "low", 5.0, "db"),
            dom("C1", "high", 20.0, "db"),
        ]);
        let mut map = ColorMap::from_table(vec![
            ("low".into(), "#111111".into()),
            ("high".into(), "#222222".into()),
        ]);
        let opts = ColorGenbankOptions {
            targets: ColorTargets {
                domains: true,
                cdss: true,
            },
            search_hit_color: None,
            clear: false,
            databases: None,
            color_feature: "name".into(),
        };
        color_seqrecord_by_domain(&mut r, &mut map, &opts);
        // CDS takes the higher-scoring domain's color.
        assert_eq!(r.features[0].get_first("Color"), Some("#222222"));
        assert_eq!(r.features[1].get_first("Color"), Some("#111111"));
        assert_eq!(r.features[2].get_first("Color"), Some("#222222"));
    }

    #[test]
    fn clear_removes_existing_colors() {
        let mut d = dom("C1", "x", 1.0, "db");
        d.set("Color", "#999999");
        let mut r = rec(vec![d]);
        let mut map = ColorMap::from_table(vec![]); // empty table -> no new colors
        let opts = ColorGenbankOptions {
            targets: ColorTargets {
                domains: true,
                cdss: false,
            },
            search_hit_color: None,
            clear: true,
            databases: None,
            color_feature: "name".into(),
        };
        color_seqrecord_by_domain(&mut r, &mut map, &opts);
        assert_eq!(r.features[0].get_first("Color"), None);
    }

    #[test]
    fn auto_map_assigns_in_first_seen_order() {
        let mut map = ColorMap::auto();
        assert_eq!(map.get("A").unwrap(), AUTO_PALETTE[0]);
        assert_eq!(map.get("B").unwrap(), AUTO_PALETTE[1]);
        assert_eq!(map.get("A").unwrap(), AUTO_PALETTE[0]); // stable
        assert_eq!(map.items().len(), 2);
    }

    #[test]
    fn databases_filter_restricts_color_source() {
        let mut r = rec(vec![cds("C1"), dom("C1", "x", 10.0, "other")]);
        let mut map = ColorMap::from_table(vec![("x".into(), "#123456".into())]);
        let mut dbs = HashSet::new();
        dbs.insert("keep".to_string());
        let opts = ColorGenbankOptions {
            targets: ColorTargets {
                domains: true,
                cdss: true,
            },
            search_hit_color: None,
            clear: false,
            databases: Some(dbs),
            color_feature: "name".into(),
        };
        color_seqrecord_by_domain(&mut r, &mut map, &opts);
        // Domain's database "other" not in {keep} -> skipped, no color anywhere.
        assert_eq!(r.features[0].get_first("Color"), None);
        assert_eq!(r.features[1].get_first("Color"), None);
    }
}
