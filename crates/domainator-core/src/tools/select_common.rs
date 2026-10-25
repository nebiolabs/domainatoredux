//! Shared helpers for the selection / reporting tools (`select_by_contig`,
//! `select_by_cds`, `enum_report`): the `DomainatorCDS` per-CDS view, the
//! `BooleanEvaluator` for `--domain_expr`, name-list/file parsing
//! (`list_and_file_to_dict_keys`), `get_sources`/`get_non_domainator_features`,
//! and the `--databases` domain filter. Faithful ports of the corresponding
//! `domainator.utils` / `domainator.filter_domains` helpers.

use std::collections::{HashMap, HashSet};

use crate::seq::{SeqFeature, SeqRecord};
use crate::{DOMAIN_FEATURE_NAME, DOMAIN_SEARCH_BEST_HIT_NAME};

/// `cli.common.list_and_file_to_dict_keys(input_list, input_file, as_set=True)`.
/// Returns `None` when both inputs are empty (Python's "no filter"). `file` is
/// additive with `list`, and is read by its extension like upstream's: an HMM file
/// gives its profile names, a FASTA or GenBank file its record ids, and anything else
/// one entry per non-empty line.
pub fn parse_name_list(
    list: &[String],
    file: Option<&str>,
) -> std::io::Result<Option<HashSet<String>>> {
    let mut out: Option<HashSet<String>> = None;
    if !list.is_empty() {
        out = Some(list.iter().cloned().collect());
    }
    if let Some(path) = file {
        let set = out.get_or_insert_with(HashSet::new);
        set.extend(names_in_file(path)?);
    }
    Ok(out)
}

/// The names a `--*_file` option lists (see [`parse_name_list`]).
fn names_in_file(path: &str) -> std::io::Result<Vec<String>> {
    use crate::io::{detect_filetype, open_decompressed, FileType};
    use std::io::{BufRead, BufReader};
    let p = std::path::Path::new(path);
    let lower = path.to_ascii_lowercase();
    let stem = [".gz", ".bgz", ".bgzf"]
        .iter()
        .find_map(|c| lower.strip_suffix(c))
        .unwrap_or(&lower);
    if stem.ends_with(".hmm") || stem.ends_with(".hmmer") {
        // Only the NAME lines, as upstream's `iter_hmm_names` reads them.
        let mut names = Vec::new();
        for line in BufReader::new(open_decompressed(p)?).lines() {
            let line = line?;
            if let Some(rest) = line.strip_prefix("NAME") {
                let name = rest.trim();
                if !name.is_empty() && rest.starts_with(char::is_whitespace) {
                    names.push(name.to_string());
                }
            }
        }
        return Ok(names);
    }
    let records = match detect_filetype(p) {
        Some(FileType::Genbank) => crate::io::genbank::read_genbank(path)?,
        Some(FileType::Fasta) => crate::io::fasta::read_fasta(path, None)?,
        None => {
            let text = std::fs::read_to_string(path)?;
            return Ok(text
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect());
        }
    };
    Ok(records.into_iter().map(|r| r.id).collect())
}

// ---------------------------------------------------------------------------
// BooleanEvaluator
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    LParen,
    RParen,
    And,
    Or,
    Not,
    Operand(String),
}

/// `utils.BooleanEvaluator`: evaluate a boolean expression over domain names
/// using `&` (AND), `|` (OR), `~` (NOT) and parentheses. Operand precedence
/// matches Python's `eval`: `not` > `and` > `or`.
pub struct BooleanEvaluator {
    parts: Vec<Tok>,
    tok_to_domain: HashMap<String, String>,
}

impl BooleanEvaluator {
    pub fn new(expression: &str) -> Self {
        // Tokenize exactly as Python does.
        let mut tokens: Vec<String> = Vec::new();
        let mut elem = String::new();
        for y in expression.chars() {
            match y {
                '&' | '|' => {
                    if !elem.is_empty() {
                        tokens.push(std::mem::take(&mut elem));
                    }
                    tokens.push(y.to_string());
                }
                '~' | '(' => tokens.push(y.to_string()),
                ')' => {
                    tokens.push(std::mem::take(&mut elem));
                    tokens.push(y.to_string());
                }
                ' ' => {}
                _ => elem.push(y),
            }
        }
        if !elem.is_empty() {
            tokens.push(elem);
        }

        // Translate to parts, assigning tok identifiers to operands.
        let mut parts = Vec::new();
        let mut tok_to_domain = HashMap::new();
        let mut i = 0usize;
        for x in tokens {
            match x.as_str() {
                "(" => parts.push(Tok::LParen),
                ")" => parts.push(Tok::RParen),
                "|" => parts.push(Tok::Or),
                "&" => parts.push(Tok::And),
                "~" => parts.push(Tok::Not),
                _ => {
                    let key = format!("tok{i}");
                    tok_to_domain.insert(key.clone(), x);
                    parts.push(Tok::Operand(key));
                    i += 1;
                }
            }
        }
        BooleanEvaluator {
            parts,
            tok_to_domain,
        }
    }

    /// Domain tokens referenced by the expression (used by the taxonomy-expr
    /// compiler, where tokens are taxids).
    pub fn token_values(&self) -> impl Iterator<Item = &String> {
        self.tok_to_domain.values()
    }

    /// True if the expression holds when `domains` is the present set.
    pub fn check_expression(&self, domains: &HashSet<String>) -> bool {
        let mut pos = 0usize;
        self.parse_or(&mut pos, domains)
    }

    fn operand_value(&self, key: &str, domains: &HashSet<String>) -> bool {
        match self.tok_to_domain.get(key) {
            Some(name) => domains.contains(name),
            None => false,
        }
    }

    fn parse_or(&self, pos: &mut usize, domains: &HashSet<String>) -> bool {
        let mut v = self.parse_and(pos, domains);
        while matches!(self.parts.get(*pos), Some(Tok::Or)) {
            *pos += 1;
            let r = self.parse_and(pos, domains);
            v = v || r;
        }
        v
    }

    fn parse_and(&self, pos: &mut usize, domains: &HashSet<String>) -> bool {
        let mut v = self.parse_not(pos, domains);
        while matches!(self.parts.get(*pos), Some(Tok::And)) {
            *pos += 1;
            let r = self.parse_not(pos, domains);
            v = v && r;
        }
        v
    }

    fn parse_not(&self, pos: &mut usize, domains: &HashSet<String>) -> bool {
        let mut negate = false;
        while matches!(self.parts.get(*pos), Some(Tok::Not)) {
            *pos += 1;
            negate = !negate;
        }
        let v = self.parse_primary(pos, domains);
        v ^ negate
    }

    fn parse_primary(&self, pos: &mut usize, domains: &HashSet<String>) -> bool {
        match self.parts.get(*pos) {
            Some(Tok::LParen) => {
                *pos += 1;
                let v = self.parse_or(pos, domains);
                if matches!(self.parts.get(*pos), Some(Tok::RParen)) {
                    *pos += 1;
                }
                v
            }
            Some(Tok::Operand(key)) => {
                let key = key.clone();
                *pos += 1;
                self.operand_value(&key, domains)
            }
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------
// Feature accessors
// ---------------------------------------------------------------------------

/// `utils.get_sources`: all `source` features.
pub fn get_sources(rec: &SeqRecord) -> Vec<&SeqFeature> {
    rec.features.iter().filter(|f| f.kind == "source").collect()
}

/// `utils.get_non_domainator_features`: everything except Domainator /
/// Domain_Search features.
pub fn get_non_domainator_features(rec: &SeqRecord) -> Vec<&SeqFeature> {
    rec.features
        .iter()
        .filter(|f| f.kind != DOMAIN_FEATURE_NAME && f.kind != DOMAIN_SEARCH_BEST_HIT_NAME)
        .collect()
}

/// `utils.get_cds_unique_name`: an existing `cds_id` qualifier, else 1-based
/// strand-aware coordinates joined by spaces.
pub fn get_cds_unique_name(feature: &SeqFeature) -> String {
    crate::tools::neighborhood::cds_unique_name(feature)
}

/// `utils.get_cds_name`: first present qualifier in `precedence`
/// (default `gene_id, locus_tag, gene, protein_id`), else the unique name.
pub fn get_cds_name(feature: &SeqFeature, precedence: Option<&[&str]>) -> String {
    let default: [&str; 4] = ["gene_id", "locus_tag", "gene", "protein_id"];
    let prec = precedence.unwrap_or(&default);
    for p in prec {
        if let Some(v) = feature.get_first(p) {
            return v.to_string();
        }
    }
    get_cds_unique_name(feature)
}

fn feature_evalue(f: &SeqFeature) -> f64 {
    f.get_first("evalue")
        .and_then(|v| v.parse().ok())
        .unwrap_or(f64::INFINITY)
}

/// Whether an annotation's e-value is at most `evalue` (`None`: no cutoff), upstream
/// `model.evalue_passes`, which every `-e/--evalue` filter uses.
pub fn evalue_passes(f: &SeqFeature, evalue: Option<f64>) -> bool {
    evalue.is_none_or(|e| feature_evalue(f) <= e)
}

fn feature_score(f: &SeqFeature) -> f64 {
    f.get_first("score")
        .and_then(|v| v.parse().ok())
        .unwrap_or(f64::NEG_INFINITY)
}

// ---------------------------------------------------------------------------
// DomainatorCDS
// ---------------------------------------------------------------------------

/// A CDS on a contig with its associated Domainator / Domain_Search features —
/// a faithful port of `utils.DomainatorCDS` (owned clones rather than
/// references, since callers re-emit these features into sliced records).
#[derive(Clone)]
pub struct DomainatorCds {
    /// Display name (`get_cds_name`).
    pub name: String,
    /// Unique id (`cds_id` / `get_cds_unique_name`).
    pub num: String,
    /// The CDS feature (or a synthesized pseudo-CDS for a nucleic-acid annotation).
    pub feature: SeqFeature,
    /// Index of `feature` in the contig's `features`; `None` for a synthesized
    /// nucleic-acid pseudo-CDS.
    pub feature_index: Option<usize>,
    pub domain_features: Vec<SeqFeature>,
    /// Index of each of `domain_features` in the contig's `features`.
    pub domain_feature_indices: Vec<usize>,
    pub domain_names: HashSet<String>,
    pub domain_search_feature: Option<SeqFeature>,
    pub is_nucleic_acid: bool,
}

impl DomainatorCds {
    /// `DomainatorCDS.list_from_contig`.
    pub fn list_from_contig(
        contig: &SeqRecord,
        domain_evalue: Option<f64>,
        domain_score: f64,
        skip_pseudo: bool,
        name_precedence: Option<&[&str]>,
        include_nucleic_acid_annotations: bool,
    ) -> Vec<DomainatorCds> {
        let mut cdss: Vec<DomainatorCds> = Vec::new();
        let mut name_to_idx: HashMap<String, usize> = HashMap::new();
        let mut nuc_idx = 0usize;

        for (fi, feature) in contig.features.iter().enumerate() {
            if feature.kind == "CDS" {
                if skip_pseudo && (feature.has("pseudo") || feature.has("pseudogene")) {
                    continue;
                }
                let cds_num = get_cds_unique_name(feature);
                let name = get_cds_name(feature, name_precedence);
                if !name_to_idx.contains_key(&cds_num) {
                    name_to_idx.insert(cds_num.clone(), cdss.len());
                    cdss.push(DomainatorCds {
                        name,
                        num: cds_num,
                        feature: feature.clone(),
                        feature_index: Some(fi),
                        domain_features: Vec::new(),
                        domain_feature_indices: Vec::new(),
                        domain_names: HashSet::new(),
                        domain_search_feature: None,
                        is_nucleic_acid: false,
                    });
                }
            }
        }

        for (fi, feature) in contig.features.iter().enumerate() {
            if feature.kind != DOMAIN_FEATURE_NAME && feature.kind != DOMAIN_SEARCH_BEST_HIT_NAME {
                continue;
            }
            if !(evalue_passes(feature, domain_evalue) && feature_score(feature) > domain_score) {
                continue;
            }
            let cds_id_qual = feature.get_first("cds_id");
            let cds_num: String;
            if include_nucleic_acid_annotations && cds_id_qual == Some(".") {
                cds_num = format!("nuc_{nuc_idx}");
                nuc_idx += 1;
                if !name_to_idx.contains_key(&cds_num) {
                    let name = feature.get_first("name").unwrap_or(&cds_num).to_string();
                    let mut pseudo = SeqFeature::new("CDS", feature.location.clone());
                    pseudo.set("cds_id", cds_num.clone());
                    name_to_idx.insert(cds_num.clone(), cdss.len());
                    cdss.push(DomainatorCds {
                        name,
                        num: cds_num.clone(),
                        feature: pseudo,
                        feature_index: None,
                        domain_features: Vec::new(),
                        domain_feature_indices: Vec::new(),
                        domain_names: HashSet::new(),
                        domain_search_feature: None,
                        is_nucleic_acid: true,
                    });
                }
            } else {
                match cds_id_qual {
                    Some(id) => cds_num = id.to_string(),
                    None => continue,
                }
                if !name_to_idx.contains_key(&cds_num) {
                    continue;
                }
            }

            let idx = name_to_idx[&cds_num];
            if feature.kind == DOMAIN_FEATURE_NAME {
                cdss[idx].domain_features.push(feature.clone());
                cdss[idx].domain_feature_indices.push(fi);
                if let Some(n) = feature.get_first("name") {
                    cdss[idx].domain_names.insert(n.to_string());
                }
            } else {
                cdss[idx].domain_search_feature = Some(feature.clone());
            }
        }

        cdss
    }

    /// `DomainatorCDS.get_domain_names`: domain names, optionally restricted to
    /// `databases` and to e-values at most `evalue`.
    pub fn get_domain_names(
        &self,
        databases: Option<&HashSet<String>>,
        evalue: Option<f64>,
    ) -> HashSet<String> {
        let mut out = HashSet::new();
        for f in &self.domain_features {
            let mut keep = true;
            if let Some(dbs) = databases {
                if !f
                    .get_first("database")
                    .map(|d| dbs.contains(d))
                    .unwrap_or(false)
                {
                    keep = false;
                }
            }
            if !evalue_passes(f, evalue) {
                keep = false;
            }
            if keep {
                if let Some(n) = f.get_first("name") {
                    out.insert(n.to_string());
                }
            }
        }
        out
    }
}

/// `filter_domains(rec, evalue=inf, max_overlap=1, databases_keep=databases)`
/// as used by `enum_report --databases`: drop Domainator / Domain_Search
/// features whose `database` qualifier is not in `databases`; keep all else.
pub fn filter_domains_databases(rec: &mut SeqRecord, databases: &HashSet<String>) {
    rec.features.retain(|f| {
        if f.kind == DOMAIN_FEATURE_NAME || f.kind == DOMAIN_SEARCH_BEST_HIT_NAME {
            f.get_first("database")
                .map(|d| databases.contains(d))
                .unwrap_or(false)
        } else {
            true
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn boolean_evaluator_precedence_and_parens() {
        // not > and > or, with parentheses (matches Python `eval`).
        let e = BooleanEvaluator::new("(CcdB) | (CAT & Condensation)");
        assert!(e.check_expression(&set(&["CcdB"])));
        assert!(e.check_expression(&set(&["CAT", "Condensation"])));
        assert!(!e.check_expression(&set(&["CAT"])));
        assert!(!e.check_expression(&set(&["Condensation"])));
        assert!(!e.check_expression(&set(&["Other"])));
    }

    #[test]
    fn boolean_evaluator_contradiction_and_not() {
        assert!(!BooleanEvaluator::new("CAT&~CAT").check_expression(&set(&["CAT"])));
        assert!(BooleanEvaluator::new("~CAT").check_expression(&set(&["Other"])));
        assert!(!BooleanEvaluator::new("~CAT").check_expression(&set(&["CAT"])));
        // `or` binds looser than `and`: A | B & C == A | (B & C)
        let e = BooleanEvaluator::new("A | B & C");
        assert!(e.check_expression(&set(&["A"])));
        assert!(!e.check_expression(&set(&["B"])));
        assert!(e.check_expression(&set(&["B", "C"])));
    }

    #[test]
    fn boolean_evaluator_token_values_are_operands() {
        let e = BooleanEvaluator::new("2 & ~1224");
        let mut toks: Vec<i64> = e.token_values().filter_map(|t| t.parse().ok()).collect();
        toks.sort();
        assert_eq!(toks, vec![2, 1224]);
    }

    #[test]
    fn parse_name_list_list_only_and_empty() {
        let s = parse_name_list(&["a".into(), "b".into()], None)
            .unwrap()
            .unwrap();
        assert_eq!(s, set(&["a", "b"]));
        assert!(parse_name_list(&[], None).unwrap().is_none());
    }

    fn dom_feat(cds_id: &str, name: &str, start: i64, end: i64, evalue: &str) -> SeqFeature {
        let mut f = SeqFeature::new(
            DOMAIN_FEATURE_NAME,
            crate::seq::Location::range(start, end, 1),
        );
        f.set("cds_id", cds_id);
        f.set("name", name);
        f.set("database", "DB");
        f.set("evalue", evalue);
        f.set("score", "5.0");
        f.set("description", "d");
        f
    }

    fn contig(features: Vec<SeqFeature>) -> SeqRecord {
        SeqRecord {
            id: "c".into(),
            name: "c".into(),
            description: String::new(),
            seq: vec![b'A'; 600],
            molecule_type: Some("DNA".into()),
            topology: crate::seq::Topology::Linear,
            annotations: crate::seq::Annotations::default(),
            features,
        }
    }

    #[test]
    fn list_from_contig_groups_domains_and_nucleic_pseudo() {
        let mut cds = SeqFeature::new("CDS", crate::seq::Location::range(0, 300, 1));
        cds.set("cds_id", "C1");
        cds.set("gene_id", "geneA");
        let rec = contig(vec![
            cds,
            dom_feat("C1", "D1", 0, 150, "1e-9"),
            dom_feat("C1", "D2", 150, 300, "1e-3"),
            dom_feat(".", "Nuc", 400, 500, "1e-2"),
        ]);
        let cdss =
            DomainatorCds::list_from_contig(&rec, None, f64::NEG_INFINITY, false, None, true);
        assert_eq!(cdss.len(), 2);
        assert_eq!(cdss[0].num, "C1");
        assert_eq!(cdss[0].name, "geneA"); // gene_id precedence
        assert_eq!(cdss[0].domain_names, set(&["D1", "D2"]));
        assert!(!cdss[0].is_nucleic_acid);
        assert_eq!(cdss[1].num, "nuc_0");
        assert!(cdss[1].is_nucleic_acid);
        assert_eq!(cdss[1].name, "Nuc");
    }

    #[test]
    fn list_from_contig_evalue_filter_and_db_filter() {
        let mut cds = SeqFeature::new("CDS", crate::seq::Location::range(0, 300, 1));
        cds.set("cds_id", "C1");
        let rec = contig(vec![
            cds,
            dom_feat("C1", "D1", 0, 150, "1e-9"),
            dom_feat("C1", "D2", 150, 300, "1e-3"),
        ]);
        // Only domains with evalue <= 1e-5 survive (D1).
        let cdss =
            DomainatorCds::list_from_contig(&rec, Some(1e-5), f64::NEG_INFINITY, false, None, true);
        assert_eq!(cdss[0].domain_names, set(&["D1"]));
        // get_domain_names db restriction.
        let full =
            DomainatorCds::list_from_contig(&rec, None, f64::NEG_INFINITY, false, None, true);
        assert_eq!(
            full[0].get_domain_names(Some(&set(&["DB"])), None),
            set(&["D1", "D2"])
        );
        assert!(full[0]
            .get_domain_names(Some(&set(&["OTHER"])), None)
            .is_empty());
    }
}
