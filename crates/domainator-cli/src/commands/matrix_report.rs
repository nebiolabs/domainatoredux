use std::io::Write;

use anyhow::{Context, Result};
use domainator_config::args::MatrixReportArgs;
use domainator_core::io::python_json;
use domainator_core::matrix::{read_matrix, require_similarity};
use domainator_core::progress::Stages;
use domainator_core::tools::matrix_report::{matrix_report, render_text, to_html_data, to_json};
use domainator_core::tools::report_html::{render_report, Encoding, MATRIX_REPORT_SHELL};

use crate::output::Output;

pub fn run(args: &MatrixReportArgs) -> Result<()> {
    let max_merge_events = usize::try_from(args.max_merge_events)
        .map_err(|_| anyhow::anyhow!("max_merge_events must be >= 0"))?;
    let mut stages = Stages::new("matrix_report", args.progress, args.profile_stages);
    let since = stages.begin(&format!("loading matrix from {}", args.input));
    let matrix =
        read_matrix(&args.input).with_context(|| format!("reading matrix {}", args.input))?;
    stages.record("load_matrix", since, || {
        vec![("shape", format!("({}, {})", matrix.nrows(), matrix.ncols()))]
    });
    require_similarity(&matrix.data_type, "matrix_report").map_err(anyhow::Error::msg)?;
    if args.text_out.is_none() && args.html_out.is_none() && args.json_out.is_none() {
        return Ok(());
    }
    let total = std::time::Instant::now();
    let report = matrix_report(
        &matrix,
        &args.merge_impact_metric,
        max_merge_events,
        args.knn_projection,
        &mut stages,
    );
    let since = stages.begin("rendering outputs");
    if let Some(path) = &args.text_out {
        let mut out = Output::create(path)?;
        out.write_all(render_text(&report).as_bytes())?;
        out.finish()?;
    }
    if let Some(path) = &args.html_out {
        let mut out = Output::create(path)?;
        let page = render_report(
            MATRIX_REPORT_SHELL,
            "Matrix Report",
            "matrix_report",
            to_html_data(&report),
            Encoding::GzipBase64,
        );
        out.write_all(page.as_bytes())?;
        out.finish()?;
    }
    if let Some(path) = &args.json_out {
        let mut out = Output::create(path)?;
        writeln!(out, "{}", python_json::to_string(&to_json(&report)))?;
        out.finish()?;
    }
    let outputs = [&args.text_out, &args.html_out, &args.json_out]
        .iter()
        .filter(|o| o.is_some())
        .count();
    stages.record("render_outputs", since, || {
        vec![("outputs", outputs.to_string())]
    });
    stages.record("matrix_report_total", total, Vec::new);
    stages.emit_timings();
    Ok(())
}
