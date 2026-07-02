//! Config-file (`--config`) and `--print_config` support for the
//! selection/reporting tools, plus `schema` output — a native Rust JSON/YAML
//! parameter schema (not byte-compatible with the Python jsonargparse files, but
//! serving the same purpose: declarative, agent-friendly, quoting-free params).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

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
    /// Emit a JSON array of row objects instead of TSV.
    pub json: bool,
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
        self.json = self.json || cli.json;
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
    pub cpu: Option<usize>,
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
        merge_scalar!(self, cli, cpu);
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
    pub cpu: Option<usize>,
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
        merge_scalar!(self, cli, cpu);
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
    pub color_table: Option<String>,
    pub color_table_out: Option<String>,
    pub cluster: bool,
    pub cluster_tsv: Option<String>,
    pub no_cluster_header: bool,
    pub mst: bool,
    pub mst_knn: Option<usize>,
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
        merge_scalar!(self, cli, color_table);
        merge_scalar!(self, cli, color_table_out);
        self.cluster |= cli.cluster;
        merge_scalar!(self, cli, cluster_tsv);
        self.no_cluster_header |= cli.no_cluster_header;
        self.mst |= cli.mst;
        merge_scalar!(self, cli, mst_knn);
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
    pub merge_impact_metric: Option<String>,
    pub max_merge_events: Option<usize>,
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
        merge_scalar!(self, cli, merge_impact_metric);
        merge_scalar!(self, cli, max_merge_events);
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
        other => anyhow::bail!(
            "no schema for tool {other:?} (have: domainate, domain-search, seq-dist, compare-contigs, hmmer-compare, transform-matrix, build-ssn, build-ssn-viewer, matrix-report, select-by-contig, select-by-cds, enum-report)"
        ),
    };
    serde_json::to_string_pretty(&s).context("serializing schema")
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
            "json": {"type": "boolean", "default": false, "description": "Emit a JSON array of row objects instead of TSV"},
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
            "ncbi_taxonomy_path": {"type": ["string", "null"]}
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
            "mode": {"type": "string", "default": "score"},
            "lb": {"type": "number", "default": 0.0},
            "k": {"type": ["integer", "null"]},
            "mst_knn": {"type": ["integer", "null"], "description": "Integer > 1"},
            "cpu": {"type": "integer", "default": 0}
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
            "output": {"type": ["string", "null"], "description": "Gzip-JSON .ssnv bundle output"},
            "html": {"type": ["string", "null"], "description": "Standalone HTML viewer output"},
            "embed_data": {"type": "boolean", "default": false, "description": "Embed the bundle into the HTML for autoload"},
            "name": {"type": ["string", "null"]},
            "metadata": {"type": "array", "items": {"type": "string"}},
            "subset": {"type": "array", "items": {"type": "string"}},
            "subset_file": {"type": ["string", "null"]},
            "color_by": {"type": ["string", "null"]},
            "label_by": {"type": ["string", "null"]},
            "merge_impact_metric": {"enum": ["min_child", "product"], "default": "min_child"},
            "max_merge_events": {"type": ["integer", "null"], "default": 500}
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
            "cluster": {"type": "boolean", "default": false},
            "cluster_tsv": {"type": ["string", "null"]},
            "no_cluster_header": {"type": "boolean", "default": false},
            "mst": {"type": "boolean", "default": false},
            "mst_knn": {"type": ["integer", "null"], "description": "Integer > 1"}
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
            "mst_knn": {"type": ["integer", "null"], "description": "Integer > 1"}
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
            "mst_knn": {"type": ["integer", "null"], "description": "Integer > 1"},
            "cpu": {"type": "integer", "default": 0}
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
            "cpu": {"type": "integer", "default": 0}
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
