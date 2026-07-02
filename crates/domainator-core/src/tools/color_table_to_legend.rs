//! `color_table_to_legend`: generate an SVG legend from a color table. Port of
//! `domainator.color_table_to_legend`.
//!
//! The color table is a TSV mapping annotations to hex colors (parsed by
//! [`crate::tools::color_genbank::read_color_table`]). The SVG contains one color
//! box + label per entry.
//!
//! Number formatting mirrors Python exactly: values that are Python `int`s render
//! without a decimal point (`260`, `40`, `2`); values produced by `/` (always
//! `float` in Python) render via `str(float)` (`190.0`, `60.0`). This split is
//! load-bearing for byte-identical SVG output.

use crate::matrix::text::py_float_str;

/// Minimal HTML/XML text escaping matching Python's `html.escape` (escapes `&`,
/// `<`, `>`, `"`, `'`). Order matters: `&` first.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

/// Render an SVG legend from ordered `(annotation, color)` pairs. Mirrors
/// `color_table_to_legend.color_table_to_legend` layout and number formatting.
pub fn color_table_to_legend(table: &[(String, String)], title: &str) -> String {
    // Integer constants (Python `int`s), rendered without a decimal point.
    let title_font_size = 24_i64;
    let item_font_size = 20_i64;
    let stroke_width = 2_i64;
    let padding = 10_i64;
    let title_space = 40_i64;

    let box_height = item_font_size * 2; // int
    let box_width = box_height; // int
    let text_offset_y = box_height as f64 / 2.0; // float (Python `/`)

    let longest_key = table.iter().map(|(k, _)| k.chars().count()).max().unwrap_or(0) as i64;

    let text_offset_x = box_width + padding * 2; // int
    let total_height = title_space + table.len() as i64 * box_height + 2 * padding; // int
    // Both branches of max() involve `/` -> float.
    let total_width = (box_width as f64 + (padding * 3) as f64 + (longest_key * item_font_size) as f64 / 2.0)
        .max((padding * 3) as f64 + (title.chars().count() as i64 * title_font_size) as f64 / 2.0);
    let total_width = py_float_str(total_width);

    let mut out = String::new();
    out.push_str(&format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="no"?>
<svg width="{tw}" height="{total_height}" xmlns="http://www.w3.org/2000/svg">
    <g font-family="Arial, sans-serif">
        <rect x="0" y="0" width="{tw}" height="{total_height}" fill="white" stroke="black" stroke-width="{stroke_width}"/>
        <text x="{padding}" y="{title_font_size}" font-size="{title_font_size}" font-weight="bold">{esc_title}</text>
"#,
        tw = total_width,
        esc_title = html_escape(title),
    ));

    let mut y = title_space; // int (starts at title_space, increments by box_height)
    for (k, v) in table {
        let box_h2 = box_height - 10; // int
        let text_y = py_float_str(y as f64 + text_offset_y); // float
        out.push_str(&format!(
            r#"        <rect x="{padding}" y="{y}" width="{box_width}" height="{box_h2}" fill="{v}" stroke="black" stroke-width="{stroke_width}"/>
        <text x="{text_offset_x}" y="{text_y}" font-size="{item_font_size}">{esc_k}</text>
"#,
            esc_k = html_escape(k),
        ));
        y += box_height;
    }
    out.push_str("    </g>\n</svg>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_special_chars() {
        assert_eq!(html_escape("a<b>&\"'"), "a&lt;b&gt;&amp;&quot;&#x27;");
    }

    #[test]
    fn number_formatting_matches_python() {
        // 5 entries, longest key "Condensation" (12) -> total_width 190.0, height 260.
        let table = vec![
            ("CcdB".into(), "#FF0000".into()),
            ("APH".into(), "#00FF00".into()),
            ("CAT".into(), "#0000FF".into()),
            ("Condensation".into(), "#FF00FF".into()),
            ("2-oxoacid_dh".into(), "#FFFFFF".into()),
        ];
        let svg = color_table_to_legend(&table, "Legend");
        assert!(svg.contains(r#"width="190.0" height="260""#));
        assert!(svg.contains(r#"stroke-width="2""#));
        assert!(svg.contains(r#"<text x="60" y="60.0" font-size="20">CcdB</text>"#));
        assert!(svg.contains(r##"<rect x="10" y="40" width="40" height="30" fill="#FF0000""##));
        assert!(svg.trim_end().ends_with("</svg>"));
    }
}
