//! `domainator <tool> [opts]` — single-executable CLI for the Domainator Rust
//! rewrite. During M0 this exposes development subcommands (`inspect`, `convert`)
//! used to validate the parser/writer against the Python fixtures; tool
//! subcommands (domainate, domain_search, seq_dist, compare_contigs) land in M1+.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use domainator_core::io::genbank::{read_genbank, write_genbank};
use domainator_core::io::{detect_filetype, fasta, FileType};
use domainator_core::seq::SeqRecord;
use domainator_core::tools::{domain_search, domainate};

#[derive(Parser)]
#[command(name = "domainator", version, about = "Domainator (Rust): protein-domain / SSN analysis")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Parse a sequence file and report per-record summary (dev/diagnostic).
    Inspect {
        /// GenBank or FASTA file (uncompressed).
        input: PathBuf,
        /// Print a one-line summary per record.
        #[arg(long)]
        verbose: bool,
    },
    /// Read a GenBank/FASTA file and write it back out as GenBank (round-trip).
    Convert {
        input: PathBuf,
        output: PathBuf,
    },
    /// Annotate sequences with hits to reference HMMs (protein hmmsearch path).
    Domainate {
        /// Input sequence file (GenBank or FASTA).
        #[arg(short, long)]
        input: PathBuf,
        /// Reference `.hmm` file(s).
        #[arg(short, long, num_args = 1..)]
        references: Vec<PathBuf>,
        /// Output GenBank file.
        #[arg(short, long)]
        output: PathBuf,
        /// E-value threshold.
        #[arg(short = 'e', long, default_value_t = 0.001)]
        evalue: f64,
        /// E-value search space size (-Z).
        #[arg(short = 'Z', long = "z", default_value_t = 1000.0)]
        z: f64,
        /// Maximum fractional domain overlap before filtering (>=1 disables).
        #[arg(long, default_value_t = 1.0)]
        max_overlap: f64,
    },
    /// Search references against a (possibly sharded/BGZF/indexed) database.
    DomainSearch {
        /// Input database (GenBank or FASTA; logical name resolves shards).
        #[arg(short, long)]
        input: PathBuf,
        /// Reference `.hmm` file(s).
        #[arg(short, long, num_args = 1..)]
        references: Vec<PathBuf>,
        /// Output GenBank file.
        #[arg(short, long)]
        output: PathBuf,
        /// E-value threshold.
        #[arg(short = 'e', long, default_value_t = 0.001)]
        evalue: f64,
        /// E-value search space size (-Z).
        #[arg(short = 'Z', long = "z", default_value_t = 1000.0)]
        z: f64,
        /// Add full Domainator annotations, not just the best-hit feature.
        #[arg(long)]
        add_annotations: bool,
        /// Cap on total hit contigs returned (top-N by best domain score).
        #[arg(long)]
        max_hits: Option<usize>,
        /// Approximate CDSs per partition.
        #[arg(long, default_value_t = 10000)]
        batch_size: u64,
    },
}

fn load(input: &PathBuf) -> Result<Vec<SeqRecord>> {
    let path = input.to_str().context("non-UTF8 path")?;
    match detect_filetype(input) {
        Some(FileType::Genbank) => read_genbank(path).context("reading GenBank"),
        Some(FileType::Fasta) => fasta::read_fasta(path, Some("protein")).context("reading FASTA"),
        None => anyhow::bail!("could not detect file type from extension: {}", input.display()),
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Inspect { input, verbose } => {
            let records = load(&input)?;
            println!("{} records", records.len());
            if verbose {
                for r in &records {
                    println!(
                        "  id={} name={} mol={:?} len={} features={}",
                        r.id, r.name, r.molecule_type, r.len(), r.features.len()
                    );
                }
            }
        }
        Command::Convert { input, output } => {
            let records = load(&input)?;
            let out = output.to_str().context("non-UTF8 path")?;
            write_genbank(out, &records).context("writing GenBank")?;
            println!("wrote {} records to {}", records.len(), output.display());
        }
        Command::Domainate { input, references, output, evalue, z, max_overlap } => {
            let records = load(&input)?;
            let ref_files: Vec<String> = references
                .iter()
                .map(|p| p.to_str().map(str::to_string).context("non-UTF8 path"))
                .collect::<Result<_>>()?;
            let dbs = domainate::read_hmm_references(&ref_files).context("reading HMM references")?;
            let opts = domainate::DomainateOptions { z, evalue, max_overlap, ..Default::default() };
            let annotated = domainate::domainate(records, &dbs, &opts);
            let out = output.to_str().context("non-UTF8 path")?;
            write_genbank(out, &annotated).context("writing GenBank")?;
            println!("wrote {} records to {}", annotated.len(), output.display());
        }
        Command::DomainSearch { input, references, output, evalue, z, add_annotations, max_hits, batch_size } => {
            let ref_files: Vec<String> = references
                .iter()
                .map(|p| p.to_str().map(str::to_string).context("non-UTF8 path"))
                .collect::<Result<_>>()?;
            let dbs = domainate::read_hmm_references(&ref_files).context("reading HMM references")?;
            let input_str = input.to_str().context("non-UTF8 path")?.to_string();
            let opts = domain_search::DomainSearchOptions {
                z, evalue, add_annotations, max_hits, batch_size, ..Default::default()
            };
            let hits = domain_search::domain_search(&[input_str], &dbs, &opts)
                .context("running domain_search")?;
            let out = output.to_str().context("non-UTF8 path")?;
            write_genbank(out, &hits).context("writing GenBank")?;
            println!("wrote {} hit contigs to {}", hits.len(), output.display());
        }
    }
    Ok(())
}
