use std::io::Write;

use anyhow::Result;
use domainator_config::args::HmmerCompareArgs;
use domainator_core::tools::hmmer_compare::{hmmer_compare, HmmerCompareOptions};

use super::{cpu_threads, read_hmm_inputs};
use crate::output::Output;

pub fn run(args: &HmmerCompareArgs) -> Result<()> {
    let opts = HmmerCompareOptions {
        min_score: args.min_score.0,
        // Upstream's top-k heap keeps nothing for a cap below 1.
        max_hits_per_query: args
            .max_hits_per_query
            .map(|k| usize::try_from(k).unwrap_or(0)),
        cpu: cpu_threads(args.cpu)?,
        max_output_bytes: crate::max_output_gb_to_bytes(args.max_output_gb.0)?,
    };
    let references: Vec<_> = read_hmm_inputs(&args.reference)?
        .into_iter()
        .flat_map(|(_, profiles)| profiles)
        .collect();
    let queries = read_hmm_inputs(&args.input)?;

    let mut tsv = args.tsv_out.as_deref().map(Output::create).transpose()?;
    let mut alignments = args
        .alignments_out
        .as_deref()
        .map(Output::create)
        .transpose()?;
    hmmer_compare(
        &queries,
        &references,
        &opts,
        tsv.as_mut().map(|o| o as &mut dyn Write),
        alignments.as_mut().map(|o| o as &mut dyn Write),
    )?;
    for out in [tsv, alignments].into_iter().flatten() {
        out.finish()?;
    }
    Ok(())
}
