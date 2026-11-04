use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use domainator_config::args::DbDownloadArgs;
use domainator_core::parallel::resolve_cpu;
use domainator_core::seq::taxonomy::default_ncbi_taxonomy_path;
use domainator_core::seq::taxonomy_download::open_taxonomy;
use domainator_core::tools::db_download::{
    db_download, is_fasta, needs_taxonomy, parse_date, Backend, DatasetsOptions, DbDownloadOptions,
    DirectOptions,
};
use domainator_core::tools::gene_call::{GeneCall, GeneCallOptions};
use domainator_core::tools::select_common::parse_name_list;

use super::cpu_threads;
use crate::output::{is_stdout, Output};

pub fn run(args: &DbDownloadArgs) -> Result<()> {
    let db = args.db.to_lowercase();
    let fasta = is_fasta(&db);
    let (output, other, flag) = if fasta {
        (&args.fasta_out, &args.genbank_out, "--fasta_out")
    } else {
        (&args.genbank_out, &args.fasta_out, "-o/--genbank_out")
    };
    if other.is_some() {
        bail!(
            "--db {db} is downloaded as {}, so write it with {flag}.",
            if fasta { "FASTA" } else { "GenBank" }
        );
    }
    let Some(output) = output.as_deref() else {
        bail!(
            "--db {db} is downloaded as {}, so write it with {flag}.",
            if fasta { "FASTA" } else { "GenBank" }
        );
    };

    // --success_log_out is rewritten on every run, so pointing it at the exclusion log
    // would erase the list of genomes already downloaded. Checked before the output is
    // touched.
    if let (Some(log), Some(exclude)) = (&args.success_log_out, &args.exclude_accessions_file) {
        if std::path::absolute(log)? == std::path::absolute(exclude)? {
            bail!(
                "--success_log_out and --exclude_accessions_file must be different files: \
                 --success_log_out is truncated on each run, which would erase the exclusion log. \
                 Write new successes to a separate file (e.g. success_log_new.txt) and merge it \
                 into the exclusion log after a clean run."
            );
        }
    }
    if args.append && (is_stdout(output) || !Path::new(output).is_file()) {
        let shown = if is_stdout(output) {
            "/dev/stdout"
        } else {
            output
        };
        bail!("{shown} is not a file, so you cannot append to it.");
    }
    let mut out = Output::in_place(output, args.append)?;

    let exclude_accessions = parse_name_list(
        args.exclude_accessions.as_deref().unwrap_or_default(),
        args.exclude_accessions_file.as_deref(),
    )
    .context("reading the accessions to exclude")?;
    let taxonomy_expr = args.taxonomy_expr.clone().filter(|e| !e.is_empty());
    if taxonomy_expr.is_some() && (args.include_taxids.is_some() || args.exclude_taxids.is_some()) {
        bail!("--taxonomy_expr is mutually exclusive with --include_taxids/--exclude_taxids.");
    }
    let filtering = taxonomy_expr.is_some()
        || args.include_taxids.as_ref().is_some_and(|t| !t.is_empty())
        || args.exclude_taxids.as_ref().is_some_and(|t| !t.is_empty());
    let taxonomy = if needs_taxonomy(&db, filtering) {
        let path = args
            .ncbi_taxonomy_path
            .clone()
            .unwrap_or_else(default_ncbi_taxonomy_path);
        // --no_taxonomy_update beats --taxonomy_update; neither leaves the check on.
        let update = !args.no_taxonomy_update;
        Some(open_taxonomy(Path::new(&path), update).context("loading NCBI taxonomy")?)
    } else {
        None
    };
    let date = |text: &Option<String>, flag: &str| -> Result<_> {
        text.as_deref()
            .map(|t| {
                parse_date(t, '-').with_context(|| {
                    format!("--{flag} {t:?} is not a date in the format YYYY-MM-DD")
                })
            })
            .transpose()
    };
    let max_records = args
        .max_records
        .map(|n| usize::try_from(n.max(0)).unwrap_or(0));
    let options = DbDownloadOptions {
        db: db.clone(),
        include_taxids: args
            .include_taxids
            .as_ref()
            .map(|t| t.iter().copied().collect()),
        exclude_taxids: args
            .exclude_taxids
            .as_ref()
            .map(|t| t.iter().copied().collect()),
        taxonomy_expr,
        partial: crate::parse_choice(&args.partial)?,
        uniprot_dir: args.uniprot_dir.clone(),
        max_records,
        ncbi_summary: args.ncbi_summary.clone(),
        exclude_accessions: exclude_accessions.map(|s| s.into_iter().collect::<HashSet<_>>()),
        before: date(&args.before, "before")?,
        after: date(&args.after, "after")?,
    };

    let cpus = resolve_cpu(cpu_threads(args.cpu)?);
    let backend = if db.starts_with("ncbi_") {
        let gene_call = args
            .gene_call
            .as_deref()
            .map(|scope| -> Result<GeneCall> {
                Ok(GeneCall {
                    scope: crate::parse_choice(scope)?,
                    // Genomes are processed in parallel already, one Orphos run each.
                    options: GeneCallOptions {
                        cpu: 1,
                        ..GeneCallOptions::default()
                    },
                })
            })
            .transpose()?;
        let backend_name = args.download_backend.to_lowercase();
        Some(if backend_name == "datasets" {
            let Some(workdir) = &args.datasets_workdir else {
                bail!(
                    "--datasets_workdir is required when --download_backend is 'datasets'. \
                     Provide one, or pass --download_backend direct."
                );
            };
            Backend::Datasets(DatasetsOptions {
                gene_call,
                max_records,
                cpu: cpus,
                datasets_path: args.datasets_path.clone(),
                workdir: PathBuf::from(workdir),
                include: args.datasets_include.clone(),
                max_workers: args.datasets_max_workers,
                api_key: args
                    .ncbi_api_key
                    .clone()
                    .or_else(|| std::env::var("NCBI_API_KEY").ok())
                    .filter(|k| !k.is_empty()),
                gzip: args.datasets_gzip,
            })
        } else {
            let email = args
                .email
                .clone()
                .or_else(|| std::env::var("NCBI_EMAIL").ok())
                .filter(|e| !e.is_empty());
            let version = domainator_core::version::VERSION;
            Backend::Direct(DirectOptions {
                gene_call,
                max_records,
                workers: if args.download_workers <= 0 {
                    cpus
                } else {
                    usize::try_from(args.download_workers).unwrap_or(cpus)
                },
                user_agent: Some(match email {
                    Some(email) => format!("domainatoredux/{version} ({email})"),
                    None => format!("domainatoredux/{version}"),
                }),
                tmp_dir: None,
            })
        })
    } else {
        None
    };

    let mut success_log = args
        .success_log_out
        .as_deref()
        .map(|path| -> Result<std::fs::File> {
            std::fs::File::create(path).with_context(|| format!("creating {path}"))
        })
        .transpose()?;
    db_download(
        &options,
        taxonomy.as_ref(),
        backend.as_ref(),
        &mut out,
        success_log.as_mut().map(|f| f as &mut dyn Write),
    )
    .with_context(|| format!("downloading {db}"))?;
    out.finish()
}
