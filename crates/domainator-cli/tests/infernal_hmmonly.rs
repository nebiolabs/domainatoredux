//! A `.cm` model with no base pairs is searched by Infernal's HMM-only pipeline, and that
//! pipeline runs an MSV composition-bias window filter (F1b) before Viterbi and Forward
//! (Infernal 1.1.5 `cm_pipeline.c:2515,2521`). infernox 0.2.0 skipped it. On this AT-rich
//! phage that let most windows through and gave many extra weak hits: DLEU2_1 alone went
//! from 2 hits to 23. The expected hits are C `cmsearch -Z 0.142094`'s, which pyinfernal
//! and upstream's `test_domainator_infernal_searches_models_without_basepairs` agree on.
//! `rfam_hmmonly.cm` is DLEU2_1 (RF02105) and TtnuCD7 (RF02284) from Rfam 15.1. The sweep
//! row `extra:infernal_hmmonly` compares the whole file against upstream's.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

/// `(location, name)` of every `Domainator` feature in a GenBank text.
fn domainator_hits(genbank: &str) -> BTreeSet<(String, String)> {
    let mut hits = BTreeSet::new();
    let mut lines = genbank.lines().peekable();
    while let Some(line) = lines.next() {
        let Some(location) = line.strip_prefix("     Domainator      ") else {
            continue;
        };
        let mut name = None;
        while let Some(q) = lines
            .peek()
            .and_then(|l| l.strip_prefix("                     "))
        {
            if let Some(v) = q.strip_prefix("/name=\"") {
                name = Some(v.trim_end_matches('"').to_string());
            }
            lines.next();
        }
        hits.insert((
            location.trim().to_string(),
            name.expect("a Domainator feature has /name"),
        ));
    }
    hits
}

#[test]
fn models_without_basepairs_get_the_hmm_only_bias_filter() {
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/data");
    let out = Command::new(env!("CARGO_BIN_EXE_domainatoredux"))
        .args(["domainate", "-q", "-i"])
        .arg(data.join("206.gb"))
        .arg("-r")
        .arg(data.join("rfam_hmmonly.cm"))
        .args(["-o", "-", "--evalue", "10"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let hits = domainator_hits(&String::from_utf8(out.stdout).unwrap());
    let expected: BTreeSet<(String, String)> = [
        ("13775..13887", "DLEU2_1"),
        ("109447..109491", "DLEU2_1"),
        ("complement(14137..14219)", "TtnuCD7"),
        ("complement(14229..14264)", "TtnuCD7"),
        ("59823..59877", "TtnuCD7"),
    ]
    .into_iter()
    .map(|(l, n)| (l.to_string(), n.to_string()))
    .collect();
    assert_eq!(hits, expected);
}
