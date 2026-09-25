//! `trim_contigs`: trim contig ends by kb, by CDS count, or by stripping end
//! CDSs that contain (or lack) certain domains. Port of
//! `domainator.trim_contigs.trim_contigs`.

use std::collections::HashSet;

use crate::seq::location::{CompoundOp, Location};
use crate::seq::{SeqFeature, SeqRecord, Topology};
use crate::tools::select_common::{BooleanEvaluator, DomainatorCds};

/// A validated trim window: `(kb (left, right), cds (left, right))`, each
/// `None` when that unit was not requested.
type TrimRanges = (Option<(f64, f64)>, Option<(i64, i64)>);

/// Resolved trim options.
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
}

/// `validate_range` for trim_contigs (`kb_both/kb_left/kb_right` and
/// `cds_both/cds_left/cds_right`). Returns `(kb_truncation, cds_truncation)`.
pub fn validate_range(
    kb_both: Option<f64>,
    kb_left: Option<f64>,
    kb_right: Option<f64>,
    cds_both: Option<i64>,
    cds_left: Option<i64>,
    cds_right: Option<i64>,
) -> Result<TrimRanges, String> {
    // (Python names these kb_range/kb_up/kb_down internally but the flags are
    // kb_both/kb_left/kb_right. There is no non-negativity check in the trim
    // validate_range.)
    let mut cds_seen = false;
    let mut cds_out: Option<(i64, i64)> = None;
    if cds_left.is_some() || cds_right.is_some() {
        cds_out = Some((0, 0));
    }
    if let Some(l) = cds_left {
        cds_out = Some((l, cds_out.unwrap().1));
        cds_seen = true;
    }
    if let Some(r) = cds_right {
        cds_out = Some((cds_out.unwrap().0, r));
        cds_seen = true;
    }
    if let Some(b) = cds_both {
        if cds_seen {
            return Err("Cannot specify cds_range with cds_down or cds_up".into());
        }
        cds_seen = true;
        cds_out = Some((b, b));
    }

    if cds_seen && (kb_both.is_some() || kb_left.is_some() || kb_right.is_some()) {
        return Err("Cannot specify kb_range and cds_range".into());
    }

    let mut kb_seen = false;
    let mut kb_out: Option<(f64, f64)> = None;
    if kb_left.is_some() || kb_right.is_some() {
        kb_out = Some((0.0, 0.0));
    }
    if let Some(l) = kb_left {
        kb_out = Some((l, kb_out.unwrap().1));
        kb_seen = true;
    }
    if let Some(r) = kb_right {
        kb_out = Some((kb_out.unwrap().0, r));
        kb_seen = true;
    }
    if let Some(b) = kb_both {
        if kb_seen {
            return Err("Cannot specify kb_range with kb_down or kb_up".into());
        }
        kb_out = Some((b, b));
    }

    Ok((kb_out, cds_out))
}

/// `strip_domains_from_ends`: advance `left_cds` / retract `right_cds` past end
/// CDSs that carry a strip domain / match the expression / (with `no_domain`)
/// carry no domains. Returns `Err` when everything is stripped.
fn strip_domains_from_ends(
    cds_list: &[DomainatorCds],
    mut left_cds: i64,
    mut right_cds: i64,
    strip_domains: &HashSet<String>,
    domain_expr: Option<&BooleanEvaluator>,
    no_domain: bool,
) -> Result<(i64, i64), String> {
    if strip_domains.is_empty() && domain_expr.is_none() && !no_domain {
        return Ok((left_cds, right_cds));
    }
    let strip = |i: i64| -> bool {
        let dn = &cds_list[i as usize].domain_names;
        dn.intersection(strip_domains).next().is_some()
            || domain_expr.map(|e| e.check_expression(dn)).unwrap_or(false)
            || (no_domain && dn.is_empty())
    };
    while left_cds < cds_list.len() as i64 && strip(left_cds) {
        left_cds += 1;
    }
    while right_cds >= 0 && strip(right_cds) {
        right_cds -= 1;
    }
    if left_cds > right_cds {
        return Err("All CDSs in the contig are composed of domains to be stripped.".into());
    }
    Ok((left_cds, right_cds))
}

/// Trim one contig. Returns `None` when the whole contig is trimmed away.
fn trim_contig(
    record: &SeqRecord,
    opts: &TrimContigsOptions,
    domain_expr: Option<&BooleanEvaluator>,
) -> Option<SeqRecord> {
    if opts.kb_truncation.is_none()
        && opts.cds_truncation.is_none()
        && opts.strip_domains.is_none()
        && opts.domain_expr.is_none()
    {
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
        DomainatorCds::list_from_contig(record, f64::INFINITY, f64::NEG_INFINITY, false, None, false);
    cdss.sort_by_key(|c| c.feature.location.start());

    let mut left_cds = 0i64;
    let mut right_cds = cdss.len() as i64 - 1;

    // Python guard: `len(cdss) > 0 and strip_domains is not None or domain_expr is
    // not None or cds_truncation is not None or no_domain`. Operator precedence:
    // (len>0 and strip) or expr or cds or no_domain.
    let empty_strip = opts.strip_domains.clone();
    let do_cds_block = (!cdss.is_empty() && opts.strip_domains.is_some())
        || opts.domain_expr.is_some()
        || opts.cds_truncation.is_some()
        || opts.no_domain;

    if do_cds_block {
        if let Some((cl, cr)) = opts.cds_truncation {
            left_cds = cl;
            right_cds = (cdss.len() as i64 - 1) - cr;
        }

        if opts.strip_domains.is_some() || opts.domain_expr.is_some() || opts.no_domain {
            let strip = empty_strip.unwrap_or_default();
            match strip_domains_from_ends(&cdss, left_cds, right_cds, &strip, domain_expr, opts.no_domain) {
                Ok((dl, dr)) => {
                    if dl > left_cds {
                        left_cds = dl;
                    }
                    if dr < right_cds {
                        right_cds = dr;
                    }
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

        // Guard against out-of-range indexing when cdss is non-empty.
        if !cdss.is_empty()
            && left_cds >= 0
            && right_cds >= 0
            && (left_cds as usize) < cdss.len()
            && (right_cds as usize) < cdss.len()
        {
            if cdss[left_cds as usize].feature.location.start()
                > cdss[right_cds as usize].feature.location.end()
            {
                eprintln!(
                    "WARNING: CDSs in contig {} overlap after trimming. Skipping contig.",
                    record.id
                );
                return None;
            }
            if left_cds > 0 && cdss[left_cds as usize].feature.location.start() > left {
                left = cdss[left_cds as usize].feature.location.start();
            }
            if right_cds < cdss.len() as i64 - 1
                && cdss[right_cds as usize].feature.location.end() < right
            {
                right = cdss[right_cds as usize].feature.location.end();
            }
        }
    }

    if left >= right {
        return None;
    }

    if left > 0 || right < rec_len {
        let mut sliced = slice_forward(record, left, right);
        // Keep ACCESSION/VERSION unchanged when suffixing the LOCUS identifier.
        sliced.id = format!("{}_{}:{}", sliced.id, left + 1, right);
        Some(sliced)
    } else {
        Some(record.clone())
    }
}

/// Linear forward slice `[left, right)` truncating ALL overlapping features to
/// the slice (matching `utils.slice_record` with `truncate_features=True`), and
/// shifting coordinates by `-left`. Circular topology is dropped.
fn slice_forward(record: &SeqRecord, left: i64, right: i64) -> SeqRecord {
    let mut features = Vec::new();
    for f in &record.features {
        let mut parts: Vec<(i64, i64, i32, bool, bool)> = Vec::new();
        for &(s0, e0, strand, before, after) in &f.location.parts {
            let (s, e) = if s0 <= e0 { (s0, e0) } else { (e0, s0) };
            let ns = s.max(left);
            let ne = e.min(right);
            if ns < ne {
                parts.push((ns - left, ne - left, strand, before, after));
            }
        }
        if parts.is_empty() {
            continue;
        }
        parts.sort_by_key(|p| p.0);
        let operator = if parts.len() > 1 { f.location.operator.or(Some(CompoundOp::Join)) } else { None };
        features.push(SeqFeature {
            kind: f.kind.clone(),
            location: Location { operator, between: false, parts },
            qualifiers: f.qualifiers.clone(),
        });
    }
    SeqRecord {
        seq: record.seq[left as usize..right as usize].to_vec(),
        features,
        topology: Topology::Linear,
        ..record.clone()
    }
}

/// Trim all records.
pub fn trim_contigs(records: Vec<SeqRecord>, opts: &TrimContigsOptions) -> Result<Vec<SeqRecord>, String> {
    let expr = opts.domain_expr.as_deref().map(BooleanEvaluator::new);
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
        assert_eq!(validate_range(Some(1.0), None, None, None, None, None).unwrap(), (Some((1.0, 1.0)), None));
        assert_eq!(validate_range(None, Some(0.5), None, None, None, None).unwrap(), (Some((0.5, 0.0)), None));
        assert_eq!(validate_range(None, None, None, Some(2), None, None).unwrap(), (None, Some((2, 2))));
        assert_eq!(validate_range(None, None, None, None, Some(1), None).unwrap(), (None, Some((1, 0))));
        // Conflicts.
        assert!(validate_range(None, None, None, Some(2), Some(1), None).is_err());
        assert!(validate_range(Some(1.0), None, None, Some(2), None, None).is_err());
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
            seq: vec![b'A'; len as usize],
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
        };
        let out = trim_contigs(vec![r], &opts).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].seq.len(), 3000);
        assert_eq!(out[0].id, "c_1001:4000");
    }

    #[test]
    fn cds_trimming() {
        // 3 CDSs; trim 1 from each side -> keep only the middle CDS bounds.
        let r = rec(vec![cds("a", 100, 200), cds("b", 300, 400), cds("c", 500, 600)], 700);
        let opts = TrimContigsOptions {
            kb_truncation: None,
            cds_truncation: Some((1, 1)),
            strip_domains: None,
            domain_expr: None,
            no_domain: false,
        };
        let out = trim_contigs(vec![r], &opts).unwrap();
        assert_eq!(out.len(), 1);
        // left becomes start of cds b (300), right becomes end of cds b (400).
        assert_eq!(out[0].seq.len(), 100);
        assert_eq!(out[0].id, "c_301:400");
    }
}
