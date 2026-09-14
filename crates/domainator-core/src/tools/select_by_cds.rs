//! `select_by_cds`: extract contig regions around CDSs selected by name, domain
//! content, a boolean domain expression, search-hit status, or "unannotated".
//! Faithful port of `domainator.select_by_cds.select_by_cds` (the standalone,
//! non-`_from_domain_search` path), reusing the neighborhood geometry in
//! [`crate::tools::neighborhood`].

use std::collections::HashSet;

use crate::seq::{SeqFeature, SeqRecord, Topology};
use crate::tools::neighborhood::{
    cds_neighborhood, overlap_size, reverse_complement_record, slice_contains, slice_record, Cds,
    Range, SliceLoc,
};
use crate::tools::select_common::{get_non_domainator_features, BooleanEvaluator, DomainatorCds};

#[derive(Default)]
pub struct SelectByCdsOptions {
    pub target_cdss: Option<HashSet<String>>,
    pub target_domains: Option<HashSet<String>>,
    pub domain_expr: Option<String>,
    pub range: Range,
    pub normalize_direction: bool,
    pub evalue: f64,
    pub invert: bool,
    pub search_hits: bool,
    pub max_region_overlap: f64,
    /// `Some(1)` = forward only, `Some(-1)` = reverse only.
    pub strand: Option<i32>,
    pub databases: Option<HashSet<String>>,
    pub unannotated: bool,
    pub include_nucleic_acids: bool,
}


/// An extracted region plus the distance from the region start to the focus
/// CDS (for `--pad` alignment).
pub struct Region {
    pub record: SeqRecord,
    pub dist_to_start: i64,
}

/// Extract regions around selected CDSs across all `records`. Regions are
/// yielded in contig/CDS order; id-deduplication and `--pad` are applied by the
/// caller ([`pad_records`]).
pub fn select_by_cds(records: &[SeqRecord], opts: &SelectByCdsOptions) -> Vec<Region> {
    let expr = opts.domain_expr.as_deref().map(BooleanEvaluator::new);
    let mut out = Vec::new();
    for rec in records {
        out.extend(select_one(rec, opts, expr.as_ref()));
    }
    out
}

fn select_one(rec: &SeqRecord, opts: &SelectByCdsOptions, expr: Option<&BooleanEvaluator>) -> Vec<Region> {
    let rec_len = rec.seq.len() as i64;
    let circular = rec.topology == Topology::Circular;

    // DomainatorCDS list (nucleic-acid annotations included, evalue-filtered),
    // paired with the geometry Cds, then sorted by stranded start together.
    let dom_cdss = DomainatorCds::list_from_contig(rec, opts.evalue, f64::NEG_INFINITY, false, None, true);
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
            let cds_id = d.num.clone();
            let nb = Cds { intervals, strand, cds_id, is_focus: false };
            (d, nb)
        })
        .collect();
    paired.sort_by_key(|(_, nb)| nb.sort_key(rec_len.max(1)));
    let dom_list: Vec<DomainatorCds> = paired.iter().map(|(d, _)| d.clone()).collect();
    let nb_list: Vec<Cds> = paired.into_iter().map(|(_, nb)| nb).collect();

    let no_selectors = opts.target_cdss.is_none()
        && opts.target_domains.is_none()
        && opts.domain_expr.is_none()
        && !opts.search_hits
        && !opts.unannotated;

    let non_dom: Vec<SeqFeature> = get_non_domainator_features(rec).into_iter().cloned().collect();

    let mut out: Vec<Region> = Vec::new();
    let mut selected_slices: Vec<SliceLoc> = Vec::new();

    for focus_index in 0..dom_list.len() {
        let cds = &dom_list[focus_index];
        let strand = nb_list[focus_index].strand;

        if cds.is_nucleic_acid && !opts.search_hits && !opts.include_nucleic_acids {
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
            let dnames = cds.get_domain_names(opts.databases.as_ref(), None);
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
            if opts.search_hits && cds.domain_search_feature.is_some() {
                k = true;
            }
            k
        };

        if let Some(sf) = opts.strand {
            // Reject anything the strand filter does not name.
            if (sf == 1 && strand == -1) || (sf == -1 && strand != -1) {
                keep = false;
            }
        }
        if opts.invert {
            keep = !keep;
        }
        if !keep {
            continue;
        }

        let slice = cds_neighborhood(rec_len, circular, &nb_list, focus_index, opts.range);

        // Features: non-Domainator features (slice_record filters by containment);
        // and, for every CDS contained in the slice, its Domainator domains and its
        // Domain_Search feature (standalone keeps all contained search features).
        let mut features: Vec<SeqFeature> = non_dom.clone();
        for (i, c) in dom_list.iter().enumerate() {
            if slice_contains(&slice, &nb_list[i].intervals) {
                features.extend(c.domain_features.iter().cloned());
                if let Some(sf) = &c.domain_search_feature {
                    features.push(sf.clone());
                }
            }
        }

        let mut region = slice_record(rec, &slice, &features);

        // Coordinate suffix + reverse-complement handling (mirrors
        // get_cds_neighborhood; the suffix/`rc` go on the LOCUS name).
        let (start, end) = (slice.name_start, slice.name_end);
        let whole_fwd = start == 0 && end == rec_len && strand != -1;
        let whole_rev = end == 0 && start == rec_len && strand == -1;
        if !whole_fwd && !whole_rev {
            region.name = format!("{}_{}:{}", region.name, start + 1, end);
        }
        let mut dist_to_start = slice.dist_to_start;
        if !opts.normalize_direction && strand == -1 {
            region = reverse_complement_record(&region);
            dist_to_start = slice.dist_to_end;
        } else if strand == -1 {
            region.name.push_str("rc");
        }

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
        out.push(Region { record: region, dist_to_start });
    }
    out
}

/// `select_by_cds.pad_records`: left-pad each region with `N` so the focus CDSs
/// align, then right-pad so all regions share a length. Features shift with the
/// left pad.
pub fn pad_records(regions: &mut [Region]) {
    if regions.is_empty() {
        return;
    }
    let offsets: Vec<i64> = regions.iter().map(|r| r.dist_to_start).collect();
    let lengths: Vec<i64> = regions.iter().map(|r| r.record.seq.len() as i64).collect();
    let max_dist = *offsets.iter().max().unwrap();
    let shift: Vec<i64> = offsets.iter().map(|o| max_dist - o).collect();
    let ends: Vec<i64> = (0..regions.len()).map(|i| shift[i] + lengths[i]).collect();
    let max_end = *ends.iter().max().unwrap();
    let right: Vec<i64> = ends.iter().map(|e| max_end - e).collect();

    for (i, region) in regions.iter_mut().enumerate() {
        let left = shift[i] as usize;
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
            region.record.seq.extend(std::iter::repeat_n(b'N', right[i] as usize));
        }
    }
}
