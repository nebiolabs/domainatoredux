//! Config-file (`--config`) and `--print_config` support for the
//! selection/reporting tools, plus `schema` output — a native Rust JSON/YAML
//! parameter schema (not byte-compatible with the Python jsonargparse files, but
//! serving the same purpose: declarative, agent-friendly, quoting-free params).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub mod resolve;

use domainator_core::tools::enum_report::{
    append_analysis, feature_count_analysis, motif_count_analysis, named_domain_count_analysis,
    net_charge_analysis, qualifier_analysis, repeat_count_analysis, static_analysis, taxid_analysis,
    taxname_analysis, Analysis, By,
};

/// Load a JSON or YAML config file into `T`, choosing the parser by extension
/// (`.json` -> JSON, otherwise YAML, which is a superset of JSON).
pub fn load_config<T: for<'de> Deserialize<'de>>(path: &str) -> Result<T> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading config {path}"))?;
    if path.rsplit('.').next().map(|e| e.eq_ignore_ascii_case("json")).unwrap_or(false) {
        serde_json::from_str(&text).with_context(|| format!("parsing JSON config {path}"))
    } else {
        serde_yaml::from_str(&text).with_context(|| format!("parsing YAML config {path}"))
    }
}

/// Serialize resolved params as YAML for `--print_config`.
pub fn print_config<T: Serialize>(cfg: &T) -> Result<String> {
    serde_yaml::to_string(cfg).context("serializing config")
}

// ---------------------------------------------------------------------------
// enum_report
// ---------------------------------------------------------------------------

/// One ordered `enum_report` column (a declarative form of the CLI's column
/// flags). Serialized as `{"type": "<name>", ...args}`.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ColumnSpec {
    Domains,
    DomainDescriptions,
    Sequence,
    Start,
    End,
    Strand,
    Length,
    Topology,
    Architecture,
    ArchitectureDetailed,
    Score,
    Identity,
    Definition,
    Filename,
    CdsCount,
    DomainCount,
    DomainSearch,
    TaxidLineage,
    TaxnameLineage,
    RankLineage,
    IsoelectricPoint,
    StartsWith,
    MolecularWeight,
    Taxid { ranks: Vec<String> },
    Taxname { ranks: Vec<String> },
    Qualifier { feature: String, qualifier: String },
    FeatureCount { feature: String },
    Append { name: String, value_type: String, value: String },
    MotifCount { pattern: String },
    NetCharge { ph: String },
    RepeatCount { min_length: String },
    NamedDomainCount { name: String },
}

impl ColumnSpec {
    /// The static-column flag name for the no-argument variants (else `None`).
    fn static_name(&self) -> Option<&'static str> {
        use ColumnSpec::*;
        Some(match self {
            Domains => "domains",
            DomainDescriptions => "domain_descriptions",
            Sequence => "sequence",
            Start => "start",
            End => "end",
            Strand => "strand",
            Length => "length",
            Topology => "topology",
            Architecture => "architecture",
            ArchitectureDetailed => "architecture_detailed",
            Score => "score",
            Identity => "identity",
            Definition => "definition",
            Filename => "filename",
            CdsCount => "cds_count",
            DomainCount => "domain_count",
            DomainSearch => "domain_search",
            TaxidLineage => "taxid_lineage",
            TaxnameLineage => "taxname_lineage",
            RankLineage => "rank_lineage",
            IsoelectricPoint => "isoelectric_point",
            StartsWith => "starts_with",
            MolecularWeight => "molecular_weight",
            _ => return None,
        })
    }

    /// Reverse of [`Self::static_name`]: the no-argument column for a flag name.
    pub fn from_static_flag(name: &str) -> Option<ColumnSpec> {
        use ColumnSpec::*;
        Some(match name {
            "domains" => Domains,
            "domain_descriptions" => DomainDescriptions,
            "sequence" => Sequence,
            "start" => Start,
            "end" => End,
            "strand" => Strand,
            "length" => Length,
            "topology" => Topology,
            "architecture" => Architecture,
            "architecture_detailed" => ArchitectureDetailed,
            "score" => Score,
            "identity" => Identity,
            "definition" => Definition,
            "filename" => Filename,
            "cds_count" => CdsCount,
            "domain_count" => DomainCount,
            "domain_search" => DomainSearch,
            "taxid_lineage" => TaxidLineage,
            "taxname_lineage" => TaxnameLineage,
            "rank_lineage" => RankLineage,
            "isoelectric_point" => IsoelectricPoint,
            "starts_with" => StartsWith,
            "molecular_weight" => MolecularWeight,
            _ => return None,
        })
    }

    /// Build the core [`Analysis`] for this column.
    pub fn to_analysis(&self) -> Result<Analysis> {
        if let Some(name) = self.static_name() {
            return static_analysis(name)
                .with_context(|| format!("unknown enum_report column: {name}"));
        }
        Ok(match self {
            ColumnSpec::Taxid { ranks } => taxid_analysis(ranks.clone()),
            ColumnSpec::Taxname { ranks } => taxname_analysis(ranks.clone()),
            ColumnSpec::Qualifier { feature, qualifier } => {
                qualifier_analysis(feature.clone(), qualifier.clone())
            }
            ColumnSpec::FeatureCount { feature } => feature_count_analysis(feature.clone()),
            ColumnSpec::Append { name, value_type, value } => {
                append_analysis(name.clone(), value_type, value).map_err(|e| anyhow::anyhow!(e))?
            }
            ColumnSpec::MotifCount { pattern } => motif_count_analysis(pattern.clone()),
            ColumnSpec::NetCharge { ph } => net_charge_analysis(ph).map_err(|e| anyhow::anyhow!(e))?,
            ColumnSpec::RepeatCount { min_length } => {
                repeat_count_analysis(min_length).map_err(|e| anyhow::anyhow!(e))?
            }
            ColumnSpec::NamedDomainCount { name } => named_domain_count_analysis(name.clone()),
            _ => unreachable!("static variants handled above"),
        })
    }
}

/// The full `enum_report` parameter set (config-file shape).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, deny_unknown_fields)]
pub struct EnumConfig {
    pub input: Vec<String>,
    pub output: Option<String>,
    /// `contig` (default) | `cds` | `domain`.
    pub by: Option<String>,
    pub databases: Vec<String>,
    pub contigs: Vec<String>,
    pub contigs_file: Option<String>,
    pub column_names: Option<Vec<String>>,
    pub ncbi_taxonomy_path: Option<String>,
    /// Ordered output columns.
    pub columns: Vec<ColumnSpec>,
    /// Write streaming NDJSON here (`-` = stdout). Sits alongside `output`, which
    /// stays TSV, so both can be produced in one run.
    pub json: Option<String>,
}

impl EnumConfig {
    pub fn by_enum(&self) -> Result<By> {
        Ok(match self.by.as_deref().unwrap_or("contig") {
            "contig" => By::Contig,
            "cds" => By::Cds,
            "domain" => By::Domain,
            other => anyhow::bail!("`by` must be contig/cds/domain, got {other:?}"),
        })
    }

    /// Merge CLI-provided values over this (file) config: scalars from `cli`
    /// override when set; list values override when non-empty; `columns` from the
    /// CLI are appended (argparse-style).
    pub fn merge_cli(&mut self, cli: EnumConfig) {
        if !cli.input.is_empty() {
            self.input = cli.input;
        }
        if cli.output.is_some() {
            self.output = cli.output;
        }
        if cli.by.is_some() {
            self.by = cli.by;
        }
        if !cli.databases.is_empty() {
            self.databases = cli.databases;
        }
        if !cli.contigs.is_empty() {
            self.contigs = cli.contigs;
        }
        if cli.contigs_file.is_some() {
            self.contigs_file = cli.contigs_file;
        }
        if cli.column_names.is_some() {
            self.column_names = cli.column_names;
        }
        if cli.ncbi_taxonomy_path.is_some() {
            self.ncbi_taxonomy_path = cli.ncbi_taxonomy_path;
        }
        self.columns.extend(cli.columns);
        if cli.json.is_some() {
            self.json = cli.json;
        }
    }
}

// ---------------------------------------------------------------------------
// select_by_contig / select_by_cds
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

/// `select_by_contig` parameters (config-file shape). Defaulted scalars are
/// `Option` so a config value is not clobbered by a CLI default.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, deny_unknown_fields)]
pub struct SelectByContigConfig {
    pub input: Vec<String>,
    pub output: Option<String>,
    pub contigs: Vec<String>,
    pub contigs_file: Option<String>,
    pub contigs_regex: Option<String>,
    pub definition_regex: Option<String>,
    pub sequence_regex: Option<String>,
    pub domains: Vec<String>,
    pub domains_file: Option<String>,
    pub domain_expr: Option<String>,
    pub domain_type: Option<String>,
    pub databases: Vec<String>,
    pub unannotated: bool,
    pub evalue: Option<f64>,
    pub fasta_out: bool,
    pub include_taxids: Vec<i64>,
    pub exclude_taxids: Vec<i64>,
    pub taxonomy_expr: Option<String>,
    pub ncbi_taxonomy_path: Option<String>,
    pub first: Option<usize>,
    pub length_lb: Option<i64>,
    pub length_ub: Option<i64>,
    pub cds_count_lb: Option<i64>,
    pub cds_count_ub: Option<i64>,
    pub search_score_lb: Option<f64>,
    pub search_score_ub: Option<f64>,
    pub invert: bool,
}

impl SelectByContigConfig {
    pub fn merge_cli(&mut self, cli: SelectByContigConfig) {
        merge_vec!(self, cli, input);
        merge_scalar!(self, cli, output);
        merge_vec!(self, cli, contigs);
        merge_scalar!(self, cli, contigs_file);
        merge_scalar!(self, cli, contigs_regex);
        merge_scalar!(self, cli, definition_regex);
        merge_scalar!(self, cli, sequence_regex);
        merge_vec!(self, cli, domains);
        merge_scalar!(self, cli, domains_file);
        merge_scalar!(self, cli, domain_expr);
        merge_scalar!(self, cli, domain_type);
        merge_vec!(self, cli, databases);
        self.unannotated |= cli.unannotated;
        merge_scalar!(self, cli, evalue);
        self.fasta_out |= cli.fasta_out;
        merge_vec!(self, cli, include_taxids);
        merge_vec!(self, cli, exclude_taxids);
        merge_scalar!(self, cli, taxonomy_expr);
        merge_scalar!(self, cli, ncbi_taxonomy_path);
        merge_scalar!(self, cli, first);
        merge_scalar!(self, cli, length_lb);
        merge_scalar!(self, cli, length_ub);
        merge_scalar!(self, cli, cds_count_lb);
        merge_scalar!(self, cli, cds_count_ub);
        merge_scalar!(self, cli, search_score_lb);
        merge_scalar!(self, cli, search_score_ub);
        self.invert |= cli.invert;
    }
}

/// `select_by_cds` parameters (config-file shape).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, deny_unknown_fields)]
pub struct SelectByCdsConfig {
    pub input: Vec<String>,
    pub output: Option<String>,
    pub kb_range: Option<f64>,
    pub kb_up: Option<f64>,
    pub kb_down: Option<f64>,
    pub cds_range: Option<i64>,
    pub cds_up: Option<i64>,
    pub cds_down: Option<i64>,
    pub whole_contig: bool,
    pub contigs: Vec<String>,
    pub contigs_file: Option<String>,
    pub cds: Vec<String>,
    pub cds_file: Option<String>,
    pub domains: Vec<String>,
    pub domains_file: Option<String>,
    pub domain_expr: Option<String>,
    pub databases: Vec<String>,
    pub strand: Option<String>,
    pub unannotated: bool,
    pub search_hits: bool,
    pub pad: bool,
    pub keep_direction: bool,
    pub skip_deduplicate: bool,
    pub max_region_overlap: Option<f64>,
    pub evalue: Option<f64>,
    pub invert: bool,
    pub include_nucleic_acids: bool,
}

impl SelectByCdsConfig {
    pub fn merge_cli(&mut self, cli: SelectByCdsConfig) {
        merge_vec!(self, cli, input);
        merge_scalar!(self, cli, output);
        merge_scalar!(self, cli, kb_range);
        merge_scalar!(self, cli, kb_up);
        merge_scalar!(self, cli, kb_down);
        merge_scalar!(self, cli, cds_range);
        merge_scalar!(self, cli, cds_up);
        merge_scalar!(self, cli, cds_down);
        self.whole_contig |= cli.whole_contig;
        merge_vec!(self, cli, contigs);
        merge_scalar!(self, cli, contigs_file);
        merge_vec!(self, cli, cds);
        merge_scalar!(self, cli, cds_file);
        merge_vec!(self, cli, domains);
        merge_scalar!(self, cli, domains_file);
        merge_scalar!(self, cli, domain_expr);
        merge_vec!(self, cli, databases);
        merge_scalar!(self, cli, strand);
        self.unannotated |= cli.unannotated;
        self.search_hits |= cli.search_hits;
        self.pad |= cli.pad;
        self.keep_direction |= cli.keep_direction;
        self.skip_deduplicate |= cli.skip_deduplicate;
        merge_scalar!(self, cli, max_region_overlap);
        merge_scalar!(self, cli, evalue);
        self.invert |= cli.invert;
        self.include_nucleic_acids |= cli.include_nucleic_acids;
    }
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
    pub max_region_overlap: Option<f64>,
    pub decoys: Vec<String>,
    pub decoys_file: Option<String>,
    pub include_taxids: Vec<i64>,
    pub exclude_taxids: Vec<i64>,
    pub taxonomy_expr: Option<String>,
    pub ncbi_taxonomy_path: Option<String>,
    pub max_output_gb: Option<f64>,
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
        merge_scalar!(self, cli, max_region_overlap);
        merge_vec!(self, cli, decoys);
        merge_scalar!(self, cli, decoys_file);
        merge_vec!(self, cli, include_taxids);
        merge_vec!(self, cli, exclude_taxids);
        merge_scalar!(self, cli, taxonomy_expr);
        merge_scalar!(self, cli, ncbi_taxonomy_path);
        merge_scalar!(self, cli, max_output_gb);
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

/// `sort_contigs` parameters.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, deny_unknown_fields)]
pub struct SortContigsConfig {
    pub input: Vec<String>,
    pub output: Option<String>,
    pub max_hits: Option<usize>,
    pub domains: Vec<String>,
    pub databases: Vec<String>,
}

impl SortContigsConfig {
    pub fn merge_cli(&mut self, cli: SortContigsConfig) {
        merge_vec!(self, cli, input);
        merge_scalar!(self, cli, output);
        merge_scalar!(self, cli, max_hits);
        merge_vec!(self, cli, domains);
        merge_vec!(self, cli, databases);
    }
}

/// `select_by_coord` parameters.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, deny_unknown_fields)]
pub struct SelectByCoordConfig {
    pub input: Vec<String>,
    pub output: Option<String>,
    pub coords: Option<String>,
}

impl SelectByCoordConfig {
    pub fn merge_cli(&mut self, cli: SelectByCoordConfig) {
        merge_vec!(self, cli, input);
        merge_scalar!(self, cli, output);
        merge_scalar!(self, cli, coords);
    }
}

/// `filter_domains` parameters.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, deny_unknown_fields)]
pub struct FilterDomainsConfig {
    pub input: Vec<String>,
    pub output: Option<String>,
    pub evalue: Option<f64>,
    pub max_overlap: Option<f64>,
    pub domains: Vec<String>,
    pub domains_file: Option<String>,
    pub databases_keep: Vec<String>,
    pub databases_filter: Vec<String>,
    pub annotation_type: Option<String>,
    pub contigs: Vec<String>,
    pub contigs_file: Option<String>,
}

impl FilterDomainsConfig {
    pub fn merge_cli(&mut self, cli: FilterDomainsConfig) {
        merge_vec!(self, cli, input);
        merge_scalar!(self, cli, output);
        merge_scalar!(self, cli, evalue);
        merge_scalar!(self, cli, max_overlap);
        merge_vec!(self, cli, domains);
        merge_scalar!(self, cli, domains_file);
        merge_vec!(self, cli, databases_keep);
        merge_vec!(self, cli, databases_filter);
        merge_scalar!(self, cli, annotation_type);
        merge_vec!(self, cli, contigs);
        merge_scalar!(self, cli, contigs_file);
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

/// `extract_domains` parameters.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ExtractDomainsConfig {
    pub input: Vec<String>,
    pub output: Option<String>,
    pub evalue: Option<f64>,
    pub score: Option<f64>,
    pub domains: Vec<String>,
    pub domains_file: Option<String>,
    pub databases: Vec<String>,
    pub search_hits: bool,
    pub pad_up: Option<i64>,
    pub pad_down: Option<i64>,
    pub combine: bool,
    pub splice: bool,
    pub keep_direction: bool,
    pub keep_name: bool,
    pub fasta_out: bool,
    pub contigs: Vec<String>,
    pub contigs_file: Option<String>,
}

impl ExtractDomainsConfig {
    pub fn merge_cli(&mut self, cli: ExtractDomainsConfig) {
        merge_vec!(self, cli, input);
        merge_scalar!(self, cli, output);
        merge_scalar!(self, cli, evalue);
        merge_scalar!(self, cli, score);
        merge_vec!(self, cli, domains);
        merge_scalar!(self, cli, domains_file);
        merge_vec!(self, cli, databases);
        self.search_hits |= cli.search_hits;
        merge_scalar!(self, cli, pad_up);
        merge_scalar!(self, cli, pad_down);
        self.combine |= cli.combine;
        self.splice |= cli.splice;
        self.keep_direction |= cli.keep_direction;
        self.keep_name |= cli.keep_name;
        self.fasta_out |= cli.fasta_out;
        merge_vec!(self, cli, contigs);
        merge_scalar!(self, cli, contigs_file);
    }
}

/// `extract_peptides` parameters.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ExtractPeptidesConfig {
    pub input: Vec<String>,
    pub output: Option<String>,
    pub domains: Vec<String>,
    pub domains_file: Option<String>,
    pub databases: Vec<String>,
    pub unannotated: bool,
    pub search_hits: bool,
    pub strand: Option<String>,
    pub invert: bool,
    pub cdss: Vec<String>,
    pub cdss_file: Option<String>,
    pub keep_name: bool,
    pub name_field: Option<String>,
    pub evalue: Option<f64>,
    pub fasta_out: bool,
    pub contigs: Vec<String>,
    pub contigs_file: Option<String>,
}

impl ExtractPeptidesConfig {
    pub fn merge_cli(&mut self, cli: ExtractPeptidesConfig) {
        merge_vec!(self, cli, input);
        merge_scalar!(self, cli, output);
        merge_vec!(self, cli, domains);
        merge_scalar!(self, cli, domains_file);
        merge_vec!(self, cli, databases);
        self.unannotated |= cli.unannotated;
        self.search_hits |= cli.search_hits;
        merge_scalar!(self, cli, strand);
        self.invert |= cli.invert;
        merge_vec!(self, cli, cdss);
        merge_scalar!(self, cli, cdss_file);
        self.keep_name |= cli.keep_name;
        merge_scalar!(self, cli, name_field);
        merge_scalar!(self, cli, evalue);
        self.fasta_out |= cli.fasta_out;
        merge_vec!(self, cli, contigs);
        merge_scalar!(self, cli, contigs_file);
    }
}

/// `extract_unannotated` parameters.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ExtractUnannotatedConfig {
    pub input: Vec<String>,
    pub output: Option<String>,
    pub evalue: Option<f64>,
    pub score: Option<f64>,
    pub databases: Vec<String>,
    pub search_hits: bool,
    pub keep_direction: bool,
    pub keep_name: bool,
    pub largest: bool,
    pub lb: Option<i64>,
    pub fasta_out: bool,
    pub contigs: Vec<String>,
    pub contigs_file: Option<String>,
}

impl ExtractUnannotatedConfig {
    pub fn merge_cli(&mut self, cli: ExtractUnannotatedConfig) {
        merge_vec!(self, cli, input);
        merge_scalar!(self, cli, output);
        merge_scalar!(self, cli, evalue);
        merge_scalar!(self, cli, score);
        merge_vec!(self, cli, databases);
        self.search_hits |= cli.search_hits;
        self.keep_direction |= cli.keep_direction;
        self.keep_name |= cli.keep_name;
        self.largest |= cli.largest;
        merge_scalar!(self, cli, lb);
        self.fasta_out |= cli.fasta_out;
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
    pub max_merge_events: Option<usize>,
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
        merge_scalar!(self, cli, max_merge_events);
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

// ---------------------------------------------------------------------------
// hmmer_report
// ---------------------------------------------------------------------------

use domainator_core::tools::hmmer_report::{
    append_analysis as hr_append, static_analysis as hr_static, Analysis as HrAnalysis,
};

/// One ordered `hmmer_report` column (declarative form of the CLI column flags).
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum HmmerReportColumn {
    Source,
    Acc,
    Desc,
    Length,
    Consensus,
    Alphabet,
    Append { name: String, value_type: String, value: String },
}

impl HmmerReportColumn {
    fn static_name(&self) -> Option<&'static str> {
        use HmmerReportColumn::*;
        Some(match self {
            Source => "source",
            Acc => "acc",
            Desc => "desc",
            Length => "length",
            Consensus => "consensus",
            Alphabet => "alphabet",
            Append { .. } => return None,
        })
    }

    /// The no-argument column for a flag name.
    pub fn from_static_flag(name: &str) -> Option<HmmerReportColumn> {
        use HmmerReportColumn::*;
        Some(match name {
            "source" => Source,
            "acc" => Acc,
            "desc" => Desc,
            "length" => Length,
            "consensus" => Consensus,
            "alphabet" => Alphabet,
            _ => return None,
        })
    }

    /// Build the core [`HrAnalysis`] for this column.
    pub fn to_analysis(&self) -> Result<HrAnalysis> {
        if let Some(name) = self.static_name() {
            return hr_static(name).with_context(|| format!("unknown hmmer_report column: {name}"));
        }
        Ok(match self {
            HmmerReportColumn::Append { name, value_type, value } => {
                hr_append(name.clone(), value_type, value).map_err(|e| anyhow::anyhow!(e))?
            }
            _ => unreachable!("static variants handled above"),
        })
    }
}

/// The full `hmmer_report` parameter set (config-file shape).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, deny_unknown_fields)]
pub struct HmmerReportConfig {
    pub input: Vec<String>,
    pub output: Option<String>,
    pub column_names: Option<Vec<String>>,
    /// Ordered output columns.
    pub columns: Vec<HmmerReportColumn>,
    /// Write streaming NDJSON here (`-` = stdout). Sits alongside `output`, which
    /// stays TSV, so both can be produced in one run.
    pub json: Option<String>,
}

impl HmmerReportConfig {
    pub fn merge_cli(&mut self, cli: HmmerReportConfig) {
        merge_vec!(self, cli, input);
        merge_scalar!(self, cli, output);
        if cli.column_names.is_some() {
            self.column_names = cli.column_names;
        }
        self.columns.extend(cli.columns);
        merge_scalar!(self, cli, json);
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

/// A pragmatic JSON Schema (draft-07) describing a tool's config file.
pub fn schema(tool: &str) -> Result<String> {
    let s = match tool {
        "enum-report" | "enum_report" => enum_schema(),
        "select-by-contig" | "select_by_contig" => select_by_contig_schema(),
        "select-by-cds" | "select_by_cds" => select_by_cds_schema(),
        "domainate" => domainate_schema(),
        "domain-search" | "domain_search" => domain_search_schema(),
        "deduplicate" | "deduplicate-genbank" | "deduplicate_genbank" => deduplicate_schema(),
        "genbank-to-fasta" | "genbank_to_fasta" => genbank_to_fasta_schema(),
        "clean-sequences" | "clean_sequences" => clean_sequences_schema(),
        "sort-contigs" | "sort_contigs" => sort_contigs_schema(),
        "select-by-coord" | "select_by_coord" => select_by_coord_schema(),
        "filter-domains" | "filter_domains" => filter_domains_schema(),
        "color-genbank" | "color_genbank" => color_genbank_schema(),
        "color-table-to-legend" | "color_table_to_legend" => color_table_to_legend_schema(),
        "trim-contigs" | "trim_contigs" => trim_contigs_schema(),
        "extract-domains" | "extract_domains" => extract_domains_schema(),
        "extract-peptides" | "extract_peptides" => extract_peptides_schema(),
        "extract-unannotated" | "extract_unannotated" => extract_unannotated_schema(),
        "seq-dist" | "seq_dist" => seq_dist_schema(),
        "compare-contigs" | "compare_contigs" => compare_contigs_schema(),
        "hmmer-compare" | "hmmer_compare" => hmmer_compare_schema(),
        "transform-matrix" | "transform_matrix" => transform_matrix_schema(),
        "build-ssn" | "build_ssn" => build_ssn_schema(),
        "build-ssn-viewer" | "build_ssn_viewer" => build_ssn_viewer_schema(),
        "matrix-report" | "matrix_report" => matrix_report_schema(),
        "ssn-navigator" | "ssn_navigator" => ssn_navigator_schema(),
        "summary-report" | "summary_report" => summary_report_schema(),
        "hmmer-report" | "hmmer_report" => hmmer_report_schema(),
        "hmmer-select" | "hmmer_select" => hmmer_select_schema(),
        "hmmer-build" | "hmmer_build" => hmmer_build_schema(),
        "hmmer-search" | "hmmer_search" => hmmer_search_schema(),
        "format-db" | "format_db" | "domainator-format-db" | "domainator_format_db" => format_db_schema(),
        other => anyhow::bail!(
            "no schema for tool {other:?} (have: domainate, domain-search, seq-dist, compare-contigs, hmmer-compare, hmmer-report, hmmer-select, format-db, transform-matrix, build-ssn, build-ssn-viewer, matrix-report, ssn-navigator, summary-report, select-by-contig, select-by-cds, enum-report)"
        ),
    };
    let mut s = s;
    fill_common_descriptions(&mut s);
    serde_json::to_string_pretty(&s).context("serializing schema")
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
        "k" => "Number of nearest neighbours (k) for the kNN graph.",
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
        "max_merge_events" => "Maximum number of cluster-merge events to record.",
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

fn enum_schema() -> serde_json::Value {
    use serde_json::json;
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "enum_report config",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "input": {"type": "array", "items": {"type": "string"}, "description": "GenBank/FASTA input files"},
            "output": {"type": ["string", "null"], "description": "Output path (stdout if omitted)"},
            "by": {"enum": ["contig", "cds", "domain"], "default": "contig"},
            "databases": {"type": "array", "items": {"type": "string"}, "description": "Restrict domains to these databases"},
            "contigs": {"type": "array", "items": {"type": "string"}},
            "contigs_file": {"type": ["string", "null"]},
            "column_names": {"type": ["array", "null"], "items": {"type": "string"}, "description": "Override output column names"},
            "ncbi_taxonomy_path": {"type": ["string", "null"]},
            "json": {"type": ["string", "null"], "description": "Write streaming NDJSON (one compact object per row) here; '-' for stdout. Independent of `output`, which stays TSV"},
            "columns": {
                "type": "array",
                "description": "Ordered output columns; each has a `type` tag",
                "items": {
                    "type": "object",
                    "required": ["type"],
                    "properties": {
                        "type": {"enum": [
                            "domains","domain_descriptions","sequence","start","end","strand","length",
                            "topology","architecture","architecture_detailed","score","identity","definition",
                            "filename","cds_count","domain_count","domain_search","taxid_lineage","taxname_lineage",
                            "rank_lineage","isoelectric_point","starts_with","molecular_weight",
                            "taxid","taxname","qualifier","feature_count","append","motif_count","net_charge",
                            "repeat_count","named_domain_count"
                        ]},
                        "ranks": {"type": "array", "items": {"type": "string"}, "description": "for taxid/taxname"},
                        "feature": {"type": "string", "description": "for qualifier/feature_count"},
                        "qualifier": {"type": "string", "description": "for qualifier"},
                        "name": {"type": "string", "description": "for append/named_domain_count"},
                        "value_type": {"enum": ["int", "float", "str"], "description": "for append"},
                        "value": {"type": "string", "description": "for append"},
                        "pattern": {"type": "string", "description": "for motif_count (PROSITE)"},
                        "ph": {"type": "string", "description": "for net_charge"},
                        "min_length": {"type": "string", "description": "for repeat_count"}
                    }
                }
            }
        }
    })
}

fn select_by_contig_schema() -> serde_json::Value {
    use serde_json::json;
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "select_by_contig config",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "input": {"type": "array", "items": {"type": "string"}},
            "output": {"type": ["string", "null"]},
            "contigs": {"type": "array", "items": {"type": "string"}},
            "contigs_file": {"type": ["string", "null"]},
            "contigs_regex": {"type": ["string", "null"]},
            "definition_regex": {"type": ["string", "null"]},
            "sequence_regex": {"type": ["string", "null"]},
            "domains": {"type": "array", "items": {"type": "string"}},
            "domains_file": {"type": ["string", "null"]},
            "domain_expr": {"type": ["string", "null"], "description": "Boolean expr (& AND, | OR, ~ NOT)"},
            "domain_type": {"enum": ["domain", "search", "both"], "default": "domain"},
            "databases": {"type": "array", "items": {"type": "string"}},
            "unannotated": {"type": "boolean", "default": false},
            "evalue": {"type": "number", "default": 100000000.0},
            "fasta_out": {"type": "boolean", "default": false},
            "include_taxids": {"type": "array", "items": {"type": "integer"}},
            "exclude_taxids": {"type": "array", "items": {"type": "integer"}},
            "taxonomy_expr": {"type": ["string", "null"]},
            "ncbi_taxonomy_path": {"type": ["string", "null"]},
            "first": {"type": ["integer", "null"]},
            "length_lb": {"type": ["integer", "null"]},
            "length_ub": {"type": ["integer", "null"]},
            "cds_count_lb": {"type": ["integer", "null"]},
            "cds_count_ub": {"type": ["integer", "null"]},
            "search_score_lb": {"type": ["number", "null"]},
            "search_score_ub": {"type": ["number", "null"]},
            "invert": {"type": "boolean", "default": false}
        }
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
            "ncbi_taxonomy_path": {"type": ["string", "null"]}
        }
    })
}

fn domain_search_schema() -> serde_json::Value {
    use serde_json::json;
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "domain_search config",
        "type": "object",
        "additionalProperties": false,
        "properties": {
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
            "max_region_overlap": {"type": "number", "default": 1.0},
            "decoys": {"type": "array", "items": {"type": "string"}},
            "decoys_file": {"type": ["string", "null"]},
            "include_taxids": {"type": "array", "items": {"type": "integer"}},
            "exclude_taxids": {"type": "array", "items": {"type": "integer"}},
            "taxonomy_expr": {"type": ["string", "null"]},
            "ncbi_taxonomy_path": {"type": ["string", "null"]},
            "max_output_gb": {"type": ["number", "null"], "default": 25.0, "description": "Fail if projected uncompressed GenBank output exceeds this many GB (0 disables)"}
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
            "algorithm": {"enum": ["hash", "diamond", "usearch", "cd-hit", null], "description": "cd-hit deferred"},
            "bin_path": {"type": ["string", "null"], "description": "Path to the external clusterer binary"},
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

fn sort_contigs_schema() -> serde_json::Value {
    use serde_json::json;
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "sort_contigs config",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "input": {"type": "array", "items": {"type": "string"}},
            "output": {"type": ["string", "null"]},
            "max_hits": {"type": ["integer", "null"], "description": "Keep top-N by score"},
            "domains": {"type": "array", "items": {"type": "string"}, "description": "Sort by best Domainator score among these"},
            "databases": {"type": "array", "items": {"type": "string"}, "description": "With domains: restrict to these databases"}
        }
    })
}

fn select_by_coord_schema() -> serde_json::Value {
    use serde_json::json;
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "select_by_coord config",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "input": {"type": "array", "items": {"type": "string"}},
            "output": {"type": ["string", "null"]},
            "coords": {"type": ["string", "null"], "description": "Labeled coordinate table (contig,start,end,strand?,file?)"}
        }
    })
}

fn filter_domains_schema() -> serde_json::Value {
    use serde_json::json;
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "filter_domains config",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "input": {"type": "array", "items": {"type": "string"}},
            "output": {"type": ["string", "null"]},
            "evalue": {"type": ["number", "null"], "default": 100000000.0},
            "max_overlap": {"type": ["number", "null"], "default": 1.0},
            "domains": {"type": "array", "items": {"type": "string"}},
            "domains_file": {"type": ["string", "null"]},
            "databases_keep": {"type": "array", "items": {"type": "string"}},
            "databases_filter": {"type": "array", "items": {"type": "string"}},
            "annotation_type": {"enum": ["domainator", "domain_search", "all", null], "default": "all"},
            "contigs": {"type": "array", "items": {"type": "string"}},
            "contigs_file": {"type": ["string", "null"]}
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

fn extract_domains_schema() -> serde_json::Value {
    use serde_json::json;
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "extract_domains config",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "input": {"type": "array", "items": {"type": "string"}},
            "output": {"type": ["string", "null"]},
            "evalue": {"type": ["number", "null"]},
            "score": {"type": ["number", "null"]},
            "domains": {"type": "array", "items": {"type": "string"}},
            "domains_file": {"type": ["string", "null"]},
            "databases": {"type": "array", "items": {"type": "string"}},
            "search_hits": {"type": "boolean", "default": false},
            "pad_up": {"type": ["integer", "null"], "default": 0},
            "pad_down": {"type": ["integer", "null"], "default": 0},
            "combine": {"type": "boolean", "default": false},
            "splice": {"type": "boolean", "default": false},
            "keep_direction": {"type": "boolean", "default": false},
            "keep_name": {"type": "boolean", "default": false},
            "fasta_out": {"type": "boolean", "default": false},
            "contigs": {"type": "array", "items": {"type": "string"}},
            "contigs_file": {"type": ["string", "null"]}
        }
    })
}

fn extract_peptides_schema() -> serde_json::Value {
    use serde_json::json;
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "extract_peptides config",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "input": {"type": "array", "items": {"type": "string"}},
            "output": {"type": ["string", "null"]},
            "domains": {"type": "array", "items": {"type": "string"}},
            "domains_file": {"type": ["string", "null"]},
            "databases": {"type": "array", "items": {"type": "string"}},
            "unannotated": {"type": "boolean", "default": false},
            "search_hits": {"type": "boolean", "default": false},
            "strand": {"enum": ["f", "r", null]},
            "invert": {"type": "boolean", "default": false},
            "cdss": {"type": "array", "items": {"type": "string"}},
            "cdss_file": {"type": ["string", "null"]},
            "keep_name": {"type": "boolean", "default": false},
            "name_field": {"type": ["string", "null"]},
            "evalue": {"type": ["number", "null"], "default": 100000000.0},
            "fasta_out": {"type": "boolean", "default": false},
            "contigs": {"type": "array", "items": {"type": "string"}},
            "contigs_file": {"type": ["string", "null"]}
        }
    })
}

fn extract_unannotated_schema() -> serde_json::Value {
    use serde_json::json;
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "extract_unannotated config",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "input": {"type": "array", "items": {"type": "string"}},
            "output": {"type": ["string", "null"]},
            "evalue": {"type": ["number", "null"]},
            "score": {"type": ["number", "null"]},
            "databases": {"type": "array", "items": {"type": "string"}},
            "search_hits": {"type": "boolean", "default": false},
            "keep_direction": {"type": "boolean", "default": false},
            "keep_name": {"type": "boolean", "default": false},
            "largest": {"type": "boolean", "default": false},
            "lb": {"type": ["integer", "null"]},
            "fasta_out": {"type": "boolean", "default": false},
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
            "merge_impact_metric": {"enum": ["min_child", "product"], "default": "min_child"},
            "max_merge_events": {"type": ["integer", "null"], "default": 500},
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
            "members": {"type": "boolean", "default": false, "description": "Include the full member id list in mode 'cluster'"}
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
            "input": {"type": "array", "items": {"type": "string"}, "description": "Query .hmm files"},
            "reference": {"type": "array", "items": {"type": "string"}, "description": "Reference .hmm files"},
            "output": {"type": ["string", "null"], "description": "TSV output (stdout if omitted)"},
            "score_cutoff": {"type": "number", "default": 0.0},
            "k": {"type": ["integer", "null"]},
            "alignments": {"type": "boolean", "default": false},
            "cpu": {"type": "integer", "default": 0},
            "max_output_gb": {"type": ["number", "null"], "default": 25.0, "description": "Fail if the projected TSV output exceeds this many GB (0 disables)"}
        }
    })
}

fn hmmer_report_schema() -> serde_json::Value {
    use serde_json::json;
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "hmmer_report config",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "input": {"type": "array", "items": {"type": "string"}, "description": "Input .hmm files"},
            "output": {"type": ["string", "null"], "description": "TSV output (stdout if omitted)"},
            "column_names": {"type": ["array", "null"], "items": {"type": "string"}, "description": "Override output column names"},
            "json": {"type": ["string", "null"], "description": "Write streaming NDJSON (one compact object per row) here; '-' for stdout. Independent of `output`, which stays TSV"},
            "columns": {
                "type": "array",
                "description": "Ordered output columns (after the leading `name`); each has a `type` tag",
                "items": {
                    "type": "object",
                    "required": ["type"],
                    "properties": {
                        "type": {"enum": ["source", "acc", "desc", "length", "consensus", "alphabet", "append"]},
                        "name": {"type": "string", "description": "for append"},
                        "value_type": {"enum": ["int", "float", "str"], "description": "for append"},
                        "value": {"type": "string", "description": "for append"}
                    }
                }
            }
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
            "output": {"type": ["string", "null"], "description": "Output .hmm (stdout if omitted)"},
            "name": {"type": ["string", "null"], "description": "Profile NAME (required)"},
            "acc": {"type": ["string", "null"]},
            "desc": {"type": ["string", "null"]},
            "alphabet": {"enum": ["amino", "dna", "rna", null], "description": "Inferred when omitted"}
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
            "input": {"type": "array", "items": {"type": "string"}, "description": "Query .hmm file(s) (the set being filtered)"},
            "reference": {"type": "array", "items": {"type": "string"}, "description": "Reference .hmm file(s) (the filter criterion)"},
            "output": {"type": ["string", "null"], "description": "Output .hmm (stdout if omitted)"},
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
            "input": {"type": "array", "items": {"type": "string"}, "description": "Input .hmm files"},
            "output": {"type": ["string", "null"], "description": "HMM output (stdout if omitted)"},
            "regex": {"type": "array", "items": {"type": "string"}, "description": "Regex pattern(s) to match"},
            "exact": {"type": "array", "items": {"type": "string"}, "description": "Exact string(s) to match"},
            "contains": {"type": "array", "items": {"type": "string"}, "description": "Substring(s) to match"},
            "field": {"enum": ["name", "desc", "acc", "all", null], "description": "Which field(s) to search (default all)"},
            "case_sensitive": {"type": "boolean", "default": false}
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
            "force": {"type": "boolean", "default": false}
        }
    })
}

fn select_by_cds_schema() -> serde_json::Value {
    use serde_json::json;
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "select_by_cds config",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "input": {"type": "array", "items": {"type": "string"}},
            "output": {"type": ["string", "null"]},
            "kb_range": {"type": ["number", "null"]},
            "kb_up": {"type": ["number", "null"]},
            "kb_down": {"type": ["number", "null"]},
            "cds_range": {"type": ["integer", "null"]},
            "cds_up": {"type": ["integer", "null"]},
            "cds_down": {"type": ["integer", "null"]},
            "whole_contig": {"type": "boolean", "default": false},
            "contigs": {"type": "array", "items": {"type": "string"}},
            "contigs_file": {"type": ["string", "null"]},
            "cds": {"type": "array", "items": {"type": "string"}},
            "cds_file": {"type": ["string", "null"]},
            "domains": {"type": "array", "items": {"type": "string"}},
            "domains_file": {"type": ["string", "null"]},
            "domain_expr": {"type": ["string", "null"]},
            "databases": {"type": "array", "items": {"type": "string"}},
            "strand": {"enum": ["f", "r", null]},
            "unannotated": {"type": "boolean", "default": false},
            "search_hits": {"type": "boolean", "default": false},
            "pad": {"type": "boolean", "default": false},
            "keep_direction": {"type": "boolean", "default": false},
            "skip_deduplicate": {"type": "boolean", "default": false},
            "max_region_overlap": {"type": "number", "default": 1.0},
            "evalue": {"type": "number", "default": 100000000.0},
            "invert": {"type": "boolean", "default": false},
            "include_nucleic_acids": {"type": "boolean", "default": false}
        }
    })
}
