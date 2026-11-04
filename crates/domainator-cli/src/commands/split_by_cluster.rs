use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use domainator_config::args::SplitByClusterArgs;
use domainator_core::io::fasta;
use domainator_core::io::genbank::writer::write_record;
use domainator_core::matrix::read_matrix;
use domainator_core::search::hmm_read::{read_hmm_file, Hmm};
use domainator_core::search::hmm_write::{write_hmm, HmmOutput};
use domainator_core::seq::SeqRecord;
use domainator_core::tools::select_by_cds::{pad_records, Anchor, Region};
use domainator_core::tools::split_by_cluster::{
    center_on_search_hit, check_labels_match, cluster_membership, cm_models, input_kind,
    input_labels, output_extension, plan_output_paths, InputKind, OutputPlan,
};

use crate::output::Output;

pub fn run(args: &SplitByClusterArgs) -> Result<()> {
    let input = Path::new(&args.input);
    let kind = input_kind(input).map_err(|e| anyhow!(e))?;
    if kind == InputKind::Cm && domainator_core::io::is_compressed_path(input) {
        anyhow::bail!(
            "Cannot split '{}': compressed cm files are not supported. Infernal covariance \
             models are split by scanning the file as text, which a compressed file would \
             silently corrupt. Decompress it first.",
            args.input
        );
    }
    let extension = output_extension(input).map_err(|e| anyhow!(e))?;
    if args.pad_on_search_hits && kind != InputKind::Genbank {
        anyhow::bail!("--pad_on_search_hits is only supported for GenBank inputs.");
    }

    let matrix = read_matrix(&args.matrix).with_context(|| format!("reading {}", args.matrix))?;
    let clusters = cluster_membership(&matrix, args.threshold.0).map_err(|e| anyhow!(e))?;

    let input_data = read_input(input, kind)?;
    let labels = input_labels(input_data.labels(), kind).map_err(|e| anyhow!(e))?;
    check_labels_match(&matrix.rows, &labels).map_err(|e| anyhow!(e))?;

    let min_cluster_size = usize::try_from(args.min_cluster_size.0).unwrap_or(usize::MAX);
    let plan = plan_output_paths(
        Path::new(&args.output_dir),
        &extension,
        &clusters.sizes,
        min_cluster_size,
        args.write_small_clusters,
    )
    .map_err(|e| anyhow!(e))?;
    let cluster_of = |label: &str| clusters.of_label[label];

    match input_data {
        InputData::Records(records) if args.pad_on_search_hits => {
            // Each output's records are padded together, so the hits line up per file.
            let mut regions: BTreeMap<&Path, Vec<Region>> = BTreeMap::new();
            for record in &records {
                let region = center_on_search_hit(record).map_err(|e| anyhow!(e))?;
                if let Some(path) = plan.destination(cluster_of(&record.id)) {
                    regions.entry(path).or_default().push(region);
                }
            }
            write_each(&plan, |path, out| {
                let mut group = regions.remove(path).unwrap_or_default();
                pad_records(&mut group, Anchor::Start);
                for region in &group {
                    write_record(out, &region.record)?;
                }
                Ok(())
            })
        }
        InputData::Records(records) => write_each(&plan, |path, out| {
            for record in records_for(&records, &plan, path, &cluster_of) {
                if kind == InputKind::Genbank {
                    write_record(out, record)?;
                } else {
                    fasta::write_fasta_bp(out, std::slice::from_ref(record))?;
                }
            }
            Ok(())
        }),
        InputData::Cm(models) => write_each(&plan, |path, out| {
            for (name, text) in &models {
                if plan.destination(cluster_of(name)) == Some(path) {
                    out.write_all(text.as_bytes())?;
                }
            }
            Ok(())
        }),
        InputData::Hmm(profiles) => write_hmms(&plan, &profiles, &cluster_of),
    }
}

/// The input, read once: its labels are checked against the matrix before anything
/// is written.
enum InputData {
    Records(Vec<SeqRecord>),
    Hmm(Vec<Hmm>),
    Cm(Vec<(String, String)>),
}

impl InputData {
    fn labels(&self) -> Vec<String> {
        match self {
            InputData::Records(records) => records.iter().map(|r| r.id.clone()).collect(),
            InputData::Hmm(profiles) => profiles.iter().map(|p| p.name.clone()).collect(),
            InputData::Cm(models) => models.iter().map(|(name, _)| name.clone()).collect(),
        }
    }
}

fn read_input(input: &Path, kind: InputKind) -> Result<InputData> {
    Ok(match kind {
        InputKind::Genbank | InputKind::Fasta => {
            InputData::Records(crate::load_typed(input, "protein")?)
        }
        InputKind::Hmm => InputData::Hmm(
            read_hmm_file(input).map_err(|e| anyhow!("reading {}: {e}", input.display()))?,
        ),
        InputKind::Cm => {
            let text = std::fs::read_to_string(input)
                .with_context(|| format!("reading {}", input.display()))?;
            InputData::Cm(cm_models(&text).map_err(|e| anyhow!(e))?)
        }
    })
}

/// The records `plan` sends to `path`, in input order.
fn records_for<'a>(
    records: &'a [SeqRecord],
    plan: &'a OutputPlan,
    path: &'a Path,
    cluster_of: &'a impl Fn(&str) -> i64,
) -> impl Iterator<Item = &'a SeqRecord> {
    records
        .iter()
        .filter(move |r| plan.destination(cluster_of(&r.id)) == Some(path))
}

/// Write every planned output with `write`, each through [`Output`], so the files
/// appear only if all of them were written. GenBank and FASTA outputs whose name ends
/// in a compression suffix are BGZF, as upstream writes sequence files.
fn write_each(
    plan: &OutputPlan,
    mut write: impl FnMut(&Path, &mut Output) -> std::io::Result<()>,
) -> Result<()> {
    let mut finished = Vec::new();
    for path in plan.paths() {
        let name = path.to_str().context("non-UTF8 output path")?;
        let mut out = Output::create(name)?;
        write(path, &mut out).with_context(|| format!("writing {name}"))?;
        finished.push(out);
    }
    for out in finished {
        out.finish()?;
    }
    Ok(())
}

/// Write the profiles to their cluster files, plain gzip for a `.gz` name as upstream's
/// `open_writable_hmm_file` writes it. A failed run removes what it wrote.
fn write_hmms(
    plan: &OutputPlan,
    profiles: &[Hmm],
    cluster_of: &impl Fn(&str) -> i64,
) -> Result<()> {
    let result = (|| -> Result<()> {
        for path in plan.paths() {
            let name = path.to_str().context("non-UTF8 output path")?;
            let mut out = HmmOutput::create(name).map_err(|e| anyhow!(e))?;
            for profile in profiles {
                if plan.destination(cluster_of(&profile.name)) == Some(path) {
                    write_hmm(&mut out, profile).with_context(|| format!("writing {name}"))?;
                }
            }
            out.finish().with_context(|| format!("writing {name}"))?;
        }
        Ok(())
    })();
    if result.is_err() {
        for path in plan.paths() {
            domainator_core::io::remove_partial_output(path);
        }
    }
    result
}
