use anyhow::{anyhow, Result};
use domainator_config::args::HmmerReportArgs;
use domainator_core::io::db_name_from_path;
use domainator_core::tools::hmmer_report::{Analysis, HmmerReportWriter, OutputFormat};

use super::for_each_hmm_input;
use crate::output::{is_stdin, Output};

pub fn run(args: &HmmerReportArgs) -> Result<()> {
    let analyses: Vec<Analysis> = args
        .columns
        .iter()
        .map(|c| c.to_analysis())
        .collect::<Result<_>>()?;
    let report =
        HmmerReportWriter::new(&analyses, args.column_names.as_deref()).map_err(|e| anyhow!(e))?;
    let mut outputs = Vec::new();
    for (path, format) in [
        (&args.tsv_out, OutputFormat::Tsv),
        (
            &args.html_out,
            OutputFormat::Html {
                max_height: args.html_max_height,
            },
        ),
        (&args.json_out, OutputFormat::Json),
    ] {
        if let Some(path) = path {
            let mut out = Output::create(path)?;
            let sink = report.start(format, &mut out).map_err(|e| anyhow!(e))?;
            outputs.push((out, sink));
        }
    }
    // Each batch of profiles written to every output as it is read. Standard input is
    // named `-`, as upstream's is.
    for_each_hmm_input(&args.input, |path, hmms| {
        let source = if is_stdin(path) {
            "-".to_string()
        } else {
            db_name_from_path(path)
        };
        for (out, sink) in &mut outputs {
            report
                .write_profiles(sink, out, &source, &hmms)
                .map_err(|e| anyhow!(e))?;
        }
        Ok(())
    })?;
    for (mut out, sink) in outputs {
        report.finish(sink, &mut out).map_err(|e| anyhow!(e))?;
        out.finish()?;
    }
    Ok(())
}
