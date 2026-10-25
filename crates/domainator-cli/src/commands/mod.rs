//! The bodies of the migrated tools, one module each: `run(&XArgs)` does the I/O
//! wiring around the core library function.

pub mod clean_sequences;
pub mod color_genbank;
pub mod deduplicate_genbank;
pub mod domain_search;
pub mod domainate;
pub mod enum_report;
pub mod extract_domains;
pub mod extract_peptides;
pub mod extract_unannotated;
pub mod filter_domains;
pub mod genbank_to_fasta;
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

/// A `--cpu` value as a thread count for the core (`0`: all cores), refusing a
/// negative one as upstream's `resolve_cpu` does.
pub fn cpu_threads(cpu: i64) -> Result<usize> {
    usize::try_from(cpu).map_err(|_| {
        anyhow::anyhow!("--cpu must be 0 (all cores) or a positive number of threads, not {cpu}.")
    })
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

/// Like [`load_seq_inputs`], with FASTA records given the molecule type `alphabet`
/// (`protein` or `nucleotide`; upstream's `default_molecule_type`).
pub fn load_typed_inputs(inputs: &[String], alphabet: &str) -> Result<Vec<SeqRecord>> {
    let mut out = Vec::new();
    for path in inputs {
        if is_stdin(path) {
            out.extend(read_stdin_genbank()?);
        } else {
            out.extend(crate::load_typed(std::path::Path::new(path), alphabet)?);
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
