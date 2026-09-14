//! NCBI taxonomy (`domainator.Taxonomy.NCBITaxonomy`): parse the NCBI taxdump
//! (`nodes.dmp` / `names.dmp` / `merged.dmp` / `delnodes.dmp`, read from a
//! directory of extracted files or from `taxdump.tar.gz`), and answer lineage /
//! rank / name queries plus compile include/exclude and boolean taxonomy
//! filters into O(1) allowed-taxid sets. Downloading/refresh is out of scope:
//! the caller points at a prepared directory.

use std::collections::{HashMap, HashSet};
use std::path::Path;

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
}

/// Split a taxdmp line into fields: strip the trailing `\n|\t` clutter, then
/// split on `\t|\t` (matching Python's `rstrip("\n|\t").split("\t|\t")`).
fn split_dmp(line: &str) -> Vec<&str> {
    line.trim_end_matches(['\n', '|', '\t']).split("\t|\t").collect()
}

impl NCBITaxonomy {
    /// Load from a directory containing either extracted `.dmp` files or
    /// `taxdump.tar.gz`.
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
        let tar = dir.join("taxdump.tar.gz");
        if tar.exists() {
            let members = read_targz_members(&tar)?;
            let get = |name: &str| -> std::io::Result<String> {
                members
                    .get(name)
                    .map(|b| String::from_utf8_lossy(b).into_owned())
                    .ok_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!("taxdump.tar.gz missing {name}"),
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
        NCBITaxonomy { nodes: nodes_tab, names: name_tab, merged: merged_tab, deleted }
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
    /// is deleted or unknown.
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

    fn finalize_allowed(&self, mut allowed: HashSet<i64>, empty_lineage_passes: bool) -> HashSet<i64> {
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

        let resolve = |t0: i64, inc_memo: &mut HashMap<i64, bool>, exc_memo: &mut HashMap<i64, bool>| {
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
    /// lineage), or `None` for an empty expression.
    pub fn compile_taxonomy_expression(&self, expression: &str) -> Option<HashSet<i64>> {
        if expression.trim().is_empty() {
            return None;
        }
        let evaluator = BooleanEvaluator::new(expression);
        let marker_taxids: HashSet<i64> =
            evaluator.token_values().filter_map(|t| t.parse::<i64>().ok()).collect();

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
        Some(self.finalize_allowed(allowed, empty_passes))
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
        let ranks: Vec<String> =
            lineage.iter().map(|&t| tax.rank(t).unwrap_or("").to_lowercase()).collect();
        let names: Vec<String> =
            lineage.iter().map(|&t| tax.name(t).unwrap_or("").to_string()).collect();
        let mut rank_to_name = HashMap::new();
        let mut rank_to_taxid = HashMap::new();
        for (i, r) in ranks.iter().enumerate() {
            rank_to_name.insert(r.clone(), names[i].clone());
            rank_to_taxid.insert(r.clone(), lineage[i]);
        }
        TaxonomyData { taxid, lineage, ranks, names, rank_to_name, rank_to_taxid }
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

#[cfg(test)]
mod tests {
    use super::*;

    // A tiny tree: cellular organisms(131567) -> Bacteria(2) -> Proteobacteria
    // (1224) -> {class 1236, genus Escherichia(561) -> species E. coli(562)}.
    // 999000 is merged into 562; 888 is deleted.
    fn tax() -> NCBITaxonomy {
        let nodes = "\
1\t|\t1\t|\tno rank\t|\t
131567\t|\t1\t|\tno rank\t|\t
2\t|\t131567\t|\tsuperkingdom\t|\t
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
        NCBITaxonomy::from_texts(nodes, names, merged, delnodes)
    }

    #[test]
    fn lineage_name_rank() {
        let t = tax();
        assert_eq!(t.lineage(562), vec![562, 561, 1224, 2, 131567, 1]);
        assert_eq!(t.name(2), Some("Bacteria"));
        assert_eq!(t.name(562), Some("Escherichia coli"));
        assert_eq!(t.rank(2), Some("superkingdom"));
        assert_eq!(t.rank(561), Some("genus"));
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
        let inc = t.compile_taxonomy_filter(Some(&[1224].into_iter().collect()), None).unwrap();
        assert!(inc.contains(&562) && inc.contains(&561) && inc.contains(&1236) && inc.contains(&1224));
        assert!(!inc.contains(&2) && !inc.contains(&131567));
        // Merged old-taxid inherits its replacement's verdict.
        assert!(inc.contains(&999000));

        let exc = t.compile_taxonomy_filter(None, Some(&[1224].into_iter().collect())).unwrap();
        assert!(exc.contains(&2) && exc.contains(&131567));
        assert!(!exc.contains(&562) && !exc.contains(&1224));
    }

    #[test]
    fn taxonomy_expression() {
        let t = tax();
        // Within Bacteria(2) but not Proteobacteria(1224).
        let a = t.compile_taxonomy_expression("2 & ~1224").unwrap();
        assert!(a.contains(&2));
        assert!(!a.contains(&562) && !a.contains(&1224) && !a.contains(&131567));
        // Anything under Proteobacteria.
        let b = t.compile_taxonomy_expression("1224").unwrap();
        assert!(b.contains(&1224) && b.contains(&562) && !b.contains(&2));
    }

    #[test]
    fn taxonomy_data_columns() {
        let t = tax();
        let td = TaxonomyData::new(&t, 562);
        assert_eq!(td.taxid_at("self"), Some(562));
        assert_eq!(td.taxid_at("superkingdom"), Some(2));
        assert_eq!(td.taxid_at("genus"), Some(561));
        assert_eq!(td.name_at("self"), Some("Escherichia coli"));
        assert_eq!(td.name_at("superkingdom"), Some("Bacteria"));
        // Root-first lineage; the lineage column drops the root.
        assert_eq!(td.lineage, vec![1, 131567, 2, 1224, 561, 562]);
        assert_eq!(td.taxid_at("order"), None);
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
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "truncated gzip"));
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
