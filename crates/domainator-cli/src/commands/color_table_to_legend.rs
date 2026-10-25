use std::collections::HashMap;
use std::io::{Read, Write};

use anyhow::{anyhow, Context, Result};
use domainator_config::args::ColorTableToLegendArgs;
use domainator_core::tools::color_genbank::read_color_table;
use domainator_core::tools::color_table_to_legend::color_table_to_legend;

use crate::output::{is_stdin, Output};

pub fn run(args: &ColorTableToLegendArgs) -> Result<()> {
    // Merge the tables in order: a later file overrides an earlier file's color for a
    // key, which keeps its first position.
    let mut table: Vec<(String, String)> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for path in &args.input {
        let text = if is_stdin(path) {
            let mut text = String::new();
            std::io::stdin()
                .read_to_string(&mut text)
                .context("reading standard input")?;
            text
        } else {
            std::fs::read_to_string(path).with_context(|| format!("reading {path}"))?
        };
        for (key, color) in read_color_table(&text).map_err(|e| anyhow!(e))? {
            match index.get(&key) {
                Some(&i) => table[i].1 = color,
                None => {
                    index.insert(key.clone(), table.len());
                    table.push((key, color));
                }
            }
        }
    }
    let mut out = Output::create(&args.svg_out)?;
    out.write_all(color_table_to_legend(&table, &args.title).as_bytes())?;
    out.finish()
}
