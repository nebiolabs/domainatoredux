//! `domainatoredux <tool> [opts]` — the CLI for the Domainator Rust rewrite, as a
//! library so that more than one front end can run it.
//!
//! [`run`] takes a whole command line and does exactly what the `domainator`
//! binary does with it (`src/main.rs` is nothing else). The GUI calls it too: its
//! Tools-menu jobs run in a copy of the GUI executable started in a hidden worker
//! mode (`apps/domainator-gui/src-tauri/src/worker.rs`), which hands the worker's
//! arguments straight to [`run`], so a GUI run and a CLI run take the same code.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};

use domainator_config as config;

mod commands;
pub mod output;

use domainator_core::io::genbank::{read_genbank, write_genbank};
use domainator_core::io::{detect_filetype, fasta, FileType};
use domainator_core::seq::taxonomy::{default_ncbi_taxonomy_path, NCBITaxonomy};
use domainator_core::seq::SeqRecord;
use domainator_core::tools;
use domainator_core::tools::gene_call;
use domainator_core::tools::select_common::parse_name_list;

#[derive(Parser)]
#[command(name = "domainatoredux", version, long_version = domainator_core::version::LONG_VERSION, about = "Domainatoredux: protein-domain / SSN analysis (a Rust port of domainator)")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
#[command(rename_all = "snake_case")]
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
    Domainate(config::args::DomainateArgs),
    /// Search references against a (possibly sharded/BGZF/indexed) database.
    DomainSearch(config::args::DomainSearchArgs),
    /// Remove redundant sequences (hash exact-dedup, or cd-hit/diamond/usearch clustering).
    DeduplicateGenbank(config::args::DeduplicateGenbankArgs),
    /// Convert GenBank record(s) to FASTA.
    GenbankToFasta(config::args::GenbankToFastaArgs),
    /// Clean/filter sequences: newick-safe names, dedup names, strip/filter residues.
    CleanSequences(config::args::CleanSequencesArgs),
    /// Call genes (Prodigal-style CDS features) on nucleotide contigs with the
    /// external Orphos binary.
    GeneCall(config::args::GeneCallArgs),
    SortContigs(config::args::SortContigsArgs),
    /// Extract contig regions using explicit 1-based coordinate ranges from a table.
    SelectByCoord(config::args::SelectByCoordArgs),
    /// Filter Domainator/Domain_Search annotations by evalue, overlap, database, name.
    #[command(allow_negative_numbers = true)]
    FilterDomains(config::args::FilterDomainsArgs),
    /// Assign hex colors to domain and/or CDS features (Color qualifier).
    ColorGenbank(config::args::ColorGenbankArgs),
    /// Generate an SVG legend from a color table.
    ColorTableToLegend(config::args::ColorTableToLegendArgs),
    /// Trim contig ends by kb, CDS count, or by stripping end CDSs by domain.
    TrimContigs(config::args::TrimContigsArgs),
    /// Extract domain-annotated regions from annotated contigs.
    #[command(allow_negative_numbers = true)]
    ExtractDomains(config::args::ExtractDomainsArgs),
    /// Extract peptide sequences from selected CDSs.
    ExtractPeptides(config::args::ExtractPeptidesArgs),
    /// Extract regions of proteins not containing domain annotations.
    #[command(allow_negative_numbers = true)]
    ExtractUnannotated(config::args::ExtractUnannotatedArgs),
    /// Compute a similarity matrix between input and reference sequences/profiles.
    SeqDist(config::args::SeqDistArgs),
    /// Pairwise contig similarity (Jaccard + Adjacency) from annotated GenBank.
    CompareContigs(config::args::CompareContigsArgs),
    /// Normalize / convert / sparsify a score matrix (dense<->sparse, --mode, --mst_knn).
    TransformMatrix(config::args::TransformMatrixArgs),
    /// Build a Cytoscape XGMML sequence-similarity network from a score matrix.
    BuildSsn(config::args::BuildSsnArgs),
    /// Build a compact `.dsnv` bundle (+ standalone HTML) for the interactive SSN viewer.
    BuildSsnViewer(config::args::BuildSsnViewerArgs),
    /// Summarize a similarity matrix (stats, cluster-vs-threshold, merge events) as text or JSON.
    MatrixReport(config::args::MatrixReportArgs),
    /// Dataset-level statistics: contig/CDS counts, domain frequency, taxonomy.
    SummaryReport(config::args::SummaryReportArgs),
    /// Navigate a `.dsnv` bundle: clusters, cluster contents, and metadata, as JSON.
    SsnNavigator(config::args::SsnNavigatorArgs),
    /// Print a JSON description of every command and its options (the tool manifest).
    Describe {
        /// Describe only these commands.
        #[arg(long, num_args = 1..)]
        tool: Vec<String>,
        /// Indent the JSON by this many spaces.
        #[arg(long)]
        indent: Option<usize>,
    },
    /// Print a shell completion script.
    Completion {
        /// The shell to complete for.
        shell: clap_complete::Shell,
    },
    EnumReport(config::args::EnumReportArgs),
    /// Extract contig regions around selected CDSs (by name/domain/expr/hits).
    SelectByCds(config::args::SelectByCdsArgs),
    /// Select a subset of contigs from a GenBank/FASTA file by various criteria.
    SelectByContig(config::args::SelectByContigArgs),
    /// Align HMM profiles against reference HMM profiles (hhsearch-like).
    HmmerCompare(config::args::HmmerCompareArgs),
    HmmerReport(config::args::HmmerReportArgs),
    /// Filter an HMM file by text matches on profile NAME/DESC/ACC fields.
    HmmerSelect(config::args::HmmerSelectArgs),
    /// Build an HMM profile from a (Stockholm) multiple sequence alignment.
    HmmerBuild(config::args::HmmerBuildArgs),
    /// Filter query HMM profiles by best score against reference HMM profiles.
    HmmerSearch(config::args::HmmerSearchArgs),
    /// Shard / BGZF-compress / .didx-index a domain_search database.
    FormatDb(config::args::FormatDbArgs),
}

fn load(input: &Path) -> Result<Vec<SeqRecord>> {
    let path = input.to_str().context("non-UTF8 path")?;
    match detect_filetype(input) {
        Some(FileType::Genbank) => read_genbank(path).with_context(|| format!("reading {path}")),
        Some(FileType::Fasta) => {
            fasta::read_fasta(path, Some("protein")).with_context(|| format!("reading {path}"))
        }
        None => anyhow::bail!(
            "could not detect file type from extension: {}",
            input.display()
        ),
    }
}

/// Filter `records` in place to those whose LOCUS name (Python's `contig.id`) is
/// in the `--contigs`/`--contigs_file` set. A no-op when both are empty.
pub(crate) fn filter_contigs(
    records: &mut Vec<SeqRecord>,
    contigs: &[String],
    contigs_file: Option<&str>,
) -> Result<()> {
    let set = parse_name_list(contigs, contigs_file)?;
    if let Some(set) = set {
        records.retain(|r| set.contains(&r.id));
    }
    Ok(())
}

/// Read a color table: a TSV with no header mapping a value to a hex color, which is
/// normalized to `#RRGGBB` (upstream `colors.read_color_table`).
pub(crate) fn read_color_table(path: &str) -> Result<std::collections::HashMap<String, String>> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading color table {path}"))?;
    let table = tools::color_genbank::read_color_table(&text).map_err(anyhow::Error::msg)?;
    Ok(table.into_iter().collect())
}

/// `output_guardrails.max_output_gb_to_bytes` (the core port): negative or
/// non-finite is an error, `0` disables (returns `None`).
pub(crate) fn max_output_gb_to_bytes(max_gb: f64) -> Result<Option<u64>> {
    tools::output_guardrails::max_output_gb_to_bytes(max_gb).map_err(anyhow::Error::msg)
}

/// Load an `NCBITaxonomy` from `--ncbi_taxonomy_path` (only when a taxonomy
/// filter/column is in use).
fn load_taxonomy(path: &str) -> Result<NCBITaxonomy> {
    NCBITaxonomy::load(Path::new(path)).context("loading NCBI taxonomy")
}

/// Load a sequence file, treating FASTA as `fasta_type` (protein/nucleotide).
pub(crate) fn load_typed(input: &Path, fasta_type: &str) -> Result<Vec<SeqRecord>> {
    let path = input.to_str().context("non-UTF8 path")?;
    match detect_filetype(input) {
        Some(FileType::Genbank) => read_genbank(path).context("reading GenBank"),
        Some(FileType::Fasta) => fasta::read_fasta(path, Some(fasta_type)).context("reading FASTA"),
        None => anyhow::bail!(
            "could not detect file type from extension: {}",
            input.display()
        ),
    }
}

/// Parse a `FromStr` choice (`--mode`, `--scope`), keeping its own error message.
fn parse_choice<T: std::str::FromStr<Err = String>>(s: &str) -> Result<T> {
    s.parse().map_err(|e: String| anyhow::anyhow!(e))
}

/// The `--gene_call` request for `domainate`/`domain_search`: Orphos in meta mode
/// (upstream's only mode) with its default genetic code, or `None` when unset.
pub(crate) fn gene_call_request(
    scope: Option<&str>,
    cpu: usize,
) -> Result<Option<gene_call::GeneCall>> {
    scope
        .map(|scope| {
            Ok(gene_call::GeneCall {
                scope: parse_choice(scope)?,
                options: gene_call::GeneCallOptions {
                    cpu,
                    ..Default::default()
                },
            })
        })
        .transpose()
}

/// Compile `--include_taxids`/`--exclude_taxids`/`--taxonomy_expr` into an
/// allowed-taxid set, loading the taxonomy from `path` when a filter is active.
/// Returns `None` when no taxonomy filtering was requested.
pub(crate) fn compile_allowed_taxids(
    include: &[i64],
    exclude: &[i64],
    expr: Option<&str>,
    path: Option<&str>,
) -> Result<Option<std::collections::HashSet<i64>>> {
    let has_expr = expr.is_some_and(|e| !e.trim().is_empty());
    if include.is_empty() && exclude.is_empty() && !has_expr {
        return Ok(None);
    }
    if has_expr && (!include.is_empty() || !exclude.is_empty()) {
        anyhow::bail!(
            "--taxonomy_expr is mutually exclusive with --include_taxids/--exclude_taxids."
        );
    }
    let tax = load_taxonomy(
        &path
            .map(str::to_string)
            .unwrap_or_else(default_ncbi_taxonomy_path),
    )?;
    let allowed = if has_expr {
        tax.compile_taxonomy_expression(expr.unwrap())
    } else {
        use std::collections::HashSet;
        let inc: HashSet<i64> = include.iter().copied().collect();
        let exc: HashSet<i64> = exclude.iter().copied().collect();
        tax.compile_taxonomy_filter(
            if inc.is_empty() { None } else { Some(&inc) },
            if exc.is_empty() { None } else { Some(&exc) },
        )
    };
    Ok(allowed)
}

/// Run one `domainatoredux` command line. `args` includes the program name first, as
/// `std::env::args_os()` does.
///
/// A command line clap rejects, and `--help`/`--version`, come back as an `Err`
/// holding the [`clap::Error`], unprinted, so the caller decides how to show it:
/// the binary calls [`clap::Error::exit`] (usage on stderr, exit 2; help on
/// stdout, exit 0), exactly as `Cli::parse()` did. Use [`clap_error`] to find it.
pub fn run<I, T>(args: I) -> Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let mut argv: Vec<std::ffi::OsString> = args.into_iter().map(Into::into).collect();
    let mut cmd = command();
    let tool = argv
        .get(1)
        .and_then(|a| a.to_str())
        .and_then(config::registry::tool_info)
        .filter(|t| t.migrated);
    if let Some(info) = tool {
        let sub = cmd
            .find_subcommand(info.name)
            .context("registry tool without a subcommand")?;
        let rest = config::config_file::expand(sub, &argv[2..])?;
        argv.truncate(2);
        argv.extend(rest);
        if argv.iter().any(|a| a == "--print_config") {
            // A config being printed need not be complete, as upstream's need not.
            cmd = cmd.mut_subcommand(info.name, |c| c.mut_args(|a| a.required(false)));
        }
    }
    let matches = cmd.clone().try_get_matches_from(&argv)?;
    if let (Some(info), Some((name, sub_m))) = (tool, matches.subcommand()) {
        let sub = cmd
            .find_subcommand(name)
            .context("parsed an unknown subcommand")?;
        if sub_m.get_flag("print_config") {
            print!("{}", config::config_file::print_config(sub, sub_m, info)?);
            return Ok(());
        }
        check_io(sub, sub_m, info)?;
    }
    dispatch(Cli::from_arg_matches(&matches)?)
}

/// The command line, with each tool's one-line summary from the registry.
pub fn command() -> clap::Command {
    Cli::command().mut_subcommands(|c| match config::registry::tool_info(c.get_name()) {
        Some(info) => c.about(info.summary),
        None => c,
    })
}

/// Upstream's `check_outputs` and `reject_stdin`: at least one of the required
/// outputs, at most one output on standard output, `-` only where it is allowed.
fn check_io(
    sub: &clap::Command,
    m: &clap::ArgMatches,
    info: &config::registry::ToolInfo,
) -> Result<()> {
    let value = |id: &str| -> Vec<String> {
        m.try_get_raw(id)
            .ok()
            .flatten()
            .map(|v| v.map(|s| s.to_string_lossy().into_owned()).collect())
            .unwrap_or_default()
    };
    let flag = |id: &str| {
        sub.get_arguments()
            .find(|a| a.get_id() == id)
            .and_then(|a| a.get_long())
            .map_or_else(|| id.to_string(), |l| format!("--{l}"))
    };
    let usage = |msg: String| -> anyhow::Error {
        let mut sub = sub
            .clone()
            .bin_name(format!("domainatoredux {}", info.name));
        sub.error(clap::error::ErrorKind::MissingRequiredArgument, msg)
            .into()
    };
    let outputs: Vec<&clap::Arg> = sub
        .get_arguments()
        .filter(|a| a.get_id().as_str().ends_with("_out"))
        .collect();
    if let Some(required) = info.required_outputs {
        if required.iter().all(|id| value(id).is_empty()) {
            let names: Vec<String> = required.iter().map(|id| flag(id)).collect();
            return Err(usage(format!(
                "No output specified. Give at least one of: {}",
                names.join(", ")
            )));
        }
    }
    let on_stdout: Vec<String> = outputs
        .iter()
        .map(|a| a.get_id().as_str())
        .filter(|id| value(id).iter().any(|v| output::is_stdout(v)))
        .map(flag)
        .collect();
    for id in info.no_stdout {
        if value(id).iter().any(|v| output::is_stdout(v)) {
            return Err(usage(format!(
                "{} cannot be written to standard output. Give a file name.",
                flag(id)
            )));
        }
    }
    if on_stdout.len() > 1 {
        return Err(usage(format!(
            "Only one output can be written to standard output, but got: {}",
            on_stdout.join(", ")
        )));
    }
    if !info.stdin && value("input").iter().any(|v| output::is_stdin(v)) {
        return Err(usage(format!(
            "This command cannot read standard input, because {}. Give a file instead.",
            info.stdin_reason
        )));
    }
    Ok(())
}

/// Print an error from [`run`] the way the binary does, and return the exit code: a
/// usage error as clap prints it (exit 2, or 0 for `--help`), anything else as one
/// line, `domainatoredux <tool>: error: <message: cause: ...>` (exit 1).
pub fn report_error(e: &anyhow::Error, argv: &[std::ffi::OsString]) -> i32 {
    if let Some(clap_err) = clap_error(e) {
        let _ = clap_err.print();
        return clap_err.exit_code();
    }
    let prog = match argv.get(1).and_then(|a| a.to_str()) {
        Some(tool) if command().find_subcommand(tool).is_some() => {
            format!("domainatoredux {tool}")
        }
        _ => "domainatoredux".to_string(),
    };
    eprintln!("{prog}: error: {e:#}");
    1
}

/// What the binary does: [`run`], then [`report_error`] on failure.
pub fn main_with<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let argv: Vec<std::ffi::OsString> = args.into_iter().map(Into::into).collect();
    match run(argv.clone()) {
        Ok(()) => 0,
        Err(e) => report_error(&e, &argv),
    }
}

/// The [`clap::Error`] inside an error from [`run`], when clap is where it came
/// from (a bad command line, or `--help`/`--version`).
pub fn clap_error(e: &anyhow::Error) -> Option<&clap::Error> {
    e.downcast_ref::<clap::Error>()
}

fn dispatch(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Inspect { input, verbose } => {
            let records = load(&input)?;
            println!("{} records", records.len());
            if verbose {
                for r in &records {
                    println!(
                        "  id={} name={} mol={:?} len={} features={}",
                        r.id,
                        r.name,
                        r.molecule_type,
                        r.len(),
                        r.features.len()
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
        Command::Domainate(args) => commands::domainate::run(&args)?,
        Command::DomainSearch(args) => commands::domain_search::run(&args)?,
        Command::DeduplicateGenbank(args) => commands::deduplicate_genbank::run(&args)?,
        Command::GenbankToFasta(args) => commands::genbank_to_fasta::run(&args)?,
        Command::CleanSequences(args) => commands::clean_sequences::run(&args)?,
        Command::GeneCall(args) => commands::gene_call::run(&args)?,
        Command::SortContigs(args) => commands::sort_contigs::run(&args)?,
        Command::SelectByCoord(args) => commands::select_by_coord::run(&args)?,
        Command::FilterDomains(args) => commands::filter_domains::run(&args)?,
        Command::ColorGenbank(args) => commands::color_genbank::run(&args)?,
        Command::ColorTableToLegend(args) => commands::color_table_to_legend::run(&args)?,
        Command::TrimContigs(args) => commands::trim_contigs::run(&args)?,
        Command::ExtractDomains(args) => commands::extract_domains::run(&args)?,
        Command::ExtractPeptides(args) => commands::extract_peptides::run(&args)?,
        Command::ExtractUnannotated(args) => commands::extract_unannotated::run(&args)?,
        Command::SeqDist(args) => commands::seq_dist::run(&args)?,
        Command::CompareContigs(args) => commands::compare_contigs::run(&args)?,
        Command::TransformMatrix(args) => commands::transform_matrix::run(&args)?,
        Command::BuildSsn(args) => commands::build_ssn::run(&args)?,
        Command::BuildSsnViewer(args) => commands::build_ssn_viewer::run(&args)?,
        Command::MatrixReport(args) => commands::matrix_report::run(&args)?,
        Command::SummaryReport(args) => commands::summary_report::run(&args)?,
        Command::SsnNavigator(args) => commands::ssn_navigator::run(&args)?,
        Command::Describe { tool, indent } => {
            for t in &tool {
                if !config::registry::tool_info(t).is_some_and(|i| i.in_manifest) {
                    return Err(commands::usage_error(format!(
                        "describe: unknown tool '{t}'; `domainatoredux --help` lists the tools"
                    )));
                }
            }
            let only = (!tool.is_empty()).then_some(tool.as_slice());
            let doc = config::manifest::manifest(&command(), only);
            let mut out = std::io::stdout().lock();
            match indent {
                Some(n) => {
                    let pad = vec![b' '; n];
                    let fmt = serde_json::ser::PrettyFormatter::with_indent(&pad);
                    let mut ser = serde_json::Serializer::with_formatter(&mut out, fmt);
                    serde::Serialize::serialize(&doc, &mut ser)?;
                }
                None => serde_json::to_writer(&mut out, &doc)?,
            }
            writeln!(out)?;
        }
        Command::Completion { shell } => {
            clap_complete::generate(
                shell,
                &mut command(),
                "domainatoredux",
                &mut std::io::stdout(),
            );
        }
        Command::EnumReport(args) => commands::enum_report::run(&args)?,
        Command::SelectByCds(args) => commands::select_by_cds::run(&args)?,
        Command::SelectByContig(args) => commands::select_by_contig::run(&args)?,
        Command::HmmerCompare(args) => commands::hmmer_compare::run(&args)?,
        Command::HmmerReport(args) => commands::hmmer_report::run(&args)?,
        Command::HmmerSelect(args) => commands::hmmer_select::run(&args)?,
        Command::HmmerBuild(args) => commands::hmmer_build::run(&args)?,
        Command::HmmerSearch(args) => commands::hmmer_search::run(&args)?,
        Command::FormatDb(args) => commands::format_db::run(&args)?,
    }
    Ok(())
}
