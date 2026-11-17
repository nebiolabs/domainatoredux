use anyhow::{anyhow, Result};
use domainator_config::args::FilterDomainsArgs;
use domainator_core::tools::filter_domains::{filter_record, AnnotationType, FilterDomainsOptions};
use domainator_core::tools::select_common::parse_name_list;

use crate::input::{InputMode, SeqInputs};
use crate::output::write_seq_stream;

pub fn run(args: &FilterDomainsArgs) -> Result<()> {
    // Only validated here: the run's pool is sized from it before dispatch.
    super::cpu_threads(args.cpu)?;
    let opts = FilterDomainsOptions {
        evalue: args.evalue.map_or(f64::INFINITY, |e| e.0),
        max_overlap: args.max_overlap.0,
        databases_keep: args
            .keep_databases
            .as_ref()
            .map(|d| d.iter().cloned().collect()),
        databases_filter: args.databases.as_ref().map(|d| d.iter().cloned().collect()),
        selected_domains: parse_name_list(
            args.domains.as_deref().unwrap_or_default(),
            args.domains_file.as_deref(),
        )?,
        annotation_type: args
            .annotation_type
            .parse::<AnnotationType>()
            .map_err(|e| anyhow!(e))?,
    };
    let records = SeqInputs::new(&args.input, InputMode::DETECT)
        .contigs(&args.contigs, args.contigs_file.as_deref())?
        .map(|rec| {
            let mut rec = rec?;
            filter_record(&mut rec, &opts);
            Ok(rec)
        });
    write_seq_stream(
        records,
        args.genbank_out.as_deref(),
        args.fasta_out.as_deref(),
        None,
    )?;
    Ok(())
}
