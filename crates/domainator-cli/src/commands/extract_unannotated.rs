use anyhow::{anyhow, Result};
use domainator_config::args::ExtractUnannotatedArgs;
use domainator_core::tools::extract_unannotated::{
    extract_unannotated_from_record, ExtractUnannotatedOptions,
};

use crate::input::{flat_map_records, InputMode, SeqInputs};
use crate::output::write_seq_stream;

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
    let records = SeqInputs::new(&args.input, InputMode::DETECT)
        .contigs(&args.contigs, args.contigs_file.as_deref())?;
    let regions = flat_map_records(records, |rec| {
        let mut out = Vec::new();
        extract_unannotated_from_record(&rec, &opts, &mut out).map_err(|e| anyhow!(e))?;
        Ok(out)
    });
    write_seq_stream(
        regions,
        args.genbank_out.as_deref(),
        args.fasta_out.as_deref(),
        None,
    )?;
    Ok(())
}
