//! `deduplicate_genbank`: collapse redundant sequences from GenBank/FASTA input.
//!
//! - `hash` (pure Rust): exact-duplicate removal. Sequences are upper-cased and
//!   stripped of leading/trailing `*`, then grouped by equality (optionally over
//!   both nucleotide strands). This is a faithful port of the Python `hash`
//!   algorithm — identical grouping without needing SHA-256, since the hashes
//!   never appear in the output; equal sequences map to the same representative.
//! - `diamond` / `usearch`: shell out to the external clustering binary.
//! - `cd-hit` / `cd-hit-est`: deferred (returns an error).
//!
//! The representative of each cluster is the first input record that created it
//! (`hash`) or the cluster centroid the tool reports (`diamond`/`usearch`).
//! Output is always emitted in input order (representatives only).

use std::collections::HashMap;
use std::io::BufRead;
use std::path::Path;
use std::process::Command;

use crate::seq::translate::{complement, revcomp};
use crate::seq::SeqRecord;

use super::external::{find_binary, spawn_error, write_fasta_ids, TempDir};

/// Options for [`deduplicate`].
pub struct DeduplicateOptions {
    /// `hash` | `diamond` | `usearch` (`cd-hit`/`cd-hit-est` are deferred).
    pub algorithm: String,
    /// Identity threshold in `[0, 1]` (must be `1.0` for `hash`).
    pub id: f64,
    /// Consider both strands (nucleotide only).
    pub both_strands: bool,
    /// Default molecule type for FASTA / unit-less input.
    pub fasta_type: String,
    /// Worker threads for the external clusterer (`0` = all cores).
    pub cpu: usize,
    /// Override path to the external binary.
    pub bin_path: Option<String>,
    /// Extra `(flag, value)` args forwarded to the external clusterer (`--params`).
    pub params: Vec<(String, Option<String>)>,
}

impl Default for DeduplicateOptions {
    fn default() -> Self {
        DeduplicateOptions {
            algorithm: "hash".into(),
            id: 1.0,
            both_strands: false,
            fasta_type: "protein".into(),
            cpu: 0,
            bin_path: None,
            params: Vec::new(),
        }
    }
}

/// Result of clustering: representative record indices (input order) and the
/// `(representative_index, member_indices)` clusters (ordered by rep index).
pub struct DedupResult {
    pub representatives: Vec<usize>,
    pub clusters: Vec<(usize, Vec<usize>)>,
}

/// Cluster `records` and return which are representatives. Does no I/O on the
/// records themselves (naming / writing is the caller's job).
pub fn deduplicate(records: &[SeqRecord], opts: &DeduplicateOptions) -> Result<DedupResult, String> {
    if opts.both_strands && opts.fasta_type != "nucleotide" {
        return Err("--both_strands is only supported for nucleotide input.".into());
    }
    if opts.id < 0.0 || opts.id > 1.0 {
        return Err(format!("id must be between 0 and 1, not {}", opts.id));
    }
    match opts.algorithm.as_str() {
        "hash" => hash_dedup(records, opts),
        "diamond" => external_dedup(records, opts, ExternalTool::Diamond),
        "usearch" => external_dedup(records, opts, ExternalTool::Usearch),
        "cd-hit" | "cd-hit-est" => {
            Err("cd-hit deduplication is not yet supported in the Rust port (deferred).".into())
        }
        other => Err(format!("unknown --algorithm {other:?} (have: hash, diamond, usearch)")),
    }
}

/// The normalized sequence key: upper-cased, with leading/trailing `*` stripped.
fn hash_key(seq: &[u8]) -> Vec<u8> {
    let upper: Vec<u8> = seq.iter().map(|b| b.to_ascii_uppercase()).collect();
    let start = upper.iter().position(|&b| b != b'*').unwrap_or(upper.len());
    let end = upper.iter().rposition(|&b| b != b'*').map_or(start, |i| i + 1);
    upper[start..end].to_vec()
}

fn hash_dedup(records: &[SeqRecord], opts: &DeduplicateOptions) -> Result<DedupResult, String> {
    if opts.id != 1.0 {
        return Err(format!("hash algorithm only valid for id = 1, not {}.", opts.id));
    }
    // Sequence key -> representative index. For both_strands a record registers
    // its forward, complement, and reverse-complement keys (mirroring Python).
    let mut seen: HashMap<Vec<u8>, usize> = HashMap::new();
    let mut members: HashMap<usize, Vec<usize>> = HashMap::new();

    for (i, rec) in records.iter().enumerate() {
        let key = hash_key(&rec.seq);
        let mut keys = vec![key.clone()];
        if opts.both_strands {
            keys.push(complement(&key));
            keys.push(revcomp(&key));
        }
        let rep = keys.iter().find_map(|k| seen.get(k).copied());
        let rep = match rep {
            Some(r) => r,
            None => {
                for k in &keys {
                    seen.entry(k.clone()).or_insert(i);
                }
                i
            }
        };
        members.entry(rep).or_default().push(i);
    }

    finalize(members)
}

enum ExternalTool {
    Diamond,
    Usearch,
}

fn external_dedup(
    records: &[SeqRecord],
    opts: &DeduplicateOptions,
    tool: ExternalTool,
) -> Result<DedupResult, String> {
    // Molecule type of the first record decides protein-vs-nucleotide behavior
    // (mirrors Python's `seqtype = first rec molecule_type`).
    let seqtype = records
        .first()
        .and_then(|r| r.molecule_type.clone())
        .unwrap_or_else(|| opts.fasta_type.clone());
    if matches!(tool, ExternalTool::Diamond) && seqtype != "protein" {
        return Err("diamond deduplication is only supported for protein input.".into());
    }

    let tmp = TempDir::new("domainator_dedup").map_err(|e| e.to_string())?;
    let input_fasta = tmp.path().join("input.fasta");
    // id=str(i), sequence upper-cased with `*` -> `f` (matches Python's fasta write).
    let cleaned: Vec<(String, Vec<u8>)> = records
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let seq: Vec<u8> = r
                .seq
                .iter()
                .map(|b| {
                    let u = b.to_ascii_uppercase();
                    if u == b'*' { b'f' } else { u }
                })
                .collect();
            (i.to_string(), seq)
        })
        .collect();
    write_fasta_ids(&cleaned, &input_fasta).map_err(|e| e.to_string())?;

    let cpus = if opts.cpu == 0 { num_cpus() } else { opts.cpu };
    let clusters_by_str = match tool {
        ExternalTool::Diamond => run_diamond(opts, &tmp, &input_fasta, cpus)?,
        ExternalTool::Usearch => run_usearch(opts, &tmp, &input_fasta, cpus, records)?,
    };

    // Map str-index cluster keys back to numeric indices.
    let mut members: HashMap<usize, Vec<usize>> = HashMap::new();
    for (rep, mem) in clusters_by_str {
        let rep_i: usize = rep.parse().map_err(|_| format!("bad cluster id {rep:?}"))?;
        let mut mem_i: Vec<usize> = mem
            .iter()
            .map(|m| m.parse::<usize>().map_err(|_| format!("bad member id {m:?}")))
            .collect::<Result<_, _>>()?;
        mem_i.sort_unstable();
        members.insert(rep_i, mem_i);
    }
    finalize(members)
}

/// Turn a `rep -> members` map into a [`DedupResult`], ordered by rep index.
fn finalize(members: HashMap<usize, Vec<usize>>) -> Result<DedupResult, String> {
    let mut reps: Vec<usize> = members.keys().copied().collect();
    reps.sort_unstable();
    let clusters: Vec<(usize, Vec<usize>)> =
        reps.iter().map(|&r| (r, members[&r].clone())).collect();
    Ok(DedupResult { representatives: reps, clusters })
}

fn run_diamond(
    opts: &DeduplicateOptions,
    tmp: &TempDir,
    input_fasta: &Path,
    cpus: usize,
) -> Result<Vec<(String, Vec<String>)>, String> {
    let bin = match &opts.bin_path {
        Some(p) => std::path::PathBuf::from(p),
        None => find_binary("diamond", &["DOMAINATOR_DIAMOND", "DIAMOND"], "diamond"),
    };
    let out_tsv = tmp.path().join("output.tsv");
    let mut cmd = Command::new(&bin);
    cmd.arg("cluster")
        .arg("-d")
        .arg(input_fasta)
        .arg("-o")
        .arg(&out_tsv)
        .arg("--approx-id")
        .arg(format!("{}", opts.id * 100.0))
        .arg("--threads")
        .arg(cpus.to_string());
    append_params(&mut cmd, &opts.params);
    let out = cmd
        .output()
        .map_err(|e| spawn_error(e, &bin, "Install diamond or set $DOMAINATOR_DIAMOND.").to_string())?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        // Before diamond 2.2.5 the cluster writer crashed when the whole input collapsed
        // into a single cluster, reporting "max_oid not set" even though the clustering
        // itself had succeeded. Upstream dropped its longest-sequence fallback when it
        // raised the prereq to >= 2.2.5; name the version here so a user on an older
        // diamond gets an actionable error rather than a bare crash message.
        if ["max_oid not set", "Record count not set"].iter().any(|e| stderr.contains(e)) {
            return Err(format!(
                "diamond cluster failed on an input that collapses into a single cluster. \
                 This is a bug in diamond before 2.2.5; upgrade diamond (or set \
                 $DOMAINATOR_DIAMOND to a newer binary).\n{stderr}"
            ));
        }
        return Err(format!("diamond cluster failed:\n{stderr}"));
    }
    parse_diamond_tsv(&out_tsv)
}

fn parse_diamond_tsv(path: &Path) -> Result<Vec<(String, Vec<String>)>, String> {
    let mut order: Vec<String> = Vec::new();
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    for line in std::io::BufReader::new(file).lines() {
        let line = line.map_err(|e| e.to_string())?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 2 {
            continue;
        }
        let (rep, member) = (parts[0], parts[1]);
        if rep.eq_ignore_ascii_case("representative") && member.eq_ignore_ascii_case("member") {
            continue;
        }
        let entry = map.entry(rep.to_string()).or_insert_with(|| {
            order.push(rep.to_string());
            Vec::new()
        });
        if !entry.contains(&member.to_string()) {
            entry.push(member.to_string());
        }
    }
    // Ensure each representative is a member of its own cluster.
    for rep in &order {
        let mem = map.get_mut(rep).unwrap();
        if !mem.contains(rep) {
            mem.insert(0, rep.clone());
        }
    }
    Ok(order.into_iter().map(|r| (r.clone(), map.remove(&r).unwrap())).collect())
}

fn run_usearch(
    opts: &DeduplicateOptions,
    tmp: &TempDir,
    input_fasta: &Path,
    cpus: usize,
    _records: &[SeqRecord],
) -> Result<Vec<(String, Vec<String>)>, String> {
    let bin = match &opts.bin_path {
        Some(p) => std::path::PathBuf::from(p),
        None => find_binary("usearch", &["DOMAINATOR_USEARCH", "USEARCH"], "usearch"),
    };
    let centroids = tmp.path().join("output.fasta");
    let uc = tmp.path().join("output.uc");
    let mut cmd = Command::new(&bin);
    cmd.arg("-sort")
        .arg("length")
        .arg("-threads")
        .arg(cpus.to_string())
        .arg("-cluster_fast")
        .arg(input_fasta)
        .arg("-id")
        .arg(format!("{}", opts.id))
        .arg("-centroids")
        .arg(&centroids)
        .arg("-uc")
        .arg(&uc);
    let mut params = opts.params.clone();
    if opts.both_strands && !params.iter().any(|(k, _)| k == "-strand") {
        params.push(("-strand".into(), Some("both".into())));
    }
    append_params(&mut cmd, &params);
    let out = cmd
        .output()
        .map_err(|e| spawn_error(e, &bin, "Install usearch or set $DOMAINATOR_USEARCH.").to_string())?;
    if !out.status.success() {
        return Err(format!("usearch failed:\n{}", String::from_utf8_lossy(&out.stderr)));
    }
    parse_uc(&uc)
}

/// Parse a USEARCH `.uc` cluster file into `(centroid, members)` clusters.
fn parse_uc(path: &Path) -> Result<Vec<(String, Vec<String>)>, String> {
    let mut order: Vec<String> = Vec::new();
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    for line in std::io::BufReader::new(file).lines() {
        let line = line.map_err(|e| e.to_string())?;
        let parts: Vec<&str> = line.trim().split('\t').collect();
        if parts.is_empty() {
            continue;
        }
        match parts[0] {
            "S" if parts.len() > 8 => {
                let c = parts[8].to_string();
                map.entry(c.clone()).or_insert_with(|| {
                    order.push(c.clone());
                    Vec::new()
                });
                map.get_mut(&c).unwrap().push(parts[8].to_string());
            }
            "H" if parts.len() > 9 => {
                let c = parts[9].to_string();
                map.entry(c.clone()).or_insert_with(|| {
                    order.push(c.clone());
                    Vec::new()
                });
                map.get_mut(&c).unwrap().push(parts[8].to_string());
            }
            _ => {}
        }
    }
    Ok(order.into_iter().map(|c| (c.clone(), map.remove(&c).unwrap())).collect())
}

fn append_params(cmd: &mut Command, params: &[(String, Option<String>)]) {
    for (k, v) in params {
        cmd.arg(k);
        if let Some(v) = v {
            cmd.arg(v);
        }
    }
}


fn num_cpus() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seq::{Annotations, SeqRecord, Topology};

    fn rec(id: &str, seq: &[u8], mol: &str) -> SeqRecord {
        SeqRecord {
            id: id.into(),
            name: id.into(),
            description: String::new(),
            seq: seq.to_vec(),
            molecule_type: Some(mol.into()),
            topology: Topology::Linear,
            annotations: Annotations::default(),
            features: Vec::new(),
        }
    }

    #[test]
    fn hash_dedups_exact_and_star_stripped() {
        // Records 0 and 2 are identical after upper + `*` strip; 1 is distinct.
        let recs = vec![
            rec("a", b"ACDEF", "protein"),
            rec("b", b"GGGGG", "protein"),
            rec("c", b"*acdef*", "protein"),
        ];
        let r = deduplicate(&recs, &DeduplicateOptions::default()).unwrap();
        assert_eq!(r.representatives, vec![0, 1]);
        assert_eq!(r.clusters, vec![(0, vec![0, 2]), (1, vec![1])]);
    }

    #[test]
    fn hash_both_strands_groups_revcomp() {
        // Record 1 is the reverse-complement of record 0 -> same cluster.
        let recs = vec![
            rec("a", b"AAATTTGGG", "nucleotide"),
            rec("b", &revcomp(b"AAATTTGGG"), "nucleotide"),
        ];
        let opts = DeduplicateOptions {
            fasta_type: "nucleotide".into(),
            both_strands: true,
            ..Default::default()
        };
        let r = deduplicate(&recs, &opts).unwrap();
        assert_eq!(r.representatives, vec![0]);
        assert_eq!(r.clusters, vec![(0, vec![0, 1])]);
    }

    #[test]
    fn hash_requires_id_one_and_both_strands_needs_nucleotide() {
        let recs = vec![rec("a", b"ACDEF", "protein")];
        let bad_id = DeduplicateOptions { id: 0.9, ..Default::default() };
        assert!(deduplicate(&recs, &bad_id).is_err());
        let bad_strand =
            DeduplicateOptions { both_strands: true, fasta_type: "protein".into(), ..Default::default() };
        assert!(deduplicate(&recs, &bad_strand).is_err());
    }
}
