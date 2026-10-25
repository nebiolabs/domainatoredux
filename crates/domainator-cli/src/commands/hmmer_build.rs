use std::path::PathBuf;

use anyhow::{Context, Result};
use domainator_config::args::HmmerBuildArgs;
use domainator_core::tools::hmmer_build::{hmmer_build, write_hmm_out, HmmerBuildOptions};

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
    // The MSA reader takes a path, so standard input goes through a temporary file.
    let (input, temporary) = if is_stdin(&args.input) {
        let mut text = Vec::new();
        std::io::Read::read_to_end(&mut std::io::stdin(), &mut text)
            .context("reading standard input")?;
        let tmp =
            std::env::temp_dir().join(format!("domainatoredux_stdin_{}.sto", std::process::id()));
        std::fs::write(&tmp, &text)?;
        (tmp, true)
    } else {
        (PathBuf::from(&args.input), false)
    };
    let hmm = hmmer_build(&input, &opts);
    if temporary {
        let _ = std::fs::remove_file(&input);
    }
    let hmm = hmm.map_err(anyhow::Error::msg)?;
    let path = args
        .hmm_out
        .as_deref()
        .expect("hmm_out is a required output");
    super::write_hmm_out(path, |mut out| write_hmm_out(&hmm, &mut out))
}
