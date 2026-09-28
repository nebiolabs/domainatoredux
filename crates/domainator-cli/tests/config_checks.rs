//! Source-level guard on the CLI's per-tool dispatch.

/// Every tool's merged config must pass `reject_non_finite` before use: it is what
/// stops `nan`/`inf` reaching the tools (upstream's parser refuses them). A tool added
/// without the call would accept them silently, so check that each merge is followed
/// by one.
#[test]
fn every_merged_config_is_checked_for_non_finite_numbers() {
    let src = include_str!("../src/lib.rs");
    let merges: Vec<usize> = src.match_indices(".merge_cli(").map(|(i, _)| i).collect();
    assert!(merges.len() > 20, "found only {} merge sites", merges.len());
    for &i in &merges {
        let after = src[i..].split_once(";\n").map_or("", |(_, rest)| rest);
        let next = after.lines().next().unwrap_or("").trim();
        let line = src[..i].lines().count() + 1;
        assert_eq!(
            next, "config::reject_non_finite(&cfg)?;",
            "the merge_cli call at src/lib.rs:{line} is not followed by reject_non_finite"
        );
    }
}
