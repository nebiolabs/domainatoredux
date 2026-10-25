use anyhow::Result;
use domainator_config::args::GenbankToFastaArgs;

use super::load_seq_inputs;
use crate::output::write_seq_outputs;

pub fn run(args: &GenbankToFastaArgs) -> Result<()> {
    let records = load_seq_inputs(&args.input)?;
    write_seq_outputs(&records, None, args.fasta_out.as_deref())
}
