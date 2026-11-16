use std::collections::HashMap;
use std::io::Write;

use anyhow::{anyhow, Result};
use domainator_config::args::DeduplicateGenbankArgs;
use domainator_core::tools::deduplicate_genbank::{DeduplicateOptions, Deduplicator};

use super::{cpu_threads, usage_error};
use crate::input::{InputMode, SeqInputs};
use crate::output::{Output, SeqWriter};

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
    // Two passes over the input, as upstream makes them: one to cluster, holding only
    // each record's id (and a digest, or a line of the clusterer's FASTA), and one to
    // write the representatives. (Standard input, which cannot be read twice, is
    // refused, as upstream refuses it.)
    let inputs = &args.input;
    let mode = InputMode::Detect {
        fasta_type: &args.alphabet,
    };
    let mut deduplicator = Deduplicator::new(&opts).map_err(|e| anyhow!(e))?;
    let mut ids: Vec<String> = Vec::new();
    for record in SeqInputs::new(inputs, mode) {
        let record = record?;
        deduplicator.add(&record).map_err(|e| anyhow!(e))?;
        ids.push(record.id);
    }
    let result = deduplicator.finish().map_err(|e| anyhow!(e))?;

    // The representatives, in input order, named with their cluster size if asked.
    let count_by_rep: HashMap<usize, usize> =
        result.clusters.iter().map(|(r, m)| (*r, m.len())).collect();
    let mut final_name: HashMap<usize, String> = HashMap::new();
    let mut out = SeqWriter::new(args.genbank_out.as_deref(), args.fasta_out.as_deref(), None)?;
    let mut reps = result.representatives.iter().copied().peekable();
    for (i, record) in SeqInputs::new(inputs, mode).enumerate() {
        if reps.peek().is_none() {
            break;
        }
        let mut record = record?;
        if reps.peek() != Some(&i) {
            continue;
        }
        reps.next();
        let count = count_by_rep.get(&i).copied().unwrap_or(1);
        if args.prefix_count {
            record.id = format!("{count}-{}", record.id);
        } else if args.suffix_count {
            record.id = format!("{}-{count}", record.id);
        }
        final_name.insert(i, record.id.clone());
        out.write(&record)?;
    }
    if reps.peek().is_some() {
        anyhow::bail!("an input changed while it was being deduplicated");
    }
    let written = out.finish()?;

    if let Some(path) = &args.cluster_tsv_out {
        let mut table = Output::create(path)?;
        writeln!(table, "representative\tcontigs")?;
        for (rep, members) in &result.clusters {
            let names: Vec<&str> = members.iter().map(|&m| ids[m].as_str()).collect();
            let rep_name = final_name.get(rep).map_or("", String::as_str);
            writeln!(table, "{rep_name}\t{}", names.join(&args.cluster_sep))?;
        }
        table.finish()?;
    }

    let log = format!("Input  size: {}\nOutput size: {}\n", ids.len(), written);
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
