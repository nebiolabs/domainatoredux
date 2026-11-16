use anyhow::Result;
use domainator_config::args::GenbankToFastaArgs;

use crate::input::{InputMode, SeqInputs};
use crate::output::write_seq_stream;

/// Upstream passes `parse_seqfiles`' generator straight to `write_seq_outputs`, so one
/// record is in memory at a time; so here.
pub fn run(args: &GenbankToFastaArgs) -> Result<()> {
    // Not upstream's: it reads every input as GenBank and so refuses FASTA, but
    // converting FASTA to FASTA is harmless and the port always has.
    let records = SeqInputs::new(&args.input, InputMode::GenbankUnlessFasta);
    write_seq_stream(records, None, args.fasta_out.as_deref(), None)?;
    Ok(())
}
