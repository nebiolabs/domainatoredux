//! `select_by_cds`: extract contig regions around CDSs selected by name, domain
//! content, a boolean domain expression, search-hit status, or "unannotated".
//! Faithful port of `domainator.select_by_cds.select_by_cds` (the standalone,
//! non-`_from_domain_search` path), reusing the neighborhood geometry in
//! [`crate::tools::neighborhood`].

use std::collections::HashSet;

use crate::seq::location::to_index;
use crate::seq::partial::{cds_fragment_status, Partial};
use crate::seq::taxonomy::TaxonomyFilter;
use crate::seq::{SeqFeature, SeqRecord, Topology};
use crate::tools::neighborhood::{
    cds_neighborhood, neighborhood_record, overlap_size, region_id, reverse_complement_record, Cds,
    Range, SliceLoc,
};
use crate::tools::select_common::{
    cds_name_precedence, get_non_domainator_features, AnnotationType, BooleanEvaluator,
    DomainatorCds, CDS_NAME_FIELDS,
};

#[derive(Default)]
pub struct SelectByCdsOptions {
    pub target_cdss: Option<HashSet<String>>,
    pub target_domains: Option<HashSet<String>>,
    pub domain_expr: Option<String>,
    pub range: Range,
    pub normalize_direction: bool,
    /// Only annotations with an e-value at most this count (`None`: no cutoff).
    pub evalue: Option<f64>,
    /// Negates the combined selectors, never the strand, partial or taxonomy
    /// restrictions; ignored, with a warning, when there are no selectors.
    pub invert: bool,
    /// Which annotations supply the domain names `target_domains`, `domain_expr` and
    /// `unannotated` look at. Selects nothing by itself.
    pub annotation_type: AnnotationType,
    /// `--has_search_hit`: also select CDSs marked as domain_search hits, whatever
    /// their names; also admits contig-level nucleotide hits.
    pub has_search_hit: bool,
    /// `--name_field`: the qualifier `target_cdss` is matched against first.
    pub name_field: Option<String>,
    /// Taxonomy restriction on the focus CDS (not the neighborhood around it).
    pub taxonomy: Option<TaxonomyFilter>,
    pub max_region_overlap: f64,
    /// `Some(1)` = forward only, `Some(-1)` = reverse only.
    pub strand: Option<i32>,
    pub databases: Option<HashSet<String>>,
    pub unannotated: bool,
    pub include_nucleic_acids: bool,
    /// `--partial`: focus CDSs by fragment status; never inverted by `invert`.
    pub partial: Partial,
}

pub use crate::tools::neighborhood::Region;

/// Extract regions around selected CDSs across all `records`. Regions are
/// yielded in contig/CDS order; id-deduplication and `--pad` are applied by the
/// caller ([`pad_records`]).
pub fn select_by_cds(
    records: &[SeqRecord],
    opts: &SelectByCdsOptions,
) -> Result<Vec<Region>, String> {
    let expr = opts
        .domain_expr
        .as_deref()
        .map(BooleanEvaluator::new)
        .transpose()?;
    let no_selectors = opts.target_cdss.is_none()
        && opts.target_domains.is_none()
        && opts.domain_expr.is_none()
        && !opts.has_search_hit
        && !opts.unannotated;
    if no_selectors && opts.invert {
        crate::log::warn(format_args!(
            "Invert supplied with no selection criteria. Invert will be ignored."
        ));
    }
    let mut out = Vec::new();
    for rec in records {
        out.extend(select_one(rec, opts, expr.as_ref(), no_selectors)?);
    }
    Ok(out)
}

fn select_one(
    rec: &SeqRecord,
    opts: &SelectByCdsOptions,
    expr: Option<&BooleanEvaluator>,
    no_selectors: bool,
) -> Result<Vec<Region>, String> {
    let rec_len = rec.seq.len() as i64;
    let circular = rec.topology == Topology::Circular;

    // DomainatorCDS list (nucleic-acid annotations included, evalue-filtered),
    // paired with the geometry Cds, then sorted by stranded start together.
    let precedence = cds_name_precedence(opts.name_field.as_deref(), &CDS_NAME_FIELDS);
    let dom_cdss = DomainatorCds::list_from_contig(
        rec,
        opts.evalue,
        f64::NEG_INFINITY,
        false,
        Some(&precedence),
        true,
    );
    let mut paired: Vec<(DomainatorCds, Cds)> = dom_cdss
        .into_iter()
        .map(|d| {
            let intervals: Vec<(i64, i64)> = d
                .feature
                .location
                .parts
                .iter()
                .map(|&(s, e, ..)| if s <= e { (s, e) } else { (e, s) })
                .collect();
            let strand = d.feature.location.strand();
            let nb = Cds {
                intervals,
                strand,
                nucleic_acid: d.is_nucleic_acid,
            };
            (d, nb)
        })
        .collect();
    paired.sort_by_key(|(_, nb)| nb.sort_key(rec_len.max(1)));
    let dom_list: Vec<DomainatorCds> = paired.iter().map(|(d, _)| d.clone()).collect();
    let nb_list: Vec<Cds> = paired.into_iter().map(|(_, nb)| nb).collect();

    let non_dom: Vec<SeqFeature> = get_non_domainator_features(rec)
        .into_iter()
        .cloned()
        .collect();

    let mut out: Vec<Region> = Vec::new();
    let mut selected_slices: Vec<SliceLoc> = Vec::new();

    for focus_index in 0..dom_list.len() {
        let cds = &dom_list[focus_index];
        let strand = nb_list[focus_index].strand;

        if cds.is_nucleic_acid
            && !opts.include_nucleic_acids
            && !(opts.has_search_hit && cds.domain_search_feature.is_some())
        {
            continue;
        }

        let mut keep = if no_selectors {
            true
        } else {
            let mut k = false;
            if let Some(tc) = &opts.target_cdss {
                if tc.contains(&cds.name) {
                    k = true;
                }
            }
            let dnames = cds.get_domain_names(opts.databases.as_ref(), None, opts.annotation_type);
            if let Some(td) = &opts.target_domains {
                if td.intersection(&dnames).next().is_some() {
                    k = true;
                }
            }
            if let Some(e) = expr {
                if e.check_expression(&dnames) {
                    k = true;
                }
            }
            if opts.unannotated && dnames.is_empty() {
                k = true;
            }
            if opts.has_search_hit && cds.domain_search_feature.is_some() {
                k = true;
            }
            k != opts.invert
        };

        if let Some(sf) = opts.strand {
            // Reject anything the strand filter does not name.
            if (sf == 1 && strand == -1) || (sf == -1 && strand != -1) {
                keep = false;
            }
        }
        if keep
            && !opts
                .partial
                .allows(cds_fragment_status(rec, Some(&cds.feature)))
        {
            keep = false;
        }
        if keep {
            if let Some(taxonomy) = &opts.taxonomy {
                keep = taxonomy.matches(rec, Some(&cds.feature))?;
            }
        }
        if !keep {
            continue;
        }

        let slice = cds_neighborhood(rec_len, circular, &nb_list, focus_index, opts.range, None);

        // Features: non-Domainator features, and every CDS's Domainator domains and
        // Domain_Search feature. `slice_record` truncates what the region cuts and
        // drops the cut hit features (upstream `8bf3c42` removed the containment
        // pre-filter here).
        let mut features: Vec<SeqFeature> = non_dom.clone();
        for c in &dom_list {
            features.extend(c.domain_features.iter().cloned());
            if let Some(sf) = &c.domain_search_feature {
                features.push(sf.clone());
            }
        }

        let mut region = neighborhood_record(rec, &slice, &features);

        let flipped = !opts.normalize_direction && strand == -1;
        let (dist_to_start, dist_to_end) = if flipped {
            (slice.dist_to_end, slice.dist_to_start)
        } else {
            (slice.dist_to_start, slice.dist_to_end)
        };
        if flipped {
            region = reverse_complement_record(&region);
        }
        region.id = region_id(rec, &slice.parts, flipped);

        if opts.max_region_overlap < 1.0 {
            let loc_len: i64 = slice.parts.iter().map(|p| p.end - p.start).sum();
            let mut drop = false;
            for other in &selected_slices {
                if loc_len > 0
                    && overlap_size(&slice, other) as f64 / loc_len as f64 > opts.max_region_overlap
                {
                    drop = true;
                    break;
                }
            }
            if drop {
                continue;
            }
            selected_slices.push(slice);
        }
        out.push(Region {
            record: region,
            dist_to_start,
            dist_to_end,
            focus_strand: if flipped { -1 } else { 1 },
        });
    }
    Ok(out)
}

/// Where `--pad` lines the focus CDSs up (`--anchor`), relative to the focus CDS in
/// its own orientation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Anchor {
    /// The first base of the focus CDSs.
    #[default]
    Start,
    /// Their last base.
    End,
    /// Their midpoints.
    Center,
}

impl std::str::FromStr for Anchor {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "start" => Ok(Anchor::Start),
            "end" => Ok(Anchor::End),
            "center" => Ok(Anchor::Center),
            _ => Err(format!(
                "invalid anchor {s:?}: expected start, end or center"
            )),
        }
    }
}

/// `select_by_cds.pad_anchor`: the 0-based coordinate in `region` to align on. For
/// [`Anchor::End`] on the forward strand, the position just after the focus CDS.
pub fn pad_anchor(region: &Region, anchor: Anchor) -> i64 {
    let left = region.dist_to_start;
    let right = region.record.seq.len() as i64 - region.dist_to_end;
    let length = right - left;
    match (anchor, region.focus_strand == -1) {
        (Anchor::Start, false) | (Anchor::End, true) => left,
        (Anchor::End, false) | (Anchor::Start, true) => right,
        (Anchor::Center, false) => left + length.div_euclid(2),
        (Anchor::Center, true) => right - length.div_euclid(2),
    }
}

/// `select_by_cds.pad_records`: left-pad each region with `N` so the focus CDSs
/// align at `anchor`, then right-pad so all regions share a length. Features shift
/// with the left pad. A padded record is linear: the padding is not part of the
/// molecule.
pub fn pad_records(regions: &mut [Region], anchor: Anchor) {
    if regions.is_empty() {
        return;
    }
    let offsets: Vec<i64> = regions.iter().map(|r| pad_anchor(r, anchor)).collect();
    let lengths: Vec<i64> = regions.iter().map(|r| r.record.seq.len() as i64).collect();
    let max_dist = *offsets.iter().max().unwrap();
    let shift: Vec<i64> = offsets.iter().map(|o| max_dist - o).collect();
    let ends: Vec<i64> = (0..regions.len()).map(|i| shift[i] + lengths[i]).collect();
    let max_end = *ends.iter().max().unwrap();
    let right: Vec<i64> = ends.iter().map(|e| max_end - e).collect();

    for (i, region) in regions.iter_mut().enumerate() {
        let left = to_index(shift[i]);
        if left > 0 || right[i] > 0 {
            let rec = &mut region.record;
            rec.topology = Topology::Linear;
            if rec.annotations.topology == Some(Topology::Circular) {
                rec.annotations.topology = Some(Topology::Linear);
            }
        }
        if left > 0 {
            let mut seq = vec![b'N'; left];
            seq.extend_from_slice(&region.record.seq);
            region.record.seq = seq;
            for f in &mut region.record.features {
                for p in &mut f.location.parts {
                    p.0 += left as i64;
                    p.1 += left as i64;
                }
            }
        }
        if right[i] > 0 {
            region
                .record
                .seq
                .extend(std::iter::repeat_n(b'N', to_index(right[i])));
        }
        region.dist_to_start += shift[i];
        region.dist_to_end += right[i];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region(len: usize, dist_to_start: i64, dist_to_end: i64, focus_strand: i32) -> Region {
        let record = SeqRecord {
            id: "r".into(),
            name: "r".into(),
            description: String::new(),
            seq: vec![b'A'; len],
            molecule_type: Some("DNA".into()),
            topology: Topology::Circular,
            annotations: crate::seq::Annotations {
                topology: Some(Topology::Circular),
                ..Default::default()
            },
            features: Vec::new(),
        };
        Region {
            record,
            dist_to_start,
            dist_to_end,
            focus_strand,
        }
    }

    /// Upstream `pad_anchor`: the focus spans [4, 10) of a 20 bp region. Start and end
    /// swap on a focus kept on the reverse strand, and the center rounds toward the
    /// focus's own start.
    #[test]
    fn the_pad_anchor_follows_the_focus_orientation() {
        let fwd = region(20, 4, 10, 1);
        let rev = region(20, 4, 10, -1);
        let at = |r: &Region, a| pad_anchor(r, a);
        assert_eq!(
            (
                at(&fwd, Anchor::Start),
                at(&fwd, Anchor::End),
                at(&fwd, Anchor::Center)
            ),
            (4, 10, 7)
        );
        assert_eq!(
            (
                at(&rev, Anchor::Start),
                at(&rev, Anchor::End),
                at(&rev, Anchor::Center)
            ),
            (10, 4, 7)
        );
        let odd = region(21, 4, 10, -1);
        assert_eq!(at(&odd, Anchor::Center), 11 - 3);
    }

    /// Padding aligns the anchors in one column, and a padded record is linear; one
    /// left as it was keeps its topology.
    #[test]
    fn padding_aligns_the_anchors_and_makes_records_linear() {
        let mut regions = vec![region(20, 4, 10, 1), region(30, 8, 2, 1)];
        pad_records(&mut regions, Anchor::End);
        let ends: Vec<i64> = regions.iter().map(|r| pad_anchor(r, Anchor::End)).collect();
        // The second region's focus ends at 28; the first is shifted right by 18 to
        // match, and the second right-padded to the same length.
        assert_eq!(ends, [28, 28]);
        assert_eq!(regions[0].record.seq[..18], [b'N'; 18]);
        assert!(regions.iter().all(|r| r.record.seq.len() == 38));
        for r in &regions {
            assert_eq!(r.record.topology, Topology::Linear);
            assert_eq!(r.record.annotations.topology, Some(Topology::Linear));
        }
        let mut alone = vec![region(20, 4, 10, 1)];
        pad_records(&mut alone, Anchor::Start);
        assert_eq!(alone[0].record.topology, Topology::Circular);
    }
}
