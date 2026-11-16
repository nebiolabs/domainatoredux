use std::io::Write;

use anyhow::Result;
use domainator_config::args::HmmerCompareArgs;
use domainator_core::tools::hmmer_compare::{HmmerCompareOptions, HmmerComparer};

use super::{cpu_threads, for_each_hmm_input, read_hmm_inputs};
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
    // Every reference is compared with every query, so the references are read whole;
    // the queries are read a batch at a time and each batch's hits written before the
    // next is read.
    let references = read_hmm_inputs(&args.reference)?;

    let mut tsv = args.tsv_out.as_deref().map(Output::create).transpose()?;
    let mut alignments = args
        .alignments_out
        .as_deref()
        .map(Output::create)
        .transpose()?;
    let mut comparer = HmmerComparer::new(
        &references,
        &opts,
        tsv.as_mut().map(|o| o as &mut dyn Write),
        alignments.is_some(),
    )?;
    for_each_hmm_input(&args.input, |file, queries| {
        comparer.compare(
            file,
            &queries,
            tsv.as_mut().map(|o| o as &mut dyn Write),
            alignments.as_mut().map(|o| o as &mut dyn Write),
        )?;
        Ok(())
    })?;
    for out in [tsv, alignments].into_iter().flatten() {
        out.finish()?;
    }
    Ok(())
}
