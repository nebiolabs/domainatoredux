//! `trim_contigs`: trim contig ends by kb, by CDS count, or by stripping end
//! CDSs that contain (or lack) certain domains. Port of
//! `domainator.trim_contigs.trim_contigs`.

use std::collections::HashSet;

use crate::seq::location::to_index;
use crate::seq::{SeqRecord, Topology};
use crate::tools::neighborhood::{region_id, slice_annotations, slice_linear_features, Env};
use crate::tools::select_common::{BooleanEvaluator, DomainatorCds};

/// A validated trim window: `(kb (left, right), cds (left, right))`, each
/// `None` when that unit was not requested.
type TrimRanges = (Option<(f64, f64)>, Option<(i64, i64)>);

/// Resolved trim options.
#[derive(Default)]
pub struct TrimContigsOptions {
    /// `(left_kb, right_kb)` if kb trimming requested.
    pub kb_truncation: Option<(f64, f64)>,
    /// `(left_cds, right_cds)` count if CDS trimming requested.
    pub cds_truncation: Option<(i64, i64)>,
    /// Strip end CDSs containing these domains.
    pub strip_domains: Option<HashSet<String>>,
    /// Strip end CDSs matching this boolean domain expression.
    pub domain_expr: Option<String>,
    /// Strip end CDSs with no domain annotations.
    pub no_domain: bool,
    /// Only domain annotations with an e-value at most this count (`None`: all).
    pub evalue: Option<f64>,
    /// Only domain annotations from these databases count (`None`: all).
    pub databases: Option<HashSet<String>>,
}

/// `validate_range` for trim_contigs (`kb_range/kb_left/kb_right` and
/// `cds_range/cds_left/cds_right`). Returns `(kb_truncation, cds_truncation)`.
///
/// Upstream's trim_contigs imports `select_by_cds.validate_range` itself, passing
/// range/left/right as range/up/down, so this delegates to the same port rather than
/// keeping a copy. A copy once dropped the non-negativity check: a negative
/// `--kb_left` then reached the slice as a negative index and panicked.
pub fn validate_range(
    kb_range: Option<f64>,
    kb_left: Option<f64>,
    kb_right: Option<f64>,
    cds_range: Option<i64>,
    cds_left: Option<i64>,
    cds_right: Option<i64>,
) -> Result<TrimRanges, String> {
    use crate::tools::neighborhood::{validate_range, Range};
    Ok(
        match validate_range(
            false, kb_range, kb_left, kb_right, cds_range, cds_left, cds_right,
        )? {
            Range::Kb(l, r) => (Some((l, r)), None),
            Range::Cds(l, r) => (None, Some((l, r))),
            Range::None | Range::Whole => (None, None),
        },
    )
}

/// `strip_domains_from_ends`: advance `left_cds` / retract `right_cds` past end
/// CDSs (given by their domain names, in contig order) that carry a strip domain /
/// match the expression / (with `no_domain`) carry no domains. Returns `Err` when
/// everything is stripped.
fn strip_domains_from_ends(
    cds_domain_names: &[HashSet<String>],
    mut left_cds: usize,
    mut right_cds: Option<usize>,
    strip_domains: &HashSet<String>,
    domain_expr: Option<&BooleanEvaluator>,
    no_domain: bool,
) -> Result<(usize, Option<usize>), String> {
    let strip = |dn: &HashSet<String>| -> bool {
        dn.intersection(strip_domains).next().is_some()
            || domain_expr.is_some_and(|e| e.check_expression(dn))
            || (no_domain && dn.is_empty())
    };
    while left_cds < cds_domain_names.len() && strip(&cds_domain_names[left_cds]) {
        left_cds += 1;
    }
    while let Some(r) = right_cds.filter(|&r| strip(&cds_domain_names[r])) {
        right_cds = r.checked_sub(1);
    }
    if right_cds.is_none_or(|r| left_cds > r) {
        return Err("All CDSs in the contig are composed of domains to be stripped.".into());
    }
    Ok((left_cds, right_cds))
}

/// Trim one contig (upstream `trim_contig`). Returns `None` when the contig is
/// trimmed away. Domain criteria only count annotations that pass `evalue` and
/// `databases`.
fn trim_contig(
    record: &SeqRecord,
    opts: &TrimContigsOptions,
    domain_expr: Option<&BooleanEvaluator>,
) -> Option<SeqRecord> {
    let by_domain = opts.strip_domains.is_some() || domain_expr.is_some() || opts.no_domain;
    if opts.kb_truncation.is_none() && opts.cds_truncation.is_none() && !by_domain {
        return Some(record.clone());
    }

    let rec_len = record.seq.len() as i64;
    let mut left = 0i64;
    let mut right = rec_len;
    if let Some((kl, kr)) = opts.kb_truncation {
        left += (kl * 1000.0) as i64;
        right -= (kr * 1000.0) as i64;
    }

    // DomainatorCDS list, sorted by feature.location.start.
    let mut cdss =
        DomainatorCds::list_from_contig(record, None, f64::NEG_INFINITY, false, None, false);
    cdss.sort_by_key(|c| c.feature.location.start());

    if !cdss.is_empty() && (by_domain || opts.cds_truncation.is_some()) {
        let n = cdss.len();
        let mut left_cds = 0usize;
        let mut right_cds = Some(n - 1);
        if let Some((cl, cr)) = opts.cds_truncation {
            // A contig with fewer CDSs than the trim asks for is trimmed away.
            let l = usize::try_from(cl).unwrap_or(usize::MAX);
            let r = usize::try_from(cr)
                .ok()
                .and_then(|cr| (n - 1).checked_sub(cr));
            match r {
                Some(r) if l <= r => (left_cds, right_cds) = (l, Some(r)),
                _ => return None,
            }
        }
        if by_domain {
            let names: Vec<HashSet<String>> = if opts.evalue.is_none() && opts.databases.is_none() {
                cdss.iter().map(|c| c.domain_names.clone()).collect()
            } else {
                cdss.iter()
                    .map(|c| c.get_domain_names(opts.databases.as_ref(), opts.evalue))
                    .collect()
            };
            let strip = opts.strip_domains.clone().unwrap_or_default();
            match strip_domains_from_ends(
                &names,
                left_cds,
                right_cds,
                &strip,
                domain_expr,
                opts.no_domain,
            ) {
                Ok((dl, dr)) => {
                    left_cds = left_cds.max(dl);
                    right_cds = right_cds.min(dr);
                }
                Err(_) => {
                    eprintln!(
                        "WARNING: All CDSs in contig {} are composed of domains to be stripped. Skipping contig.",
                        record.id
                    );
                    return None;
                }
            }
        }
        let right_cds = right_cds?;
        let lc_start = cdss[left_cds].feature.location.start();
        let rc_end = cdss[right_cds].feature.location.end();
        if lc_start > rc_end {
            eprintln!(
                "WARNING: CDSs in contig {} overlap after trimming. Skipping contig.",
                record.id
            );
            return None;
        }
        if left_cds > 0 && lc_start > left {
            left = lc_start;
        }
        if right_cds < n - 1 && rc_end < right {
            right = rc_end;
        }
    }

    // Validated options keep both edges inside the contig already; clamping makes
    // that local, so the slice below cannot be handed a negative index.
    let (left, right) = (left.max(0), right.min(rec_len));
    if left >= right {
        return None;
    }

    if left > 0 || right < rec_len {
        let mut sliced = slice_forward(record, left, right);
        // ACCESSION/VERSION keep the contig's.
        let part = Env {
            start: left,
            end: right,
            strand: 1,
        };
        sliced.id = region_id(record, &[part], false);
        Some(sliced)
    } else {
        Some(record.clone())
    }
}

/// Linear forward slice `[left, right)` (`utils.slice_record(record, left, right)`):
/// features truncated with `<`/`>` on the cut ends, cut hit features dropped,
/// coordinates shifted by `-left`. Circular topology is dropped.
fn slice_forward(record: &SeqRecord, left: i64, right: i64) -> SeqRecord {
    SeqRecord {
        seq: record.seq[to_index(left)..to_index(right)].to_vec(),
        features: slice_linear_features(record, left, right),
        topology: Topology::Linear,
        annotations: slice_annotations(record),
        ..record.clone()
    }
}

/// Trim all records.
pub fn trim_contigs(
    records: Vec<SeqRecord>,
    opts: &TrimContigsOptions,
) -> Result<Vec<SeqRecord>, String> {
    let expr = opts
        .domain_expr
        .as_deref()
        .map(BooleanEvaluator::new)
        .transpose()?;
    let mut out = Vec::new();
    for record in records {
        // Circular contigs are linearized before trimming (topology dropped).
        let mut record = record;
        record.topology = Topology::Linear;
        if let Some(t) = trim_contig(&record, opts, expr.as_ref()) {
            out.push(t);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seq::{Annotations, Location, SeqFeature, Topology};

    #[test]
    fn validate_range_both_and_conflicts() {
        assert_eq!(
            validate_range(Some(1.0), None, None, None, None, None).unwrap(),
            (Some((1.0, 1.0)), None)
        );
        assert_eq!(
            validate_range(None, Some(0.5), None, None, None, None).unwrap(),
            (Some((0.5, 0.0)), None)
        );
        assert_eq!(
            validate_range(None, None, None, Some(2), None, None).unwrap(),
            (None, Some((2, 2)))
        );
        assert_eq!(
            validate_range(None, None, None, None, Some(1), None).unwrap(),
            (None, Some((1, 0)))
        );
        // Conflicts.
        assert!(validate_range(None, None, None, Some(2), Some(1), None).is_err());
        assert!(validate_range(Some(1.0), None, None, Some(2), None, None).is_err());
        // Negative ranges are rejected, as upstream does; one used to panic in the slice.
        for bad in [
            validate_range(Some(-1.0), None, None, None, None, None),
            validate_range(None, Some(-1.0), Some(1.0), None, None, None),
            validate_range(None, None, Some(-0.5), None, None, None),
            validate_range(None, None, None, Some(-1), None, None),
            validate_range(None, None, None, None, Some(-1), None),
            validate_range(None, None, None, None, None, Some(-2)),
        ] {
            assert!(bad.is_err(), "{bad:?}");
        }
    }

    fn cds(cds_id: &str, start: i64, end: i64) -> SeqFeature {
        let mut f = SeqFeature::new("CDS", Location::range(start, end, 1));
        f.set("cds_id", cds_id);
        f
    }

    fn rec(features: Vec<SeqFeature>, len: i64) -> SeqRecord {
        SeqRecord {
            id: "c".into(),
            name: "c".into(),
            description: String::new(),
            seq: vec![b'A'; usize::try_from(len).unwrap()],
            molecule_type: Some("DNA".into()),
            topology: Topology::Linear,
            annotations: Annotations::default(),
            features,
        }
    }

    #[test]
    fn kb_trimming() {
        let r = rec(vec![], 5000);
        let opts = TrimContigsOptions {
            kb_truncation: Some((1.0, 1.0)),
            cds_truncation: None,
            strip_domains: None,
            domain_expr: None,
            no_domain: false,
            ..Default::default()
        };
        let out = trim_contigs(vec![r], &opts).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].seq.len(), 3000);
        assert_eq!(out[0].id, "c_1001:4000");
    }

    #[test]
    fn cds_trimming() {
        // 3 CDSs; trim 1 from each side -> keep only the middle CDS bounds.
        let r = rec(
            vec![cds("a", 100, 200), cds("b", 300, 400), cds("c", 500, 600)],
            700,
        );
        let opts = TrimContigsOptions {
            kb_truncation: None,
            cds_truncation: Some((1, 1)),
            strip_domains: None,
            domain_expr: None,
            no_domain: false,
            ..Default::default()
        };
        let out = trim_contigs(vec![r], &opts).unwrap();
        assert_eq!(out.len(), 1);
        // left becomes start of cds b (300), right becomes end of cds b (400).
        assert_eq!(out[0].seq.len(), 100);
        assert_eq!(out[0].id, "c_301:400");
    }

    fn dom(cds_id: &str, name: &str, start: i64, end: i64, evalue: &str) -> SeqFeature {
        let mut f = SeqFeature::new(crate::DOMAIN_FEATURE_NAME, Location::range(start, end, 1));
        f.set("cds_id", cds_id);
        f.set("name", name);
        f.set("database", "db");
        f.set("evalue", evalue);
        f.set("score", "50");
        f
    }

    /// Upstream 1.0: `--unannotated` trims on its own, and `-e`/`--databases` limit
    /// which annotations make a CDS "annotated".
    #[test]
    fn unannotated_trims_alone_and_evalue_decides_what_counts() {
        let r = rec(
            vec![
                cds("a", 100, 200),
                cds("b", 300, 400),
                dom("b", "D", 300, 400, "1e-20"),
                cds("c", 500, 600),
                dom("c", "D", 500, 600, "1e-2"),
            ],
            700,
        );
        let opts = TrimContigsOptions {
            no_domain: true,
            ..Default::default()
        };
        let out = trim_contigs(vec![r.clone()], &opts).unwrap();
        assert_eq!(out[0].id, "c_301:700");
        let strict = TrimContigsOptions {
            no_domain: true,
            evalue: Some(1e-10),
            ..Default::default()
        };
        let out = trim_contigs(vec![r], &strict).unwrap();
        assert_eq!(out[0].id, "c_301:400");
    }

    /// A contig with fewer CDSs than a CDS trim removes is dropped, and one with no
    /// CDSs at all is left as it is.
    #[test]
    fn a_cds_trim_past_the_last_cds_drops_the_contig() {
        let opts = TrimContigsOptions {
            cds_truncation: Some((1, 1)),
            ..Default::default()
        };
        let two = rec(vec![cds("a", 100, 200), cds("b", 300, 400)], 500);
        assert!(trim_contigs(vec![two], &opts).unwrap().is_empty());
        let none = rec(vec![], 500);
        let out = trim_contigs(vec![none], &opts).unwrap();
        assert_eq!(out[0].seq.len(), 500);
    }
}
