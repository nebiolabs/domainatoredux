//! `domainator <tool> [opts]` — single-executable CLI for the Domainator Rust
//! rewrite. During M0 this exposes development subcommands (`inspect`, `convert`)
//! used to validate the parser/writer against the Python fixtures; tool
//! subcommands (domainate, domain_search, seq_dist, compare_contigs) land in M1+.

use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use domainator_core::io::genbank::{read_genbank, write_genbank};
use domainator_core::io::{detect_filetype, fasta, FileType};
use domainator_core::matrix::{hdf5_io, text};
use domainator_core::seq::SeqRecord;
use domainator_core::tools::compare_contigs::{compare_contigs, CompareContigsOptions};
use domainator_core::tools::hmmer_compare::{hmmer_compare, HmmerCompareOptions};
use domainator_core::tools::seq_dist::{detect_kind, seq_dist, SeqDistOptions};
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
        /// Worker threads (0 = all cores).
        #[arg(long, default_value_t = 0)]
        cpu: usize,
        /// Extract whole matching contigs (nucleotide).
        #[arg(long = "whole_contig")]
        whole_contig: bool,
        /// Extract this many CDSs up- and downstream of each hit CDS.
        #[arg(long = "cds_range")]
        cds_range: Option<i64>,
        /// CDSs upstream of each hit CDS.
        #[arg(long = "cds_up")]
        cds_up: Option<i64>,
        /// CDSs downstream of each hit CDS.
        #[arg(long = "cds_down")]
        cds_down: Option<i64>,
        /// Extract this many kb up- and downstream of each hit CDS.
        #[arg(long = "kb_range")]
        kb_range: Option<f64>,
        /// kb upstream of each hit CDS.
        #[arg(long = "kb_up")]
        kb_up: Option<f64>,
        /// kb downstream of each hit CDS.
        #[arg(long = "kb_down")]
        kb_down: Option<f64>,
        /// Only extract regions around hit CDSs on this strand (f/r).
        #[arg(long)]
        strand: Option<String>,
        /// Keep the hit CDS on its original strand (don't reverse-complement).
        #[arg(long = "keep_direction")]
        keep_direction: bool,
        /// Translate nucleotide CDS hits to peptides (no neighborhood extraction).
        #[arg(long)]
        translate: bool,
        /// Max fractional overlap between output regions (>=1 disables).
        #[arg(long = "max_region_overlap", default_value_t = 1.0)]
        max_region_overlap: f64,
    },
    /// Compute a similarity matrix between input and reference sequences/profiles.
    SeqDist {
        /// Input query file (FASTA/GenBank; HMM input needs hmmer_compare, deferred).
        #[arg(short, long)]
        input: PathBuf,
        /// Reference file (FASTA/GenBank/HMM). Defaults to the input (all-vs-all).
        #[arg(short, long)]
        reference: Option<PathBuf>,
        /// Write a dense HDF5 matrix here.
        #[arg(long)]
        dense: Option<PathBuf>,
        /// Write a dense TSV matrix here.
        #[arg(long = "dense_text")]
        dense_text: Option<PathBuf>,
        /// Write a sparse (CSR) HDF5 matrix here.
        #[arg(long)]
        sparse: Option<PathBuf>,
        /// Distance metric. diamond_*: f=fast, d=default, mids=mid-sensitive,
        /// s=sensitive, mors=more-sensitive, vs=very-sensitive, us=ultra-sensitive
        /// (needs the external diamond binary). hmmer: phmmer/hmmsearch.
        /// hmmer_compare: HMM-vs-HMM (auto for two HMM files).
        #[arg(long, default_value = "diamond_us", value_parser = [
            "diamond_f", "diamond_d", "diamond_mids", "diamond_s", "diamond_mors",
            "diamond_vs", "diamond_us", "hmmer", "hmmer_compare",
        ])]
        algorithm: String,
        /// Matrix value type.
        #[arg(long, default_value = "score")]
        mode: String,
        /// Round raw scores <= this down to zero.
        #[arg(long, default_value_t = 0.0)]
        lb: f64,
        /// Keep at most this many non-zero entries per input row.
        #[arg(short = 'k', long)]
        k: Option<usize>,
        /// Prune to MST + OR-symmetric kNN edges (integer > 1; requires reference
        /// == input and --mode score/bool/efi_score). When set, -k is ignored.
        #[arg(long = "mst_knn", value_parser = parse_mst_knn)]
        mst_knn: Option<usize>,
        /// Worker threads (0 = all cores).
        #[arg(long, default_value_t = 0)]
        cpu: usize,
    },
    /// Pairwise contig similarity (Jaccard + Adjacency) from annotated GenBank.
    CompareContigs {
        /// Annotated GenBank file(s).
        #[arg(short, long, num_args = 1..)]
        input: Vec<PathBuf>,
        /// Write a dense HDF5 matrix here.
        #[arg(long)]
        dense: Option<PathBuf>,
        /// Write a dense TSV matrix here.
        #[arg(long = "dense_text")]
        dense_text: Option<PathBuf>,
        /// Write a sparse (CSR) HDF5 matrix here.
        #[arg(long)]
        sparse: Option<PathBuf>,
        /// Jaccard-index weight.
        #[arg(long, default_value_t = 0.5)]
        ji: f64,
        /// Adjacency-index weight.
        #[arg(long, default_value_t = 0.5)]
        ai: f64,
        /// Restrict to domains from these databases.
        #[arg(long, num_args = 1..)]
        databases: Vec<String>,
        /// Keep at most this many non-zero entries per row (incl. the diagonal).
        #[arg(short = 'k', long)]
        k: Option<usize>,
        /// Zero out final scores <= this.
        #[arg(long, default_value_t = 0.0)]
        lb: f64,
        /// Prune to MST + OR-symmetric kNN edges (integer > 1). When set, -k is ignored.
        #[arg(long = "mst_knn", value_parser = parse_mst_knn)]
        mst_knn: Option<usize>,
        /// Worker threads (0 = all cores).
        #[arg(long, default_value_t = 0)]
        cpu: usize,
    },
    /// Align HMM profiles against reference HMM profiles (hhsearch-like).
    HmmerCompare {
        /// Query `.hmm` file(s).
        #[arg(short, long, num_args = 1..)]
        input: Vec<PathBuf>,
        /// Reference `.hmm` file(s).
        #[arg(short, long, num_args = 1..)]
        reference: Vec<PathBuf>,
        /// Output TSV (default: stdout).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Report hits with score >= this.
        #[arg(long = "score_cutoff", default_value_t = 0.0)]
        score_cutoff: f64,
        /// Keep at most this many top hits per query.
        #[arg(short = 'k', long)]
        k: Option<usize>,
        /// Emit rendered alignments under each hit.
        #[arg(long)]
        alignments: bool,
        /// Worker threads (0 = all cores).
        #[arg(long, default_value_t = 0)]
        cpu: usize,
    },
}

/// Parse `--mst_knn`: an integer greater than 1 (mirrors
/// `transform_matrix._mst_knn_arg`).
fn parse_mst_knn(value: &str) -> std::result::Result<usize, String> {
    let v: i64 = value.parse().map_err(|_| format!("`{value}` is not an integer"))?;
    if v <= 1 {
        return Err("--mst_knn must be an integer greater than 1".to_string());
    }
    Ok(v as usize)
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
        Command::DomainSearch { input, references, output, evalue, z, add_annotations, max_hits, batch_size, cpu, whole_contig, cds_range, cds_up, cds_down, kb_range, kb_up, kb_down, strand, keep_direction, max_region_overlap, translate } => {
            let ref_files: Vec<String> = references
                .iter()
                .map(|p| p.to_str().map(str::to_string).context("non-UTF8 path"))
                .collect::<Result<_>>()?;
            let dbs = domainate::read_hmm_references(&ref_files).context("reading HMM references")?;
            let input_str = input.to_str().context("non-UTF8 path")?.to_string();
            let range = domainator_core::tools::neighborhood::validate_range(
                whole_contig, kb_range, kb_up, kb_down, cds_range, cds_up, cds_down,
            )
            .map_err(|e| anyhow::anyhow!(e))?;
            if translate && !matches!(range, domainator_core::tools::neighborhood::Range::None) {
                anyhow::bail!("--translate is incompatible with neighborhood extraction (--cds_range/--kb_range/--whole_contig)");
            }
            let strand = match strand.as_deref() {
                Some("f") => Some(1),
                Some("r") => Some(-1),
                Some(other) => anyhow::bail!("--strand must be 'f' or 'r', got {other:?}"),
                None => None,
            };
            let opts = domain_search::DomainSearchOptions {
                z, evalue, add_annotations, max_hits, batch_size, cpu,
                range,
                normalize_direction: !keep_direction,
                strand,
                max_region_overlap,
                translate,
                ..Default::default()
            };
            let hits = domain_search::domain_search(&[input_str], &dbs, &opts)
                .context("running domain_search")?;
            let out = output.to_str().context("non-UTF8 path")?;
            write_genbank(out, &hits).context("writing GenBank")?;
            println!("wrote {} hit contigs to {}", hits.len(), output.display());
        }
        Command::SeqDist { input, reference, dense, dense_text, sparse, algorithm, mode, lb, k, mst_knn, cpu } => {
            if dense.is_none() && dense_text.is_none() && sparse.is_none() {
                anyhow::bail!("no output specified: pass at least one of --dense, --dense_text, --sparse");
            }
            let input_str = input.to_str().context("non-UTF8 path")?;
            let input_kind = detect_kind(&input)
                .with_context(|| format!("could not detect input kind: {}", input.display()))?;
            let ref_str = reference.as_deref().map(|p| p.to_str().context("non-UTF8 path")).transpose()?;
            let ref_kind = match &reference {
                Some(p) => Some(
                    detect_kind(p).with_context(|| format!("could not detect reference kind: {}", p.display()))?,
                ),
                None => None,
            };
            let opts = SeqDistOptions { algorithm, mode, lb, k, mst_knn, cpu };
            let matrix = seq_dist(input_str, input_kind, ref_str, ref_kind, &opts)
                .context("running seq_dist")?;
            if let Some(p) = &dense {
                hdf5_io::write_dense(p.to_str().context("non-UTF8 path")?, &matrix)?;
            }
            if let Some(p) = &dense_text {
                text::write_dense_text(p.to_str().context("non-UTF8 path")?, &matrix)?;
            }
            if let Some(p) = &sparse {
                hdf5_io::write_sparse(p.to_str().context("non-UTF8 path")?, &matrix)?;
            }
            println!("wrote {}x{} matrix", matrix.nrows(), matrix.ncols());
        }
        Command::CompareContigs { input, dense, dense_text, sparse, ji, ai, databases, k, lb, mst_knn, cpu } => {
            if dense.is_none() && dense_text.is_none() && sparse.is_none() {
                anyhow::bail!("no output specified: pass at least one of --dense, --dense_text, --sparse");
            }
            if ji == 0.0 && ai == 0.0 {
                anyhow::bail!("specify a non-zero weight for at least one metric (--ji / --ai)");
            }
            let genbanks: Vec<String> = input
                .iter()
                .map(|p| p.to_str().map(str::to_string).context("non-UTF8 path"))
                .collect::<Result<_>>()?;
            let opts = CompareContigsOptions {
                ji_weight: ji,
                ai_weight: ai,
                databases: if databases.is_empty() { None } else { Some(databases.into_iter().collect()) },
                k,
                lb,
                mst_knn,
                cpu,
            };
            let matrix = compare_contigs(&genbanks, &opts).context("running compare_contigs")?;
            if let Some(p) = &dense {
                hdf5_io::write_dense(p.to_str().context("non-UTF8 path")?, &matrix)?;
            }
            if let Some(p) = &dense_text {
                text::write_dense_text(p.to_str().context("non-UTF8 path")?, &matrix)?;
            }
            if let Some(p) = &sparse {
                hdf5_io::write_sparse(p.to_str().context("non-UTF8 path")?, &matrix)?;
            }
            println!("wrote {}x{} matrix", matrix.nrows(), matrix.ncols());
        }
        Command::HmmerCompare { input, reference, output, score_cutoff, k, alignments, cpu } => {
            let q_files: Vec<String> = input
                .iter()
                .map(|p| p.to_str().map(str::to_string).context("non-UTF8 path"))
                .collect::<Result<_>>()?;
            let r_files: Vec<String> = reference
                .iter()
                .map(|p| p.to_str().map(str::to_string).context("non-UTF8 path"))
                .collect::<Result<_>>()?;
            let opts = HmmerCompareOptions { score_cutoff, alignments, k, cpu };
            match output {
                Some(p) => {
                    let mut f = std::io::BufWriter::new(std::fs::File::create(&p)?);
                    hmmer_compare(&q_files, &r_files, &opts, &mut f).context("running hmmer_compare")?;
                    f.flush()?;
                }
                None => {
                    let stdout = std::io::stdout();
                    let mut h = stdout.lock();
                    hmmer_compare(&q_files, &r_files, &opts, &mut h).context("running hmmer_compare")?;
                }
            }
        }
    }
    Ok(())
}
