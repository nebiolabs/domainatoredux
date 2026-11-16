use anyhow::Result;
use domainator_config::args::HmmerSearchArgs;
use domainator_core::log;
use domainator_core::search::hmm_compare::common_file_alphabet;
use domainator_core::search::results::py_g;
use domainator_core::tools::hmmer_search::{default_min_score, HmmerSearchOptions, HmmerSearcher};

use super::{cpu_threads, for_each_hmm_input, read_hmm_inputs, usage_error, write_hmm_out};

pub fn run(args: &HmmerSearchArgs) -> Result<()> {
    let max_hits = match args.max_hits {
        Some(n) if n < 1 => {
            return Err(usage_error(
                "--max_hits must be at least 1. Leave it out to return all hits.",
            ))
        }
        n => n.map(|n| usize::try_from(n).unwrap_or(usize::MAX)),
    };
    let path = args
        .hmm_out
        .as_deref()
        .expect("hmm_out is a required output");
    let opts = HmmerSearchOptions {
        min_score: args.min_score.map(|s| s.0),
        max_hits,
        cpu: cpu_threads(args.cpu)?,
        max_output_bytes: crate::max_output_gb_to_bytes(args.max_output_gb.0)?,
        output_description: format!("hmmer_search HMM output '{path}'"),
    };
    let references = read_hmm_inputs(&args.reference)?;
    if opts.min_score.is_none() && log::enabled(log::Level::Info) {
        let alphabet =
            common_file_alphabet(&references, "reference").map_err(anyhow::Error::msg)?;
        let (min_score, name) = default_min_score(alphabet);
        log::info(format_args!(
            "Using --min_score {} (the default for {name} profiles).",
            py_g(min_score)
        ));
    }
    // The queries are read a batch of profiles at a time.
    write_hmm_out(path, |mut out| {
        let mut searcher = HmmerSearcher::new(&references, &opts)?;
        for_each_hmm_input(&args.input, |file, queries| {
            searcher
                .search(file, &queries, &mut out)
                .map_err(anyhow::Error::msg)
        })
        .map_err(|e| format!("{e:#}"))?;
        searcher.finish(&mut out)
    })?;
    Ok(())
}
