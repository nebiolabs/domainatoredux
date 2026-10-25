//! The tools' interface: the 1.0 layer (`args`, `manifest`, `config_file`, `columns`,
//! `registry`) for migrated tools, and for the rest, until each is migrated, the old
//! native config structs with their `merge_cli` and the config schemas that
//! [`tool_manifest`] converts for the GUI.

// `domain_search_schema`'s `json!` literal outgrew the default limit of 128.
#![recursion_limit = "256"]

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub mod args;
pub mod columns;
pub mod config_file;
pub mod manifest;
pub mod registry;
pub mod resolve;
pub mod types;

/// Load a JSON or YAML config file into `T`, choosing the parser by extension
/// (`.json` -> JSON, otherwise YAML, which is a superset of JSON).
pub fn load_config<T: for<'de> Deserialize<'de>>(path: &str) -> Result<T> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading config {path}"))?;
    if path
        .rsplit('.')
        .next()
        .map(|e| e.eq_ignore_ascii_case("json"))
        .unwrap_or(false)
    {
        serde_json::from_str(&text).with_context(|| format!("parsing JSON config {path}"))
    } else {
        serde_yaml::from_str(&text).with_context(|| format!("parsing YAML config {path}"))
    }
}

/// Reject a NaN or infinite number anywhere in a resolved config, naming its key.
///
/// Upstream's jsonargparse refuses `nan` and `inf` for every float option ("Expected
/// a <class 'float'>. Got value: nan"). Rust's `f64` parser accepts both, and so does
/// YAML (`.nan`, `.inf`), so without this a `--kb_range inf` reached the coordinate
/// math and overflowed. Run once on the merged config, it covers every float flag
/// and config field of every tool, with nothing per-field to forget.
pub fn reject_non_finite<T: Serialize>(cfg: &T) -> Result<()> {
    fn walk(v: &serde_yaml::Value, path: &mut Vec<String>) -> Option<(String, f64)> {
        match v {
            serde_yaml::Value::Number(n) => n
                .as_f64()
                .filter(|f| !f.is_finite())
                .map(|f| (path.join("."), f)),
            serde_yaml::Value::Sequence(items) => items.iter().enumerate().find_map(|(i, x)| {
                path.push(i.to_string());
                let hit = walk(x, path);
                path.pop();
                hit
            }),
            serde_yaml::Value::Mapping(m) => m.iter().find_map(|(k, x)| {
                path.push(k.as_str().map_or_else(|| format!("{k:?}"), str::to_string));
                let hit = walk(x, path);
                path.pop();
                hit
            }),
            serde_yaml::Value::Tagged(t) => walk(&t.value, path),
            _ => None,
        }
    }
    let value = serde_yaml::to_value(cfg).context("serializing config")?;
    match walk(&value, &mut Vec::new()) {
        Some((key, f)) => anyhow::bail!("`{key}` must be a finite number, not {f}"),
        None => Ok(()),
    }
}

/// Serialize resolved params as YAML for `--print_config`.
pub fn print_config<T: Serialize>(cfg: &T) -> Result<String> {
    serde_yaml::to_string(cfg).context("serializing config")
}

/// Parse a resolved `partial` value (`None` = the default, `include`). Config files
/// skip clap's `value_parser`, so every tool validates here after the merge.
pub fn parse_partial(value: Option<&str>) -> Result<domainator_core::seq::partial::Partial> {
    value
        .unwrap_or("include")
        .parse()
        .map_err(|e: String| anyhow::anyhow!(e))
}

// ---------------------------------------------------------------------------
// Old config structs of the unmigrated tools, merged with their CLI flags
// ---------------------------------------------------------------------------

macro_rules! merge_scalar {
    ($self:ident, $cli:ident, $field:ident) => {
        if $cli.$field.is_some() {
            $self.$field = $cli.$field;
        }
    };
}
macro_rules! merge_vec {
    ($self:ident, $cli:ident, $field:ident) => {
        if !$cli.$field.is_empty() {
            $self.$field = $cli.$field;
        }
    };
}

// ---------------------------------------------------------------------------
// domainate / domain_search / seq_dist / compare_contigs / hmmer_compare
// ---------------------------------------------------------------------------

/// `domainate` parameters.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, deny_unknown_fields)]
pub struct DomainateConfig {
    pub input: Option<String>,
    pub references: Vec<String>,
    pub output: Option<String>,
    pub evalue: Option<f64>,
    pub min_evalue: Option<f64>,
    pub z: Option<f64>,
    pub max_overlap: Option<f64>,
    pub max_domains: Option<usize>,
    pub max_mode: bool,
    pub overlap_by_db: bool,
    pub hits_only: bool,
    pub no_annotations: bool,
    pub cpu: Option<usize>,
    pub fasta_type: Option<String>,
    pub include_taxids: Vec<i64>,
    pub exclude_taxids: Vec<i64>,
    pub taxonomy_expr: Option<String>,
    pub ncbi_taxonomy_path: Option<String>,
    /// `--partial`: `include` (default) | `exclude` | `only`, by fragment status.
    pub partial: Option<String>,
    /// `--gene_call`: `all` | `unannotated` (Orphos, meta mode); unset = no calling.
    pub gene_call: Option<String>,
}

impl DomainateConfig {
    pub fn merge_cli(&mut self, cli: DomainateConfig) {
        merge_scalar!(self, cli, input);
        merge_vec!(self, cli, references);
        merge_scalar!(self, cli, output);
        merge_scalar!(self, cli, evalue);
        merge_scalar!(self, cli, min_evalue);
        merge_scalar!(self, cli, z);
        merge_scalar!(self, cli, max_overlap);
        merge_scalar!(self, cli, max_domains);
        self.max_mode |= cli.max_mode;
        self.overlap_by_db |= cli.overlap_by_db;
        self.hits_only |= cli.hits_only;
        self.no_annotations |= cli.no_annotations;
        merge_scalar!(self, cli, cpu);
        merge_scalar!(self, cli, fasta_type);
        merge_vec!(self, cli, include_taxids);
        merge_vec!(self, cli, exclude_taxids);
        merge_scalar!(self, cli, taxonomy_expr);
        merge_scalar!(self, cli, ncbi_taxonomy_path);
        merge_scalar!(self, cli, partial);
        merge_scalar!(self, cli, gene_call);
    }
}

/// `domain_search` parameters.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, deny_unknown_fields)]
pub struct DomainSearchConfig {
    pub input: Option<String>,
    pub references: Vec<String>,
    pub output: Option<String>,
    pub evalue: Option<f64>,
    pub min_evalue: Option<f64>,
    pub z: Option<f64>,
    pub add_annotations: bool,
    pub max_hits: Option<usize>,
    pub max_hits_per_contig: Option<usize>,
    pub max_overlap: Option<f64>,
    pub max_mode: bool,
    pub batch_size: Option<u64>,
    pub cpu: Option<usize>,
    pub fasta_type: Option<String>,
    pub whole_contig: bool,
    pub cds_range: Option<i64>,
    pub cds_up: Option<i64>,
    pub cds_down: Option<i64>,
    pub kb_range: Option<f64>,
    pub kb_up: Option<f64>,
    pub kb_down: Option<f64>,
    pub strand: Option<String>,
    pub keep_direction: bool,
    pub translate: bool,
    pub pad: bool,
    pub max_region_overlap: Option<f64>,
    pub decoys: Vec<String>,
    pub decoys_file: Option<String>,
    pub include_taxids: Vec<i64>,
    pub exclude_taxids: Vec<i64>,
    pub taxonomy_expr: Option<String>,
    pub ncbi_taxonomy_path: Option<String>,
    pub max_output_gb: Option<f64>,
    /// `--partial`: `include` (default) | `exclude` | `only`, by fragment status.
    pub partial: Option<String>,
    /// `--gene_call`: `all` | `unannotated` (Orphos, meta mode); unset = no calling.
    pub gene_call: Option<String>,
}

impl DomainSearchConfig {
    pub fn merge_cli(&mut self, cli: DomainSearchConfig) {
        merge_scalar!(self, cli, input);
        merge_vec!(self, cli, references);
        merge_scalar!(self, cli, output);
        merge_scalar!(self, cli, evalue);
        merge_scalar!(self, cli, min_evalue);
        merge_scalar!(self, cli, z);
        self.add_annotations |= cli.add_annotations;
        merge_scalar!(self, cli, max_hits);
        merge_scalar!(self, cli, max_hits_per_contig);
        merge_scalar!(self, cli, max_overlap);
        self.max_mode |= cli.max_mode;
        merge_scalar!(self, cli, batch_size);
        merge_scalar!(self, cli, cpu);
        merge_scalar!(self, cli, fasta_type);
        self.whole_contig |= cli.whole_contig;
        merge_scalar!(self, cli, cds_range);
        merge_scalar!(self, cli, cds_up);
        merge_scalar!(self, cli, cds_down);
        merge_scalar!(self, cli, kb_range);
        merge_scalar!(self, cli, kb_up);
        merge_scalar!(self, cli, kb_down);
        merge_scalar!(self, cli, strand);
        self.keep_direction |= cli.keep_direction;
        self.translate |= cli.translate;
        self.pad |= cli.pad;
        merge_scalar!(self, cli, max_region_overlap);
        merge_vec!(self, cli, decoys);
        merge_scalar!(self, cli, decoys_file);
        merge_vec!(self, cli, include_taxids);
        merge_vec!(self, cli, exclude_taxids);
        merge_scalar!(self, cli, taxonomy_expr);
        merge_scalar!(self, cli, ncbi_taxonomy_path);
        merge_scalar!(self, cli, max_output_gb);
        merge_scalar!(self, cli, partial);
        merge_scalar!(self, cli, gene_call);
    }
}

/// `deduplicate_genbank` parameters.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, deny_unknown_fields)]
pub struct DeduplicateConfig {
    pub input: Vec<String>,
    pub output: Option<String>,
    pub log: Option<String>,
    pub fasta_type: Option<String>,
    pub algorithm: Option<String>,
    pub bin_path: Option<String>,
    pub id: Option<f64>,
    pub prefix_count: bool,
    pub suffix_count: bool,
    pub cpu: Option<usize>,
    pub params: Option<String>,
    pub fasta_out: bool,
    pub both_strands: bool,
    pub cluster_table: Option<String>,
    pub cluster_sep: Option<String>,
}

impl DeduplicateConfig {
    pub fn merge_cli(&mut self, cli: DeduplicateConfig) {
        merge_vec!(self, cli, input);
        merge_scalar!(self, cli, output);
        merge_scalar!(self, cli, log);
        merge_scalar!(self, cli, fasta_type);
        merge_scalar!(self, cli, algorithm);
        merge_scalar!(self, cli, bin_path);
        merge_scalar!(self, cli, id);
        self.prefix_count |= cli.prefix_count;
        self.suffix_count |= cli.suffix_count;
        merge_scalar!(self, cli, cpu);
        merge_scalar!(self, cli, params);
        self.fasta_out |= cli.fasta_out;
        self.both_strands |= cli.both_strands;
        merge_scalar!(self, cli, cluster_table);
        merge_scalar!(self, cli, cluster_sep);
    }
}

/// `genbank_to_fasta` parameters.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, deny_unknown_fields)]
pub struct GenbankToFastaConfig {
    pub input: Vec<String>,
    pub output: Option<String>,
}

impl GenbankToFastaConfig {
    pub fn merge_cli(&mut self, cli: GenbankToFastaConfig) {
        merge_vec!(self, cli, input);
        merge_scalar!(self, cli, output);
    }
}

/// `clean_sequences` parameters.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, deny_unknown_fields)]
pub struct CleanSequencesConfig {
    pub input: Vec<String>,
    pub output: Option<String>,
    pub log: Option<String>,
    pub fasta_type: Option<String>,
    pub fasta_out: bool,
    pub clean_name: bool,
    pub deduplicate_names: bool,
    pub strip: bool,
    pub filter_by_aa: bool,
}

impl CleanSequencesConfig {
    pub fn merge_cli(&mut self, cli: CleanSequencesConfig) {
        merge_vec!(self, cli, input);
        merge_scalar!(self, cli, output);
        merge_scalar!(self, cli, log);
        merge_scalar!(self, cli, fasta_type);
        self.fasta_out |= cli.fasta_out;
        self.clean_name |= cli.clean_name;
        self.deduplicate_names |= cli.deduplicate_names;
        self.strip |= cli.strip;
        self.filter_by_aa |= cli.filter_by_aa;
    }
}

/// `gene_call` parameters (a port-only tool: Prodigal-style gene calling through the
/// external Orphos binary).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, deny_unknown_fields)]
pub struct GeneCallConfig {
    pub input: Vec<String>,
    pub output: Option<String>,
    pub fasta_type: Option<String>,
    /// `meta` (default) | `single`.
    pub mode: Option<String>,
    /// `unannotated` (default) | `all`.
    pub scope: Option<String>,
    pub translation_table: Option<u8>,
    pub closed: bool,
    pub mask: bool,
    pub cpu: Option<usize>,
    pub bin_path: Option<String>,
}

impl GeneCallConfig {
    pub fn merge_cli(&mut self, cli: GeneCallConfig) {
        merge_vec!(self, cli, input);
        merge_scalar!(self, cli, output);
        merge_scalar!(self, cli, fasta_type);
        merge_scalar!(self, cli, mode);
        merge_scalar!(self, cli, scope);
        merge_scalar!(self, cli, translation_table);
        self.closed |= cli.closed;
        self.mask |= cli.mask;
        merge_scalar!(self, cli, cpu);
        merge_scalar!(self, cli, bin_path);
    }
}

/// `color_genbank` parameters.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ColorGenbankConfig {
    pub input: Vec<String>,
    pub output: Option<String>,
    pub contigs: Vec<String>,
    pub contigs_file: Option<String>,
    pub color_table: Option<String>,
    pub color_table_out: Option<String>,
    pub search_hit_color: Option<String>,
    pub clear: bool,
    pub databases: Vec<String>,
    pub mode: Option<String>,
    pub domain_database: bool,
}

impl ColorGenbankConfig {
    pub fn merge_cli(&mut self, cli: ColorGenbankConfig) {
        merge_vec!(self, cli, input);
        merge_scalar!(self, cli, output);
        merge_vec!(self, cli, contigs);
        merge_scalar!(self, cli, contigs_file);
        merge_scalar!(self, cli, color_table);
        merge_scalar!(self, cli, color_table_out);
        merge_scalar!(self, cli, search_hit_color);
        self.clear |= cli.clear;
        merge_vec!(self, cli, databases);
        merge_scalar!(self, cli, mode);
        self.domain_database |= cli.domain_database;
    }
}

/// `color_table_to_legend` parameters.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ColorTableToLegendConfig {
    pub input: Vec<String>,
    pub svg: Option<String>,
    pub title: Option<String>,
}

impl ColorTableToLegendConfig {
    pub fn merge_cli(&mut self, cli: ColorTableToLegendConfig) {
        merge_vec!(self, cli, input);
        merge_scalar!(self, cli, svg);
        merge_scalar!(self, cli, title);
    }
}

/// `trim_contigs` parameters.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, deny_unknown_fields)]
pub struct TrimContigsConfig {
    pub input: Vec<String>,
    pub output: Option<String>,
    pub kb_both: Option<f64>,
    pub kb_left: Option<f64>,
    pub kb_right: Option<f64>,
    pub cds_both: Option<i64>,
    pub cds_left: Option<i64>,
    pub cds_right: Option<i64>,
    pub domains: Vec<String>,
    pub domains_file: Option<String>,
    pub domain_expr: Option<String>,
    pub no_domain: bool,
    pub contigs: Vec<String>,
    pub contigs_file: Option<String>,
}

impl TrimContigsConfig {
    pub fn merge_cli(&mut self, cli: TrimContigsConfig) {
        merge_vec!(self, cli, input);
        merge_scalar!(self, cli, output);
        merge_scalar!(self, cli, kb_both);
        merge_scalar!(self, cli, kb_left);
        merge_scalar!(self, cli, kb_right);
        merge_scalar!(self, cli, cds_both);
        merge_scalar!(self, cli, cds_left);
        merge_scalar!(self, cli, cds_right);
        merge_vec!(self, cli, domains);
        merge_scalar!(self, cli, domains_file);
        merge_scalar!(self, cli, domain_expr);
        self.no_domain |= cli.no_domain;
        merge_vec!(self, cli, contigs);
        merge_scalar!(self, cli, contigs_file);
    }
}

/// `seq_dist` parameters.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, deny_unknown_fields)]
pub struct SeqDistConfig {
    pub input: Option<String>,
    pub reference: Option<String>,
    pub dense: Option<String>,
    pub dense_text: Option<String>,
    pub sparse: Option<String>,
    pub algorithm: Option<String>,
    pub mode: Option<String>,
    pub lb: Option<f64>,
    pub k: Option<usize>,
    pub mst_knn: Option<usize>,
    pub knn: Option<usize>,
    pub params: Option<String>,
    pub cpu: Option<usize>,
    pub max_output_gb: Option<f64>,
}

impl SeqDistConfig {
    pub fn merge_cli(&mut self, cli: SeqDistConfig) {
        merge_scalar!(self, cli, input);
        merge_scalar!(self, cli, reference);
        merge_scalar!(self, cli, dense);
        merge_scalar!(self, cli, dense_text);
        merge_scalar!(self, cli, sparse);
        merge_scalar!(self, cli, algorithm);
        merge_scalar!(self, cli, mode);
        merge_scalar!(self, cli, lb);
        merge_scalar!(self, cli, k);
        merge_scalar!(self, cli, mst_knn);
        merge_scalar!(self, cli, knn);
        merge_scalar!(self, cli, params);
        merge_scalar!(self, cli, cpu);
        merge_scalar!(self, cli, max_output_gb);
    }
}

/// `compare_contigs` parameters.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, deny_unknown_fields)]
pub struct CompareContigsConfig {
    pub input: Vec<String>,
    pub dense: Option<String>,
    pub dense_text: Option<String>,
    pub sparse: Option<String>,
    pub ji: Option<f64>,
    pub ai: Option<f64>,
    pub databases: Vec<String>,
    pub k: Option<usize>,
    pub lb: Option<f64>,
    pub mst_knn: Option<usize>,
    pub knn: Option<usize>,
    pub cpu: Option<usize>,
    pub max_output_gb: Option<f64>,
}

impl CompareContigsConfig {
    pub fn merge_cli(&mut self, cli: CompareContigsConfig) {
        merge_vec!(self, cli, input);
        merge_scalar!(self, cli, dense);
        merge_scalar!(self, cli, dense_text);
        merge_scalar!(self, cli, sparse);
        merge_scalar!(self, cli, ji);
        merge_scalar!(self, cli, ai);
        merge_vec!(self, cli, databases);
        merge_scalar!(self, cli, k);
        merge_scalar!(self, cli, lb);
        merge_scalar!(self, cli, mst_knn);
        merge_scalar!(self, cli, knn);
        merge_scalar!(self, cli, cpu);
        merge_scalar!(self, cli, max_output_gb);
    }
}

/// `transform_matrix` parameters.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, deny_unknown_fields)]
pub struct TransformMatrixConfig {
    pub input: Option<String>,
    pub dense: Option<String>,
    pub dense_text: Option<String>,
    pub sparse: Option<String>,
    pub mode: Option<String>,
    pub input_type: Option<String>,
    pub lb: Option<f64>,
    pub mst_knn: Option<usize>,
    pub knn: Option<usize>,
    pub max_output_gb: Option<f64>,
}

impl TransformMatrixConfig {
    pub fn merge_cli(&mut self, cli: TransformMatrixConfig) {
        merge_scalar!(self, cli, input);
        merge_scalar!(self, cli, dense);
        merge_scalar!(self, cli, dense_text);
        merge_scalar!(self, cli, sparse);
        merge_scalar!(self, cli, mode);
        merge_scalar!(self, cli, input_type);
        merge_scalar!(self, cli, lb);
        merge_scalar!(self, cli, mst_knn);
        merge_scalar!(self, cli, knn);
        merge_scalar!(self, cli, max_output_gb);
    }
}

/// `build_ssn` parameters.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, deny_unknown_fields)]
pub struct BuildSsnConfig {
    pub input: Option<String>,
    pub xgmml: Option<String>,
    pub lb: Option<f64>,
    pub metadata: Vec<String>,
    pub subset: Vec<String>,
    pub subset_file: Option<String>,
    pub color_by: Option<String>,
    pub max_color_groups: Option<usize>,
    pub color_table: Option<String>,
    pub color_table_out: Option<String>,
    pub cluster: bool,
    pub cluster_tsv: Option<String>,
    pub no_cluster_header: bool,
    pub mst: bool,
    pub mst_knn: Option<usize>,
    pub knn: Option<usize>,
    pub max_output_gb: Option<f64>,
}

impl BuildSsnConfig {
    pub fn merge_cli(&mut self, cli: BuildSsnConfig) {
        merge_scalar!(self, cli, input);
        merge_scalar!(self, cli, xgmml);
        merge_scalar!(self, cli, lb);
        merge_vec!(self, cli, metadata);
        merge_vec!(self, cli, subset);
        merge_scalar!(self, cli, subset_file);
        merge_scalar!(self, cli, color_by);
        merge_scalar!(self, cli, max_color_groups);
        merge_scalar!(self, cli, color_table);
        merge_scalar!(self, cli, color_table_out);
        self.cluster |= cli.cluster;
        merge_scalar!(self, cli, cluster_tsv);
        self.no_cluster_header |= cli.no_cluster_header;
        self.mst |= cli.mst;
        merge_scalar!(self, cli, mst_knn);
        merge_scalar!(self, cli, knn);
        merge_scalar!(self, cli, max_output_gb);
    }
}

/// `matrix_report` parameters.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, deny_unknown_fields)]
pub struct MatrixReportConfig {
    pub input: Option<String>,
    pub output: Option<String>,
    pub json: Option<String>,
    pub merge_impact_metric: Option<String>,
    pub max_merge_events: Option<usize>,
}

impl MatrixReportConfig {
    pub fn merge_cli(&mut self, cli: MatrixReportConfig) {
        merge_scalar!(self, cli, input);
        merge_scalar!(self, cli, output);
        merge_scalar!(self, cli, json);
        merge_scalar!(self, cli, merge_impact_metric);
        merge_scalar!(self, cli, max_merge_events);
    }
}

/// `summary_report` parameters.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, deny_unknown_fields)]
pub struct SummaryReportConfig {
    pub input: Vec<String>,
    pub output: Option<String>,
    pub json: Option<String>,
    pub domains: Vec<String>,
    pub databases: Vec<String>,
    pub taxonomy: bool,
    pub ncbi_taxonomy_path: Option<String>,
    /// `--partial`: `include` (default) | `exclude` | `only`, by fragment status.
    pub partial: Option<String>,
}

impl SummaryReportConfig {
    pub fn merge_cli(&mut self, cli: SummaryReportConfig) {
        merge_vec!(self, cli, input);
        merge_scalar!(self, cli, output);
        merge_scalar!(self, cli, json);
        merge_vec!(self, cli, domains);
        merge_vec!(self, cli, databases);
        self.taxonomy |= cli.taxonomy;
        merge_scalar!(self, cli, ncbi_taxonomy_path);
        merge_scalar!(self, cli, partial);
    }
}

/// `ssn_navigator` parameters.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, deny_unknown_fields)]
pub struct SsnNavigatorConfig {
    pub input: Option<String>,
    pub output: Option<String>,
    pub mode: Option<String>,
    /// A number or `inf`; kept as a string so `inf` round-trips through JSON/YAML,
    /// which have no infinity literal.
    pub threshold: Option<String>,
    pub id: Option<usize>,
    pub node: Option<String>,
    pub min_size: Option<usize>,
    pub top_n: Option<usize>,
    pub members: bool,
    pub max_merge_events: Option<usize>,
}

impl SsnNavigatorConfig {
    pub fn merge_cli(&mut self, cli: SsnNavigatorConfig) {
        merge_scalar!(self, cli, input);
        merge_scalar!(self, cli, output);
        merge_scalar!(self, cli, mode);
        merge_scalar!(self, cli, threshold);
        merge_scalar!(self, cli, id);
        merge_scalar!(self, cli, node);
        merge_scalar!(self, cli, min_size);
        merge_scalar!(self, cli, top_n);
        merge_scalar!(self, cli, max_merge_events);
        self.members |= cli.members;
    }
}

/// `build_ssn_viewer` parameters.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, deny_unknown_fields)]
pub struct BuildSsnViewerConfig {
    pub input: Option<String>,
    pub output: Option<String>,
    pub html: Option<String>,
    pub embed_data: bool,
    pub name: Option<String>,
    pub metadata: Vec<String>,
    pub subset: Vec<String>,
    pub subset_file: Option<String>,
    pub color_by: Option<String>,
    pub label_by: Option<String>,
    pub categorical: Vec<String>,
    pub merge_impact_metric: Option<String>,
    pub max_output_gb: Option<f64>,
}

impl BuildSsnViewerConfig {
    pub fn merge_cli(&mut self, cli: BuildSsnViewerConfig) {
        merge_scalar!(self, cli, input);
        merge_scalar!(self, cli, output);
        merge_scalar!(self, cli, html);
        self.embed_data |= cli.embed_data;
        merge_scalar!(self, cli, name);
        merge_vec!(self, cli, metadata);
        merge_vec!(self, cli, subset);
        merge_scalar!(self, cli, subset_file);
        merge_scalar!(self, cli, color_by);
        merge_scalar!(self, cli, label_by);
        merge_vec!(self, cli, categorical);
        merge_scalar!(self, cli, merge_impact_metric);
        merge_scalar!(self, cli, max_output_gb);
    }
}

/// `hmmer_compare` parameters.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, deny_unknown_fields)]
pub struct HmmerCompareConfig {
    pub input: Vec<String>,
    pub reference: Vec<String>,
    pub output: Option<String>,
    pub score_cutoff: Option<f64>,
    pub k: Option<usize>,
    pub alignments: bool,
    pub cpu: Option<usize>,
    pub max_output_gb: Option<f64>,
}

impl HmmerCompareConfig {
    pub fn merge_cli(&mut self, cli: HmmerCompareConfig) {
        merge_vec!(self, cli, input);
        merge_vec!(self, cli, reference);
        merge_scalar!(self, cli, output);
        merge_scalar!(self, cli, score_cutoff);
        merge_scalar!(self, cli, k);
        self.alignments |= cli.alignments;
        merge_scalar!(self, cli, cpu);
        merge_scalar!(self, cli, max_output_gb);
    }
}

/// `hmmer_select` parameters (config-file shape).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, deny_unknown_fields)]
pub struct HmmerSelectConfig {
    pub input: Vec<String>,
    pub output: Option<String>,
    pub regex: Vec<String>,
    pub exact: Vec<String>,
    pub contains: Vec<String>,
    pub field: Option<String>,
    pub case_sensitive: bool,
    pub alphabet: Option<String>,
}

impl HmmerSelectConfig {
    pub fn merge_cli(&mut self, cli: HmmerSelectConfig) {
        merge_vec!(self, cli, input);
        merge_scalar!(self, cli, output);
        merge_vec!(self, cli, regex);
        merge_vec!(self, cli, exact);
        merge_vec!(self, cli, contains);
        merge_scalar!(self, cli, field);
        self.case_sensitive |= cli.case_sensitive;
        merge_scalar!(self, cli, alphabet);
    }
}

/// `hmmer_build` parameters.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, deny_unknown_fields)]
pub struct HmmerBuildConfig {
    pub input: Option<String>,
    pub output: Option<String>,
    pub name: Option<String>,
    pub acc: Option<String>,
    pub desc: Option<String>,
    pub alphabet: Option<String>,
    pub window_length: Option<i32>,
    pub window_beta: Option<f64>,
}

impl HmmerBuildConfig {
    pub fn merge_cli(&mut self, cli: HmmerBuildConfig) {
        merge_scalar!(self, cli, input);
        merge_scalar!(self, cli, output);
        merge_scalar!(self, cli, name);
        merge_scalar!(self, cli, acc);
        merge_scalar!(self, cli, desc);
        merge_scalar!(self, cli, alphabet);
        merge_scalar!(self, cli, window_length);
        merge_scalar!(self, cli, window_beta);
    }
}

/// `hmmer_search` parameters.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, deny_unknown_fields)]
pub struct HmmerSearchConfig {
    pub input: Vec<String>,
    pub reference: Vec<String>,
    pub output: Option<String>,
    pub score_cutoff: Option<f64>,
    pub max_hits: Option<usize>,
    pub cpu: Option<usize>,
    pub max_output_gb: Option<f64>,
}

impl HmmerSearchConfig {
    pub fn merge_cli(&mut self, cli: HmmerSearchConfig) {
        merge_vec!(self, cli, input);
        merge_vec!(self, cli, reference);
        merge_scalar!(self, cli, output);
        merge_scalar!(self, cli, score_cutoff);
        merge_scalar!(self, cli, max_hits);
        merge_scalar!(self, cli, cpu);
        merge_scalar!(self, cli, max_output_gb);
    }
}

/// `format_db` (domainator_format_db) parameters (config-file shape).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, deny_unknown_fields)]
pub struct FormatDbConfig {
    pub input: Vec<String>,
    pub shards: Option<usize>,
    pub compress: bool,
    pub index: bool,
    pub output_dir: Option<String>,
    pub name: Option<String>,
    pub level: Option<u8>,
    pub force: bool,
    pub log: Option<String>,
}

impl FormatDbConfig {
    pub fn merge_cli(&mut self, cli: FormatDbConfig) {
        merge_vec!(self, cli, input);
        merge_scalar!(self, cli, shards);
        self.compress |= cli.compress;
        self.index |= cli.index;
        merge_scalar!(self, cli, output_dir);
        merge_scalar!(self, cli, name);
        merge_scalar!(self, cli, level);
        self.force |= cli.force;
        merge_scalar!(self, cli, log);
    }
}

/// A tool's manifest entry, as `domainatoredux describe` gives it: what the GUI renders
/// its dialogs from. A migrated tool's comes from its clap arguments. An unmigrated
/// tool's is converted from its old config schema, whose property names are its
/// config keys (until migration they need not equal the command line's).
pub fn tool_manifest(tool: &str) -> Result<serde_json::Value> {
    use serde_json::{json, Value};
    let info = registry::tool_info(tool)
        .with_context(|| format!("unknown tool {tool:?} (`domainatoredux --help` lists them)"))?;
    if let Some(cmd) = args::tool_command(tool) {
        return Ok(manifest::describe_tool(&cmd, info));
    }
    let schema = legacy_schema(tool)?;
    let parameters: Vec<Value> = schema["properties"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(name, prop)| legacy_parameter(name, prop))
        .collect();
    Ok(json!({
        "name": info.name,
        "category": info.category,
        "summary": info.summary,
        "description": schema["description"].as_str().unwrap_or(info.summary),
        "parameters": parameters,
        "required_outputs": info.required_outputs,
        "mutually_exclusive": [],
    }))
}

/// One property of an old config schema as a manifest parameter.
fn legacy_parameter(name: &str, prop: &serde_json::Value) -> serde_json::Value {
    use serde_json::{json, Map, Value};
    let first_type = |p: &Value| -> String {
        match &p["type"] {
            Value::String(t) => t.clone(),
            Value::Array(ts) => ts
                .iter()
                .filter_map(Value::as_str)
                .find(|t| *t != "null")
                .unwrap_or("string")
                .to_string(),
            _ => "string".to_string(),
        }
    };
    let base = first_type(prop);
    let multiple = base == "array";
    let (ty, item) = if multiple {
        (first_type(&prop["items"]), &prop["items"])
    } else {
        (base, prop)
    };
    let role = manifest::path_role(name);
    let kind = if ty == "boolean" && !multiple {
        "flag"
    } else if role.is_some() {
        "path"
    } else {
        "value"
    };
    let mut p = Map::new();
    p.insert("name".into(), json!(name));
    p.insert("flags".into(), json!([format!("--{name}")]));
    p.insert(
        "help".into(),
        json!(prop["description"].as_str().unwrap_or_default()),
    );
    p.insert("kind".into(), json!(kind));
    p.insert("type".into(), json!(ty));
    if let Some(role) = role {
        p.insert("role".into(), json!(role));
    }
    if multiple {
        p.insert("multiple".into(), json!(true));
    }
    p.insert("required".into(), json!(false));
    p.insert(
        "default".into(),
        prop.get("default").cloned().unwrap_or(Value::Null),
    );
    let choices: Vec<Value> = item["enum"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|v| !v.is_null())
        .cloned()
        .collect();
    if !choices.is_empty() {
        p.insert("choices".into(), Value::Array(choices));
    }
    for (theirs, ours) in [
        ("minimum", "min"),
        ("maximum", "max"),
        ("exclusiveMinimum", "exclusive_min"),
    ] {
        if let Some(v) = item.get(theirs) {
            p.insert(ours.into(), v.clone());
        }
    }
    Value::Object(p)
}

/// The draft-07 JSON Schema of an unmigrated tool's config file, which
/// [`tool_manifest`] turns into manifest parameters for the GUI. Each one goes when
/// its tool is migrated.
fn legacy_schema(tool: &str) -> Result<serde_json::Value> {
    let s = match tool {
        "domainate" => domainate_schema(),
        "domain_search" => domain_search_schema(),
        "deduplicate_genbank" => deduplicate_schema(),
        "genbank_to_fasta" => genbank_to_fasta_schema(),
        "clean_sequences" => clean_sequences_schema(),
        "gene_call" => gene_call_schema(),
        "color_genbank" => color_genbank_schema(),
        "color_table_to_legend" => color_table_to_legend_schema(),
        "trim_contigs" => trim_contigs_schema(),
        "seq_dist" => seq_dist_schema(),
        "compare_contigs" => compare_contigs_schema(),
        "hmmer_compare" => hmmer_compare_schema(),
        "transform_matrix" => transform_matrix_schema(),
        "build_ssn" => build_ssn_schema(),
        "build_ssn_viewer" => build_ssn_viewer_schema(),
        "matrix_report" => matrix_report_schema(),
        "ssn_navigator" => ssn_navigator_schema(),
        "summary_report" => summary_report_schema(),
        "hmmer_select" => hmmer_select_schema(),
        "hmmer_build" => hmmer_build_schema(),
        "hmmer_search" => hmmer_search_schema(),
        "format_db" => format_db_schema(),
        other => {
            anyhow::bail!("no schema for tool {other:?} (`domainatoredux --help` lists the tools)")
        }
    };
    let mut s = s;
    fill_common_descriptions(&mut s);
    Ok(s)
}

/// Fill in a `description` for any top-level property that lacks one, from the
/// shared [`common_help`] dictionary. Tool-specific descriptions written in the
/// per-tool schema functions always take precedence (they're never overwritten).
/// This gives every option a hint/tooltip in the GUI without duplicating text.
fn fill_common_descriptions(schema: &mut serde_json::Value) {
    let Some(props) = schema.get_mut("properties").and_then(|p| p.as_object_mut()) else {
        return;
    };
    for (name, prop) in props.iter_mut() {
        if let Some(obj) = prop.as_object_mut() {
            if !obj.contains_key("description") {
                if let Some(help) = common_help(name) {
                    obj.insert("description".into(), serde_json::Value::String(help.into()));
                }
            }
        }
    }
}

/// One-line help for a parameter name, shared across every tool that uses it.
fn common_help(name: &str) -> Option<&'static str> {
    Some(match name {
        "input" => "Input file(s) for the tool.",
        "output" => "Output file path.",
        "references" => "Reference HMM or FASTA file(s) to search with (protein or nucleotide).",
        "acc" => "Profile accession (ACC line) to set.",
        "name" => "Profile NAME to set.",
        "desc" => "Profile description (DESC line) to set.",
        "add_annotations" => "Also add domain annotations to the extracted neighborhoods.",
        "algorithm" => "Algorithm to use.",
        "alignments" => "Include per-hit alignment strings in the output.",
        "annotation_type" => "Which annotation kind to act on: all, domainator, or domain_search.",
        "batch_size" => "Number of records to process per batch.",
        "by" => "Report granularity: contig, cds, or domain.",
        "case_sensitive" => "Match names case-sensitively.",
        "cds" => "CDS names to select.",
        "cds_file" => "File listing CDS names (one per line).",
        "cdss" => "CDS names to extract.",
        "cdss_file" => "File listing CDS names to extract (one per line).",
        "cds_both" => "Trim this many CDSs from both ends.",
        "cds_left" => "Trim this many CDSs from the left end.",
        "cds_right" => "Trim this many CDSs from the right end.",
        "cds_up" => "Extend the neighborhood this many CDSs upstream of the hit.",
        "cds_down" => "Extend the neighborhood this many CDSs downstream of the hit.",
        "cds_range" => "Extend the neighborhood this many CDSs on both sides of the hit.",
        "cds_count_lb" => "Keep contigs with at least this many CDSs.",
        "cds_count_ub" => "Keep contigs with at most this many CDSs.",
        "clear" => "Clear existing colors before applying new ones.",
        "cluster" => "Cluster nodes into connected components and add a cluster attribute.",
        "cluster_tsv" => "Also write a TSV mapping each node to its cluster.",
        "no_cluster_header" => "Omit the header row from the cluster TSV.",
        "color_by" => "Node attribute to color by.",
        "max_color_groups" => "Give distinct colors only to this many of the largest color_by groups; the rest share a neutral gray.",
        "label_by" => "Node attribute to label by.",
        "categorical" => "Numeric metadata columns to color as discrete categories rather than with a gradient.",
        "color_table" => "TSV of name→color to apply.",
        "color_table_out" => "Write the resolved color table (name→color TSV) to this path.",
        "search_hit_color" => "Color to use for Domain_Search best-hit features.",
        "combine" => "Combine adjacent/overlapping domains into a single region.",
        "splice" => "Splice multi-segment domains into a single sequence.",
        "contigs" => "Restrict to these contig (LOCUS) names.",
        "contigs_file" => "File listing contig names to restrict to (one per line).",
        "contigs_regex" => "Restrict to contigs whose name matches this regex.",
        "definition_regex" => "Keep contigs whose DEFINITION matches this regex.",
        "sequence_regex" => "Keep contigs whose sequence matches this regex.",
        "cpu" => "Number of CPU threads to use (0 = all cores).",
        "databases" => "Restrict to domains from these databases.",
        "databases_keep" => "Keep only domains from these databases.",
        "databases_filter" => "Remove domains from these databases.",
        "decoys" => "Decoy reference names that suppress a hit when they score best.",
        "decoys_file" => "File listing decoy reference names (one per line).",
        "deduplicate_names" => "Make duplicate record names unique.",
        "dense" => "Write the matrix as a dense HDF5 file.",
        "dense_text" => "Write the matrix as a dense TSV file.",
        "sparse" => "Write the matrix as a sparse HDF5 file.",
        "domain_database" => "Color by database name instead of domain name.",
        "domain_expr" => "Boolean expression over domain names to select by.",
        "domain_type" => "Annotation feature type to match (default 'domain').",
        "domains" => "Domain names to select.",
        "domains_file" => "File listing domain names to select (one per line).",
        "evalue" => "E-value threshold.",
        "min_evalue" => "Discard hits with an E-value below this floor.",
        "score" => "Bit-score threshold.",
        "score_cutoff" => "Minimum score to keep a hit.",
        "exclude_taxids" => "Exclude records under these NCBI taxids.",
        "include_taxids" => "Keep only records under these NCBI taxids.",
        "taxonomy_expr" => "Boolean taxonomy expression (mutually exclusive with include/exclude taxids).",
        "ncbi_taxonomy_path" => "Path to an NCBI taxdump directory (required for taxonomy filters).",
        "fasta_out" => "Write FASTA instead of GenBank.",
        "fasta_type" => "Molecule type for FASTA input: protein, dna, or rna.",
        "first" => "Keep only the first N contigs.",
        "include_nucleic_acids" => "Include nucleic-acid (non-CDS) regions.",
        "invert" => "Invert the selection (keep everything that does NOT match).",
        "k" => "Number of nearest neighbors (k) for the kNN graph.",
        "kb_both" => "Trim this many kb from both ends.",
        "kb_left" => "Trim this many kb from the left end.",
        "kb_right" => "Trim this many kb from the right end.",
        "kb_up" => "Extend the neighborhood this many kb upstream of the hit.",
        "kb_down" => "Extend the neighborhood this many kb downstream of the hit.",
        "kb_range" => "Extend the neighborhood this many kb on both sides of the hit.",
        "whole_contig" => "Extract the entire contig around each hit.",
        "keep_direction" => "Keep the original strand orientation (don't normalize direction).",
        "keep_name" => "Keep the original record name.",
        "largest" => "Extract only the largest unannotated region per record.",
        "lb" => "Lower bound (minimum value or length to keep).",
        "length_lb" => "Keep contigs at least this many bp long.",
        "length_ub" => "Keep contigs at most this many bp long.",
        "log" => "Write a run log/summary to this file.",
        "max_hits" => "Maximum number of hits to keep.",
        "max_hits_per_contig" => "Maximum number of hits to keep per contig.",
        "max_merge_events" => "How many of the strongest cluster-merge (split) events to select, applied where the series is displayed.",
        "max_mode" => "Score-aggregation mode when picking the best hit.",
        "max_overlap" => "Maximum allowed fractional overlap between domains.",
        "max_region_overlap" => "Maximum allowed fractional overlap between extracted regions.",
        "overlap_by_db" => "Apply the overlap limit per-database rather than globally.",
        "merge_impact_metric" => "Metric used to rank cluster-merge events.",
        "metadata" => "Additional per-node metadata TSV to attach.",
        "mode" => "Operation mode / variant (see the tool's documentation).",
        "mst" => "Build a minimum spanning tree only.",
        "name_field" => "Qualifier to use as the output sequence name.",
        "no_annotations" => "Don't write domain annotations (report hits only).",
        "no_domain" => "Drop contigs that have no matching domain.",
        "pad" => "Pad extracted regions to equal length.",
        "partial" => "Select by fragment status: include (all), exclude (skip fragments), or only (only fragments). A CDS is a fragment if its location has '<' or '>', or it has a bare /partial qualifier; a protein record if it has a UniProt ' (Fragment)' header, a 'Flags: Fragment' definition, NON_TER features, or a CDS/Protein feature with '<' or '>' spanning it. Never inverted by --invert.",
        "pad_up" => "Pad the extracted region this many residues upstream.",
        "pad_down" => "Pad the extracted region this many residues downstream.",
        "prefix_count" => "Prefix the cluster size to each representative's name.",
        "suffix_count" => "Suffix the cluster size to each representative's name.",
        "skip_deduplicate" => "Don't deduplicate output records by name.",
        "search_hits" => "Operate on Domain_Search best-hit features.",
        "search_score_lb" => "Keep contigs whose best search score is at least this.",
        "search_score_ub" => "Keep contigs whose best search score is at most this.",
        "strand" => "Restrict to a strand: f (forward) or r (reverse).",
        "subset" => "Restrict the matrix to these row/column labels.",
        "subset_file" => "File listing matrix labels to restrict to (one per line).",
        "svg" => "Output SVG file path.",
        "title" => "Title text.",
        "translate" => "Translate extracted nucleotide regions to protein.",
        "unannotated" => "Select CDSs/contigs with no domain annotation.",
        "z" => "Effective database size (Z) used for E-value calculation.",
        _ => return None,
    })
}

fn domainate_schema() -> serde_json::Value {
    use serde_json::json;
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "domainate config",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "partial": {"enum": ["include", "exclude", "only"], "default": "include"},
            "input": {"type": ["string", "null"], "description": "Sequence file (GenBank/FASTA)"},
            "references": {"type": "array", "items": {"type": "string"}, "description": ".hmm reference files"},
            "output": {"type": ["string", "null"]},
            "evalue": {"type": "number", "default": 0.001},
            "min_evalue": {"type": "number", "default": 0.0, "description": "Drop hits with i-evalue below this"},
            "z": {"type": "number", "default": 1000.0, "description": "E-value search space size (-Z)"},
            "max_overlap": {"type": "number", "default": 1.0},
            "max_domains": {"type": ["integer", "null"], "description": "Per-CDS domain cap (0/unset = unlimited)"},
            "max_mode": {"type": "boolean", "default": false, "description": "hmmsearch --max (max sensitivity)"},
            "overlap_by_db": {"type": "boolean", "default": false},
            "hits_only": {"type": "boolean", "default": false, "description": "Output only contigs with a hit"},
            "no_annotations": {"type": "boolean", "default": false},
            "cpu": {"type": ["integer", "null"], "description": "Worker threads (0 = all cores)"},
            "fasta_type": {"enum": ["protein", "nucleotide", null]},
            "include_taxids": {"type": "array", "items": {"type": "integer"}},
            "exclude_taxids": {"type": "array", "items": {"type": "integer"}},
            "taxonomy_expr": {"type": ["string", "null"]},
            "ncbi_taxonomy_path": {"type": ["string", "null"]},
            "gene_call": {"enum": ["all", "unannotated", null], "description": GENE_CALL_HELP}
        }
    })
}

/// `--gene_call` help, shared by `domainate` and `domain_search`.
const GENE_CALL_HELP: &str = "Call genes with Orphos (Prodigal's metagenomic mode) before searching: 'all' deletes every existing CDS (and its Domainator annotations) and re-calls every nucleotide contig; 'unannotated' calls only contigs with no CDS. Needs the orphos binary. Give -Z too: the database size is fixed before genes are called.";

fn domain_search_schema() -> serde_json::Value {
    use serde_json::json;
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "domain_search config",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "partial": {"enum": ["include", "exclude", "only"], "default": "include"},
            "input": {"type": ["string", "null"], "description": "Input database (GenBank/FASTA; logical shard name)"},
            "references": {"type": "array", "items": {"type": "string"}},
            "output": {"type": ["string", "null"]},
            "evalue": {"type": "number", "default": 0.001},
            "min_evalue": {"type": "number", "default": 0.0},
            "z": {"type": "number", "default": 1000.0},
            "add_annotations": {"type": "boolean", "default": false},
            "max_hits": {"type": ["integer", "null"]},
            "max_hits_per_contig": {"type": ["integer", "null"]},
            "max_overlap": {"type": "number", "default": 1.0},
            "max_mode": {"type": "boolean", "default": false},
            "batch_size": {"type": "integer", "default": 10000},
            "cpu": {"type": "integer", "default": 0},
            "fasta_type": {"enum": ["protein", "nucleotide", null]},
            "whole_contig": {"type": "boolean", "default": false},
            "cds_range": {"type": ["integer", "null"]},
            "cds_up": {"type": ["integer", "null"]},
            "cds_down": {"type": ["integer", "null"]},
            "kb_range": {"type": ["number", "null"]},
            "kb_up": {"type": ["number", "null"]},
            "kb_down": {"type": ["number", "null"]},
            "strand": {"enum": ["f", "r", null]},
            "keep_direction": {"type": "boolean", "default": false},
            "translate": {"type": "boolean", "default": false},
            "pad": {"type": "boolean", "default": false},
            "max_region_overlap": {"type": "number", "default": 1.0},
            "decoys": {"type": "array", "items": {"type": "string"}},
            "decoys_file": {"type": ["string", "null"]},
            "include_taxids": {"type": "array", "items": {"type": "integer"}},
            "exclude_taxids": {"type": "array", "items": {"type": "integer"}},
            "taxonomy_expr": {"type": ["string", "null"]},
            "ncbi_taxonomy_path": {"type": ["string", "null"]},
            "max_output_gb": {"type": ["number", "null"], "default": 25.0, "description": "Fail if projected uncompressed GenBank output exceeds this many GB (0 disables)"},
            "gene_call": {"enum": ["all", "unannotated", null], "description": GENE_CALL_HELP}
        }
    })
}

fn deduplicate_schema() -> serde_json::Value {
    use serde_json::json;
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "deduplicate_genbank config",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "input": {"type": "array", "items": {"type": "string"}, "description": "GenBank/FASTA inputs"},
            "output": {"type": ["string", "null"], "description": "Output path (stdout if omitted)"},
            "log": {"type": ["string", "null"]},
            "fasta_type": {"enum": ["protein", "nucleotide", null]},
            "algorithm": {"enum": ["hash", "cd-hit", "cd-hit-est", "diamond", "usearch", null], "default": "cd-hit", "description": "Clustering algorithm (default cd-hit; cd-hit switches to cd-hit-est for non-protein input)"},
            "bin_path": {"type": ["string", "null"], "description": "Path to the external clusterer binary (else $DOMAINATOREDUX_CDHIT / $DOMAINATOREDUX_DIAMOND / $DOMAINATOREDUX_USEARCH, then PATH)"},
            "id": {"type": ["number", "null"], "description": "Identity threshold 0-1 (must be 1 for hash)"},
            "prefix_count": {"type": "boolean", "default": false},
            "suffix_count": {"type": "boolean", "default": false},
            "cpu": {"type": ["integer", "null"], "description": "Threads (0 = all cores)"},
            "params": {"type": ["string", "null"], "description": "Extra clusterer args as an inner JSON dict, e.g. '\"-s\":0.9'"},
            "fasta_out": {"type": "boolean", "default": false},
            "both_strands": {"type": "boolean", "default": false, "description": "Nucleotide only"},
            "cluster_table": {"type": ["string", "null"], "description": "Write representative/contigs TSV"},
            "cluster_sep": {"type": ["string", "null"], "description": "Separator in the cluster table (default ' ; ')"}
        }
    })
}

fn genbank_to_fasta_schema() -> serde_json::Value {
    use serde_json::json;
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "genbank_to_fasta config",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "input": {"type": "array", "items": {"type": "string"}, "description": "GenBank inputs (stdin if empty)"},
            "output": {"type": ["string", "null"], "description": "Output FASTA (stdout if omitted)"}
        }
    })
}

fn clean_sequences_schema() -> serde_json::Value {
    use serde_json::json;
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "clean_sequences config",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "input": {"type": "array", "items": {"type": "string"}},
            "output": {"type": ["string", "null"]},
            "log": {"type": ["string", "null"]},
            "fasta_type": {"enum": ["protein", "nucleotide", null]},
            "fasta_out": {"type": "boolean", "default": false},
            "clean_name": {"type": "boolean", "default": false, "description": "Newick-safe names"},
            "deduplicate_names": {"type": "boolean", "default": false},
            "strip": {"type": "boolean", "default": false, "description": "Trim non-canonical residues from ends"},
            "filter_by_aa": {"type": "boolean", "default": false, "description": "Drop sequences with non-canonical residues"}
        }
    })
}

fn gene_call_schema() -> serde_json::Value {
    use serde_json::json;
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "gene_call config",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "input": {"type": "array", "items": {"type": "string"}, "description": "GenBank/FASTA nucleotide inputs"},
            "output": {"type": ["string", "null"], "description": "Output GenBank (stdout if omitted)"},
            "fasta_type": {"enum": ["protein", "nucleotide", null], "default": "nucleotide", "description": "Molecule type of FASTA inputs (protein records are passed through)"},
            "mode": {"enum": ["meta", "single", null], "default": "meta", "description": "meta: pre-trained models chosen per contig (metagenomes, plasmids, phages, short contigs). single: train on all input records together (one complete genome)."},
            "scope": {"enum": ["unannotated", "all", null], "default": "unannotated", "description": "unannotated: call only contigs with no CDS. all: delete every CDS (and its Domainator annotations) and re-call every contig."},
            "translation_table": {"type": ["integer", "null"], "description": "Genetic code (1-6, 9-16, 21-25). Default: 11, or in meta mode the table of each contig's best model (11 or 4)."},
            "closed": {"type": "boolean", "default": false, "description": "Closed ends: no genes running off the ends of a contig"},
            "mask": {"type": "boolean", "default": false, "description": "No genes across runs of N"},
            "cpu": {"type": ["integer", "null"], "description": "Orphos threads (0 = all cores)"},
            "bin_path": {"type": ["string", "null"], "description": "Path to the orphos binary (else $DOMAINATOREDUX_ORPHOS, then PATH)"}
        }
    })
}

fn color_genbank_schema() -> serde_json::Value {
    use serde_json::json;
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "color_genbank config",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "input": {"type": "array", "items": {"type": "string"}},
            "output": {"type": ["string", "null"]},
            "contigs": {"type": "array", "items": {"type": "string"}},
            "contigs_file": {"type": ["string", "null"]},
            "color_table": {"type": ["string", "null"]},
            "color_table_out": {"type": ["string", "null"]},
            "search_hit_color": {"type": ["string", "null"]},
            "clear": {"type": "boolean", "default": false},
            "databases": {"type": "array", "items": {"type": "string"}},
            "mode": {"enum": ["domains", "cdss", "domains_and_cdss", null], "default": "cdss"},
            "domain_database": {"type": "boolean", "default": false}
        }
    })
}

fn color_table_to_legend_schema() -> serde_json::Value {
    use serde_json::json;
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "color_table_to_legend config",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "input": {"type": "array", "items": {"type": "string"}},
            "svg": {"type": ["string", "null"]},
            "title": {"type": ["string", "null"], "default": "Legend"}
        }
    })
}

fn trim_contigs_schema() -> serde_json::Value {
    use serde_json::json;
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "trim_contigs config",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "input": {"type": "array", "items": {"type": "string"}},
            "output": {"type": ["string", "null"]},
            "kb_both": {"type": ["number", "null"]},
            "kb_left": {"type": ["number", "null"]},
            "kb_right": {"type": ["number", "null"]},
            "cds_both": {"type": ["integer", "null"]},
            "cds_left": {"type": ["integer", "null"]},
            "cds_right": {"type": ["integer", "null"]},
            "domains": {"type": "array", "items": {"type": "string"}},
            "domains_file": {"type": ["string", "null"]},
            "domain_expr": {"type": ["string", "null"]},
            "no_domain": {"type": "boolean", "default": false},
            "contigs": {"type": "array", "items": {"type": "string"}},
            "contigs_file": {"type": ["string", "null"]}
        }
    })
}

fn seq_dist_schema() -> serde_json::Value {
    use serde_json::json;
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "seq_dist config",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "input": {"type": ["string", "null"]},
            "reference": {"type": ["string", "null"], "description": "Defaults to input (all-vs-all)"},
            "dense": {"type": ["string", "null"], "description": "Dense HDF5 output path"},
            "dense_text": {"type": ["string", "null"], "description": "Dense TSV output path"},
            "sparse": {"type": ["string", "null"], "description": "Sparse CSR HDF5 output path"},
            "algorithm": {"enum": [
                "diamond_f","diamond_d","diamond_mids","diamond_s","diamond_mors","diamond_vs",
                "diamond_us","hmmer","hmmer_compare"
            ], "default": "diamond_us"},
            "mode": {"enum": [
                "none","score","bool","norm_score","row_norm_score","score_dist","efi_score","efi_score_dist"
            ], "default": "score", "description": "Matrix value transform (mst_knn requires score/bool/efi_score)"},
            "lb": {"type": "number", "default": 0.0},
            "k": {"type": ["integer", "null"]},
            "mst_knn": {"type": ["integer", "null"], "description": "MST + OR-symmetric kNN; integer >= 0 (0 reduces to the MST alone). Mutually exclusive with knn"},
            "params": {"type": ["string", "null"], "description": "Extra diamond blastp options as a json dict body without the outer braces, e.g. '\"-c\":4'"},
            "knn": {"type": ["integer", "null"], "description": "OR-symmetric kNN without the spanning tree; integer >= 1. Unlike mst_knn this does not preserve connected components"},
            "cpu": {"type": "integer", "default": 0},
            "max_output_gb": {"type": ["number", "null"], "default": 25.0, "description": "Fail if a projected matrix output exceeds this many GB (0 disables)"}
        }
    })
}

fn matrix_report_schema() -> serde_json::Value {
    use serde_json::json;
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "matrix_report config",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "input": {"type": ["string", "null"], "description": "Input matrix (HDF5 or dense TSV)"},
            "output": {"type": ["string", "null"], "description": "Text report output path"},
            "json": {"type": ["string", "null"], "description": "Agent-friendly JSON report output path"},
            "merge_impact_metric": {"enum": ["min_child", "product"], "default": "min_child"},
            "max_merge_events": {"type": ["integer", "null"], "default": 500}
        }
    })
}

fn build_ssn_viewer_schema() -> serde_json::Value {
    use serde_json::json;
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "build_ssn_viewer config",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "input": {"type": ["string", "null"], "description": "Symmetric matrix (HDF5 or dense TSV)"},
            "output": {"type": ["string", "null"], "description": "Gzip-JSON .dsnv bundle output"},
            "html": {"type": ["string", "null"], "description": "Standalone HTML viewer output"},
            "embed_data": {"type": "boolean", "default": false, "description": "Embed the bundle into the HTML for autoload"},
            "name": {"type": ["string", "null"]},
            "metadata": {"type": "array", "items": {"type": "string"}},
            "subset": {"type": "array", "items": {"type": "string"}},
            "subset_file": {"type": ["string", "null"]},
            "color_by": {"type": ["string", "null"]},
            "label_by": {"type": ["string", "null"]},
            "categorical": {"type": "array", "items": {"type": "string"}, "description": "Numeric metadata columns to color as discrete categories instead of a gradient"},
            "merge_impact_metric": {"enum": ["min_child", "product"], "default": "min_child", "description": "Metric the viewer uses for split events, recorded in the bundle as the default; switchable in the viewer"},
            "max_output_gb": {"type": ["number", "null"], "default": 25.0, "description": "Fail if the projected .dsnv bundle output exceeds this many GB (0 disables)"}
        }
    })
}

fn summary_report_schema() -> serde_json::Value {
    use serde_json::json;
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "summary_report parameters",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "partial": {"enum": ["include", "exclude", "only"], "default": "include"},
            "input": {"type": "array", "items": {"type": "string"}, "description": "Input GenBank file(s)"},
            "output": {"type": ["string", "null"], "description": "Text report output path"},
            "json": {"type": ["string", "null"], "description": "Write the full summary as one compact JSON object here; '-' for stdout"},
            "domains": {"type": "array", "items": {"type": "string"}, "description": "Focus domains to report co-occurrence for"},
            "databases": {"type": "array", "items": {"type": "string"}, "description": "Only count domains from these databases"},
            "taxonomy": {"type": "boolean", "default": false},
            "ncbi_taxonomy_path": {"type": ["string", "null"]}
        }
    })
}

fn ssn_navigator_schema() -> serde_json::Value {
    use serde_json::json;
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "ssn_navigator parameters",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "input": {"type": ["string", "null"], "description": "Input .dsnv bundle (gzip JSON, or plain JSON)"},
            "output": {"type": ["string", "null"], "description": "Write the JSON here (default: stdout)"},
            "mode": {"enum": ["overview", "thresholds", "clusters", "cluster", "node"], "default": "overview"},
            "threshold": {"type": ["string", "null"], "description": "Similarity cut: a number, or 'inf'. Omit for the floor (one cluster per connected component); higher thresholds give finer clusters"},
            "id": {"type": ["integer", "null"], "description": "Cluster id, for mode 'cluster'"},
            "node": {"type": ["string", "null"], "description": "Node id, for mode 'cluster' or 'node'"},
            "min_size": {"type": ["integer", "null"], "default": 1, "description": "Ignore clusters smaller than this (mode 'clusters')"},
            "top_n": {"type": ["integer", "null"], "default": 50, "description": "Bound every list in the output"},
            "members": {"type": "boolean", "default": false, "description": "Include the full member id list in mode 'cluster'"},
            "max_merge_events": {"type": ["integer", "null"], "default": 500, "description": "How many split events the mode 'thresholds'/'node' cut-points are drawn from; 0 offers one per split event"}
        }
    })
}

fn build_ssn_schema() -> serde_json::Value {
    use serde_json::json;
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "build_ssn config",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "input": {"type": ["string", "null"], "description": "Input matrix (HDF5 or dense TSV)"},
            "xgmml": {"type": ["string", "null"], "description": "Cytoscape XGMML output path"},
            "lb": {"type": "number", "default": 0.0, "description": "Exclude edges with weight <= lb"},
            "metadata": {"type": "array", "items": {"type": "string"}, "description": "TSV node metadata files"},
            "subset": {"type": "array", "items": {"type": "string"}},
            "subset_file": {"type": ["string", "null"]},
            "color_by": {"type": ["string", "null"]},
            "color_table": {"type": ["string", "null"]},
            "color_table_out": {"type": ["string", "null"]},
            "max_color_groups": {"type": ["integer", "null"], "description": "Give distinct colors only to this many of the largest color_by groups; the rest share a neutral gray"},
            "cluster": {"type": "boolean", "default": false},
            "cluster_tsv": {"type": ["string", "null"]},
            "no_cluster_header": {"type": "boolean", "default": false},
            "mst": {"type": "boolean", "default": false},
            "mst_knn": {"type": ["integer", "null"], "description": "MST + OR-symmetric kNN; integer >= 0 (0 reduces to the MST alone). Mutually exclusive with knn"},
            "knn": {"type": ["integer", "null"], "description": "OR-symmetric kNN without the spanning tree; integer >= 1. Unlike mst_knn this does not preserve connected components"},
            "max_output_gb": {"type": ["number", "null"], "default": 25.0, "description": "Fail if the projected XGMML output exceeds this many GB (0 disables)"}
        }
    })
}

fn transform_matrix_schema() -> serde_json::Value {
    use serde_json::json;
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "transform_matrix config",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "input": {"type": ["string", "null"], "description": "Input matrix (HDF5 or dense TSV)"},
            "dense": {"type": ["string", "null"], "description": "Dense HDF5 output path"},
            "dense_text": {"type": ["string", "null"], "description": "Dense TSV output path"},
            "sparse": {"type": ["string", "null"], "description": "Sparse CSR HDF5 output path"},
            "mode": {"enum": [
                "none","score","bool","norm_score","row_norm_score","score_dist","efi_score","efi_score_dist"
            ], "default": "none"},
            "input_type": {"type": ["string", "null"], "description": "Override the input matrix data type"},
            "lb": {"type": ["number", "null"], "description": "Zero out values <= lb after the transform"},
            "mst_knn": {"type": ["integer", "null"], "description": "MST + OR-symmetric kNN; integer >= 0 (0 reduces to the MST alone). Mutually exclusive with knn"},
            "knn": {"type": ["integer", "null"], "description": "OR-symmetric kNN without the spanning tree; integer >= 1. Unlike mst_knn this does not preserve connected components"},
            "max_output_gb": {"type": ["number", "null"], "default": 25.0, "description": "Fail if a projected matrix output exceeds this many GB (0 disables)"}
        }
    })
}

fn compare_contigs_schema() -> serde_json::Value {
    use serde_json::json;
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "compare_contigs config",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "input": {"type": "array", "items": {"type": "string"}, "description": "Annotated GenBank files"},
            "dense": {"type": ["string", "null"]},
            "dense_text": {"type": ["string", "null"]},
            "sparse": {"type": ["string", "null"]},
            "ji": {"type": "number", "default": 0.5, "description": "Jaccard-index weight"},
            "ai": {"type": "number", "default": 0.5, "description": "Adjacency-index weight"},
            "databases": {"type": "array", "items": {"type": "string"}},
            "k": {"type": ["integer", "null"]},
            "lb": {"type": "number", "default": 0.0},
            "mst_knn": {"type": ["integer", "null"], "description": "MST + OR-symmetric kNN; integer >= 0 (0 reduces to the MST alone). Mutually exclusive with knn"},
            "knn": {"type": ["integer", "null"], "description": "OR-symmetric kNN without the spanning tree; integer >= 1. Unlike mst_knn this does not preserve connected components"},
            "cpu": {"type": "integer", "default": 0},
            "max_output_gb": {"type": ["number", "null"], "default": 25.0, "description": "Fail if a projected matrix output exceeds this many GB (0 disables)"}
        }
    })
}

fn hmmer_compare_schema() -> serde_json::Value {
    use serde_json::json;
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "hmmer_compare config",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "input": {"type": "array", "items": {"type": "string"}, "description": "Query .hmm files (optionally compressed)"},
            "reference": {"type": "array", "items": {"type": "string"}, "description": "Reference .hmm files (optionally compressed)"},
            "output": {"type": ["string", "null"], "description": "TSV output (stdout if omitted)"},
            "score_cutoff": {"type": "number", "default": 0.0},
            "k": {"type": ["integer", "null"]},
            "alignments": {"type": "boolean", "default": false},
            "cpu": {"type": "integer", "default": 0},
            "max_output_gb": {"type": ["number", "null"], "default": 25.0, "description": "Fail if the projected TSV output exceeds this many GB (0 disables)"}
        }
    })
}

fn hmmer_build_schema() -> serde_json::Value {
    use serde_json::json;
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "hmmer_build config",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "input": {"type": ["string", "null"], "description": "Stockholm MSA (stdin if omitted)"},
            "output": {"type": ["string", "null"], "description": "Output .hmm (stdout if omitted); '.gz' writes plain gzip, '.bgz' is refused (HMMER cannot read BGZF)"},
            "name": {"type": ["string", "null"], "description": "Profile NAME (required)"},
            "acc": {"type": ["string", "null"]},
            "desc": {"type": ["string", "null"]},
            "alphabet": {"enum": ["amino", "dna", "rna", null], "description": "Inferred when omitted"},
            "window_length": {"type": ["integer", "null"], "description": "Max expected hit length (the profile's MAXL field), used by nhmmer for windowed E-values. Nucleotide alphabets only"},
            "window_beta": {"type": ["number", "null"], "description": "Tail mass from which to derive the window length. Nucleotide alphabets only; only one of window_length and window_beta may be given (deriving MAXL from a tail mass is not yet supported, so this is refused rather than silently ignored)"}
        }
    })
}

fn hmmer_search_schema() -> serde_json::Value {
    use serde_json::json;
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "hmmer_search config",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "input": {"type": "array", "items": {"type": "string"}, "description": "Query .hmm file(s) (the set being filtered; optionally compressed)"},
            "reference": {"type": "array", "items": {"type": "string"}, "description": "Reference .hmm file(s) (the filter criterion; optionally compressed)"},
            "output": {"type": ["string", "null"], "description": "Output .hmm (stdout if omitted); '.gz' writes plain gzip, '.bgz' is refused (HMMER cannot read BGZF)"},
            "score_cutoff": {"type": ["number", "null"], "default": 10.0, "description": "Keep queries whose best reference score is >= this"},
            "max_hits": {"type": ["integer", "null"], "description": "Keep top-N queries by best score"},
            "cpu": {"type": ["integer", "null"], "description": "Threads (0 = all cores)"},
            "max_output_gb": {"type": ["number", "null"], "description": "Fail if projected HMM output exceeds this many GB (0 disables)"}
        }
    })
}

fn hmmer_select_schema() -> serde_json::Value {
    use serde_json::json;
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "hmmer_select config",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "input": {"type": "array", "items": {"type": "string"}, "description": "Input .hmm files (optionally gzip/BGZF-compressed)"},
            "output": {"type": ["string", "null"], "description": "HMM output (stdout if omitted); '.gz' writes plain gzip, '.bgz' is refused (HMMER cannot read BGZF)"},
            "regex": {"type": "array", "items": {"type": "string"}, "description": "Regex pattern(s) to match"},
            "exact": {"type": "array", "items": {"type": "string"}, "description": "Exact string(s) to match"},
            "contains": {"type": "array", "items": {"type": "string"}, "description": "Substring(s) to match"},
            "field": {"enum": ["name", "desc", "acc", "all", null], "default": "all", "description": "Which field(s) to search (default all)"},
            "case_sensitive": {"type": "boolean", "default": false},
            "alphabet": {"enum": ["amino", "dna", "rna", null], "description": "Only read input files whose profiles use this alphabet"}
        }
    })
}

fn format_db_schema() -> serde_json::Value {
    use serde_json::json;
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "format_db config",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "input": {"type": "array", "items": {"type": "string"}, "description": "GenBank/FASTA database file(s)"},
            "shards": {"type": ["integer", "null"], "description": "Split each input into this many shards"},
            "compress": {"type": "boolean", "default": false, "description": "Write BGZF output"},
            "index": {"type": "boolean", "default": false, "description": "Write a .didx index per output"},
            "output_dir": {"type": ["string", "null"]},
            "name": {"type": ["string", "null"], "description": "Base name for the output database (single input only)"},
            "level": {"type": "integer", "default": 6, "description": "BGZF compression level (1-12)"},
            "force": {"type": "boolean", "default": false},
            "log": {"type": ["string", "null"], "description": "Write the run log here instead of stderr"}
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reject_non_finite_names_the_key() {
        #[derive(Serialize)]
        struct Inner {
            ph: f64,
        }
        #[derive(Serialize)]
        struct Cfg {
            kb_range: Option<f64>,
            evalues: Vec<f64>,
            nested: Inner,
        }
        let ok = Cfg {
            kb_range: Some(1.5),
            evalues: vec![0.0, 1e-300],
            nested: Inner { ph: 7.0 },
        };
        assert!(reject_non_finite(&ok).is_ok());
        let unset = Cfg {
            kb_range: None,
            ..ok
        };
        assert!(reject_non_finite(&unset).is_ok());
        for (cfg, key) in [
            (
                Cfg {
                    kb_range: Some(f64::NAN),
                    evalues: vec![],
                    nested: Inner { ph: 7.0 },
                },
                "`kb_range`",
            ),
            (
                Cfg {
                    kb_range: None,
                    evalues: vec![1.0, f64::INFINITY],
                    nested: Inner { ph: 7.0 },
                },
                "`evalues.1`",
            ),
            (
                Cfg {
                    kb_range: None,
                    evalues: vec![],
                    nested: Inner {
                        ph: f64::NEG_INFINITY,
                    },
                },
                "`nested.ph`",
            ),
        ] {
            let err = reject_non_finite(&cfg).unwrap_err().to_string();
            assert!(err.starts_with(key), "{err}");
        }
    }

    /// Every tool, paired with the config struct `resolve.rs` deserializes into.
    ///
    /// Adding a tool here is the point: a new tool with no entry is a tool whose
    /// schema nothing checks.
    macro_rules! tools {
        ($($tool:literal => $ty:ty),* $(,)?) => {
            vec![$(($tool, serde_json::to_value(<$ty>::default()).unwrap())),*]
        };
    }

    fn every_tool() -> Vec<(&'static str, serde_json::Value)> {
        tools![
            "domainate" => DomainateConfig,
            "domain_search" => DomainSearchConfig,
            "deduplicate_genbank" => DeduplicateConfig,
            "genbank_to_fasta" => GenbankToFastaConfig,
            "clean_sequences" => CleanSequencesConfig,
            "gene_call" => GeneCallConfig,
            "color_genbank" => ColorGenbankConfig,
            "color_table_to_legend" => ColorTableToLegendConfig,
            "trim_contigs" => TrimContigsConfig,
            "seq_dist" => SeqDistConfig,
            "compare_contigs" => CompareContigsConfig,
            "transform_matrix" => TransformMatrixConfig,
            "build_ssn" => BuildSsnConfig,
            "build_ssn_viewer" => BuildSsnViewerConfig,
            "matrix_report" => MatrixReportConfig,
            "ssn_navigator" => SsnNavigatorConfig,
            "summary_report" => SummaryReportConfig,
            "hmmer_compare" => HmmerCompareConfig,
            "hmmer_select" => HmmerSelectConfig,
            "hmmer_build" => HmmerBuildConfig,
            "hmmer_search" => HmmerSearchConfig,
            "format_db" => FormatDbConfig,
        ]
    }

    fn schema_properties(tool: &str) -> Vec<String> {
        let parsed = legacy_schema(tool).unwrap_or_else(|e| panic!("no schema for {tool}: {e}"));
        parsed["properties"]
            .as_object()
            .unwrap_or_else(|| panic!("{tool} schema has no properties object"))
            .keys()
            .cloned()
            .collect()
    }

    fn config_fields(default: &serde_json::Value) -> Vec<String> {
        default.as_object().unwrap().keys().cloned().collect()
    }

    /// An unmigrated tool's old config schema is what the GUI renders its dialog from
    /// (converted by [`tool_manifest`]), while the config struct is what actually gets
    /// deserialized. The two are written by hand in different places, so they drift
    /// silently and in both directions:
    ///
    /// * a field missing from the schema is a flag no GUI dialog can discover --
    ///   which is how `hmmer_build --window_length` and `format_db --log` went
    ///   missing;
    /// * a property the struct does not have is worse, because every config struct
    ///   is `deny_unknown_fields`, so a config written to match the schema is
    ///   *rejected*.
    #[test]
    fn every_schema_matches_its_config_struct() {
        let mut problems = Vec::new();
        for (tool, default) in every_tool() {
            let in_schema = schema_properties(tool);
            let in_struct = config_fields(&default);
            let missing: Vec<_> = in_struct
                .iter()
                .filter(|f| !in_schema.contains(f))
                .cloned()
                .collect();
            let extra: Vec<_> = in_schema
                .iter()
                .filter(|p| !in_struct.contains(p))
                .cloned()
                .collect();
            if !missing.is_empty() {
                problems.push(format!(
                    "{tool}: in the struct but not the schema: {missing:?}"
                ));
            }
            if !extra.is_empty() {
                problems.push(format!(
                    "{tool}: in the schema but not the struct (a schema-conforming \
                     config would be REJECTED): {extra:?}"
                ));
            }
        }
        assert!(
            problems.is_empty(),
            "schema/config drift:\n  {}",
            problems.join("\n  ")
        );
    }

    /// A tool the CLI accepts but `every_tool()` forgets is a tool this file stops
    /// guarding, so cross-check against the dispatcher's own list of names.
    #[test]
    fn the_tool_list_covers_every_unmigrated_tool() {
        let listed: Vec<&str> = every_tool().iter().map(|(t, _)| *t).collect();
        for info in registry::TOOLS.iter().filter(|t| t.in_manifest) {
            assert_eq!(
                listed.contains(&info.name),
                !info.migrated,
                "{}: list the unmigrated tools, and only them",
                info.name
            );
            assert_eq!(
                legacy_schema(info.name).is_ok(),
                !info.migrated,
                "{}",
                info.name
            );
            let m = tool_manifest(info.name).unwrap();
            assert!(
                !m["parameters"].as_array().unwrap().is_empty(),
                "{}",
                info.name
            );
        }
        assert!(tool_manifest("not-a-tool").is_err());
    }

    /// The GUI reads an unmigrated tool's dialog from its old schema, converted.
    #[test]
    fn a_legacy_schema_converts_to_manifest_parameters() {
        let m = tool_manifest("format_db").unwrap();
        let param = |name: &str| {
            m["parameters"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["name"] == name)
                .unwrap_or_else(|| panic!("no {name}"))
                .clone()
        };
        assert_eq!(param("input")["multiple"], true);
        assert_eq!(param("input")["role"], "input");
        assert_eq!(param("force")["kind"], "flag");
    }

    /// A field `merge_cli` forgets is a CLI flag that silently does nothing: every
    /// flag set on the command line must survive a merge into an empty config.
    #[test]
    fn gene_call_merge_keeps_every_flag() {
        let cli = GeneCallConfig {
            input: vec!["a.fasta".into()],
            output: Some("out.gb".into()),
            fasta_type: Some("protein".into()),
            mode: Some("single".into()),
            scope: Some("all".into()),
            translation_table: Some(4),
            closed: true,
            mask: true,
            cpu: Some(2),
            bin_path: Some("/opt/orphos".into()),
        };
        let mut merged = GeneCallConfig::default();
        merged.merge_cli(cli.clone());
        assert_eq!(
            serde_json::to_value(&merged).unwrap(),
            serde_json::to_value(&cli).unwrap()
        );
        for (tool, mut cfg) in [
            (
                "domainate",
                serde_json::to_value(DomainateConfig::default()).unwrap(),
            ),
            (
                "domain_search",
                serde_json::to_value(DomainSearchConfig::default()).unwrap(),
            ),
        ] {
            cfg["gene_call"] = "all".into();
            let merged = match tool {
                "domainate" => {
                    let mut m = DomainateConfig::default();
                    m.merge_cli(serde_json::from_value(cfg).unwrap());
                    m.gene_call
                }
                _ => {
                    let mut m = DomainSearchConfig::default();
                    m.merge_cli(serde_json::from_value(cfg).unwrap());
                    m.gene_call
                }
            };
            assert_eq!(merged.as_deref(), Some("all"), "{tool}");
        }
    }
}
