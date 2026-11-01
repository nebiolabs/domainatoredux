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

/// A parsed expression: a name, or a negation, conjunction or disjunction.
#[derive(Debug)]
enum Node {
    Name(String),
    Not(Box<Node>),
    And(Box<Node>, Box<Node>),
    Or(Box<Node>, Box<Node>),
}

const OPERATORS: &str = "&|~()";

/// `expr.BooleanEvaluator`: a boolean expression over names, with `&` (and), `|`
/// (or), `~` (not) and parentheses. `~` binds tightest, then `&`, then `|`, all left
/// associative, as in Python. A name is any run of other non-space characters. The
/// expression is parsed once, by [`BooleanEvaluator::new`], which rejects a
/// malformed or empty one.
#[derive(Debug)]
pub struct BooleanEvaluator {
    tree: Node,
    /// Every name the expression mentions.
    names: HashSet<String>,
}

impl BooleanEvaluator {
    /// Upstream `BooleanEvaluator.sanitize_identifier`: `identifier` with the
    /// characters an expression gives meaning to (and PROSITE's `-`, `<`, `>`, `.`)
    /// replaced, so a name such as a motif pattern can be used in `--domain_expr`.
    pub fn sanitize_identifier(identifier: &str) -> String {
        const REPLACEMENTS: [(&str, &str); 10] = [
            ("(", "__"),
            (")", "___"),
            ("&", "_AND_"),
            ("|", "_OR_"),
            ("~", "_NOT_"),
            (" ", "_"),
            ("-", "_"),
            ("<", "Nterm_"),
            (">", "_Cterm"),
            (".", ""),
        ];
        let mut result = identifier.to_string();
        for (old, new) in REPLACEMENTS {
            result = result.replace(old, new);
        }
        result.trim_end_matches('_').to_string()
    }

    pub fn new(expression: &str) -> Result<Self, String> {
        let mut tokens: Vec<String> = Vec::new();
        let mut name = String::new();
        for c in expression.chars() {
            if OPERATORS.contains(c) || c.is_whitespace() {
                if !name.is_empty() {
                    tokens.push(std::mem::take(&mut name));
                }
                if !c.is_whitespace() {
                    tokens.push(c.to_string());
                }
            } else {
                name.push(c);
            }
        }
        if !name.is_empty() {
            tokens.push(name);
        }
        if tokens.is_empty() {
            return Err(format!("Empty boolean expression: {expression:?}"));
        }
        let mut parser = Parser {
            expression,
            tokens: &tokens,
            pos: 0,
        };
        let tree = parser.parse_or()?;
        if let Some(extra) = parser.peek() {
            return Err(parser.error(&format!("unexpected {extra:?}")));
        }
        let names = tokens.iter().filter(|t| !is_operator(t)).cloned().collect();
        Ok(BooleanEvaluator { tree, names })
    }

    /// Every name the expression mentions (the taxids of a `--taxonomy_expr`).
    pub fn names(&self) -> &HashSet<String> {
        &self.names
    }

    /// True if the expression holds when exactly the names in `present` are true.
    pub fn check_expression(&self, present: &HashSet<String>) -> bool {
        fn evaluate(node: &Node, present: &HashSet<String>) -> bool {
            match node {
                Node::Name(name) => present.contains(name),
                Node::Not(inner) => !evaluate(inner, present),
                Node::And(l, r) => evaluate(l, present) && evaluate(r, present),
                Node::Or(l, r) => evaluate(l, present) || evaluate(r, present),
            }
        }
        evaluate(&self.tree, present)
    }
}

fn is_operator(token: &str) -> bool {
    token.len() == 1 && OPERATORS.contains(token)
}

/// The recursive-descent parser behind [`BooleanEvaluator::new`].
struct Parser<'a> {
    expression: &'a str,
    tokens: &'a [String],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&'a str> {
        self.tokens.get(self.pos).map(String::as_str)
    }

    fn error(&self, problem: &str) -> String {
        format!(
            "Invalid boolean expression {:?}: {problem}.",
            self.expression
        )
    }

    fn parse_or(&mut self) -> Result<Node, String> {
        let mut node = self.parse_and()?;
        while self.peek() == Some("|") {
            self.pos += 1;
            node = Node::Or(Box::new(node), Box::new(self.parse_and()?));
        }
        Ok(node)
    }

    fn parse_and(&mut self) -> Result<Node, String> {
        let mut node = self.parse_not()?;
        while self.peek() == Some("&") {
            self.pos += 1;
            node = Node::And(Box::new(node), Box::new(self.parse_not()?));
        }
        Ok(node)
    }

    fn parse_not(&mut self) -> Result<Node, String> {
        let Some(token) = self.peek() else {
            return Err(self.error("it ends where a name was expected"));
        };
        self.pos += 1;
        match token {
            "~" => Ok(Node::Not(Box::new(self.parse_not()?))),
            "(" => {
                let node = self.parse_or()?;
                if self.peek() != Some(")") {
                    return Err(self.error("a '(' is never closed"));
                }
                self.pos += 1;
                Ok(node)
            }
            t if is_operator(t) => Err(self.error(&format!("{t:?} where a name was expected"))),
            t => Ok(Node::Name(t.to_string())),
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
    let prec = precedence.unwrap_or(&CDS_NAME_FIELDS);
    for p in prec {
        if let Some(v) = feature.get_first(p) {
            return v.to_string();
        }
    }
    get_cds_unique_name(feature)
}

/// `--annotation_type`: which annotations a tool reads (upstream
/// `model.ANNOTATION_TYPES`/`annotation_feature_types`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AnnotationType {
    /// `Domainator` features, from domainate.
    #[default]
    Domainator,
    /// `Domain_Search` best-hit features, from domain_search.
    DomainSearch,
    /// Both.
    All,
}

impl std::str::FromStr for AnnotationType {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "domainator" => Ok(AnnotationType::Domainator),
            "domain_search" => Ok(AnnotationType::DomainSearch),
            "all" => Ok(AnnotationType::All),
            _ => Err(format!(
                "annotation_type must be one of domainator, domain_search, all, not {s:?}."
            )),
        }
    }
}

impl AnnotationType {
    /// Whether a feature of type `kind` is one of these annotations.
    pub fn includes(self, kind: &str) -> bool {
        match self {
            AnnotationType::Domainator => kind == DOMAIN_FEATURE_NAME,
            AnnotationType::DomainSearch => kind == DOMAIN_SEARCH_BEST_HIT_NAME,
            AnnotationType::All => {
                kind == DOMAIN_FEATURE_NAME || kind == DOMAIN_SEARCH_BEST_HIT_NAME
            }
        }
    }
}

/// Upstream `model.cds_name_precedence`: the qualifiers a CDS name is read from,
/// `name_field` first when given, then `default`'s others in order.
pub fn cds_name_precedence<'a>(name_field: Option<&'a str>, default: &[&'a str]) -> Vec<&'a str> {
    name_field
        .into_iter()
        .chain(default.iter().copied().filter(|f| Some(*f) != name_field))
        .collect()
}

/// `get_cds_name`'s default qualifier order.
pub const CDS_NAME_FIELDS: [&str; 4] = ["gene_id", "locus_tag", "gene", "protein_id"];

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

    /// `DomainatorCDS.get_domain_names`: the names of the CDS's `annotation_type`
    /// annotations, optionally restricted to `databases` and to e-values at most
    /// `evalue`.
    pub fn get_domain_names(
        &self,
        databases: Option<&HashSet<String>>,
        evalue: Option<f64>,
        annotation_type: AnnotationType,
    ) -> HashSet<String> {
        let domains = self
            .domain_features
            .iter()
            .filter(|_| annotation_type.includes(DOMAIN_FEATURE_NAME));
        let search_hit = self
            .domain_search_feature
            .iter()
            .filter(|_| annotation_type.includes(DOMAIN_SEARCH_BEST_HIT_NAME));
        domains
            .chain(search_hit)
            .filter(|f| {
                databases.is_none_or(|dbs| f.get_first("database").is_some_and(|d| dbs.contains(d)))
                    && evalue_passes(f, evalue)
            })
            .filter_map(|f| f.get_first("name").map(str::to_string))
            .collect()
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
        let e = BooleanEvaluator::new("(CcdB) | (CAT & Condensation)").unwrap();
        assert!(e.check_expression(&set(&["CcdB"])));
        assert!(e.check_expression(&set(&["CAT", "Condensation"])));
        assert!(!e.check_expression(&set(&["CAT"])));
        assert!(!e.check_expression(&set(&["Condensation"])));
        assert!(!e.check_expression(&set(&["Other"])));
    }

    #[test]
    fn boolean_evaluator_contradiction_and_not() {
        let check = |e: &str, present: &[&str]| {
            BooleanEvaluator::new(e)
                .unwrap()
                .check_expression(&set(present))
        };
        assert!(!check("CAT&~CAT", &["CAT"]));
        assert!(check("~CAT", &["Other"]));
        assert!(!check("~CAT", &["CAT"]));
        assert!(check("~~CAT", &["CAT"]));
        // `or` binds looser than `and`: A | B & C == A | (B & C)
        let e = BooleanEvaluator::new("A | B & C").unwrap();
        assert!(e.check_expression(&set(&["A"])));
        assert!(!e.check_expression(&set(&["B"])));
        assert!(e.check_expression(&set(&["B", "C"])));
    }

    #[test]
    fn boolean_evaluator_names_are_operands() {
        let e = BooleanEvaluator::new("2 & ~1224").unwrap();
        let mut names: Vec<&str> = e.names().iter().map(String::as_str).collect();
        names.sort_unstable();
        assert_eq!(names, ["1224", "2"]);
        // A name is any run of characters other than operators and whitespace.
        let e = BooleanEvaluator::new("PF00001.2&a-b").unwrap();
        assert!(e.check_expression(&set(&["PF00001.2", "a-b"])));
    }

    /// Upstream rejects a malformed expression when it is parsed, instead of
    /// evaluating whatever part of it makes sense.
    #[test]
    fn boolean_evaluator_rejects_malformed_expressions() {
        for bad in ["", "  ", "A &", "(A | B", "A B", "A | | B", ")A", "A)", "~"] {
            assert!(BooleanEvaluator::new(bad).is_err(), "{bad:?}");
        }
        let err = BooleanEvaluator::new("(A | B").unwrap_err();
        assert_eq!(
            err,
            "Invalid boolean expression \"(A | B\": a '(' is never closed."
        );
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
            full[0].get_domain_names(Some(&set(&["DB"])), None, AnnotationType::Domainator),
            set(&["D1", "D2"])
        );
        assert!(full[0]
            .get_domain_names(Some(&set(&["OTHER"])), None, AnnotationType::Domainator)
            .is_empty());
    }
}
