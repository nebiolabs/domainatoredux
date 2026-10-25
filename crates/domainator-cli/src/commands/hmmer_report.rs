use anyhow::{anyhow, Context, Result};
use domainator_config::args::HmmerReportArgs;
use domainator_core::tools::hmmer_report::{
    hmmer_report, read_hmms_to_iterators, Analysis, OutputFormat,
};

use crate::output::{is_stdin, Output};

pub fn run(args: &HmmerReportArgs) -> Result<()> {
    let analyses: Vec<Analysis> = args
        .columns
        .iter()
        .map(|c| c.to_analysis())
        .collect::<Result<_>>()?;
    let mut records = Vec::new();
    for path in &args.input {
        if is_stdin(path) {
            // Read through a temporary file; the source is named `-`, as upstream's is.
            let mut text = Vec::new();
            std::io::Read::read_to_end(&mut std::io::stdin(), &mut text)
                .context("reading standard input")?;
            let tmp = std::env::temp_dir()
                .join(format!("domainatoredux_stdin_{}.hmm", std::process::id()));
            std::fs::write(&tmp, &text)?;
            let read = read_hmms_to_iterators(&[tmp.to_string_lossy().into_owned()]);
            let _ = std::fs::remove_file(&tmp);
            let (_, hmms) = read
                .map_err(|e| anyhow!(e))?
                .pop()
                .context("reading standard input")?;
            records.push(("-".to_string(), hmms));
        } else {
            records.extend(
                read_hmms_to_iterators(std::slice::from_ref(path)).map_err(|e| anyhow!(e))?,
            );
        }
    }

    let mut outputs = Vec::new();
    for (path, format) in [
        (&args.tsv_out, OutputFormat::Tsv),
        (&args.json_out, OutputFormat::Json),
    ] {
        if let Some(path) = path {
            outputs.push((Output::create(path)?, format));
        }
    }
    for (out, format) in &mut outputs {
        hmmer_report(
            &records,
            &analyses,
            args.column_names.as_deref(),
            *format,
            out,
        )
        .map_err(|e| anyhow!(e))?;
    }
    for (out, _) in outputs {
        out.finish()?;
    }
    Ok(())
}
