//! The best `n` of a stream of scored items, as upstream keeps them in a heap of
//! `(score, -arrival)`: higher scores first, and of equal scores the one that arrived
//! first. Holds at most `n` items, so the stream itself is never collected.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

/// The best `n` items pushed so far (every item when `n` is `None`).
pub struct TopN<T> {
    limit: Option<usize>,
    arrivals: usize,
    heap: BinaryHeap<Ranked<T>>,
}

impl<T> TopN<T> {
    pub fn new(limit: Option<usize>) -> Self {
        TopN {
            limit,
            arrivals: 0,
            heap: BinaryHeap::new(),
        }
    }

    /// Offer `item`, scored `score`; with the limit reached it replaces the worst kept
    /// item if it ranks above it, and is dropped otherwise.
    pub fn push(&mut self, score: f64, item: T) {
        let entry = Ranked {
            score,
            arrival: self.arrivals,
            item,
        };
        self.arrivals += 1;
        match self.limit {
            Some(n) if self.heap.len() >= n => {
                // The heap's top is its worst item.
                if self.heap.peek().is_some_and(|worst| entry < *worst) {
                    self.heap.pop();
                    self.heap.push(entry);
                }
            }
            _ => self.heap.push(entry),
        }
    }

    /// The kept items, best first.
    pub fn into_sorted_vec(self) -> Vec<T> {
        // Ascending in `Ranked`'s order, which is best first.
        self.heap
            .into_sorted_vec()
            .into_iter()
            .map(|r| r.item)
            .collect()
    }
}

/// An item and its rank, ordered worst-greatest -- lower score, or of equal scores the
/// later arrival -- so a max-heap's top is the item to drop.
struct Ranked<T> {
    score: f64,
    arrival: usize,
    item: T,
}

impl<T> Ord for Ranked<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .score
            .total_cmp(&self.score)
            .then(self.arrival.cmp(&other.arrival))
    }
}

impl<T> PartialOrd for Ranked<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> PartialEq for Ranked<T> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl<T> Eq for Ranked<T> {}

#[cfg(test)]
mod tests {
    use super::TopN;

    fn top(limit: Option<usize>, scores: &[f64]) -> Vec<usize> {
        let mut top = TopN::new(limit);
        for (i, &s) in scores.iter().enumerate() {
            top.push(s, i);
        }
        top.into_sorted_vec()
    }

    #[test]
    fn keeps_the_best_in_score_order_ties_by_arrival() {
        let scores = [1.0, 5.0, 3.0, 5.0, 2.0, 3.0];
        assert_eq!(top(None, &scores), [1, 3, 2, 5, 4, 0]);
        assert_eq!(top(Some(3), &scores), [1, 3, 2]);
        // The tie at the cut goes to the earlier item.
        assert_eq!(top(Some(4), &scores), [1, 3, 2, 5]);
        assert_eq!(top(Some(1), &scores), [1]);
    }

    #[test]
    fn matches_a_stable_sort_and_truncate() {
        // A deterministic scramble with many ties.
        let scores: Vec<f64> = (0u32..200).map(|i| f64::from(i * 37 % 11)).collect();
        let mut expected: Vec<usize> = (0..scores.len()).collect();
        expected.sort_by(|&a, &b| scores[b].total_cmp(&scores[a]));
        for n in [1, 7, 50, 199, 200, 500] {
            let mut want = expected.clone();
            want.truncate(n);
            assert_eq!(top(Some(n), &scores), want, "limit {n}");
        }
    }
}
