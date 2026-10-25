use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use domainator_config::args::BuildSsnViewerArgs;
use domainator_core::matrix::read_matrix;
use domainator_core::tools::build_ssn_viewer::{
    build_ssn_viewer_bundle, gzip, render_html, serialize_bundle, BuildSsnViewerOptions,
    VIEWER_APP_NAME,
};
use domainator_core::tools::output_guardrails::output_limit_message;
use domainator_core::tools::select_common::parse_name_list;

use super::usage_error;
use crate::output::Output;

pub fn run(args: &BuildSsnViewerArgs) -> Result<()> {
    let Some(input) = &args.input else {
        // No matrix: an empty viewer page that opens bundles itself.
        if args.dsnv_out.is_some() {
            return Err(usage_error("-o/--dsnv_out requires -i/--input."));
        }
        if args.embed_data {
            return Err(usage_error("--embed_data requires -i/--input."));
        }
        let html_out = args
            .html_out
            .as_deref()
            .ok_or_else(|| usage_error("No output specified. Give at least one of: --html_out"))?;
        let name = args.name.as_deref().unwrap_or(VIEWER_APP_NAME);
        return write_text(html_out, &render_html(name, None));
    };

    if args.embed_data && args.html_out.is_none() {
        return Err(usage_error("--embed_data requires --html_out."));
    }
    if args.dsnv_out.is_none() && !args.embed_data {
        return Err(usage_error(
            "When -i/--input is supplied, provide -o/--dsnv_out or --embed_data.",
        ));
    }
    if args.color_table.is_some() && args.color_by.is_none() {
        return Err(usage_error("--color_table requires --color_by."));
    }

    let max_bytes = crate::max_output_gb_to_bytes(args.max_output_gb.0)?;
    let subset_labels = parse_name_list(
        args.subset.as_deref().unwrap_or_default(),
        args.subset_file.as_deref(),
    )?;
    let stem = |p: &String| {
        Path::new(p)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string()
    };
    let name = args
        .name
        .clone()
        .or_else(|| args.dsnv_out.as_ref().map(stem))
        .or_else(|| args.html_out.as_ref().map(stem))
        .unwrap_or_else(|| VIEWER_APP_NAME.to_string());
    let color_table = args
        .color_table
        .as_deref()
        .map(|path| {
            crate::read_color_table(path).map(|table| table.into_iter().collect::<Vec<_>>())
        })
        .transpose()?;

    let matrix = read_matrix(input).with_context(|| format!("reading matrix {input}"))?;
    let opts = BuildSsnViewerOptions {
        name: name.clone(),
        metadata_files: args.metadata.clone().unwrap_or_default(),
        subset_labels,
        merge_impact_metric: args.merge_impact_metric.clone(),
        color_by: args.color_by.clone(),
        color_table,
        label_by: args.label_by.clone(),
        categorical_columns: args.categorical.clone().unwrap_or_default(),
    };
    let bundle = build_ssn_viewer_bundle(&matrix, &opts)?;
    let compressed = gzip(&serialize_bundle(&bundle));

    if let Some(path) = &args.dsnv_out {
        if let Some(max) = max_bytes.filter(|&max| compressed.len() as u64 > max) {
            anyhow::bail!(output_limit_message(
                &format!("Domainator Similarity Network Viewer bundle output '{path}'"),
                compressed.len() as u64,
                max,
                &["--subset", "--subset_file"],
                Some(
                    "This bundle stores only MST-derived hierarchy data; subset the network \
                     before export if it is still too large."
                ),
            ));
        }
        let mut out = Output::create(path)?;
        out.write_all(&compressed)?;
        out.finish()?;
    }
    if let Some(path) = &args.html_out {
        let embed = args.embed_data.then_some(compressed.as_slice());
        write_text(path, &render_html(&name, embed))?;
    }
    Ok(())
}

fn write_text(path: &str, text: &str) -> Result<()> {
    let mut out = Output::create(path)?;
    out.write_all(text.as_bytes())?;
    out.finish()
}
