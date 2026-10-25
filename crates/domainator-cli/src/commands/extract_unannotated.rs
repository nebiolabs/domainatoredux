use anyhow::{anyhow, Result};
use domainator_config::args::ExtractUnannotatedArgs;
use domainator_core::tools::extract_unannotated::{extract_unannotated, ExtractUnannotatedOptions};

use super::load_seq_inputs;
use crate::output::write_seq_outputs;

pub fn run(args: &ExtractUnannotatedArgs) -> Result<()> {
    let opts = ExtractUnannotatedOptions {
        evalue: args.evalue.map(|e| e.0),
        score: args.min_score.map(|s| s.0),
        keep_direction: args.keep_direction,
        keep_name: args.keep_name,
        search_hits: args.annotation_type == "domain_search",
        databases: args.databases.as_ref().map(|d| d.iter().cloned().collect()),
        largest: args.largest,
        lb: args.min_length,
    };
    let mut records = load_seq_inputs(&args.input)?;
    crate::filter_contigs(&mut records, &args.contigs, args.contigs_file.as_deref())?;
    let out = extract_unannotated(&records, &opts).map_err(|e| anyhow!(e))?;
    write_seq_outputs(&out, args.genbank_out.as_deref(), args.fasta_out.as_deref())
}
