use std::path::Path;

use anyhow::{anyhow, Context, Result};
use domainator_config::args::DomainateArgs;
use domainator_core::io::{detect_filetype, FileType};
use domainator_core::search::foldseek::FoldseekSearch;
use domainator_core::seq::SeqRecord;
use domainator_core::tools::domainate::{self, DomainateOptions};
use domainator_core::tools::external::{resolve_binary, FOLDSEEK};

use super::{cpu_threads, usage_error};
use crate::input::{until_error, InputMode, SeqInputs};
use crate::output::{OutputCap, SeqWriter};

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
    let mode = InputMode::Detect {
        fasta_type: &args.alphabet,
    };
    // `-Z 0` counts the peptides first, in a pass of its own, as upstream does.
    let mut z = args.z;
    if z == 0 {
        for input in &args.input {
            let fasta = detect_filetype(Path::new(input)) == Some(FileType::Fasta);
            for record in SeqInputs::new(std::slice::from_ref(input), mode) {
                // Upstream's count_peptides: one per FASTA record, else one per protein
                // record or CDS feature.
                z += if fasta { 1 } else { count_peptides(&record?) };
            }
        }
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
    let has_nucleotide = dbs.has_nucleotide();
    let checked = SeqInputs::new(&args.input, mode).map(|record| {
        let record = record?;
        if has_nucleotide && record.is_protein() {
            anyhow::bail!("Cannot search protein inputs with a nucleotide reference.");
        }
        Ok(record)
    });
    // A read error (or a protein record) ends the input; it is reported as itself.
    let mut input_error = None;
    let records = until_error(checked, &mut input_error).map(Ok::<_, anyhow::Error>);
    let mut out = SeqWriter::new(
        args.genbank_out.as_deref(),
        args.fasta_out.as_deref(),
        max_bytes.map(|max_bytes| OutputCap {
            max_bytes,
            mitigation: &["--max_hits_per_cds", "--evalue"],
            extra_guidance: None,
        }),
    )?;
    // The cap's error (or a write error) stops the run, unwrapped.
    let mut write_error = None;
    let annotated = domainate::domainate_stream(records, &dbs, &opts, args.batch_size, |rec| {
        out.write(&rec).map_err(|e| {
            let message = anyhow!("{e:#}");
            write_error = Some(e);
            message
        })
    });
    if let Some(e) = input_error.or(write_error) {
        return Err(e);
    }
    annotated.context("running domainate")?;
    out.finish()?;
    Ok(())
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
