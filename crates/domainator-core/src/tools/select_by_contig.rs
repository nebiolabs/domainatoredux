//! `select_by_contig`: emit the subset of input contigs matching selection
//! criteria. Faithful port of `domainator.select_by_contig.select_by_contig`.
//!
//! Selection is:
//! ```text
//! (length in [lb,ub]) AND (cds_count in [lb,ub]) AND (taxonomy) AND
//! INVERT( name-in-list OR name-regex OR definition-regex OR sequence-regex
//!         OR has-target-domain OR domain-expr OR unannotated OR search-score )
//! AND (count <= first)
//! ```
//! Contig names are [`SeqRecord::id`] (GenBank LOCUS or FASTA header word).

use std::collections::HashSet;

use regex::Regex;

use crate::seq::taxid::record_taxid;
use crate::seq::taxonomy::NCBITaxonomy;
use crate::seq::SeqRecord;
use crate::tools::select_common::BooleanEvaluator;
use crate::{DOMAIN_FEATURE_NAME, DOMAIN_SEARCH_BEST_HIT_NAME};

#[derive(Default)]
pub struct SelectByContigOptions {
    pub target_domains: Option<HashSet<String>>,
    pub domain_expr: Option<String>,
    pub domain_evalue: f64,
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
    pub include_taxids: Option<HashSet<i64>>,
    pub exclude_taxids: Option<HashSet<i64>>,
    pub taxonomy_expr: Option<String>,
    pub databases: Option<HashSet<String>>,
    pub unannotated: bool,
    pub search_score_lb: Option<f64>,
    pub search_score_ub: Option<f64>,
    pub domain_type: String,
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
        Some(dbs) => f.get_first("database").map(|d| dbs.contains(d)).unwrap_or(false),
    }
}

fn feat_evalue(f: &crate::seq::SeqFeature) -> f64 {
    f.get_first("evalue").and_then(|v| v.parse().ok()).unwrap_or(f64::INFINITY)
}

/// Run the selection. `ncbi_taxonomy` is required iff a taxonomy filter is set.
pub fn select_by_contig(
    records: Vec<SeqRecord>,
    ncbi_taxonomy: Option<&NCBITaxonomy>,
    opts: &SelectByContigOptions,
) -> Result<Vec<SeqRecord>, String> {
    let expr = opts.domain_expr.as_deref().map(BooleanEvaluator::new);
    let def_re = compile(opts.definition_regex.as_deref())?;
    let seq_re = compile(opts.sequence_regex.as_deref())?;
    let name_re = compile(opts.contigs_regex.as_deref())?;
    let domain_type = if opts.domain_type.is_empty() { "domain" } else { opts.domain_type.as_str() };

    // Taxonomy filter compiled to an allowed-taxid set (O(1) membership).
    let allowed: Option<HashSet<i64>> = {
        let has_tax = opts.include_taxids.as_ref().is_some_and(|s| !s.is_empty())
            || opts.exclude_taxids.as_ref().is_some_and(|s| !s.is_empty())
            || opts.taxonomy_expr.as_deref().is_some_and(|e| !e.trim().is_empty());
        if !has_tax {
            None
        } else {
            let tax = ncbi_taxonomy.ok_or("taxonomy filter requires an NCBI taxonomy database")?;
            if opts.taxonomy_expr.as_deref().is_some_and(|e| !e.trim().is_empty())
                && (opts.include_taxids.as_ref().is_some_and(|s| !s.is_empty())
                    || opts.exclude_taxids.as_ref().is_some_and(|s| !s.is_empty()))
            {
                return Err(
                    "--taxonomy_expr is mutually exclusive with --include_taxids/--exclude_taxids."
                        .into(),
                );
            }
            if let Some(e) = opts.taxonomy_expr.as_deref().filter(|e| !e.trim().is_empty()) {
                tax.compile_taxonomy_expression(e)
            } else {
                tax.compile_taxonomy_filter(opts.include_taxids.as_ref(), opts.exclude_taxids.as_ref())
            }
        }
    };

    let search_score_lb = opts.search_score_lb;
    let search_score_ub = opts.search_score_ub;
    let use_search_score = search_score_lb.is_some() || search_score_ub.is_some();
    let score_lb = search_score_lb.unwrap_or(f64::NEG_INFINITY);
    let score_ub = search_score_ub.unwrap_or(f64::INFINITY);

    let mut out = Vec::new();
    for contig in records {
        // Taxonomy pre-filter (matches filter_by_taxonomy applied first).
        if let Some(allowed) = &allowed {
            if !allowed.contains(&record_taxid(&contig)) {
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
            || use_search_score
        {
            at_least_one_test = true;
            let mut domains: HashSet<String> = HashSet::new();
            let mut search_feature: Option<&crate::seq::SeqFeature> = None;
            // Loop-invariant: which feature kinds `--domain_type` draws names from.
            let want_domain = domain_type == "domain" || domain_type == "both";
            let want_search = domain_type == "search" || domain_type == "both";
            for f in &contig.features {
                if f.kind == DOMAIN_FEATURE_NAME
                    && feat_evalue(f) < opts.domain_evalue
                    && db_ok(opts.databases.as_ref(), f)
                    && want_domain
                {
                    if let Some(n) = f.get_first("name") {
                        domains.insert(n.to_string());
                    }
                }
                if f.kind == DOMAIN_SEARCH_BEST_HIT_NAME
                    && feat_evalue(f) < opts.domain_evalue
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
                    let score: f64 =
                        sf.get_first("score").and_then(|v| v.parse().ok()).unwrap_or(f64::NAN);
                    if score >= score_lb && score <= score_ub {
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

fn compile(pattern: Option<&str>) -> Result<Option<Regex>, String> {
    match pattern {
        None => Ok(None),
        Some(p) => Regex::new(p).map(Some).map_err(|e| format!("invalid regex {p:?}: {e}")),
    }
}
