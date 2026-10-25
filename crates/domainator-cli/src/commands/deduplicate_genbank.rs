use std::collections::HashMap;
use std::io::Write;

use anyhow::{anyhow, Result};
use domainator_config::args::DeduplicateGenbankArgs;
use domainator_core::seq::SeqRecord;
use domainator_core::tools::deduplicate_genbank::{deduplicate, DeduplicateOptions};

use super::{cpu_threads, load_typed_inputs, usage_error};
use crate::output::{write_seq_outputs, Output};

pub fn run(args: &DeduplicateGenbankArgs) -> Result<()> {
    let cpu = cpu_threads(args.cpu)?;
    // Each clusterer's extra arguments, which only that algorithm may be given.
    let mut extra = None;
    for (algorithm, option, value) in [
        ("cd-hit", "cdhit", &args.cdhit_args),
        ("usearch", "usearch", &args.usearch_args),
        ("diamond", "diamond", &args.diamond_args),
    ] {
        match value {
            Some(_) if algorithm != args.algorithm => {
                return Err(usage_error(format!(
                    "--{option}_args was given, but --algorithm is {}.",
                    args.algorithm
                )))
            }
            Some(v) => {
                extra = Some(shlex::split(v).ok_or_else(|| {
                    anyhow!("--{option}_args: unbalanced quotes or a trailing backslash in {v:?}")
                })?)
            }
            None => {}
        }
    }
    let identity = args.identity.0;
    if !(0.0..=1.0).contains(&identity) {
        return Err(usage_error(format!(
            "--identity must be between 0 and 1, not {identity}"
        )));
    }
    let bin_path = match args.algorithm.as_str() {
        "cd-hit" => args.cdhit_path.clone(),
        "usearch" => args.usearch_path.clone(),
        "diamond" => args.diamond_path.clone(),
        _ => None,
    };
    let records = load_typed_inputs(&args.input, &args.alphabet)?;
    let opts = DeduplicateOptions {
        algorithm: args.algorithm.clone(),
        id: identity,
        both_strands: args.both_strands,
        fasta_type: args.alphabet.clone(),
        cpu,
        bin_path,
        cdhit_est_path: args.cdhit_est_path.clone(),
        extra_args: extra.unwrap_or_default(),
    };
    let result = deduplicate(&records, &opts).map_err(|e| anyhow!(e))?;

    // The representatives, in input order, named with their cluster size if asked.
    let count_by_rep: HashMap<usize, usize> =
        result.clusters.iter().map(|(r, m)| (*r, m.len())).collect();
    let mut final_name: HashMap<usize, String> = HashMap::new();
    let mut out_records: Vec<SeqRecord> = Vec::new();
    for &i in &result.representatives {
        let mut rec = records[i].clone();
        let count = count_by_rep.get(&i).copied().unwrap_or(1);
        if args.prefix_count {
            rec.id = format!("{count}-{}", rec.id);
        } else if args.suffix_count {
            rec.id = format!("{}-{count}", rec.id);
        }
        final_name.insert(i, rec.id.clone());
        out_records.push(rec);
    }
    write_seq_outputs(
        &out_records,
        args.genbank_out.as_deref(),
        args.fasta_out.as_deref(),
    )?;

    if let Some(path) = &args.cluster_tsv_out {
        let mut table = Output::create(path)?;
        writeln!(table, "representative\tcontigs")?;
        for (rep, members) in &result.clusters {
            let names: Vec<&str> = members.iter().map(|&m| records[m].id.as_str()).collect();
            let rep_name = final_name.get(rep).map_or("", String::as_str);
            writeln!(table, "{rep_name}\t{}", names.join(&args.cluster_sep))?;
        }
        table.finish()?;
    }

    let log = format!(
        "Input  size: {}\nOutput size: {}\n",
        records.len(),
        out_records.len()
    );
    match &args.log_out {
        Some(path) => {
            let mut out = Output::create(path)?;
            out.write_all(log.as_bytes())?;
            out.finish()?;
        }
        None => eprint!("{log}"),
    }
    Ok(())
}
