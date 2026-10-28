use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context, Result};
use domainator_config::args::InfernalBuildArgs;
use domainator_core::tools::infernal_build::{infernal_build, InfernalBuildOptions};

use crate::output::{is_stdin, Output};

pub fn run(args: &InfernalBuildArgs) -> Result<()> {
    let opts = InfernalBuildOptions {
        name: args.name.clone(),
        acc: args.acc.clone(),
        desc: args.desc.clone(),
        input_format: args.input_format.clone(),
        cmbuild_path: args.cmbuild_path.clone(),
        noss: args.noss,
        hand: args.hand,
        symfrac: args.symfrac.map(|s| s.0),
    };
    // cmbuild takes a path, so standard input goes through a temporary file.
    let (input, temporary) = if is_stdin(&args.input) {
        let mut text = Vec::new();
        std::io::Read::read_to_end(&mut std::io::stdin(), &mut text)
            .context("reading standard input")?;
        let tmp =
            std::env::temp_dir().join(format!("domainatoredux_stdin_{}.msa", std::process::id()));
        std::fs::write(&tmp, &text)?;
        (tmp, true)
    } else {
        (PathBuf::from(&args.input), false)
    };
    let cm = infernal_build(&input, &opts);
    if temporary {
        let _ = std::fs::remove_file(&input);
    }
    let cm = cm?;
    let path = args.cm_out.as_deref().expect("cm_out is a required output");
    let mut out = Output::create(path)?;
    out.write_all(cm.as_bytes())
        .with_context(|| format!("writing {path}"))?;
    out.finish()
}
