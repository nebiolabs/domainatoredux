//! Rfam models on `206.gb`, a 142 kb AT-rich phage (NC_023573.1), where infernox 0.2.0
//! disagreed with C Infernal 1.1.5. Each fixture holds models from Rfam 15.1 and pins one
//! fix in the vendored crate (`vendor/infernox/PATCHES.md`). The expected hits are C
//! `cmsearch -Z 0.142094`'s, which pyinfernal and upstream's tests of the same names agree
//! on. The sweep rows `extra:infernal_hmmonly` and `extra:infernal_ssv_ties` compare the
//! whole files against upstream's.

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

/// The hits `domainate --evalue 10` reports on `206.gb` with the fixture `cm`.
fn hits_on_206(cm: &str) -> BTreeSet<(String, String)> {
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/data");
    let out = Command::new(env!("CARGO_BIN_EXE_domainatoredux"))
        .args(["domainate", "-q", "-i"])
        .arg(data.join("206.gb"))
        .arg("-r")
        .arg(data.join(cm))
        .args(["-o", "-", "--evalue", "10"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    domainator_hits(&String::from_utf8(out.stdout).unwrap())
}

fn set(hits: &[(&str, &str)]) -> BTreeSet<(String, String)> {
    hits.iter()
        .map(|(l, n)| (l.to_string(), n.to_string()))
        .collect()
}

/// A `.cm` model with no base pairs is searched by Infernal's HMM-only pipeline, and that
/// pipeline runs an MSV composition-bias window filter (F1b) before Viterbi and Forward
/// (Infernal 1.1.5 `cm_pipeline.c:2515,2521`). infernox 0.2.0 skipped it, which let most
/// windows through and gave many extra weak hits: DLEU2_1 alone went from 2 hits to 23.
/// `rfam_hmmonly.cm` is DLEU2_1 (RF02105) and TtnuCD7 (RF02284).
#[test]
fn models_without_basepairs_get_the_hmm_only_bias_filter() {
    assert_eq!(
        hits_on_206("rfam_hmmonly.cm"),
        set(&[
            ("13775..13887", "DLEU2_1"),
            ("109447..109491", "DLEU2_1"),
            ("complement(14137..14219)", "TtnuCD7"),
            ("complement(14229..14264)", "TtnuCD7"),
            ("59823..59877", "TtnuCD7"),
        ])
    );
}

/// The SSV filter ends each diagonal at the best-scoring model position of the row that
/// crossed the threshold. HMMER scans that row in its striped SIMD layout, so on a tie it
/// keeps the position it meets first in striped order, not the lowest one
/// (`msvfilter.c:373-381`). infernox 0.2.0 kept the lowest, which recovered other
/// diagonals and so other windows: 1469 SSV windows where C has 1456 on the top strand,
/// and three extra hits. `rfam_ssv_ties.cm` is S16-Flavobacteria (RF03138).
#[test]
fn ssv_windows_break_ties_in_striped_order() {
    let n = "S16-Flavobacteria";
    assert_eq!(
        hits_on_206("rfam_ssv_ties.cm"),
        set(&[
            ("complement(6353..6397)", n),
            ("complement(10272..10302)", n),
            ("complement(33740..33761)", n),
            ("complement(53425..53453)", n),
            ("complement(85966..86000)", n),
            ("90408..90437", n),
            ("complement(108973..108990)", n),
            ("complement(110119..110137)", n),
            ("112685..112701", n),
        ])
    );
}
