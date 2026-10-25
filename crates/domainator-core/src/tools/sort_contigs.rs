//! `sort_contigs`: order contigs by best `Domain_Search` score (or by the best
//! `Domainator` score among selected domains), optionally keeping the top N.
//! Port of `domainator.sort_contigs.sort_contigs`.

use std::collections::HashSet;

use crate::seq::{SeqFeature, SeqRecord};
use crate::{DOMAIN_FEATURE_NAME, DOMAIN_SEARCH_BEST_HIT_NAME};

/// Options for [`sort_contigs`].
#[derive(Default)]
pub struct SortContigsOptions {
    /// Keep only the top-N contigs by score (`None` = all).
    pub max_hits: Option<usize>,
    /// Sort by the best `Domainator` score among these domain names instead of
    /// `Domain_Search` scores.
    pub domains: Option<HashSet<String>>,
    /// When `domains` is set, only consider these databases.
    pub databases: Option<HashSet<String>>,
}

fn feature_score(f: &SeqFeature, record_id: &str) -> Result<f64, String> {
    f.get_first("score")
        .and_then(|s| s.parse::<f64>().ok())
        .ok_or_else(|| {
            format!(
                "Record {record_id} has a {} feature without a valid score.",
                f.kind
            )
        })
}

/// The best score for a record, per the selection rules. Errors when the record
/// has no matching feature (matching Python's `set_record_best_score`).
fn best_score(rec: &SeqRecord, opts: &SortContigsOptions) -> Result<f64, String> {
    let mut best: Option<f64> = None;
    match &opts.domains {
        None => {
            for f in &rec.features {
                if f.kind != DOMAIN_SEARCH_BEST_HIT_NAME {
                    continue;
                }
                let s = feature_score(f, &rec.id)?;
                best = Some(best.map_or(s, |b| b.max(s)));
            }
            best.ok_or_else(|| {
                format!(
                    "Record {} has no {DOMAIN_SEARCH_BEST_HIT_NAME} features to sort by.",
                    rec.id
                )
            })
        }
        Some(domains) => {
            for f in &rec.features {
                if f.kind != DOMAIN_FEATURE_NAME {
                    continue;
                }
                let name = f.get_first("name");
                let db = f.get_first("database");
                let (name, db) = match (name, db) {
                    (Some(n), Some(d)) => (n, d),
                    _ => {
                        return Err(format!(
                        "Record {} has a {DOMAIN_FEATURE_NAME} feature missing name or database.",
                        rec.id
                    ))
                    }
                };
                if !domains.contains(name) {
                    continue;
                }
                if let Some(dbs) = &opts.databases {
                    if !dbs.contains(db) {
                        continue;
                    }
                }
                let s = feature_score(f, &rec.id)?;
                best = Some(best.map_or(s, |b| b.max(s)));
            }
            best.ok_or_else(|| {
                format!(
                    "Record {} has no matching {DOMAIN_FEATURE_NAME} features.",
                    rec.id
                )
            })
        }
    }
}

/// Sort `records` by descending best score, keeping the top `max_hits`. Ties keep
/// input order (a stable secondary key).
pub fn sort_contigs(
    records: Vec<SeqRecord>,
    opts: &SortContigsOptions,
) -> Result<Vec<SeqRecord>, String> {
    let mut scored: Vec<(f64, usize, SeqRecord)> = Vec::with_capacity(records.len());
    for (i, rec) in records.into_iter().enumerate() {
        let s = best_score(&rec, opts)?;
        scored.push((s, i, rec));
    }
    // Descending score; stable on original index for ties.
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    if let Some(n) = opts.max_hits {
        scored.truncate(n);
    }
    Ok(scored.into_iter().map(|(_, _, r)| r).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seq::{Annotations, Location, Topology};

    fn rec_with_search(name: &str, score: f64) -> SeqRecord {
        let mut f = SeqFeature::new(DOMAIN_SEARCH_BEST_HIT_NAME, Location::range(0, 9, 1));
        f.set("score", format!("{score}"));
        SeqRecord {
            id: name.into(),
            name: name.into(),
            description: String::new(),
            seq: b"ACDEFGHIK".to_vec(),
            molecule_type: Some("protein".into()),
            topology: Topology::Linear,
            annotations: Annotations::default(),
            features: vec![f],
        }
    }

    #[test]
    fn sorts_descending_and_truncates() {
        let recs = vec![
            rec_with_search("a", 10.0),
            rec_with_search("b", 30.0),
            rec_with_search("c", 20.0),
        ];
        let out = sort_contigs(recs, &SortContigsOptions::default()).unwrap();
        assert_eq!(
            out.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            vec!["b", "c", "a"]
        );

        let recs = vec![
            rec_with_search("a", 10.0),
            rec_with_search("b", 30.0),
            rec_with_search("c", 20.0),
        ];
        let out = sort_contigs(
            recs,
            &SortContigsOptions {
                max_hits: Some(2),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            out.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            vec!["b", "c"]
        );
    }

    /// Upstream keeps tied contigs in input order, including at the `--max_hits` cut.
    #[test]
    fn ties_keep_input_order() {
        let recs = vec![
            rec_with_search("a", 10.0),
            rec_with_search("b", 20.0),
            rec_with_search("c", 10.0),
            rec_with_search("d", 20.0),
        ];
        let opts = SortContigsOptions {
            max_hits: Some(3),
            ..Default::default()
        };
        let out = sort_contigs(recs, &opts).unwrap();
        assert_eq!(
            out.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            vec!["b", "d", "a"]
        );
    }

    #[test]
    fn errors_when_no_search_feature() {
        let mut r = rec_with_search("a", 10.0);
        r.features.clear();
        assert!(sort_contigs(vec![r], &SortContigsOptions::default()).is_err());
    }
}
