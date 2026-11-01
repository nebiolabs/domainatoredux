use anyhow::{anyhow, Result};
use domainator_config::args::ExtractUnannotatedArgs;
use domainator_core::tools::extract_unannotated::{extract_unannotated, ExtractUnannotatedOptions};

use super::load_seq_inputs;
use crate::output::write_seq_outputs;

pub fn run(args: &ExtractUnannotatedArgs) -> Result<()> {
    let opts = ExtractUnannotatedOptions {
        evalue: args.evalue.map(|e| e.0),
        score: args.min_annotation_score.map(|s| s.0),
        keep_parent_id: args.keep_parent_id,
        annotation_type: args
            .annotation_type
            .parse()
            .map_err(|e: String| anyhow!(e))?,
        databases: args.databases.as_ref().map(|d| d.iter().cloned().collect()),
        largest: args.largest,
        lb: args.min_region_length,
        taxonomy: crate::taxonomy_filter(&args.taxonomy)?,
    };
    let mut records = load_seq_inputs(&args.input)?;
    crate::filter_contigs(&mut records, &args.contigs, args.contigs_file.as_deref())?;
    let out = extract_unannotated(&records, &opts).map_err(|e| anyhow!(e))?;
    write_seq_outputs(&out, args.genbank_out.as_deref(), args.fasta_out.as_deref())
}
