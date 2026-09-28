//! `enum_report`: write a TSV table describing each contig / CDS / domain of a
//! Domainator-annotated GenBank (or FASTA) file. Faithful port of
//! `domainator.enum_report` (TSV output; the HTML writer is out of scope).

use std::collections::HashSet;
use std::io::Write;

use crate::seq::location::Location;
use crate::seq::protein_analysis::{self, SeqType};
use crate::seq::taxid::record_taxid;
use crate::seq::taxonomy::{NCBITaxonomy, TaxonomyData};
use crate::seq::{SeqFeature, SeqRecord};
use crate::tools::neighborhood::{slice_record, SliceLoc};
use crate::tools::select_common::{filter_domains_databases, get_sources, DomainatorCds};
use crate::{DOMAIN_FEATURE_NAME, DOMAIN_SEARCH_BEST_HIT_NAME};

#[derive(Clone, Copy, PartialEq)]
pub enum By {
    Contig,
    Cds,
    Domain,
}

#[derive(Clone, Copy, PartialEq)]
pub enum ColType {
    Str,
    Int,
    Float,
}

/// A computed cell value.
pub enum Value {
    Str(String),
    Int(i64),
    Float(f64),
    Null,
}

/// One requested analysis: the columns it contributes and how to compute them.
pub struct Analysis {
    pub columns: Vec<(String, ColType)>,
    kind: Kind,
}

enum Kind {
    Domains,
    DomainDescriptions,
    Sequence,
    Start,
    End,
    Strand,
    Length,
    Topology,
    Architecture,
    ArchitectureDetailed,
    Score,
    Identity,
    Definition,
    Filename,
    CdsCount,
    DomainCount,
    DomainSearch,
    TaxidLineage,
    TaxnameLineage,
    RankLineage,
    /// The record's own taxid (from the source `db_xref`; no taxonomy DB needed).
    TaxidSelf,
    /// The record's own organism name (from the ORGANISM annotation).
    TaxnameSelf,
    Taxid(Vec<String>),
    Taxname(Vec<String>),
    Qualifier(String, String),
    FeatureCount(String),
    Append(Value),
    MotifCount(String),
    NetCharge(f64),
    RepeatCount(i64),
    NamedDomainCount(String),
    IsoelectricPoint,
    StartsWith,
    MolecularWeight,
}

/// Build a static (no-argument) analysis from its flag name. Returns `None`
/// for an unknown name.
pub fn static_analysis(name: &str) -> Option<Analysis> {
    let one = |n: &str, t: ColType, k: Kind| Analysis {
        columns: vec![(n.to_string(), t)],
        kind: k,
    };
    Some(match name {
        "domains" => one("domains", ColType::Str, Kind::Domains),
        "domain_descriptions" => one(
            "domain_descriptions",
            ColType::Str,
            Kind::DomainDescriptions,
        ),
        "sequence" => one("sequence", ColType::Str, Kind::Sequence),
        "start" => one("start", ColType::Int, Kind::Start),
        "end" => one("end", ColType::Int, Kind::End),
        "strand" => one("strand", ColType::Str, Kind::Strand),
        "length" => one("length", ColType::Int, Kind::Length),
        "topology" => one("topology", ColType::Str, Kind::Topology),
        "architecture" => one("architecture", ColType::Str, Kind::Architecture),
        "architecture_detailed" => one(
            "architecture_detailed",
            ColType::Str,
            Kind::ArchitectureDetailed,
        ),
        "score" => one("score", ColType::Float, Kind::Score),
        "identity" => one("identity", ColType::Float, Kind::Identity),
        "definition" => one("definition", ColType::Str, Kind::Definition),
        "filename" => one("filename", ColType::Str, Kind::Filename),
        "cds_count" => one("cds_count", ColType::Int, Kind::CdsCount),
        "domain_count" => one("domain_count", ColType::Int, Kind::DomainCount),
        "taxid_lineage" => one("taxid_lineage", ColType::Str, Kind::TaxidLineage),
        "taxname_lineage" => one("taxname_lineage", ColType::Str, Kind::TaxnameLineage),
        "rank_lineage" => one("rank_lineage", ColType::Str, Kind::RankLineage),
        "taxid_self" => one("taxid_self", ColType::Int, Kind::TaxidSelf),
        "taxname_self" => one("taxname_self", ColType::Str, Kind::TaxnameSelf),
        "isoelectric_point" => one("isoelectric_point", ColType::Float, Kind::IsoelectricPoint),
        "starts_with" => one("starts_with", ColType::Str, Kind::StartsWith),
        "molecular_weight" => one("molecular_weight", ColType::Float, Kind::MolecularWeight),
        "domain_search" => Analysis {
            columns: vec![
                ("best_query".into(), ColType::Str),
                ("query_description".into(), ColType::Str),
                ("query_start".into(), ColType::Int),
                ("query_end".into(), ColType::Int),
                ("query_length".into(), ColType::Int),
                ("match_start".into(), ColType::Int),
                ("match_end".into(), ColType::Int),
                ("match_strand".into(), ColType::Str),
                ("match_score".into(), ColType::Float),
                ("match_identity".into(), ColType::Float),
            ],
            kind: Kind::DomainSearch,
        },
        _ => return None,
    })
}

pub fn taxid_analysis(ranks: Vec<String>) -> Analysis {
    let columns = ranks
        .iter()
        .map(|r| (format!("taxid_{r}"), ColType::Int))
        .collect();
    Analysis {
        columns,
        kind: Kind::Taxid(ranks),
    }
}
pub fn taxname_analysis(ranks: Vec<String>) -> Analysis {
    let columns = ranks
        .iter()
        .map(|r| (format!("taxname_{r}"), ColType::Str))
        .collect();
    Analysis {
        columns,
        kind: Kind::Taxname(ranks),
    }
}
pub fn qualifier_analysis(ftype: String, qual: String) -> Analysis {
    Analysis {
        columns: vec![(format!("qualifier_{ftype}_{qual}"), ColType::Str)],
        kind: Kind::Qualifier(ftype, qual),
    }
}
pub fn feature_count_analysis(ftype: String) -> Analysis {
    Analysis {
        columns: vec![(format!("{ftype}_count"), ColType::Int)],
        kind: Kind::FeatureCount(ftype),
    }
}
pub fn append_analysis(name: String, ctype: &str, value: &str) -> Result<Analysis, String> {
    let (t, v) = match ctype {
        "int" => (
            ColType::Int,
            Value::Int(
                value
                    .parse()
                    .map_err(|_| format!("append int value {value:?}"))?,
            ),
        ),
        "float" => (
            ColType::Float,
            Value::Float(
                value
                    .parse()
                    .map_err(|_| format!("append float value {value:?}"))?,
            ),
        ),
        "str" => (ColType::Str, Value::Str(value.to_string())),
        _ => return Err("append spec must be one of int, float, str".into()),
    };
    Ok(Analysis {
        columns: vec![(name, t)],
        kind: Kind::Append(v),
    })
}
pub fn motif_count_analysis(pattern: String) -> Analysis {
    let safe = pattern
        .replace('-', "_")
        .replace(['[', ']', '{', '}', '(', ')'], "")
        .replace(',', "_");
    Analysis {
        columns: vec![(format!("motif_count_{safe}"), ColType::Int)],
        kind: Kind::MotifCount(pattern),
    }
}
pub fn net_charge_analysis(ph_str: &str) -> Result<Analysis, String> {
    let ph: f64 = ph_str
        .parse()
        .map_err(|_| format!("net_charge pH {ph_str:?}"))?;
    Ok(Analysis {
        columns: vec![(format!("net_charge_{ph_str}"), ColType::Float)],
        kind: Kind::NetCharge(ph),
    })
}
pub fn repeat_count_analysis(min_len_str: &str) -> Result<Analysis, String> {
    let n: i64 = min_len_str
        .parse()
        .map_err(|_| format!("repeat_count length {min_len_str:?}"))?;
    Ok(Analysis {
        columns: vec![(format!("repeat_count_{n}"), ColType::Int)],
        kind: Kind::RepeatCount(n),
    })
}
pub fn named_domain_count_analysis(domain: String) -> Analysis {
    let mut safe: String = domain
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    // collapse runs of '_' and trim, matching re.sub(r"[^A-Za-z0-9]+","_").strip("_")
    while safe.contains("__") {
        safe = safe.replace("__", "_");
    }
    let safe = safe.trim_matches('_').to_string();
    let safe = if safe.is_empty() {
        "domain".to_string()
    } else {
        safe
    };
    Analysis {
        columns: vec![(format!("named_domain_count_{safe}"), ColType::Int)],
        kind: Kind::NamedDomainCount(domain),
    }
}

// --- per-record computation helpers ----------------------------------------

fn feat_f64(f: &SeqFeature, key: &str, default: f64) -> f64 {
    f.get_first(key)
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn get_strand(loc: &Location) -> &'static str {
    if loc.strand() == -1 {
        "-"
    } else {
        "+"
    }
}

fn stranded_start_hr(loc: &Location) -> i64 {
    if loc.strand() == -1 {
        loc.end()
    } else {
        loc.start() + 1
    }
}
fn stranded_end_hr(loc: &Location) -> i64 {
    if loc.strand() == -1 {
        loc.start() + 1
    } else {
        loc.end()
    }
}

/// `get_domain_content`: alphabetically sorted unique `(name, description)`.
fn get_domain_content(rec: &SeqRecord) -> (String, String) {
    let mut set: HashSet<(String, String)> = HashSet::new();
    for f in &rec.features {
        if f.kind == DOMAIN_FEATURE_NAME {
            let name = f.get_first("name").unwrap_or("").to_string();
            let desc = f.get_first("description").unwrap_or("").to_string();
            set.insert((name, desc));
        }
    }
    let mut v: Vec<(String, String)> = set.into_iter().collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    let names: Vec<String> = v.iter().map(|x| x.0.clone()).collect();
    let descs: Vec<String> = v.iter().map(|x| x.1.clone()).collect();
    (names.join("; "), descs.join("; "))
}

/// `get_domain_architecture`: Domainator names in ascending location-start order.
fn get_domain_architecture(rec: &SeqRecord) -> String {
    let mut feats: Vec<&SeqFeature> = rec
        .features
        .iter()
        .filter(|f| f.kind == DOMAIN_FEATURE_NAME)
        .collect();
    feats.sort_by_key(|f| f.location.start());
    feats
        .iter()
        .map(|f| f.get_first("name").unwrap_or("").to_string())
        .collect::<Vec<_>>()
        .join("; ")
}

fn get_domain_architecture_detailed(rec: &SeqRecord) -> String {
    let no_domains = "no_domains";
    if rec.is_protein() {
        let mut feats: Vec<&SeqFeature> = rec
            .features
            .iter()
            .filter(|f| f.kind == DOMAIN_FEATURE_NAME)
            .collect();
        if feats.is_empty() {
            return no_domains.to_string();
        }
        feats.sort_by_key(|f| f.location.start());
        return feats
            .iter()
            .map(|f| f.get_first("name").unwrap_or(""))
            .collect::<Vec<_>>()
            .join("/");
    }
    // nucleic acid
    let mut cdss =
        DomainatorCds::list_from_contig(rec, f64::INFINITY, f64::NEG_INFINITY, true, None, true);
    cdss.sort_by_key(|c| stranded_start_loc(&c.feature.location, rec.seq.len() as i64));
    let mut parts: Vec<String> = Vec::new();
    for cds in &cdss {
        if cds.domain_features.is_empty() {
            parts.push(no_domains.to_string());
        } else {
            let mut dom = cds.domain_features.clone();
            dom.sort_by_key(|f| stranded_start_loc(&f.location, rec.seq.len() as i64));
            if cds.feature.location.strand() == -1 {
                dom.reverse();
            }
            parts.push(
                dom.iter()
                    .map(|f| f.get_first("name").unwrap_or(""))
                    .collect::<Vec<_>>()
                    .join("/"),
            );
        }
    }
    parts.join("; ")
}

/// Stranded start for sorting (Biopython `location.stranded_start`): linear case.
fn stranded_start_loc(loc: &Location, _l: i64) -> i64 {
    if loc.strand() == -1 {
        loc.end()
    } else {
        loc.start()
    }
}

fn count_feature(rec: &SeqRecord, ftype: &str) -> i64 {
    rec.features
        .iter()
        .filter(|f| {
            f.kind == ftype && !(ftype == "CDS" && (f.has("pseudo") || f.has("pseudogene")))
        })
        .count() as i64
}

fn single_protein_sequence(rec: &SeqRecord) -> Option<Vec<u8>> {
    if rec.is_protein() {
        let mut s = rec.seq.clone();
        while s.last() == Some(&b'*') {
            s.pop();
        }
        return Some(s);
    }
    let cdss =
        DomainatorCds::list_from_contig(rec, f64::INFINITY, f64::NEG_INFINITY, true, None, false);
    if cdss.len() == 1 {
        let cds = &cdss[0];
        let pep = match cds.feature.get_first("translation") {
            Some(t) if !t.is_empty() => t.to_string(),
            _ => crate::seq::translate::translate_dna(&extract_cds_seq(rec, &cds.feature)),
        };
        let mut bytes = pep.into_bytes();
        while bytes.last() == Some(&b'*') {
            bytes.pop();
        }
        return Some(bytes);
    }
    None
}

/// Spliced coding nucleotides for a CDS feature.
fn extract_cds_seq(rec: &SeqRecord, cds: &SeqFeature) -> Vec<u8> {
    let mut out = Vec::new();
    for &(s, e, strand, _, _) in &cds.location.parts {
        let (lo, hi) = if s <= e { (s, e) } else { (e, s) };
        let sub = &rec.seq[lo as usize..hi as usize];
        if strand == -1 {
            out.extend(crate::seq::translate::revcomp(sub));
        } else {
            out.extend_from_slice(sub);
        }
    }
    out
}

/// Build a [`SliceLoc`] from a feature `Location` (preserving part order).
fn slice_loc_from(loc: &Location) -> SliceLoc {
    let parts = loc
        .parts
        .iter()
        .map(|&(s, e, strand, _, _)| {
            let (lo, hi) = if s <= e { (s, e) } else { (e, s) };
            crate::tools::neighborhood::Env {
                start: lo,
                end: hi,
                strand,
            }
        })
        .collect();
    SliceLoc {
        parts,
        name_start: 0,
        name_end: 0,
        overall_strand: loc.strand(),
        dist_to_start: 0,
        dist_to_end: 0,
    }
}

struct Ctx<'a> {
    loc: Location,
    tax: Option<&'a TaxonomyData>,
    filename: &'a str,
}

impl Analysis {
    fn compute(&self, rec: &SeqRecord, ctx: &Ctx<'_>) -> Vec<Value> {
        match &self.kind {
            Kind::Domains => vec![Value::Str(get_domain_content(rec).0)],
            Kind::DomainDescriptions => vec![Value::Str(get_domain_content(rec).1)],
            Kind::Sequence => vec![Value::Str(String::from_utf8_lossy(&rec.seq).into_owned())],
            Kind::Start => vec![Value::Int(stranded_start_hr(&ctx.loc))],
            Kind::End => vec![Value::Int(stranded_end_hr(&ctx.loc))],
            Kind::Strand => vec![Value::Str(get_strand(&ctx.loc).to_string())],
            Kind::Length => vec![Value::Int(rec.seq.len() as i64)],
            Kind::Topology => vec![Value::Str(rec.topology.as_str().to_string())],
            Kind::Architecture => vec![Value::Str(get_domain_architecture(rec))],
            Kind::ArchitectureDetailed => vec![Value::Str(get_domain_architecture_detailed(rec))],
            Kind::Score => {
                let s: f64 = rec
                    .features
                    .iter()
                    .filter(|f| f.kind == DOMAIN_FEATURE_NAME)
                    .map(|f| feat_f64(f, "score", 0.0))
                    .sum();
                vec![Value::Float(s)]
            }
            Kind::Identity => {
                let s: f64 = rec
                    .features
                    .iter()
                    .filter(|f| f.kind == DOMAIN_FEATURE_NAME)
                    .map(|f| feat_f64(f, "identity", 0.0))
                    .sum();
                vec![Value::Float(s)]
            }
            Kind::Definition => vec![Value::Str(rec.description.clone())],
            Kind::Filename => vec![Value::Str(ctx.filename.to_string())],
            Kind::CdsCount => vec![Value::Int(count_feature(rec, "CDS"))],
            Kind::DomainCount => vec![Value::Int(count_feature(rec, DOMAIN_FEATURE_NAME))],
            Kind::DomainSearch => domain_search(rec),
            Kind::TaxidLineage => vec![Value::Str(lineage_str(ctx.tax, |t| {
                t.lineage.iter().skip(1).map(|x| x.to_string()).collect()
            }))],
            Kind::TaxnameLineage => {
                vec![Value::Str(lineage_str(ctx.tax, |t| {
                    t.names.iter().skip(1).cloned().collect()
                }))]
            }
            Kind::RankLineage => {
                vec![Value::Str(lineage_str(ctx.tax, |t| {
                    t.ranks.iter().skip(1).cloned().collect()
                }))]
            }
            Kind::TaxidSelf => vec![Value::Int(record_taxid(rec))],
            Kind::TaxnameSelf => vec![match &rec.annotations.organism {
                Some(o) => Value::Str(o.clone()),
                None => Value::Null,
            }],
            Kind::Taxid(ranks) => ranks
                .iter()
                .map(|r| match ctx.tax.and_then(|t| t.taxid_at(r)) {
                    Some(id) => Value::Int(id),
                    None => Value::Null,
                })
                .collect(),
            Kind::Taxname(ranks) => ranks
                .iter()
                .map(|r| match ctx.tax.and_then(|t| t.name_at(r)) {
                    Some(n) => Value::Str(n.to_string()),
                    None => Value::Null,
                })
                .collect(),
            Kind::Qualifier(ftype, qual) => {
                let mut feats: Vec<&SeqFeature> = rec.features.iter().collect();
                feats.sort_by_key(|f| stranded_start_loc(&f.location, rec.seq.len() as i64));
                let mut hits = Vec::new();
                for f in feats {
                    if f.kind == *ftype {
                        if f.kind == "CDS" && f.has("pseudo") && f.has("pseudogene") {
                            continue;
                        }
                        if let Some(v) = f.get_first(qual) {
                            hits.push(v.to_string());
                        }
                    }
                }
                vec![Value::Str(hits.join("; "))]
            }
            Kind::FeatureCount(ftype) => vec![Value::Int(count_feature(rec, ftype))],
            Kind::Append(v) => vec![match v {
                Value::Int(i) => Value::Int(*i),
                Value::Float(f) => Value::Float(*f),
                Value::Str(s) => Value::Str(s.clone()),
                Value::Null => Value::Null,
            }],
            Kind::MotifCount(pattern) => {
                let seq = String::from_utf8_lossy(&rec.seq);
                vec![Value::Int(protein_analysis::motif_count(&seq, pattern))]
            }
            Kind::NetCharge(ph) => match single_protein_sequence(rec) {
                Some(s) if !s.is_empty() => match protein_analysis::charge_at_ph(&s, *ph) {
                    Some(c) => vec![Value::Float(c)],
                    None => vec![Value::Null],
                },
                _ => vec![Value::Null],
            },
            Kind::RepeatCount(min_len) => vec![Value::Int(repeat_count(&rec.seq, *min_len))],
            Kind::NamedDomainCount(domain) => {
                let n = rec
                    .features
                    .iter()
                    .filter(|f| {
                        f.kind == DOMAIN_FEATURE_NAME && f.get_first("name") == Some(domain)
                    })
                    .count() as i64;
                vec![Value::Int(n)]
            }
            Kind::IsoelectricPoint => match single_protein_sequence(rec) {
                Some(s) if !s.is_empty() => match protein_analysis::isoelectric_point(&s) {
                    Some(p) => vec![Value::Float(p)],
                    None => vec![Value::Null],
                },
                _ => vec![Value::Null],
            },
            Kind::StartsWith => {
                vec![Value::Str(
                    rec.seq
                        .first()
                        .map(|b| (*b as char).to_string())
                        .unwrap_or_default(),
                )]
            }
            Kind::MolecularWeight => {
                if rec.seq.is_empty() {
                    return vec![Value::Null];
                }
                let mol = rec.molecule_type.as_deref().unwrap_or("DNA").to_lowercase();
                let (seq_type, seq): (SeqType, Vec<u8>) = if mol == "protein" {
                    let mut s = rec.seq.clone();
                    while s.last() == Some(&b'*') {
                        s.pop();
                    }
                    (SeqType::Protein, s)
                } else if mol.contains("rna") {
                    (SeqType::Rna, rec.seq.clone())
                } else {
                    (SeqType::Dna, rec.seq.clone())
                };
                match protein_analysis::molecular_weight(&seq, seq_type) {
                    Some(w) => vec![Value::Float(w)],
                    None => vec![Value::Null],
                }
            }
        }
    }
}

fn lineage_str(tax: Option<&TaxonomyData>, f: impl Fn(&TaxonomyData) -> Vec<String>) -> String {
    match tax {
        Some(t) => f(t).join("; "),
        None => String::new(),
    }
}

fn repeat_count(seq: &[u8], min_len: i64) -> i64 {
    if seq.is_empty() {
        return 0;
    }
    let up: Vec<u8> = seq.iter().map(|b| b.to_ascii_uppercase()).collect();
    let mut count = 0i64;
    let mut i = 0usize;
    while i < up.len() {
        let c = up[i];
        let mut run = 1usize;
        while i + run < up.len() && up[i + run] == c {
            run += 1;
        }
        if run as i64 >= min_len {
            count += 1;
        }
        i += run;
    }
    count
}

fn domain_search(rec: &SeqRecord) -> Vec<Value> {
    for f in &rec.features {
        if f.kind == DOMAIN_SEARCH_BEST_HIT_NAME {
            let g = |k: &str| f.get_first(k).unwrap_or("").to_string();
            let int_q = |k: &str| f.get_first(k).and_then(|v| v.parse::<i64>().ok());
            return vec![
                Value::Str(g("name")),
                Value::Str(g("description")),
                int_q("rstart").map(Value::Int).unwrap_or(Value::Int(0)),
                int_q("rend").map(Value::Int).unwrap_or(Value::Int(0)),
                int_q("rlen").map(Value::Int).unwrap_or(Value::Int(0)),
                Value::Int(stranded_start_hr(&f.location)),
                Value::Int(stranded_end_hr(&f.location)),
                Value::Str(get_strand(&f.location).to_string()),
                Value::Float(feat_f64(f, "score", 0.0)),
                Value::Float(feat_f64(f, "identity", 0.0)),
            ];
        }
    }
    vec![
        Value::Str(String::new()),
        Value::Str(String::new()),
        Value::Int(0),
        Value::Int(0),
        Value::Int(0),
        Value::Int(0),
        Value::Int(0),
        Value::Str(String::new()),
        Value::Float(0.0),
        Value::Float(0.0),
    ]
}

// --- record processing -----------------------------------------------------

/// One output row: the leading id columns + analysis values.
fn process_record(
    rec: &SeqRecord,
    by: By,
    analyses: &[Analysis],
    ncbi_taxonomy: Option<&NCBITaxonomy>,
    filename: &str,
    rows: &mut Vec<Vec<Value>>,
) {
    let contig_name = rec.id.clone();
    let mut sub_recs: Vec<SeqRecord> = Vec::new();
    let mut locs: Vec<Location> = Vec::new();
    let mut cds_names: Vec<String> = Vec::new();
    let mut domain_names: Vec<String> = Vec::new();

    if by == By::Contig {
        sub_recs.push(rec.clone());
        locs.push(Location::range(0, rec.seq.len() as i64, 1));
    } else {
        let sources: Vec<SeqFeature> = get_sources(rec).into_iter().cloned().collect();
        if !rec.is_protein() {
            let mut cdss = DomainatorCds::list_from_contig(
                rec,
                f64::INFINITY,
                f64::NEG_INFINITY,
                false,
                None,
                true,
            );
            cdss.sort_by_key(|c| stranded_start_loc(&c.feature.location, rec.seq.len() as i64));
            for cds in &cdss {
                if by == By::Cds {
                    if cds.is_nucleic_acid {
                        continue;
                    }
                    let mut keep = sources.clone();
                    keep.push(cds.feature.clone());
                    keep.extend(cds.domain_features.iter().cloned());
                    let slice = slice_loc_from(&cds.feature.location);
                    sub_recs.push(slice_record(rec, &slice, &keep));
                    locs.push(cds.feature.location.clone());
                    cds_names.push(cds.name.clone());
                } else {
                    for domain in &cds.domain_features {
                        let mut keep = sources.clone();
                        keep.push(domain.clone());
                        let slice = slice_loc_from(&domain.location);
                        sub_recs.push(slice_record(rec, &slice, &keep));
                        locs.push(domain.location.clone());
                        cds_names.push(cds.name.clone());
                        domain_names.push(domain.get_first("name").unwrap_or("").to_string());
                    }
                }
            }
        } else {
            // protein
            if by == By::Cds {
                sub_recs.push(rec.clone());
                cds_names.push(" ".to_string());
                locs.push(Location::range(0, rec.seq.len() as i64, 1));
            } else {
                for f in &rec.features {
                    if f.kind == DOMAIN_FEATURE_NAME {
                        let mut keep = sources.clone();
                        keep.push(f.clone());
                        let slice = slice_loc_from(&f.location);
                        sub_recs.push(slice_record(rec, &slice, &keep));
                        cds_names.push(" ".to_string());
                        domain_names.push(f.get_first("name").unwrap_or("").to_string());
                        locs.push(f.location.clone());
                    }
                }
            }
        }
    }

    for (i, sub_rec) in sub_recs.iter().enumerate() {
        let mut row: Vec<Value> = vec![Value::Str(contig_name.clone())];
        if by == By::Cds || by == By::Domain {
            row.push(Value::Str(cds_names[i].clone()));
            if by == By::Domain {
                row.push(Value::Str(domain_names[i].clone()));
            }
        }
        let tax_data = ncbi_taxonomy.map(|t| TaxonomyData::new(t, record_taxid(sub_rec)));
        let ctx = Ctx {
            loc: locs[i].clone(),
            tax: tax_data.as_ref(),
            filename,
        };
        for analysis in analyses {
            row.extend(analysis.compute(sub_rec, &ctx));
        }
        rows.push(row);
    }
}

/// Format one value for TSV output (`EnumTSVWriter.write_row`).
fn format_value(v: &Value, ctype: ColType) -> String {
    match v {
        Value::Null => String::new(),
        Value::Str(s) => s.replace('\t', " "),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => {
            if ctype == ColType::Int {
                (*f as i64).to_string()
            } else {
                // Normalize -0.0 to 0.0 (Python's score/charge sums never yield a
                // signed zero, so "%0.1f" gives "0.0", not "-0.0").
                let f = if *f == 0.0 { 0.0 } else { *f };
                format!("{f:.1}")
            }
        }
    }
}

/// Output serialization for [`enum_report`].
#[derive(Clone, Copy, PartialEq)]
pub enum OutputFormat {
    /// Tab-separated values (the Python-parity format).
    Tsv,
    /// A JSON array of row objects (agent-friendly; native to this port).
    Json,
}

/// Convert a cell to a typed JSON value (numbers for int/float columns, `null`
/// for missing).
fn json_value(v: &Value, ctype: ColType) -> serde_json::Value {
    use serde_json::Value as J;
    match v {
        Value::Null => J::Null,
        Value::Str(s) => J::String(s.clone()),
        Value::Int(i) => J::Number((*i).into()),
        Value::Float(f) => {
            if ctype == ColType::Int {
                J::Number((*f as i64).into())
            } else {
                let f = if *f == 0.0 { 0.0 } else { *f };
                serde_json::Number::from_f64(f)
                    .map(J::Number)
                    .unwrap_or(J::Null)
            }
        }
    }
}

/// Run the report, writing to `out` in `format`.
// Mirrors the Python entry point's parameter list one-for-one; bundling these into
// an options struct would put the port a rename away from its parity reference.
#[allow(clippy::too_many_arguments)]
pub fn enum_report<W: Write>(
    mut records: Vec<(SeqRecord, String)>,
    by: By,
    analyses: &[Analysis],
    column_names: Option<&[String]>,
    ncbi_taxonomy: Option<&NCBITaxonomy>,
    databases: Option<&HashSet<String>>,
    format: OutputFormat,
    out: &mut W,
) -> Result<(), String> {
    // Build headers + types.
    let mut headers: Vec<String> = vec!["contig".to_string()];
    let mut types: Vec<ColType> = vec![ColType::Str];
    if by == By::Cds || by == By::Domain {
        headers.push("cds".to_string());
        types.push(ColType::Str);
    }
    if by == By::Domain {
        headers.push("domain".to_string());
        types.push(ColType::Str);
    }
    for a in analyses {
        for (name, t) in &a.columns {
            headers.push(name.clone());
            types.push(*t);
        }
    }
    if let Some(cn) = column_names {
        if cn.len() != headers.len() {
            return Err(format!(
                "The number of override column names must be the same as the number of columns.\nColumns:\n{}\nOverride columns:\n{}",
                headers.join("; "),
                cn.join("; ")
            ));
        }
        headers = cn.to_vec();
    }

    let mut rows: Vec<Vec<Value>> = Vec::new();
    for (rec, filename) in &mut records {
        if let Some(dbs) = databases {
            filter_domains_databases(rec, dbs);
        }
        process_record(rec, by, analyses, ncbi_taxonomy, filename, &mut rows);
    }

    match format {
        OutputFormat::Tsv => {
            writeln!(out, "{}", headers.join("\t")).map_err(|e| e.to_string())?;
            for row in &rows {
                let cells: Vec<String> = row
                    .iter()
                    .enumerate()
                    .map(|(i, v)| format_value(v, types[i]))
                    .collect();
                writeln!(out, "{}", cells.join("\t")).map_err(|e| e.to_string())?;
            }
        }
        OutputFormat::Json => {
            // Streaming NDJSON: one compact object per record, keyed by column name
            // (column order preserved via serde_json's preserve_order feature). Not a
            // JSON array -- NDJSON keeps the writer streaming and is what upstream's
            // agent contract specifies. Missing values are null, so the key set is
            // stable across rows.
            for row in &rows {
                let mut obj = serde_json::Map::new();
                for (i, v) in row.iter().enumerate() {
                    obj.insert(headers[i].clone(), json_value(v, types[i]));
                }
                let line = serde_json::to_string(&serde_json::Value::Object(obj))
                    .map_err(|e| e.to_string())?;
                writeln!(out, "{line}").map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

/// Whether any requested analysis needs the NCBI taxonomy database.
///
/// `rank_lineage` is included although upstream's list leaves it out: without a
/// loaded taxonomy Python's lambda dereferences `None` and the run crashes, and
/// here it silently came out as an empty column, so no working invocation relies
/// on the omission.
pub fn needs_taxonomy(analyses: &[Analysis]) -> bool {
    analyses.iter().any(|a| {
        matches!(
            a.kind,
            Kind::TaxidLineage
                | Kind::TaxnameLineage
                | Kind::RankLineage
                | Kind::Taxid(_)
                | Kind::Taxname(_)
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seq::{Location, SeqRecord, Topology};

    fn dom(name: &str, desc: &str, start: i64) -> SeqFeature {
        let mut f = SeqFeature::new(DOMAIN_FEATURE_NAME, Location::range(start, start + 30, 1));
        f.set("name", name);
        f.set("description", desc);
        f.set("database", "DB");
        f.set("evalue", "1e-5");
        f.set("score", "5.0");
        f
    }

    fn rec(seq: &[u8], features: Vec<SeqFeature>) -> SeqRecord {
        SeqRecord {
            id: "c".into(),
            name: "c".into(),
            description: String::new(),
            seq: seq.to_vec(),
            molecule_type: Some("protein".into()),
            topology: Topology::Linear,
            annotations: Default::default(),
            features,
        }
    }

    #[test]
    fn domain_content_is_alphabetical_and_unique() {
        let r = rec(
            b"M",
            vec![
                dom("Zed", "z", 200),
                dom("Abe", "a", 50),
                dom("Abe", "a", 90),
            ],
        );
        let (names, descs) = get_domain_content(&r);
        assert_eq!(names, "Abe; Zed");
        assert_eq!(descs, "a; z");
    }

    #[test]
    fn architecture_is_in_start_order() {
        let r = rec(b"M", vec![dom("B", "", 200), dom("A", "", 50)]);
        assert_eq!(get_domain_architecture(&r), "A; B");
    }

    #[test]
    fn repeat_count_runs() {
        // AGGKKK -> runs A(1), GG(2), KKK(3).
        assert_eq!(repeat_count(b"AGGKKK", 1), 3);
        assert_eq!(repeat_count(b"AGGKKK", 2), 2);
        assert_eq!(repeat_count(b"AGGKKK", 3), 1);
        assert_eq!(repeat_count(b"", 1), 0);
    }

    #[test]
    fn safe_column_names() {
        assert_eq!(
            named_domain_count_analysis("foo-bar!".into()).columns[0].0,
            "named_domain_count_foo_bar"
        );
        assert_eq!(
            named_domain_count_analysis("***".into()).columns[0].0,
            "named_domain_count_domain"
        );
        assert_eq!(
            motif_count_analysis("[AG]-x-[DE]".into()).columns[0].0,
            "motif_count_AG_x_DE"
        );
    }

    #[test]
    fn append_typing() {
        assert!(matches!(
            append_analysis("n".into(), "int", "1").unwrap().columns[0].1,
            ColType::Int
        ));
        assert!(matches!(
            append_analysis("n".into(), "float", "2.0").unwrap().columns[0].1,
            ColType::Float
        ));
        assert!(append_analysis("n".into(), "bogus", "x").is_err());
    }
}
