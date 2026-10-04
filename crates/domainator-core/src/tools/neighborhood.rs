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
    /// `cds_id` (`get_cds_unique_name`), used to attach domain features.
    pub(crate) cds_id: String,
    /// True if this CDS carries a `Domain_Search` best-hit feature.
    pub(crate) is_focus: bool,
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
/// forward sub-parts within `[0, l)`, tagged `strand`, with the stranded
/// `(name_start, name_end)` for the coordinate suffix.
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
    let first = parts[0];
    let last = *parts.last().unwrap();
    let (ns, ne) = if strand == -1 {
        (first.end, last.start)
    } else {
        (first.start, last.end)
    };
    SliceLoc {
        parts,
        name_start: ns,
        name_end: ne,
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
    pub(crate) name_start: i64,
    pub(crate) name_end: i64,
    pub(crate) overall_strand: i32,
    /// Upstream / downstream extension sizes (bp) of the focus CDS within the
    /// region, for `--pad` alignment. Only computed for linear contigs (0 for
    /// circular, where `--pad` is unsupported).
    pub(crate) dist_to_start: i64,
    pub(crate) dist_to_end: i64,
}

/// An extracted region plus the distance from the region start to the focus CDS
/// (what `pad_records` aligns on). Both `select_by_cds` and `domain_search`'s
/// neighborhood path produce these.
pub struct Region {
    pub record: SeqRecord,
    pub dist_to_start: i64,
}

impl SliceLoc {
    /// The 1-based `(start, end)` for the `_start:end` coordinate suffix —
    /// `stranded_start_human_readable` / `stranded_end_human_readable` of the slice.
    ///
    /// Forward that is `(name_start + 1, name_end)`. On a **reverse** slice the roles
    /// invert: `name_start` is already an exclusive end so it takes no `+1`, and
    /// `name_end` is a 0-based start so it does. Applying the forward rule to both
    /// (as upstream's `get_cds_neighborhood` did until `f0741bc`) leaves a reverse
    /// label off by one in each direction — a 306 bp region named `1265:958`, which
    /// spans 308.
    ///
    /// The whole-contig no-rename checks use the raw 0-based `name_start`/`name_end`,
    /// not this.
    pub(crate) fn name_label(&self) -> (i64, i64) {
        if self.overall_strand == -1 {
            (self.name_start, self.name_end + 1)
        } else {
            (self.name_start + 1, self.name_end)
        }
    }
}

/// Compute the slice location for the focus CDS at `focus_idx` in the
/// stranded-start-sorted `cds_list`. Mirrors `get_cds_neighborhood`.
pub(crate) fn cds_neighborhood(
    rec_len: i64,
    circular: bool,
    cds_list: &[Cds],
    focus_idx: usize,
    range: Range,
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
                let n = cds_list.len() as i64;
                let (left_cds, right_cds) = if strand == -1 {
                    (
                        (focus_idx as i64 - cd).max(0),
                        (focus_idx as i64 + cu).min(n - 1),
                    )
                } else {
                    (
                        (focus_idx as i64 - cu).max(0),
                        (focus_idx as i64 + cd).min(n - 1),
                    )
                };
                let left_coord = cds_list[to_index(left_cds)].start();
                let right_coord = cds_list[to_index(right_cds)].end();
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
        let (ns, ne) = if padded.strand == -1 {
            (padded.end, padded.start)
        } else {
            (padded.start, padded.end)
        };
        return SliceLoc {
            parts: vec![padded],
            name_start: ns,
            name_end: ne,
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
            let (pu, pd) = ((u * 1000.0) as i64, (d * 1000.0) as i64);
            if strand == -1 {
                (a - pd, b + pu)
            } else {
                (a - pu, b + pd)
            }
        }
        Range::Cds(cu, cd) => {
            let n = cds_list.len() as i64;
            let (li, ri) = if strand == -1 {
                pad_circular_cds(focus_idx as i64, n, cd, cu)
            } else {
                pad_circular_cds(focus_idx as i64, n, cu, cd)
            };
            // Window = directed circular envelope of the left neighbor, focus, and
            // right neighbor gene arcs, extended in that order (matching Python's
            // `left_env + focus_env + right_env` then `circular_envelope`).
            let arc = |c: &Cds| {
                let (ca, cb) = min_covering_arc(&c.intervals, l);
                arc_to_forward_parts(ca, cb, l)
            };
            let mut nbhd: Vec<(i64, i64)> = Vec::new();
            nbhd.extend(arc(&cds_list[to_index(li)]));
            nbhd.extend(arc_to_forward_parts(a, b, l));
            nbhd.extend(arc(&cds_list[to_index(ri)]));
            ordered_circular_envelope(&nbhd, l)
        }
        Range::Whole => (a, b), // replaced below
    };

    // Whole contig (or any window covering the full circle): the whole circle,
    // split at the midpoint of the gap opposite the focus.
    let is_whole = matches!(range, Range::Whole) || (w_hi - w_lo) >= l;
    if is_whole {
        let split = ((a + b + l) / 2).rem_euclid(l);
        return build_slice_from_window(split, split + l, l, strand);
    }
    build_slice_from_window(w_lo, w_hi, l, strand)
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

    // cds_ids carrying a Domain_Search best-hit feature (the focus CDSs), plus
    // the best-hit domain name per focus cds_id (for decoy filtering).
    let mut focus_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut hit_name_by_cid: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for f in &rec.features {
        if f.kind == DOMAIN_SEARCH_BEST_HIT_NAME {
            if let Some(cid) = f.get_first("cds_id") {
                focus_ids.insert(cid.to_string());
                if let Some(name) = f.get_first("name") {
                    hit_name_by_cid.insert(cid.to_string(), name.to_string());
                }
            }
        }
    }

    // Build the CDS list, keeping each CDS's forward intervals and cds_id.
    let mut cds_list: Vec<Cds> = Vec::new();
    for f in &rec.features {
        if f.kind != "CDS" {
            continue;
        }
        let cid = cds_unique_name(f);
        let intervals: Vec<(i64, i64)> = f
            .location
            .parts
            .iter()
            .map(|&(s, e, ..)| if s <= e { (s, e) } else { (e, s) })
            .collect();
        let is_focus = focus_ids.contains(&cid);
        cds_list.push(Cds {
            intervals,
            strand: f.location.strand(),
            cds_id: cid,
            is_focus,
        });
    }

    // Whole-contig (nhmmer) hits carry `cds_id="."`; each best-hit contig feature
    // becomes its own focus pseudo-CDS (mirrors `list_from_contig`'s
    // `include_nucleic_acid_annotations` path). Its cds_id is a synthetic `nuc_N`
    // so it never collides with a real CDS, and the originating feature is kept
    // verbatim when it is the focus.
    let mut nuc_features: std::collections::HashMap<String, &SeqFeature> =
        std::collections::HashMap::new();
    let mut nuc_idx = 0usize;
    for f in &rec.features {
        if f.kind == DOMAIN_SEARCH_BEST_HIT_NAME && f.get_first("cds_id") == Some(".") {
            let cid = format!("nuc_{nuc_idx}");
            nuc_idx += 1;
            let intervals: Vec<(i64, i64)> = f
                .location
                .parts
                .iter()
                .map(|&(s, e, ..)| if s <= e { (s, e) } else { (e, s) })
                .collect();
            let strand = f.location.strand();
            if let Some(name) = f.get_first("name") {
                hit_name_by_cid.insert(cid.clone(), name.to_string());
            }
            nuc_features.insert(cid.clone(), f);
            cds_list.push(Cds {
                intervals,
                strand,
                cds_id: cid,
                is_focus: true,
            });
        }
    }

    // Sort by stranded_start (matches Python's cds_list ordering).
    cds_list.sort_by_key(|c| c.sort_key(rec_len));

    // Non-Domainator features (CDS, gene, source, ...) — slice_record filters these
    // by containment. Domainator features and the focus Domain_Search feature are
    // attached per CDS below.
    let non_dom: Vec<&SeqFeature> = rec
        .features
        .iter()
        .filter(|f| f.kind != DOMAIN_FEATURE_NAME && f.kind != DOMAIN_SEARCH_BEST_HIT_NAME)
        .collect();
    let dom_by_cid: Vec<(&SeqFeature, String)> = rec
        .features
        .iter()
        .filter(|f| f.kind == DOMAIN_FEATURE_NAME)
        .filter_map(|f| f.get_first("cds_id").map(|c| (f, c.to_string())))
        .collect();
    let search_by_cid: Vec<(&SeqFeature, String)> = rec
        .features
        .iter()
        .filter(|f| f.kind == DOMAIN_SEARCH_BEST_HIT_NAME)
        .filter_map(|f| f.get_first("cds_id").map(|c| (f, c.to_string())))
        .collect();

    let cds_ids: std::collections::HashSet<&str> =
        cds_list.iter().map(|c| c.cds_id.as_str()).collect();

    let mut out: Vec<Region> = Vec::new();
    let mut selected: Vec<SliceLoc> = Vec::new();

    for focus_idx in 0..cds_list.len() {
        let cds = &cds_list[focus_idx];
        if !cds.is_focus {
            continue;
        }
        if let Some(sf) = strand_filter {
            if cds.strand != sf {
                continue;
            }
        }
        let focus_cid = cds.cds_id.clone();

        // Decoy filter: skip focus CDSs whose best hit is a decoy domain.
        if let Some(decoys) = decoys {
            if hit_name_by_cid
                .get(&focus_cid)
                .is_some_and(|n| decoys.contains(n))
            {
                continue;
            }
        }

        let slice = cds_neighborhood(rec_len, circular, &cds_list, focus_idx, range);

        // Collect features: non-Domainator features, the Domainator features of every
        // CDS, the contig-level (whole-contig nhmmer, `cds_id="."`) Domainator
        // features, and the focus CDS's Domain_Search feature. `slice_record` drops
        // the hit features the region cuts, and those outside it (upstream `8bf3c42`
        // removed `get_cds_neighborhood`'s containment pre-filter).
        let mut keep: Vec<SeqFeature> = non_dom.iter().map(|f| (*f).clone()).collect();
        for &(domf, ref cid) in &dom_by_cid {
            if cds_ids.contains(cid.as_str()) || cid == "." {
                keep.push(domf.clone());
            }
        }
        if let Some(nucf) = nuc_features.get(focus_cid.as_str()) {
            // Nucleic-acid (whole-contig nhmmer) focus: keep just this contig hit.
            keep.push((*nucf).clone());
        } else {
            for &(sf, ref cid) in &search_by_cid {
                if *cid == focus_cid {
                    keep.push(sf.clone());
                }
            }
        }

        let mut region = slice_record(rec, &slice, &keep);

        // Coordinate renaming and reverse-complement handling.
        let (start, end) = (slice.name_start, slice.name_end);
        let strand = slice.overall_strand;
        // The coordinate suffix and `rc` marker belong to the LOCUS identifier;
        // ACCESSION/VERSION remain unchanged.
        let whole_fwd = start == 0 && end == rec_len && strand != -1;
        let whole_rev = end == 0 && start == rec_len && strand == -1;
        if !whole_fwd && !whole_rev {
            let (ls, le) = slice.name_label();
            region.id = format!("{}_{}:{}", region.id, ls, le);
        }
        if !normalize_direction && strand == -1 {
            // Flip back to the original orientation.
            region = reverse_complement_record(&region);
        } else if strand == -1 {
            region.id.push_str("rc");
        }

        // `dist_to_start` travels with the region so `--pad` can align the focus CDSs
        // after the whole search completes; it is 0 for circular contigs, where padding
        // is unsupported. Read before `slice` can be moved into `selected` below, and
        // swapped with `dist_to_end` on an un-normalized reverse region, which was
        // flipped back to the original orientation above.
        let dist_to_start = if !normalize_direction && strand == -1 {
            slice.dist_to_end
        } else {
            slice.dist_to_start
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
        });
        if max_hits_per_contig.is_some_and(|m| out.len() >= m) {
            break;
        }
    }
    Ok(out)
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
pub fn translate_peptides(rec: &SeqRecord, strand_filter: Option<i32>) -> Vec<SeqRecord> {
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
            name_start: 0,
            name_end: 0,
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
            name_start: 0,
            name_end: 0,
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
            name_start: 14,
            name_end: 4,
            overall_strand: 1,
            dist_to_start: 0,
            dist_to_end: 0,
        }
    }

    fn joined(parts: &[(i64, i64, i32)]) -> Location {
        Location {
            operator: Some(CompoundOp::Join),
            between: false,
            parts: parts
                .iter()
                .map(|&(s, e, st)| (s, e, st, false, false))
                .collect(),
        }
    }

    #[test]
    fn name_label_inverts_on_the_reverse_strand() {
        // Forward: the plain 1-based conversion.
        let f = build_slice_from_window(4, 10, 20, 1);
        assert_eq!((f.name_start, f.name_end), (4, 10));
        assert_eq!(f.name_label(), (5, 10));

        // Reverse: `name_start` is already an exclusive end and `name_end` a 0-based
        // start, so the `+1` moves to the other coordinate. Applying the forward rule
        // here (upstream did until f0741bc) gives a label spanning two more bases than
        // the region: upstream's own fixture named a 306 bp slice `1265:958`.
        let r = build_slice_from_window(4, 10, 20, -1);
        assert_eq!((r.name_start, r.name_end), (10, 4));
        assert_eq!(r.name_label(), (10, 5));
        // 10 down to 5 inclusive is 6 bases -- the length of the slice.
        let (ls, le) = r.name_label();
        assert_eq!(
            ls - le + 1,
            r.parts.iter().map(|p| p.end - p.start).sum::<i64>()
        );
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
        assert_eq!((f.name_start, f.name_end), (14, 4));

        // Reverse is the mirror image: each part is reverse-complemented before being
        // concatenated, so reading the arc backwards needs ascending order.
        let r = build_slice_from_window(14, 24, 20, -1);
        assert_eq!(
            r.parts.iter().map(|p| (p.start, p.end)).collect::<Vec<_>>(),
            vec![(0, 4), (14, 20)]
        );
        assert_eq!((r.name_start, r.name_end), (4, 14));

        // A window that does not wrap is a single part either way.
        let n = build_slice_from_window(4, 10, 20, 1);
        assert_eq!(
            n.parts.iter().map(|p| (p.start, p.end)).collect::<Vec<_>>(),
            vec![(4, 10)]
        );
        assert_eq!((n.name_start, n.name_end), (4, 10));

        // A full circle stays rotated to its split point (Python names it `15:14`).
        let w = build_slice_from_window(14, 34, 20, 1);
        assert_eq!(
            w.parts.iter().map(|p| (p.start, p.end)).collect::<Vec<_>>(),
            vec![(14, 20), (0, 14)]
        );
        assert_eq!((w.name_start, w.name_end), (14, 14));
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

        crate::tools::select_by_cds::pad_records(&mut out);

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
