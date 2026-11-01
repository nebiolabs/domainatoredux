//! NCBI taxonomy (`domainator.Taxonomy.NCBITaxonomy`): parse the NCBI taxdump
//! (`nodes.dmp` / `names.dmp` / `merged.dmp` / `delnodes.dmp`, read from a
//! directory of extracted files or from `taxdump.tar.gz`), and answer lineage /
//! rank / name queries plus compile include/exclude and boolean taxonomy
//! filters into O(1) allowed-taxid sets. Downloading/refresh is out of scope:
//! the caller points at a prepared directory.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;

use super::{SeqFeature, SeqRecord};
use crate::tools::select_common::BooleanEvaluator;

/// "unidentified sequences" — the normalized taxid for deleted/unknown taxa.
pub const UNIDENTIFIED_TAXID: i64 = 32644;

pub struct NCBITaxonomy {
    /// taxid -> (parent taxid, rank).
    nodes: HashMap<i64, (i64, String)>,
    /// taxid -> scientific name (or first name).
    names: HashMap<i64, String>,
    /// old taxid -> new taxid.
    merged: HashMap<i64, i64>,
    /// deleted taxids.
    deleted: HashSet<i64>,
    /// Unknown taxids already warned about, so each is reported once.
    warned_unknown: Mutex<HashSet<i64>>,
}

/// Split a taxdmp line into fields: strip the trailing `\n|\t` clutter, then
/// split on `\t|\t` (matching Python's `rstrip("\n|\t").split("\t|\t")`).
fn split_dmp(line: &str) -> Vec<&str> {
    line.trim_end_matches(['\n', '|', '\t'])
        .split("\t|\t")
        .collect()
}

impl NCBITaxonomy {
    /// Load from a directory containing either extracted `.dmp` files or
    /// `taxdump.tar.gz`, or from the path of a taxdump archive itself (how the GUI
    /// registers its managed NCBI Taxonomy download, as one file).
    pub fn load(dir: &Path) -> std::io::Result<NCBITaxonomy> {
        let nodes_p = dir.join("nodes.dmp");
        let names_p = dir.join("names.dmp");
        let merged_p = dir.join("merged.dmp");
        let delnodes_p = dir.join("delnodes.dmp");
        if nodes_p.exists() && names_p.exists() && merged_p.exists() && delnodes_p.exists() {
            let nodes = std::fs::read_to_string(&nodes_p)?;
            let names = std::fs::read_to_string(&names_p)?;
            let merged = std::fs::read_to_string(&merged_p)?;
            let delnodes = std::fs::read_to_string(&delnodes_p)?;
            return Ok(Self::from_texts(&nodes, &names, &merged, &delnodes));
        }
        let tar = if dir.is_file() {
            dir.to_path_buf()
        } else {
            dir.join("taxdump.tar.gz")
        };
        if tar.exists() {
            let members = read_targz_members(&tar)?;
            let get = |name: &str| -> std::io::Result<String> {
                members
                    .get(name)
                    .map(|b| String::from_utf8_lossy(b).into_owned())
                    .ok_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!("{} is missing {name}", tar.display()),
                        )
                    })
            };
            return Ok(Self::from_texts(
                &get("nodes.dmp")?,
                &get("names.dmp")?,
                &get("merged.dmp")?,
                &get("delnodes.dmp")?,
            ));
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "NCBI taxonomy database not found in {}: expected taxdump.tar.gz or extracted .dmp files",
                dir.display()
            ),
        ))
    }

    fn from_texts(nodes: &str, names: &str, merged: &str, delnodes: &str) -> NCBITaxonomy {
        let mut nodes_tab = HashMap::new();
        for line in nodes.lines() {
            let f = split_dmp(line);
            if f.len() >= 3 {
                if let (Ok(tid), Ok(parent)) = (f[0].parse::<i64>(), f[1].parse::<i64>()) {
                    nodes_tab.insert(tid, (parent, f[2].to_string()));
                }
            }
        }
        let mut name_tab: HashMap<i64, String> = HashMap::new();
        let mut first_name: HashMap<i64, String> = HashMap::new();
        for line in names.lines() {
            let f = split_dmp(line);
            if f.len() >= 4 {
                if let Ok(tid) = f[0].parse::<i64>() {
                    first_name.entry(tid).or_insert_with(|| f[1].to_string());
                    if f[3] == "scientific name" {
                        name_tab.insert(tid, f[1].to_string());
                    }
                }
            }
        }
        for (tid, n) in first_name {
            name_tab.entry(tid).or_insert(n);
        }
        let mut merged_tab = HashMap::new();
        for line in merged.lines() {
            let f = split_dmp(line);
            if f.len() >= 2 {
                if let (Ok(old), Ok(new)) = (f[0].parse::<i64>(), f[1].parse::<i64>()) {
                    merged_tab.insert(old, new);
                }
            }
        }
        let mut deleted = HashSet::new();
        for line in delnodes.lines() {
            let f = split_dmp(line);
            if let Some(Ok(tid)) = f.first().map(|s| s.parse::<i64>()) {
                deleted.insert(tid);
            }
        }
        NCBITaxonomy {
            nodes: nodes_tab,
            names: name_tab,
            merged: merged_tab,
            deleted,
            warned_unknown: Mutex::new(HashSet::new()),
        }
    }

    pub fn normalized_taxid(&self, tax_id: i64) -> i64 {
        if let Some(&new) = self.merged.get(&tax_id) {
            new
        } else if self.deleted.contains(&tax_id) {
            UNIDENTIFIED_TAXID
        } else {
            tax_id
        }
    }

    /// All ancestors of `tax_id` (self first, root `1` last). Empty if the taxid
    /// is deleted or unknown; an unknown one is warned about once (upstream
    /// `NCBITaxonomy._lineage`). NCBI deprecates taxids rather than removing them,
    /// so an unknown taxid is newer than the database.
    pub fn lineage(&self, tax_id: i64) -> Vec<i64> {
        let mut out = Vec::new();
        let mut tax_id = tax_id;
        if let Some(&new) = self.merged.get(&tax_id) {
            tax_id = new;
        }
        if self.deleted.contains(&tax_id) {
            return out;
        }
        if !self.nodes.contains_key(&tax_id) {
            let first = self.warned_unknown.lock().map(|mut w| w.insert(tax_id));
            if first.unwrap_or(false) {
                crate::log::warn(format_args!(
                    "taxid {tax_id} is not in the NCBI taxonomy database, which is probably older \
                     than the sequences; it is treated as unknown (no lineage). Update the \
                     database with --taxonomy_update."
                ));
            }
            return out;
        }
        out.push(tax_id);
        while self.nodes[&tax_id].0 != 1 {
            let mut parent = self.nodes[&tax_id].0;
            if let Some(&new) = self.merged.get(&parent) {
                parent = new;
            }
            out.push(parent);
            tax_id = parent;
            if !self.nodes.contains_key(&tax_id) {
                return out;
            }
        }
        out.push(1);
        out
    }

    pub fn name(&self, tax_id: i64) -> Option<&str> {
        self.names.get(&tax_id).map(String::as_str)
    }

    pub fn rank(&self, tax_id: i64) -> Option<&str> {
        self.nodes.get(&tax_id).map(|(_, r)| r.as_str())
    }

    fn normalized_parent(&self, tid: i64) -> i64 {
        let mut p = self.nodes[&tid].0;
        if let Some(&new) = self.merged.get(&p) {
            p = new;
        }
        p
    }

    fn finalize_allowed(
        &self,
        mut allowed: HashSet<i64>,
        empty_lineage_passes: bool,
    ) -> HashSet<i64> {
        let news: Vec<(i64, i64)> = self.merged.iter().map(|(&o, &n)| (o, n)).collect();
        for (old, new) in news {
            if allowed.contains(&new) {
                allowed.insert(old);
            }
        }
        if empty_lineage_passes {
            for &t in &self.deleted {
                allowed.insert(t);
            }
        }
        allowed
    }

    /// `compile_taxonomy_filter`: allowed-taxid set for an include/exclude
    /// filter, or `None` when neither is given.
    pub fn compile_taxonomy_filter(
        &self,
        include: Option<&HashSet<i64>>,
        exclude: Option<&HashSet<i64>>,
    ) -> Option<HashSet<i64>> {
        let include = include.filter(|s| !s.is_empty());
        let exclude = exclude.filter(|s| !s.is_empty());
        if include.is_none() && exclude.is_none() {
            return None;
        }
        let mut inc_memo: HashMap<i64, bool> = HashMap::new();
        let mut exc_memo: HashMap<i64, bool> = HashMap::new();

        let resolve =
            |t0: i64, inc_memo: &mut HashMap<i64, bool>, exc_memo: &mut HashMap<i64, bool>| {
                let mut chain = Vec::new();
                let mut t = t0;
                while !inc_memo.contains_key(&t) {
                    let p = self.normalized_parent(t);
                    if p == t {
                        inc_memo.insert(t, include.map(|s| s.contains(&t)).unwrap_or(false));
                        exc_memo.insert(t, exclude.map(|s| s.contains(&t)).unwrap_or(false));
                        break;
                    }
                    chain.push(t);
                    t = p;
                }
                for &t in chain.iter().rev() {
                    let p = self.normalized_parent(t);
                    let inc = inc_memo[&p] || include.map(|s| s.contains(&t)).unwrap_or(false);
                    let exc = exc_memo[&p] || exclude.map(|s| s.contains(&t)).unwrap_or(false);
                    inc_memo.insert(t, inc);
                    exc_memo.insert(t, exc);
                }
            };

        let mut allowed = HashSet::new();
        for &tid in self.nodes.keys() {
            if !inc_memo.contains_key(&tid) {
                resolve(tid, &mut inc_memo, &mut exc_memo);
            }
            if (include.is_none() || inc_memo[&tid]) && (exclude.is_none() || !exc_memo[&tid]) {
                allowed.insert(tid);
            }
        }
        Some(self.finalize_allowed(allowed, include.is_none()))
    }

    /// `compile_taxonomy_expression`: allowed-taxid set for a boolean expression
    /// over taxids (a token is true for a node when it is in the node's
    /// lineage), or `None` for an empty expression. A malformed expression, or one
    /// naming something other than a taxid, is an error.
    pub fn compile_taxonomy_expression(
        &self,
        expression: &str,
    ) -> Result<Option<HashSet<i64>>, String> {
        if expression.trim().is_empty() {
            return Ok(None);
        }
        let evaluator = BooleanEvaluator::new(expression)?;
        let marker_taxids: HashSet<i64> = evaluator
            .names()
            .iter()
            .map(|t| {
                t.parse::<i64>()
                    .map_err(|_| format!("--taxonomy_expr names {t:?}, which is not a taxid"))
            })
            .collect::<Result<_, _>>()?;

        // present[t] = sorted set of marker taxids in t's lineage, memoized.
        let mut present: HashMap<i64, Vec<i64>> = HashMap::new();
        let resolve = |t0: i64, present: &mut HashMap<i64, Vec<i64>>| {
            let mut chain = Vec::new();
            let mut t = t0;
            while !present.contains_key(&t) {
                let p = self.normalized_parent(t);
                if p == t {
                    let mut v = Vec::new();
                    if marker_taxids.contains(&t) {
                        v.push(t);
                    }
                    present.insert(t, v);
                    break;
                }
                chain.push(t);
                t = p;
            }
            for &t in chain.iter().rev() {
                let p = self.normalized_parent(t);
                let mut v = present[&p].clone();
                if marker_taxids.contains(&t) {
                    v.push(t);
                    v.sort_unstable();
                }
                present.insert(t, v);
            }
        };

        let verdict = |present_set: &[i64]| -> bool {
            let names: HashSet<String> = present_set.iter().map(|t| t.to_string()).collect();
            evaluator.check_expression(&names)
        };

        let mut allowed = HashSet::new();
        for &tid in self.nodes.keys() {
            if !present.contains_key(&tid) {
                resolve(tid, &mut present);
            }
            if verdict(&present[&tid]) {
                allowed.insert(tid);
            }
        }
        let empty_passes = verdict(&[]);
        Ok(Some(self.finalize_allowed(allowed, empty_passes)))
    }

    /// A criterion taxid, followed through `merged.dmp`; an error if it is not a
    /// taxid of the database (upstream `compile_taxonomy_filter`'s `normalize`).
    fn normalize_criterion(&self, taxid: i64) -> Result<i64, String> {
        let mut taxid = taxid;
        let mut seen = HashSet::new();
        while let Some(&new) = self.merged.get(&taxid) {
            if !seen.insert(taxid) {
                return Err(format!("Cyclic merged taxonomy criterion: {taxid}."));
            }
            taxid = new;
        }
        if !self.nodes.contains_key(&taxid) {
            return Err(format!(
                "Taxonomy criterion {taxid} is deleted or absent from the database; check \
                 the taxid or update the taxonomy database."
            ));
        }
        Ok(taxid)
    }

    /// Upstream `taxonomy.records.compile_taxonomy_filter`: compile
    /// `--include_taxids`/`--exclude_taxids` or `--taxonomy_expr` into a
    /// [`TaxonomyFilter`] that applies `unknown` to every candidate whose taxid the
    /// database cannot resolve. `None` when no criterion is given. A criterion that is
    /// not a taxid of the database is an error, as is giving an expression together
    /// with include/exclude taxids.
    pub fn compile_filter(
        &self,
        include: &[i64],
        exclude: &[i64],
        expression: Option<&str>,
        unknown: TaxonomyUnknown,
    ) -> Result<Option<TaxonomyFilter>, String> {
        let expression = expression.map(str::trim).filter(|e| !e.is_empty());
        if expression.is_some() && (!include.is_empty() || !exclude.is_empty()) {
            return Err(
                "taxonomy_expr is mutually exclusive with include_taxids/exclude_taxids.".into(),
            );
        }
        if !taxonomy_active(include, exclude, expression) {
            return Ok(None);
        }
        let (allowed, empty_matches) = if let Some(expression) = expression {
            // Each name becomes its normalized taxid, so a merged taxid selects
            // what its replacement does.
            let evaluator = BooleanEvaluator::new(expression)?;
            let mut replacements = HashMap::new();
            for name in evaluator.names() {
                let taxid = name.parse::<i64>().map_err(|_| {
                    format!("Invalid taxonomy criterion: {name:?}. Expected an NCBI taxid.")
                })?;
                replacements.insert(name.clone(), self.normalize_criterion(taxid)?.to_string());
            }
            let normalized = replace_names(expression, &replacements);
            let empty_matches =
                BooleanEvaluator::new(&normalized)?.check_expression(&HashSet::new());
            let allowed = self
                .compile_taxonomy_expression(&normalized)?
                .unwrap_or_default();
            (allowed, empty_matches)
        } else {
            let normalize = |taxids: &[i64]| {
                taxids
                    .iter()
                    .map(|&t| self.normalize_criterion(t))
                    .collect::<Result<HashSet<i64>, String>>()
            };
            let (include, exclude) = (normalize(include)?, normalize(exclude)?);
            let allowed = self
                .compile_taxonomy_filter(
                    Some(&include).filter(|s| !s.is_empty()),
                    Some(&exclude).filter(|s| !s.is_empty()),
                )
                .unwrap_or_default();
            (allowed, include.is_empty())
        };
        // Every node but "unidentified" is resolved; a merged taxid is resolved when
        // its replacement is, and takes its verdict.
        let mut verdicts: HashMap<i64, bool> = self
            .nodes
            .keys()
            .filter(|&&t| t != UNIDENTIFIED_TAXID)
            .map(|&t| (t, allowed.contains(&t)))
            .collect();
        for &old in self.merged.keys() {
            let mut replacement = old;
            let mut seen = HashSet::new();
            while let Some(&new) = self.merged.get(&replacement) {
                if !seen.insert(replacement) {
                    break;
                }
                replacement = new;
            }
            if let Some(&verdict) = verdicts.get(&replacement) {
                verdicts.insert(old, verdict || allowed.contains(&old));
            }
        }
        Ok(Some(TaxonomyFilter {
            verdicts,
            unknown,
            empty_matches,
        }))
    }
}

/// Upstream `taxonomy_active`: whether any taxonomy criterion was given (a blank
/// expression is none).
pub fn taxonomy_active(include: &[i64], exclude: &[i64], expression: Option<&str>) -> bool {
    !include.is_empty() || !exclude.is_empty() || expression.is_some_and(|e| !e.trim().is_empty())
}

/// `expression` with each name (a run of characters other than operators,
/// parentheses and whitespace) replaced by its entry in `replacements`.
fn replace_names(expression: &str, replacements: &HashMap<String, String>) -> String {
    let is_name = |c: char| !"&|~()".contains(c) && !c.is_whitespace();
    let mut out = String::with_capacity(expression.len());
    let mut name = String::new();
    for c in expression.chars().chain(std::iter::once(' ')) {
        if is_name(c) {
            name.push(c);
            continue;
        }
        if !name.is_empty() {
            out.push_str(replacements.get(&name).unwrap_or(&name));
            name.clear();
        }
        out.push(c);
    }
    out.pop();
    out
}

/// `--taxonomy_unknown`: what a taxonomy filter does with a candidate whose taxid the
/// database cannot resolve — missing, "unidentified" (32644), deleted, or absent.
/// Upstream `TAXONOMY_UNKNOWN_CHOICES`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TaxonomyUnknown {
    /// Reject it (the default).
    #[default]
    Exclude,
    /// Accept it.
    Include,
    /// Judge it as if its lineage were empty: `~2` accepts it, `2` does not.
    Evaluate,
    /// Fail the run.
    Error,
}

impl std::str::FromStr for TaxonomyUnknown {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "exclude" => Ok(Self::Exclude),
            "include" => Ok(Self::Include),
            "evaluate" => Ok(Self::Evaluate),
            "error" => Ok(Self::Error),
            _ => Err(format!(
                "Invalid taxonomy_unknown {s:?}; choose from exclude, include, evaluate, error."
            )),
        }
    }
}

/// A compiled taxonomy gate, upstream `taxonomy.records.TaxonomyFilter`: a verdict for
/// every taxid the database resolves, and the `--taxonomy_unknown` policy for the
/// rest. Built by [`NCBITaxonomy::compile_filter`]; needs no database afterwards.
#[derive(Clone, Debug, PartialEq)]
pub struct TaxonomyFilter {
    verdicts: HashMap<i64, bool>,
    unknown: TaxonomyUnknown,
    /// Whether the criteria accept an empty lineage (`--taxonomy_unknown evaluate`).
    empty_matches: bool,
}

impl TaxonomyFilter {
    /// The unresolved-taxonomy policy.
    pub fn unknown(&self) -> TaxonomyUnknown {
        self.unknown
    }

    /// Whether a candidate with `taxid` (`None`: none found) passes. `context` names
    /// the candidate for the error `--taxonomy_unknown error` raises.
    pub fn allows(
        &self,
        taxid: Option<i64>,
        context: impl FnOnce() -> String,
    ) -> Result<bool, String> {
        if let Some(&verdict) = taxid.and_then(|t| self.verdicts.get(&t)) {
            return Ok(verdict);
        }
        match self.unknown {
            TaxonomyUnknown::Error => Err(format!(
                "Unresolved taxonomy for {}: taxid {} (missing, unidentified, deleted, or \
                 absent from the taxonomy database).",
                context(),
                taxid.map_or_else(|| "None".to_string(), |t| t.to_string())
            )),
            TaxonomyUnknown::Include => Ok(true),
            TaxonomyUnknown::Evaluate => Ok(self.empty_matches),
            TaxonomyUnknown::Exclude => Ok(false),
        }
    }

    /// Upstream `TaxonomyFilter.matches`: whether `record` passes, or with `feature`,
    /// whether that feature of it does. A protein record, and a whole record, take
    /// the record's taxid; a feature of a nucleotide record takes the taxid of the
    /// longest `source` covering it. A malformed taxid is an error, never unknown.
    pub fn matches(
        &self,
        record: &SeqRecord,
        feature: Option<&SeqFeature>,
    ) -> Result<bool, String> {
        use super::taxid::location_taxid;
        let mut context = format!("record {:?}", record.id);
        if let Some(f) = feature {
            context.push_str(&format!(", {} {}", f.kind, location_str(&f.location)));
        }
        let invalid = |e: String| format!("Invalid taxonomy for {context}: {e}");
        let sources: Vec<&SeqFeature> = record
            .features
            .iter()
            .filter(|f| f.kind == "source")
            .collect();
        let (taxid, mut candidates) = match feature {
            Some(f) if !record.is_protein() => {
                let region = f.location.intervals();
                let covering: Vec<&SeqFeature> = sources
                    .iter()
                    .copied()
                    .filter(|s| covers(&s.location.intervals(), &region))
                    .collect();
                (location_taxid(&record.features, &f.location), covering)
            }
            _ => {
                // `get_taxid`: a UniProt/UniRef header wins over the sources.
                let mut header = None;
                for marker in [" OX=", " TaxID="] {
                    if let Some(rest) = record
                        .description
                        .rsplit(marker)
                        .next()
                        .filter(|_| record.description.contains(marker))
                    {
                        let token = rest.split(' ').next().unwrap_or("");
                        header = Some(token.parse::<i64>().map_err(|_| {
                            invalid(format!("invalid literal for int() with base 10: {token:?}"))
                        })?);
                        break;
                    }
                }
                match header {
                    Some(t) => (Some(t), Vec::new()),
                    None => (Some(super::taxid::record_taxid(record)), sources),
                }
            }
        };
        // The source the taxid came from must carry a well-formed taxon.
        candidates.sort_by_key(|s| {
            std::cmp::Reverse(
                s.location
                    .intervals()
                    .iter()
                    .map(|(a, b)| b - a)
                    .sum::<i64>(),
            )
        });
        if let Some(source) = candidates.first() {
            let xrefs = source.qualifiers.iter().filter(|(k, _)| k == "db_xref");
            for xref in xrefs.flat_map(|(_, values)| values) {
                if let Some(rest) = xref.strip_prefix("taxon:") {
                    let token = rest.rsplit(':').next().unwrap_or(rest);
                    token.parse::<i64>().map_err(|_| {
                        invalid(format!("invalid literal for int() with base 10: {token:?}"))
                    })?;
                }
            }
        }
        self.allows(taxid, || context.clone())
    }
}

/// Whether merged `outer` intervals cover every one of `inner` (`location_covers`).
fn covers(outer: &[(i64, i64)], inner: &[(i64, i64)]) -> bool {
    let mut merged: Vec<(i64, i64)> = outer.to_vec();
    merged.sort_unstable();
    let mut out: Vec<(i64, i64)> = Vec::new();
    for (s, e) in merged {
        match out.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => out.push((s, e)),
        }
    }
    inner
        .iter()
        .all(|&(s, e)| out.iter().any(|&(os, oe)| os <= s && e <= oe))
}

/// A location as Biopython prints one, `[0:60](+)` or `join{[0:3](+), [9:12](+)}`,
/// for naming a candidate in an error.
fn location_str(location: &super::Location) -> String {
    let parts: Vec<String> = location
        .parts
        .iter()
        .map(|&(s, e, strand, ..)| {
            let strand = match strand {
                1 => "(+)",
                -1 => "(-)",
                _ => "",
            };
            format!("[{}:{}]{strand}", s.min(e), s.max(e))
        })
        .collect();
    match parts.as_slice() {
        [one] => one.clone(),
        _ => format!("join{{{}}}", parts.join(", ")),
    }
}

/// Per-record taxonomy view (`utils.TaxonomyData`): normalized taxid, lineage
/// (root-first, self-last), ranks (lowercased), names, and rank lookups.
pub struct TaxonomyData {
    pub taxid: i64,
    /// Root-first, self-last (Python reverses `lineage`).
    pub lineage: Vec<i64>,
    pub ranks: Vec<String>,
    pub names: Vec<String>,
    rank_to_name: HashMap<String, String>,
    rank_to_taxid: HashMap<String, i64>,
}

impl TaxonomyData {
    pub fn new(tax: &NCBITaxonomy, taxid: i64) -> TaxonomyData {
        let taxid = tax.normalized_taxid(taxid);
        let mut lineage = tax.lineage(taxid);
        lineage.reverse();
        let ranks: Vec<String> = lineage
            .iter()
            .map(|&t| tax.rank(t).unwrap_or("").to_lowercase())
            .collect();
        let names: Vec<String> = lineage
            .iter()
            .map(|&t| tax.name(t).unwrap_or("").to_string())
            .collect();
        let mut rank_to_name = HashMap::new();
        let mut rank_to_taxid = HashMap::new();
        for (i, r) in ranks.iter().enumerate() {
            rank_to_name.insert(r.clone(), names[i].clone());
            rank_to_taxid.insert(r.clone(), lineage[i]);
        }
        TaxonomyData {
            taxid,
            lineage,
            ranks,
            names,
            rank_to_name,
            rank_to_taxid,
        }
    }

    /// taxid at `rank` ("self" -> lowest/self), or `None` if absent.
    pub fn taxid_at(&self, rank: &str) -> Option<i64> {
        let r = rank.to_lowercase();
        if r == "self" {
            self.lineage.last().copied()
        } else {
            self.rank_to_taxid.get(&r).copied()
        }
    }

    /// name at `rank` ("self" -> lowest/self), or `None` if absent.
    pub fn name_at(&self, rank: &str) -> Option<&str> {
        let r = rank.to_lowercase();
        if r == "self" {
            self.names.last().map(String::as_str)
        } else {
            self.rank_to_name.get(&r).map(String::as_str)
        }
    }
}

/// Default location for the cached NCBI taxonomy database.
///
/// Namespaced by user so separate users on a shared machine do not collide on the same
/// directory and lockfile — one user writing a lockfile owned by another fails with a
/// permission error. Concurrent runs by the *same* user share it safely.
///
/// Mirrors `domainator.Taxonomy.default_ncbi_taxonomy_path`, whose name comes from
/// `getpass.getuser()`: the environment first, then the password database. We read the
/// same environment variables and fall back to a fixed name rather than pulling in a
/// libc dependency for the last resort — the directory only has to be stable and
/// per-user, not match Python's byte for byte.
pub fn default_ncbi_taxonomy_path() -> String {
    let user = ["LOGNAME", "USER", "LNAME", "USERNAME"]
        .iter()
        .find_map(|key| std::env::var(key).ok().filter(|v| !v.is_empty()))
        .unwrap_or_else(|| "default".to_string());
    format!("/tmp/ncbi_taxonomy_{user}")
}

/// Decompress a gzip member fully via libdeflater (ISIZE from the trailer).
fn gunzip(bytes: &[u8]) -> std::io::Result<Vec<u8>> {
    if bytes.len() < 18 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "truncated gzip",
        ));
    }
    let isize = u32::from_le_bytes(bytes[bytes.len() - 4..].try_into().unwrap()) as usize;
    let mut dec = libdeflater::Decompressor::new();
    // ISIZE is mod 2^32; for the small taxdump this is exact. Grow if needed.
    let mut cap = isize.max(1 << 20);
    loop {
        let mut out = vec![0u8; cap];
        match dec.gzip_decompress(bytes, &mut out) {
            Ok(n) => {
                out.truncate(n);
                return Ok(out);
            }
            Err(libdeflater::DecompressionError::InsufficientSpace) => {
                cap *= 2;
                if cap > (1 << 30) {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "taxdump.tar.gz too large to decompress",
                    ));
                }
            }
            Err(e) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("gzip decompress failed: {e:?}"),
                ))
            }
        }
    }
}

/// Read the members of a gzipped tar into `{basename: bytes}` (ustar format,
/// enough for the flat taxdump archive).
fn read_targz_members(path: &Path) -> std::io::Result<HashMap<String, Vec<u8>>> {
    let raw = std::fs::read(path)?;
    let data = gunzip(&raw)?;
    let mut out = HashMap::new();
    let mut pos = 0usize;
    while pos + 512 <= data.len() {
        let header = &data[pos..pos + 512];
        // Two consecutive zero blocks mark the end of the archive.
        if header.iter().all(|&b| b == 0) {
            break;
        }
        let name_end = header[..100].iter().position(|&b| b == 0).unwrap_or(100);
        let name = String::from_utf8_lossy(&header[..name_end]).into_owned();
        let size_str = String::from_utf8_lossy(&header[124..136]);
        let size = usize::from_str_radix(size_str.trim_matches(['\0', ' ']), 8).unwrap_or(0);
        pos += 512;
        if pos + size > data.len() {
            break;
        }
        let basename = name.rsplit('/').next().unwrap_or(&name).to_string();
        if !basename.is_empty() {
            out.insert(basename, data[pos..pos + size].to_vec());
        }
        // Advance past the file data, rounded up to the next 512-byte boundary.
        pos += size.div_ceil(512) * 512;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A tiny tree: cellular organisms(131567) -> Bacteria(2) -> Proteobacteria
    // (1224) -> {class 1236, genus Escherichia(561) -> species E. coli(562)}.
    // 999000 is merged into 562; 888 is deleted.
    fn tax() -> NCBITaxonomy {
        let [nodes, names, merged, delnodes] = dmp_texts();
        NCBITaxonomy::from_texts(nodes, names, merged, delnodes)
    }

    /// `nodes.dmp`, `names.dmp`, `merged.dmp`, `delnodes.dmp` for [`tax`].
    fn dmp_texts() -> [&'static str; 4] {
        let nodes = "\
1\t|\t1\t|\tno rank\t|\t
131567\t|\t1\t|\tcellular root\t|\t
2\t|\t131567\t|\tdomain\t|\t
1224\t|\t2\t|\tphylum\t|\t
1236\t|\t1224\t|\tclass\t|\t
561\t|\t1224\t|\tgenus\t|\t
562\t|\t561\t|\tspecies\t|\t";
        let names = "\
1\t|\troot\t|\t\t|\tscientific name\t|\t
131567\t|\tcellular organisms\t|\t\t|\tscientific name\t|\t
2\t|\tBacteria\t|\t\t|\tscientific name\t|\t
1224\t|\tProteobacteria\t|\t\t|\tscientific name\t|\t
561\t|\tEscherichia\t|\t\t|\tscientific name\t|\t
562\t|\tEscherichia coli\t|\t\t|\tscientific name\t|\t";
        let merged = "999000\t|\t562\t|\t";
        let delnodes = "888\t|\t";
        [nodes, names, merged, delnodes]
    }

    /// A gzipped ustar archive of `files`, laid out as NCBI's taxdump is.
    fn targz(files: &[(&str, &str)]) -> Vec<u8> {
        let mut tar = Vec::new();
        for (name, body) in files {
            let mut h = [0u8; 512];
            h[..name.len()].copy_from_slice(name.as_bytes());
            h[100..107].copy_from_slice(b"0000644");
            h[124..135].copy_from_slice(format!("{:011o}", body.len()).as_bytes());
            h[156] = b'0';
            h[257..262].copy_from_slice(b"ustar");
            // The checksum is summed with its own field read as spaces.
            h[148..156].copy_from_slice(b"        ");
            let sum: u32 = h.iter().map(|&b| b as u32).sum();
            h[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
            tar.extend_from_slice(&h);
            tar.extend_from_slice(body.as_bytes());
            tar.resize(tar.len().div_ceil(512) * 512, 0);
        }
        tar.resize(tar.len() + 1024, 0);
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut gz, &tar).unwrap();
        gz.finish().unwrap()
    }

    /// The GUI registers its managed download as the archive's own path, while the
    /// CLI (and Python) pass the directory holding it; both must load.
    #[test]
    fn loads_a_taxdump_archive_by_its_directory_or_its_own_path() {
        let dir = std::env::temp_dir().join(format!("domainator-taxdump-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let [nodes, names, merged, delnodes] = dmp_texts();
        let archive = dir.join("taxdump.tar.gz");
        std::fs::write(
            &archive,
            targz(&[
                ("nodes.dmp", nodes),
                ("names.dmp", names),
                ("merged.dmp", merged),
                ("delnodes.dmp", delnodes),
            ]),
        )
        .unwrap();
        for path in [dir.as_path(), archive.as_path()] {
            let t = NCBITaxonomy::load(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            assert_eq!(t.name(562), Some("Escherichia coli"), "{}", path.display());
            assert_eq!(t.normalized_taxid(999000), 562);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lineage_name_rank() {
        let t = tax();
        assert_eq!(t.lineage(562), vec![562, 561, 1224, 2, 131567, 1]);
        assert_eq!(t.name(2), Some("Bacteria"));
        assert_eq!(t.name(562), Some("Escherichia coli"));
        assert_eq!(t.rank(2), Some("domain"));
        assert_eq!(t.rank(561), Some("genus"));
    }

    #[test]
    fn unknown_taxid_has_no_lineage_and_is_warned_about_once() {
        let t = tax();
        assert!(t.lineage(59919).is_empty());
        assert!(t.lineage(59919).is_empty());
        assert_eq!(*t.warned_unknown.lock().unwrap(), HashSet::from([59919]));
    }

    #[test]
    fn normalized_taxid_merged_and_deleted() {
        let t = tax();
        assert_eq!(t.normalized_taxid(999000), 562);
        assert_eq!(t.normalized_taxid(888), UNIDENTIFIED_TAXID);
        assert_eq!(t.normalized_taxid(562), 562);
    }

    #[test]
    fn include_exclude_filters() {
        let t = tax();
        let inc = t
            .compile_taxonomy_filter(Some(&[1224].into_iter().collect()), None)
            .unwrap();
        assert!(
            inc.contains(&562) && inc.contains(&561) && inc.contains(&1236) && inc.contains(&1224)
        );
        assert!(!inc.contains(&2) && !inc.contains(&131567));
        // Merged old-taxid inherits its replacement's verdict.
        assert!(inc.contains(&999000));

        let exc = t
            .compile_taxonomy_filter(None, Some(&[1224].into_iter().collect()))
            .unwrap();
        assert!(exc.contains(&2) && exc.contains(&131567));
        assert!(!exc.contains(&562) && !exc.contains(&1224));
    }

    #[test]
    fn taxonomy_expression() {
        let t = tax();
        // Within Bacteria(2) but not Proteobacteria(1224).
        let a = t.compile_taxonomy_expression("2 & ~1224").unwrap().unwrap();
        assert!(a.contains(&2));
        assert!(!a.contains(&562) && !a.contains(&1224) && !a.contains(&131567));
        // Anything under Proteobacteria.
        let b = t.compile_taxonomy_expression("1224").unwrap().unwrap();
        assert!(t.compile_taxonomy_expression("2 &").is_err());
        assert!(t.compile_taxonomy_expression("Bacteria").is_err());
        assert!(b.contains(&1224) && b.contains(&562) && !b.contains(&2));
    }

    /// Upstream `test_unknown_policies`: a missing, unidentified or unknown taxid
    /// gets the `--taxonomy_unknown` verdict; `evaluate` judges an empty lineage.
    #[test]
    fn unresolved_taxids_follow_the_unknown_policy() {
        let t = tax();
        // (include, exclude, expression, whether an empty lineage passes)
        type Criteria<'a> = (&'a [i64], &'a [i64], Option<&'a str>, bool);
        let criteria: [Criteria<'_>; 4] = [
            (&[], &[], Some("~2"), true),
            (&[], &[2], None, true),
            (&[2], &[], None, false),
            (&[], &[], Some("2 & ~1224"), false),
        ];
        for (include, exclude, expr, empty) in criteria {
            for unknown in [
                TaxonomyUnknown::Exclude,
                TaxonomyUnknown::Include,
                TaxonomyUnknown::Evaluate,
                TaxonomyUnknown::Error,
            ] {
                let gate = t
                    .compile_filter(include, exclude, expr, unknown)
                    .unwrap()
                    .unwrap();
                for taxid in [None, Some(UNIDENTIFIED_TAXID), Some(999_999_999), Some(888)] {
                    let verdict = gate.allows(taxid, || "record 'unknown'".into());
                    match unknown {
                        TaxonomyUnknown::Error => {
                            assert!(verdict.unwrap_err().contains("record 'unknown'"))
                        }
                        _ => assert_eq!(
                            verdict.unwrap(),
                            unknown == TaxonomyUnknown::Include
                                || (unknown == TaxonomyUnknown::Evaluate && empty),
                            "{expr:?} {include:?} {exclude:?} {unknown:?} {taxid:?}"
                        ),
                    }
                }
            }
        }
    }

    /// Upstream `test_known_merged_deleted_and_criteria`.
    #[test]
    fn known_merged_and_criterion_taxids() {
        let t = tax();
        let gate = t
            .compile_filter(&[2], &[1224], None, TaxonomyUnknown::Exclude)
            .unwrap()
            .unwrap();
        let allows = |taxid| gate.allows(Some(taxid), String::new).unwrap();
        assert!(allows(2) && !allows(562) && !allows(1224));
        // A merged taxid takes its replacement's verdict, as a criterion too.
        assert_eq!(allows(999000), allows(562));
        assert_eq!(
            t.compile_filter(&[], &[], Some("999000"), TaxonomyUnknown::Exclude),
            t.compile_filter(&[562], &[], None, TaxonomyUnknown::Exclude)
        );
        // Criteria must be taxids the database knows.
        for (include, expr) in [
            (&[59919_i64][..], None),
            (&[][..], Some("Bacteria")),
            (&[888][..], None),
        ] {
            let err = t
                .compile_filter(include, &[], expr, TaxonomyUnknown::Exclude)
                .unwrap_err();
            assert!(err.contains("criterion"), "{err}");
        }
        assert!(t
            .compile_filter(&[2], &[], Some("~2"), TaxonomyUnknown::Exclude)
            .unwrap_err()
            .contains("mutually exclusive"));
        assert_eq!(
            t.compile_filter(&[], &[], Some("  "), TaxonomyUnknown::Exclude),
            Ok(None)
        );
    }

    /// Upstream `test_circular_anchor_source_precedence` and the malformed-taxid
    /// check: a feature takes the longest source covering it; a whole record, its
    /// longest source; a malformed taxon is an error, not "unknown".
    #[test]
    fn a_feature_takes_the_longest_source_covering_it() {
        use crate::seq::{Annotations, CompoundOp, Location, Topology};
        let source = |parts: Vec<(i64, i64)>, taxon: &str| {
            let mut location = Location::range(parts[0].0, parts[0].1, 1);
            if parts.len() > 1 {
                location.operator = Some(CompoundOp::Order);
                location.parts = parts
                    .iter()
                    .map(|&(s, e)| (s, e, 1, false, false))
                    .collect();
            }
            let mut f = SeqFeature::new("source", location);
            f.set("db_xref", format!("taxon:{taxon}"));
            f
        };
        let mut record = SeqRecord {
            id: "mixed".into(),
            name: "mixed".into(),
            description: "mixed".into(),
            seq: b"ATG".repeat(100),
            molecule_type: Some("DNA".into()),
            topology: Topology::Circular,
            annotations: Annotations::default(),
            features: vec![
                source(vec![(0, 90), (180, 300)], "562"),
                source(vec![(90, 180)], "2"),
            ],
        };
        let mut wrapped = SeqFeature::new("CDS", Location::range(270, 300, 1));
        wrapped.location.operator = Some(CompoundOp::Join);
        wrapped.location.parts.push((0, 30, 1, false, false));
        let coli = t_filter(&[562]);
        assert_eq!(coli.matches(&record, Some(&wrapped)), Ok(true));
        let inside = SeqFeature::new("CDS", Location::range(100, 150, 1));
        assert_eq!(coli.matches(&record, Some(&inside)), Ok(false));
        // An uncovered CDS has no taxid.
        let straddling = SeqFeature::new("CDS", Location::range(60, 120, 1));
        assert_eq!(coli.matches(&record, Some(&straddling)), Ok(false));
        // A source over the whole contig is longer, so it wins.
        record.features.push(source(vec![(0, 300)], "2"));
        assert_eq!(coli.matches(&record, Some(&wrapped)), Ok(false));
        assert_eq!(t_filter(&[2]).matches(&record, Some(&wrapped)), Ok(true));
        // The whole record takes its longest source.
        assert_eq!(t_filter(&[2]).matches(&record, None), Ok(true));
        record.features.push(source(vec![(0, 300)], "bad"));
        record.features.swap(0, 3);
        let err = t_filter(&[2]).matches(&record, None).unwrap_err();
        assert!(
            err.contains("Invalid taxonomy for record \"mixed\""),
            "{err}"
        );
    }

    fn t_filter(include: &[i64]) -> TaxonomyFilter {
        tax()
            .compile_filter(include, &[], None, TaxonomyUnknown::Exclude)
            .unwrap()
            .unwrap()
    }

    #[test]
    fn taxonomy_data_columns() {
        let t = tax();
        let td = TaxonomyData::new(&t, 562);
        assert_eq!(td.taxid_at("self"), Some(562));
        assert_eq!(td.taxid_at("domain"), Some(2));
        assert_eq!(td.taxid_at("genus"), Some(561));
        assert_eq!(td.name_at("self"), Some("Escherichia coli"));
        assert_eq!(td.name_at("domain"), Some("Bacteria"));
        // Root-first lineage; the lineage column drops the root.
        assert_eq!(td.lineage, vec![1, 131567, 2, 1224, 561, 562]);
        assert_eq!(td.taxid_at("order"), None);
    }
}
