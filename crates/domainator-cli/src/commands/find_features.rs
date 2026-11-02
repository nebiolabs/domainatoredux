use anyhow::{anyhow, Context, Result};
use domainator_config::args::FindFeaturesArgs;
use domainator_core::plm::tmbed::Tmbed;
use domainator_core::tools::find_features::{find_features, FindFeaturesOptions, TmbedRun};
use domainator_core::tools::tmbed_model;

use super::{cpu_threads, load_typed_inputs};
use crate::output::write_seq_outputs;

pub fn run(args: &FindFeaturesArgs) -> Result<()> {
    let cpu = cpu_threads(args.cpu)?;
    let tmbed = if args.tmbed {
        let dir = tmbed_model::resolve_model_dir(
            args.model_dir.as_deref(),
            args.tmbed_model_dir.as_deref(),
        )?;
        tmbed_model::ensure_model(&dir)
            .with_context(|| format!("getting TMbed's ProtT5 model into {}", dir.display()))?;
        let (device, warning) = domainator_core::plm::select_device(Some(&args.device));
        if let Some(warning) = warning {
            domainator_core::log::warn(warning);
        }
        let model = Tmbed::load(&dir, &device)
            .map_err(|e| anyhow!("loading TMbed's ProtT5 model from {}: {e}", dir.display()))?;
        Some(TmbedRun {
            model,
            batch_size: args.batch_size,
        })
    } else {
        None
    };
    let opts = FindFeaturesOptions {
        motifs: args.motif.clone().unwrap_or_default(),
        partial: args.partial.parse().map_err(|e: String| anyhow!(e))?,
        gene_call: crate::gene_call_request(args.gene_call.as_deref(), cpu)?,
        tmbed,
    };
    let records = load_typed_inputs(&args.input, &args.alphabet)?;
    // The thread pool bounds TMbed's matrix multiplications as well.
    let annotated = domainator_core::parallel::with_threads(cpu, || find_features(records, &opts))
        .context("running find_features")?;
    write_seq_outputs(
        &annotated,
        args.genbank_out.as_deref(),
        args.fasta_out.as_deref(),
    )
}
