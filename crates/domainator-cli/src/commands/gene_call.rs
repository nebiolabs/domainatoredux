use anyhow::{Context, Result};
use domainator_config::args::GeneCallArgs;
use domainator_core::tools::gene_call::{gene_call, GeneCall, GeneCallMode, GeneCallOptions};

use super::cpu_threads;
use crate::input::{load_all, InputMode, SeqInputs};
use crate::output::{write_seq_outputs, SeqWriter};

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
    let mode = InputMode::Detect {
        fasta_type: &args.alphabet,
    };
    let (mut added, mut count) = (0, 0);
    if request.options.mode == GeneCallMode::Single {
        // Single mode trains on every record together, so they are all read first.
        let mut records = load_all(&args.input, mode)?;
        added = gene_call(&mut records, &request).context("calling genes")?;
        count = records.len();
        write_seq_outputs(&records, Some(&args.genbank_out), None)?;
    } else {
        // Meta mode picks a model per sequence, so the records are called a chunk at a
        // time (one Orphos run each) and written as they are done.
        let mut out = SeqWriter::new(Some(&args.genbank_out), None, None)?;
        let mut records = SeqInputs::new(&args.input, mode);
        loop {
            let mut chunk = Vec::new();
            let mut residues = 0;
            while chunk.len() < RECORDS_PER_RUN && residues < RESIDUES_PER_RUN {
                let Some(record) = records.next() else {
                    break;
                };
                let record = record?;
                residues += record.seq.len();
                chunk.push(record);
            }
            if chunk.is_empty() {
                break;
            }
            added += gene_call(&mut chunk, &request).context("calling genes")?;
            count += chunk.len();
            for record in &chunk {
                out.write(record)?;
            }
        }
        out.finish()?;
    }
    domainator_core::log::info(format_args!("called {added} genes on {count} records"));
    Ok(())
}

/// Records per Orphos run in meta mode ...
const RECORDS_PER_RUN: usize = 1000;
/// ... or fewer, once a run holds this many residues.
const RESIDUES_PER_RUN: usize = 64 << 20;
