//! `gene_call` end to end. Orphos is an external binary, so the tests that need it
//! skip (with a note) when it is not installed: set `$DOMAINATOREDUX_ORPHOS` or put
//! `orphos` on PATH to run them. The expected genes are pyrodigal's (upstream's
//! engine) on the same fixtures.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("domainator-cli-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn fixture(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test/data")
        .join(name)
        .to_str()
        .unwrap()
        .to_string()
}

fn orphos_available() -> bool {
    let bin = std::env::var("DOMAINATOREDUX_ORPHOS").unwrap_or_else(|_| "orphos".into());
    let found = Command::new(bin)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !found {
        eprintln!("skipping: orphos not found (set $DOMAINATOREDUX_ORPHOS)");
    }
    found
}

/// Run `gene_call` and return the output GenBank's CDS location lines and text.
fn gene_call(name: &str, args: &[&str]) -> (Vec<String>, String) {
    let out_path = scratch_dir(name).join("out.gb");
    let out = Command::new(env!("CARGO_BIN_EXE_domainatoredux"))
        .arg("gene_call")
        .args(args)
        .args(["-o", out_path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = std::fs::read_to_string(&out_path).unwrap();
    let cds = text
        .lines()
        .filter_map(|l| l.strip_prefix("     CDS             "))
        .map(str::to_string)
        .collect();
    (cds, text)
}

#[test]
fn calls_the_genes_pyrodigal_calls_on_a_linear_contig() {
    if !orphos_available() {
        return;
    }
    let (cds, text) = gene_call("linear", &["-i", &fixture("pDONR201.fasta")]);
    assert_eq!(
        cds,
        [
            "<1..106",
            "complement(959..1264)",
            "complement(1606..2265)",
            "2868..3677"
        ]
    );
    // The 5' end ran off the contig 1 bp in: extended to the edge, so the reading
    // frame starts at the second base, and the first codon is no start (no `M`).
    assert!(text.contains("/translation=\"FPALSPDSVDNRITASQEEFVETQKGHPSGWPSA*\""));
    assert!(text.contains("/codon_start=2"));
    assert!(text.contains("/gene_id=\"pDONR201_3\""));
    assert!(text.contains("/cds_id=\"1264_-1_959\""));
}

#[test]
fn a_circular_contig_has_no_edge_genes() {
    if !orphos_available() {
        return;
    }
    // The contig is circular, so its ends aren't real: the gene running off the
    // start is not kept as a partial, and the existing CDSs are replaced.
    let (cds, text) = gene_call(
        "circular",
        &[
            "-i",
            &fixture("pDONR201_domainator_circular.gb"),
            "--scope",
            "all",
        ],
    );
    assert_eq!(
        cds,
        [
            "complement(959..1264)",
            "complement(1606..2265)",
            "2868..3677"
        ]
    );
    assert!(
        !text.contains("Domainator "),
        "domain annotations are cleared"
    );
}

#[test]
fn unannotated_scope_leaves_annotated_contigs_alone() {
    // pDONR201.gb already has CDSs, so nothing is called -- and orphos is never run,
    // which this proves by pointing at a binary that does not exist.
    let (cds, text) = gene_call(
        "unannotated",
        &[
            "-i",
            &fixture("pDONR201.gb"),
            "--bin_path",
            "/nonexistent/orphos",
        ],
    );
    assert!(!cds.is_empty());
    assert!(!text.contains("/gene_id="));
}

#[test]
fn a_missing_orphos_says_how_to_get_one() {
    let out = Command::new(env!("CARGO_BIN_EXE_domainatoredux"))
        .args([
            "gene_call",
            "-i",
            &fixture("pDONR201.fasta"),
            "--bin_path",
            "/nonexistent/orphos",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("Install orphos"), "{err}");
}
