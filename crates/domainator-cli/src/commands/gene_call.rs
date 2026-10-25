use anyhow::{Context, Result};
use domainator_config::args::GeneCallArgs;
use domainator_core::tools::gene_call::{gene_call, GeneCall, GeneCallOptions};

use super::{cpu_threads, load_typed_inputs};
use crate::output::write_seq_outputs;

pub fn run(args: &GeneCallArgs) -> Result<()> {
    let request = GeneCall {
        scope: crate::parse_choice(&args.scope)?,
        options: GeneCallOptions {
            mode: crate::parse_choice(&args.mode)?,
            translation_table: args.translation_table,
            closed: args.closed,
            mask: args.mask,
            cpu: cpu_threads(args.cpu)?,
            bin_path: args.orphos_path.clone(),
        },
    };
    let mut records = load_typed_inputs(&args.input, &args.alphabet)?;
    let added = gene_call(&mut records, &request).context("calling genes")?;
    write_seq_outputs(&records, Some(&args.genbank_out), None)?;
    if !args.common.quiet {
        eprintln!("called {added} genes on {} records", records.len());
    }
    Ok(())
}
