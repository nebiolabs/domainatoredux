//! What a Tools-menu job reads and writes, worked out from its tool spec and
//! params rather than by each tool: the provenance `IoRef`s, the outputs a
//! canceled job must clean up, and the job message taken from the worker's output.
//!
//! A job runs the CLI in a worker process (`worker.rs`), so nothing in the GUI
//! knows what the tool did beyond its exit status and what it printed. The spec
//! (`tools/<id>.yml`) says which fields are files and of what type, which is
//! enough for the rest.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use domainator_core::app_support::provenance::IoRef;
use domainator_core::io::{detect_filetype, FileType};
use serde_json::Value;

use crate::tool_specs::{FieldSpec, ToolSpec};

/// The provenance role of an input field.
///
/// **Load-bearing**: `trace_source_sequences` reads these roles, and they must be
/// what the per-tool runners that preceded the worker recorded, or SSN node →
/// source-sequence tracing breaks without any error. `cmd_tools`'s
/// `derived_provenance_roles_match_the_golden_table` holds the old values.
pub fn input_role(field: &FieldSpec) -> String {
    // Search references are never a node's sequence source, whatever their format
    // (`hmmer_compare`'s are `.hmm`, `seq_dist`'s may be FASTA).
    if matches!(field.name.as_str(), "reference" | "references") {
        return "reference".into();
    }
    match field.file_type.first().map(String::as_str) {
        Some("sequence") => "sequence",
        Some("genbank") => "genbank",
        Some("hdf5") => "matrix",
        Some("hmm") => "hmm",
        Some("stockholm") => "msa",
        Some("table") => "table",
        Some("text") => "list",
        Some("taxonomy") => "taxonomy",
        Some(other) => other,
        None => "file",
    }
    .into()
}

/// The provenance role of an output file, from what it is.
pub fn output_role(path: &str) -> String {
    let p = Path::new(path);
    match detect_filetype(p) {
        Some(FileType::Genbank) => return "genbank".into(),
        Some(FileType::Fasta) => return "fasta".into(),
        None => {}
    }
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let name = name.strip_suffix(".gz").unwrap_or(&name);
    let ext = name.rsplit('.').next().unwrap_or("");
    match ext {
        "hdf5" | "h5" => "matrix",
        "hmm" => "hmm",
        "dvb" | "dsnv" | "ssnv" => "viewer_bundle",
        "afa" | "afasta" => "alignment",
        "nwk" | "newick" => "newick",
        "xgmml" => "xgmml",
        "svg" => "svg",
        "json" | "ndjson" => "report",
        "txt" => "report",
        _ => "table",
    }
    .into()
}

/// The paths a field's value names: one string, or a list of them. Blank entries
/// and `-` (stdout/stdin) name no file.
fn paths_of(value: Option<&Value>) -> Vec<String> {
    let mut out = Vec::new();
    let mut push = |v: &Value| {
        if let Some(s) = v.as_str() {
            let s = s.trim();
            if !s.is_empty() && s != "-" {
                out.push(s.to_string());
            }
        }
    };
    match value {
        Some(Value::Array(items)) => items.iter().for_each(&mut push),
        Some(v) => push(v),
        None => {}
    }
    out
}

fn is_input(field: &FieldSpec) -> bool {
    matches!(field.role.as_str(), "input_file" | "input_files")
}

/// Every input file the params name, with its provenance role.
pub fn declared_inputs(spec: &ToolSpec, params: &Value) -> Vec<IoRef> {
    spec.fields
        .iter()
        .filter(|f| is_input(f))
        .flat_map(|f| {
            let role = input_role(f);
            paths_of(params.get(&f.name))
                .into_iter()
                .map(move |path| IoRef {
                    path,
                    role: role.clone(),
                })
        })
        .collect()
}

/// Every output file the params name. Only these are ever deleted by a cancel,
/// and only these are recorded, so a file is never touched for matching a pattern.
pub fn declared_outputs(spec: &ToolSpec, params: &Value) -> Vec<String> {
    spec.fields
        .iter()
        .filter(|f| f.role == "output_file")
        .flat_map(|f| paths_of(params.get(&f.name)))
        .collect()
}

/// The required inputs the params leave empty, by label.
///
/// Checked before a worker is started, not left to the CLI: a CLI tool with no
/// input reads stdin, and a worker's stdin is its lifeline (`worker.rs`).
pub fn missing_required_inputs(spec: &ToolSpec, params: &Value) -> Vec<String> {
    spec.fields
        .iter()
        .filter(|f| f.required && is_input(f) && paths_of(params.get(&f.name)).is_empty())
        .map(|f| f.label.clone())
        .collect()
}

/// The outputs a successful run left on disk, with their roles. Many outputs are
/// optional, and a tool may decline to write one, so existence is the test.
pub fn existing_outputs(outputs: &[String]) -> Vec<IoRef> {
    outputs
        .iter()
        .filter(|p| Path::new(p).exists())
        .map(|p| IoRef {
            path: p.clone(),
            role: output_role(p),
        })
        .collect()
}

/// A file's identity for "did the run change it": size and modification time.
type Stamp = (u64, Option<SystemTime>);

fn stamp(path: &Path) -> Option<Stamp> {
    let m = std::fs::metadata(path).ok()?;
    m.is_file().then(|| (m.len(), m.modified().ok()))
}

/// Each declared output's state before a job starts, so a cancel can tell the
/// files it created or changed from the ones it never touched.
pub struct OutputSnapshot(Vec<(PathBuf, Option<Stamp>)>);

/// What [`OutputSnapshot::remove_changed`] deleted.
#[derive(Debug, Default, PartialEq)]
pub struct Cleanup {
    /// Files the job created.
    pub created: Vec<String>,
    /// Files that existed before the job and that it had already rewritten.
    /// Their old contents were gone before the cancel; the partial new ones are
    /// removed like any other.
    pub overwritten: Vec<String>,
}

impl OutputSnapshot {
    pub fn take(outputs: &[String]) -> Self {
        OutputSnapshot(
            outputs
                .iter()
                .map(|p| (PathBuf::from(p), stamp(Path::new(p))))
                .collect(),
        )
    }

    /// Delete every declared output that did not exist before the job, or that
    /// changed since it started. A killed tool can leave a truncated file that
    /// still reads as valid (a GenBank file cut at a record boundary), which is
    /// worse than no file.
    pub fn remove_changed(&self) -> Cleanup {
        let mut done = Cleanup::default();
        for (path, before) in &self.0 {
            let Some(now) = stamp(path) else { continue };
            if before.as_ref() == Some(&now) {
                continue;
            }
            if std::fs::remove_file(path).is_ok() {
                let shown = path.to_string_lossy().into_owned();
                if before.is_some() {
                    done.overwritten.push(shown);
                } else {
                    done.created.push(shown);
                }
            }
        }
        done
    }
}

fn file_name(p: &str) -> String {
    Path::new(p)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.to_string())
}

/// The job message for a canceled run, saying what the cleanup removed.
pub fn canceled_message(c: &Cleanup) -> String {
    let mut msg = String::from("canceled");
    if !c.created.is_empty() {
        let names: Vec<String> = c.created.iter().map(|p| file_name(p)).collect();
        msg.push_str(&format!("; removed partial output {}", names.join(", ")));
    }
    if !c.overwritten.is_empty() {
        let names: Vec<String> = c.overwritten.iter().map(|p| file_name(p)).collect();
        msg.push_str(&format!(
            "; {} existed before the run and had already been overwritten, so it is gone too",
            names.join(", ")
        ));
    }
    msg
}

const MAX_MESSAGE: usize = 400;

fn clip(s: &str) -> String {
    if s.chars().count() <= MAX_MESSAGE {
        return s.to_string();
    }
    let mut out: String = s.chars().take(MAX_MESSAGE).collect();
    out.push('…');
    out
}

fn last_line(s: &str) -> Option<&str> {
    s.lines().map(str::trim).rfind(|l| !l.is_empty())
}

/// A successful run's message: the tool's last line of output (the CLI's tools
/// end on a summary such as `wrote 12 records to out.gb`), else `completed`.
pub fn success_message(stdout: &str, stderr: &str) -> String {
    last_line(stdout)
        .or_else(|| last_line(stderr))
        .map(clip)
        .unwrap_or_else(|| "completed".into())
}

/// A failed run's message, from its stderr.
///
/// The CLI reports an error as one line, `domainatoredux <tool>: error: <msg>: <cause>:
/// ...` (`domainator_cli::report_error`), and a usage error as clap does, `error: <msg>`
/// followed by the usage. Either way the message is what follows `error: `. The full
/// text is in the job's `worker.log`.
pub fn failure_message(stderr: &str, exit: &str) -> String {
    let reported = stderr.lines().rev().find_map(|l| {
        l.strip_prefix("domainatoredux")
            .and_then(|rest| rest.split_once(": error: "))
            .map(|(_, msg)| msg)
    });
    let usage = || stderr.lines().find_map(|l| l.strip_prefix("error: "));
    if let Some(msg) = reported
        .or_else(usage)
        .map(str::trim)
        .filter(|m| !m.is_empty())
    {
        return clip(msg);
    }
    // A panic, or a crash with nothing printed.
    let tail: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let tail = tail[tail.len().saturating_sub(3)..].join(" ");
    if tail.is_empty() {
        format!("the tool {exit}")
    } else {
        clip(&format!("the tool {exit}: {tail}"))
    }
}

/// How a worker ended, in words, for [`failure_message`].
pub fn describe_exit(status: &std::process::ExitStatus) -> String {
    if let Some(code) = status.code() {
        // Rust's exit code for a panic that unwound out of `main`.
        if code == 101 {
            return "crashed".into();
        }
        return format!("exited with code {code}");
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = status.signal() {
            let what = match sig {
                libc::SIGKILL => " (SIGKILL: out of memory?)",
                libc::SIGSEGV => " (SIGSEGV)",
                libc::SIGABRT => " (SIGABRT: aborted, possibly out of memory)",
                libc::SIGBUS => " (SIGBUS)",
                _ => "",
            };
            return format!("was killed by signal {sig}{what}");
        }
    }
    "stopped unexpectedly".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "domainatoredux-gui-jobio-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A cancel deletes what the job created or changed, and nothing else: not an
    /// untouched pre-existing output, and never a file that is not a declared
    /// output, whatever it is called.
    #[test]
    fn a_canceled_job_removes_only_the_outputs_it_created() {
        let d = scratch("cleanup");
        let p = |n: &str| d.join(n).to_string_lossy().into_owned();
        std::fs::write(p("untouched.gb"), "old").unwrap();
        std::fs::write(p("rewritten.gb"), "old").unwrap();
        std::fs::write(p("neighbor.gb"), "not an output").unwrap();
        let outputs = vec![
            p("new.gb"),
            p("untouched.gb"),
            p("rewritten.gb"),
            p("never_written.gb"),
        ];
        let snap = OutputSnapshot::take(&outputs);

        // The "run": creates one output, rewrites another (a different size, so
        // the change shows even on a coarse-mtime filesystem), and writes a file
        // that is not declared.
        std::fs::write(p("new.gb"), "partial").unwrap();
        std::fs::write(p("rewritten.gb"), "partial new contents").unwrap();
        std::fs::write(p("new.gb.didx"), "sidecar").unwrap();

        let c = snap.remove_changed();
        assert_eq!(c.created, vec![p("new.gb")]);
        assert_eq!(c.overwritten, vec![p("rewritten.gb")]);
        assert!(!Path::new(&p("new.gb")).exists());
        assert!(!Path::new(&p("rewritten.gb")).exists());
        assert_eq!(std::fs::read_to_string(p("untouched.gb")).unwrap(), "old");
        assert!(Path::new(&p("neighbor.gb")).exists());
        assert!(
            Path::new(&p("new.gb.didx")).exists(),
            "an undeclared file was deleted"
        );

        let msg = canceled_message(&c);
        assert!(
            msg.contains("new.gb") && msg.contains("rewritten.gb") && msg.contains("gone"),
            "{msg}"
        );
    }

    #[test]
    fn the_cli_error_line_is_the_message() {
        let stderr = "warning: something\ndomainatoredux domainate: error: reading references: opening x.hmm: No such file or directory (os error 2)\n";
        assert_eq!(
            failure_message(stderr, "exited with code 1"),
            "reading references: opening x.hmm: No such file or directory (os error 2)"
        );
        let usage = "error: No output specified. Give at least one of: --genbank_out, --fasta_out\n\nUsage: domainatoredux sort_contigs [OPTIONS] --input <INPUT>...\n";
        assert_eq!(
            failure_message(usage, "exited with code 2"),
            "No output specified. Give at least one of: --genbank_out, --fasta_out"
        );
        // No CLI error report: the tail of stderr, said to be a crash.
        let panic =
            "thread 'main' panicked at src/x.rs:1:1:\nboom\nnote: run with `RUST_BACKTRACE=1`\n";
        let m = failure_message(panic, "crashed");
        assert!(
            m.starts_with("the tool crashed: ") && m.contains("boom"),
            "{m}"
        );
        assert_eq!(
            failure_message("", "exited with code 7"),
            "the tool exited with code 7"
        );
    }

    #[test]
    fn the_success_message_is_the_last_line_printed() {
        assert_eq!(
            success_message("reading\nwrote 3 records to o.gb\n\n", ""),
            "wrote 3 records to o.gb"
        );
        assert_eq!(success_message("", "done\n"), "done");
        assert_eq!(success_message(" \n", ""), "completed");
    }

    #[test]
    fn output_roles_follow_the_file() {
        for (path, role) in [
            ("a/x.gb", "genbank"),
            ("x.fasta", "fasta"),
            ("x.hdf5", "matrix"),
            ("x.hmm", "hmm"),
            ("x.hmm.gz", "hmm"),
            ("x.dvb", "viewer_bundle"),
            ("x.dsnv", "viewer_bundle"),
            ("x.afa", "alignment"),
            ("x.nwk", "newick"),
            ("x.xgmml", "xgmml"),
            ("x.legend.svg", "svg"),
            ("x.enum.tsv", "table"),
            ("x.report.txt", "report"),
        ] {
            assert_eq!(output_role(path), role, "{path}");
        }
    }
}
