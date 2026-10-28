use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use domainator_config::args::BuildTreeArgs;
use domainator_core::io::newick;
use domainator_core::log;
use domainator_core::matrix::cytoscape::{write_xgmml_with, XgmmlExtras};
use domainator_core::matrix::read_matrix;
use domainator_core::palette::sort_palette_values;
use domainator_core::tools::build_ssn::subset_matrix;
use domainator_core::tools::build_tree::{
    network_fills, read_msa, subset_tree, tree_from_matrix, tree_from_newick, tree_network,
    tree_view, SCORE_COLUMN,
};
use domainator_core::tools::node_table::{read_node_table, resolve_colors, ColorOptions};
use domainator_core::tools::select_common::parse_name_list;
use domainator_core::tools::viewer_bundle::{
    default_name, viewer_bundle, viewer_colors, viewer_defaults, ViewerDefaults,
};

use super::{usage_error, write_viewer_outputs};
use crate::output::Output;

pub fn run(args: &BuildTreeArgs) -> Result<()> {
    if args.input.is_some() == args.newick.is_some() {
        return Err(usage_error("Give exactly one of -i/--input and --newick."));
    }
    if args.newick.is_some() && (args.distance != "auto" || args.algorithm != "upgma") {
        return Err(usage_error(
            "--distance and --algorithm apply to a matrix, not to a --newick tree.",
        ));
    }
    let viewer = args.html_out.is_some() || args.dvb_out.is_some();
    if args.msa.is_some() && !viewer {
        return Err(usage_error(
            "--msa is shown only in the viewer, so it requires --html_out or --dvb_out.",
        ));
    }
    if args.color_table.is_some() && args.color_by.is_none() {
        return Err(usage_error("--color_table requires --color_by."));
    }
    let max_bytes = crate::max_output_gb_to_bytes(args.max_output_gb.0)?;
    let color_table = args
        .color_table
        .as_deref()
        .map(crate::read_color_table)
        .transpose()?;
    let subset = parse_name_list(
        args.subset.as_deref().unwrap_or_default(),
        args.subset_file.as_deref(),
    )?;
    if args.color_table_out.is_some() && args.color_by.is_none() && color_table.is_none() {
        anyhow::bail!(
            "color_table_out requires either color_by or color_table, otherwise there are no \
             colors to write."
        );
    }
    // Upstream refuses a negative offset only where it builds a palette.
    let palette_offset = match usize::try_from(args.palette_offset) {
        Ok(n) => n,
        Err(_) if args.color_by.is_none() || color_table.is_some() => 0,
        Err(_) => anyhow::bail!(
            "The palette offset must be 0 or more, not {}.",
            args.palette_offset
        ),
    };
    // Upstream colors no group at all for a negative count.
    let max_color_groups = args
        .max_color_groups
        .map(|n| usize::try_from(n).unwrap_or(0));

    let tree = match (&args.input, &args.newick) {
        (Some(input), _) => {
            let matrix = read_matrix(input).with_context(|| format!("reading matrix {input}"))?;
            tree_from_matrix(
                &subset_matrix(&matrix, &subset),
                &args.algorithm,
                &args.distance,
            )?
        }
        (None, Some(path)) => {
            let tree = tree_from_newick(path)?;
            match &subset {
                Some(keep) => subset_tree(&tree, keep)?,
                None => tree,
            }
        }
        (None, None) => unreachable!("checked above"),
    };
    let leaf_names = tree.leaf_names();

    if let Some(path) = &args.newick_out {
        let quiet = args.common.quiet;
        let text = newick::write(&tree, |from, to| {
            if !quiet {
                log::warn(format!(
                    "sequence name {from} contains characters that don't play well with \
                     newick, so for newick export, it has been changed to {to}"
                ));
            }
        });
        let mut out = Output::create(path)?;
        writeln!(out, "{text}")?;
        out.finish()?;
    }

    let metadata = args.metadata.clone().unwrap_or_default();
    let node_data = read_node_table(&leaf_names, Vec::new(), &metadata)?;
    if let Some(color_by) = &args.color_by {
        if !node_data.columns.contains(color_by) {
            anyhow::bail!(
                "Requested color_by column '{color_by}' was not found in the merged metadata."
            );
        }
    }
    let colors = resolve_colors(
        &node_data,
        &ColorOptions {
            color_by: args.color_by.as_deref(),
            color_table: color_table.as_ref(),
            max_color_groups,
            palette_offset,
        },
    );

    if let Some(path) = &args.xgmml_out {
        let network = tree_network(&tree, &metadata)?;
        let fills = colors
            .as_ref()
            .map(|c| network_fills(&network, &tree, &c.fills));
        let name = Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        let mut out = Output::create(path)?;
        write_xgmml_with(
            &mut out,
            name,
            &network.table,
            &network.edges,
            &XgmmlExtras {
                edge_score_name: SCORE_COLUMN,
                node_fills: fills.as_deref(),
                positions: Some(&network.positions),
                ..Default::default()
            },
        )?;
        out.finish()?;
    }

    if let Some(path) = &args.color_table_out {
        let colors = colors.as_ref().context(
            "color_table_out requires either color_by or color_table, otherwise there are no \
             colors to write.",
        )?;
        let values: Vec<String> = colors.table.keys().cloned().collect();
        let mut out = Output::create(path)?;
        for value in sort_palette_values(&values) {
            writeln!(out, "{value}\t{}", colors.table[&value])?;
        }
        if colors.palette_has_missing {
            // Upstream writes the missing values' key, None, as an empty string here.
            writeln!(out, "\t{}", domainator_core::palette::OTHER_COLOR)?;
        }
        out.finish()?;
    }

    if viewer {
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
                    max_color_groups,
                ),
                label_by: args.label_by.as_deref(),
                categorical: args.categorical.as_deref().unwrap_or_default(),
            },
        )?;
        let msa = match &args.msa {
            Some(path) => {
                let (msa, warnings) = read_msa(path, &leaf_names)?;
                for warning in warnings {
                    log::warn(warning);
                }
                Some(msa)
            }
            None => None,
        };
        let bundle = viewer_bundle(
            &name,
            &leaf_names,
            &node_data,
            defaults,
            "tree",
            tree_view(&tree, msa),
        );
        write_viewer_outputs(
            &bundle,
            args.html_out.as_deref(),
            args.dvb_out.as_deref(),
            max_bytes,
            &["--subset", "--subset_file"],
        )?;
    }
    Ok(())
}
