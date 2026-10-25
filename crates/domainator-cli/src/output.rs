//! Output files, the way upstream's `outputs.output_file` writes them.
//!
//! `-` and `/dev/stdout` are standard output. A file is written under a temporary name
//! in its directory and renamed into place only when the run succeeds, so a failed run
//! leaves nothing that looks like a result. A path ending in `.gz` or `.bgz` is
//! compressed as BGZF, which gzip reads.

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use domainator_core::io::fasta;
use domainator_core::io::genbank::writer::write_record;
use domainator_core::matrix::estimate::estimate_write_size;
use domainator_core::matrix::{hdf5_io, text, DataMatrix};
use domainator_core::seq::SeqRecord;
use domainator_core::tools::output_guardrails::output_limit_message;

/// Paths that mean standard output.
pub const STDOUT_PATHS: [&str; 2] = ["-", "/dev/stdout"];
/// Paths that mean standard input.
pub const STDIN_PATHS: [&str; 2] = ["-", "/dev/stdin"];

pub fn is_stdout(path: &str) -> bool {
    STDOUT_PATHS.contains(&path)
}

pub fn is_stdin(path: &str) -> bool {
    STDIN_PATHS.contains(&path)
}

enum Sink {
    Stdout(BufWriter<io::Stdout>),
    File(BufWriter<File>),
    Bgzf(bgzf::Writer<BufWriter<File>>),
}

/// One output, open for writing. Call [`Output::finish`] when the run succeeded;
/// dropping it unfinished removes the temporary file.
pub struct Output {
    sink: Option<Sink>,
    temp: Option<PathBuf>,
    target: PathBuf,
}

fn temp_path(target: &Path) -> PathBuf {
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    target.with_file_name(format!(".{name}.{}.tmp", std::process::id()))
}

impl Output {
    pub fn create(path: &str) -> Result<Output> {
        if is_stdout(path) {
            return Ok(Output {
                sink: Some(Sink::Stdout(BufWriter::new(io::stdout()))),
                temp: None,
                target: PathBuf::from(path),
            });
        }
        let target = PathBuf::from(path);
        let temp = temp_path(&target);
        let file =
            BufWriter::new(File::create(&temp).with_context(|| format!("creating output {path}"))?);
        let compressed = path.ends_with(".gz") || path.ends_with(".bgz");
        let sink = if compressed {
            let level = bgzf::CompressionLevel::new(6)
                .map_err(|e| anyhow::anyhow!("BGZF compression level: {e:?}"))?;
            Sink::Bgzf(bgzf::Writer::new(file, level))
        } else {
            Sink::File(file)
        };
        Ok(Output {
            sink: Some(sink),
            temp: Some(temp),
            target,
        })
    }

    /// Flush, and move a file into place.
    pub fn finish(mut self) -> Result<()> {
        let what = || format!("writing {}", self.target.display());
        match self.sink.take() {
            Some(Sink::Stdout(mut w)) => w.flush().with_context(what)?,
            Some(Sink::File(mut w)) => w.flush().with_context(what)?,
            Some(Sink::Bgzf(w)) => w.finish().with_context(what)?.flush().with_context(what)?,
            None => {}
        }
        if let Some(temp) = self.temp.take() {
            std::fs::rename(&temp, &self.target).with_context(what)?;
        }
        Ok(())
    }
}

impl Write for Output {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.sink.as_mut() {
            Some(Sink::Stdout(w)) => w.write(buf),
            Some(Sink::File(w)) => w.write(buf),
            Some(Sink::Bgzf(w)) => w.write(buf),
            None => Err(io::Error::other("output already finished")),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.sink.as_mut() {
            Some(Sink::Stdout(w)) => w.flush(),
            Some(Sink::File(w)) => w.flush(),
            Some(Sink::Bgzf(w)) => w.flush(),
            None => Ok(()),
        }
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        if let Some(temp) = self.temp.take() {
            drop(self.sink.take());
            let _ = std::fs::remove_file(temp);
        }
    }
}

/// Write records to a GenBank output, a FASTA output, or both (upstream
/// `write_seq_outputs`). Nothing appears at either path unless both succeed.
pub fn write_seq_outputs(
    records: &[SeqRecord],
    genbank: Option<&str>,
    fasta_out: Option<&str>,
) -> Result<()> {
    write_seq_outputs_capped(records, genbank, fasta_out, None)
}

/// A `--max_output_gb` budget for [`write_seq_outputs_capped`]: the byte limit, and
/// the options and any further advice the error gives.
pub struct OutputCap<'a> {
    pub max_bytes: u64,
    pub mitigation: &'a [&'a str],
    pub extra_guidance: Option<&'a str>,
}

/// [`write_seq_outputs`], stopping with upstream's error before a record that would
/// take the two outputs together past `cap` (upstream `SeqOutputs.write`).
pub fn write_seq_outputs_capped(
    records: &[SeqRecord],
    genbank: Option<&str>,
    fasta_out: Option<&str>,
    cap: Option<OutputCap<'_>>,
) -> Result<()> {
    let mut gb = genbank.map(Output::create).transpose()?;
    let mut fa = fasta_out.map(Output::create).transpose()?;
    let description = || {
        [("GenBank", genbank), ("FASTA", fasta_out)]
            .iter()
            .filter_map(|(kind, path)| path.map(|p| format!("{kind} output '{p}'")))
            .collect::<Vec<_>>()
            .join(" and ")
    };
    let mut written: u64 = 0;
    let (mut gb_buf, mut fa_buf) = (Vec::new(), Vec::new());
    for rec in records {
        gb_buf.clear();
        fa_buf.clear();
        if gb.is_some() {
            write_record(&mut gb_buf, rec).context("writing GenBank")?;
        }
        if fa.is_some() {
            fasta::write_fasta_bp(&mut fa_buf, std::slice::from_ref(rec))
                .context("writing FASTA")?;
        }
        written += (gb_buf.len() + fa_buf.len()) as u64;
        if let Some(cap) = &cap {
            if written > cap.max_bytes {
                anyhow::bail!(output_limit_message(
                    &description(),
                    written,
                    cap.max_bytes,
                    cap.mitigation,
                    cap.extra_guidance,
                ));
            }
        }
        if let Some(w) = gb.as_mut() {
            w.write_all(&gb_buf).context("writing GenBank")?;
        }
        if let Some(w) = fa.as_mut() {
            w.write_all(&fa_buf).context("writing FASTA")?;
        }
    }
    gb.map(Output::finish).transpose()?;
    fa.map(Output::finish).transpose()?;
    Ok(())
}

/// `--hdf_out` must name an hdf5 file (upstream `resolve_matrix_outputs`).
pub fn check_hdf_out(hdf: Option<&str>) -> Result<()> {
    let is_hdf5 = hdf.is_none_or(|p| {
        Path::new(p)
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| ["hdf5", "hdf", "h5"].contains(&e.to_ascii_lowercase().as_str()))
    });
    anyhow::ensure!(
        is_hdf5,
        "Please use an hdf5 related extension for the --hdf_out output, such as .h5, .hdf5, or \
         .hdf."
    );
    Ok(())
}

/// Write a matrix to `--hdf_out` and/or `--dense_text_out` (upstream
/// `write_matrix_outputs`), each checked against `max_bytes` (`--max_output_gb`)
/// before it is written. The hdf5 file is renamed into place once complete.
pub fn write_matrix_outputs(
    matrix: &DataMatrix,
    hdf: Option<&str>,
    dense_text: Option<&str>,
    max_bytes: Option<u64>,
    mitigation: &[&str],
) -> Result<()> {
    let check = |output_type: &str, estimate: &str, path: &str, mitigation: &[&str]| {
        let projected = estimate_write_size(estimate, matrix);
        match max_bytes {
            Some(max) if projected > max => Err(anyhow::anyhow!(output_limit_message(
                &format!("{output_type} matrix output '{path}'"),
                projected,
                max,
                mitigation,
                None,
            ))),
            _ => Ok(()),
        }
    };
    if let Some(path) = hdf {
        check("hdf", "sparse", path, mitigation)?;
        let target = PathBuf::from(path);
        let temp = temp_path(&target);
        let written = hdf5_io::write(&temp.to_string_lossy(), matrix)
            .and_then(|()| std::fs::rename(&temp, &target));
        if written.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        written.with_context(|| format!("writing {path}"))?;
    }
    if let Some(path) = dense_text {
        let mitigation: Vec<&str> = std::iter::once("--hdf_out")
            .chain(mitigation.iter().copied())
            .collect();
        check("dense_text", "dense_text", path, &mitigation)?;
        let mut out = Output::create(path)?;
        text::write_dense_text_to(&mut out, matrix).with_context(|| format!("writing {path}"))?;
        out.finish()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_appears_only_when_finished() {
        let dir =
            std::env::temp_dir().join(format!("domainatoredux-output-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.txt");
        let _ = std::fs::remove_file(&path);
        {
            let mut out = Output::create(path.to_str().unwrap()).unwrap();
            out.write_all(b"partial").unwrap();
        }
        assert!(
            !path.exists(),
            "an unfinished output must not be left behind"
        );
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().count(),
            0,
            "temp file left"
        );
        let mut out = Output::create(path.to_str().unwrap()).unwrap();
        out.write_all(b"done").unwrap();
        out.finish().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"done");
    }

    #[test]
    fn a_gz_output_is_bgzf() {
        let dir =
            std::env::temp_dir().join(format!("domainatoredux-output-gz-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.gb.gz");
        let mut out = Output::create(path.to_str().unwrap()).unwrap();
        out.write_all(b"LOCUS\n").unwrap();
        out.finish().unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..2], &[0x1f, 0x8b]);
        let mut text = String::new();
        io::Read::read_to_string(
            &mut flate2::read::MultiGzDecoder::new(&bytes[..]),
            &mut text,
        )
        .unwrap();
        assert_eq!(text, "LOCUS\n");
    }
}
