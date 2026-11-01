//! `select_by_contig`: emit the subset of input contigs matching selection
//! criteria. Faithful port of `domainator.select_by_contig.select_by_contig`.
//!
//! Selection is:
//! ```text
//! (length in [lb,ub]) AND (cds_count in [lb,ub]) AND (partial) AND
//! INVERT( name-in-list OR name-regex OR definition-regex OR sequence-regex
//!         OR has-target-domain OR domain-expr OR unannotated OR has-search-hit
//!         OR search-score )
//! AND (taxonomy) AND (count <= first)
//! ```
//! Contig names are [`SeqRecord::id`] (GenBank LOCUS or FASTA header word).

use std::collections::HashSet;

use regex::Regex;

use crate::seq::partial::{contig_has_fragment, Partial};
use crate::seq::taxonomy::TaxonomyFilter;
use crate::seq::SeqRecord;
pub use crate::tools::select_common::AnnotationType;
use crate::tools::select_common::{evalue_passes, BooleanEvaluator};
use crate::{DOMAIN_FEATURE_NAME, DOMAIN_SEARCH_BEST_HIT_NAME};

#[derive(Default)]
pub struct SelectByContigOptions {
    pub target_domains: Option<HashSet<String>>,
    pub domain_expr: Option<String>,
    /// Only annotations with an e-value at most this count (`None`: no cutoff).
    pub domain_evalue: Option<f64>,
    pub first: Option<usize>,
    pub length_lb: Option<i64>,
    pub length_ub: Option<i64>,
    pub cds_count_lb: Option<i64>,
    pub cds_count_ub: Option<i64>,
    pub definition_regex: Option<String>,
    pub sequence_regex: Option<String>,
    pub target_contigs: Option<HashSet<String>>,
    pub contigs_regex: Option<String>,
    pub invert: bool,
    /// Taxonomy restriction on the selected records; never inverted.
    pub taxonomy: Option<TaxonomyFilter>,
    pub databases: Option<HashSet<String>>,
    pub unannotated: bool,
    /// `--min_search_score`/`--max_search_score`: also select records whose
    /// `Domain_Search` hit scores in this inclusive range.
    pub search_score_lb: Option<f64>,
    pub search_score_ub: Option<f64>,
    pub annotation_type: AnnotationType,
    /// `--has_search_hit`: also select records with a search hit passing the e-value
    /// (and score range, when one is given), whatever its database.
    pub has_search_hit: bool,
    /// `--partial`: contigs by fragment status ([`contig_has_fragment`]); never
    /// inverted by `invert`.
    pub partial: Partial,
}

/// `select_by_contig.count_cds`: CDS features that are not pseudogenes.
fn count_cds(contig: &SeqRecord) -> i64 {
    contig
        .features
        .iter()
        .filter(|f| f.kind == "CDS" && !(f.has("pseudo") || f.has("pseudogene")))
        .count() as i64
}

fn db_ok(databases: Option<&HashSet<String>>, f: &crate::seq::SeqFeature) -> bool {
    match databases {
        None => true,
        Some(dbs) => f
            .get_first("database")
            .map(|d| dbs.contains(d))
            .unwrap_or(false),
    }
}

/// Run the selection.
pub fn select_by_contig(
    records: Vec<SeqRecord>,
    opts: &SelectByContigOptions,
) -> Result<Vec<SeqRecord>, String> {
    let expr = opts
        .domain_expr
        .as_deref()
        .map(BooleanEvaluator::new)
        .transpose()?;
    let def_re = compile(opts.definition_regex.as_deref())?;
    let seq_re = compile(opts.sequence_regex.as_deref())?;
    let name_re = compile(opts.contigs_regex.as_deref())?;

    let search_score_lb = opts.search_score_lb;
    let search_score_ub = opts.search_score_ub;
    let use_search_score = search_score_lb.is_some() || search_score_ub.is_some();
    let score_lb = search_score_lb.unwrap_or(f64::NEG_INFINITY);
    let score_ub = search_score_ub.unwrap_or(f64::INFINITY);

    let mut out = Vec::new();
    for contig in records {
        // Fragment filter (filter_by_partial), applied before the others.
        if !opts.partial.is_include() {
            let status = contig_has_fragment(&contig).then_some("?");
            if !opts.partial.allows(status) {
                continue;
            }
        }
        // Length filter.
        if opts.length_lb.is_some() || opts.length_ub.is_some() {
            let size = contig.len() as i64;
            let lb = opts.length_lb.unwrap_or(0);
            let ub = opts.length_ub.unwrap_or(i64::MAX);
            if size < lb || size > ub {
                continue;
            }
        }
        // CDS-count filter.
        if opts.cds_count_lb.is_some() || opts.cds_count_ub.is_some() {
            let n = count_cds(&contig);
            let lb = opts.cds_count_lb.unwrap_or(0);
            let ub = opts.cds_count_ub.unwrap_or(i64::MAX);
            if n < lb || n > ub {
                continue;
            }
        }

        let mut selected = false;
        let mut at_least_one_test = false;

        if let Some(set) = &opts.target_contigs {
            at_least_one_test = true;
            if set.contains(&contig.id) {
                selected = true;
            }
        }
        if let Some(re) = &name_re {
            at_least_one_test = true;
            if re.is_match(&contig.id) {
                selected = true;
            }
        }
        if let Some(re) = &def_re {
            at_least_one_test = true;
            if re.is_match(&contig.description) {
                selected = true;
            }
        }
        if let Some(re) = &seq_re {
            at_least_one_test = true;
            if re.is_match(&String::from_utf8_lossy(&contig.seq)) {
                selected = true;
            }
        }

        if opts.target_domains.is_some()
            || opts.domain_expr.is_some()
            || opts.unannotated
            || opts.has_search_hit
            || use_search_score
        {
            at_least_one_test = true;
            let mut domains: HashSet<String> = HashSet::new();
            let mut search_feature: Option<&crate::seq::SeqFeature> = None;
            // Loop-invariant: which feature kinds `--annotation_type` draws names from.
            let want_domain = opts.annotation_type.includes(DOMAIN_FEATURE_NAME);
            let want_search = opts.annotation_type.includes(DOMAIN_SEARCH_BEST_HIT_NAME);
            for f in &contig.features {
                if opts.has_search_hit
                    && f.kind == DOMAIN_SEARCH_BEST_HIT_NAME
                    && evalue_passes(f, opts.domain_evalue)
                    && (!use_search_score || (score_lb..=score_ub).contains(&feature_score(f)))
                {
                    selected = true;
                }
                if f.kind == DOMAIN_FEATURE_NAME
                    && evalue_passes(f, opts.domain_evalue)
                    && db_ok(opts.databases.as_ref(), f)
                    && want_domain
                {
                    if let Some(n) = f.get_first("name") {
                        domains.insert(n.to_string());
                    }
                }
                if f.kind == DOMAIN_SEARCH_BEST_HIT_NAME
                    && evalue_passes(f, opts.domain_evalue)
                    && db_ok(opts.databases.as_ref(), f)
                {
                    search_feature = Some(f);
                    if want_search {
                        if let Some(n) = f.get_first("name") {
                            domains.insert(n.to_string());
                        }
                    }
                }
            }

            if use_search_score {
                if let Some(sf) = search_feature {
                    if (score_lb..=score_ub).contains(&feature_score(sf)) {
                        selected = true;
                    }
                }
            }
            if let Some(td) = &opts.target_domains {
                if td.intersection(&domains).next().is_some() {
                    selected = true;
                }
            }
            if let Some(e) = &expr {
                if e.check_expression(&domains) {
                    selected = true;
                }
            }
            if opts.unannotated && domains.is_empty() {
                selected = true;
            }
        }

        if !at_least_one_test {
            selected = true;
        }
        if opts.invert {
            selected = !selected;
        }
        if selected {
            if let Some(taxonomy) = &opts.taxonomy {
                if !taxonomy.matches(&contig, None)? {
                    continue;
                }
            }
            out.push(contig);
            if let Some(first) = opts.first {
                if out.len() == first {
                    break;
                }
            }
        }
    }
    Ok(out)
}

/// A feature's `score` (NaN when it has none, which no range contains).
fn feature_score(f: &crate::seq::SeqFeature) -> f64 {
    f.get_first("score")
        .and_then(|v| v.parse().ok())
        .unwrap_or(f64::NAN)
}

fn compile(pattern: Option<&str>) -> Result<Option<Regex>, String> {
    match pattern {
        None => Ok(None),
        Some(p) => Regex::new(p)
            .map(Some)
            .map_err(|e| format!("invalid regex {p:?}: {e}")),
    }
}
