//! Gene-neighborhood extraction for `domain_search` (the `select_by_cds`
//! `_from_domain_search` path): given a Domainator-annotated contig with CDS
//! hits, emit the contig region around each hit CDS — `--cds_range` (N CDSs
//! up/downstream) or `--kb_range` (kb up/downstream), or the whole contig.
//!
//! Faithful port of `domainator.select_by_cds.{get_cds_neighborhood,select_by_cds}`
//! plus `utils.{pad_location,slice_record_from_location}` and
//! `select_by_cds.pad_circular_cds`, for linear and circular topologies, with
//! `normalize_direction` (reverse-complement reverse-strand neighborhoods).

use crate::seq::location::{merge_touching_parts, to_index, CompoundOp, Location};
use crate::seq::partial::{codon_offset, location_partial_ends, on_contig_translation};
use crate::seq::translate::revcomp;
use crate::seq::{SeqFeature, SeqRecord, Topology};
use crate::tools::select_common::{get_non_domainator_features, DomainatorCds};
use crate::{DOMAIN_FEATURE_NAME, DOMAIN_SEARCH_BEST_HIT_NAME};

/// Which neighborhood window to extract.
#[derive(Clone, Copy, Default)]
pub enum Range {
    /// `(upstream, downstream)` count of CDSs.
    Cds(i64, i64),
    /// `(upstream, downstream)` in kb.
    Kb(f64, f64),
    /// The whole contig.
    Whole,
    /// Just the hit CDS.
    #[default]
    None,
}

/// A single-part location envelope: half-open `[start, end)` with a strand.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Env {
    pub(crate) start: i64,
    pub(crate) end: i64,
    pub(crate) strand: i32,
}

/// A CDS on the contig. `intervals` are its forward sub-spans (within `[0,L)`),
/// so an origin-crossing CDS keeps both pieces.
pub(crate) struct Cds {
    pub(crate) intervals: Vec<(i64, i64)>,
    pub(crate) strand: i32,
    /// A pseudo-CDS standing for a whole-contig nucleotide annotation: it can be the
    /// focus, but `--cds_range` never counts it as a neighbor.
    pub(crate) nucleic_acid: bool,
}

impl Cds {
    /// Lowest forward coordinate (ignores origin wrap).
    pub(crate) fn start(&self) -> i64 {
        self.intervals.iter().map(|x| x.0).min().unwrap_or(0)
    }
    /// Highest forward coordinate (ignores origin wrap).
    pub(crate) fn end(&self) -> i64 {
        self.intervals.iter().map(|x| x.1).max().unwrap_or(0)
    }
    /// Sort key matching Python's `location.stranded_start`: the gene's reading
    /// start — `end` of the covering arc on the reverse strand, else `start`.
    ///
    /// The arc's end is folded back only when it actually runs past the origin: a
    /// reverse-strand CDS ending exactly at the contig end has `stranded_start ==
    /// len`, not 0 (which sorted it ahead of every other CDS).
    pub(crate) fn sort_key(&self, l: i64) -> i64 {
        let (a, b) = min_covering_arc(&self.intervals, l);
        if self.strand == -1 {
            if b > l {
                b - l
            } else {
                b
            }
        } else {
            a.rem_euclid(l)
        }
    }
}

/// Minimal arc covering `intervals` on a circle of length `l` — the circle minus
/// its largest empty gap. Returns `(lo, hi)` unwrapped: `hi - lo` is the arc
/// length (`<= l`) and `hi` may exceed `l` when the arc crosses the origin.
/// Correctly collapses an origin-crossing gene (e.g. `[0,130)+[1842,2552)`) into a
/// single arc (`[1842, 2682)`).
fn min_covering_arc(intervals: &[(i64, i64)], l: i64) -> (i64, i64) {
    let mut segs: Vec<(i64, i64)> = intervals
        .iter()
        .map(|&(s, e)| if s <= e { (s, e) } else { (e, s) })
        .collect();
    segs.sort();
    let mut merged: Vec<(i64, i64)> = Vec::new();
    for (s, e) in segs {
        if let Some(last) = merged.last_mut() {
            if s <= last.1 {
                last.1 = last.1.max(e);
                continue;
            }
        }
        merged.push((s, e));
    }
    let n = merged.len();
    if n == 0 {
        return (0, 0);
    }
    if n == 1 {
        return (merged[0].0, merged[0].1);
    }
    // Largest circular gap between consecutive merged segments.
    let (mut max_gap, mut gap_after) = (-1i64, 0usize);
    for i in 0..n {
        let gap_start = merged[i].1;
        let gap_end = if i + 1 == n {
            merged[0].0 + l
        } else {
            merged[i + 1].0
        };
        let gap = gap_end - gap_start;
        if gap > max_gap {
            max_gap = gap;
            gap_after = i;
        }
    }
    let lo = merged[(gap_after + 1) % n].0;
    let mut hi = merged[gap_after].1;
    if hi <= lo {
        hi += l;
    }
    (lo, hi)
}

/// A gene arc `[a, b)` (b may exceed `l`) as forward sub-parts within `[0, l)`,
/// high part first then the wrapped low part — the order Python's
/// `circular_envelope` produces for an origin-crossing gene.
fn arc_to_forward_parts(a: i64, b: i64, l: i64) -> Vec<(i64, i64)> {
    if b <= l {
        vec![(a, b)]
    } else {
        vec![(a, l), (0, b - l)]
    }
}

/// Directed circular envelope of forward intervals taken **in connection order**
/// (left neighbor → focus → right neighbor), mirroring `circular_envelope` for the
/// all-forward case: each part is extended forward to the next, crossing the origin
/// when the next part lies behind it. Returns the covering arc as `(w_lo, w_hi)`
/// unwrapped (`w_hi - w_lo >= l` means the whole circle).
fn ordered_circular_envelope(parts: &[(i64, i64)], l: i64) -> (i64, i64) {
    let mut acc: Vec<(i64, i64)> = vec![parts[0]];
    for &(ns, ne) in &parts[1..] {
        let (ts, te) = *acc.last().unwrap();
        let touches = ns <= te && ts <= ne;
        if touches || ns >= te {
            let last = acc.last_mut().unwrap();
            *last = (ts.min(ns), te.max(ne));
        } else {
            // Next part lies behind the current end: cross the origin.
            acc.last_mut().unwrap().1 = l;
            acc.push((0, ne));
        }
    }
    acc.sort();
    let mut merged: Vec<(i64, i64)> = Vec::new();
    for (s, e) in acc {
        if let Some(last) = merged.last_mut() {
            if s <= last.1 {
                last.1 = last.1.max(e);
                continue;
            }
        }
        merged.push((s, e));
    }
    if merged.len() == 1 {
        return (merged[0].0, merged[0].1);
    }
    // Two parts -> origin-crossing arc [(0, x), (y, l)] => window [y, x + l).
    (merged[1].0, merged[0].1 + l)
}

/// Turn an unwrapped window `[w_lo, w_hi)` (length `<= l`) into a [`SliceLoc`]:
/// forward sub-parts within `[0, l)`, tagged `strand`, in reading order.
fn build_slice_from_window(w_lo: i64, w_hi: i64, l: i64, strand: i32) -> SliceLoc {
    let len = (w_hi - w_lo).min(l);
    let lo0 = w_lo.rem_euclid(l);
    let mut parts: Vec<Env> = Vec::new();
    let first_end = (lo0 + len).min(l);
    parts.push(Env {
        start: lo0,
        end: first_end,
        strand,
    });
    let rem = len - (first_end - lo0);
    if rem > 0 {
        parts.push(Env {
            start: 0,
            end: rem,
            strand,
        });
    }
    // Any **forward** window that wraps the origin -- not just the full circle --
    // stays in rotation order `[lo0, l) + [0, rem)`: that is the order the bases are
    // read in, and Python concatenates `location.parts` as listed. Sorting ascending
    // instead yields the same arc rotated to a different origin, and collapses
    // `stranded_start`/`stranded_end` to `0`/`l` so the coordinate suffix that marks
    // the new origin is dropped as well.
    //
    // A **reverse** wrapped window is the mirror image: each part is
    // reverse-complemented before concatenation, so reading the arc backwards needs
    // `[0, rem) + [lo0, l)` -- exactly the ascending order.
    let wrapped_forward = strand != -1 && parts.len() > 1;
    if !wrapped_forward {
        parts.sort_by_key(|p| p.start);
    }
    SliceLoc {
        parts,
        overall_strand: strand,
        dist_to_start: 0,
        dist_to_end: 0,
    }
}

/// Recompute a CDS feature's `cds_id` (1-based strand-aware coords), matching
/// `utils.get_cds_unique_name` so we can pair CDS features with the `cds_id`
/// qualifier carried on `Domainator`/`Domain_Search` features. Prefers an
/// existing `cds_id` qualifier (as Python does), else builds from the location.
pub(crate) fn cds_unique_name(feature: &SeqFeature) -> String {
    if let Some(cid) = feature.get_first("cds_id") {
        return cid.to_string();
    }
    let parts: Vec<String> = feature
        .location
        .parts
        .iter()
        .map(|&(s, e, strand, _, _)| {
            let (lo, hi) = if s <= e { (s, e) } else { (e, s) };
            if strand == -1 {
                format!("{}_{}_{}", hi, strand, lo + 1)
            } else {
                format!("{}_{}_{}", lo + 1, strand, hi)
            }
        })
        .collect();
    parts.join(" ")
}

/// Slice `[lo, hi)` of `contig`, reverse-complemented when `strand == -1`.
fn extract(contig: &[u8], lo: i64, hi: i64, strand: i32) -> Vec<u8> {
    let sub = &contig[to_index(lo)..to_index(hi)];
    if strand == -1 {
        revcomp(sub)
    } else {
        sub.to_vec()
    }
}

/// Pad a single-part `env` by `up`/`down` bases on a **linear** contig, clamped
/// to `[0, rec_len)`; returns the padded part and `(upstream_ext, downstream_ext)`.
fn pad_linear(env: Env, rec_len: i64, up: i64, down: i64) -> (Env, (i64, i64)) {
    if up == 0 && down == 0 {
        return (env, (0, 0));
    }
    if env.strand == -1 {
        let min_coord = (env.start - down).max(0);
        let down_ext = env.start - min_coord;
        let max_coord = (env.end + up).min(rec_len);
        let up_ext = max_coord - env.end;
        (
            Env {
                start: min_coord,
                end: max_coord,
                strand: env.strand,
            },
            (up_ext, down_ext),
        )
    } else {
        let min_coord = (env.start - up).max(0);
        let up_ext = env.start - min_coord;
        let max_coord = (env.end + down).min(rec_len);
        let down_ext = max_coord - env.end;
        (
            Env {
                start: min_coord,
                end: max_coord,
                strand: env.strand,
            },
            (up_ext, down_ext),
        )
    }
}

/// `pad_circular_cds`: indices of the left/right-most CDS in a circular CDS-range
/// window around `center`, dividing overlap when the window wraps the origin.
fn pad_circular_cds(center: i64, size: i64, left_pad: i64, right_pad: i64) -> (i64, i64) {
    let (mut left_pad, mut right_pad) = (left_pad, right_pad);
    if size == 1 {
        left_pad = 0;
        right_pad = 0;
    }
    let end_to_origin = size - (center + 1);
    let mut min_coord = center - left_pad;
    let mut max_coord = center + right_pad;
    if min_coord >= 0 && max_coord < size {
        return (min_coord, max_coord);
    }
    if min_coord < 0 && (-min_coord > end_to_origin) {
        min_coord = -end_to_origin;
    }
    if max_coord >= size && (max_coord - size >= center) {
        max_coord = size + (center - 1);
    }
    let min_shifted = 0;
    let mut max_shifted = max_coord - min_coord;
    let overlap = (max_shifted + 1) - size;
    let mut min_shifted = min_shifted;
    if overlap > 0 {
        min_shifted += overlap / 2;
        max_shifted -= overlap / 2;
        if overlap % 2 == 1 {
            max_shifted -= 1;
        }
    }
    max_coord = max_shifted + min_coord;
    min_coord += min_shifted;
    if min_coord < 0 {
        min_coord += size;
    }
    (min_coord, max_coord.rem_euclid(size))
}

/// Resolve `--cds_range`/`--cds_up`/`--cds_down`/`--kb_range`/`--kb_up`/`--kb_down`
/// into a [`Range`], mirroring `select_by_cds.validate_range`. `whole` overrides
/// to the whole-contig window. Returns an error on conflicting/negative options.
pub fn validate_range(
    whole: bool,
    kb_range: Option<f64>,
    kb_up: Option<f64>,
    kb_down: Option<f64>,
    cds_range: Option<i64>,
    cds_up: Option<i64>,
    cds_down: Option<i64>,
) -> Result<Range, String> {
    // `!(v >= 0.0)` also catches NaN; an infinite kb range would overflow the
    // coordinate math. The CLI rejects both earlier (`reject_non_finite`); this keeps
    // the library safe for other callers.
    let neg_f = |x: Option<f64>| x.is_some_and(|v| !(v >= 0.0 && v.is_finite()));
    let neg_i = |x: Option<i64>| x.is_some_and(|v| v < 0);
    if neg_f(kb_range) || neg_f(kb_up) || neg_f(kb_down) {
        return Err("kb_range must be >= 0".into());
    }
    if neg_i(cds_range) || neg_i(cds_up) || neg_i(cds_down) {
        return Err("cds_range must be >= 0".into());
    }

    let mut cds_seen = false;
    let mut cds_out = (0i64, 0i64);
    if cds_up.is_some() || cds_down.is_some() {
        cds_seen = true;
        cds_out = (cds_up.unwrap_or(0), cds_down.unwrap_or(0));
    }
    if let Some(c) = cds_range {
        if cds_seen {
            return Err("Cannot specify cds_range with cds_down or cds_up".into());
        }
        cds_seen = true;
        cds_out = (c, c);
    }

    let kb_present = kb_range.is_some() || kb_up.is_some() || kb_down.is_some();
    if cds_seen && kb_present {
        return Err("Cannot specify kb_range and cds_range".into());
    }

    let mut kb_seen = false;
    let mut kb_out = (0.0f64, 0.0f64);
    if kb_up.is_some() || kb_down.is_some() {
        kb_seen = true;
        kb_out = (kb_up.unwrap_or(0.0), kb_down.unwrap_or(0.0));
    }
    if let Some(k) = kb_range {
        if kb_seen {
            return Err("Cannot specify kb_range with kb_down or kb_up".into());
        }
        kb_seen = true;
        kb_out = (k, k);
    }

    if whole {
        return Ok(Range::Whole);
    }
    if cds_seen {
        Ok(Range::Cds(cds_out.0, cds_out.1))
    } else if kb_seen {
        Ok(Range::Kb(kb_out.0, kb_out.1))
    } else {
        Ok(Range::None)
    }
}

/// Result of computing a neighborhood: the ordered slice parts (each
/// `[start,end)` + strand) and the human-readable `(start, end)` for naming.
#[derive(Clone)]
pub(crate) struct SliceLoc {
    pub(crate) parts: Vec<Env>,
    pub(crate) overall_strand: i32,
    /// Upstream / downstream extension sizes (bp) of the focus CDS within the
    /// region, for `--pad` alignment. Only computed for linear contigs (0 for
    /// circular, where `--pad` is unsupported).
    pub(crate) dist_to_start: i64,
    pub(crate) dist_to_end: i64,
}

/// An extracted region plus where its focus CDS sits in it (what `pad_records`
/// aligns on; upstream's `dist_to_start`/`dist_to_end`/`focus_strand` record
/// annotations). Both `select_by_cds` and `domain_search`'s neighborhood path produce
/// these.
pub struct Region {
    pub record: SeqRecord,
    /// Bases from the start of the region to the start of the focus CDS's envelope.
    pub dist_to_start: i64,
    /// Bases from the end of the focus CDS's envelope to the end of the region.
    pub dist_to_end: i64,
    /// The focus CDS's strand in region coordinates: -1 only when `--keep_direction`
    /// kept a reverse-strand focus as it was.
    pub focus_strand: i32,
}

impl Region {
    /// A region whose focus is the whole record (a protein, or a translated peptide).
    pub fn whole(record: SeqRecord) -> Region {
        Region {
            record,
            dist_to_start: 0,
            dist_to_end: 0,
            focus_strand: 1,
        }
    }
}

/// Upstream `model.region_id`: the id of the region of `parent` that `parts` slices
/// out, reverse complemented afterwards if `flip`. Every tool that extracts a region
/// names it this way: `{id}_{first}:{last}`, plus `rc` when the region reads the
/// parent's reverse strand. `first`/`last` are the 1-based parent coordinates of the
/// region's first and last bases as written, so `first > last` means reversed or
/// wrapping the origin. The whole contig from its own origin drops the coordinates:
/// `{id}` forward, `{id}_rc` reverse. Protein regions are never reversed.
pub(crate) fn region_id(parent: &SeqRecord, parts: &[Env], flip: bool) -> String {
    let (head, tail) = (parts[0], parts[parts.len() - 1]);
    let mut first = if head.strand == -1 {
        head.end
    } else {
        head.start + 1
    };
    let mut last = if tail.strand == -1 {
        tail.start + 1
    } else {
        tail.end
    };
    let mut reverse = parent.molecule_type.as_deref() != Some("protein") && head.strand == -1;
    if flip {
        std::mem::swap(&mut first, &mut last);
        reverse = !reverse;
    }
    let contig_length = parent.seq.len() as i64;
    let length: i64 = parts.iter().map(|p| p.end - p.start).sum();
    let whole = if reverse {
        (contig_length, 1)
    } else {
        (1, contig_length)
    };
    let rc = if reverse { "rc" } else { "" };
    if length == contig_length && (first, last) == whole {
        if reverse {
            format!("{}_rc", parent.id)
        } else {
            parent.id.clone()
        }
    } else {
        format!("{}_{first}:{last}{rc}", parent.id)
    }
}

impl SliceLoc {
    /// Upstream `stranded_start` (0-based): where reading the slice begins, the first
    /// part's start forward and its exclusive end on the reverse strand.
    fn stranded_start(&self) -> i64 {
        let head = self.parts[0];
        if head.strand == -1 {
            head.end
        } else {
            head.start
        }
    }
}

/// Upstream `cds_range_members`: the indices into `cds_list` that `--cds_range` counts,
/// which are the protein-coding CDSs plus the focus itself, and the focus's position
/// among them. A nucleotide annotation's pseudo-CDS is not a gene, so it is never a
/// neighbor, though it can be the focus.
fn cds_range_members(cds_list: &[Cds], focus_idx: usize) -> (Vec<usize>, i64) {
    let members: Vec<usize> = (0..cds_list.len())
        .filter(|&i| i == focus_idx || !cds_list[i].nucleic_acid)
        .collect();
    let member_idx = members.partition_point(|&i| i < focus_idx) as i64;
    (members, member_idx)
}

/// Compute the slice location for the focus CDS at `focus_idx` in the
/// stranded-start-sorted `cds_list`. Mirrors `get_cds_neighborhood`.
///
/// `hit` is the focus's search hit (forward intervals), which a whole circular
/// contig is centered on instead of the CDS (`_from_domain_search`); the distances
/// stay to the CDS, so `--pad` still aligns CDSs.
pub(crate) fn cds_neighborhood(
    rec_len: i64,
    circular: bool,
    cds_list: &[Cds],
    focus_idx: usize,
    range: Range,
    hit: Option<&[(i64, i64)]>,
) -> SliceLoc {
    let focus = &cds_list[focus_idx];
    let strand = focus.strand;

    if !circular {
        let envelope = Env {
            start: focus.start(),
            end: focus.end(),
            strand,
        };
        let (mut up, mut down) = (0i64, 0i64);
        match range {
            Range::Whole => {
                up = rec_len;
                down = rec_len;
            }
            Range::Kb(u, d) => {
                up = (u * 1000.0) as i64;
                down = (d * 1000.0) as i64;
            }
            Range::Cds(cu, cd) => {
                let (members, member_idx) = cds_range_members(cds_list, focus_idx);
                let n = members.len() as i64;
                let (left_cds, right_cds) = if strand == -1 {
                    ((member_idx - cd).max(0), (member_idx + cu).min(n - 1))
                } else {
                    ((member_idx - cu).max(0), (member_idx + cd).min(n - 1))
                };
                let left_coord = cds_list[members[to_index(left_cds)]].start();
                let right_coord = cds_list[members[to_index(right_cds)]].end();
                let left_pad = (focus.start() - left_coord).max(0);
                let right_pad = (right_coord - focus.end()).max(0);
                if strand == -1 {
                    up = right_pad;
                    down = left_pad;
                } else {
                    up = left_pad;
                    down = right_pad;
                }
            }
            Range::None => {}
        }
        let (padded, (up_ext, down_ext)) = pad_linear(envelope, rec_len, up, down);
        return SliceLoc {
            parts: vec![padded],
            overall_strand: strand,
            dist_to_start: up_ext,
            dist_to_end: down_ext,
        };
    }

    // Circular topology. Work with the gene's covering arc [A, B) (B may exceed L
    // when the gene crosses the origin) and compute an unwrapped window [w_lo, w_hi).
    let l = rec_len;
    let (a, b) = min_covering_arc(&focus.intervals, l);
    let (w_lo, w_hi) = match range {
        Range::None => (a, b),
        Range::Kb(u, d) => {
            pad_circular_arc(a, b, l, strand, (u * 1000.0) as i64, (d * 1000.0) as i64)
        }
        Range::Cds(cu, cd) => {
            let (members, member_idx) = cds_range_members(cds_list, focus_idx);
            let n = members.len() as i64;
            let (li, ri) = if strand == -1 {
                pad_circular_cds(member_idx, n, cd, cu)
            } else {
                pad_circular_cds(member_idx, n, cu, cd)
            };
            let (li, ri) = (members[to_index(li)], members[to_index(ri)]);
            // Window = directed circular envelope of the left neighbor, focus, and
            // right neighbor gene arcs, extended in that order (matching Python's
            // `left_env + focus_env + right_env` then `circular_envelope`).
            let arc = |c: &Cds| {
                let (ca, cb) = min_covering_arc(&c.intervals, l);
                arc_to_forward_parts(ca, cb, l)
            };
            let mut nbhd: Vec<(i64, i64)> = Vec::new();
            nbhd.extend(arc(&cds_list[li]));
            nbhd.extend(arc_to_forward_parts(a, b, l));
            nbhd.extend(arc(&cds_list[ri]));
            ordered_circular_envelope(&nbhd, l)
        }
        Range::Whole => (a, b), // replaced below
    };

    // Whole contig (or a CDS neighborhood covering the full circle): the whole circle,
    // split at the midpoint of the gap opposite the focus (or its hit). A kb window has
    // already been fitted to the circle by `pad_circular_arc`.
    let is_whole =
        matches!(range, Range::Whole) || (matches!(range, Range::Cds(..)) && (w_hi - w_lo) >= l);
    let mut slice = if is_whole {
        let (ca, cb) = match hit {
            Some(hit) if matches!(range, Range::Whole) => min_covering_arc(hit, l),
            _ => (a, b),
        };
        let split = ((ca + cb + l) / 2).rem_euclid(l);
        build_slice_from_window(split, split + l, l, strand)
    } else {
        build_slice_from_window(w_lo, w_hi, l, strand)
    };
    (slice.dist_to_start, slice.dist_to_end) = circular_focus_distances(&slice, (a, b), l);
    slice
}

/// Pad the focus arc `[a, b)` (`b` past `l` when it crosses the origin) by `pu` bases
/// upstream and `pd` downstream on a circular contig of length `l`: upstream
/// `pad_location`'s circular branch, as the window `[w_lo, w_hi)` it covers.
///
/// Padding never reads back into the focus. A pad that would is cut where it meets the
/// focus, and when the two pads then overlap each other the overlap is split between
/// them, so a window larger than the contig still starts and ends where its own pads
/// put it: `--kb_down 100` on a small plasmid starts at the focus, it is not centered
/// on it. The odd base of an uneven split comes off the high end for a focus that does
/// not cross the origin and off the downstream pad for one that does, as upstream's two
/// branches take it.
fn pad_circular_arc(a: i64, b: i64, l: i64, strand: i32, pu: i64, pd: i64) -> (i64, i64) {
    if pu == 0 && pd == 0 {
        return (a, b);
    }
    if b > l {
        // The focus crosses the origin, so the padding cannot.
        if b - a >= l {
            return (a, b);
        }
        let remaining = l - (b - a);
        let (mut pu, mut pd) = (pu.min(remaining), pd.min(remaining));
        let excess = pu + pd - remaining;
        if excess > 0 {
            pu -= excess / 2;
            pd -= excess / 2;
            if excess % 2 == 1 {
                pd -= 1;
            }
        }
        return if strand == -1 {
            (a - pd, b + pu)
        } else {
            (a - pu, b + pd)
        };
    }
    let (mut lo, mut hi) = if strand == -1 {
        (a - pd, b + pu)
    } else {
        (a - pu, b + pd)
    };
    if lo >= 0 && hi <= l {
        return (lo, hi);
    }
    // Neither side may run past the origin into the focus.
    if lo < 0 && -lo > l - b {
        lo = b - l;
    }
    if hi > l && hi - l > a {
        hi = l + a;
    }
    let overlap = hi - lo - l;
    if overlap > 0 {
        lo += overlap / 2;
        hi -= overlap / 2;
        if overlap % 2 == 1 {
            hi -= 1;
        }
    }
    (lo, hi)
}

/// Upstream `circular_focus_distances`: the distances, in the slice's direction,
/// from the start of a circular slice to the start of the focus arc `[a, b)`, and
/// from the end of the focus to the end of the slice.
fn circular_focus_distances(slice: &SliceLoc, (a, b): (i64, i64), l: i64) -> (i64, i64) {
    let circular_dist = |start: i64, end: i64| (end - start).rem_euclid(l);
    // The slice's stranded start is a 0-based start forward and an exclusive end on
    // the reverse strand, as the focus's `b` is.
    let dist_to_start = if slice.overall_strand == -1 {
        circular_dist(b, slice.stranded_start())
    } else {
        circular_dist(slice.stranded_start(), a)
    };
    let len: i64 = slice.parts.iter().map(|p| p.end - p.start).sum();
    (dist_to_start, len - dist_to_start - (b - a))
}

/// Slice `rec` to `slice`, keeping `keep_features` truncated to it, remapping
/// coordinates. Port of `slice_record_from_location` (upstream `8bf3c42`, where
/// `truncate_features=True` became the only behavior any tool uses); the feature
/// rules are [`slice_features`]'s.
pub(crate) fn slice_record(
    rec: &SeqRecord,
    slice: &SliceLoc,
    keep_features: &[SeqFeature],
) -> SeqRecord {
    // Build sequence and per-part coordinate shifts.
    let mut seq = Vec::new();
    let mut shifts = Vec::with_capacity(slice.parts.len());
    let mut acc = 0i64;
    for p in &slice.parts {
        shifts.push(acc);
        seq.extend(extract(&rec.seq, p.start, p.end, p.strand));
        acc += p.end - p.start;
    }
    let new_features = slice_features(keep_features, &slice.parts, &shifts, rec.is_protein());

    let annotations = slice_annotations(rec);

    // Do NOT sort the features here: `slice_record_from_location` preserves the
    // input order so the GenBank writer can rebuild the `domainator_<db>` rollup
    // from that order before it sorts features by start for output.
    SeqRecord {
        id: rec.id.clone(),
        name: rec.name.clone(),
        description: rec.description.clone(),
        seq,
        molecule_type: rec.molecule_type.clone(),
        topology: Topology::Linear,
        annotations,
        features: new_features,
    }
}

/// `get_cds_neighborhood` keeps a neighborhood that is the whole of a circular
/// contig (rotated to start at its focus) circular; every other slice is linear.
pub(crate) fn neighborhood_record(
    rec: &SeqRecord,
    slice: &SliceLoc,
    keep_features: &[SeqFeature],
) -> SeqRecord {
    let mut region = slice_record(rec, slice, keep_features);
    let len: i64 = slice.parts.iter().map(|p| p.end - p.start).sum();
    if rec.topology == Topology::Circular && len == rec.seq.len() as i64 {
        region.topology = Topology::Circular;
        region.annotations.topology = Some(Topology::Circular);
    }
    region
}

/// The annotations a sliced record keeps. `slice_record_from_location` keeps only
/// molecule_type, source, organism, taxonomy and `dbxrefs` (DBLINK), plus the
/// accession identity for ACCESSION/VERSION; everything else (keywords, division,
/// date, comment, references, structured_comment, ...) is dropped so the writer
/// emits defaults -- and stamps today's date -- matching the Python slice.
pub(crate) fn slice_annotations(rec: &SeqRecord) -> crate::seq::Annotations {
    let src = &rec.annotations;
    crate::seq::Annotations {
        accessions: src.accessions.clone(),
        sequence_version: src.sequence_version,
        source: src.source.clone(),
        organism: src.organism.clone(),
        taxonomy: src.taxonomy.clone(),
        dblink: src.dblink.clone(),
        ..Default::default()
    }
}

/// The features of a forward linear slice `[start, end)` of a record: what
/// `utils.slice_record(r, start, stop)` keeps. For the tools (`trim_contigs`,
/// `clean_sequences --strip`) that build the rest of the record themselves.
pub(crate) fn slice_linear_features(rec: &SeqRecord, start: i64, end: i64) -> Vec<SeqFeature> {
    let part = Env {
        start,
        end,
        strand: 1,
    };
    slice_features(&rec.features, &[part], &[0], rec.is_protein())
}

/// One kept piece of a feature: the overlap of one feature part with one slice part.
struct Piece {
    /// The piece's interval in coordinates along the feature, 5' to 3'.
    transcript: (i64, i64),
    /// The feature part it came from, normalized `(start, end, strand, before, after)`.
    part: (i64, i64, i32, bool, bool),
    /// Index of the slice part it falls in.
    slice_part: usize,
    kept_start: i64,
    kept_end: i64,
}

/// Merge touching or overlapping `(start, end)` intervals, sorted by start
/// (`utils._merge_intervals`).
fn merge_intervals(mut intervals: Vec<(i64, i64)>) -> Vec<(i64, i64)> {
    intervals.sort_unstable();
    let mut merged: Vec<(i64, i64)> = Vec::with_capacity(intervals.len());
    for (s, e) in intervals {
        match merged.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => merged.push((s, e)),
        }
    }
    merged
}

/// Remap `features` into a slice made of `parts` (concatenated in order, each
/// starting at `shifts[i]` in the new record), the feature half of
/// `slice_record_from_location`:
///
/// - a feature is kept if any part of it overlaps the slice, truncated to the slice;
/// - an end the slice really cuts gets a new `<`/`>`, except on `source` features,
///   and the ends of the original feature keep their own markers. "Really cut" is
///   decided on intervals along the feature, so a feature split across the ends of
///   a rotated circular contig -- all of it still in the record -- is not marked;
/// - a cut `Domainator`/`Domain_Search` feature is dropped, since its score and
///   coordinates describe the whole hit;
/// - a cut CDS on a nucleotide record gets an updated `/codon_start` and a trimmed
///   (or removed) `/translation`, see [`truncate_cds_qualifiers`].
///
/// Pieces are listed in order along the feature and merged if they touch (`join` /
/// single-part locations only); the original operator is kept.
pub(crate) fn slice_features(
    features: &[SeqFeature],
    parts: &[Env],
    shifts: &[i64],
    is_protein: bool,
) -> Vec<SeqFeature> {
    let mut new_features = Vec::new();
    for f in features {
        let is_source = f.kind == "source";
        let is_hit = f.kind == DOMAIN_FEATURE_NAME || f.kind == DOMAIN_SEARCH_BEST_HIT_NAME;

        let mut pieces: Vec<Piece> = Vec::new();
        let mut transcript_len = 0i64;
        for &(s0, e0, fstrand, before, after) in &f.location.parts {
            let (fs, fe) = if s0 <= e0 { (s0, e0) } else { (e0, s0) };
            // Deliberately every slice part, not the first match: when the window
            // crosses the origin of a circular contig one feature part can reach into
            // more than one of them, and Python emits a piece per match.
            for (li, lp) in parts.iter().enumerate() {
                let contained = lp.start <= fs && lp.end >= fe;
                let overlaps = fs.max(lp.start) < fe.min(lp.end);
                if !(contained || overlaps) {
                    continue;
                }
                let kept_start = fs.max(lp.start);
                let kept_end = fe.min(lp.end);
                let transcript = if fstrand == -1 {
                    (
                        transcript_len + fe - kept_end,
                        transcript_len + fe - kept_start,
                    )
                } else {
                    (
                        transcript_len + kept_start - fs,
                        transcript_len + kept_end - fs,
                    )
                };
                pieces.push(Piece {
                    transcript,
                    part: (fs, fe, fstrand, before, after),
                    slice_part: li,
                    kept_start,
                    kept_end,
                });
            }
            transcript_len += fe - fs;
        }

        let kept = merge_intervals(pieces.iter().map(|p| p.transcript).collect());
        let is_cut = kept.iter().map(|(s, e)| e - s).sum::<i64>() < transcript_len;
        if is_cut && is_hit {
            continue;
        }
        let cut_5p = |t: i64| kept.iter().any(|&(s, _)| s == t && s > 0);
        let cut_3p = |t: i64| kept.iter().any(|&(_, e)| e == t && e < transcript_len);

        // In order along the feature (stable, like Python's `sorted`).
        pieces.sort_by_key(|p| p.transcript);
        let mut new_parts: Vec<(i64, i64, i32, bool, bool)> = Vec::with_capacity(pieces.len());
        for p in &pieces {
            let (fs, fe, fstrand, before, after) = p.part;
            let (t_start, t_end) = p.transcript;
            let (start_is_cut, end_is_cut) = if fstrand == -1 {
                (cut_3p(t_end), cut_5p(t_start))
            } else {
                (cut_5p(t_start), cut_3p(t_end))
            };
            // Ends of the original feature keep their position type (an existing `<`
            // or `>`); ends that are really cut get a new one, except on sources.
            let new_before = if p.kept_start == fs {
                before
            } else {
                start_is_cut && !is_source
            };
            let new_after = if p.kept_end == fe {
                after
            } else {
                end_is_cut && !is_source
            };
            let lp = parts[p.slice_part];
            let (mut a, mut b) = (p.kept_start - lp.start, p.kept_end - lp.start);
            let (mut st, mut lo_flag, mut hi_flag) = (fstrand, new_before, new_after);
            if lp.strand == -1 {
                // Flip within this part; the markers swap ends with the coordinates.
                let part_len = lp.end - lp.start;
                (a, b) = (part_len - b, part_len - a);
                (lo_flag, hi_flag) = (hi_flag, lo_flag);
                st = -st;
            }
            new_parts.push((
                a + shifts[p.slice_part],
                b + shifts[p.slice_part],
                st,
                lo_flag,
                hi_flag,
            ));
        }
        // Coalesce the pieces slicing split apart, mirroring
        // `CompoundLocation.merge_parts`: an origin-crossing window cuts a feature at
        // the junction, and the two halves come back abutting (`[0:130]` +
        // `[130:840]`); without this they stay a spurious `join`. Python gates the
        // merge on the original operator, so `order` locations keep their parts apart.
        if matches!(f.location.operator, None | Some(CompoundOp::Join)) {
            merge_touching_parts(&mut new_parts);
        }
        if new_parts.is_empty() {
            continue;
        }
        let operator = if new_parts.len() > 1 {
            Some(f.location.operator.unwrap_or(CompoundOp::Join))
        } else {
            None
        };
        let qualifiers = if is_cut && f.kind == "CDS" && !is_protein {
            truncate_cds_qualifiers(f, &kept)
        } else {
            f.qualifiers.clone()
        };
        new_features.push(SeqFeature {
            kind: f.kind.clone(),
            location: Location {
                operator,
                between: false,
                reference: None,
                parts: new_parts,
            },
            qualifiers,
        });
    }
    new_features
}

/// The qualifiers of a CDS that slicing cut (`utils._truncate_cds_qualifiers`).
/// `kept` is the kept part of the CDS as merged intervals along it, 5' to 3'.
///
/// `/codon_start` is updated so the reading frame is preserved (written when it is
/// not 1, or was already there), and `/translation` is trimmed to the residues whose
/// codons are completely kept -- the NCBI convention for a partial CDS. If the kept
/// part is not contiguous (an interior exon was cut out), or the translation cannot
/// be placed on the CDS, `/translation` is removed, to be re-translated later.
fn truncate_cds_qualifiers(
    feature: &SeqFeature,
    kept: &[(i64, i64)],
) -> Vec<(String, Vec<String>)> {
    let mut out = feature.clone();
    let offset = codon_offset(feature);
    let (kept_start, kept_end) = (kept[0].0, kept[kept.len() - 1].1);
    let new_codon_start = (offset - kept_start).rem_euclid(3) + 1;
    if new_codon_start != 1 || out.has("codon_start") {
        out.set("codon_start", new_codon_start.to_string());
    }

    let mut translation = Some(feature.get_first("translation").unwrap_or("").to_string());
    if translation.as_deref().is_some_and(|t| !t.is_empty()) {
        let (five, three) = location_partial_ends(&feature.location);
        translation = on_contig_translation(
            translation.as_deref().unwrap_or(""),
            feature.location.len(),
            offset,
            five,
            three,
        );
    }
    match translation {
        Some(t) if kept.len() == 1 => {
            if !t.is_empty() {
                // First codon starting at or after kept_start; codons ending at or
                // before kept_end.
                let first = (-(-(kept_start - offset)).div_euclid(3)).max(0);
                let last = (kept_end - offset).div_euclid(3).max(0);
                let chars: Vec<char> = t.chars().collect();
                let clamp = |r: i64| usize::try_from(r).unwrap_or(0).min(chars.len());
                let (first, last) = (clamp(first), clamp(last));
                let trimmed: String = if first < last {
                    chars[first..last].iter().collect()
                } else {
                    String::new()
                };
                out.set("translation", trimmed);
            }
        }
        _ => out.qualifiers.retain(|(k, _)| k != "translation"),
    }
    out.qualifiers
}

/// Extract neighborhoods around every hit CDS in `rec` (a nucleotide contig that
/// came out of `domainate(hits_only, best_annotation)`). Mirrors the
/// `_from_domain_search` branch of `select_by_cds`.
pub fn select_by_cds_hits(
    rec: &SeqRecord,
    range: Range,
    normalize_direction: bool,
    strand_filter: Option<i32>,
    max_region_overlap: f64,
    decoys: Option<&std::collections::HashSet<String>>,
    max_hits_per_contig: Option<usize>,
) -> std::io::Result<Vec<Region>> {
    let rec_len = rec.seq.len() as i64;
    let circular = rec.topology == Topology::Circular;

    // Upstream's `select_by_cds` as `domain_search` calls it: every CDS, plus a
    // pseudo-CDS for each whole-contig (`cds_id="."`) annotation, `Domain_Search` and
    // `Domainator` alike, sorted together by stranded start. That order is the order
    // features are emitted in; `--cds_range` counts only the real CDSs among them
    // (`cds_range_members`). A focus is an entry carrying a `Domain_Search` feature
    // (upstream's `cds.index in hit_scores`).
    let mut paired: Vec<(DomainatorCds, Cds)> =
        DomainatorCds::list_from_contig(rec, None, f64::NEG_INFINITY, false, None, true)
            .into_iter()
            .map(|d| {
                let nb = Cds {
                    intervals: forward_intervals(&d.feature),
                    strand: d.feature.location.strand(),
                    nucleic_acid: d.is_nucleic_acid,
                };
                (d, nb)
            })
            .collect();
    paired.sort_by_key(|(_, nb)| nb.sort_key(rec_len.max(1)));
    let (dom_list, cds_list): (Vec<DomainatorCds>, Vec<Cds>) = paired.into_iter().unzip();

    let non_dom: Vec<SeqFeature> = get_non_domainator_features(rec)
        .into_iter()
        .cloned()
        .collect();

    let mut out: Vec<Region> = Vec::new();
    let mut selected: Vec<SliceLoc> = Vec::new();

    for focus_idx in 0..cds_list.len() {
        let cds = &cds_list[focus_idx];
        let Some(search_hit) = dom_list[focus_idx].domain_search_feature.as_ref() else {
            continue;
        };
        if let Some(sf) = strand_filter {
            if (sf == 1 && cds.strand == -1) || (sf == -1 && cds.strand != -1) {
                continue;
            }
        }
        // Decoy filter: skip focus CDSs whose best hit is a decoy domain.
        if let Some(decoys) = decoys {
            if search_hit
                .get_first("name")
                .is_some_and(|n| decoys.contains(n))
            {
                continue;
            }
        }

        let hit = forward_intervals(search_hit);
        let slice = cds_neighborhood(rec_len, circular, &cds_list, focus_idx, range, Some(&hit));

        // Features in upstream's `neighborhood_feature_index` order: non-Domainator
        // features, then each sorted entry's Domainator features followed by its
        // Domain_Search feature, which only the focus keeps. The writer's sort by start
        // is stable, so this order decides features that share a start. `slice_record`
        // drops the hit features the region cuts, and those outside it (upstream
        // `8bf3c42` removed `get_cds_neighborhood`'s containment pre-filter).
        let mut keep: Vec<SeqFeature> = non_dom.clone();
        for (i, entry) in dom_list.iter().enumerate() {
            keep.extend(entry.domain_features.iter().cloned());
            if i == focus_idx {
                keep.push(search_hit.clone());
            }
        }

        let mut region = neighborhood_record(rec, &slice, &keep);

        let strand = slice.overall_strand;
        let flip = !normalize_direction && strand == -1;
        if flip {
            // Flip back to the original orientation.
            region = reverse_complement_record(&region);
        }
        // The id names the region; ACCESSION/VERSION keep the contig's.
        region.id = region_id(rec, &slice.parts, flip);

        // The focus position travels with the region so `--pad` can align the focus
        // CDSs after the whole search completes. Read before `slice` can be moved into `selected`
        // below, and swapped on an un-normalized reverse region, which was flipped
        // back to the original orientation above.
        let (dist_to_start, dist_to_end) = if flip {
            (slice.dist_to_end, slice.dist_to_start)
        } else {
            (slice.dist_to_start, slice.dist_to_end)
        };
        if max_region_overlap < 1.0 {
            let loc_len: i64 = slice.parts.iter().map(|p| p.end - p.start).sum();
            let mut drop = false;
            for other in &selected {
                if (overlap_size(&slice, other) as f64 / loc_len as f64) > max_region_overlap {
                    drop = true;
                    break;
                }
            }
            if drop {
                continue;
            }
            selected.push(slice);
        }
        out.push(Region {
            record: region,
            dist_to_start,
            dist_to_end,
            focus_strand: if flip { -1 } else { 1 },
        });
        if max_hits_per_contig.is_some_and(|m| out.len() >= m) {
            break;
        }
    }
    Ok(out)
}

/// A feature's parts as `(low, high)` intervals.
fn forward_intervals(f: &SeqFeature) -> Vec<(i64, i64)> {
    f.location
        .parts
        .iter()
        .map(|&(s, e, ..)| if s <= e { (s, e) } else { (e, s) })
        .collect()
}

/// CDS display name: first present of `protein_id`/`gene_id`/`locus_tag`/`gene`,
/// else the `cds_id`. Mirrors `utils.get_cds_name` with `extract_peptides`'
/// precedence.
fn get_cds_name(f: &SeqFeature) -> String {
    for key in ["protein_id", "gene_id", "locus_tag", "gene"] {
        if let Some(v) = f.get_first(key) {
            return v.to_string();
        }
    }
    cds_unique_name(f)
}

/// `--translate`: emit each hit CDS as a translated peptide record. Mirrors the
/// `extract_peptides(_from_domain_search=True)` branch of `domain_search`. The
/// peptide uses the CDS `/translation` qualifier when present, else a table-1
/// translation; features are the contig's `source` features, the focus
/// `Domain_Search` feature, the CDS's `Domainator` features, and the CDS itself
/// (with `/translation` dropped and a `source_contig` qualifier), remapped to
/// peptide coordinates.
pub fn translate_peptides(
    rec: &SeqRecord,
    strand_filter: Option<i32>,
    decoys: Option<&std::collections::HashSet<String>>,
) -> Vec<SeqRecord> {
    use crate::seq::translate::translate_dna;

    let sources: Vec<&SeqFeature> = rec.features.iter().filter(|f| f.kind == "source").collect();
    let mut search_by: std::collections::HashMap<String, &SeqFeature> =
        std::collections::HashMap::new();
    let mut dom_by: std::collections::HashMap<String, Vec<&SeqFeature>> =
        std::collections::HashMap::new();
    for f in &rec.features {
        if let Some(cid) = f.get_first("cds_id") {
            if f.kind == DOMAIN_SEARCH_BEST_HIT_NAME {
                search_by.insert(cid.to_string(), f);
            } else if f.kind == DOMAIN_FEATURE_NAME {
                dom_by.entry(cid.to_string()).or_default().push(f);
            }
        }
    }

    let mut out: Vec<SeqRecord> = Vec::new();
    for f in &rec.features {
        if f.kind != "CDS" {
            continue;
        }
        let cid = cds_unique_name(f);
        let Some(search) = search_by.get(&cid) else {
            continue;
        };
        // A CDS whose best hit is a decoy is not a hit.
        if decoys.is_some_and(|d| search.get_first("name").is_some_and(|n| d.contains(n))) {
            continue;
        }
        let strand = f.location.strand();
        if let Some(sf) = strand_filter {
            if strand != sf {
                continue;
            }
        }

        // Slice the contig to the CDS (coding orientation), carrying the features.
        let parts: Vec<Env> = f
            .location
            .parts
            .iter()
            .map(|&(s, e, st, _, _)| {
                let (lo, hi) = if s <= e { (s, e) } else { (e, s) };
                Env {
                    start: lo,
                    end: hi,
                    strand: st,
                }
            })
            .collect();
        let slice = SliceLoc {
            parts,
            overall_strand: strand,
            dist_to_start: 0,
            dist_to_end: 0,
        };

        let mut cds_feat = f.clone();
        cds_feat
            .qualifiers
            .retain(|(k, _)| k != "translation" && k != "codon_start");
        // `source_contig` identifies the original LOCUS.
        cds_feat.set("source_contig", rec.id.clone());
        let mut keep: Vec<SeqFeature> = sources.iter().map(|x| (*x).clone()).collect();
        keep.push((*search).clone());
        if let Some(ds) = dom_by.get(&cid) {
            keep.extend(ds.iter().map(|d| (*d).clone()));
        }
        keep.push(cds_feat);

        let mut region = slice_record(rec, &slice, &keep);

        // Peptide sequence: the CDS `/translation` if present, else translate from
        // its reading frame.
        let frame = codon_offset(f);
        let pep = match f.get_first("translation") {
            Some(t) if !t.is_empty() => t.to_string(),
            _ => translate_dna(
                &region.seq[usize::try_from(frame).unwrap_or(0).min(region.seq.len())..],
            ),
        };
        region.seq = pep.into_bytes();
        region.molecule_type = Some("protein".to_string());
        // The peptide is named for the CDS, with no carried accession identity.
        let name = get_cds_name(f);
        region.id = name.clone();
        region.name = name;
        region.annotations.accessions.clear();
        region.annotations.sequence_version = None;
        crate::tools::extract_common::dna_to_peptide_features(&mut region.features, frame);
        out.push(region);
    }
    out
}

/// Total overlap (bp) between two slice locations.
pub(crate) fn overlap_size(a: &SliceLoc, b: &SliceLoc) -> i64 {
    let mut total = 0;
    for pa in &a.parts {
        for pb in &b.parts {
            total += (pa.end.min(pb.end) - pa.start.max(pb.start)).max(0);
        }
    }
    total
}

/// Reverse-complement a record in place (sequence + feature coordinates).
pub(crate) fn reverse_complement_record(rec: &SeqRecord) -> SeqRecord {
    let n = rec.seq.len() as i64;
    let seq = revcomp(&rec.seq);
    let mut features: Vec<SeqFeature> = rec
        .features
        .iter()
        .map(|f| {
            let parts = f
                .location
                .parts
                .iter()
                .map(|&(s, e, st, before, after)| {
                    let (lo, hi) = if s <= e { (s, e) } else { (e, s) };
                    (n - hi, n - lo, -st, after, before)
                })
                .collect();
            let operator = f.location.operator;
            SeqFeature {
                kind: f.kind.clone(),
                location: Location {
                    operator,
                    between: f.location.between,
                    reference: None,
                    parts,
                },
                qualifiers: f.qualifiers.clone(),
            }
        })
        .collect();
    features.sort_by_key(|f| f.location.start());
    SeqRecord {
        seq,
        features,
        ..rec.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pad_circular_cds_cases() {
        // Non-wrapping window.
        assert_eq!(pad_circular_cds(2, 5, 1, 1), (1, 3));
        assert_eq!(pad_circular_cds(0, 5, 0, 0), (0, 0));
        // A single CDS cannot be padded.
        assert_eq!(pad_circular_cds(0, 1, 5, 5), (0, 0));
        // Window wraps the origin to the left.
        assert_eq!(pad_circular_cds(0, 5, 1, 0), (4, 0));
    }

    #[test]
    fn validate_range_resolution() {
        assert!(matches!(
            validate_range(true, None, None, None, None, None, None),
            Ok(Range::Whole)
        ));
        assert!(matches!(
            validate_range(false, None, None, None, Some(2), None, None),
            Ok(Range::Cds(2, 2))
        ));
        assert!(matches!(
            validate_range(false, None, None, None, None, Some(1), None),
            Ok(Range::Cds(1, 0))
        ));
        assert!(matches!(
            validate_range(false, Some(0.5), None, None, None, None, None),
            Ok(Range::Kb(a, b)) if a == 0.5 && b == 0.5
        ));
        assert!(matches!(
            validate_range(false, None, None, None, None, None, None),
            Ok(Range::None)
        ));
        // Conflicts / negatives are errors.
        assert!(validate_range(false, None, None, None, Some(2), Some(1), None).is_err());
        assert!(validate_range(false, Some(1.0), None, None, Some(2), None, None).is_err());
        for bad in [f64::NAN, f64::INFINITY, -1.0] {
            assert!(validate_range(false, Some(bad), None, None, None, None, None).is_err());
            assert!(validate_range(false, None, None, Some(bad), None, None, None).is_err());
        }
        assert!(validate_range(false, None, None, None, Some(-1), None, None).is_err());
    }

    fn one_cds(start: i64, end: i64, strand: i32) -> Vec<Cds> {
        vec![Cds {
            intervals: vec![(start, end)],
            strand,
            nucleic_acid: false,
        }]
    }

    /// A whole circular contig is split opposite its focus, and the focus distances
    /// (upstream `circular_focus_distances`) say where the CDS sits in it, so `--pad`
    /// can align circular regions too.
    #[test]
    fn a_whole_circular_slice_records_where_its_focus_is() {
        for strand in [1, -1] {
            let slice =
                cds_neighborhood(100, true, &one_cds(10, 20, strand), 0, Range::Whole, None);
            let expected = if strand == 1 { "c_66:65" } else { "c_65:66rc" };
            assert_eq!(id_of(&slice, 100), expected);
            assert_eq!(
                (slice.dist_to_start, slice.dist_to_end),
                (45, 45),
                "strand {strand}"
            );
        }
    }

    /// From `domain_search`, a whole circular contig is centered on the search hit,
    /// not its CDS, while the distances stay to the CDS.
    #[test]
    fn a_whole_circular_hit_region_centers_on_the_hit() {
        let hit = [(16, 20)];
        let slice = cds_neighborhood(100, true, &one_cds(10, 20, 1), 0, Range::Whole, Some(&hit));
        assert_eq!(id_of(&slice, 100), "c_69:68");
        assert_eq!((slice.dist_to_start, slice.dist_to_end), (42, 48));
        // Only the whole contig is centered on the hit; a window stays on the CDS.
        let window = cds_neighborhood(
            100,
            true,
            &one_cds(10, 20, 1),
            0,
            Range::Kb(0.005, 0.005),
            Some(&hit),
        );
        assert_eq!(id_of(&window, 100), "c_6:25");
        assert_eq!((window.dist_to_start, window.dist_to_end), (5, 5));
    }

    /// A kb window larger than a circular contig keeps the side its pads put it on:
    /// `--kb_down 100` around pDONR201's reverse-strand ccdB (`complement(959..1264)`)
    /// starts at ccdB, as upstream's `pad_location` cuts the pad where it would run back
    /// into the focus. It used to be centered opposite the gene.
    #[test]
    fn an_oversized_kb_window_is_cut_at_the_focus() {
        let cds = one_cds(958, 1264, -1);
        let slice = cds_neighborhood(4470, true, &cds, 0, Range::Kb(0.0, 100.0), None);
        assert_eq!(id_of(&slice, 4470), "c_1264:1265rc");
    }

    /// `pad_circular_arc` against upstream `pad_location` on a circle: `(length, a, b,
    /// strand, pad_up, pad_down)` -> the window's start (mod length) and size, from
    /// Python. Covers both branches (a focus crossing the origin or not), both
    /// strands, and pads that overlap by an odd and an even amount.
    #[test]
    fn circular_padding_matches_upstream() {
        type Case = (i64, i64, i64, i32, i64, i64, i64, i64);
        let cases: &[Case] = &[
            (50, 38, 62, 1, 130, 0, 12, 50),
            (80, 29, 100, -1, 200, 70, 25, 80),
            (101, 81, 111, 1, 19, 104, 71, 101),
            (119, 97, 118, 1, 75, 99, 59, 119),
            (80, 50, 105, -1, 0, 0, 50, 55),
            (37, 16, 30, -1, 27, 86, 4, 37),
            (96, 52, 78, 1, 37, 0, 15, 63),
            (58, 50, 51, -1, 26, 106, 6, 58),
            (95, 81, 89, 1, 80, 45, 20, 95),
            (52, 37, 57, -1, 0, 23, 14, 43),
            (117, 76, 185, -1, 33, 0, 76, 117),
            (92, 64, 104, 1, 28, 0, 36, 68),
            (89, 8, 38, -1, 0, 0, 8, 30),
            (35, 15, 26, 1, 0, 0, 15, 11),
            (108, 54, 157, 1, 0, 3, 54, 106),
            (105, 56, 71, -1, 0, 67, 94, 82),
        ];
        for &(l, a, b, strand, pu, pd, lo, n) in cases {
            let (w0, w1) = pad_circular_arc(a, b, l, strand, pu, pd);
            assert_eq!(
                (w0.rem_euclid(l), w1 - w0),
                (lo, n),
                "{l} {a} {b} {strand} {pu} {pd}"
            );
        }
    }

    #[test]
    fn min_covering_arc_origin_crossing() {
        // An origin-crossing gene [0,130)+[1842,2552) on a length-2552 circle
        // collapses to a single arc [1842, 2682).
        assert_eq!(
            min_covering_arc(&[(0, 130), (1842, 2552)], 2552),
            (1842, 2682)
        );
        // A simple interval is unchanged.
        assert_eq!(min_covering_arc(&[(100, 200)], 1000), (100, 200));
    }

    fn test_rec(seq: &[u8], features: Vec<SeqFeature>) -> SeqRecord {
        SeqRecord {
            id: "c".into(),
            name: "c".into(),
            description: String::new(),
            seq: seq.to_vec(),
            molecule_type: Some("DNA".into()),
            topology: Topology::Linear,
            annotations: crate::seq::Annotations::default(),
            features,
        }
    }

    fn slice1(start: i64, end: i64, strand: i32) -> SliceLoc {
        SliceLoc {
            parts: vec![Env { start, end, strand }],
            overall_strand: strand,
            dist_to_start: 0,
            dist_to_end: 0,
        }
    }

    /// A two-part slice, as produced by a window that crosses the origin of a
    /// circular contig: `[14,20)` then `[0,4)`, concatenated into a 10-long record.
    fn slice_origin_crossing() -> SliceLoc {
        SliceLoc {
            parts: vec![
                Env {
                    start: 14,
                    end: 20,
                    strand: 1,
                },
                Env {
                    start: 0,
                    end: 4,
                    strand: 1,
                },
            ],
            overall_strand: 1,
            dist_to_start: 0,
            dist_to_end: 0,
        }
    }

    fn joined(parts: &[(i64, i64, i32)]) -> Location {
        Location {
            operator: Some(CompoundOp::Join),
            between: false,
            reference: None,
            parts: parts
                .iter()
                .map(|&(s, e, st)| (s, e, st, false, false))
                .collect(),
        }
    }

    /// The id of the region `slice` cuts from a length-`l` test contig.
    fn id_of(slice: &SliceLoc, l: i64) -> String {
        region_id(
            &test_rec(&vec![b'A'; to_index(l)], vec![]),
            &slice.parts,
            false,
        )
    }

    /// Upstream `model.region_id`.
    #[test]
    fn region_ids_name_the_bases_as_written() {
        // Forward: the plain 1-based conversion.
        assert_eq!(id_of(&build_slice_from_window(4, 10, 20, 1), 20), "c_5:10");
        // Reverse: the first base written is the highest. Applying the forward rule
        // (upstream did until f0741bc) names a label two bases longer than the region:
        // upstream's own fixture named a 306 bp slice `1265:958`.
        let r = build_slice_from_window(4, 10, 20, -1);
        assert_eq!(id_of(&r, 20), "c_10:5rc");
        // Flipped back to the parent's direction, the region reads forward.
        let rec = test_rec(&[b'A'; 20], vec![]);
        assert_eq!(region_id(&rec, &r.parts, true), "c_5:10");
        // The whole contig from its own origin has no coordinates.
        assert_eq!(id_of(&slice1(0, 20, 1), 20), "c");
        assert_eq!(id_of(&slice1(0, 20, -1), 20), "c_rc");
        assert_eq!(region_id(&rec, &slice1(0, 20, -1).parts, true), "c");
        // A protein is never reverse complemented.
        let mut protein = rec.clone();
        protein.molecule_type = Some("protein".into());
        assert_eq!(region_id(&protein, &r.parts, false), "c_10:5");
    }

    #[test]
    fn wrapped_window_part_order_and_name_coords() {
        // A window that wraps the origin of a length-20 circle: [14, 24) -> [14,20)+[0,4).
        // Forward, the parts must stay in *rotation* order, because that is the order
        // the bases are read in and the order Python concatenates `location.parts`.
        let f = build_slice_from_window(14, 24, 20, 1);
        assert_eq!(
            f.parts.iter().map(|p| (p.start, p.end)).collect::<Vec<_>>(),
            vec![(14, 20), (0, 4)]
        );
        // `stranded_start`/`stranded_end` come off the first and last listed part, so
        // they name the new origin. Sorting the parts ascending would give (0, 20),
        // which reads as "the whole contig" and suppresses the coordinate suffix.
        assert_eq!(id_of(&f, 20), "c_15:4");

        // Reverse is the mirror image: each part is reverse-complemented before being
        // concatenated, so reading the arc backwards needs ascending order.
        let r = build_slice_from_window(14, 24, 20, -1);
        assert_eq!(
            r.parts.iter().map(|p| (p.start, p.end)).collect::<Vec<_>>(),
            vec![(0, 4), (14, 20)]
        );
        assert_eq!(id_of(&r, 20), "c_4:15rc");

        // A window that does not wrap is a single part either way.
        let n = build_slice_from_window(4, 10, 20, 1);
        assert_eq!(
            n.parts.iter().map(|p| (p.start, p.end)).collect::<Vec<_>>(),
            vec![(4, 10)]
        );
        assert_eq!(id_of(&n, 20), "c_5:10");

        // A full circle stays rotated to its split point.
        let w = build_slice_from_window(14, 34, 20, 1);
        assert_eq!(
            w.parts.iter().map(|p| (p.start, p.end)).collect::<Vec<_>>(),
            vec![(14, 20), (0, 14)]
        );
        assert_eq!(id_of(&w, 20), "c_15:14");
    }

    #[test]
    fn slice_record_source_spans_every_slice_part() {
        // A source feature covering the whole circle overlaps both slice parts. Taking
        // only the first match truncated it to that fragment (`[0,6)` here) instead of
        // the whole sliced record.
        let rec = test_rec(
            b"AAAACCCCGGGGTTTTACGT",
            vec![SeqFeature::new("source", Location::range(0, 20, 1))],
        );
        let out = slice_record(&rec, &slice_origin_crossing(), &rec.features);
        assert_eq!(out.seq, b"TTACGTAAAA");
        assert_eq!(out.features.len(), 1);
        assert_eq!(
            out.features[0].location.parts,
            vec![(0, 10, 1, false, false)]
        );
    }

    #[test]
    fn slice_record_coalesces_parts_split_at_the_origin() {
        // A gene stored as two parts either side of the origin lands in two different
        // slice parts, which come back abutting at the junction (6). They describe one
        // contiguous run, so they must merge rather than stay a `join`.
        let rec = test_rec(
            b"AAAACCCCGGGGTTTTACGT",
            vec![SeqFeature::new("gene", joined(&[(16, 20, 1), (0, 2, 1)]))],
        );
        let out = slice_record(&rec, &slice_origin_crossing(), &rec.features);
        assert_eq!(
            out.features[0].location.parts,
            vec![(2, 8, 1, false, false)]
        );
        assert!(
            out.features[0].location.operator.is_none(),
            "one part must not stay a join"
        );
    }

    #[test]
    fn slice_record_keeps_a_genuine_gap_split() {
        // Same two-part slice, but the pieces do not meet at the junction: merging them
        // would claim the feature covers sequence it does not.
        let rec = test_rec(
            b"AAAACCCCGGGGTTTTACGT",
            vec![SeqFeature::new("gene", joined(&[(14, 16, 1), (1, 3, 1)]))],
        );
        let out = slice_record(&rec, &slice_origin_crossing(), &rec.features);
        assert_eq!(
            out.features[0].location.parts,
            vec![(0, 2, 1, false, false), (7, 9, 1, false, false)]
        );
        assert_eq!(out.features[0].location.operator, Some(CompoundOp::Join));
    }

    #[test]
    fn slice_record_forward_remap() {
        let rec = test_rec(
            b"TTTTAACCCGTTTTTT",
            vec![SeqFeature::new("gene", Location::range(5, 8, 1))],
        );
        let out = slice_record(&rec, &slice1(4, 10, 1), &rec.features);
        assert_eq!(out.seq, b"AACCCG");
        assert_eq!(out.features.len(), 1);
        assert_eq!(
            out.features[0].location.parts,
            vec![(1, 4, 1, false, false)]
        );
    }

    #[test]
    fn slice_record_reverse_flips_seq_and_feature() {
        let rec = test_rec(
            b"TTTTAACCCGTTTTTT",
            vec![SeqFeature::new("gene", Location::range(5, 8, 1))],
        );
        let out = slice_record(&rec, &slice1(4, 10, -1), &rec.features);
        // revcomp("AACCCG") == "CGGGTT"; the feature flips within the part.
        assert_eq!(out.seq, b"CGGGTT");
        assert_eq!(
            out.features[0].location.parts,
            vec![(2, 5, -1, false, false)]
        );
    }

    #[test]
    fn slice_record_drops_uncontained_nonsource() {
        let rec = test_rec(
            b"TTTTAACCCGTTTTTT",
            vec![SeqFeature::new("gene", Location::range(0, 3, 1))], // outside [4,10)
        );
        let out = slice_record(&rec, &slice1(4, 10, 1), &rec.features);
        assert!(out.features.is_empty());
    }

    /// A CDS + its `Domain_Search` best-hit feature, both tagged with `cds_id`.
    fn hit_cds(start: i64, end: i64, cds_id: &str, hit_name: &str) -> Vec<SeqFeature> {
        let mut cds = SeqFeature::new("CDS", Location::range(start, end, 1));
        cds.set("cds_id", cds_id.to_string());
        let mut ds = SeqFeature::new(DOMAIN_SEARCH_BEST_HIT_NAME, Location::range(start, end, 1));
        ds.set("cds_id", cds_id.to_string());
        ds.set("name", hit_name.to_string());
        ds.set("score", "10.0");
        vec![cds, ds]
    }

    /// `--pad` needs each region's offset from its start to the focus CDS, so
    /// `select_by_cds_hits` has to carry `dist_to_start` out rather than drop it (it
    /// did until `domain_search` grew `--pad`). Padding then left-pads every region to
    /// the largest offset and right-pads to a common length, which lines the focus
    /// CDSs up in a column.
    #[test]
    fn hit_regions_carry_dist_to_start_so_padding_aligns_the_focus() {
        let mut feats = hit_cds(0, 9, "1_1_9", "A");
        feats.extend(hit_cds(20, 29, "21_1_29", "B"));
        feats.extend(hit_cds(40, 49, "41_1_49", "C"));
        let rec = test_rec(&[b'A'; 60], feats);
        let mut out =
            select_by_cds_hits(&rec, Range::Cds(1, 1), true, None, 1.0, None, None).unwrap();

        // The first hit has no upstream neighbor, so its region starts at the focus.
        assert_eq!(
            out.iter()
                .map(|r| (r.record.seq.len(), r.dist_to_start))
                .collect::<Vec<_>>(),
            vec![(29, 0), (49, 20), (29, 20)]
        );

        crate::tools::select_by_cds::pad_records(
            &mut out,
            crate::tools::select_by_cds::Anchor::Start,
        );

        // Every region is now the same length, and every focus CDS starts at the same
        // offset -- the largest original distance.
        assert!(
            out.iter().all(|r| r.record.seq.len() == 49),
            "regions must share a length"
        );
        let focus_starts: Vec<i64> = out
            .iter()
            .map(|r| {
                r.record
                    .features
                    .iter()
                    .find(|f| f.kind == DOMAIN_SEARCH_BEST_HIT_NAME)
                    .expect("each region keeps its focus hit feature")
                    .location
                    .start()
            })
            .collect();
        assert_eq!(focus_starts, vec![20, 20, 20]);
    }

    #[test]
    fn select_by_cds_hits_max_hits_per_contig_caps_regions() {
        // Two hit CDSs on a 60 bp contig; whole-contig extraction yields one region
        // per focus CDS. max_hits_per_contig = 1 keeps only the first.
        let mut feats = hit_cds(0, 9, "1_1_9", "A");
        feats.extend(hit_cds(30, 39, "31_1_39", "B"));
        let rec = test_rec(&[b'A'; 60], feats);
        let all = select_by_cds_hits(&rec, Range::Whole, true, None, 1.0, None, None).unwrap();
        assert_eq!(all.len(), 2);
        let capped =
            select_by_cds_hits(&rec, Range::Whole, true, None, 1.0, None, Some(1)).unwrap();
        assert_eq!(capped.len(), 1);
    }

    #[test]
    fn select_by_cds_hits_decoy_drops_matching_focus() {
        // Focus hit "A" is a decoy -> only the "B" hit region is emitted.
        let mut feats = hit_cds(0, 9, "1_1_9", "A");
        feats.extend(hit_cds(30, 39, "31_1_39", "B"));
        let rec = test_rec(&[b'A'; 60], feats);
        let decoys: std::collections::HashSet<String> = ["A".to_string()].into_iter().collect();
        let out =
            select_by_cds_hits(&rec, Range::Whole, true, None, 1.0, Some(&decoys), None).unwrap();
        assert_eq!(out.len(), 1);
    }

    /// `--translate` drops a CDS whose best hit is a decoy, as region extraction does.
    #[test]
    fn translate_peptides_drops_decoy_hits() {
        let mut feats = hit_cds(0, 9, "1_1_9", "A");
        feats.extend(hit_cds(30, 39, "31_1_39", "B"));
        let rec = test_rec(&[b'A'; 60], feats);
        assert_eq!(translate_peptides(&rec, None, None).len(), 2);
        let decoys: std::collections::HashSet<String> = ["A".to_string()].into_iter().collect();
        assert_eq!(translate_peptides(&rec, None, Some(&decoys)).len(), 1);
    }

    /// A whole-contig annotation as `domainate` writes one: `cds_id="."`.
    fn contig_hit(kind: &str, start: i64, end: i64, strand: i32, name: &str) -> SeqFeature {
        let mut f = SeqFeature::new(kind, Location::range(start, end, strand));
        f.set("cds_id", ".");
        f.set("name", name.to_string());
        f.set("score", "10.0");
        f
    }

    /// `domainate` appends a contig's `Domain_Search` hits before its `Domainator`
    /// hits, and upstream turns each into a pseudo-CDS in that order, so where the two
    /// share a start the region lists the search hit first (upstream
    /// `neighborhood_feature_index`). The port used to put every `Domainator` feature
    /// ahead of the focus hit (`gap:feature-order`). Checked on both strands, since a
    /// reverse hit's region is reverse-complemented.
    #[test]
    fn a_tied_contig_search_hit_precedes_its_domainator_twin() {
        for strand in [1, -1] {
            let rec = test_rec(
                &[b'A'; 60],
                vec![
                    contig_hit(DOMAIN_SEARCH_BEST_HIT_NAME, 20, 40, strand, "q"),
                    contig_hit(DOMAIN_FEATURE_NAME, 20, 40, strand, "q"),
                ],
            );
            let out = select_by_cds_hits(&rec, Range::Whole, true, None, 1.0, None, None).unwrap();
            assert_eq!(out.len(), 1, "strand {strand}");
            let kinds: Vec<&str> = out[0]
                .record
                .features
                .iter()
                .map(|f| f.kind.as_str())
                .collect();
            assert_eq!(
                kinds,
                [DOMAIN_SEARCH_BEST_HIT_NAME, DOMAIN_FEATURE_NAME],
                "strand {strand}"
            );
        }
    }

    /// The region lengths `select_by_cds_hits` cuts from `features` on a 100 bp
    /// contig of either topology, with a CDS range of `range`.
    fn cds_range_region_lengths(features: Vec<SeqFeature>, range: Range) -> Vec<Vec<usize>> {
        [Topology::Linear, Topology::Circular]
            .into_iter()
            .map(|topology| {
                let mut rec = test_rec(&[b'A'; 100], features.clone());
                rec.topology = topology;
                select_by_cds_hits(&rec, range, true, None, 1.0, None, None)
                    .unwrap()
                    .iter()
                    .map(|r| r.record.seq.len())
                    .collect()
            })
            .collect()
    }

    /// Upstream `test_cds_range_skips_nucleotide_annotations_around_a_nucleotide_hit`:
    /// a whole-contig hit can be the focus, but only real CDSs are neighbors, so with
    /// nothing protein-coding downstream the region ends at the hit, not at the
    /// nucleotide annotation at 70..80.
    #[test]
    fn cds_range_skips_nucleotide_annotations_around_a_nucleotide_hit() {
        let mut feats = hit_cds(0, 10, "a", "A");
        feats.truncate(1); // the CDS alone, no hit on it
        feats.push(contig_hit(DOMAIN_SEARCH_BEST_HIT_NAME, 40, 50, 1, "q"));
        feats.push(contig_hit(DOMAIN_FEATURE_NAME, 70, 80, 1, "other"));
        assert_eq!(
            cds_range_region_lengths(feats, Range::Cds(1, 1)),
            [vec![50], vec![50]]
        );
    }

    /// Upstream `test_cds_range_skips_nucleotide_annotations_between_cdss`: the next
    /// CDS downstream of `a` is `b`, not the nucleotide annotation between them.
    #[test]
    fn cds_range_skips_nucleotide_annotations_between_cdss() {
        let mut feats = hit_cds(0, 10, "a", "A");
        feats.push(contig_hit(DOMAIN_FEATURE_NAME, 20, 30, 1, "other"));
        feats.push(SeqFeature::new("CDS", Location::range(40, 50, 1)));
        assert_eq!(
            cds_range_region_lengths(feats, Range::Cds(0, 1)),
            [vec![50], vec![50]]
        );
    }

    // --- partial (`<`/`>`) locations and cut features: upstream `test_utils.py` ---

    use crate::seq::location::Part;
    use crate::seq::partial::{feature_fragment_status, translate_feature};

    /// 74 nt (upstream's comment says 75, and its tests slice to 75, which Python
    /// clamps; these use the record's end); ORF from 5 to 71.
    const PARTIAL_TEST_SEQ: &[u8] =
        b"AAAAAATGGCTAGCAAAGGTGAAGAACTGTTTACCGGTGTTGTGCCGATTCTGGTGGAACTGGATGGCAAAAAA";

    /// `_partial_test_record`: a source, then a CDS at `parts` translated from the
    /// record (unless `qualifiers` already carries a translation), then `extra`.
    fn partial_test_record(
        parts: Vec<Part>,
        qualifiers: &[(&str, &str)],
        extra: Vec<SeqFeature>,
    ) -> SeqRecord {
        let operator = (parts.len() > 1).then_some(CompoundOp::Join);
        let mut cds = SeqFeature::new(
            "CDS",
            Location {
                operator,
                between: false,
                reference: None,
                parts,
            },
        );
        for (k, v) in qualifiers {
            cds.set(k, *v);
        }
        if !cds.has("translation") {
            let t = translate_feature(&cds, PARTIAL_TEST_SEQ);
            cds.set("translation", t);
        }
        let mut features = vec![
            SeqFeature::new(
                "source",
                Location::range(0, PARTIAL_TEST_SEQ.len() as i64, 1),
            ),
            cds,
        ];
        features.extend(extra);
        test_rec(PARTIAL_TEST_SEQ, features)
    }

    /// Biopython's `str(location)` for a single part: `[<3:>69](+)`.
    fn loc_str(loc: &Location) -> String {
        let part = |&(s, e, st, b, a): &Part| {
            format!(
                "[{}{s}:{}{e}]({})",
                if b { "<" } else { "" },
                if a { ">" } else { "" },
                if st == -1 { '-' } else { '+' }
            )
        };
        if loc.parts.len() == 1 {
            part(&loc.parts[0])
        } else {
            let inner: Vec<String> = loc.parts.iter().map(part).collect();
            format!("join{{{}}}", inner.join(", "))
        }
    }

    fn sliced_cds(rec: &SeqRecord) -> &SeqFeature {
        rec.features.iter().find(|f| f.kind == "CDS").unwrap()
    }

    #[test]
    fn slice_keeps_existing_partial_markers() {
        for (strand, expected) in [(1, "[<3:>69](+)"), (-1, "[<3:>69](-)")] {
            let rec = partial_test_record(vec![(5, 71, 1, true, true)], &[], vec![]);
            let sliced = slice_record(&rec, &slice1(2, 74, strand), &rec.features);
            let cds = sliced_cds(&sliced);
            // Reverse complemented, '<' and '>' swap ends.
            assert_eq!(loc_str(&cds.location), expected);
            assert_eq!(feature_fragment_status(cds), Some("NC"));
            assert_eq!(
                cds.get_first("translation"),
                rec.features[1].get_first("translation")
            );
        }
    }

    #[test]
    fn slice_marks_cut_ends() {
        let rec = partial_test_record(vec![(5, 71, 1, false, false)], &[], vec![]);
        let sliced = slice_record(&rec, &slice1(20, 74, 1), &rec.features);
        assert_eq!(loc_str(&sliced_cds(&sliced).location), "[<0:51](+)");
        assert_eq!(feature_fragment_status(sliced_cds(&sliced)), Some("N"));
        // Source features don't get new markers.
        assert_eq!(loc_str(&sliced.features[0].location), "[0:54](+)");

        let sliced = slice_record(&rec, &slice1(0, 40, 1), &rec.features);
        assert_eq!(loc_str(&sliced_cds(&sliced).location), "[5:>40](+)");
        assert_eq!(feature_fragment_status(sliced_cds(&sliced)), Some("C"));

        // Reverse slice: the cut 5' end of a forward CDS becomes the high coordinate.
        let sliced = slice_record(&rec, &slice1(20, 74, -1), &rec.features);
        assert_eq!(loc_str(&sliced_cds(&sliced).location), "[3:>54](-)");
        assert_eq!(feature_fragment_status(sliced_cds(&sliced)), Some("N"));
    }

    #[test]
    fn slice_cut_cds_codon_start_and_translation() {
        for cut in 6..13i64 {
            for cds_strand in [1, -1] {
                for slice_strand in [1, -1] {
                    for codon_start in 1..=3i64 {
                        let rec = partial_test_record(
                            vec![(5, 71, cds_strand, false, false)],
                            &[("codon_start", &codon_start.to_string())],
                            vec![],
                        );
                        let original = &rec.features[1];
                        // Cut the same number of bases off the CDS's 5' end.
                        let slice = if cds_strand == 1 {
                            slice1(cut, rec.seq.len() as i64, slice_strand)
                        } else {
                            slice1(0, 71 + 5 - cut, slice_strand)
                        };
                        let sliced = slice_record(&rec, &slice, &rec.features);
                        let cds = sliced_cds(&sliced);
                        let case = format!(
                            "cut {cut} cds {cds_strand} slice {slice_strand} cs {codon_start}"
                        );
                        assert_eq!(feature_fragment_status(cds), Some("N"), "{case}");
                        let removed = cut - 5;
                        let expected_cs = (codon_start - 1 - removed).rem_euclid(3) + 1;
                        assert_eq!(
                            cds.get_first("codon_start"),
                            Some(expected_cs.to_string().as_str()),
                            "{case}"
                        );
                        // The trimmed translation is the cut CDS translated in its new frame.
                        let tr = cds.get_first("translation").unwrap();
                        assert_eq!(tr, translate_feature(cds, &sliced.seq), "{case}");
                        assert!(
                            original.get_first("translation").unwrap().ends_with(tr),
                            "{case}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn slice_cut_cds_3_prime() {
        let rec = partial_test_record(vec![(5, 71, 1, false, false)], &[], vec![]);
        let original = rec.features[1]
            .get_first("translation")
            .unwrap()
            .to_string();
        for end in 40..46i64 {
            let sliced = slice_record(&rec, &slice1(0, end, 1), &rec.features);
            let cds = sliced_cds(&sliced);
            assert_eq!(feature_fragment_status(cds), Some("C"));
            assert!(!cds.has("codon_start"));
            assert_eq!(
                cds.get_first("translation"),
                Some(&original[..usize::try_from((end - 5) / 3).unwrap()])
            );
        }
    }

    #[test]
    fn slice_cut_cds_interior_removed_drops_translation() {
        let rec = partial_test_record(
            vec![
                (5, 20, 1, false, false),
                (30, 50, 1, false, false),
                (60, 71, 1, false, false),
            ],
            &[],
            vec![],
        );
        let slice = SliceLoc {
            parts: vec![
                Env {
                    start: 0,
                    end: 25,
                    strand: 1,
                },
                Env {
                    start: 55,
                    end: 74,
                    strand: 1,
                },
            ],
            ..slice1(0, 0, 1)
        };
        let sliced = slice_record(&rec, &slice, &rec.features);
        assert!(!sliced_cds(&sliced).has("translation"));
    }

    #[test]
    fn slice_drops_cut_domain_features() {
        let mut domain = SeqFeature::new(DOMAIN_FEATURE_NAME, Location::range(10, 40, 1));
        domain.set("name", "d");
        let mut best = SeqFeature::new(DOMAIN_SEARCH_BEST_HIT_NAME, Location::range(30, 50, 1));
        best.set("name", "d");
        let rec = partial_test_record(vec![(5, 71, 1, false, false)], &[], vec![domain, best]);
        let sliced = slice_record(&rec, &slice1(20, 74, 1), &rec.features);
        let kinds: Vec<&str> = sliced.features.iter().map(|f| f.kind.as_str()).collect();
        assert_eq!(kinds, ["source", "CDS", DOMAIN_SEARCH_BEST_HIT_NAME]);
    }

    /// A CDS split across the ends of a rotated circular contig is not cut: the whole
    /// of it is still in the record, so it gets no markers and keeps its translation.
    #[test]
    fn slice_origin_split_feature_is_not_cut() {
        let mut gene = SeqFeature::new(
            "CDS",
            Location {
                operator: Some(CompoundOp::Join),
                between: false,
                reference: None,
                parts: vec![(16, 20, 1, false, false), (0, 2, 1, false, false)],
            },
        );
        gene.set("translation", "MK");
        let rec = test_rec(b"GGAAAAAAAAAAAAAAATGA", vec![gene]);
        let sliced = slice_record(&rec, &slice_origin_crossing(), &rec.features);
        let cds = sliced_cds(&sliced);
        assert_eq!(loc_str(&cds.location), "[2:8](+)");
        assert_eq!(cds.get_first("translation"), Some("MK"));
        assert!(!cds.has("codon_start"));
    }
}
