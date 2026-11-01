use anyhow::{anyhow, Context, Result};
use domainator_config::args::FindFeaturesArgs;
use domainator_core::tools::find_features::{
    find_features, FindFeaturesOptions, TMBED_UNSUPPORTED,
};

use super::{cpu_threads, load_typed_inputs, usage_error};
use crate::output::write_seq_outputs;

pub fn run(args: &FindFeaturesArgs) -> Result<()> {
    if args.tmbed {
        return Err(usage_error(TMBED_UNSUPPORTED));
    }
    let cpu = cpu_threads(args.cpu)?;
    let opts = FindFeaturesOptions {
        motifs: args.motif.clone().unwrap_or_default(),
        partial: args.partial.parse().map_err(|e: String| anyhow!(e))?,
        gene_call: crate::gene_call_request(args.gene_call.as_deref(), cpu)?,
    };
    let records = load_typed_inputs(&args.input, &args.alphabet)?;
    let annotated = find_features(records, &opts).context("running find_features")?;
    write_seq_outputs(
        &annotated,
        args.genbank_out.as_deref(),
        args.fasta_out.as_deref(),
    )
}
