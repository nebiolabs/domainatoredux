//! Shared helpers for the `extract_domains` / `extract_unannotated` tools:
//! slicing a record from a (possibly compound) location with source/feature
//! propagation, padding a location on a linear or circular contig, and the
//! human-readable coordinate naming used by both tools.

use crate::seq::{SeqFeature, SeqRecord, Topology};
use crate::tools::neighborhood::{reverse_complement_record, slice_record, Env, SliceLoc};

/// A location as an ordered list of forward sub-parts (each `[start,end)`) with a
/// single overall strand — enough to represent the FeatureLocation /
/// single-operator CompoundLocation cases the extract tools produce.
#[derive(Clone, Debug)]
pub struct Loc {
    /// Forward sub-parts within `[0, L)`, in the order Python lists them.
    pub parts: Vec<(i64, i64)>,
    pub strand: i32,
}

impl Loc {
    pub fn single(start: i64, end: i64, strand: i32) -> Loc {
        Loc {
            parts: vec![(start, end)],
            strand,
        }
    }
    /// `location.start` (lowest coord).
    pub fn start(&self) -> i64 {
        self.parts.iter().map(|p| p.0).min().unwrap_or(0)
    }
    /// `location.end` (highest coord).
    pub fn end(&self) -> i64 {
        self.parts.iter().map(|p| p.1).max().unwrap_or(0)
    }
    /// Total covered length.
    pub fn len(&self) -> i64 {
        self.parts.iter().map(|p| p.1 - p.0).sum()
    }
    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }
    /// `stranded_start_human_readable`: `parts[0].end` if reverse, else `parts[0].start + 1`.
    pub fn stranded_start_hr(&self) -> i64 {
        if self.strand == -1 {
            self.parts[0].1
        } else {
            self.parts[0].0 + 1
        }
    }
    /// `stranded_end_human_readable`: parts[-1].start+1 if reverse, else parts[-1].end.
    ///
    /// The **last** part, matching `stranded_end`. Upstream read `parts[0]` here until
    /// `2de50d8`, which named an origin-crossing region `2438:2552` -- the end of the
    /// pre-origin segment -- where it continues to 707.
    ///
    /// Upstream also branches per part (`parts[-1].strand`) rather than on the
    /// location's overall strand, which is `None` for a mixed-strand join and silently
    /// took the forward branch. `Loc` carries one strand for all its parts, so it
    /// cannot represent that case and the two are equivalent here.
    pub fn stranded_end_hr(&self) -> i64 {
        let last = *self.parts.last().unwrap();
        if self.strand == -1 {
            last.0 + 1
        } else {
            last.1
        }
    }
}

/// `pad_location` for a **single-part** location on a linear or circular contig.
/// Returns the padded location. Circular padding wraps the origin (adding parts)
/// and splits overlap between the two ends. Only the single-part case is used by
/// the extract tools' `--pad_up/--pad_down`.
pub fn pad_location(rec: &SeqRecord, loc: &Loc, pad_up: i64, pad_down: i64) -> Loc {
    if pad_up == 0 && pad_down == 0 {
        return loc.clone();
    }
    let rec_len = rec.seq.len() as i64;
    // Compound locations.
    if loc.parts.len() != 1 {
        if rec.topology != Topology::Circular {
            // Linear: pad the first part upstream and last part downstream.
            let mut parts = loc.parts.clone();
            let first = pad_location(
                rec,
                &Loc {
                    parts: vec![parts[0]],
                    strand: loc.strand,
                },
                pad_up,
                0,
            );
            let last_idx = parts.len() - 1;
            let last = pad_location(
                rec,
                &Loc {
                    parts: vec![parts[last_idx]],
                    strand: loc.strand,
                },
                0,
                pad_down,
            );
            parts[0] = first.parts[0];
            parts[last_idx] = *last.parts.last().unwrap();
            return Loc {
                parts,
                strand: loc.strand,
            };
        }
        return pad_location_circular_compound(rec, loc, pad_up, pad_down, rec_len);
    }

    let (l_start, l_end) = loc.parts[0];
    if rec.topology != Topology::Circular {
        let (min_coord, max_coord) = if loc.strand == -1 {
            ((l_start - pad_down).max(0), (l_end + pad_up).min(rec_len))
        } else {
            ((l_start - pad_up).max(0), (l_end + pad_down).min(rec_len))
        };
        return Loc {
            parts: vec![(min_coord, max_coord)],
            strand: loc.strand,
        };
    }

    // Circular, single part (mirrors utils.pad_location circular single-part).
    let end_to_origin = rec_len - l_end;
    let (mut min_coord, mut max_coord) = if loc.strand == -1 {
        (l_start - pad_down, l_end + pad_up)
    } else {
        (l_start - pad_up, l_end + pad_down)
    };

    if min_coord >= 0 && max_coord <= rec_len {
        // Bounds don't cross the origin.
        return Loc {
            parts: vec![(min_coord, max_coord)],
            strand: loc.strand,
        };
    }

    // Truncate extensions that would run into the FeatureLocation from the far side.
    if min_coord < 0 && (-min_coord > end_to_origin) {
        min_coord = -end_to_origin;
    }
    if max_coord > rec_len && (max_coord - rec_len > l_start) {
        max_coord = rec_len + l_start;
    }
    // Divide any overlap equally between the two ends.
    let max_coord_shifted = max_coord - min_coord;
    let overlap = max_coord_shifted - rec_len;
    let (mut min_shifted, mut max_shifted) = (0i64, max_coord_shifted);
    if overlap > 0 {
        min_shifted += overlap / 2;
        max_shifted -= overlap / 2;
        if overlap % 2 == 1 {
            max_shifted -= 1;
        }
    }
    max_coord = max_shifted + min_coord;
    min_coord += min_shifted;

    // Build the (possibly wrapping) parts, following Python's construction.
    let mut parts: Vec<(i64, i64)> = Vec::new();
    if min_coord < 0 {
        parts.push((rec_len + min_coord, rec_len));
        if l_end > 0 {
            parts.push((0, l_end));
        }
    } else if min_coord != l_start {
        parts.push((min_coord, l_end));
    } else {
        parts.push((l_start, l_end));
    }
    if max_coord > rec_len {
        let last = parts.last_mut().unwrap();
        last.1 = rec_len;
        parts.push((0, max_coord - rec_len));
    } else {
        parts.last_mut().unwrap().1 = max_coord;
    }
    if loc.strand == -1 {
        parts.reverse();
    }
    Loc {
        parts,
        strand: loc.strand,
    }
}

/// `CompoundLocation.circular_envelope`: the smallest gapless arc containing all
/// parts and their junctions on a circular record. Returns forward sub-parts and
/// the overall strand. A single part means it does not cross the origin; two
/// parts mean it does. Port of the multi-part branch of `circular_envelope`.
fn circular_envelope(parts: &[(i64, i64, i32)], rec_len: i64, strand: i32) -> Vec<(i64, i64)> {
    if parts.is_empty() {
        return vec![];
    }
    if parts.len() == 1 {
        return vec![(parts[0].0, parts[0].1)];
    }
    // Extend each accumulated part to the next, crossing the origin as needed.
    let touches = |a: (i64, i64), b: (i64, i64)| a.0 <= b.1 && b.0 <= a.1;
    let mut loc_parts: Vec<(i64, i64, i32)> = vec![parts[0]];
    for &(next_start, next_end, next_strand) in &parts[1..] {
        let (this_start, this_end, this_strand) = *loc_parts.last().unwrap();
        if touches((this_start, this_end), (next_start, next_end)) {
            *loc_parts.last_mut().unwrap() = (
                this_start.min(next_start),
                this_end.max(next_end),
                next_strand,
            );
        } else if this_strand == -1 {
            let no_cross = if next_strand == -1 {
                next_end <= this_start
            } else {
                next_start <= this_start
            };
            if no_cross {
                *loc_parts.last_mut().unwrap() = (
                    this_start.min(next_start),
                    this_end.max(next_end),
                    next_strand,
                );
            } else {
                if this_start != 0 {
                    *loc_parts.last_mut().unwrap() = (0, this_end, this_strand);
                }
                loc_parts.push((next_start, rec_len, next_strand));
            }
        } else {
            let no_cross = if next_strand == -1 {
                next_end >= this_end
            } else {
                next_start >= this_end
            };
            if no_cross {
                *loc_parts.last_mut().unwrap() = (
                    this_start.min(next_start),
                    this_end.max(next_end),
                    next_strand,
                );
            } else {
                if this_end != rec_len {
                    *loc_parts.last_mut().unwrap() = (this_start, rec_len, this_strand);
                }
                loc_parts.push((0, next_end, next_strand));
            }
        }
    }
    // Merge (sorted by start).
    loc_parts.sort_by_key(|p| p.0);
    let mut merged: Vec<(i64, i64)> = Vec::new();
    for (s, e, _) in loc_parts {
        if let Some(last) = merged.last_mut() {
            if s <= last.1 {
                last.1 = last.1.max(e);
                continue;
            }
        }
        merged.push((s, e));
    }
    if merged.len() == 1 {
        merged
    } else {
        // Spans origin: for non-reverse strand, swap so the arc crosses the origin.
        if strand != -1 {
            vec![merged[1], merged[0]]
        } else {
            merged
        }
    }
}

/// `pad_location` multi-part circular branch (utils.pad_location lines 1408-1458).
fn pad_location_circular_compound(
    rec: &SeqRecord,
    loc: &Loc,
    pad_up: i64,
    pad_down: i64,
    rec_len: i64,
) -> Loc {
    let strand = loc.strand;
    let parts3: Vec<(i64, i64, i32)> = loc.parts.iter().map(|&(s, e)| (s, e, strand)).collect();
    let env = circular_envelope(&parts3, rec_len, strand);
    let env_len: i64 = env.iter().map(|(s, e)| e - s).sum();
    if env_len == rec_len {
        // Envelope covers the whole contig: return the original location.
        return loc.clone();
    }
    if env.len() == 1 {
        // Envelope does not cross the origin: padding might. Pad the single-part
        // envelope to get extension sizes, then apply to the first/last parts.
        let (up_ext, down_ext) =
            single_pad_extensions(rec, env[0].0, env[0].1, strand, pad_up, pad_down, rec_len);
        let first = pad_location(
            rec,
            &Loc::single(loc.parts[0].0, loc.parts[0].1, strand),
            up_ext,
            0,
        );
        let last_idx = loc.parts.len() - 1;
        let last = pad_location(
            rec,
            &Loc::single(loc.parts[last_idx].0, loc.parts[last_idx].1, strand),
            0,
            down_ext,
        );
        let mut out: Vec<(i64, i64)> = Vec::new();
        out.extend(first.parts);
        out.extend(loc.parts[1..last_idx].iter().copied());
        out.extend(last.parts);
        return Loc { parts: out, strand };
    }
    // Envelope crosses the origin: padding will not. Adjust pads to not run into it.
    // stranded_start/end of the (two-part, origin-crossing) envelope.
    let (env_start, env_end) = if strand == -1 {
        (env[0].1, env.last().unwrap().0)
    } else {
        (env[0].0, env.last().unwrap().1)
    };
    let remaining_span = (env_start - env_end).abs();
    let mut pu = pad_up.min(remaining_span);
    let mut pd = pad_down.min(remaining_span);
    let padding_size = pu + pd;
    if padding_size > remaining_span {
        let diff = padding_size - remaining_span;
        pu -= diff / 2;
        pd -= diff / 2;
        if diff % 2 == 1 {
            pd -= 1;
        }
    }
    let first = pad_location(
        rec,
        &Loc::single(loc.parts[0].0, loc.parts[0].1, strand),
        pu,
        0,
    );
    let last_idx = loc.parts.len() - 1;
    let last = pad_location(
        rec,
        &Loc::single(loc.parts[last_idx].0, loc.parts[last_idx].1, strand),
        0,
        pd,
    );
    let mut out: Vec<(i64, i64)> = Vec::new();
    out.extend(first.parts);
    out.extend(loc.parts[1..last_idx].iter().copied());
    out.extend(last.parts);
    Loc { parts: out, strand }
}

/// Extension sizes `(upstream, downstream)` from padding a single-part location on
/// a circular contig (mirrors pad_location's return_extension_sizes path).
fn single_pad_extensions(
    _rec: &SeqRecord,
    l_start: i64,
    l_end: i64,
    strand: i32,
    pad_up: i64,
    pad_down: i64,
    rec_len: i64,
) -> (i64, i64) {
    let end_to_origin = rec_len - l_end;
    let (mut min_coord, mut max_coord) = if strand == -1 {
        (l_start - pad_down, l_end + pad_up)
    } else {
        (l_start - pad_up, l_end + pad_down)
    };
    if min_coord >= 0 && max_coord <= rec_len {
        return (pad_up, pad_down);
    }
    if min_coord < 0 && (-min_coord > end_to_origin) {
        min_coord = -end_to_origin;
    }
    if max_coord > rec_len && (max_coord - rec_len > l_start) {
        max_coord = rec_len + l_start;
    }
    let max_shift = max_coord - min_coord;
    let overlap = max_shift - rec_len;
    let (mut min_s, mut max_s) = (0i64, max_shift);
    if overlap > 0 {
        min_s += overlap / 2;
        max_s -= overlap / 2;
        if overlap % 2 == 1 {
            max_s -= 1;
        }
    }
    max_coord = max_s + min_coord;
    min_coord += min_s;
    if strand == -1 {
        (max_coord - l_end, l_start - min_coord)
    } else {
        (l_start - min_coord, max_coord - l_end)
    }
}

/// Slice `rec` to `loc` with `keep_features`, mirroring
/// `slice_record_from_location`: each sub-part is extracted (reverse-strand parts
/// are reverse-complemented and concatenated in listed order) and the features are
/// truncated to it with `<`/`>` on the cut ends (see `neighborhood::slice_features`).
pub fn slice_from_location(rec: &SeqRecord, loc: &Loc, keep_features: &[SeqFeature]) -> SeqRecord {
    let parts: Vec<Env> = loc
        .parts
        .iter()
        .map(|&(s, e)| Env {
            start: s,
            end: e,
            strand: loc.strand,
        })
        .collect();
    let (ns, ne) = if loc.strand == -1 {
        (parts.last().unwrap().end, parts[0].start)
    } else {
        (parts[0].start, parts.last().unwrap().end)
    };
    let slice = SliceLoc {
        parts,
        name_start: ns,
        name_end: ne,
        overall_strand: loc.strand,
        dist_to_start: 0,
        dist_to_end: 0,
    };
    slice_record(rec, &slice, keep_features)
}

/// Convert the features of a CDS-sized nucleotide slice to peptide coordinates, in
/// place (`extract_peptides.dna_to_peptide_location`): each position becomes
/// `max(0, (position - offset) // 3)`, where `offset` is the CDS's reading-frame
/// offset (`/codon_start - 1`), and `<`/`>` markers stay on their ends.
pub fn dna_to_peptide_features(features: &mut [SeqFeature], offset: i64) {
    let to_pep = |pos: i64| ((pos - offset).div_euclid(3)).max(0);
    for f in features {
        for p in &mut f.location.parts {
            p.0 = to_pep(p.0);
            p.1 = to_pep(p.1);
        }
    }
}

/// Reverse-complement a record (used for `--keep_direction` on reverse-strand
/// nucleotide hits, which need flipping back).
pub fn revcomp_record(rec: &SeqRecord) -> SeqRecord {
    reverse_complement_record(rec)
}

#[cfg(test)]
mod tests {
    use super::{circular_envelope, slice_from_location, Loc};

    /// `(description, input parts, overall strand, expected envelope)`.
    type Case = (
        &'static str,
        &'static [(i64, i64, i32)],
        i32,
        &'static [(i64, i64)],
    );

    /// `circular_envelope`'s multi-part branch: the smallest gapless arc containing
    /// every part and the junctions between them, on a circular record.
    ///
    /// Expected values are the output of Python's
    /// `CompoundLocation.circular_envelope(100, strand)` for the same inputs, so this
    /// is a parity test rather than a lock on current behavior. It covers the four
    /// branches the accumulator can take: touching parts, disjoint-but-not-crossing
    /// on each strand, and the two origin-crossing pushes.
    #[test]
    fn circular_envelope_matches_python() {
        const REC: i64 = 100;
        let cases: &[Case] = &[
            (
                "touching parts merge",
                &[(10, 30, 1), (25, 50, 1)],
                1,
                &[(10, 50)],
            ),
            (
                "disjoint, no origin cross",
                &[(10, 30, 1), (40, 60, 1)],
                1,
                &[(10, 60)],
            ),
            (
                "forward crosses the origin",
                &[(80, 100, 1), (0, 20, 1)],
                1,
                &[(80, 100), (0, 20)],
            ),
            (
                "reverse crosses the origin",
                &[(0, 20, -1), (80, 100, -1)],
                -1,
                &[(0, 20), (80, 100)],
            ),
            (
                "reverse, no origin cross",
                &[(60, 80, -1), (10, 30, -1)],
                -1,
                &[(10, 80)],
            ),
            (
                "three parts, no wrap",
                &[(10, 20, 1), (30, 40, 1), (50, 60, 1)],
                1,
                &[(10, 60)],
            ),
            (
                "three parts, wrapping",
                &[(70, 90, 1), (95, 100, 1), (0, 10, 1)],
                1,
                &[(70, 100), (0, 10)],
            ),
        ];
        for (name, parts, strand, expected) in cases {
            assert_eq!(
                circular_envelope(parts, REC, *strand),
                *expected,
                "case: {name}"
            );
        }
    }

    /// `extract_peptides` slices with `truncate_features = true`, which goes through
    /// the second remapping implementation (`slice_record_truncating`). It carried the
    /// same missing merge: an origin-crossing CDS left the source feature as a `join`,
    /// which then survived the divide-by-3 into peptide coordinates.
    #[test]
    fn truncating_slice_coalesces_parts_split_at_the_origin() {
        use crate::seq::location::Location;
        use crate::seq::{Annotations, SeqFeature, SeqRecord, Topology};

        let rec = SeqRecord {
            id: "c".into(),
            name: "c".into(),
            description: String::new(),
            seq: b"AAAACCCCGGGGTTTTACGT".to_vec(),
            molecule_type: Some("DNA".into()),
            topology: Topology::Circular,
            annotations: Annotations::default(),
            features: vec![SeqFeature::new("source", Location::range(0, 20, 1))],
        };
        // The origin-crossing window [14,20) + [0,4), as a CDS location would give it.
        let loc = Loc {
            parts: vec![(14, 20), (0, 4)],
            strand: 1,
        };
        let out = slice_from_location(&rec, &loc, &rec.features);
        assert_eq!(out.seq, b"TTACGTAAAA");
        assert_eq!(
            out.features[0].location.parts,
            vec![(0, 10, 1, false, false)]
        );
        assert!(
            out.features[0].location.operator.is_none(),
            "must not stay a join"
        );
    }

    /// `stranded_{start,end}_human_readable`: the start reads `parts[0]`, the end
    /// reads `parts[-1]`, matching `stranded_start`/`stranded_end`. Expected values are
    /// the patched upstream's (`2de50d8`), which previously read `parts[0]` for the end
    /// and so named an origin-crossing region `2438:2552` rather than `2438:707`.
    /// Single-part locations cannot tell the two apart.
    #[test]
    fn stranded_human_readable_start_is_first_part_end_is_last() {
        // The origin-crossing Domainator feature join(2438..2552, 1..707).
        let fwd = Loc {
            parts: vec![(2437, 2552), (0, 707)],
            strand: 1,
        };
        assert_eq!(
            (fwd.stranded_start_hr(), fwd.stranded_end_hr()),
            (2438, 707)
        );

        let rev = Loc {
            parts: vec![(2437, 2552), (0, 707)],
            strand: -1,
        };
        assert_eq!((rev.stranded_start_hr(), rev.stranded_end_hr()), (2552, 1));

        // A single part is unaffected either way.
        let one = Loc {
            parts: vec![(99, 200)],
            strand: 1,
        };
        assert_eq!((one.stranded_start_hr(), one.stranded_end_hr()), (100, 200));
        let one_rev = Loc {
            parts: vec![(99, 200)],
            strand: -1,
        };
        assert_eq!(
            (one_rev.stranded_start_hr(), one_rev.stranded_end_hr()),
            (200, 100)
        );
    }

    #[test]
    fn circular_envelope_short_circuits() {
        assert_eq!(circular_envelope(&[], 100, 1), Vec::<(i64, i64)>::new());
        assert_eq!(circular_envelope(&[(10, 30, 1)], 100, 1), vec![(10, 30)]);
    }
}
