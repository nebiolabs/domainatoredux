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
    pub z: Option<f64>,
    pub max_overlap: Option<f64>,
}

impl DomainateConfig {
    pub fn merge_cli(&mut self, cli: DomainateConfig) {
        merge_scalar!(self, cli, input);
        merge_vec!(self, cli, references);
        merge_scalar!(self, cli, output);
        merge_scalar!(self, cli, evalue);
        merge_scalar!(self, cli, z);
        merge_scalar!(self, cli, max_overlap);
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
    pub z: Option<f64>,
    pub add_annotations: bool,
    pub max_hits: Option<usize>,
    pub batch_size: Option<u64>,
    pub cpu: Option<usize>,
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
}

impl DomainSearchConfig {
    pub fn merge_cli(&mut self, cli: DomainSearchConfig) {
        merge_scalar!(self, cli, input);
        merge_vec!(self, cli, references);
        merge_scalar!(self, cli, output);
        merge_scalar!(self, cli, evalue);
        merge_scalar!(self, cli, z);
        self.add_annotations |= cli.add_annotations;
        merge_scalar!(self, cli, max_hits);
        merge_scalar!(self, cli, batch_size);
        merge_scalar!(self, cli, cpu);
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
        "seq-dist" | "seq_dist" => seq_dist_schema(),
        "compare-contigs" | "compare_contigs" => compare_contigs_schema(),
        "hmmer-compare" | "hmmer_compare" => hmmer_compare_schema(),
        other => anyhow::bail!(
            "no schema for tool {other:?} (have: domainate, domain-search, seq-dist, compare-contigs, hmmer-compare, select-by-contig, select-by-cds, enum-report)"
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
            "z": {"type": "number", "default": 1000.0, "description": "E-value search space size (-Z)"},
            "max_overlap": {"type": "number", "default": 1.0}
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
            "z": {"type": "number", "default": 1000.0},
            "add_annotations": {"type": "boolean", "default": false},
            "max_hits": {"type": ["integer", "null"]},
            "batch_size": {"type": "integer", "default": 10000},
            "cpu": {"type": "integer", "default": 0},
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
            "max_region_overlap": {"type": "number", "default": 1.0}
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
