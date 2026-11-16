//! Tools that handle each record on its own read their input lazily and write each
//! result as it is made, as upstream's generators do, so a run's memory does not grow
//! with its input. These pin down what that changes beyond memory: standard input is
//! read as a stream (any bytes, gzip or not), `--max_records` stops reading, and a
//! run that fails partway leaves no output file however much it wrote.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn data() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/data")
}

fn scratch_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("domainator-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(args: &[&str], stdin: Option<&[u8]>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_domainatoredux"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut pipe = child.stdin.take().unwrap();
    if let Some(bytes) = stdin {
        pipe.write_all(bytes).unwrap();
    }
    drop(pipe);
    child.wait_with_output().unwrap()
}

fn ok(out: &Output) -> String {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn standard_input_is_read_as_a_stream_of_any_bytes() {
    let fixture = data().join("FeSOD_20.gb");
    let expected = ok(&run(
        &[
            "genbank_to_fasta",
            "-i",
            fixture.to_str().unwrap(),
            "-o",
            "-",
        ],
        None,
    ));
    assert_eq!(expected.matches('>').count(), 20);

    let text = std::fs::read(&fixture).unwrap();
    let from_stdin = ok(&run(
        &["genbank_to_fasta", "-i", "-", "-o", "-"],
        Some(&text),
    ));
    assert_eq!(from_stdin, expected);

    // Gzip content is decompressed, as it is for a file.
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&text).unwrap();
    let gzipped = gz.finish().unwrap();
    let from_gzip = ok(&run(
        &["genbank_to_fasta", "-i", "-", "-o", "-"],
        Some(&gzipped),
    ));
    assert_eq!(from_gzip, expected);

    // A byte that is not UTF-8 in a header line used to fail the whole read.
    let latin1 = b"LOCUS       x    4 bp    DNA     linear   UNK 01-JAN-1980\n\
DEFINITION  caf\xe9.\nORIGIN\n        1 acgt\n//\n";
    let fasta = run(&["genbank_to_fasta", "-i", "-", "-o", "-"], Some(latin1));
    assert!(ok(&fasta).ends_with("ACGT\n"));
}

#[test]
fn max_records_stops_reading_once_it_has_enough() {
    // The second input does not exist; it is never opened, as upstream's lazy
    // `parse_seqfiles` never opens it.
    let fixture = data().join("FeSOD_20.gb");
    let out = ok(&run(
        &[
            "select_by_contig",
            "-i",
            fixture.to_str().unwrap(),
            "/nonexistent/second_input.gb",
            "--max_records",
            "3",
            "--fasta_out",
            "-",
        ],
        None,
    ));
    assert_eq!(out.matches('>').count(), 3);
}

#[test]
fn a_run_that_fails_partway_leaves_no_output() {
    let dir = scratch_dir("streaming-failure");
    let fixture = data().join("FeSOD_20.gb");
    let fasta = dir.join("out.fasta");
    // Every record of the first input is written before the second fails to open.
    let out = run(
        &[
            "genbank_to_fasta",
            "-i",
            fixture.to_str().unwrap(),
            dir.join("missing.gb").to_str().unwrap(),
            "-o",
            fasta.to_str().unwrap(),
        ],
        None,
    );
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("missing.gb"));
    assert!(!fasta.exists());
    assert_eq!(
        std::fs::read_dir(&dir).unwrap().count(),
        0,
        "no temporary file left"
    );

    // --max_output_gb stops a streamed run at the record that crosses it.
    let genbank = dir.join("out.gb");
    let out = run(
        &[
            "domainate",
            "-i",
            fixture.to_str().unwrap(),
            "-r",
            data().join("FeSOD_pfam.hmm").to_str().unwrap(),
            "-o",
            genbank.to_str().unwrap(),
            "--max_output_gb",
            "0.000001",
        ],
        None,
    );
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("--max_hits_per_cds"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!genbank.exists());
    assert_eq!(
        std::fs::read_dir(&dir).unwrap().count(),
        0,
        "no temporary file left"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn deduplicate_clusters_in_one_pass_and_writes_in_another() {
    let dir = scratch_dir("streaming-dedup");
    let fixture = data().join("FeSOD_20.gb");
    let text = std::fs::read(&fixture).unwrap();
    let twice = dir.join("twice.gb");
    std::fs::write(&twice, [text.as_slice(), text.as_slice()].concat()).unwrap();
    let clusters = dir.join("clusters.tsv");
    let fasta = ok(&run(
        &[
            "deduplicate_genbank",
            "-i",
            twice.to_str().unwrap(),
            "--algorithm",
            "hash",
            "--identity",
            "1",
            "--fasta_out",
            "-",
            "--cluster_tsv_out",
            clusters.to_str().unwrap(),
            "--suffix_count",
            "--log_out",
            "/dev/null",
        ],
        None,
    ));
    // The first copy of each record, in input order, named with its cluster's size.
    let expected = ok(&run(
        &[
            "genbank_to_fasta",
            "-i",
            fixture.to_str().unwrap(),
            "-o",
            "-",
        ],
        None,
    ));
    let renamed: Vec<String> = expected
        .lines()
        .map(|line| match line.strip_prefix('>') {
            Some(title) => {
                let (id, rest) = title.split_once(' ').unwrap_or((title, ""));
                format!(">{id}-2 {rest}").trim_end().to_string()
            }
            None => line.to_string(),
        })
        .collect();
    let got: Vec<&str> = fasta.lines().collect();
    assert_eq!(got.len(), renamed.len());
    for (got, want) in got.iter().zip(&renamed) {
        if want.starts_with('>') {
            assert!(
                got.starts_with(want.split(' ').next().unwrap()),
                "{got} vs {want}"
            );
        } else {
            assert_eq!(got, want);
        }
    }
    let table = std::fs::read_to_string(&clusters).unwrap();
    assert_eq!(
        table.lines().count(),
        21,
        "a header and one row per cluster"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
