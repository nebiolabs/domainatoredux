use std::path::Path;

use anyhow::{anyhow, Context, Result};
use domainator_config::args::DomainateArgs;
use domainator_core::io::{detect_filetype, FileType};
use domainator_core::seq::SeqRecord;
use domainator_core::tools::domainate::{self, DomainateOptions};

use super::{cpu_threads, usage_error};
use crate::output::{write_seq_outputs_capped, OutputCap};

pub fn run(args: &DomainateArgs) -> Result<()> {
    if args.evalue.0 > 10.0 {
        return Err(usage_error("-e/--evalue must be <= 10."));
    }
    if args.z == 0 && args.gene_call.is_some() {
        return Err(usage_error(
            "-Z 0 can't be used with --gene_call: give the database size explicitly.",
        ));
    }
    let Some(references) = &args.reference else {
        return Err(usage_error(
            "You must specify at least one reference database (-r/--reference or --foldseek).",
        ));
    };
    if args.max_hits_per_cds.is_some_and(|n| n < 1) {
        return Err(usage_error(
            "--max_hits_per_cds must be at least 1. Leave it out to report all hits.",
        ));
    }
    let cpu = cpu_threads(args.cpu)?;
    let max_bytes = crate::max_output_gb_to_bytes(args.max_output_gb.0)?;

    let dbs = domainate::read_hmm_references(references).context("reading references")?;
    let mut records = Vec::new();
    let mut z = args.z;
    for input in &args.input {
        let path = Path::new(input);
        let loaded = crate::load_typed(path, &args.alphabet)?;
        // Upstream's count_peptides: one per FASTA record, else one per protein record
        // or CDS feature, counted before any taxonomy filter.
        if args.z == 0 {
            z += match detect_filetype(path) {
                Some(FileType::Fasta) => loaded.len() as i64,
                _ => loaded.iter().map(count_peptides).sum(),
            };
        }
        records.extend(loaded);
    }
    if dbs.has_nucleotide() && records.iter().any(SeqRecord::is_protein) {
        anyhow::bail!("Cannot search protein inputs with a nucleotide reference.");
    }
    if args.no_annotations && !args.hits_only {
        anyhow::bail!(
            "You've specified no_annotations, without specifying hits_only. This seems like a \
             useless thing to do. If you want to select a subset of contigs, please use \
             domainatoredux select_by_contig."
        );
    }
    let allowed_taxids = crate::compile_allowed_taxids(
        args.include_taxids.as_deref().unwrap_or_default(),
        args.exclude_taxids.as_deref().unwrap_or_default(),
        args.taxonomy_expr.as_deref(),
        args.ncbi_taxonomy_path.as_deref(),
    )?;

    let opts = DomainateOptions {
        z: z as f64,
        evalue: args.evalue.0,
        min_evalue: args.min_evalue.0,
        max_hits: args
            .max_hits_per_cds
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(usize::MAX),
        max_overlap: args.max_overlap.0,
        no_annotations: args.no_annotations,
        hits_only: args.hits_only,
        overlap_by_db: args.overlap_by_db,
        max_mode: args.max_mode,
        allowed_taxids,
        cpu,
        partial: args.partial.parse().map_err(|e: String| anyhow!(e))?,
        gene_call: crate::gene_call_request(args.gene_call.as_deref(), cpu)?,
        ..Default::default()
    };
    let annotated = domainate::domainate(records, &dbs, &opts).context("running domainate")?;
    write_seq_outputs_capped(
        &annotated,
        args.genbank_out.as_deref(),
        args.fasta_out.as_deref(),
        max_bytes.map(|max_bytes| OutputCap {
            max_bytes,
            mitigation: &["--hits_only", "--max_hits_per_cds", "--evalue"],
            extra_guidance: None,
        }),
    )
}

/// Upstream `count_peptides_in_record`: a protein record is one peptide, a
/// nucleotide record one per CDS.
fn count_peptides(rec: &SeqRecord) -> i64 {
    if rec.is_protein() {
        1
    } else {
        rec.features.iter().filter(|f| f.kind == "CDS").count() as i64
    }
}
