//! `--dedup`, the output-uniqueness policy `domain_search` and `select_by_cds` share
//! (upstream `dedup.py`).
//!
//! Domainator assumes a record ID always names the same sequence (annotations and
//! hit scores may differ), so a sequence policy also makes IDs unique, and each policy
//! needs only one key.

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::seq::SeqRecord;
use crate::tools::deduplicate_genbank::sequence_keys;

/// Which output records count as copies of one another (upstream `DEDUP_CHOICES`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Dedup {
    /// Keep every record.
    #[default]
    None,
    /// Drop a repeated record ID.
    Id,
    /// Drop an identical sequence, ignoring case and a terminal `*`.
    Sequence,
    /// As `Sequence`, and also a nucleotide sequence's reverse complement.
    SequenceRc,
}

impl std::str::FromStr for Dedup {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "none" => Ok(Dedup::None),
            "id" => Ok(Dedup::Id),
            "sequence" => Ok(Dedup::Sequence),
            "sequence_rc" => Ok(Dedup::SequenceRc),
            _ => Err(format!(
                "dedup must be one of none, id, sequence, sequence_rc; got {s:?}."
            )),
        }
    }
}

/// What identifies a record under a policy.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Key {
    Id(String),
    Sequence(Vec<u8>),
    /// No policy: every record is its own key.
    Arrival(u64),
}

/// Upstream `_record_key`: `record`'s key, or `None` under `Dedup::None`. A
/// reverse-complement policy refuses a protein.
fn record_key(record: &SeqRecord, dedup: Dedup) -> Result<Option<Key>, String> {
    Ok(match dedup {
        Dedup::None => None,
        Dedup::Id => Some(Key::Id(record.id.clone())),
        Dedup::Sequence | Dedup::SequenceRc => {
            let both_strands = dedup == Dedup::SequenceRc;
            if both_strands && record.is_protein() {
                return Err("--dedup sequence_rc is only supported for nucleotide output.".into());
            }
            // The smaller of the two strands stands for both.
            sequence_keys(&record.seq, both_strands)
                .into_iter()
                .min()
                .map(Key::Sequence)
        }
    })
}

/// Upstream `RecordDeduplicator`: keeps the first record with each key.
#[derive(Debug, Default)]
pub struct RecordDeduplicator {
    dedup: Dedup,
    seen: HashSet<Key>,
}

impl RecordDeduplicator {
    pub fn new(dedup: Dedup) -> Self {
        RecordDeduplicator {
            dedup,
            seen: HashSet::new(),
        }
    }

    /// Whether `record` is the first with its key (recording it if so).
    pub fn keep(&mut self, record: &SeqRecord) -> Result<bool, String> {
        Ok(match record_key(record, self.dedup)? {
            None => true,
            Some(key) => self.seen.insert(key),
        })
    }
}

/// A score ordered by `total_cmp`, so ranks can live in a `BTreeSet`.
#[derive(Clone, Copy, Debug)]
struct Score(f64);

impl PartialEq for Score {
    fn eq(&self, other: &Self) -> bool {
        self.0.total_cmp(&other.0).is_eq()
    }
}
impl Eq for Score {}
impl PartialOrd for Score {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Score {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

/// A rank: higher score first, then the earlier arrival (`(score, -arrival)`).
type Rank = (Score, std::cmp::Reverse<u64>);

/// Upstream `TopHits`: at most `max_hits` of the best-scoring items, unique by the
/// policy. A better-scoring copy replaces its representative; of equal scores the
/// earlier arrival wins. Only the kept items are held.
#[derive(Debug)]
pub struct TopHits<T> {
    max_hits: usize,
    dedup: Dedup,
    arrival: u64,
    kept: HashMap<Key, (Rank, T)>,
    /// The kept items' ranks, weakest first.
    ranks: BTreeSet<(Rank, Key)>,
}

impl<T> TopHits<T> {
    /// `max_hits` must be at least 1.
    pub fn new(max_hits: usize, dedup: Dedup) -> Result<Self, String> {
        if max_hits < 1 {
            return Err("max_hits must be at least 1.".into());
        }
        Ok(TopHits {
            max_hits,
            dedup,
            arrival: 0,
            kept: HashMap::new(),
            ranks: BTreeSet::new(),
        })
    }

    pub fn len(&self) -> usize {
        self.kept.len()
    }

    pub fn is_empty(&self) -> bool {
        self.kept.is_empty()
    }

    /// Offer `item`, whose record `record_of` returns, with `score`.
    pub fn add(
        &mut self,
        item: T,
        record_of: impl Fn(&T) -> &SeqRecord,
        score: f64,
    ) -> Result<(), String> {
        let arrival = self.arrival;
        self.arrival += 1;
        let key = record_key(record_of(&item), self.dedup)?.unwrap_or(Key::Arrival(arrival));
        let rank: Rank = (Score(score), std::cmp::Reverse(arrival));
        if let Some((current, _)) = self.kept.get(&key) {
            if rank <= *current {
                return Ok(());
            }
            self.ranks.remove(&(*current, key.clone()));
        } else if self.kept.len() == self.max_hits {
            let weakest = self.ranks.first().expect("max_hits >= 1").clone();
            if rank <= weakest.0 {
                return Ok(());
            }
            self.ranks.remove(&weakest);
            self.kept.remove(&weakest.1);
        }
        self.ranks.insert((rank, key.clone()));
        self.kept.insert(key, (rank, item));
        Ok(())
    }

    /// The kept items, best first.
    pub fn into_sorted(self) -> Vec<T> {
        let mut kept = self.kept;
        self.ranks
            .into_iter()
            .rev()
            .map(|(_, key)| kept.remove(&key).expect("every rank is kept").1)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seq::{Annotations, Topology};

    fn rec(id: &str, seq: &str, molecule: &str) -> SeqRecord {
        SeqRecord {
            id: id.into(),
            name: id.into(),
            description: String::new(),
            seq: seq.as_bytes().to_vec(),
            molecule_type: Some(molecule.into()),
            topology: Topology::Linear,
            features: Vec::new(),
            annotations: Annotations::default(),
        }
    }

    #[test]
    fn record_deduplicator_policies() {
        let records = [
            rec("a", "ACGT*", "DNA"),
            rec("a", "TTTT", "DNA"),
            rec("b", "acgt", "DNA"),
            rec("c", "ACGT", "DNA"),
        ];
        let kept = |dedup| {
            let mut d = RecordDeduplicator::new(dedup);
            records
                .iter()
                .filter(|r| d.keep(r).unwrap())
                .map(|r| r.seq.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(kept(Dedup::None).len(), 4);
        assert_eq!(
            kept(Dedup::Id),
            [b"ACGT*".to_vec(), b"acgt".to_vec(), b"ACGT".to_vec()]
        );
        // ACGT is its own reverse complement; TTTT's is AAAA.
        assert_eq!(kept(Dedup::Sequence), [b"ACGT*".to_vec(), b"TTTT".to_vec()]);
        let mut d = RecordDeduplicator::new(Dedup::SequenceRc);
        assert!(d.keep(&rec("x", "AAAAC", "DNA")).unwrap());
        assert!(!d.keep(&rec("y", "GTTTT", "DNA")).unwrap());
        assert!(d.keep(&rec("p", "MKV", "protein")).is_err());
    }

    #[test]
    fn top_hits_keeps_the_best_unique_representatives() {
        let mut top = TopHits::new(2, Dedup::Sequence).unwrap();
        for (id, seq, score) in [
            ("a", "AAA", 5.0),
            ("b", "CCC", 7.0),
            ("a2", "AAA", 9.0),
            ("d", "GGG", 6.0),
        ] {
            top.add(rec(id, seq, "DNA"), |r| r, score).unwrap();
        }
        // a2 replaced its copy a, then d (6) could not displace b (7).
        let ids: Vec<String> = top.into_sorted().into_iter().map(|r| r.id).collect();
        assert_eq!(ids, ["a2", "b"]);

        // Equal scores favor the earlier arrival, and a later equal copy does not replace.
        let mut top = TopHits::new(2, Dedup::None).unwrap();
        for id in ["first", "second", "third"] {
            top.add(rec(id, "A", "DNA"), |r| r, 1.0).unwrap();
        }
        let ids: Vec<String> = top.into_sorted().into_iter().map(|r| r.id).collect();
        assert_eq!(ids, ["first", "second"]);
        assert!(TopHits::<()>::new(0, Dedup::None).is_err());
    }
}
