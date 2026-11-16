use anyhow::{anyhow, Result};
use domainator_config::args::HmmerSelectArgs;
use domainator_core::log;
use domainator_core::tools::hmmer_select::{
    parse_alphabet, Fields, ProfileSelector, SelectOptions,
};

use super::{for_each_hmm_input, write_hmm_out};

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
    let path = args
        .hmm_out
        .as_deref()
        .expect("hmm_out is a required output");
    let summary = write_hmm_out(path, |mut out| {
        let mut selector = ProfileSelector::new(&opts)?;
        // Which file is open, and whether its alphabet keeps it.
        let mut current: Option<(String, bool)> = None;
        for_each_hmm_input(&args.input, |file, batch| {
            if current.as_ref().is_none_or(|(name, _)| name != file) {
                let keep = selector
                    .start_file(file, batch[0].abc_type)
                    .map_err(anyhow::Error::msg)?;
                current = Some((file.to_string(), keep));
            }
            if current.as_ref().is_some_and(|(_, keep)| *keep) {
                selector
                    .write_matching(&batch, &mut out)
                    .map_err(anyhow::Error::msg)?;
            }
            Ok(())
        })
        .map_err(|e| format!("{e:#}"))?;
        Ok(selector.finish())
    })?;
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
