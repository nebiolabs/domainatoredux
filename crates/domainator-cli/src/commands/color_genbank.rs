use std::io::Write;

use anyhow::{anyhow, Context, Result};
use domainator_config::args::ColorGenbankArgs;
use domainator_core::tools::color_genbank::{
    color_genbank, normalize_color_hex, read_color_table, ColorGenbankOptions, ColorMap,
    ColorTargets,
};

use super::load_seq_inputs;
use crate::output::{write_seq_outputs, Output};

pub fn run(args: &ColorGenbankArgs) -> Result<()> {
    let targets = ColorTargets::parse(&args.target).map_err(|e| anyhow!(e))?;
    let mut color_map = match &args.color_table {
        Some(path) => {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("reading color table {path}"))?;
            ColorMap::from_table(read_color_table(&text).map_err(|e| anyhow!(e))?)
        }
        None => ColorMap::auto(),
    };
    let search_hit_color = args
        .search_hit_color
        .as_deref()
        .map(normalize_color_hex)
        .transpose()
        .map_err(|e| anyhow!(e))?;
    let opts = ColorGenbankOptions {
        targets,
        search_hit_color,
        clear: args.clear,
        databases: args.databases.as_ref().map(|d| d.iter().cloned().collect()),
        color_feature: if args.domain_database {
            "database"
        } else {
            "name"
        }
        .to_string(),
    };
    let mut records = load_seq_inputs(&args.input)?;
    crate::filter_contigs(&mut records, &args.contigs, args.contigs_file.as_deref())?;
    let out = color_genbank(records, &mut color_map, &opts);
    write_seq_outputs(&out, args.genbank_out.as_deref(), args.fasta_out.as_deref())?;
    if let Some(path) = &args.color_table_out {
        let mut table = Output::create(path)?;
        for (annotation, color) in color_map.items() {
            writeln!(table, "{annotation}\t{color}")?;
        }
        table.finish()?;
    }
    Ok(())
}
