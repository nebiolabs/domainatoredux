//! `split_by_cluster`: split a sequence or profile file into one file per connected
//! component of a similarity matrix. Port of upstream `split_by_cluster.py`.
//!
//! The matrix's entries above the threshold are the graph; its components are
//! numbered by size as `build_ssn --cluster` numbers them. Clusters smaller than the
//! minimum size are skipped or pooled into `small_clusters<ext>`. This module holds
//! the reading, validation and planning; the CLI writes the files.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::matrix::{require_similarity, DataMatrix};
use crate::seq::SeqRecord;
use crate::tools::neighborhood::{Range, Region};
use crate::tools::select_by_cds::{select_by_cds, SelectByCdsOptions};

/// What the input holds, which decides how it is read and written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputKind {
    Genbank,
    Fasta,
    Hmm,
    Cm,
}

/// Upstream `_get_supported_input_type`: the input's kind from its extension (a
/// compression suffix ignored), or an error for anything else.
pub fn input_kind(path: &Path) -> Result<InputKind, String> {
    let kind = match crate::io::detect_filetype(path) {
        Some(crate::io::FileType::Genbank) => Some(InputKind::Genbank),
        Some(crate::io::FileType::Fasta) => Some(InputKind::Fasta),
        None => match format_suffix(path).as_deref() {
            Some("hmm" | "hmmer") => Some(InputKind::Hmm),
            Some("cm") => Some(InputKind::Cm),
            _ => None,
        },
    };
    kind.ok_or_else(|| {
        format!(
            "Unsupported input type for {}. Supported file types are: genbank, fasta, hmm, and cm.",
            path.display()
        )
    })
}

/// The path's lowercase suffixes, as `pathlib`'s `suffixes` splits them.
fn suffixes(path: &Path) -> Vec<String> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = name.trim_start_matches('.');
    name.split('.').skip(1).map(|s| format!(".{s}")).collect()
}

fn is_compression_suffix(suffix: &str) -> bool {
    matches!(
        suffix[1..].to_ascii_lowercase().as_str(),
        "gz" | "bgz" | "bgzf"
    )
}

/// The format suffix, lowercase and without its dot, after any compression suffix.
fn format_suffix(path: &Path) -> Option<String> {
    let mut sfx = suffixes(path);
    if sfx.last().is_some_and(|s| is_compression_suffix(s)) {
        sfx.pop();
    }
    sfx.last().map(|s| s[1..].to_ascii_lowercase())
}

/// Upstream `_get_output_extension`: the suffix every output carries, keeping a
/// compression suffix with the format suffix before it (`x.hmm.gz` gives `1.hmm.gz`),
/// since the outputs are written in the input's format and compression.
pub fn output_extension(path: &Path) -> Result<String, String> {
    let sfx = suffixes(path);
    let extension = match sfx.as_slice() {
        [.., format, last] if is_compression_suffix(last) => format!("{format}{last}"),
        [.., last] => last.clone(),
        [] => String::new(),
    };
    if extension.is_empty() {
        return Err(
            "Input file must have an extension so output files can preserve the input format."
                .into(),
        );
    }
    Ok(extension)
}

/// Upstream `_validate_unique_labels`: `labels`, or an error naming the repeats.
fn unique_labels(labels: Vec<String>, source: &str) -> Result<Vec<String>, String> {
    let mut seen = HashSet::new();
    let mut duplicates: Vec<&str> = labels
        .iter()
        .filter(|l| !seen.insert(l.as_str()))
        .map(String::as_str)
        .collect();
    if duplicates.is_empty() {
        return Ok(labels);
    }
    duplicates.sort_unstable();
    duplicates.dedup();
    Err(format!(
        "{source} contain duplicate labels: {}",
        duplicates.join(", ")
    ))
}

/// Upstream `_iter_cm_blocks`: `(NAME, text)` for each model of an Infernal CM file.
/// A model is its CM block and the filter HMM block stored after it, each ending in
/// `//` and each carrying the model's NAME, so the two stay together.
pub fn cm_models(text: &str) -> Result<Vec<(String, String)>, String> {
    let mut models: Vec<(String, String)> = Vec::new();
    let mut pending: Option<(String, String)> = None; // a CM block, awaiting its filter
    for (name, block) in slash_blocks(text)? {
        if block.trim_start().starts_with("HMMER3") {
            let Some((cm_name, mut cm)) = pending.take() else {
                return Err(format!(
                    "CM file has an HMM filter block ({name}) with no CM block before it."
                ));
            };
            cm.push_str(&block);
            models.push((cm_name, cm));
        } else {
            models.extend(pending.replace((name, block)));
        }
    }
    models.extend(pending);
    Ok(models)
}

/// Upstream `_iter_slash_blocks`: `(NAME, text)` for each `//`-terminated block.
fn slash_blocks(text: &str) -> Result<Vec<(String, String)>, String> {
    let mut blocks = Vec::new();
    let mut lines = String::new();
    let mut name: Option<String> = None;
    for line in text.split_inclusive('\n') {
        lines.push_str(line);
        if line.starts_with("NAME") {
            let mut parts = line.splitn(2, char::is_whitespace);
            parts.next();
            match parts.next().map(str::trim).filter(|n| !n.is_empty()) {
                Some(n) => name = Some(n.to_string()),
                None => {
                    return Err(format!(
                        "Malformed NAME line in CM file: {}",
                        line.trim_end()
                    ))
                }
            }
        }
        if line.trim() == "//" {
            let Some(n) = name.take() else {
                return Err("Encountered a CM record without a NAME field.".into());
            };
            blocks.push((n, std::mem::take(&mut lines)));
        }
    }
    if !lines.is_empty() {
        return Err("CM file ended before a record terminator ('//').".into());
    }
    Ok(blocks)
}

/// Which cluster each matrix label is in, and how big each cluster is.
#[derive(Debug)]
pub struct Clusters {
    /// Cluster number (1 is the largest) by matrix label.
    pub of_label: HashMap<String, i64>,
    /// Size by cluster number.
    pub sizes: BTreeMap<i64, usize>,
}

/// Upstream `_load_cluster_membership`: the components of the matrix entries above
/// `threshold`, numbered by size as `build_ssn --cluster` numbers them.
pub fn cluster_membership(matrix: &DataMatrix, threshold: f64) -> Result<Clusters, String> {
    if !matrix.symmetric_labels() {
        return Err("Input matrix must have symmetric row and column labels.".into());
    }
    require_similarity(&matrix.data_type, "split_by_cluster")?;
    let labels = unique_labels(matrix.rows.clone(), "Matrix rows")?;
    if labels.is_empty() {
        return Err("Input matrix does not contain any labels.".into());
    }
    let clusters = crate::tools::build_ssn::cluster_from_graph(matrix, threshold);
    let mut sizes: BTreeMap<i64, usize> = BTreeMap::new();
    for &c in &clusters {
        *sizes.entry(c).or_default() += 1;
    }
    Ok(Clusters {
        of_label: labels.into_iter().zip(clusters).collect(),
        sizes,
    })
}

/// Upstream `_read_input_labels`' uniqueness check, for labels read by the caller.
pub fn input_labels(labels: Vec<String>, kind: InputKind) -> Result<Vec<String>, String> {
    let source = match kind {
        InputKind::Genbank | InputKind::Fasta => "Input records",
        InputKind::Hmm => "Input HMM profiles",
        InputKind::Cm => "Input CM profiles",
    };
    unique_labels(labels, source)
}

/// Upstream `_validate_label_reconciliation`: the matrix and the input must name
/// exactly the same things.
pub fn check_labels_match(matrix_labels: &[String], input_labels: &[String]) -> Result<(), String> {
    let matrix: HashSet<&str> = matrix_labels.iter().map(String::as_str).collect();
    let input: HashSet<&str> = input_labels.iter().map(String::as_str).collect();
    fn first_ten<'a>(only: &HashSet<&'a str>, other: &HashSet<&'a str>) -> Vec<&'a str> {
        let mut v: Vec<&str> = only.difference(other).copied().collect();
        v.sort_unstable();
        v.truncate(10);
        v
    }
    let matrix_only = first_ten(&matrix, &input);
    let input_only = first_ten(&input, &matrix);
    if matrix_only.is_empty() && input_only.is_empty() {
        return Ok(());
    }
    let mut details = Vec::new();
    if !matrix_only.is_empty() {
        details.push(format!("matrix-only labels: {}", matrix_only.join(", ")));
    }
    if !input_only.is_empty() {
        details.push(format!("input-only labels: {}", input_only.join(", ")));
    }
    Err(format!(
        "Matrix labels and input labels do not match exactly ({}).",
        details.join("; ")
    ))
}

/// Where each cluster is written.
#[derive(Debug, Default)]
pub struct OutputPlan {
    /// A file per cluster of at least the minimum size, by cluster number.
    pub clusters: BTreeMap<i64, PathBuf>,
    /// `small_clusters<ext>`, when asked for and some cluster is below the minimum.
    pub small_clusters: Option<PathBuf>,
}

impl OutputPlan {
    /// The file `cluster` goes to: its own, the small-clusters file, or none.
    pub fn destination(&self, cluster: i64) -> Option<&Path> {
        self.clusters
            .get(&cluster)
            .or(self.small_clusters.as_ref())
            .map(PathBuf::as_path)
    }

    /// Every planned path, the small-clusters file last.
    pub fn paths(&self) -> impl Iterator<Item = &Path> {
        self.clusters
            .values()
            .chain(&self.small_clusters)
            .map(PathBuf::as_path)
    }
}

/// Upstream `_plan_output_paths`: create `output_dir` and name each output in it,
/// refusing to overwrite a file already there.
pub fn plan_output_paths(
    output_dir: &Path,
    extension: &str,
    sizes: &BTreeMap<i64, usize>,
    min_cluster_size: usize,
    write_small_clusters: bool,
) -> Result<OutputPlan, String> {
    if output_dir.exists() && !output_dir.is_dir() {
        return Err(format!(
            "Output path exists and is not a directory: {}",
            output_dir.display()
        ));
    }
    std::fs::create_dir_all(output_dir)
        .map_err(|e| format!("creating {}: {e}", output_dir.display()))?;
    let plan = OutputPlan {
        clusters: sizes
            .iter()
            .filter(|&(_, &size)| size >= min_cluster_size)
            .map(|(&id, _)| (id, output_dir.join(format!("{id}{extension}"))))
            .collect(),
        small_clusters: (write_small_clusters
            && sizes.values().any(|&size| size < min_cluster_size))
        .then(|| output_dir.join(format!("small_clusters{extension}"))),
    };
    if let Some(existing) = plan.paths().find(|p| p.exists()) {
        return Err(format!(
            "Refusing to overwrite existing file: {}",
            existing.display()
        ));
    }
    Ok(plan)
}

/// Upstream `_center_record_on_search_hit`: the whole contig around its first CDS
/// (by stranded start) carrying a `Domain_Search` hit, as `select_by_cds
/// --whole_contig` cuts it, so that `pad_records` can line the hits up.
pub fn center_on_search_hit(record: &SeqRecord) -> Result<Region, String> {
    let opts = SelectByCdsOptions {
        has_search_hit: true,
        range: Range::Whole,
        normalize_direction: true,
        max_region_overlap: 1.0,
        ..SelectByCdsOptions::default()
    };
    select_by_cds(std::slice::from_ref(record), &opts)?
        .into_iter()
        .next()
        .ok_or_else(|| {
            format!(
                "Record {} does not contain any Domain_Search annotations required by \
                 --pad_on_search_hits.",
                record.id
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matrix::Storage;

    #[test]
    fn output_extension_keeps_compression_with_its_format() {
        let ext = |p: &str| output_extension(Path::new(p));
        assert_eq!(ext("dir/clusters.hmm.gz").unwrap(), ".hmm.gz");
        assert_eq!(ext("in.gb.bgz").unwrap(), ".gb.bgz");
        assert_eq!(ext("FeSOD_20.gb").unwrap(), ".gb");
        assert_eq!(ext("a.b.fasta").unwrap(), ".fasta");
        assert!(ext("noext").is_err());
    }

    #[test]
    fn input_kinds_follow_upstreams_extension_table() {
        let kind = |p: &str| input_kind(Path::new(p));
        assert_eq!(kind("x.gbk.gz").unwrap(), InputKind::Genbank);
        assert_eq!(kind("x.faa").unwrap(), InputKind::Fasta);
        assert_eq!(kind("x.hmm.gz").unwrap(), InputKind::Hmm);
        assert_eq!(kind("x.hmmer").unwrap(), InputKind::Hmm);
        assert_eq!(kind("x.cm").unwrap(), InputKind::Cm);
        assert!(kind("x.tsv")
            .unwrap_err()
            .contains("Unsupported input type"));
    }

    /// An Infernal model is a CM block and its filter HMM; both carry the NAME.
    #[test]
    fn a_cm_model_keeps_its_filter_hmm() {
        let model = |name: &str| {
            format!(
                "INFERNAL1/a [1.1.5]\nNAME     {name}\nCLEN 3\n//\nHMMER3/f [3.4]\nNAME  {name}\n//\n"
            )
        };
        let text = model("CopA") + &model("CopB");
        let models = cm_models(&text).unwrap();
        assert_eq!(
            models.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
            ["CopA", "CopB"]
        );
        assert_eq!(models[0].1, model("CopA"));
        // A CM with no filter HMM is still one model.
        assert_eq!(
            cm_models("INFERNAL1/a\nNAME x\n//\n").unwrap(),
            [("x".to_string(), "INFERNAL1/a\nNAME x\n//\n".to_string())]
        );
        assert!(cm_models("HMMER3/f\nNAME x\n//\n").is_err());
        assert!(cm_models("INFERNAL1/a\nNAME x\n")
            .unwrap_err()
            .contains("terminator"));
        assert!(cm_models("INFERNAL1/a\n//\n")
            .unwrap_err()
            .contains("without a NAME"));
    }

    fn matrix(labels: &[&str], values: Vec<f64>) -> DataMatrix {
        let labels: Vec<String> = labels.iter().map(|s| s.to_string()).collect();
        DataMatrix {
            rows: labels.clone(),
            cols: labels,
            row_lengths: None,
            col_lengths: None,
            data_type: "score".into(),
            storage: Storage::Dense(values),
        }
    }

    /// Upstream `test_split_by_cluster_reuses_build_ssn_mst_cut_logic`: clusters are
    /// numbered by size, and an edge at the threshold does not join.
    #[test]
    fn clusters_are_size_ranked_components_above_the_threshold() {
        #[rustfmt::skip]
        let m = matrix(&["A", "B", "C", "D", "E"], vec![
            0.0, 9.0, 0.0, 0.0, 0.0,
            9.0, 0.0, 5.0, 0.0, 0.0,
            0.0, 5.0, 0.0, 0.0, 0.0,
            0.0, 0.0, 0.0, 0.0, 7.0,
            0.0, 0.0, 0.0, 7.0, 0.0,
        ]);
        let clusters = cluster_membership(&m, 5.0).unwrap();
        let ids: Vec<i64> = ["A", "B", "C", "D", "E"]
            .iter()
            .map(|l| clusters.of_label[*l])
            .collect();
        assert_eq!(ids, [1, 1, 3, 2, 2]);
        assert_eq!(
            clusters.sizes.values().copied().collect::<Vec<_>>(),
            [2, 2, 1]
        );

        let mut dist = m;
        dist.data_type = "score_dist".into();
        assert!(cluster_membership(&dist, 0.0)
            .unwrap_err()
            .contains("distance matrix"));
    }

    #[test]
    fn labels_must_match_exactly() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert!(check_labels_match(&s(&["a", "b"]), &s(&["b", "a"])).is_ok());
        let err = check_labels_match(&s(&["a", "b"]), &s(&["a", "c"])).unwrap_err();
        assert_eq!(
            err,
            "Matrix labels and input labels do not match exactly (matrix-only labels: b; \
             input-only labels: c)."
        );
        let err = input_labels(s(&["x", "y", "x"]), InputKind::Hmm).unwrap_err();
        assert_eq!(err, "Input HMM profiles contain duplicate labels: x");
    }

    #[test]
    fn the_plan_refuses_to_overwrite_and_pools_small_clusters() {
        let dir =
            std::env::temp_dir().join(format!("split_by_cluster_plan_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let sizes: BTreeMap<i64, usize> = [(1, 3), (2, 2), (3, 1)].into_iter().collect();
        let plan = plan_output_paths(&dir, ".gb", &sizes, 2, true).unwrap();
        assert_eq!(plan.clusters.keys().copied().collect::<Vec<_>>(), [1, 2]);
        assert_eq!(
            plan.destination(3),
            Some(dir.join("small_clusters.gb").as_path())
        );
        let no_small = plan_output_paths(&dir, ".gb", &sizes, 2, false).unwrap();
        assert_eq!(no_small.destination(3), None);
        std::fs::write(dir.join("1.gb"), "already here").unwrap();
        let err = plan_output_paths(&dir, ".gb", &sizes, 2, false).unwrap_err();
        assert!(
            err.starts_with("Refusing to overwrite existing file"),
            "{err}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
