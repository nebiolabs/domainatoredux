//! `infernal_build`: build a covariance model from a multiple sequence alignment with
//! Infernal's `cmbuild`, run as a subprocess (upstream `infernal_build.py`), then
//! set the model's NAME, ACC and DESC in its header.
//!
//! C `cmbuild` and infernox's `infernox-cmbuild` (a byte-parity Rust port, which
//! also builds on Windows) take the same arguments, so either works; the port reads
//! only Stockholm, Pfam and SELEX alignments, not aligned FASTA.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::external::{spawn_error, TempDir};

/// The executable's name.
pub const CMBUILD_BIN_NAME: &str = "cmbuild";
/// The environment variable naming it.
pub const CMBUILD_ENV_VAR: &str = "DOMAINATOREDUX_CMBUILD";

const INSTALL_HINT: &str = "Install Infernal (conda install -c bioconda infernal), or \
     `cargo install infernox` and set $DOMAINATOREDUX_CMBUILD to its infernox-cmbuild.";

/// Options for [`infernal_build`], mirroring upstream's keyword arguments.
#[derive(Clone, Debug, Default)]
pub struct InfernalBuildOptions {
    pub name: String,
    pub acc: Option<String>,
    pub desc: Option<String>,
    /// cmbuild's `--informat`.
    pub input_format: Option<String>,
    /// The executable to run instead of the located one.
    pub cmbuild_path: Option<String>,
    pub noss: bool,
    pub hand: bool,
    pub symfrac: Option<f64>,
}

/// Upstream `sanitize_string`: every character but word characters, space, `-`, `.`,
/// `;` and `:` becomes `_` (Python's `\w` is Unicode-aware, as `char::is_alphanumeric`
/// plus `_` is).
pub fn sanitize_string(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '_' | ' ' | '-' | '.' | ';' | ':') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Upstream `_count_summary_models`: the non-blank, non-comment lines of cmbuild's
/// `-o` summary, one per model built.
fn count_summary_models(summary: &str) -> usize {
    summary
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .count()
}

/// Upstream `_set_header_field`: replace the `key` line, or insert one after the
/// nearest earlier field of NAME, ACC, DESC that is present, else before `STATES`.
fn set_header_field(header: &mut Vec<String>, key: &str, value: Option<&str>) {
    let Some(value) = value else {
        return;
    };
    let prefix = format!("{key:<9}");
    let formatted = format!("{prefix}{}\n", sanitize_string(value));
    if let Some(i) = header.iter().position(|l| l.starts_with(&prefix)) {
        header[i] = formatted;
        return;
    }
    const ORDER: [&str; 3] = ["NAME", "ACC", "DESC"];
    let position = ORDER.iter().position(|k| *k == key).unwrap_or(0);
    for previous in ORDER[..position].iter().rev() {
        let previous = format!("{previous:<9}");
        if let Some(i) = header.iter().position(|l| l.starts_with(&previous)) {
            header.insert(i + 1, formatted);
            return;
        }
    }
    let at = header
        .iter()
        .position(|l| l.starts_with("STATES"))
        .unwrap_or(header.len());
    header.insert(at, formatted);
}

/// Upstream `_patch_cm_header`: set NAME, ACC and DESC in the header that precedes
/// the `CM` line; the body is left as cmbuild wrote it.
pub fn patch_cm_header(
    cm_text: &str,
    name: Option<&str>,
    acc: Option<&str>,
    desc: Option<&str>,
) -> Result<String, String> {
    let mut header: Vec<String> = Vec::new();
    let mut body = String::new();
    let mut in_body = false;
    for line in cm_text.split_inclusive('\n') {
        if !in_body && line.trim() == "CM" {
            in_body = true;
        }
        if in_body {
            body.push_str(line);
        } else {
            header.push(line.to_string());
        }
    }
    if body.is_empty() {
        return Err("Invalid CM output: could not find CM body".into());
    }
    set_header_field(&mut header, "NAME", name);
    set_header_field(&mut header, "ACC", acc);
    set_header_field(&mut header, "DESC", desc);
    Ok(header.concat() + &body)
}

/// Build a covariance model from the alignment at `input` and return it as text
/// (upstream `infernal_build`). Exactly one alignment is accepted.
pub fn infernal_build(input: &Path, opts: &InfernalBuildOptions) -> io::Result<String> {
    let bin = match &opts.cmbuild_path {
        Some(p) => PathBuf::from(p),
        None => super::external::CMBUILD.find(),
    };
    let tmp = TempDir::new("domainatoredux_cmbuild")?;
    let raw_output = tmp.path().join("output.cm");
    let summary = tmp.path().join("summary.txt");
    let mut cmd = Command::new(&bin);
    super::external::hide_console(&mut cmd);
    cmd.arg("-F")
        .arg("-o")
        .arg(&summary)
        .arg("-n")
        .arg(sanitize_string(&opts.name));
    if let Some(format) = &opts.input_format {
        cmd.arg("--informat").arg(format);
    }
    if opts.noss {
        cmd.arg("--noss");
    }
    if opts.hand {
        cmd.arg("--hand");
    }
    if let Some(symfrac) = opts.symfrac {
        cmd.arg("--symfrac")
            .arg(crate::matrix::text::py_float_str(symfrac));
    }
    cmd.arg(&raw_output).arg(input);
    let out = cmd
        .output()
        .map_err(|e| spawn_error(e, &bin, INSTALL_HINT))?;
    if !out.status.success() {
        // Upstream `run_command`: the end of stderr, or of stdout when stderr is empty.
        let stderr = String::from_utf8_lossy(&out.stderr);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let output = if stderr.trim().is_empty() {
            stdout.trim()
        } else {
            stderr.trim()
        };
        let lines: Vec<&str> = output.split('\n').collect();
        let tail = lines[lines.len().saturating_sub(25)..].join("\n");
        return Err(io::Error::other(format!(
            "cmbuild on {} failed (exit {}):\n{tail}",
            input.display(),
            out.status.code().unwrap_or(-1)
        )));
    }
    if count_summary_models(&std::fs::read_to_string(&summary)?) != 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "domainator infernal_build currently supports exactly one alignment per invocation",
        ));
    }
    let cm_text = std::fs::read_to_string(&raw_output)?;
    patch_cm_header(
        &cm_text,
        Some(&opts.name),
        opts.acc.as_deref(),
        opts.desc.as_deref(),
    )
    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CM: &str = "INFERNAL1/a [1.1.5 | Sep 2023]\nNAME     raw\nSTATES   9\nNODES    3\n\
                      CM\n  [ ROOT    0 ]\n//\nHMMER3/f\nNAME  raw\n//\n";

    #[test]
    fn sanitize_keeps_word_characters_and_a_few_separators() {
        assert_eq!(sanitize_string("a b-c.d;e:f_g"), "a b-c.d;e:f_g");
        assert_eq!(sanitize_string("a/b(c)\td"), "a_b_c__d");
        assert_eq!(sanitize_string("é1"), "é1");
    }

    #[test]
    fn summary_counts_one_line_per_model() {
        let summary = "# cmbuild\n#\n# idx name\n#--- ----\n   1 test_cm  5 92\n\n";
        assert_eq!(count_summary_models(summary), 1);
        assert_eq!(count_summary_models("# only comments\n"), 0);
        assert_eq!(count_summary_models("1 a\n2 b\n"), 2);
    }

    #[test]
    fn header_fields_replace_or_insert_in_name_acc_desc_order() {
        let patched = patch_cm_header(CM, Some("new name"), Some("RF1"), Some("a/desc")).unwrap();
        let header: Vec<&str> = patched.lines().take(6).collect();
        assert_eq!(
            header,
            [
                "INFERNAL1/a [1.1.5 | Sep 2023]",
                "NAME     new name",
                "ACC      RF1",
                "DESC     a_desc",
                "STATES   9",
                "NODES    3",
            ]
        );
        // The body, and the filter HMM's own NAME, are untouched.
        assert!(patched.ends_with("CM\n  [ ROOT    0 ]\n//\nHMMER3/f\nNAME  raw\n//\n"));
    }

    #[test]
    fn a_field_without_an_earlier_neighbor_goes_before_states() {
        let no_name = CM.replace("NAME     raw\n", "");
        let patched = patch_cm_header(&no_name, None, None, Some("d")).unwrap();
        assert!(patched.starts_with("INFERNAL1/a [1.1.5 | Sep 2023]\nDESC     d\nSTATES"));
        let unchanged = patch_cm_header(CM, None, None, None).unwrap();
        assert_eq!(unchanged, CM);
    }

    #[test]
    fn a_cm_without_a_body_is_an_error() {
        assert!(patch_cm_header("INFERNAL1/a\nNAME x\n", Some("y"), None, None).is_err());
    }
}
