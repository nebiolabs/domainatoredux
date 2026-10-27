//! The `--html_out` report pages (upstream `report_html.py`).
//!
//! Each page is a generated shell, `assets/report_shells/<name>.html`: upstream's
//! `report_html.report_shell(name)`, written by `scripts/regen_report_shells.py` and never
//! edited by hand. It carries upstream's own JavaScript, with no third-party libraries, and
//! three tokens this module fills: the HTML-escaped title, the `report-data` envelope
//! (`{"report", "payload_version", "encoding", "data"}`), and, in a table page, the
//! streamed rows. So a page written here differs from upstream's only inside those tokens,
//! and `scripts/parity/differential.py` compares exactly that: the rest byte for byte, the
//! envelope and rows parsed.
//!
//! The pages and their data are not part of upstream's 1.x interface; `PAYLOAD_VERSION`
//! moves with the shape of the data the page reads.

use std::io::Write;

use flate2::write::GzEncoder;
use flate2::Compression;
use serde_json::Value;

use crate::io::python_json;

/// Upstream's `report_html.PAYLOAD_VERSION`.
pub const PAYLOAD_VERSION: u64 = 1;

pub const TITLE_TOKEN: &str = "@@REPORT_TITLE@@";
pub const DATA_TOKEN: &str = "@@REPORT_DATA@@";
pub const ROWS_TOKEN: &str = "@@REPORT_ROWS@@";

/// The table page of `enum_report` and `hmmer_report`.
pub const TABLE_REPORT_SHELL: &str = include_str!("../../assets/report_shells/table_report.html");
/// `summary_report`'s page.
pub const SUMMARY_REPORT_SHELL: &str =
    include_str!("../../assets/report_shells/summary_report.html");
/// `matrix_report`'s page.
pub const MATRIX_REPORT_SHELL: &str = include_str!("../../assets/report_shells/matrix_report.html");
/// `plot_contigs`' page.
pub const PLOT_CONTIGS_SHELL: &str = include_str!("../../assets/report_shells/plot_contigs.html");

/// How the envelope carries `data`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    /// As JSON.
    Json,
    /// As a base64 string of gzipped JSON, for the large payloads.
    GzipBase64,
}

impl Encoding {
    fn name(self) -> &'static str {
        match self {
            Encoding::Json => "json",
            Encoding::GzipBase64 => "gzip+base64",
        }
    }
}

/// `value` as compact JSON that is safe inside a `<script>` element (upstream
/// `script_json`). Only a string can hold `<`, and inside one `\u003c` means the same
/// character, so no value can close the element or open a comment.
pub fn script_json(value: &Value) -> String {
    python_json::to_string(value).replace('<', "\\u003c")
}

/// One line of a table page's `report-rows` block (upstream `report_row`).
pub fn report_row(row: &Value) -> String {
    let mut line = script_json(row);
    line.push('\n');
    line
}

/// The `report-data` block's contents (upstream `report_envelope`).
pub fn report_envelope(report: &str, data: Value, encoding: Encoding) -> String {
    let data = match encoding {
        Encoding::Json => data,
        Encoding::GzipBase64 => {
            let mut gz = GzEncoder::new(Vec::new(), Compression::default());
            gz.write_all(python_json::to_string(&data).as_bytes())
                .expect("writing to a Vec cannot fail");
            Value::String(base64_encode(
                &gz.finish().expect("writing to a Vec cannot fail"),
            ))
        }
    };
    let mut envelope = serde_json::Map::new();
    envelope.insert("report".into(), Value::String(report.into()));
    envelope.insert("payload_version".into(), PAYLOAD_VERSION.into());
    envelope.insert("encoding".into(), Value::String(encoding.name().into()));
    envelope.insert("data".into(), data);
    script_json(&Value::Object(envelope))
}

/// A page with no rows block, filled in (upstream `render_report`).
pub fn render_report(
    shell: &str,
    title: &str,
    report: &str,
    data: Value,
    encoding: Encoding,
) -> String {
    assert!(
        !shell.contains(ROWS_TOKEN),
        "a table shell streams rows; use report_parts"
    );
    let title = html_escape(title);
    let envelope = report_envelope(report, data, encoding);
    fill(
        shell,
        &[
            (TITLE_TOKEN, title.as_str()),
            (DATA_TOKEN, envelope.as_str()),
        ],
    )
}

/// A table page as the text before its rows and the text after them (upstream
/// `report_parts`); write each row between the two with [`report_row`].
pub fn report_parts(
    shell: &str,
    title: &str,
    report: &str,
    data: Value,
    encoding: Encoding,
) -> (String, String) {
    let (head, tail) = shell
        .split_once(ROWS_TOKEN)
        .expect("a table shell has a rows token");
    let title = html_escape(title);
    let envelope = report_envelope(report, data, encoding);
    let values = [
        (TITLE_TOKEN, title.as_str()),
        (DATA_TOKEN, envelope.as_str()),
    ];
    (fill(head, &values), fill(tail, &values))
}

/// `text` with each token replaced by its value, in one pass, so a value that happens to
/// contain a token stays a value.
fn fill(text: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some((at, token, value)) = values
        .iter()
        .filter_map(|(token, value)| rest.find(token).map(|at| (at, *token, *value)))
        .min_by_key(|(at, _, _)| *at)
    {
        out.push_str(&rest[..at]);
        out.push_str(value);
        rest = &rest[at + token.len()..];
    }
    out.push_str(rest);
    out
}

/// `&`, `<` and `>` as entities, as upstream's `escape_html` (and the SSN viewer's) does.
pub(crate) fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding, as Python's `base64.b64encode`.
pub(crate) fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64[(n >> 18 & 63) as usize] as char);
        out.push(B64[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            B64[(n >> 6 & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// A table page being written: [`TablePage::start`] writes everything up to the rows,
/// [`TablePage::row`] one row, [`TablePage::finish`] the rest. The shared writer of
/// `enum_report` and `hmmer_report` (upstream `table_writers.HTMLTableWriter`).
pub struct TablePage {
    tail: String,
}

impl TablePage {
    /// `columns` are `(name, type)`, type `"str"`, `"int"` or `"float"`.
    pub fn start<W: Write>(
        out: &mut W,
        title: &str,
        report: &str,
        columns: &[(String, &str)],
        max_height: Option<i64>,
    ) -> std::io::Result<TablePage> {
        let columns: Vec<Value> = columns
            .iter()
            .map(|(name, kind)| serde_json::json!({"name": name, "type": kind}))
            .collect();
        let data = serde_json::json!({
            "columns": columns,
            "max_height": max_height,
            "file_stem": report,
        });
        let (head, tail) = report_parts(TABLE_REPORT_SHELL, title, report, data, Encoding::Json);
        out.write_all(head.as_bytes())?;
        Ok(TablePage { tail })
    }

    pub fn row<W: Write>(&self, out: &mut W, row: &Value) -> std::io::Result<()> {
        out.write_all(report_row(row).as_bytes())
    }

    pub fn finish<W: Write>(self, out: &mut W) -> std::io::Result<()> {
        out.write_all(self.tail.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::read::GzDecoder;
    use serde_json::json;
    use std::io::Read;

    fn between<'a>(page: &'a str, open: &str, close: &str) -> &'a str {
        let start = page.find(open).unwrap() + open.len();
        &page[start..start + page[start..].find(close).unwrap()]
    }

    fn table(title: &str, rows: &[Value]) -> String {
        let mut out = Vec::new();
        let page = TablePage::start(
            &mut out,
            title,
            "enum_report",
            &[("name".into(), "str"), ("score".into(), "float")],
            Some(300),
        )
        .unwrap();
        for row in rows {
            page.row(&mut out, row).unwrap();
        }
        page.finish(&mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn the_shells_have_each_token_once_where_upstream_puts_it() {
        for (shell, rows) in [
            (TABLE_REPORT_SHELL, 1),
            (SUMMARY_REPORT_SHELL, 0),
            (MATRIX_REPORT_SHELL, 0),
            (PLOT_CONTIGS_SHELL, 0),
        ] {
            assert_eq!(shell.matches(TITLE_TOKEN).count(), 2);
            assert_eq!(shell.matches(DATA_TOKEN).count(), 1);
            assert_eq!(shell.matches(ROWS_TOKEN).count(), rows);
            // The SVG namespace is a name, never fetched.
            let shell = shell.replace("\"http://www.w3.org/2000/svg\"", "");
            assert!(!shell.contains("http://") && !shell.contains("https://"));
        }
    }

    #[test]
    fn a_page_without_rows_is_filled_once() {
        let page = render_report(
            SUMMARY_REPORT_SHELL,
            "S & <S>",
            "summary_report",
            json!({"summary": {"note": "@@REPORT_TITLE@@"}}),
            Encoding::Json,
        );
        assert_eq!(page.matches("S &amp; &lt;S&gt;").count(), 2);
        assert!(page.contains(
            r#"{"report":"summary_report","payload_version":1,"encoding":"json","data":{"summary":{"note":"@@REPORT_TITLE@@"}}}"#
        ));
    }

    #[test]
    fn a_table_page_carries_the_envelope_and_rows() {
        let rows = [
            json!({"name": "a", "score": 1.25}),
            json!({"name": null, "score": null}),
        ];
        let page = table("Enum Report", &rows);
        assert!(page.contains("<title>Enum Report</title>"));
        assert!(!page.contains("@@REPORT_"));
        let envelope: Value = serde_json::from_str(between(
            &page,
            r#"<script type="application/json" id="report-data">"#,
            "</script>",
        ))
        .unwrap();
        assert_eq!(
            envelope,
            json!({"report": "enum_report", "payload_version": 1, "encoding": "json", "data": {
                "columns": [{"name": "name", "type": "str"}, {"name": "score", "type": "float"}],
                "max_height": 300, "file_stem": "enum_report"}})
        );
        let written: Vec<Value> = between(
            &page,
            "<script type=\"application/x-ndjson\" id=\"report-rows\">\n",
            "</script>",
        )
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
        assert_eq!(written, rows);
    }

    #[test]
    fn no_value_can_close_a_script_element_or_fill_a_token() {
        let page = table(
            "</title><b>@@REPORT_DATA@@",
            &[
                json!({"name": "</script><script>alert(1)</script>", "score": 1.0}),
                json!({"name": "@@REPORT_TITLE@@ <!-- é", "score": 2.0}),
            ],
        );
        // report-data, report-rows and the page's own script.
        assert_eq!(page.matches("</script>").count(), 3);
        assert!(page.contains("<title>&lt;/title&gt;&lt;b&gt;@@REPORT_DATA@@</title>"));
        assert_eq!(page.matches(r#"{"report":"enum_report""#).count(), 1);
        assert!(page.contains(
            r#"{"name":"\u003c/script>\u003cscript>alert(1)\u003c/script>","score":1.0}"#
        ));
        // Non-ASCII is escaped as Python's json.dumps escapes it.
        assert!(page.contains(r#""@@REPORT_TITLE@@ \u003c!-- \u00e9""#));
    }

    #[test]
    fn compressed_data_round_trips() {
        let data = json!({"values": (0..100).collect::<Vec<_>>(), "label": "<b>"});
        let envelope: Value = serde_json::from_str(&report_envelope(
            "matrix_report",
            data.clone(),
            Encoding::GzipBase64,
        ))
        .unwrap();
        assert_eq!(envelope["encoding"], "gzip+base64");
        let b64 = envelope["data"].as_str().unwrap();
        let mut bytes = Vec::new();
        let mut buf = 0u32;
        let mut bits = 0;
        for c in b64.bytes().filter(|&c| c != b'=') {
            buf = buf << 6 | B64.iter().position(|&b| b == c).unwrap() as u32;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                bytes.push((buf >> bits) as u8);
            }
        }
        let mut text = String::new();
        GzDecoder::new(&bytes[..])
            .read_to_string(&mut text)
            .unwrap();
        assert_eq!(serde_json::from_str::<Value>(&text).unwrap(), data);
    }

    #[test]
    fn base64_matches_reference_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }
}
