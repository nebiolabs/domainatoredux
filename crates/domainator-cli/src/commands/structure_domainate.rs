use anyhow::{bail, Result};
use domainator_config::args::StructureDomainateArgs;
use domainator_core::tools::structure_domainate::{structure_domainate, StructureDomainateOptions};

use super::structure_to_genbank::{aligner, check_backend, max_seqs};
use crate::output::{write_seq_outputs_capped, OutputCap};

pub fn run(args: &StructureDomainateArgs) -> Result<()> {
    let max_hits_per_cds = match args.max_hits_per_cds {
        Some(n) if n < 1 => {
            bail!("--max_hits_per_cds must be at least 1. Leave it out to add every annotation.")
        }
        n => n.and_then(|n| usize::try_from(n).ok()),
    };
    let aligner = aligner(&args.backend, Some(&args.search))?;
    let metrics = args.search.metrics.clone().unwrap_or_default();
    check_backend(&aligner, &args.search, &metrics)?;
    let max_bytes = crate::max_output_gb_to_bytes(args.max_output_gb.0)?;

    let opts = StructureDomainateOptions {
        evalue: args.evalue.0,
        min_evalue: args.min_evalue.0,
        max_hits_per_cds,
        max_overlap: args.max_overlap.0,
        overlap_by_db: args.overlap_by_db,
        alignment_type: args.search.alignment_type,
        metrics,
        max_seqs: max_seqs(&args.search)?,
        db_out: args.db_out.clone(),
        database_name: args.database_name.clone(),
        hits_tsv_out: args.hits_tsv_out.clone(),
    };
    let records = structure_domainate(&args.input, &args.reference, &aligner, &opts)?;
    write_seq_outputs_capped(
        &records,
        args.genbank_out.as_deref(),
        args.fasta_out.as_deref(),
        max_bytes.map(|max_bytes| OutputCap {
            max_bytes,
            mitigation: &["--max_hits_per_cds", "-e/--evalue"],
            extra_guidance: None,
        }),
    )
}
