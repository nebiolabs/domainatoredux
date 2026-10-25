//! The bodies of the migrated tools, one module each: `run(&XArgs)` does the I/O
//! wiring around the core library function.

pub mod color_genbank;
pub mod enum_report;
pub mod extract_domains;
pub mod extract_peptides;
pub mod extract_unannotated;
pub mod filter_domains;
pub mod hmmer_report;
pub mod select_by_cds;
pub mod select_by_contig;
pub mod select_by_coord;
pub mod sort_contigs;
pub mod trim_contigs;

use anyhow::{Context, Result};
use domainator_core::io::genbank::read_genbank;
use domainator_core::seq::SeqRecord;

use crate::output::is_stdin;

/// A usage error: reported like clap's own (`error: ...`, exit 2), for a check that
/// clap cannot express, as upstream's `parser.error` is.
pub fn usage_error(message: impl std::fmt::Display) -> anyhow::Error {
    clap::Error::raw(
        clap::error::ErrorKind::ValueValidation,
        format!("{message}\n"),
    )
    .into()
}

/// Read every `-i` input in order; `-` or `/dev/stdin` is GenBank on standard input.
pub fn load_seq_inputs(inputs: &[String]) -> Result<Vec<SeqRecord>> {
    let mut out = Vec::new();
    for path in inputs {
        if is_stdin(path) {
            out.extend(read_stdin_genbank()?);
        } else {
            out.extend(crate::load(std::path::Path::new(path))?);
        }
    }
    Ok(out)
}

pub(crate) fn read_stdin_genbank() -> Result<Vec<SeqRecord>> {
    use std::io::Read;
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .context("reading standard input")?;
    let tmp = std::env::temp_dir().join(format!("domainatoredux_stdin_{}.gb", std::process::id()));
    std::fs::write(&tmp, &buf)?;
    let records =
        read_genbank(&tmp.to_string_lossy()).context("parsing GenBank from standard input");
    let _ = std::fs::remove_file(&tmp);
    records
}
