use anyhow::{anyhow, Result};
use domainator_config::args::FilterDomainsArgs;
use domainator_core::tools::filter_domains::{
    filter_domains, AnnotationType, FilterDomainsOptions,
};
use domainator_core::tools::select_common::parse_name_list;

use super::load_seq_inputs;
use crate::output::write_seq_outputs;

pub fn run(args: &FilterDomainsArgs) -> Result<()> {
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
        annotation_type: AnnotationType::parse(&args.annotation_type).map_err(|e| anyhow!(e))?,
    };
    let mut records = load_seq_inputs(&args.input)?;
    crate::filter_contigs(&mut records, &args.contigs, args.contigs_file.as_deref())?;
    let out = filter_domains(records, &opts);
    write_seq_outputs(&out, args.genbank_out.as_deref(), args.fasta_out.as_deref())
}
