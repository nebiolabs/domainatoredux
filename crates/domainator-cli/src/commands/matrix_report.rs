use std::io::Write;

use anyhow::{Context, Result};
use domainator_config::args::MatrixReportArgs;
use domainator_core::io::python_json;
use domainator_core::matrix::{read_matrix, require_similarity};
use domainator_core::tools::matrix_report::{matrix_report, render_text, to_json};

use crate::output::Output;

pub fn run(args: &MatrixReportArgs) -> Result<()> {
    let max_merge_events = usize::try_from(args.max_merge_events)
        .map_err(|_| anyhow::anyhow!("max_merge_events must be >= 0"))?;
    let matrix =
        read_matrix(&args.input).with_context(|| format!("reading matrix {}", args.input))?;
    require_similarity(&matrix.data_type, "matrix_report").map_err(anyhow::Error::msg)?;
    let report = matrix_report(&matrix, &args.merge_impact_metric, max_merge_events);
    if let Some(path) = &args.text_out {
        let mut out = Output::create(path)?;
        out.write_all(render_text(&report).as_bytes())?;
        out.finish()?;
    }
    if let Some(path) = &args.json_out {
        let mut out = Output::create(path)?;
        writeln!(out, "{}", python_json::to_string(&to_json(&report)))?;
        out.finish()?;
    }
    Ok(())
}
