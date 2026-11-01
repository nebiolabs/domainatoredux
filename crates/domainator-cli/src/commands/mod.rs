//! The bodies of the tools, one module each: `run(&XArgs)` does the I/O
//! wiring around the core library function.

pub mod build_projection;
pub mod build_ssn;
pub mod build_tree;
pub mod clean_sequences;
pub mod color_genbank;
pub mod color_table_to_legend;
pub mod compare_contigs;
pub mod deduplicate_genbank;
pub mod doctor;
pub mod domain_search;
pub mod domainate;
pub mod enum_report;
pub mod extract_domains;
pub mod extract_peptides;
pub mod extract_unannotated;
pub mod filter_domains;
pub mod find_features;
pub mod format_db;
pub mod genbank_to_fasta;
pub mod gene_call;
pub mod hmmer_build;
pub mod hmmer_compare;
pub mod hmmer_report;
pub mod hmmer_search;
pub mod hmmer_select;
pub mod infernal_build;
pub mod matrix_report;
pub mod plot_contigs;
pub mod rotate_contigs;
pub mod select_by_cds;
pub mod select_by_contig;
pub mod select_by_coord;
pub mod seq_dist;
pub mod sort_contigs;
pub mod split_by_cluster;
pub mod ssn_navigator;
pub mod summary_report;
pub mod transform_matrix;
pub mod trim_contigs;
pub mod viewer;

use std::io::Write;

use anyhow::{Context, Result};
use domainator_core::io::genbank::read_genbank;
use domainator_core::search::hmm_read::{read_hmm_file, Hmm};
use domainator_core::search::hmm_write::write_hmm_output;
use domainator_core::seq::SeqRecord;

use crate::output::{is_stdin, Output};

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

/// Read HMM profile files, each with the name it is reported under; `-` reads standard
/// input (through a temporary file, as the reader takes a path).
pub fn read_hmm_inputs(paths: &[String]) -> Result<Vec<(String, Vec<Hmm>)>> {
    let mut out = Vec::new();
    for path in paths {
        let profiles = if is_stdin(path) {
            use std::io::Read;
            let mut text = Vec::new();
            std::io::stdin()
                .read_to_end(&mut text)
                .context("reading standard input")?;
            let tmp = std::env::temp_dir()
                .join(format!("domainatoredux_stdin_{}.hmm", std::process::id()));
            std::fs::write(&tmp, &text)?;
            let read = read_hmm_file(&tmp);
            let _ = std::fs::remove_file(&tmp);
            read
        } else {
            read_hmm_file(std::path::Path::new(path))
        };
        let profiles =
            profiles.map_err(|e| anyhow::anyhow!("reading HMM profiles from {path}: {e:?}"))?;
        out.push((path.clone(), profiles));
    }
    Ok(out)
}

/// Write HMM profiles to `path` through `write`: `-` is standard output (never
/// compressed), anything else goes through the core's `write_hmm_output` (`.gz` is
/// gzip, `.bgz` refused, a failed run leaves no file).
pub fn write_hmm_out<T>(
    path: &str,
    write: impl FnOnce(&mut dyn Write) -> Result<T, String>,
) -> Result<T> {
    if crate::output::is_stdout(path) {
        let stdout = std::io::stdout();
        let mut handle = stdout.lock();
        let value = write(&mut handle).map_err(anyhow::Error::msg)?;
        Write::flush(&mut handle)?;
        Ok(value)
    } else {
        write_hmm_output(path, |out| write(out)).map_err(anyhow::Error::msg)
    }
}

/// Write a Domainator Viewer bundle to a `.dvb` file at `dvb_out` and as a viewer page
/// that opens it at `html_out`, each optional, failing first if either would exceed
/// `max_bytes` (upstream `write_viewer_outputs`).
pub fn write_viewer_outputs(
    bundle: &serde_json::Value,
    html_out: Option<&str>,
    dvb_out: Option<&str>,
    max_bytes: Option<u64>,
    mitigation: &[&str],
) -> Result<()> {
    use domainator_core::tools::output_guardrails::output_limit_message;
    use domainator_core::tools::viewer_bundle::serialize_bundle;
    use domainator_core::tools::viewer_html::{projected_html_bytes, render_html, VIEWER_APP_NAME};

    let compressed = serialize_bundle(bundle);
    let check = |description: String, bytes: u64| -> Result<()> {
        match max_bytes.filter(|&max| bytes > max) {
            Some(max) => Err(anyhow::anyhow!(output_limit_message(
                &description,
                bytes,
                max,
                mitigation,
                None
            ))),
            None => Ok(()),
        }
    };
    if let Some(path) = dvb_out {
        check(
            format!("Domainator Viewer bundle output '{path}'"),
            compressed.len() as u64,
        )?;
        let mut out = Output::create(path)?;
        out.write_all(&compressed)?;
        out.finish()?;
    }
    if let Some(path) = html_out {
        // The page embeds the bundle's bytes in base64.
        check(
            format!("Domainator Viewer page '{path}'"),
            projected_html_bytes(compressed.len()),
        )?;
        let name = bundle
            .get("name")
            .and_then(serde_json::Value::as_str)
            .filter(|n| !n.is_empty())
            .unwrap_or(VIEWER_APP_NAME);
        let mut out = Output::create(path)?;
        out.write_all(render_html(name, Some(&compressed)).as_bytes())?;
        out.finish()?;
    }
    Ok(())
}
