use anyhow::{anyhow, bail, Result};
use domainator_config::args::KmerDistArgs;
use domainator_core::io::FileType;
use domainator_core::tools::kmer_dist::{kmer_dist, KmerDistOptions};

use super::cpu_threads;
use crate::output::{check_hdf_out, write_matrix_outputs};

pub fn run(args: &KmerDistArgs) -> Result<()> {
    check_hdf_out(args.hdf_out.as_deref(), None)?;
    if !(0.0..=1.0).contains(&args.threshold.0) {
        bail!("--threshold must be between 0 and 1 for k-mer similarities.");
    }
    let max_hits_per_query = args
        .max_hits_per_query
        .map(|k| usize::try_from(k).map_err(|_| anyhow!("--max_hits_per_query must be >= 0")))
        .transpose()?;
    if args.gene_call.is_some() && args.nucleotide_kmers {
        domainator_core::log::warn(
            "--gene_call has no effect when --nucleotide_kmers is supplied.",
        );
    }
    let max_bytes = crate::max_output_gb_to_bytes(args.max_output_gb.0)?;
    let cpu = cpu_threads(args.cpu)?;

    let opts = KmerDistOptions {
        max_hits_per_query,
        metric: args.metric.parse().map_err(|e: String| anyhow!(e))?,
        mode: args.mode.clone(),
        threshold: args.threshold.0,
        alphabet: args.alphabet.clone(),
        // Below 1 is refused by kmer_dist itself, after the inputs are checked.
        ksize: args.ksize.map(|k| usize::try_from(k).unwrap_or(0)),
        nucleotide_kmers: args.nucleotide_kmers,
        gene_call: crate::gene_call_request(args.gene_call.as_deref(), cpu)?,
        mst: args.mst,
        knn: args.knn.map(|k| k.0 as usize),
        cpu,
        progress: args.progress,
    };
    let matrix = kmer_dist(
        &args.input,
        format(args.input_format.as_deref()),
        args.reference.as_deref(),
        format(args.reference_format.as_deref()),
        &opts,
    )?;
    write_matrix_outputs(
        &matrix,
        args.hdf_out.as_deref(),
        args.dense_text_out.as_deref(),
        max_bytes,
        &["--max_hits_per_query", "--threshold", "--mst", "--knn"],
    )
}

/// An `--input_format`/`--reference_format` value.
fn format(name: Option<&str>) -> Option<FileType> {
    match name? {
        "fasta" => Some(FileType::Fasta),
        "genbank" => Some(FileType::Genbank),
        _ => None,
    }
}
