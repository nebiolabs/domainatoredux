//! The bodies of the tools, one module each: `run(&XArgs)` does the I/O
//! wiring around the core library function.

pub mod build_projection;
pub mod build_ssn;
pub mod build_tree;
pub mod clean_sequences;
pub mod color_genbank;
pub mod color_table_to_legend;
pub mod compare_contigs;
pub mod db_download;
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
pub mod kmer_dist;
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
pub mod structure_dist;
pub mod structure_domainate;
pub mod structure_search;
pub mod structure_to_genbank;
pub mod summary_report;
pub mod transform_matrix;
pub mod trim_contigs;
pub mod viewer;

use std::io::Write;

use anyhow::{Context, Result};
use domainator_core::search::hmm_read::{for_each_hmm_batch, read_hmm_file, Hmm, HmmerError};
use domainator_core::search::hmm_write::write_hmm_output;
use domainator_core::seq::SeqRecord;

use crate::input::{load_all, InputMode};
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

/// Every record of `inputs` (`-` is standard input, read as GenBank), FASTA read as
/// protein. Only for a tool that needs them all at once; one that handles records
/// one at a time reads [`crate::input::SeqInputs`].
pub fn load_seq_inputs(inputs: &[String]) -> Result<Vec<SeqRecord>> {
    load_all(inputs, InputMode::DETECT)
}

/// Read HMM profile files, each with the name it is reported under; `-` reads standard
/// input (through a temporary file, as the reader takes a path).
pub fn read_hmm_inputs(paths: &[String]) -> Result<Vec<(String, Vec<Hmm>)>> {
    let mut out = Vec::new();
    for path in paths {
        let profiles = with_hmm_path(path, read_hmm_file)?.map_err(|e| hmm_read_error(path, &e))?;
        out.push((path.clone(), profiles));
    }
    Ok(out)
}

/// [`read_hmm_inputs`] a batch of profiles at a time, file after file, so a library is
/// never read whole: `f` gets each batch with the name of its file. A file with no
/// profiles gets no call.
pub fn for_each_hmm_input(
    paths: &[String],
    mut f: impl FnMut(&str, Vec<Hmm>) -> Result<()>,
) -> Result<()> {
    /// A read error, kept apart from `f`'s so it can be reported with its file.
    enum BatchError {
        Read(HmmerError),
        Caller(anyhow::Error),
    }
    impl From<HmmerError> for BatchError {
        fn from(e: HmmerError) -> Self {
            BatchError::Read(e)
        }
    }
    for path in paths {
        let read = with_hmm_path(path, |p| {
            for_each_hmm_batch(p, |batch| f(path, batch).map_err(BatchError::Caller))
        })?;
        match read {
            Ok(()) => {}
            Err(BatchError::Read(e)) => return Err(hmm_read_error(path, &e)),
            Err(BatchError::Caller(e)) => return Err(e),
        }
    }
    Ok(())
}

fn hmm_read_error(path: &str, e: &HmmerError) -> anyhow::Error {
    anyhow::anyhow!(
        "reading HMM profiles: {}",
        domainator_core::search::hmm_read::error_message(std::path::Path::new(path), e)
    )
}

/// Run `read` on the file `path` names; standard input is copied to a temporary file
/// first, since the HMM reader may need to read a file twice.
fn with_hmm_path<T>(path: &str, read: impl FnOnce(&std::path::Path) -> T) -> Result<T> {
    if !is_stdin(path) {
        return Ok(read(std::path::Path::new(path)));
    }
    struct Remove(std::path::PathBuf);
    impl Drop for Remove {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let tmp = Remove(
        std::env::temp_dir().join(format!("domainatoredux_stdin_{}.hmm", std::process::id())),
    );
    let mut file = std::fs::File::create(&tmp.0).context("staging standard input")?;
    std::io::copy(&mut std::io::stdin().lock(), &mut file).context("reading standard input")?;
    drop(file);
    Ok(read(&tmp.0))
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
