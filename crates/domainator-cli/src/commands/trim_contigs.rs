use anyhow::{anyhow, Result};
use domainator_config::args::TrimContigsArgs;
use domainator_core::tools::select_common::parse_name_list;
use domainator_core::tools::trim_contigs::{
    trim_contigs_expr, trim_record, validate_range, TrimContigsOptions,
};

use crate::input::{InputMode, SeqInputs};
use crate::output::write_seq_stream;

pub fn run(args: &TrimContigsArgs) -> Result<()> {
    // Only validated here: the run's pool is sized from it before dispatch.
    super::cpu_threads(args.cpu)?;
    let (kb_truncation, cds_truncation) = validate_range(
        args.kb_range.map(|v| v.0),
        args.kb_left.map(|v| v.0),
        args.kb_right.map(|v| v.0),
        args.cds_range,
        args.cds_left,
        args.cds_right,
    )
    .map_err(|e| anyhow!(e))?;
    let opts = TrimContigsOptions {
        kb_truncation,
        cds_truncation,
        strip_domains: parse_name_list(
            args.domains.as_deref().unwrap_or_default(),
            args.domains_file.as_deref(),
        )?,
        domain_expr: args.domain_expr.clone(),
        no_domain: args.unannotated,
        evalue: args.evalue.map(|e| e.0),
        databases: args.databases.as_ref().map(|d| d.iter().cloned().collect()),
    };
    let expr = trim_contigs_expr(&opts).map_err(|e| anyhow!(e))?;
    let trimmed = SeqInputs::new(&args.input, InputMode::DETECT)
        .contigs(
            args.contigs.as_deref().unwrap_or_default(),
            args.contigs_file.as_deref(),
        )?
        .filter_map(|rec| match rec {
            Ok(rec) => trim_record(rec, &opts, expr.as_ref()).map(Ok),
            Err(e) => Some(Err(e)),
        });
    write_seq_stream(
        trimmed,
        args.genbank_out.as_deref(),
        args.fasta_out.as_deref(),
        None,
    )?;
    Ok(())
}
