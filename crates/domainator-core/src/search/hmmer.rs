//! hmmsearch via hmmer-pure-rs: load HMM databases and search profiles against
//! named peptides, producing [`SearchResult`]s (mirrors `domainate.run_search` +
//! `hmmer_hits_to_search_results` for the `hmmsearch` program).

use std::path::Path;

use hmmer_pure_rs::profile::{profile_config, P7_LOCAL};
use hmmer_pure_rs::sequence::Sequence;
use hmmer_pure_rs::{Alphabet, Bg, Hmm, OProfile, Pipeline, Profile, TopHits};

use super::results::SearchResult;

/// An HMM reference database: a `.hmm` file's profiles under its file-stem name
/// (the value written into the `database` qualifier).
pub struct HmmDb {
    pub name: String,
    pub hmms: Vec<Hmm>,
}

/// Load HMM databases from `.hmm` files. `name` is the file stem (e.g.
/// `FeSOD_pfam.hmm` -> `FeSOD_pfam`).
pub fn load_hmm_dbs(files: &[String]) -> std::io::Result<Vec<HmmDb>> {
    let mut out = Vec::new();
    for f in files {
        let hmms = hmmer_pure_rs::hmmfile::read_hmm_file(Path::new(f))
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{e:?}")))?;
        let name = Path::new(f)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("references")
            .to_string();
        out.push(HmmDb { name, hmms });
    }
    Ok(out)
}

/// One domain hit tagged with the target peptide name (`"{contig},{cds}"`).
pub struct NamedHit {
    pub target: String,
    pub result: SearchResult,
}

/// Run every profile in `db` against `peptides` (`(name, residues)`), returning
/// the domains with `min_evalue <= i_evalue < evalue`. `z` is the E-value search
/// space (domainate's `-Z`).
pub fn hmmsearch_db(
    db: &HmmDb,
    peptides: &[(String, Vec<u8>)],
    z: f64,
    evalue: f64,
    min_evalue: f64,
    max_mode: bool,
) -> Vec<NamedHit> {
    let mut out = Vec::new();
    for hmm in &db.hmms {
        let abc = Alphabet::new(hmm.abc_type);
        let bg = Bg::new(&abc);
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
        let rlen = hmm.m as i64;
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
                        program: "hmmsearch".to_string(),
                        strand: 1,
                    },
                });
            }
        }
    }
    out
}
