//! E1 probe: confirm hmmer-pure-rs runs an hmmsearch and exposes the fields we
//! need to build a Domainator SearchResult. Usage: e1_probe <hmm> <fasta>
use std::path::Path;

use hmmer_pure_rs::profile::{profile_config, P7_LOCAL};
use hmmer_pure_rs::sequence::Sequence;
use hmmer_pure_rs::{Alphabet, Bg, OProfile, Pipeline, Profile, TopHits};

use domainator_core::io::fasta::read_fasta;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let hmm_path = &args[1];
    let fasta_path = &args[2];
    let z = 1000.0_f64; // domainate default -Z

    hmmer_pure_rs::logsum::p7_flogsuminit();
    let hmms = hmmfile_read(hmm_path);
    let records = read_fasta(fasta_path, Some("protein")).unwrap();
    println!(
        "loaded {} HMM(s), {} sequences; Z={}",
        hmms.len(),
        records.len(),
        z
    );

    for hmm in &hmms {
        let abc = Alphabet::new(hmm.abc_type);
        let bg = Bg::new(&abc);
        let mut gm = Profile::new(hmm.m, &abc);
        profile_config(hmm, &bg, &mut gm, 400, P7_LOCAL);
        let mut om = OProfile::convert(&gm);
        let mut pli = Pipeline::new();
        pli.new_model(&gm);
        let mut th = TopHits::new();

        for rec in &records {
            let dsq = abc.digitize(&rec.seq);
            let sq = Sequence {
                name: rec.id.clone(),
                acc: String::new(),
                desc: String::new(),
                dsq,
                n: rec.seq.len(),
                l: rec.seq.len(),
                taxid: -1,
            };
            pli.run(&mut gm, &mut om, &bg, hmm, &sq, &mut th);
        }
        th.sort_by_sortkey();
        println!(
            "\n=== HMM '{}' (m={}) : {} hits ===",
            hmm_name(hmm),
            hmm.m,
            th.hits.len()
        );
        for hit in th.hits.iter().take(8) {
            let evalue = (hit.lnp.exp()) * z;
            print!(
                "{:<28} score={:>7.1} evalue={:>10.2e} ndom={}",
                hit.name, hit.score, evalue, hit.ndom
            );
            if let Some(d) = hit.dcl.first() {
                let dev = d.lnp.exp() * z;
                print!(
                    " | dom: env[{},{}] ali[{},{}] bits={:.1} i-eval={:.2e}",
                    d.ienv, d.jenv, d.iali, d.jali, d.bitscore, dev
                );
                if let Some(ad) = &d.ad {
                    let (matches, total) = identity(&ad.mline);
                    print!(
                        " hmm[{}-{}] id={:.1}% ({}/{})",
                        ad.hmmfrom,
                        ad.hmmto,
                        100.0 * matches as f64 / total.max(1) as f64,
                        matches,
                        total
                    );
                }
            }
            println!();
        }
    }
}

/// Identity from the match line: count non-space alignment columns vs all columns.
fn identity(mline: &str) -> (usize, usize) {
    let total = mline
        .chars()
        .filter(|c| !c.is_whitespace())
        .count()
        .max(mline.len());
    // HMMER's mline: letter = identical, '+' = positive, ' ' = mismatch.
    let matches = mline.chars().filter(|c| c.is_ascii_alphabetic()).count();
    (matches, total)
}

fn hmmfile_read(path: &str) -> Vec<hmmer_pure_rs::Hmm> {
    hmmer_pure_rs::hmmfile::read_hmm_file(Path::new(path)).expect("read hmm")
}

fn hmm_name(hmm: &hmmer_pure_rs::Hmm) -> String {
    format!("{:?}", hmm.name)
}
