use std::collections::HashSet;

use anyhow::{anyhow, Result};
use domainator_config::args::EnumReportArgs;
use domainator_config::columns::enum_analyses;
use domainator_core::seq::taxonomy::default_ncbi_taxonomy_path;
use domainator_core::tools::enum_report::{needs_taxonomy, By, EnumReportWriter, OutputFormat};
use domainator_core::tools::select_common::parse_name_list;

use crate::input::{InputMode, SeqInputs};
use crate::output::{is_stdin, Output};

pub fn run(args: &EnumReportArgs) -> Result<()> {
    let by = match args.by.to_lowercase().as_str() {
        "cds" => By::Cds,
        "domain" => By::Domain,
        _ => By::Contig,
    };
    let partial = args.partial.parse().map_err(|e: String| anyhow!(e))?;
    let analyses = enum_analyses(&args.columns)?;
    let targets = parse_name_list(&args.contigs, args.contigs_file.as_deref())?;
    let taxonomy = if needs_taxonomy(&analyses) {
        let path = args
            .ncbi_taxonomy_path
            .clone()
            .unwrap_or_else(default_ncbi_taxonomy_path);
        Some(crate::load_taxonomy(&path, args.taxonomy_update)?)
    } else {
        None
    };
    let databases: Option<HashSet<String>> =
        args.databases.as_ref().map(|d| d.iter().cloned().collect());

    let mut report = EnumReportWriter::new(
        by,
        &analyses,
        args.column_names.as_deref(),
        taxonomy.as_ref(),
        databases.as_ref(),
        partial,
    )
    .map_err(|e| anyhow!(e))?;
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
    // One pass over the records, each row written to every output as it is made.
    let mut row_targets = Vec::new();
    for path in &args.input {
        // The name of the file each record came from, for the `filename` column.
        let filename = if is_stdin(path) {
            "<stdin>"
        } else {
            path.as_str()
        };
        let mut records = SeqInputs::new(std::slice::from_ref(path), InputMode::DETECT);
        if let Some(set) = &targets {
            records = records.with_contigs(set.clone());
        }
        for rec in records {
            let mut rec = rec?;
            row_targets.clear();
            report.add_record(&mut rec, filename, &mut row_targets);
            for (out, sink) in &mut outputs {
                report.write_rows(sink, out).map_err(|e| anyhow!(e))?;
            }
        }
    }
    for (mut out, sink) in outputs {
        report.finish(sink, &mut out).map_err(|e| anyhow!(e))?;
        out.finish()?;
    }
    Ok(())
}
