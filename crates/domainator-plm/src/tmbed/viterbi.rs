//! TMbed's Viterbi decoder (`tmbed/viterbi.py`), for one unpadded sequence.
//!
//! 27 states, each emitting one class's probability: five each of B (beta strand,
//! inside to outside) and b (outside to inside), H and h (alpha helix, the same two
//! orientations) and S (signal peptide), then i (inside) and o (outside). A segment's
//! states form a chain `X1 -> ... -> X5` with a self-loop on `X5`, so every TM segment
//! and signal peptide is at least five residues long. Scores are sums of raw
//! probabilities, not log-probabilities, with 0 for an allowed move and -100 for a
//! forbidden one; ties go to the lowest state, as `torch.max` breaks them.

/// The number of states.
const NUM_TAGS: usize = 27;
const I: usize = 25;
const O: usize = 26;

/// Which class (B, H, S, i, o) each state emits (`repeat_interleave([10, 10, 5, 1, 1])`).
const EMITS: [usize; NUM_TAGS] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 3, 4,
];

/// The label of each state: 0 B, 1 b, 2 H, 3 h, 4 S, 5 i, 6 o (TMbed's `mapping`).
const LABEL: [u8; NUM_TAGS] = [
    0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 5, 6,
];

/// The decoder's start, transition and end scores (`Decoder._init_transitions`).
struct Transitions {
    start: [f32; NUM_TAGS],
    end: [f32; NUM_TAGS],
    /// `next[from][to]`.
    next: [[f32; NUM_TAGS]; NUM_TAGS],
}

fn transitions() -> Transitions {
    let forbidden = -100.0;
    let mut start = [forbidden; NUM_TAGS];
    let mut end = [forbidden; NUM_TAGS];
    let mut next = [[forbidden; NUM_TAGS]; NUM_TAGS];
    // B1a, B1b, H1a, H1b, S1, i, o
    for s in [0, 5, 10, 15, 20, I, O] {
        start[s] = 0.0;
    }
    for i in 0..4 {
        for chain in [0, 5, 10, 15, 20] {
            next[chain + i][chain + i + 1] = 0.0; // X(i+1) -> X(i+2)
        }
    }
    for x5 in [4, 9, 14, 19, 24] {
        next[x5][x5] = 0.0;
    }
    next[4][O] = 0.0; // B5a -> o
    next[9][I] = 0.0; // B5b -> i
    next[14][O] = 0.0; // H5a -> o
    next[19][I] = 0.0; // H5b -> i
    next[24][I] = 0.0; // S5 -> i, o
    next[24][O] = 0.0;
    next[I][0] = 0.0; // i -> B1a, H1a, i, o
    next[I][10] = 0.0;
    next[I][I] = 0.0;
    next[I][O] = 0.0;
    next[O][5] = 0.0; // o -> B1b, H1b, i, o
    next[O][15] = 0.0;
    next[O][I] = 0.0;
    next[O][O] = 0.0;
    // B5a, B5b, H5a, H5b, S5, i, o
    for s in [4, 9, 14, 19, 24, I, O] {
        end[s] = 0.0;
    }
    Transitions { start, end, next }
}

/// The index of the first maximum.
fn argmax(values: &[f32]) -> usize {
    let mut best = 0;
    for (i, &v) in values.iter().enumerate().skip(1) {
        if v > values[best] {
            best = i;
        }
    }
    best
}

/// The most likely label (0 B, 1 b, 2 H, 3 h, 4 S, 5 i, 6 o) of each residue, from its
/// class probabilities in TMbed's order (B, H, S, i, o).
pub fn decode(probabilities: &[[f32; 5]]) -> Vec<u8> {
    let len = probabilities.len();
    if len == 0 {
        return Vec::new();
    }
    let t = transitions();
    let emission = |pos: usize, tag: usize| probabilities[pos][EMITS[tag]];
    let mut score: [f32; NUM_TAGS] = std::array::from_fn(|j| t.start[j] + emission(0, j));
    // history[i][j]: the best state at residue i, given state j at residue i + 1.
    let mut history = vec![[0u8; NUM_TAGS]; len];
    let mut candidates = [0f32; NUM_TAGS];
    for i in 1..len {
        let mut next_score = [0f32; NUM_TAGS];
        for j in 0..NUM_TAGS {
            let e = emission(i, j);
            for k in 0..NUM_TAGS {
                // torch adds transitions + score, then the emission.
                candidates[k] = t.next[k][j] + score[k] + e;
            }
            let k = argmax(&candidates);
            next_score[j] = candidates[k];
            history[i - 1][j] = k as u8;
        }
        score = next_score;
    }
    for (s, e) in score.iter_mut().zip(t.end) {
        *s += e;
    }
    let mut tag = argmax(&score);
    let mut labels = vec![0u8; len];
    labels[len - 1] = LABEL[tag];
    for i in (0..len - 1).rev() {
        tag = usize::from(history[i][tag]);
        labels[i] = LABEL[tag];
    }
    labels
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probs(classes: &[usize]) -> Vec<[f32; 5]> {
        classes
            .iter()
            .map(|&c| {
                let mut p = [0.05f32; 5];
                p[c] = 0.8;
                p
            })
            .collect()
    }

    #[test]
    fn a_confident_helix_between_inside_and_outside() {
        // i x3, H x6, o x3 -> the helix runs inside to outside: H.
        let p = probs(&[3, 3, 3, 1, 1, 1, 1, 1, 1, 4, 4, 4]);
        assert_eq!(decode(&p), [5, 5, 5, 2, 2, 2, 2, 2, 2, 6, 6, 6]);
        // o -> helix -> i is the other orientation: h.
        let p = probs(&[4, 4, 4, 1, 1, 1, 1, 1, 1, 3, 3, 3]);
        assert_eq!(decode(&p), [6, 6, 6, 3, 3, 3, 3, 3, 3, 5, 5, 5]);
    }

    // Expected labels below are TMbed's own `Decoder` on the same probabilities.

    #[test]
    fn a_short_helix_signal_is_stretched_to_five_residues() {
        // Three helix residues cannot make a helix; the decoder trades the following
        // inside residues for a five-residue segment, which may end the sequence.
        let p = probs(&[3, 3, 3, 1, 1, 1, 3, 3, 3]);
        assert_eq!(decode(&p), [5, 5, 5, 2, 2, 2, 2, 2, 2]);
    }

    #[test]
    fn a_signal_peptide_starts_at_the_first_residue() {
        let p = probs(&[2, 2, 2, 2, 2, 2, 4, 4, 4, 4]);
        assert_eq!(decode(&p), [4, 4, 4, 4, 4, 4, 6, 6, 6, 6]);
        // A signal peptide later on is pulled back to the start, over the residues
        // before it.
        let p = probs(&[4, 4, 4, 2, 2, 2, 2, 2, 2, 4, 4]);
        assert_eq!(decode(&p), [4, 4, 4, 4, 4, 4, 4, 4, 4, 6, 6]);
    }

    #[test]
    fn empty_and_single_residue() {
        assert!(decode(&[]).is_empty());
        assert_eq!(decode(&probs(&[4])), [6]);
    }
}
