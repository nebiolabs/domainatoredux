use std::io::Write;

use anyhow::{anyhow, Result};
use domainator_config::args::FormatDbArgs;
use domainator_core::parallel::with_threads;
use domainator_core::tools::format_db::{format_db, FormatDbOptions};

use super::cpu_threads;

pub fn run(args: &FormatDbArgs) -> Result<()> {
    let opts = FormatDbOptions {
        // Upstream refuses a count below 1 with the same message the core gives.
        shards: args
            .shards
            .map(|n| usize::try_from(n).map_err(|_| anyhow!("--shards must be >= 1")))
            .transpose()?,
        compress: args.compress,
        index: args.index,
        output_dir: args.output_dir.clone(),
        name: args.name.clone(),
        level: u8::try_from(args.level)
            .map_err(|_| anyhow!("bad compression level {}", args.level))?,
        force: args.force,
    };
    let cpu = cpu_threads(args.cpu)?;
    let result = match &args.log_out {
        Some(path) => {
            let mut log = std::io::BufWriter::new(std::fs::File::create(path)?);
            let result = with_threads(cpu, || format_db(&args.input, &opts, &mut log));
            log.flush()?;
            result
        }
        None => with_threads(cpu, || {
            format_db(&args.input, &opts, &mut std::io::stderr())
        }),
    };
    result.map(|_| ()).map_err(anyhow::Error::msg)
}
