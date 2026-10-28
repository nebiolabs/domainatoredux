use std::io::Write;

use anyhow::{Context, Result};
use domainator_config::args::ViewerArgs;
use domainator_core::tools::ssn_bundle::parse_any_bundle;
use domainator_core::tools::viewer_bundle::gzip;
use domainator_core::tools::viewer_html::{render_html, VIEWER_APP_NAME};

use super::usage_error;
use crate::output::Output;

pub fn run(args: &ViewerArgs) -> Result<()> {
    let html_out = args
        .html_out
        .as_deref()
        .ok_or_else(|| usage_error("No output specified. Give at least one of: --html_out"))?;
    let html = match &args.input {
        None => render_html(args.name.as_deref().unwrap_or(VIEWER_APP_NAME), None),
        Some(input) => {
            let raw = std::fs::read(input).with_context(|| format!("reading {input}"))?;
            // The page is handed the file's own bytes, gzip-compressed if they were not.
            let compressed = if raw.starts_with(&[0x1f, 0x8b]) {
                raw
            } else {
                gzip(&raw)
            };
            let bundle = parse_any_bundle(&compressed, input)?;
            let title = args
                .name
                .as_deref()
                .or_else(|| bundle.get("name").and_then(serde_json::Value::as_str))
                .filter(|n| !n.is_empty())
                .unwrap_or(VIEWER_APP_NAME);
            render_html(title, Some(&compressed))
        }
    };
    let mut out = Output::create(html_out)?;
    out.write_all(html.as_bytes())?;
    out.finish()
}
