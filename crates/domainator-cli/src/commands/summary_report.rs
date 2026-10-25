use std::io::Write;

use anyhow::{anyhow, Result};
use domainator_config::args::SummaryReportArgs;
use domainator_core::io::python_json;
use domainator_core::matrix::text::write_dense_text_to;
use domainator_core::seq::taxonomy::default_ncbi_taxonomy_path;
use domainator_core::tools::select_common::parse_name_list;
use domainator_core::tools::summary_report::{
    cooccurrence_matrix, domains_tsv, render_text, summary_report, to_json, SummaryReportOptions,
};

use super::load_seq_inputs;
use crate::output::Output;

pub fn run(args: &SummaryReportArgs) -> Result<()> {
    let partial = args.partial.parse().map_err(|e: String| anyhow!(e))?;
    let contigs = parse_name_list(
        args.contigs.as_deref().unwrap_or_default(),
        args.contigs_file.as_deref(),
    )?;
    let focus_domains = parse_name_list(
        args.domains.as_deref().unwrap_or_default(),
        args.domains_file.as_deref(),
    )?;
    let taxonomy = if args.taxonomy {
        let path = args
            .ncbi_taxonomy_path
            .clone()
            .unwrap_or_else(default_ncbi_taxonomy_path);
        Some(crate::load_taxonomy(&path)?)
    } else {
        None
    };

    let mut records = load_seq_inputs(&args.input)?;
    if let Some(contigs) = &contigs {
        records.retain(|r| contigs.contains(&r.id));
    }
    let mut focus_domains: Vec<String> = focus_domains.into_iter().flatten().collect();
    focus_domains.sort();
    let opts = SummaryReportOptions {
        focus_domains,
        databases: args.databases.clone().unwrap_or_default(),
        report_taxonomy: args.taxonomy,
        partial,
    };
    let report = summary_report(&records, &opts, taxonomy.as_ref());

    if let Some(path) = &args.text_out {
        write_text(path, &render_text(&report))?;
    }
    if let Some(path) = &args.json_out {
        write_text(
            path,
            &format!("{}\n", python_json::to_string(&to_json(&report))),
        )?;
    }
    for (path, fraction) in [
        (&args.co_occurrence_fraction_out, true),
        (&args.co_occurrence_count_out, false),
    ] {
        if let Some(path) = path {
            let mut out = Output::create(path)?;
            write_dense_text_to(&mut out, &cooccurrence_matrix(&report, fraction))?;
            out.finish()?;
        }
    }
    if let Some(path) = &args.domains_tsv_out {
        write_text(path, &domains_tsv(&report))?;
    }
    Ok(())
}

fn write_text(path: &str, text: &str) -> Result<()> {
    let mut out = Output::create(path)?;
    out.write_all(text.as_bytes())?;
    out.finish()
}
