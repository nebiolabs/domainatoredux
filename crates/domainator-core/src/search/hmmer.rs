//! hmmsearch via hmmer-pure-rs: load HMM databases and search profiles against
//! named peptides, producing [`SearchResult`]s (mirrors `domainate.run_search` +
//! `hmmer_hits_to_search_results` for the `hmmsearch` program).

use std::path::Path;

use hmmer_pure_rs::profile::{profile_config, P7_LOCAL};
use hmmer_pure_rs::sequence::Sequence;
use hmmer_pure_rs::{Alphabet, Bg, Hmm, OProfile, Pipeline, Profile, TopHits};

use super::results::SearchResult;

/// phmmer gap-open / gap-extend probabilities (HMMER defaults).
const PHMMER_POPEN: f32 = 0.02;
const PHMMER_PEXTEND: f32 = 0.4;

/// An HMM reference database: a set of profiles searched against protein peptides
/// under a file-stem `name` (the value written into the `database` qualifier).
///
/// The profiles are HMMs whether they came from an `.hmm` file (hmmsearch) or were
/// built from single reference sequences (phmmer). `program` labels which — it is
/// written verbatim into the output `program` qualifier and selects the phmmer
/// background-filter setup used by [`search_protein_db`].
pub struct HmmDb {
    pub name: String,
    pub hmms: Vec<Hmm>,
    /// `"hmmsearch"` or `"phmmer"`.
    pub program: String,
    /// When `true` (phmmer), configure the background filter from each profile's
    /// composition before scoring (mirrors pyhmmer.phmmer's single-seq builder).
    pub phmmer_bg_filter: bool,
    /// Per-hmm reference length override for the output `rlen` qualifier. Empty =
    /// use each profile's `M`. phmmer sets this to the reference sequence length
    /// (which equals `M` for single-seq profiles, but is tracked explicitly to
    /// match Python's `len(reference.sequence)`).
    pub rlen: Vec<i64>,
}

/// Load HMM databases from `.hmm` files. `name` is the file stem (e.g.
/// `FeSOD_pfam.hmm` -> `FeSOD_pfam`). All profiles are searched with `hmmsearch`.
pub fn load_hmm_dbs(files: &[String]) -> std::io::Result<Vec<HmmDb>> {
    let mut out = Vec::new();
    for f in files {
        let hmms = crate::search::hmm_read::read_hmm_file(Path::new(f))
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{e:?}")))?;
        let name = file_stem(f);
        out.push(HmmDb { name, hmms, program: "hmmsearch".to_string(), phmmer_bg_filter: false, rlen: Vec::new() });
    }
    Ok(out)
}

/// Build a phmmer `HmmDb` from a protein FASTA file: one single-sequence profile
/// per reference sequence (mirrors `read_pyhmmer_peptide_fastas` +
/// `pyhmmer.hmmer.phmmer`). Each profile's `name`/`desc`/`acc` come from the
/// FASTA record, so the output `name` qualifier is the reference sequence id.
pub fn load_phmmer_db(file: &str) -> std::io::Result<HmmDb> {
    let recs = crate::io::fasta::read_fasta(file, Some("protein"))?;
    let abc = Alphabet::amino();
    let bg = Bg::new(&abc);
    let mut hmms = Vec::new();
    let mut rlen = Vec::new();
    for rec in &recs {
        // Mirror pyhmmer's easel digitization: keep every alphabet symbol
        // (including a trailing `*` stop, mapped to a residue), drop only
        // whitespace. This is why M includes the stop (Python's rlen = 102, not
        // 101, for a 101-aa + `*` sequence).
        let residues: Vec<u8> =
            rec.seq.iter().copied().filter(|b| !b.is_ascii_whitespace()).collect();
        let dsq = abc.digitize(&residues);
        let ref_len = dsq.len().saturating_sub(2); // strip the two sentinels
        if ref_len == 0 {
            continue;
        }
        let mut hmm = hmmer_pure_rs::seqmodel::build_single_seq_hmm(
            &fasta_description(&rec.description).0, &dsq, ref_len, &abc, &bg, PHMMER_POPEN, PHMMER_PEXTEND,
        );
        // build_single_seq_hmm names the profile after the sequence id; carry the
        // FASTA id/description so the output `name`/`description` match Python.
        let (id, desc) = fasta_description(&rec.description);
        hmm.name = id;
        hmm.desc = desc;
        rlen.push(ref_len as i64);
        hmms.push(hmm);
    }
    Ok(HmmDb { name: file_stem(file), hmms, program: "phmmer".to_string(), phmmer_bg_filter: true, rlen })
}

fn file_stem(f: &str) -> String {
    Path::new(f)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("references")
        .to_string()
}

/// Split a FASTA title into `(id, description)` the way easel/pyhmmer does: the id
/// is up to the first whitespace, the description is the remainder (`None` when
/// absent, so it renders as `.` in output — matching `reference.description` being
/// `None` for title-only records).
pub fn fasta_description(title: &str) -> (String, Option<String>) {
    match title.split_once(char::is_whitespace) {
        Some((id, rest)) => {
            let rest = rest.trim();
            (id.to_string(), if rest.is_empty() { None } else { Some(rest.to_string()) })
        }
        None => (title.to_string(), None),
    }
}

/// One domain hit tagged with the target peptide name (`"{contig},{cds}"`).
pub struct NamedHit {
    pub target: String,
    pub result: SearchResult,
}

/// Run every profile in `db` against `peptides` (`(name, residues)`), returning
/// the domains with `min_evalue <= i_evalue < evalue`. `z` is the E-value search
/// space (domainate's `-Z`). Handles both hmmsearch (`.hmm` profiles) and phmmer
/// (single-seq profiles, `db.phmmer_bg_filter` sets the composition filter); the
/// output `program` qualifier is `db.program`.
pub fn hmmsearch_db(
    db: &HmmDb,
    peptides: &[(String, Vec<u8>)],
    z: f64,
    evalue: f64,
    min_evalue: f64,
    max_mode: bool,
) -> Vec<NamedHit> {
    let mut out = Vec::new();
    for (hi, hmm) in db.hmms.iter().enumerate() {
        let abc = Alphabet::new(hmm.abc_type);
        let mut bg = Bg::new(&abc);
        // phmmer configures the background filter from the single-seq profile's
        // composition (mirrors pyhmmer.phmmer / seq_compare::phmmer_scores).
        if db.phmmer_bg_filter {
            bg.set_filter(hmm.m, &hmm.compo);
        }
        let mut gm = Profile::new(hmm.m, &abc);
        profile_config(hmm, &bg, &mut gm, 400, P7_LOCAL);
        let mut om = OProfile::convert(&gm);
        let mut pli = Pipeline::new();
        // `--max_mode` (hmmsearch `--max`): skip the MSV/Viterbi/Fwd heuristic
        // filters for maximum sensitivity.
        pli.do_max = max_mode;
        pli.new_model(&gm);
        let mut th = TopHits::new();

        for (name, pep) in peptides {
            if pep.is_empty() {
                continue;
            }
            let dsq = abc.digitize(pep);
            let sq = Sequence {
                name: name.clone(),
                acc: String::new(),
                desc: String::new(),
                dsq,
                n: pep.len(),
                l: pep.len(),
            };
            pli.run(&mut gm, &mut om, &bg, hmm, &sq, &mut th);
        }

        let desc = hmm.desc.clone().unwrap_or_default();
        let acc = hmm.acc.clone().unwrap_or_default();
        let rlen = db.rlen.get(hi).copied().unwrap_or(hmm.m as i64);
        for hit in &th.hits {
            for dom in &hit.dcl {
                let i_evalue = dom.lnp.exp() * z;
                if !(i_evalue < evalue && i_evalue >= min_evalue) {
                    continue;
                }
                let (rstart, rend, identity) = match &dom.ad {
                    Some(ad) => {
                        let matches = ad.mline.chars().filter(|c| c.is_ascii_alphabetic()).count();
                        let aln_len = (ad.hmmto as i64 - (ad.hmmfrom as i64 - 1)).max(1);
                        (ad.hmmfrom as i64, ad.hmmto as i64, 100.0 * matches as f64 / aln_len as f64)
                    }
                    None => (0, 0, 0.0),
                };
                out.push(NamedHit {
                    target: hit.name.clone(),
                    result: SearchResult {
                        name: hmm.name.clone(),
                        desc: desc.clone(),
                        acc: acc.clone(),
                        evalue: i_evalue,
                        score: dom.bitscore as f64,
                        start: dom.ienv - 1,
                        end: dom.jenv,
                        database: db.name.clone(),
                        identity,
                        rstart,
                        rend,
                        rlen,
                        program: db.program.clone(),
                        strand: 1,
                        // Structural metrics are absent for sequence backends.
                        ..SearchResult::default()
                    },
                });
            }
        }
    }
    out
}
