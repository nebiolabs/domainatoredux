use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use domainator_config::args::BuildProjectionArgs;
use domainator_core::matrix::cytoscape::{write_xgmml_with, XgmmlExtras};
use domainator_core::matrix::read_matrix;
use domainator_core::matrix::text::py_float_str;
use domainator_core::palette::{sort_palette_values, OTHER_COLOR};
use domainator_core::tools::build_projection::{
    axes, center_and_scale, parse_keyword_args, project, projection_view,
};
use domainator_core::tools::node_table::{read_node_table, resolve_colors, ColorOptions};
use domainator_core::tools::viewer_bundle::{
    default_name, viewer_bundle, viewer_colors, viewer_defaults, ViewerDefaults,
};

use super::{usage_error, write_viewer_outputs};
use crate::output::Output;

pub fn run(args: &BuildProjectionArgs) -> Result<()> {
    let all_args = [
        ("pca", &args.pca_args),
        ("tsne", &args.tsne_args),
        ("umap", &args.umap_args),
    ];
    for (algorithm, given) in all_args {
        if algorithm != args.algorithm && given.as_deref().is_some_and(|s| !s.is_empty()) {
            return Err(usage_error(format!(
                "--{algorithm}_args was given, but --algorithm is {}.",
                args.algorithm
            )));
        }
    }
    let algorithm_args = all_args
        .iter()
        .find(|(a, _)| *a == args.algorithm)
        .and_then(|(_, given)| given.as_deref());
    let parameters = parse_keyword_args(algorithm_args, &format!("--{}_args", args.algorithm))?;

    // Kept in file order: --color_table_out writes a given table as it was read.
    let given_table: Option<Vec<(String, String)>> = args
        .color_table
        .as_deref()
        .map(|path| -> Result<_> {
            let text = std::fs::read_to_string(path).with_context(|| format!("reading {path}"))?;
            domainator_core::tools::color_genbank::read_color_table(&text)
                .map_err(anyhow::Error::msg)
        })
        .transpose()?;
    let color_table: Option<HashMap<String, String>> =
        given_table.as_ref().map(|t| t.iter().cloned().collect());
    if args.color_table_out.is_some() && args.color_by.is_none() && color_table.is_none() {
        anyhow::bail!(
            "--color_table_out requires either --color_by or --color_table, otherwise there are \
             no colors to write."
        );
    }

    let matrix =
        read_matrix(&args.input).with_context(|| format!("reading matrix {}", args.input))?;
    let n_components = if args.three_d { 3 } else { 2 };
    let mut coords = project(&matrix, &args.algorithm, n_components, &parameters)?;
    center_and_scale(&mut coords, args.scale.0);
    let axes = axes(n_components);
    let nodes = &matrix.rows;

    if let Some(path) = &args.coords_out {
        let mut out = Output::create(path)?;
        writeln!(out, "\t{}", axes.join("\t"))?;
        for (i, name) in nodes.iter().enumerate() {
            let values: Vec<String> = coords.iter().map(|axis| py_float_str(axis[i])).collect();
            writeln!(out, "{name}\t{}", values.join("\t"))?;
        }
        out.finish()?;
    }

    let metadata = args.metadata.clone().unwrap_or_default();
    let node_data = read_node_table(nodes, Vec::new(), &metadata)?;
    if let Some(color_by) = &args.color_by {
        if !node_data.columns.contains(color_by) {
            anyhow::bail!(
                "Requested color_by column '{color_by}' was not found in the merged metadata."
            );
        }
    }
    let palette_offset = match usize::try_from(args.palette_offset) {
        Ok(n) => n,
        Err(_) if args.color_by.is_none() || color_table.is_some() => 0,
        Err(_) => anyhow::bail!(
            "The palette offset must be 0 or more, not {}.",
            args.palette_offset
        ),
    };
    let colors = resolve_colors(
        &node_data,
        &ColorOptions {
            color_by: args.color_by.as_deref(),
            color_table: color_table.as_ref(),
            max_color_groups: None,
            palette_offset,
        },
    );

    if args.html_out.is_some() || args.dvb_out.is_some() {
        let name = args
            .name
            .clone()
            .unwrap_or_else(|| default_name(args.html_out.as_deref(), args.dvb_out.as_deref()));
        let defaults = viewer_defaults(
            &node_data,
            &ViewerDefaults {
                color_by: args.color_by.as_deref(),
                color_table: viewer_colors(
                    color_table.as_ref(),
                    colors.as_ref().map(|c| &c.table),
                    palette_offset,
                    None,
                ),
                label_by: args.label_by.as_deref(),
                categorical: args.categorical.as_deref().unwrap_or_default(),
            },
        )?;
        let bundle = viewer_bundle(
            &name,
            nodes,
            &node_data,
            defaults,
            "projection",
            projection_view(&args.algorithm, &coords),
        );
        write_viewer_outputs(
            &bundle,
            args.html_out.as_deref(),
            args.dvb_out.as_deref(),
            None,
            &[],
        )?;
    }

    if let Some(path) = &args.xgmml_out {
        let positions: Vec<(f64, f64, Option<f64>)> = (0..nodes.len())
            .map(|i| (coords[0][i], coords[1][i], coords.get(2).map(|z| z[i])))
            .collect();
        let name = Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        let fills = colors.as_ref().map(|c| c.fills.clone());
        let mut out = Output::create(path)?;
        write_xgmml_with(
            &mut out,
            name,
            &node_data,
            &[],
            &XgmmlExtras {
                node_fills: fills.as_deref(),
                positions: Some(&positions),
                ..Default::default()
            },
        )?;
        out.finish()?;
    }

    if let Some(path) = &args.color_table_out {
        let mut out = Output::create(path)?;
        match (&given_table, &colors) {
            (Some(table), _) => {
                for (value, color) in table {
                    writeln!(out, "{value}\t{color}")?;
                }
            }
            (None, Some(colors)) => {
                let values: Vec<String> = colors.table.keys().cloned().collect();
                for value in sort_palette_values(&values) {
                    writeln!(out, "{value}\t{}", colors.table[&value])?;
                }
                if colors.palette_has_missing {
                    writeln!(out, "None\t{OTHER_COLOR}")?;
                }
            }
            (None, None) => unreachable!("checked above"),
        }
        out.finish()?;
    }
    Ok(())
}
