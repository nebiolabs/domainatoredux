use anyhow::{bail, Result};
use domainator_config::args::{
    StructureAlignmentArgs, StructureBackendArgs, StructureToGenbankArgs,
};
use domainator_core::tools::structure_lib::{Aligner, BackendOptions};
use domainator_core::tools::structure_to_genbank::structure_to_genbank;

use crate::output::{write_seq_outputs_capped, OutputCap};

pub fn run(args: &StructureToGenbankArgs) -> Result<()> {
    let aligner = aligner(&args.backend, None)?;
    if args.store_3di && !aligner.supports_3di() {
        bail!(
            "--store_3di is not available with the '{}' backend, which does not expose a \
             per-residue structural alphabet. Use --algorithm foldseek, or drop --store_3di.",
            aligner.name()
        );
    }
    let max_bytes = crate::max_output_gb_to_bytes(args.max_output_gb.0)?;
    let records = structure_to_genbank(
        &args.input,
        &aligner,
        args.db_out.as_deref(),
        args.store_3di,
    )?;
    write_seq_outputs_capped(
        &records,
        args.genbank_out.as_deref(),
        args.fasta_out.as_deref(),
        max_bytes.map(|max_bytes| OutputCap {
            max_bytes,
            mitigation: &["--max_output_gb"],
            extra_guidance: None,
        }),
    )
}

/// The selected backend with its executable resolved (upstream `build_aligner`).
/// `search` carries `--device`, which only the searching tools have.
pub(crate) fn aligner(
    backend: &StructureBackendArgs,
    search: Option<&StructureAlignmentArgs>,
) -> Result<Aligner> {
    let options = BackendOptions {
        algorithm: backend.algorithm.parse().map_err(anyhow::Error::msg)?,
        foldseek_path: backend.foldseek_path.clone(),
        reseek_path: backend.reseek_path.clone(),
        foldseek_args: backend.foldseek_args.clone(),
        reseek_args: backend.reseek_args.clone(),
        cpu: backend.cpu,
        device: search.map(|s| s.device.clone()),
        tmp_dir: backend.tmp_dir.clone(),
        reseek_stats: backend.reseek_stats.clone(),
        reseek_sensitivity: backend.reseek_sensitivity.clone(),
    };
    Ok(Aligner::new(&options)?)
}

/// `--max_seqs` as a count; a negative cap is refused.
pub(crate) fn max_seqs(search: &StructureAlignmentArgs) -> Result<Option<usize>> {
    search
        .max_seqs
        .map(|n| usize::try_from(n).map_err(|_| anyhow::anyhow!("--max_seqs must be >= 0")))
        .transpose()
}

/// Validate the backend-capability options before any database is built, so an
/// unsupported request fails before the expensive step.
pub(crate) fn check_backend(
    aligner: &Aligner,
    search: &StructureAlignmentArgs,
    metrics: &[String],
) -> Result<()> {
    max_seqs(search)?;
    aligner.check_capabilities(search.alignment_type, metrics, &[])?;
    Ok(())
}
