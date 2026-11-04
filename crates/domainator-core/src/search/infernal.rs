//! Infernal `cmsearch` via infernox: search covariance models against whole
//! nucleotide contigs (both strands), producing [`NamedHit`]s (mirrors
//! `domainate.run_search` for the `infernal` program +
//! `infernal_hits_to_search_results`).
//!
//! Python calls `pyinfernal.cmsearch(cms, block)` with no options, which differs
//! from the `cmsearch` command line in one place: the search space `Z` is the
//! block's residue count, **not** doubled for the two strands. Everything else is
//! cmsearch's default pipeline (local, truncated hits allowed, HMM filters, `E = 10`
//! as the scan and report cutoff), which infernox reproduces byte for byte.

use std::io::{BufReader, Read};
use std::path::Path;

use infernox::cm::CM_EXPTAIL_STATS;
use infernox::cm_file::cm_file_read_from_reader_opt;
use infernox::cm_search::{FaithfulConfig, FaithfulSearcher};
use infernox::constants::MATP_ND;

use super::hmmer::NamedHit;
use super::results::SearchResult;

/// pyinfernal's `Pipeline` default `E`, the cutoff the scan itself applies. Python
/// passes no `E`, so `--evalue` above it cannot find more hits; it only filters.
const PIPELINE_E: f64 = 10.0;

/// One covariance model's identity, beside its prepared searcher.
pub struct CmInfo {
    pub name: String,
    pub acc: String,
    pub desc: String,
    /// The model's state count (`CM.M`), which Python uses both as the hit's
    /// reference length and as the circular-contig extension.
    pub m: usize,
}

/// A `.cm` reference database: every model in one file, searched together.
pub struct CmDb {
    pub name: String,
    /// Parallel to `searchers`.
    pub models: Vec<CmInfo>,
    searchers: Vec<FaithfulSearcher>,
}

impl CmDb {
    /// The largest state count among the models (`0` for an empty file).
    pub fn max_states(&self) -> usize {
        self.models.iter().map(|m| m.m).max().unwrap_or(0)
    }
}

fn invalid(message: String) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message)
}

/// Split a multi-model CM file into one text per model. A record starts at its
/// `INFERNAL1/a` format line and runs through its trailing p7 filter HMM, so the
/// `//` lines are no use as separators (infernox's `cmsearch` splits the same way).
fn split_cm_records(text: &str) -> Vec<&str> {
    let mut starts: Vec<usize> = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        if line.trim_start().starts_with("INFERNAL") {
            starts.push(offset);
        }
        offset += line.len();
    }
    let mut records: Vec<&str> = Vec::new();
    for (i, &start) in starts.iter().enumerate() {
        let end = starts.get(i + 1).copied().unwrap_or(text.len());
        records.push(&text[start..end]);
    }
    records
}

/// Load every model in a `.cm` file (port of `hmmio.read_infernal_cms` for one file).
///
/// Models are keyed by name as in Python's `OrderedDict`: a repeated name keeps the
/// first one's position and the last one's model, with a warning. A model that needs
/// CM E-value statistics but was never run through `cmcalibrate` is an error here,
/// before any search starts.
pub fn load_cm_db(file: &str) -> std::io::Result<CmDb> {
    let mut text = String::new();
    crate::io::open_decompressed(Path::new(file))?.read_to_string(&mut text)?;
    let mut models: Vec<CmInfo> = Vec::new();
    let mut searchers: Vec<FaithfulSearcher> = Vec::new();
    for record in split_cm_records(&text) {
        let cm = cm_file_read_from_reader_opt(BufReader::new(record.as_bytes()), false)
            .map_err(|e| invalid(format!("reading covariance model from {file:?}: {e:?}")))?;
        let info = CmInfo {
            name: cm.name.clone(),
            acc: cm.acc.clone().unwrap_or_default(),
            desc: cm.desc.clone().unwrap_or_default(),
            m: usize::try_from(cm.m).unwrap_or(0),
        };
        let searcher = FaithfulSearcher::new(cm)
            .map_err(|e| invalid(format!("{} in {file:?}: {e}", info.name)))?;
        // C cmsearch: a model with base pairs is scored by the CM, whose E-values
        // come from `cmcalibrate`'s exponential tails.
        let basepairs = searcher
            .cm()
            .ndtype
            .iter()
            .any(|&t| i32::from(t) == MATP_ND);
        if basepairs && searcher.cm().flags & CM_EXPTAIL_STATS == 0 {
            return Err(invalid(format!(
                "no E-value parameters were read for CM {} in {file:?}; you may need to run \
                 cmcalibrate",
                info.name
            )));
        }
        match models.iter().position(|m| m.name == info.name) {
            Some(i) => {
                crate::log::warn(format!(
                    "multiple covariance models with the same name ({}) in file: {file}, only \
                     one will be used.",
                    info.name
                ));
                models[i] = info;
                searchers[i] = searcher;
            }
            None => {
                models.push(info);
                searchers.push(searcher);
            }
        }
    }
    Ok(CmDb {
        name: crate::io::db_name_from_path(file),
        models,
        searchers,
    })
}

/// The letters easel's RNA alphabet digitizes (T as U, X as N, I as A, the IUPAC
/// degeneracy codes), in either case. pyhmmer refuses a sequence with any other.
fn is_rna_symbol(b: u8) -> bool {
    matches!(
        b.to_ascii_uppercase(),
        b'A' | b'B'
            | b'C'
            | b'D'
            | b'G'
            | b'H'
            | b'I'
            | b'K'
            | b'M'
            | b'N'
            | b'R'
            | b'S'
            | b'T'
            | b'U'
            | b'V'
            | b'W'
            | b'X'
            | b'Y'
    )
}

/// infernox takes `Z` in megabases and truncates `Z * 1e6` to an integer, so step
/// the float up until that lands exactly on `residues`.
fn z_megabases(residues: usize) -> f64 {
    #[allow(clippy::cast_precision_loss)] // a contig set is far below 2^52 residues
    let target = residues as f64;
    let mut mb = target / 1e6;
    #[allow(clippy::cast_possible_truncation)] // infernox's own `(mb * 1e6) as i64`
    while ((mb * 1e6) as i64 as f64) < target {
        mb = mb.next_up();
    }
    mb
}

/// Run every model in `db` against the whole-contig `targets` (`(name, residues)`,
/// both strands), returning hits with `min_evalue <= evalue <= max_evalue`. Target
/// names follow Python's `"{contig_index},contig,{len}"`; a hit starting in a
/// circular contig's extension (at or past `len`) is dropped, as Python does.
///
/// Hits come per model in file order, each model's in cmsearch's E-value order,
/// which is how Python walks pyinfernal's `TopHits`.
pub fn cmsearch_db(
    db: &CmDb,
    targets: &[(String, Vec<u8>)],
    max_evalue: f64,
    min_evalue: f64,
) -> std::io::Result<Vec<NamedHit>> {
    if db.searchers.is_empty() || targets.is_empty() {
        return Ok(Vec::new());
    }
    let mut seqs: Vec<&str> = Vec::with_capacity(targets.len());
    for (name, residues) in targets {
        let seq = std::str::from_utf8(residues)
            .ok()
            .filter(|s| s.bytes().all(is_rna_symbol))
            .ok_or_else(|| {
                invalid(format!(
                    "{name}: cannot digitize sequence with alphabet RNA: invalid chars in \
                     sequence"
                ))
            })?;
        seqs.push(seq);
    }
    let residues: usize = seqs.iter().map(|s| s.len()).sum();
    let cfg = FaithfulConfig {
        e_report: PIPELINE_E,
        // The library default is off; cmsearch's (and pyinfernal's) is on.
        notrunc: false,
        // pyinfernal runs the pipeline over each whole sequence; C cmsearch (and
        // infernox's default) over 100 kb windows, which scores long contigs
        // differently near the threshold. A vendored-infernox addition.
        whole_sequence: true,
        z_mb_override: Some(z_megabases(residues)),
        ..FaithfulConfig::default()
    };
    // The default Forward filter fuses multiply-adds where the CPU has FMA, which
    // changes scores in the last bits and makes x86 and ARM disagree; strict mode is
    // C Infernal's arithmetic.
    infernox::cm_pipeline::set_forward_strict(true);
    let per_model = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        FaithfulSearcher::search_many(&db.searchers, &seqs, &cfg)
    }))
    .map_err(|_| std::io::Error::other(format!("infernal search with {} failed", db.name)))?;

    let mut out = Vec::new();
    for (model, hits) in db.models.iter().zip(per_model) {
        for hit in hits {
            if hit.evalue > max_evalue || hit.evalue < min_evalue {
                continue;
            }
            let target = &targets[hit.seq_idx].0;
            let contig_length: Option<i64> = target.split(',').nth(2).and_then(|s| s.parse().ok());
            let start = hit.start.min(hit.stop) - 1;
            let end = hit.start.max(hit.stop);
            if contig_length.is_some_and(|len| start >= len) {
                continue;
            }
            out.push(NamedHit {
                target: target.clone(),
                result: SearchResult {
                    name: model.name.clone(),
                    desc: model.desc.clone(),
                    acc: model.acc.clone(),
                    evalue: hit.evalue,
                    score: f64::from(hit.score),
                    start,
                    end,
                    database: db.name.clone(),
                    identity: 0.0,
                    rstart: i64::from(hit.mdl_from),
                    rend: i64::from(hit.mdl_to),
                    rlen: i64::try_from(model.m).unwrap_or(i64::MAX),
                    program: "infernal".into(),
                    strand: if hit.in_rc { -1 } else { 1 },
                    ..SearchResult::default()
                },
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_split_at_each_format_line() {
        let text = "INFERNAL1/a\nNAME a\nCM\n//\nHMMER3/f\n//\nINFERNAL1/a\nNAME b\n//\n";
        let records = split_cm_records(text);
        assert_eq!(records.len(), 2);
        assert!(records[0].ends_with("HMMER3/f\n//\n"));
        assert!(records[1].starts_with("INFERNAL1/a\nNAME b"));
        assert!(split_cm_records("").is_empty());
    }

    #[test]
    fn z_lands_exactly_on_the_residue_count() {
        for residues in [0usize, 1, 7, 457_887, 1_000_000, 4_641_652, 123_456_789] {
            let mb = z_megabases(residues);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let z = (mb * 1e6) as i64 as usize;
            assert_eq!(z, residues);
        }
    }

    #[test]
    fn rna_symbols_match_easel() {
        let accepted: String = (b'A'..=b'Z')
            .filter(|&b| is_rna_symbol(b))
            .map(char::from)
            .collect();
        assert_eq!(accepted, "ABCDGHIKMNRSTUVWXY");
        assert!(is_rna_symbol(b't'));
        assert!(!is_rna_symbol(b'-'));
    }
}
