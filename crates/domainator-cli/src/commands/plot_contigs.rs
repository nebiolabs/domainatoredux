use std::io::Write;

use anyhow::{Context, Result};
use domainator_config::args::PlotContigsArgs;
use domainator_core::tools::plot_contigs::{plot_contigs_data, PlotContigsOptions, DEFAULT_TYPES};
use domainator_core::tools::report_html::{render_report, Encoding, PLOT_CONTIGS_SHELL};

use super::{load_seq_inputs, usage_error};
use crate::output::Output;

pub fn run(args: &PlotContigsArgs) -> Result<()> {
    if args.color_table.is_some() && args.color_by.is_none() {
        return Err(usage_error("--color_table requires --color_by."));
    }
    if args.color_by.is_some() && args.metadata.is_none() {
        return Err(usage_error("--color_by requires --metadata."));
    }
    let color_table = args
        .color_table
        .as_deref()
        .map(crate::read_color_table)
        .transpose()?;
    let metadata = args
        .metadata
        .iter()
        .flatten()
        .map(|path| std::fs::read_to_string(path).with_context(|| format!("reading {path}")))
        .collect::<Result<Vec<_>>>()?;

    let records = load_seq_inputs(&args.input)?;
    let opts = PlotContigsOptions {
        types: args
            .types
            .clone()
            .unwrap_or_else(|| DEFAULT_TYPES.iter().map(|t| t.to_string()).collect()),
        width: args.width,
        height: args.height,
        feature_height: args.feature_height,
        feature_font_size: args.feature_font_size,
        color_by: args.color_by.clone(),
        color_table,
        palette_offset: args.palette_offset,
    };
    let data = plot_contigs_data(&records, &opts, &metadata)?;
    let mut out = Output::create(&args.html_out)?;
    out.write_all(
        render_report(
            PLOT_CONTIGS_SHELL,
            "Contig Plot",
            "plot_contigs",
            data,
            Encoding::GzipBase64,
        )
        .as_bytes(),
    )?;
    out.finish()
}
