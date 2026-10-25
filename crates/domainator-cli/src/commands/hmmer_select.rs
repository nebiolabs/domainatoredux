use anyhow::{anyhow, Result};
use domainator_config::args::HmmerSelectArgs;
use domainator_core::log;
use domainator_core::tools::hmmer_select::{hmmer_select, parse_alphabet, Fields, SelectOptions};

use super::{read_hmm_inputs, write_hmm_out};

pub fn run(args: &HmmerSelectArgs) -> Result<()> {
    let opts = SelectOptions {
        regex: args.regex.clone().unwrap_or_default(),
        exact: args.exact.clone().unwrap_or_default(),
        contains: args.contains.clone().unwrap_or_default(),
        fields: Fields::from_field(&args.field.to_lowercase()).map_err(|e| anyhow!(e))?,
        case_sensitive: args.case_sensitive,
        alphabet: parse_alphabet(args.alphabet.as_deref().map(str::to_lowercase).as_deref())
            .map_err(|e| anyhow!(e))?,
    };
    let inputs = read_hmm_inputs(&args.input)?;
    let path = args
        .hmm_out
        .as_deref()
        .expect("hmm_out is a required output");
    let summary = write_hmm_out(path, |mut out| hmmer_select(&inputs, &opts, &mut out))?;
    for line in &summary.skipped {
        log::info(line);
    }
    log::info(format_args!("Found {} matching profiles.", summary.found));
    if summary.skipped_files > 0 {
        log::info(format_args!(
            "Skipped {} input file(s) whose alphabet did not match --alphabet.",
            summary.skipped_files
        ));
    }
    Ok(())
}
