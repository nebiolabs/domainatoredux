use std::path::Path;

use anyhow::{Context, Result};
use domainator_config::args::HmmerBuildArgs;
use domainator_core::tools::hmmer_build::{
    hmmer_build, hmmer_build_from_bytes, write_hmm_out, HmmerBuildOptions,
};

use crate::output::is_stdin;

pub fn run(args: &HmmerBuildArgs) -> Result<()> {
    let opts = HmmerBuildOptions {
        name: args.name.clone(),
        acc: args.acc.clone(),
        desc: args.desc.clone(),
        alphabet: args.alphabet.as_deref().map(str::to_lowercase),
        window_length: args.window_length,
        window_beta: args.window_beta.map(|b| b.0),
    };
    // Standard input has no name, so its format is guessed from the content alone, as
    // upstream's spooled copy's is.
    let hmm = if is_stdin(&args.input) {
        let mut text = Vec::new();
        std::io::Read::read_to_end(&mut std::io::stdin(), &mut text)
            .context("reading standard input")?;
        hmmer_build_from_bytes(&text, None, &opts)
    } else {
        hmmer_build(Path::new(&args.input), &opts)
    };
    let hmm = hmm.map_err(anyhow::Error::msg)?;
    let path = args
        .hmm_out
        .as_deref()
        .expect("hmm_out is a required output");
    super::write_hmm_out(path, |mut out| write_hmm_out(&hmm, &mut out))
}
