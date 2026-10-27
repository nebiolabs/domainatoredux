//! `plot_contigs` (upstream `plot_contigs.py`): the data of the contig plot page.
//!
//! The page is the vendored shell `assets/report_shells/plot_contigs.html`, and it lays the
//! plot out itself: it stacks each contig's features and spaces the contigs. What it is given
//! is built here, field for field as upstream builds it: each contig's name, length, line
//! color and details (record annotations and `--metadata` values, as text), and each drawn
//! feature's name, type, color, parts and qualifiers.

use std::collections::HashMap;
use std::io;

use serde_json::{json, Value};

use crate::palette::{get_palette, OTHER_COLOR};
use crate::seq::{SeqFeature, SeqRecord};
use crate::tools::color_genbank::normalize_color_hex;
use crate::tools::select_common::get_cds_name;
use crate::{DOMAIN_FEATURE_NAME, DOMAIN_SEARCH_BEST_HIT_NAME};

/// The feature types drawn by default, in stacking order.
pub const DEFAULT_TYPES: [&str; 8] = [
    "CDS",
    "exon",
    "repeat_region",
    "ncRNA",
    "rRNA",
    "tRNA",
    DOMAIN_FEATURE_NAME,
    DOMAIN_SEARCH_BEST_HIT_NAME,
];

/// The color of a contig's line when it is not colored by metadata, and of a value a color
/// table does not list.
pub const DEFAULT_CONTIG_COLOR: &str = "#000000";

/// The options upstream's library function has and its command does not, at their defaults.
const ARROW_WIDTH: i64 = 20;
const CONTIG_THICKNESS: i64 = 20;
const LABEL_FONT: &str = "sans-serif";
const CONTIG_LABEL_PX: i64 = 32;
const DETAILS_HEIGHT: i64 = 100;

/// What `plot_contigs` draws and how big.
#[derive(Clone, Debug)]
pub struct PlotContigsOptions {
    /// Feature types to draw, in stacking order.
    pub types: Vec<String>,
    pub width: i64,
    pub height: i64,
    pub feature_height: i64,
    pub feature_font_size: i64,
    /// The metadata column the contig lines are colored by.
    pub color_by: Option<String>,
    /// `--color_by` value to `#RRGGBB`; without it, colors come from the palette.
    pub color_table: Option<HashMap<String, String>>,
    pub palette_offset: i64,
}

impl Default for PlotContigsOptions {
    fn default() -> Self {
        Self {
            types: DEFAULT_TYPES.iter().map(|t| t.to_string()).collect(),
            width: 1200,
            height: 2400,
            feature_height: 50,
            feature_font_size: 26,
            color_by: None,
            color_table: None,
            palette_offset: 0,
        }
    }
}

/// A feature's fill when it has no `color`/`Color` qualifier, upstream `feature_to_dict`'s
/// `default_color`. The GUI contig viewer uses the same colors.
pub fn default_feature_color(kind: &str) -> &'static str {
    match kind {
        "repeat_region" => "#0000FF",
        "source" => "#2626FF",
        k if k == DOMAIN_FEATURE_NAME => "#00FFFF",
        k if k == DOMAIN_SEARCH_BEST_HIT_NAME => "#06FF0D",
        _ => "#CCCCCC",
    }
}

/// Upstream `get_feature_name`.
pub fn feature_name(feature: &SeqFeature) -> String {
    match feature.kind.as_str() {
        "CDS" | "gene" => return get_cds_name(feature, None),
        "source" => {
            if let Some(organism) = feature.get_first("organism") {
                return organism.to_string();
            }
        }
        _ => {}
    }
    ["name", "product", "gene", "locus_tag"]
        .iter()
        .find_map(|key| feature.get_first(key))
        .map_or_else(|| feature.kind.clone(), str::to_string)
}

/// A feature as the page draws it, upstream `feature_to_dict`: `{name, type, color, parts,
/// qualifiers}`, parts being `[start, end, strand]` and qualifiers `[key, [values]]`.
pub fn feature_to_value(feature: &SeqFeature) -> Result<Value, String> {
    let color = match feature
        .get_first("color")
        .or_else(|| feature.get_first("Color"))
    {
        Some(color) => normalize_color_hex(color)?,
        None => default_feature_color(&feature.kind).to_string(),
    };
    let parts: Vec<Value> = feature
        .location
        .parts
        .iter()
        .map(|&(start, end, strand, ..)| json!([start, end, if strand == -1 { "-" } else { "+" }]))
        .collect();
    let qualifiers: Vec<Value> = feature
        .qualifiers
        .iter()
        .map(|(key, values)| json!([key, values]))
        .collect();
    Ok(json!({
        "name": feature_name(feature),
        "type": feature.kind,
        "color": color,
        "parts": parts,
        "qualifiers": qualifiers,
    }))
}

/// What the page lists for a contig, upstream `contig_details`: its name, description and
/// length; the annotations of upstream's `CONTIG_ANNOTATIONS` that have a value, in that
/// order, lists joined with `"; "`; its cross-references as `[db, id]`; then `metadata`.
pub fn contig_details(record: &SeqRecord, metadata: &[(String, String)]) -> Vec<(String, String)> {
    let a = &record.annotations;
    let mut details = vec![
        ("Name".to_string(), record.id.clone()),
        ("Description".to_string(), record.description.clone()),
        ("Length".to_string(), record.len().to_string()),
    ];
    let annotations = [
        ("molecule_type", record.molecule_type.clone()),
        ("topology", a.topology.map(|t| t.as_str().to_string())),
        ("data_file_division", a.division.clone()),
        ("date", a.date.clone()),
        (
            "accessions",
            (!a.accessions.is_empty()).then(|| a.accessions.join("; ")),
        ),
        (
            "sequence_version",
            a.sequence_version.map(|v| v.to_string()),
        ),
        (
            "keywords",
            (!a.keywords.is_empty()).then(|| a.keywords.join("; ")),
        ),
        ("source", a.source.clone()),
        ("organism", a.organism.clone()),
        (
            "taxonomy",
            (!a.taxonomy.is_empty()).then(|| a.taxonomy.join("; ")),
        ),
        ("comment", a.comment.clone()),
    ];
    for (key, text) in annotations {
        if let Some(text) = text.filter(|t| !t.is_empty()) {
            details.push((key.to_string(), text));
        }
    }
    for xref in &a.dblink {
        details.push(match xref.split_once(':') {
            Some((db, id)) => (db.trim().to_string(), id.trim().to_string()),
            None => ("dbxref".to_string(), xref.clone()),
        });
    }
    details.extend(metadata.iter().cloned());
    details
}

/// Contig metadata tables, upstream `read_metadata`: the columns in the order first seen,
/// and each contig id's `{column: value}`.
///
/// A table's first column is the contig id and its first row the column names. Values are
/// the text in the file, and an empty cell is no value. Within a table the first row for an
/// id counts; when tables share a column, a contig gets the first table's value.
pub fn read_metadata(texts: &[String]) -> (Vec<String>, HashMap<String, HashMap<String, String>>) {
    let mut columns: Vec<String> = Vec::new();
    let mut values: HashMap<String, HashMap<String, String>> = HashMap::new();
    for text in texts {
        let mut lines = text.lines();
        let names: Vec<&str> = lines
            .next()
            .map(|header| header.split('\t').skip(1).collect())
            .unwrap_or_default();
        for name in &names {
            if !columns.iter().any(|c| c == name) {
                columns.push((*name).to_string());
            }
        }
        let mut seen = std::collections::HashSet::new();
        for line in lines {
            if line.is_empty() {
                continue;
            }
            let mut cells = line.split('\t');
            let id = cells.next().unwrap_or_default();
            if !seen.insert(id) {
                continue;
            }
            let contig = values.entry(id.to_string()).or_default();
            for (name, value) in names.iter().zip(cells) {
                if !value.is_empty() && !contig.contains_key(*name) {
                    contig.insert((*name).to_string(), value.to_string());
                }
            }
        }
    }
    (columns, values)
}

/// The page's data (upstream `plot_contigs`'s `data`): `{types, color_by, layout, contigs}`.
///
/// `metadata` are the `--metadata` files' contents. A contig with no `--color_by` value is
/// drawn in [`OTHER_COLOR`], and one whose value the color table does not list in
/// [`DEFAULT_CONTIG_COLOR`].
pub fn plot_contigs_data(
    records: &[SeqRecord],
    opts: &PlotContigsOptions,
    metadata: &[String],
) -> io::Result<Value> {
    let invalid = |msg: String| io::Error::new(io::ErrorKind::InvalidInput, msg);
    let (columns, values) = read_metadata(metadata);
    if let Some(color_by) = &opts.color_by {
        if !columns.contains(color_by) {
            return Err(invalid(format!(
                "--color_by {color_by} is not a column of the --metadata tables."
            )));
        }
    }
    let contig_metadata: Vec<Vec<(String, String)>> = records
        .iter()
        .map(|record| {
            let row = values.get(&record.id);
            columns
                .iter()
                .filter_map(|c| row.and_then(|r| r.get(c)).map(|v| (c.clone(), v.clone())))
                .collect()
        })
        .collect();

    let mut colors = vec![DEFAULT_CONTIG_COLOR.to_string(); records.len()];
    if let Some(color_by) = &opts.color_by {
        let by: Vec<Option<&String>> = contig_metadata
            .iter()
            .map(|row| row.iter().find(|(c, _)| c == color_by).map(|(_, v)| v))
            .collect();
        let mut present: Vec<String> = Vec::new();
        for value in by.iter().flatten() {
            if !present.contains(value) {
                present.push((*value).clone());
            }
        }
        let palette: HashMap<String, String> = match &opts.color_table {
            None => {
                let offset = usize::try_from(opts.palette_offset).map_err(|_| {
                    invalid(format!(
                        "The palette offset must be 0 or more, not {}.",
                        opts.palette_offset
                    ))
                })?;
                get_palette(&present, None, None, offset)
            }
            Some(table) => present
                .iter()
                .map(|v| {
                    let color = table.get(v).map_or(DEFAULT_CONTIG_COLOR, String::as_str);
                    (v.clone(), color.to_string())
                })
                .collect(),
        };
        for (color, value) in colors.iter_mut().zip(&by) {
            *color = value.map_or(OTHER_COLOR.to_string(), |v| palette[v].clone());
        }
    }

    let mut contigs = Vec::with_capacity(records.len());
    for ((record, row), color) in records.iter().zip(&contig_metadata).zip(colors) {
        let features = record
            .features
            .iter()
            .filter(|f| opts.types.contains(&f.kind))
            .map(feature_to_value)
            .collect::<Result<Vec<_>, _>>()
            .map_err(invalid)?;
        let details: Vec<Value> = contig_details(record, row)
            .into_iter()
            .map(|(label, text)| json!([label, text]))
            .collect();
        contigs.push(json!({
            "name": record.id,
            "length": record.len(),
            "color": color,
            "details": details,
            "features": features,
        }));
    }
    Ok(json!({
        "types": opts.types,
        "color_by": opts.color_by,
        "layout": {
            "width": opts.width,
            "height": opts.height,
            "feature_height": opts.feature_height,
            "arrow_width": ARROW_WIDTH,
            "contig_thickness": CONTIG_THICKNESS,
            "contig_label_font": LABEL_FONT,
            "contig_label_px": CONTIG_LABEL_PX,
            "feature_label_font": LABEL_FONT,
            "feature_font_size": opts.feature_font_size,
            "details_width": opts.width,
            "details_height": DETAILS_HEIGHT,
        },
        "contigs": contigs,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seq::{CompoundOp, Location};

    fn feature(
        kind: &str,
        parts: Vec<(i64, i64, i32)>,
        qualifiers: &[(&str, &[&str])],
    ) -> SeqFeature {
        SeqFeature {
            kind: kind.to_string(),
            location: Location {
                operator: (parts.len() > 1).then_some(CompoundOp::Join),
                between: false,
                parts: parts
                    .into_iter()
                    .map(|(s, e, st)| (s, e, st, false, false))
                    .collect(),
            },
            qualifiers: qualifiers
                .iter()
                .map(|(k, vs)| (k.to_string(), vs.iter().map(|v| v.to_string()).collect()))
                .collect(),
        }
    }

    #[test]
    fn features_keep_each_part_and_every_qualifier() {
        let f = feature(
            "CDS",
            vec![(50, 80, -1), (10, 30, -1)],
            &[("locus_tag", &["X1"]), ("note", &["a", "b"])],
        );
        assert_eq!(
            feature_to_value(&f).unwrap(),
            json!({
                "name": "X1", "type": "CDS", "color": "#CCCCCC",
                "parts": [[50, 80, "-"], [10, 30, "-"]],
                "qualifiers": [["locus_tag", ["X1"]], ["note", ["a", "b"]]],
            })
        );
    }

    #[test]
    fn a_color_qualifier_wins_and_a_bad_one_is_an_error() {
        let f = feature("misc_feature", vec![(0, 3, 0)], &[("Color", &["ab12cd"])]);
        let value = feature_to_value(&f).unwrap();
        assert_eq!(value["color"], "#AB12CD");
        assert_eq!(value["parts"], json!([[0, 3, "+"]]));
        let f = feature("misc_feature", vec![(0, 3, 1)], &[("color", &["red"])]);
        assert!(feature_to_value(&f).is_err());
    }

    #[test]
    fn features_are_named_as_upstream_names_them() {
        let source = feature("source", vec![(0, 9, 1)], &[("organism", &["E. coli"])]);
        assert_eq!(feature_name(&source), "E. coli");
        let trna = feature(
            "tRNA",
            vec![(0, 9, 1)],
            &[("gene", &["g"]), ("product", &["p"])],
        );
        assert_eq!(feature_name(&trna), "p");
        let bare = feature("misc_feature", vec![(0, 9, 1)], &[]);
        assert_eq!(feature_name(&bare), "misc_feature");
    }

    #[test]
    fn metadata_keeps_text_and_the_first_value() {
        let first = "id\tcount\tgroup\na\t007\t\na\t8\tlater row\nb\t1\tB\n".to_string();
        let second = "name\tgroup\textra\na\tA\tx\nb\tnot this\t\n".to_string();
        let (columns, values) = read_metadata(&[first, second]);
        assert_eq!(columns, ["count", "group", "extra"]);
        let row = |id: &str| {
            let mut row: Vec<(String, String)> = values[id].clone().into_iter().collect();
            row.sort();
            row
        };
        let pairs = |p: &[(&str, &str)]| -> Vec<(String, String)> {
            p.iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect()
        };
        assert_eq!(
            row("a"),
            pairs(&[("count", "007"), ("extra", "x"), ("group", "A")])
        );
        assert_eq!(row("b"), pairs(&[("count", "1"), ("group", "B")]));
    }

    #[test]
    fn contigs_are_colored_by_metadata() {
        let record = |id: &str| SeqRecord {
            id: id.to_string(),
            name: id.to_string(),
            description: String::new(),
            seq: b"ACGT".to_vec(),
            molecule_type: Some("DNA".to_string()),
            topology: crate::seq::Topology::Linear,
            annotations: crate::seq::Annotations::default(),
            features: Vec::new(),
        };
        let records = [record("a"), record("b"), record("c"), record("d")];
        let metadata = ["id\tgroup\na\t2\nb\t10\nc\t2\n".to_string()];
        let mut opts = PlotContigsOptions {
            color_by: Some("group".to_string()),
            ..PlotContigsOptions::default()
        };
        let colors = |opts: &PlotContigsOptions| -> Vec<Value> {
            let data = plot_contigs_data(&records, opts, &metadata).unwrap();
            data["contigs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| c["color"].clone())
                .collect()
        };
        // Numeric values in numeric order; no value is gray.
        let palette = crate::palette::DISTINCT_COLORS;
        assert_eq!(
            colors(&opts),
            json!([palette[0], palette[1], palette[0], OTHER_COLOR])
                .as_array()
                .unwrap()
                .clone()
        );
        opts.palette_offset = 3;
        assert_eq!(colors(&opts)[1], json!(palette[4]));
        opts.color_table = Some(HashMap::from([("10".to_string(), "#AB12CD".to_string())]));
        assert_eq!(
            colors(&opts),
            json!([
                DEFAULT_CONTIG_COLOR,
                "#AB12CD",
                DEFAULT_CONTIG_COLOR,
                OTHER_COLOR
            ])
            .as_array()
            .unwrap()
            .clone()
        );
        opts.color_by = Some("missing".to_string());
        assert!(plot_contigs_data(&records, &opts, &metadata).is_err());
    }
}
