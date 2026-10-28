use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use domainator_config::args::BuildSsnArgs;
use domainator_core::matrix::cytoscape::write_xgmml_to;
use domainator_core::matrix::read_matrix;
use domainator_core::palette::{sort_palette_values, OTHER_COLOR};
use domainator_core::tools::build_ssn::{
    build_ssn, BuildSsnOptions, BuildSsnResult, ViewerOptions,
};
use domainator_core::tools::output_guardrails::output_limit_message;
use domainator_core::tools::select_common::parse_name_list;
use domainator_core::tools::viewer_bundle::default_name;

use super::{usage_error, write_viewer_outputs};
use crate::output::Output;

pub fn run(args: &BuildSsnArgs) -> Result<()> {
    let viewer = args.html_out.is_some() || args.dvb_out.is_some();
    if viewer && args.color_table.is_some() && args.color_by.is_none() {
        return Err(usage_error("--color_table requires --color_by."));
    }
    let max_bytes = crate::max_output_gb_to_bytes(args.max_output_gb.0)?;
    let color_table = args
        .color_table
        .as_deref()
        .map(crate::read_color_table)
        .transpose()?;
    let subset_labels = parse_name_list(
        args.subset.as_deref().unwrap_or_default(),
        args.subset_file.as_deref(),
    )?;
    // Upstream refuses a negative offset only where it builds a palette, so one that
    // would go unused is not an error.
    let palette_offset = match usize::try_from(args.palette_offset) {
        Ok(n) => n,
        Err(_) if args.color_by.is_none() || color_table.is_some() => 0,
        Err(_) => anyhow::bail!(
            "The palette offset must be 0 or more, not {}.",
            args.palette_offset
        ),
    };
    let matrix =
        read_matrix(&args.input).with_context(|| format!("reading matrix {}", args.input))?;
    let opts = BuildSsnOptions {
        threshold: args.threshold.0,
        cluster: args.cluster || args.cluster_tsv_out.is_some(),
        mst: args.mst,
        knn: args.knn.map(|k| k.0 as usize),
        mark_mst: args.mark_mst,
        metadata_files: args.metadata.clone().unwrap_or_default(),
        subset_labels,
        color_by: args.color_by.clone(),
        color_table,
        // Upstream colors no group at all for a negative count.
        max_color_groups: args
            .max_color_groups
            .map(|n| usize::try_from(n).unwrap_or(0)),
        palette_offset,
        viewer: viewer.then(|| ViewerOptions {
            name: args
                .name
                .clone()
                .unwrap_or_else(|| default_name(args.html_out.as_deref(), args.dvb_out.as_deref())),
            merge_impact_metric: args.merge_impact_metric.clone(),
            label_by: args.label_by.clone(),
            categorical: args.categorical.clone().unwrap_or_default(),
        }),
    };
    let result = build_ssn(&matrix, &opts)?;

    if let Some(bundle) = &result.bundle {
        write_viewer_outputs(
            bundle,
            args.html_out.as_deref(),
            args.dvb_out.as_deref(),
            max_bytes,
            &["--subset", "--subset_file"],
        )?;
    }
    if let Some(xgmml_out) = &args.xgmml_out {
        write_xgmml(xgmml_out, &result, max_bytes)?;
    }

    if let Some(path) = &args.cluster_tsv_out {
        let labels = result
            .cluster_labels
            .as_ref()
            .expect("--cluster_tsv_out turns clustering on");
        let mut out = Output::create(path)?;
        if !args.no_cluster_header {
            writeln!(out, "contig\tcluster")?;
        }
        for (name, id) in result.table.names.iter().zip(labels) {
            writeln!(out, "{name}\t{id}")?;
        }
        out.finish()?;
    }

    if let Some(path) = &args.color_table_out {
        let table = result.resolved_color_table.as_ref().context(
            "--color_table_out requires either --color_by or --color_table, otherwise there are \
             no colors to write.",
        )?;
        let values: Vec<String> = table.keys().cloned().collect();
        let mut out = Output::create(path)?;
        for value in sort_palette_values(&values) {
            writeln!(out, "{value}\t{}", table[&value])?;
        }
        if result.palette_has_missing {
            writeln!(out, "None\t{OTHER_COLOR}")?;
        }
        out.finish()?;
    }
    Ok(())
}

/// Write the network as XGMML. Serialized first, so the size limit can refuse the
/// network before it is written.
fn write_xgmml(xgmml_out: &str, result: &BuildSsnResult, max_bytes: Option<u64>) -> Result<()> {
    let name = Path::new(xgmml_out)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let mut xgmml = Vec::new();
    write_xgmml_to(
        &mut xgmml,
        name,
        &result.table,
        &result.edges,
        result.edge_mst.as_deref(),
        result.node_fills.as_deref(),
    )?;
    if let Some(max) = max_bytes.filter(|&max| xgmml.len() as u64 > max) {
        anyhow::bail!(output_limit_message(
            &format!("XGMML network output '{xgmml_out}'"),
            xgmml.len() as u64,
            max,
            &["--threshold", "--mst", "--knn", "--subset"],
            None,
        ));
    }
    let mut out = Output::create(xgmml_out)?;
    out.write_all(&xgmml)?;
    out.finish()?;
    Ok(())
}
