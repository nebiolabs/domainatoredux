use std::collections::HashSet;

use anyhow::{anyhow, Result};
use domainator_config::args::EnumReportArgs;
use domainator_config::columns::enum_analyses;
use domainator_core::seq::taxonomy::default_ncbi_taxonomy_path;
use domainator_core::seq::SeqRecord;
use domainator_core::tools::enum_report::{enum_report, needs_taxonomy, By, OutputFormat};
use domainator_core::tools::select_common::parse_name_list;

use super::read_stdin_genbank;
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
        Some(crate::load_taxonomy(&path)?)
    } else {
        None
    };
    let databases: Option<HashSet<String>> =
        args.databases.as_ref().map(|d| d.iter().cloned().collect());

    // Each record with the name of the file it came from, for the `filename` column.
    let mut records: Vec<(SeqRecord, String)> = Vec::new();
    for path in &args.input {
        let (recs, name) = if is_stdin(path) {
            (read_stdin_genbank()?, "<stdin>".to_string())
        } else {
            (crate::load(std::path::Path::new(path))?, path.clone())
        };
        records.extend(recs.into_iter().map(|r| (r, name.clone())));
    }
    if let Some(set) = &targets {
        records.retain(|(r, _)| set.contains(&r.id));
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
        enum_report(
            records.clone(),
            by,
            &analyses,
            args.column_names.as_deref(),
            taxonomy.as_ref(),
            databases.as_ref(),
            partial,
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
