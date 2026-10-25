use std::io::Write;

use anyhow::Result;
use domainator_config::args::CleanSequencesArgs;
use domainator_core::tools::clean_sequences::{
    clean_sequences, CleanSequencesOptions, CleaningStats,
};

use super::load_typed_inputs;
use crate::output::{write_seq_outputs, Output};

pub fn run(args: &CleanSequencesArgs) -> Result<()> {
    let records = load_typed_inputs(&args.input, &args.alphabet)?;
    let opts = CleanSequencesOptions {
        fasta_type: args.alphabet.clone(),
        clean_name: args.clean_name,
        deduplicate_names: args.deduplicate_names,
        strip: args.strip,
        filter_by_aa: args.filter_by_aa,
    };
    let mut stats = CleaningStats::default();
    let out = clean_sequences(records, &opts, &mut stats);
    write_seq_outputs(&out, args.genbank_out.as_deref(), args.fasta_out.as_deref())?;
    // Six lines, spelled exactly as `clean_sequences.py` prints them -- the log is the
    // tool's report, so its wording is part of the output.
    let msg = format!(
        "Input sequences:            {}\n\
         Output sequences:           {}\n\
         Filtered (non-canonical):   {}\n\
         Filtered (empty after strip): {}\n\
         Names changed:              {}\n\
         Sequences changed:          {}\n",
        stats.input_count,
        stats.output_count,
        stats.filtered_count,
        stats.empty_after_strip_count,
        stats.names_changed_count,
        stats.sequences_changed_count,
    );
    match &args.log_out {
        Some(path) => {
            let mut log = Output::create(path)?;
            log.write_all(msg.as_bytes())?;
            log.finish()?;
        }
        None => eprint!("{msg}"),
    }
    Ok(())
}
