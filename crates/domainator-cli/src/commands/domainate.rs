use std::path::Path;

use anyhow::{anyhow, Context, Result};
use domainator_config::args::DomainateArgs;
use domainator_core::io::{detect_filetype, FileType};
use domainator_core::search::foldseek::FoldseekSearch;
use domainator_core::seq::SeqRecord;
use domainator_core::tools::domainate::{self, DomainateOptions};
use domainator_core::tools::external::{resolve_binary, FOLDSEEK};

use super::{cpu_threads, usage_error};
use crate::output::{write_seq_outputs_capped, OutputCap};

/// Why `--esm2_3di_weights` is refused: upstream runs the ESM-2 3B 3Di model in PyTorch.
const ESM2_3DI_UNSUPPORTED: &str = "--esm2_3di_weights is not supported by domainatoredux: \
    the ESM-2 3B 3Di model runs in PyTorch. Use --prostt5_weights instead, which has foldseek \
    predict the 3Di sequences with ProstT5 (download its weights with 'foldseek databases \
    ProstT5 prostt5_weights tmp').";

pub fn run(args: &DomainateArgs) -> Result<()> {
    if args.evalue.0 > 10.0 {
        return Err(usage_error("-e/--evalue must be <= 10."));
    }
    if args.z == 0 && args.gene_call.is_some() {
        return Err(usage_error(
            "-Z 0 can't be used with --gene_call: give the database size explicitly.",
        ));
    }
    if args.reference.is_none() && args.foldseek.is_none() {
        return Err(usage_error(
            "You must specify at least one reference database (-r/--reference or --foldseek).",
        ));
    }
    if args.foldseek.is_some() && args.esm2_3di_weights.is_none() == args.prostt5_weights.is_none()
    {
        return Err(usage_error(
            "--foldseek needs exactly one of --esm2_3di_weights and --prostt5_weights.",
        ));
    }
    if args.foldseek.is_some() && args.esm2_3di_weights.is_some() {
        return Err(usage_error(ESM2_3DI_UNSUPPORTED));
    }
    if args.max_hits_per_cds.is_some_and(|n| n < 1) {
        return Err(usage_error(
            "--max_hits_per_cds must be at least 1. Leave it out to report all hits.",
        ));
    }
    let cpu = cpu_threads(args.cpu)?;
    let max_bytes = crate::max_output_gb_to_bytes(args.max_output_gb.0)?;

    let dbs = domainate::read_hmm_references(args.reference.as_deref().unwrap_or_default())
        .context("reading references")?;
    let foldseek = match (&args.foldseek, &args.prostt5_weights) {
        (Some(databases), Some(weights)) => Some(FoldseekSearch::new(
            databases,
            resolve_binary(
                &FOLDSEEK,
                args.foldseek_path.as_deref(),
                "Install foldseek (for example 'conda install -c conda-forge -c bioconda \
                 foldseek').",
            )?,
            weights.clone(),
            args.prostt5_device.clone(),
            args.foldseek_device.clone(),
        )),
        _ => None,
    };
    let mut records = Vec::new();
    let mut z = args.z;
    for input in &args.input {
        let path = Path::new(input);
        let loaded = crate::load_typed(path, &args.alphabet)?;
        // Upstream's count_peptides: one per FASTA record, else one per protein record
        // or CDS feature.
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
    let opts = DomainateOptions {
        z: z as f64,
        evalue: args.evalue.0,
        min_evalue: args.min_evalue.0,
        max_hits: args
            .max_hits_per_cds
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(usize::MAX),
        max_overlap: args.max_overlap.0,
        overlap_by_db: args.overlap_by_db,
        max_mode: args.max_mode,
        cpu,
        partial: args.partial.parse().map_err(|e: String| anyhow!(e))?,
        gene_call: crate::gene_call_request(args.gene_call.as_deref(), cpu)?,
        foldseek,
        ..Default::default()
    };
    let annotated = domainate::domainate(records, &dbs, &opts).context("running domainate")?;
    write_seq_outputs_capped(
        &annotated,
        args.genbank_out.as_deref(),
        args.fasta_out.as_deref(),
        max_bytes.map(|max_bytes| OutputCap {
            max_bytes,
            mitigation: &["--max_hits_per_cds", "--evalue"],
            extra_guidance: None,
        }),
    )
}

/// Upstream `count_peptides_in_record`: a protein record is one peptide, a
/// nucleotide record one per CDS.
pub(super) fn count_peptides(rec: &SeqRecord) -> i64 {
    if rec.is_protein() {
        1
    } else {
        rec.features.iter().filter(|f| f.kind == "CDS").count() as i64
    }
}
