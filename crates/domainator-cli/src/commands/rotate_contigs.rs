use std::path::Path;

use anyhow::{anyhow, Context, Result};
use domainator_config::args::RotateContigsArgs;
use domainator_core::io::{detect_filetype, FileType};
use domainator_core::tools::domainate;
use domainator_core::tools::rotate_contigs::{check_options, rotate_contigs, RotateContigsOptions};
use domainator_core::tools::select_common::parse_name_list;

use super::domainate::count_peptides;
use super::{cpu_threads, load_typed_inputs, usage_error};
use crate::output::{is_stdin, write_seq_outputs};

pub fn run(args: &RotateContigsArgs) -> Result<()> {
    if args.evalue.0 > 10.0 {
        return Err(usage_error("-e/--evalue must be <= 10."));
    }
    let domains = parse_name_list(
        args.domains.as_deref().unwrap_or_default(),
        args.domains_file.as_deref(),
    )?;
    if args.z == 0 && args.input.iter().any(|p| is_stdin(p)) {
        return Err(usage_error(
            "-Z 0 counts the sequences in the input files, so it can't be used when reading \
             from stdin.",
        ));
    }
    let cpu = cpu_threads(args.cpu)?;
    let opts = RotateContigsOptions {
        z: 0.0,
        evalue: args.evalue.0,
        domains: domains.filter(|d| !d.is_empty()),
        databases: args.databases.as_ref().map(|d| d.iter().cloned().collect()),
        annotation_type: args
            .annotation_type
            .parse()
            .map_err(|e: String| anyhow!(e))?,
        has_search_hit: args.has_search_hit,
        anchor: args.anchor.parse().map_err(|e: String| anyhow!(e))?,
        keep_direction: args.keep_direction,
        circularize: args.circularize,
        keep_name: args.keep_name,
        keep_annotation: args.keep_annotation,
        gene_call: crate::gene_call_request(args.gene_call.as_deref(), cpu)?,
        cpu,
        max_mode: args.max_mode,
        batch_size: usize::try_from(args.batch_size).unwrap_or(0),
    };
    check_options(args.reference.is_some(), &opts).map_err(|e| anyhow!(e))?;
    let refs = args
        .reference
        .as_deref()
        .map(domainate::read_hmm_references)
        .transpose()
        .context("reading references")?;

    let mut records = Vec::new();
    let mut z = args.z;
    for input in &args.input {
        let loaded = load_typed_inputs(std::slice::from_ref(input), "nucleotide")?;
        // Upstream's count_peptides: one per FASTA record, else one per protein record
        // or CDS feature.
        if args.z == 0 {
            z += match detect_filetype(Path::new(input)) {
                Some(FileType::Fasta) => loaded.len() as i64,
                _ => loaded.iter().map(count_peptides).sum(),
            };
        }
        records.extend(loaded);
    }
    let rotated = rotate_contigs(
        records,
        refs.as_ref(),
        &RotateContigsOptions {
            z: z as f64,
            ..opts
        },
    )
    .context("rotating contigs")?;
    write_seq_outputs(
        &rotated,
        args.genbank_out.as_deref(),
        args.fasta_out.as_deref(),
    )
}
