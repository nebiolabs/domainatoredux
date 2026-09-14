//! Tool listing / schema / execution.
//!
//! Forms are driven by `domainator-config`'s per-tool JSON Schema; the form's
//! values deserialize straight back into the same `*Config` struct, which is
//! resolved (shared with the CLI via `domainator_config::resolve`) and run off the
//! UI thread. Each run records provenance (`jobs/<id>/job.json` + output sidecars).
//!
//! Phase-3 scope: a curated matrix-tool set (`transform-matrix`,
//! `build-ssn-viewer`) that pairs with the existing viewers. Generic dispatch for
//! all tools, a job registry with live progress, and cancellation are deferred.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;

use domainator_config::resolve::{resolve_build_ssn, resolve_build_ssn_viewer, resolve_transform_matrix};
use domainator_config::{
    schema, BuildSsnConfig, BuildSsnViewerConfig, CleanSequencesConfig, ColorGenbankConfig,
    ColorTableToLegendConfig, CompareContigsConfig, DeduplicateConfig, DomainSearchConfig,
    DomainateConfig, ExtractDomainsConfig, ExtractPeptidesConfig, ExtractUnannotatedConfig,
    FilterDomainsConfig, GenbankToFastaConfig, HmmerBuildConfig, HmmerCompareConfig,
    HmmerReportConfig, HmmerSearchConfig, HmmerSelectConfig, SelectByCdsConfig, SelectByCoordConfig,
    SelectByContigConfig, SeqDistConfig, SortContigsConfig, TransformMatrixConfig, TrimContigsConfig,
};
use domainator_core::app_support::provenance::{new_job_id, now_ms, write_job, IoRef, JobRecord};
use domainator_core::io::fasta::{read_fasta, write_fasta};
use domainator_core::io::genbank::{read_genbank, write_genbank};
use domainator_core::io::{detect_filetype, FileType};
use domainator_core::matrix::{cytoscape, hdf5_io, read_matrix, text};
use domainator_core::seq::SeqRecord;
use domainator_core::tools::build_ssn::build_ssn;
use domainator_core::tools::build_ssn_viewer::{build_ssn_viewer_bundle, gzip, serialize_bundle};
use domainator_core::tools::compare_contigs::{compare_contigs, CompareContigsOptions};
use domainator_core::tools::domain_search::{domain_search, DomainSearchOptions};
use domainator_core::tools::domainate::{self, domainate, DomainateOptions};
use domainator_core::tools::enum_report::{enum_report, static_analysis, By, OutputFormat};
use domainator_core::tools::filter_domains::{filter_domains, AnnotationType, FilterDomainsOptions};
use domainator_core::tools::hmmer_build::{hmmer_build, write_hmm_out, HmmerBuildOptions};
use domainator_core::tools::hmmer_select::{hmmer_select, Fields, SelectOptions};
use domainator_core::tools::{
    clean_sequences, color_genbank, color_table_to_legend, deduplicate_genbank, extract_domains,
    extract_peptides, extract_unannotated, hmmer_compare as hmmer_compare_m,
    hmmer_report as hmmer_report_m, hmmer_search as hmmer_search_m, select_by_coord, sort_contigs,
    trim_contigs,
};
use domainator_core::io::fasta::write_fasta_bp;
use domainator_core::tools::select_by_cds::{pad_records, select_by_cds, SelectByCdsOptions};
use domainator_core::tools::select_by_contig::{select_by_contig, SelectByContigOptions};
use domainator_core::tools::matrix_report::{
    matrix_report, render_text, DEFAULT_MAX_MERGE, DEFAULT_MERGE_IMPACT_METRIC,
};
use domainator_core::tools::neighborhood::{validate_range, Range};
use domainator_core::tools::select_common::parse_name_list;
use domainator_core::tools::seq_dist::{detect_kind, seq_dist, SeqDistOptions};
use domainator_core::seq::taxonomy::NCBITaxonomy;
use domainator_core::tools::transform_matrix::transform_matrix;
use domainator_core::app_support::workspace::dot_dir;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter};

/// Live state of a tool run, tracked in the [`JobRegistry`] managed state and
/// broadcast to the UI via the `job-update` event.
#[derive(Serialize, Deserialize, Clone)]
pub struct JobInfo {
    pub job_id: String,
    pub tool: String,
    /// `running` | `completed` | `failed` | `cancelled`.
    pub status: String,
    pub message: String,
    pub started_at_ms: String, // u128 as string (JSON-safe)
    pub outputs: Vec<String>,
    /// The params the tool was run with — lets the UI "clone" a job into a fresh
    /// pre-filled dialog.
    pub params: Value,
}

/// Managed state: all jobs seen this session + a cancel flag set.
#[derive(Default)]
pub struct JobRegistry {
    jobs: Mutex<HashMap<String, JobInfo>>,
    cancelled: Mutex<std::collections::HashSet<String>>,
    /// Workspace root for persisting job history to `.domainator/jobs.json`
    /// (set when a workspace is opened via `load_jobs`).
    root: Mutex<Option<String>>,
}

fn ms_str(ms: u128) -> String {
    ms.to_string()
}

/// One form field. `role` is `input_file` (dropdown of current-dir files of
/// `file_type`), `output_file` (filename; the app prepends the current dir), or
/// `param` (rendered from the tool's JSON schema by type).
#[derive(Serialize)]
pub struct ToolField {
    pub name: String,
    pub label: String,
    pub role: String,
    pub file_type: Option<String>,
    pub required: bool,
}

#[derive(Serialize)]
pub struct ToolMeta {
    pub id: String,
    pub label: String,
    pub description: String,
    pub fields: Vec<ToolField>,
}

fn f(name: &str, label: &str, role: &str, file_type: Option<&str>, required: bool) -> ToolField {
    ToolField {
        name: name.into(),
        label: label.into(),
        role: role.into(),
        file_type: file_type.map(str::to_string),
        required,
    }
}

/// Tools currently runnable from the GUI, each with its form field spec.
///
/// `async` so it runs on the async runtime, not Tauri's main thread — a
/// synchronous command can't be serviced while the main thread is busy (a tool
/// running, a big viewer painting), which would leave a tool dialog at "Loading…".
#[tauri::command]
pub async fn list_tools() -> Vec<ToolMeta> {
    vec![
        ToolMeta {
            id: "domain-search".into(),
            label: "Domain search".into(),
            description: "Search sequences with HMM/FASTA references and extract hit CDSs or their genomic neighborhoods (step 1).".into(),
            fields: vec![
                f("input", "Input sequences", "input_file", Some("sequence"), true),
                f("references", "References (HMM/FASTA)", "input_files", Some("reference"), true),
                f("output", "Output (GenBank)", "output_file", None, true),
                f("evalue", "E-value", "param", None, false),
                f("z", "Z (db size)", "param", None, false),
                f("add_annotations", "Add annotations", "param", None, false),
                f("max_hits", "Max hits", "param", None, false),
                f("max_mode", "Max mode", "param", None, false),
                // Neighborhood extraction around each hit CDS:
                f("whole_contig", "Whole contig", "param", None, false),
                f("cds_range", "CDS range (±)", "param", None, false),
                f("cds_up", "CDS upstream", "param", None, false),
                f("cds_down", "CDS downstream", "param", None, false),
                f("kb_range", "kb range (±)", "param", None, false),
                f("kb_up", "kb upstream", "param", None, false),
                f("kb_down", "kb downstream", "param", None, false),
                f("strand", "Strand (f/r)", "param", None, false),
                f("keep_direction", "Keep direction", "param", None, false),
                f("translate", "Translate", "param", None, false),
                // Decoy references + taxonomy filters:
                f("decoys", "Decoy refs", "param", None, false),
                f("decoys_file", "Decoys file (.txt)", "input_file", Some("text"), false),
                f("include_taxids", "Include taxids", "param", None, false),
                f("exclude_taxids", "Exclude taxids", "param", None, false),
                f("taxonomy_expr", "Taxonomy expr", "param", None, false),
                f("ncbi_taxonomy_path", "Taxdump dir", "param", None, false),
            ],
        },
        ToolMeta {
            id: "domainate".into(),
            label: "Domainate".into(),
            description: "Annotate sequences with domains from HMM/FASTA references (step 2).".into(),
            fields: vec![
                f("input", "Input sequences", "input_file", Some("sequence"), true),
                f("references", "References (HMM/FASTA)", "input_files", Some("reference"), true),
                f("output", "Output (GenBank)", "output_file", None, true),
                f("evalue", "E-value", "param", None, false),
                f("z", "Z (db size)", "param", None, false),
                f("max_domains", "Max domains", "param", None, false),
                f("max_overlap", "Max overlap", "param", None, false),
                f("max_mode", "Max mode", "param", None, false),
                f("hits_only", "Hits only", "param", None, false),
                f("include_taxids", "Include taxids", "param", None, false),
                f("exclude_taxids", "Exclude taxids", "param", None, false),
                f("taxonomy_expr", "Taxonomy expr", "param", None, false),
                f("ncbi_taxonomy_path", "Taxdump dir", "param", None, false),
            ],
        },
        ToolMeta {
            id: "seq-dist".into(),
            label: "Sequence distance".into(),
            description: "All-vs-all sequence similarity matrix (step 3; default 'hmmer' is pure-Rust — diamond algorithms need the diamond binary).".into(),
            fields: vec![
                f("input", "Input sequences", "input_file", Some("sequence"), true),
                f("reference", "Reference (optional)", "input_file", Some("sequence"), false),
                f("sparse", "Output (matrix)", "output_file", None, true),
                f("algorithm", "Algorithm", "param", None, false),
                f("mode", "Mode", "param", None, false),
                f("lb", "Lower bound", "param", None, false),
                f("mst_knn", "MST-kNN k", "param", None, false),
            ],
        },
        ToolMeta {
            id: "compare-contigs".into(),
            label: "Compare contigs".into(),
            description: "Neighborhood-similarity matrix from domain content (step 3 alternative; pure-Rust).".into(),
            fields: vec![
                f("input", "Input GenBank", "input_files", Some("genbank"), true),
                f("sparse", "Output (matrix)", "output_file", None, true),
                f("ji", "Jaccard weight", "param", None, false),
                f("ai", "Adjacency weight", "param", None, false),
                f("k", "kNN k", "param", None, false),
                f("lb", "Lower bound", "param", None, false),
                f("mst_knn", "MST-kNN k", "param", None, false),
            ],
        },
        ToolMeta {
            id: "select-by-cds".into(),
            label: "Select by CDS".into(),
            description: "Extract CDSs (and optionally their genomic neighborhoods) by name, domain content, or search hit.".into(),
            fields: vec![
                f("input", "Input sequences", "input_files", Some("sequence"), true),
                f("output", "Output (GenBank)", "output_file", None, true),
                // Which CDSs to select:
                f("cds", "CDS names", "param", None, false),
                f("cds_file", "CDS names file (.txt)", "input_file", Some("text"), false),
                f("domains", "Domain names", "param", None, false),
                f("domains_file", "Domain names file (.txt)", "input_file", Some("text"), false),
                f("domain_expr", "Domain expr", "param", None, false),
                f("databases", "Databases", "param", None, false),
                f("search_hits", "Search hits", "param", None, false),
                f("unannotated", "Unannotated", "param", None, false),
                f("invert", "Invert", "param", None, false),
                f("evalue", "E-value", "param", None, false),
                f("contigs", "Restrict to contigs", "param", None, false),
                f("contigs_file", "Contigs file (.txt)", "input_file", Some("text"), false),
                // Neighborhood extraction around each selected CDS:
                f("whole_contig", "Whole contig", "param", None, false),
                f("cds_range", "CDS range (±)", "param", None, false),
                f("cds_up", "CDS upstream", "param", None, false),
                f("cds_down", "CDS downstream", "param", None, false),
                f("kb_range", "kb range (±)", "param", None, false),
                f("kb_up", "kb upstream", "param", None, false),
                f("kb_down", "kb downstream", "param", None, false),
                f("strand", "Strand (f/r)", "param", None, false),
                f("keep_direction", "Keep direction", "param", None, false),
                f("pad", "Pad to equal length", "param", None, false),
                f("skip_deduplicate", "Skip dedup", "param", None, false),
                f("max_region_overlap", "Max region overlap", "param", None, false),
                f("include_nucleic_acids", "Include nucleic acids", "param", None, false),
            ],
        },
        ToolMeta {
            id: "select-by-contig".into(),
            label: "Select by contig".into(),
            description: "Select whole contigs by name/regex, domain content, taxonomy, length, or CDS count.".into(),
            fields: vec![
                f("input", "Input sequences", "input_files", Some("sequence"), true),
                f("output", "Output", "output_file", None, true),
                f("fasta_out", "FASTA output", "param", None, false),
                // Name / regex selection:
                f("contigs", "Contig names", "param", None, false),
                f("contigs_file", "Contigs file (.txt)", "input_file", Some("text"), false),
                f("contigs_regex", "Contigs regex", "param", None, false),
                f("definition_regex", "Definition regex", "param", None, false),
                f("sequence_regex", "Sequence regex", "param", None, false),
                // Domain-content selection:
                f("domains", "Domain names", "param", None, false),
                f("domains_file", "Domain names file (.txt)", "input_file", Some("text"), false),
                f("domain_expr", "Domain expr", "param", None, false),
                f("domain_type", "Domain type", "param", None, false),
                f("databases", "Databases", "param", None, false),
                f("unannotated", "Unannotated", "param", None, false),
                f("evalue", "E-value", "param", None, false),
                // Numeric filters:
                f("first", "First N", "param", None, false),
                f("length_lb", "Length ≥", "param", None, false),
                f("length_ub", "Length ≤", "param", None, false),
                f("cds_count_lb", "CDS count ≥", "param", None, false),
                f("cds_count_ub", "CDS count ≤", "param", None, false),
                f("search_score_lb", "Search score ≥", "param", None, false),
                f("search_score_ub", "Search score ≤", "param", None, false),
                f("invert", "Invert", "param", None, false),
                // Taxonomy filters:
                f("include_taxids", "Include taxids", "param", None, false),
                f("exclude_taxids", "Exclude taxids", "param", None, false),
                f("taxonomy_expr", "Taxonomy expr", "param", None, false),
                f("ncbi_taxonomy_path", "Taxdump dir", "param", None, false),
            ],
        },
        ToolMeta {
            id: "filter-domains".into(),
            label: "Filter domains".into(),
            description: "Remove domain annotations from records by e-value, overlap, name, or database (keeps the sequences).".into(),
            fields: vec![
                f("input", "Input sequences", "input_files", Some("sequence"), true),
                f("output", "Output (GenBank)", "output_file", None, true),
                f("evalue", "Max E-value", "param", None, false),
                f("max_overlap", "Max overlap", "param", None, false),
                f("domains", "Keep domains", "param", None, false),
                f("domains_file", "Keep domains file (.txt)", "input_file", Some("text"), false),
                f("databases_keep", "Keep databases", "param", None, false),
                f("databases_filter", "Remove databases", "param", None, false),
                f("annotation_type", "Annotation type", "param", None, false),
                f("contigs", "Restrict to contigs", "param", None, false),
                f("contigs_file", "Contigs file (.txt)", "input_file", Some("text"), false),
            ],
        },
        ToolMeta {
            id: "hmmer-select".into(),
            label: "HMM select".into(),
            description: "Extract a subset of profiles from HMM file(s) by name/description/accession (regex, exact, or substring match).".into(),
            fields: vec![
                f("input", "Input HMM(s)", "input_files", Some("hmm"), true),
                f("output", "Output (HMM)", "output_file", None, true),
                f("regex", "Regex match", "param", None, false),
                f("exact", "Exact match", "param", None, false),
                f("contains", "Substring match", "param", None, false),
                f("field", "Match field", "param", None, false),
                f("case_sensitive", "Case sensitive", "param", None, false),
            ],
        },
        ToolMeta {
            id: "hmmer-build".into(),
            label: "HMM build".into(),
            description: "Build an HMM profile from a Stockholm multiple-sequence alignment.".into(),
            fields: vec![
                f("input", "Input MSA (Stockholm)", "input_file", Some("stockholm"), true),
                f("output", "Output (HMM)", "output_file", None, true),
                f("name", "Profile name", "param", None, true),
                f("acc", "Accession", "param", None, false),
                f("desc", "Description", "param", None, false),
                f("alphabet", "Alphabet", "param", None, false),
            ],
        },
        ToolMeta {
            id: "genbank-to-fasta".into(),
            label: "GenBank → FASTA".into(),
            description: "Convert sequence records to FASTA.".into(),
            fields: vec![
                f("input", "Input sequences", "input_files", Some("sequence"), true),
                f("output", "Output (FASTA)", "output_file", None, true),
            ],
        },
        ToolMeta {
            id: "clean-sequences".into(),
            label: "Clean sequences".into(),
            description: "Normalize sequences: clean/deduplicate names, strip gaps, filter non-canonical residues.".into(),
            fields: vec![
                f("input", "Input sequences", "input_files", Some("sequence"), true),
                f("output", "Output", "output_file", None, true),
                f("fasta_out", "FASTA output", "param", None, false),
                f("fasta_type", "FASTA type", "param", None, false),
                f("clean_name", "Clean names", "param", None, false),
                f("deduplicate_names", "Deduplicate names", "param", None, false),
                f("strip", "Strip gaps", "param", None, false),
                f("filter_by_aa", "Filter non-canonical", "param", None, false),
            ],
        },
        ToolMeta {
            id: "sort-contigs".into(),
            label: "Sort contigs".into(),
            description: "Sort contigs by best search/domain score; optionally keep only the top N.".into(),
            fields: vec![
                f("input", "Input sequences", "input_files", Some("sequence"), true),
                f("output", "Output (GenBank)", "output_file", None, true),
                f("max_hits", "Keep top N", "param", None, false),
                f("domains", "Sort by domains", "param", None, false),
                f("databases", "Databases", "param", None, false),
            ],
        },
        ToolMeta {
            id: "select-by-coord".into(),
            label: "Select by coordinate".into(),
            description: "Extract sub-regions listed in a coordinate table (contig, start, end).".into(),
            fields: vec![
                f("input", "Input sequences", "input_files", Some("sequence"), true),
                f("coords", "Coordinate table", "input_file", Some("table"), true),
                f("output", "Output (GenBank)", "output_file", None, true),
            ],
        },
        ToolMeta {
            id: "trim-contigs".into(),
            label: "Trim contigs".into(),
            description: "Trim the ends of contigs by kb, CDS count, or up to a domain boundary.".into(),
            fields: vec![
                f("input", "Input sequences", "input_files", Some("sequence"), true),
                f("output", "Output (GenBank)", "output_file", None, true),
                f("kb_both", "kb both ends", "param", None, false),
                f("kb_left", "kb left", "param", None, false),
                f("kb_right", "kb right", "param", None, false),
                f("cds_both", "CDS both ends", "param", None, false),
                f("cds_left", "CDS left", "param", None, false),
                f("cds_right", "CDS right", "param", None, false),
                f("domains", "Trim to domains", "param", None, false),
                f("domain_expr", "Domain expr", "param", None, false),
                f("no_domain", "Drop domain-less", "param", None, false),
                f("contigs", "Restrict to contigs", "param", None, false),
            ],
        },
        ToolMeta {
            id: "extract-domains".into(),
            label: "Extract domains".into(),
            description: "Extract annotated domain regions as their own records (GenBank or FASTA).".into(),
            fields: vec![
                f("input", "Input sequences", "input_files", Some("sequence"), true),
                f("output", "Output", "output_file", None, true),
                f("fasta_out", "FASTA output", "param", None, false),
                f("domains", "Domain names", "param", None, false),
                f("databases", "Databases", "param", None, false),
                f("search_hits", "Search hits", "param", None, false),
                f("evalue", "Max E-value", "param", None, false),
                f("score", "Min score", "param", None, false),
                f("pad_up", "Pad upstream", "param", None, false),
                f("pad_down", "Pad downstream", "param", None, false),
                f("combine", "Combine adjacent", "param", None, false),
                f("splice", "Splice", "param", None, false),
                f("keep_direction", "Keep direction", "param", None, false),
                f("keep_name", "Keep name", "param", None, false),
                f("contigs", "Restrict to contigs", "param", None, false),
            ],
        },
        ToolMeta {
            id: "extract-peptides".into(),
            label: "Extract peptides".into(),
            description: "Extract CDS translations (peptides) by domain/CDS name, search hit, or all.".into(),
            fields: vec![
                f("input", "Input sequences", "input_files", Some("sequence"), true),
                f("output", "Output", "output_file", None, true),
                f("fasta_out", "FASTA output", "param", None, false),
                f("domains", "Domain names", "param", None, false),
                f("cdss", "CDS names", "param", None, false),
                f("databases", "Databases", "param", None, false),
                f("unannotated", "Unannotated", "param", None, false),
                f("search_hits", "Search hits", "param", None, false),
                f("strand", "Strand (f/r)", "param", None, false),
                f("invert", "Invert", "param", None, false),
                f("keep_name", "Keep name", "param", None, false),
                f("name_field", "Name field", "param", None, false),
                f("evalue", "Max E-value", "param", None, false),
                f("contigs", "Restrict to contigs", "param", None, false),
            ],
        },
        ToolMeta {
            id: "extract-unannotated".into(),
            label: "Extract unannotated".into(),
            description: "Extract the regions of each CDS/contig with no domain annotation.".into(),
            fields: vec![
                f("input", "Input sequences", "input_files", Some("sequence"), true),
                f("output", "Output", "output_file", None, true),
                f("fasta_out", "FASTA output", "param", None, false),
                f("databases", "Databases", "param", None, false),
                f("search_hits", "Search hits", "param", None, false),
                f("largest", "Largest only", "param", None, false),
                f("lb", "Min length", "param", None, false),
                f("keep_direction", "Keep direction", "param", None, false),
                f("keep_name", "Keep name", "param", None, false),
                f("evalue", "Max E-value", "param", None, false),
                f("score", "Min score", "param", None, false),
                f("contigs", "Restrict to contigs", "param", None, false),
            ],
        },
        ToolMeta {
            id: "color-genbank".into(),
            label: "Color GenBank".into(),
            description: "Batch-assign feature colors by name/database (color table or auto palette).".into(),
            fields: vec![
                f("input", "Input sequences", "input_files", Some("sequence"), true),
                f("output", "Output (GenBank)", "output_file", None, true),
                f("mode", "Color targets", "param", None, false),
                f("domain_database", "Color by database", "param", None, false),
                f("color_table", "Color table (in)", "input_file", Some("table"), false),
                f("color_table_out", "Color table (out)", "output_file", None, false),
                f("search_hit_color", "Search-hit color", "param", None, false),
                f("clear", "Clear existing", "param", None, false),
                f("databases", "Databases", "param", None, false),
                f("contigs", "Restrict to contigs", "param", None, false),
            ],
        },
        ToolMeta {
            id: "color-table-to-legend".into(),
            label: "Color table → legend".into(),
            description: "Render a color table as an SVG legend.".into(),
            fields: vec![
                f("input", "Color table(s)", "input_files", Some("table"), true),
                f("svg", "Output (SVG)", "output_file", None, true),
                f("title", "Title", "param", None, false),
            ],
        },
        ToolMeta {
            id: "deduplicate".into(),
            label: "Deduplicate".into(),
            description: "Collapse near-identical sequences to representatives (hash, or diamond/cd-hit/usearch backends).".into(),
            fields: vec![
                f("input", "Input sequences", "input_files", Some("sequence"), true),
                f("output", "Output", "output_file", None, true),
                f("algorithm", "Algorithm", "param", None, false),
                f("id", "Identity threshold", "param", None, true),
                f("fasta_out", "FASTA output", "param", None, false),
                f("fasta_type", "FASTA type", "param", None, false),
                f("both_strands", "Both strands", "param", None, false),
                f("prefix_count", "Prefix count", "param", None, false),
                f("suffix_count", "Suffix count", "param", None, false),
                f("cluster_table", "Cluster table (out)", "output_file", None, false),
                f("cluster_sep", "Cluster separator", "param", None, false),
                f("bin_path", "Binary path", "param", None, false),
            ],
        },
        ToolMeta {
            id: "hmmer-compare".into(),
            label: "HMM compare".into(),
            description: "Profile-vs-profile Viterbi comparison; writes a scores/alignments table.".into(),
            fields: vec![
                f("input", "Query HMM(s)", "input_files", Some("hmm"), true),
                f("reference", "Reference HMM(s)", "input_files", Some("hmm"), true),
                f("output", "Output (table)", "output_file", None, true),
                f("score_cutoff", "Score cutoff", "param", None, false),
                f("k", "Top-k per query", "param", None, false),
                f("alignments", "Include alignments", "param", None, false),
            ],
        },
        ToolMeta {
            id: "hmmer-search".into(),
            label: "HMM search".into(),
            description: "Keep query profiles that score against a reference HMM set (writes an HMM file).".into(),
            fields: vec![
                f("input", "Query HMM(s)", "input_files", Some("hmm"), true),
                f("reference", "Reference HMM(s)", "input_files", Some("hmm"), true),
                f("output", "Output (HMM)", "output_file", None, true),
                f("score_cutoff", "Score cutoff", "param", None, false),
                f("max_hits", "Max hits", "param", None, false),
            ],
        },
        ToolMeta {
            id: "hmmer-report".into(),
            label: "HMM report (export)".into(),
            description: "Export a per-profile TSV/JSON report from HMM file(s) with a custom column set.".into(),
            fields: vec![
                f("input", "Input HMM(s)", "input_files", Some("hmm"), true),
                f("output", "Output", "output_file", None, true),
                f("columns", "Columns", "hmm_columns", None, false),
                f("json", "JSON output", "param", None, false),
            ],
        },
        ToolMeta {
            id: "enum-report".into(),
            label: "Enum report (export)".into(),
            description: "Export a per-contig/cds/domain TSV report from a sequence file with a chosen column set.".into(),
            fields: vec![
                f("input", "Input sequences", "input_file", Some("sequence"), true),
                f("output", "Output (TSV)", "output_file", None, true),
                f("by", "By", "param", None, false),
                f("columns", "Columns", "enum_columns", None, false),
            ],
        },
        ToolMeta {
            id: "matrix-report".into(),
            label: "Matrix report (export)".into(),
            description: "Export a text summary + suggested cutoffs from a similarity matrix.".into(),
            fields: vec![
                f("input", "Input matrix", "input_file", Some("hdf5"), true),
                f("output", "Output (text)", "output_file", None, true),
            ],
        },
        ToolMeta {
            id: "transform-matrix".into(),
            label: "Transform matrix".into(),
            description: "Rescale or prune a similarity matrix (mode, lower bound, MST-kNN).".into(),
            fields: vec![
                f("input", "Input matrix", "input_file", Some("hdf5"), true),
                f("sparse", "Output (matrix)", "output_file", None, true),
                f("mode", "Mode", "param", None, false),
                f("lb", "Lower bound", "param", None, false),
                f("mst_knn", "MST-kNN k", "param", None, false),
            ],
        },
        ToolMeta {
            id: "build-ssn".into(),
            label: "Build SSN (XGMML)".into(),
            description: "Build a Cytoscape XGMML network from a similarity matrix.".into(),
            fields: vec![
                f("input", "Input matrix", "input_file", Some("hdf5"), true),
                f("xgmml", "Output (XGMML)", "output_file", None, true),
                f("metadata", "Metadata (TSV)", "input_files", Some("table"), false),
                f("cluster", "Cluster", "param", None, false),
                f("cluster_tsv", "Cluster TSV", "output_file", None, false),
                f("lb", "Lower bound", "param", None, false),
                f("mst", "MST only", "param", None, false),
                f("mst_knn", "MST-kNN k", "param", None, false),
                f("color_by", "Color by", "param", None, false),
                f("color_table", "Color table (in)", "input_file", Some("table"), false),
                f("color_table_out", "Color table (out)", "output_file", None, false),
            ],
        },
        ToolMeta {
            id: "build-ssn-viewer".into(),
            label: "Build SSN viewer".into(),
            description: "Build a .dsnv bundle for the interactive network viewer.".into(),
            fields: vec![
                f("input", "Input matrix", "input_file", Some("hdf5"), true),
                f("output", "Output (.dsnv)", "output_file", None, true),
                f("metadata", "Metadata (TSV)", "input_files", Some("table"), false),
                f("merge_impact_metric", "Merge metric", "param", None, false),
                f("max_merge_events", "Max merge events", "param", None, false),
            ],
        },
    ]
}

/// The draft-07 JSON Schema for a tool's parameters (drives the form).
///
/// `async` so a tool dialog's schema fetch never waits on the main thread (see
/// [`list_tools`]) — otherwise the dialog can hang at "Loading…".
#[tauri::command]
pub async fn tool_schema(tool: String) -> Result<String, String> {
    schema(&tool).map_err(|e| e.to_string())
}

#[derive(Serialize)]
pub struct RunResult {
    pub job_id: String,
    pub outputs: Vec<String>,
    pub message: String,
}

/// The result the caller awaits; also mirrored into the `JobRegistry` and emitted
/// as `job-update` events (so any panel can react — a running-jobs indicator, etc.).
///
/// Runs off the UI thread. `params` is the tool's `*Config` as JSON (from the
/// form); `workspace_root` is where `jobs/` lives.
#[tauri::command]
pub async fn run_tool(
    app: AppHandle,
    registry: tauri::State<'_, JobRegistry>,
    tool: String,
    params: Value,
    workspace_root: String,
) -> Result<RunResult, String> {
    let started = now_ms();
    let job_id = new_job_id(started);

    let running = JobInfo {
        job_id: job_id.clone(),
        tool: tool.clone(),
        status: "running".into(),
        message: String::new(),
        started_at_ms: ms_str(started),
        outputs: Vec::new(),
        params: params.clone(),
    };
    set_job(&registry, &app, running);

    // Off-thread work. Core tools aren't interruptible, so cancellation is
    // best-effort: if the job was cancelled while running, we discard the result
    // and don't write outputs/provenance.
    let jid = job_id.clone();
    let tool_c = tool.clone();
    let root_c = workspace_root.clone();
    let params_c = params.clone();
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        run_one(&tool_c, params_c, &root_c, started, &jid)
    })
    .await
    .map_err(|e| format!("job failed to run: {e}"))?;

    let cancelled = registry.cancelled.lock().unwrap().contains(&job_id);
    let mut info = JobInfo {
        job_id: job_id.clone(),
        tool: tool.clone(),
        status: String::new(),
        message: String::new(),
        started_at_ms: ms_str(started),
        outputs: Vec::new(),
        params: params.clone(),
    };
    match outcome {
        _ if cancelled => {
            info.status = "cancelled".into();
            info.message = "cancelled".into();
            set_job(&registry, &app, info.clone());
            Err("job cancelled".into())
        }
        Ok(r) => {
            info.status = "completed".into();
            info.message = r.message.clone();
            info.outputs = r.outputs.clone();
            set_job(&registry, &app, info);
            Ok(r)
        }
        Err(e) => {
            info.status = "failed".into();
            info.message = e.clone();
            set_job(&registry, &app, info);
            Err(e)
        }
    }
}

/// Persisted job-history cap (newest kept), so `.domainator/jobs.json` can't grow
/// without bound.
const MAX_PERSISTED_JOBS: usize = 500;

fn jobs_file(root: &str) -> std::path::PathBuf {
    dot_dir(Path::new(root)).join("jobs.json")
}

/// Write the current job history to `.domainator/jobs.json` (newest first, capped).
/// Best-effort: no-op until a workspace root is known; failures are ignored.
fn persist_jobs(registry: &JobRegistry) {
    let Some(root) = registry.root.lock().unwrap().clone() else { return };
    let mut list: Vec<JobInfo> = registry.jobs.lock().unwrap().values().cloned().collect();
    list.sort_by_key(|b| std::cmp::Reverse(job_ms(b)));
    list.truncate(MAX_PERSISTED_JOBS);
    let path = jobs_file(&root);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(bytes) = serde_json::to_vec_pretty(&list) {
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, &bytes).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

fn job_ms(j: &JobInfo) -> u128 {
    j.started_at_ms.parse().unwrap_or(0)
}

/// Record a job's state, broadcast it to the UI, and persist the history.
fn set_job(registry: &JobRegistry, app: &AppHandle, info: JobInfo) {
    registry.jobs.lock().unwrap().insert(info.job_id.clone(), info.clone());
    let _ = app.emit("job-update", &info);
    persist_jobs(registry);
}

/// Status of one job (for polling).
#[tauri::command]
pub fn job_status(registry: tauri::State<'_, JobRegistry>, job_id: String) -> Option<JobInfo> {
    registry.jobs.lock().unwrap().get(&job_id).cloned()
}

/// All jobs seen this session (newest state).
#[tauri::command]
pub fn list_jobs(registry: tauri::State<'_, JobRegistry>) -> Vec<JobInfo> {
    registry.jobs.lock().unwrap().values().cloned().collect()
}

/// Load persisted job history for a workspace (called on open). Sets the registry's
/// root (so future updates persist there), replaces the in-memory jobs with the
/// file's, and marks any left-over `running` jobs as interrupted (the process that
/// ran them is gone). Returns the history.
#[tauri::command]
pub fn load_jobs(registry: tauri::State<'_, JobRegistry>, workspace_root: String) -> Vec<JobInfo> {
    *registry.root.lock().unwrap() = Some(workspace_root.clone());
    let loaded: Vec<JobInfo> = std::fs::read(jobs_file(&workspace_root))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let list = {
        let mut map = registry.jobs.lock().unwrap();
        map.clear();
        for mut j in loaded {
            if j.status == "running" {
                j.status = "failed".into();
                j.message = "interrupted (app closed)".into();
            }
            map.insert(j.job_id.clone(), j);
        }
        map.values().cloned().collect::<Vec<_>>()
    };
    persist_jobs(&registry); // save the corrected (no-longer-running) statuses
    list
}

/// Delete one job from the history.
#[tauri::command]
pub fn delete_job(registry: tauri::State<'_, JobRegistry>, job_id: String) {
    registry.jobs.lock().unwrap().remove(&job_id);
    persist_jobs(&registry);
}

/// Clear the entire job history.
#[tauri::command]
pub fn clear_jobs(registry: tauri::State<'_, JobRegistry>) {
    registry.jobs.lock().unwrap().clear();
    persist_jobs(&registry);
}

/// Best-effort cancel: flags the job so its result is discarded on completion.
#[tauri::command]
pub fn cancel_job(registry: tauri::State<'_, JobRegistry>, job_id: String) {
    registry.cancelled.lock().unwrap().insert(job_id);
}

/// The actual tool dispatch + provenance write (runs on the blocking pool).
fn run_one(tool: &str, params: Value, root: &str, started: u128, job_id: &str) -> Result<RunResult, String> {
    let (inputs, outputs, message) = match tool {
        "domain-search" => run_domain_search(params.clone())?,
        "domainate" => run_domainate(params.clone())?,
        "seq-dist" => run_seq_dist(params.clone())?,
        "compare-contigs" => run_compare_contigs(params.clone())?,
        "select-by-cds" => run_select_by_cds(params.clone())?,
        "select-by-contig" => run_select_by_contig(params.clone())?,
        "filter-domains" => run_filter_domains(params.clone())?,
        "hmmer-select" => run_hmmer_select(params.clone())?,
        "hmmer-build" => run_hmmer_build(params.clone())?,
        "hmmer-compare" => run_hmmer_compare(params.clone())?,
        "hmmer-search" => run_hmmer_search(params.clone())?,
        "hmmer-report" => run_hmmer_report(params.clone())?,
        "genbank-to-fasta" => run_genbank_to_fasta(params.clone())?,
        "clean-sequences" => run_clean_sequences(params.clone())?,
        "sort-contigs" => run_sort_contigs(params.clone())?,
        "select-by-coord" => run_select_by_coord(params.clone())?,
        "trim-contigs" => run_trim_contigs(params.clone())?,
        "extract-domains" => run_extract_domains(params.clone())?,
        "extract-peptides" => run_extract_peptides(params.clone())?,
        "extract-unannotated" => run_extract_unannotated(params.clone())?,
        "color-genbank" => run_color_genbank(params.clone())?,
        "color-table-to-legend" => run_color_table_to_legend(params.clone())?,
        "deduplicate" => run_deduplicate(params.clone())?,
        "enum-report" => run_export_enum_report(params.clone())?,
        "matrix-report" => run_export_matrix_report(params.clone())?,
        "transform-matrix" => run_transform_matrix(params.clone())?,
        "build-ssn" => run_build_ssn(params.clone())?,
        "build-ssn-viewer" => run_build_ssn_viewer(params.clone())?,
        other => return Err(format!("tool not runnable from the GUI yet: {other}")),
    };
    let record = JobRecord {
        job_id: job_id.to_string(),
        tool: tool.to_string(),
        params,
        inputs,
        outputs: outputs.clone(),
        started_at_ms: started,
        finished_at_ms: now_ms(),
        status: "completed".into(),
        engine_version: env!("CARGO_PKG_VERSION").into(),
    };
    write_job(Path::new(root), &record, &format!("{tool} output"))
        .map_err(|e| format!("writing provenance: {e}"))?;
    Ok(RunResult { job_id: job_id.to_string(), outputs: outputs.into_iter().map(|o| o.path).collect(), message })
}

type RunOutput = (Vec<IoRef>, Vec<IoRef>, String);

/// Load a GenBank or FASTA sequence file (FASTA gets the given molecule type).
fn load_sequence(path: &str, fasta_type: &str) -> Result<Vec<SeqRecord>, String> {
    match detect_filetype(Path::new(path)) {
        Some(FileType::Genbank) => read_genbank(path).map_err(|e| format!("reading GenBank: {e}")),
        Some(FileType::Fasta) => read_fasta(path, Some(fasta_type)).map_err(|e| format!("reading FASTA: {e}")),
        None => Err(format!("unsupported sequence file: {path}")),
    }
}

/// Compile a taxid allow-set from include/exclude lists or a taxonomy expression,
/// using an NCBI taxdump at `path` (mirrors the CLI `compile_allowed_taxids`).
/// Returns `None` when no filter is requested. Requires `path` when a filter is set.
fn compile_allowed_taxids(
    include: &[i64],
    exclude: &[i64],
    expr: Option<&str>,
    path: Option<&str>,
) -> Result<Option<HashSet<i64>>, String> {
    let has_expr = expr.is_some_and(|e| !e.trim().is_empty());
    if include.is_empty() && exclude.is_empty() && !has_expr {
        return Ok(None);
    }
    if has_expr && (!include.is_empty() || !exclude.is_empty()) {
        return Err("taxonomy_expr is mutually exclusive with include_taxids/exclude_taxids".into());
    }
    let path = path
        .filter(|p| !p.trim().is_empty())
        .ok_or("taxonomy filters require ncbi_taxonomy_path (a taxdump directory)")?;
    let tax = NCBITaxonomy::load(Path::new(path)).map_err(|e| format!("loading taxonomy: {e}"))?;
    let allowed = if has_expr {
        tax.compile_taxonomy_expression(expr.unwrap())
    } else {
        let inc: HashSet<i64> = include.iter().copied().collect();
        let exc: HashSet<i64> = exclude.iter().copied().collect();
        tax.compile_taxonomy_filter(
            if inc.is_empty() { None } else { Some(&inc) },
            if exc.is_empty() { None } else { Some(&exc) },
        )
    };
    Ok(allowed)
}

fn ref_inputs(seq: &str, references: &[String]) -> Vec<IoRef> {
    let mut v = vec![IoRef { path: seq.to_string(), role: "sequence".into() }];
    for r in references {
        v.push(IoRef { path: r.clone(), role: "reference".into() });
    }
    v
}

fn run_domain_search(params: Value) -> Result<RunOutput, String> {
    let cfg: DomainSearchConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    let input = cfg.input.clone().ok_or("missing input sequences")?;
    let output = cfg.output.clone().ok_or("missing output (GenBank)")?;
    if cfg.references.is_empty() {
        return Err("select at least one HMM reference".into());
    }
    let dbs = domainate::read_hmm_references(&cfg.references).map_err(|e| format!("reading references: {e}"))?;
    // Neighborhood extraction (--cds_range/--kb_range/--whole_contig), mirroring the CLI.
    let range = validate_range(
        cfg.whole_contig,
        cfg.kb_range,
        cfg.kb_up,
        cfg.kb_down,
        cfg.cds_range,
        cfg.cds_up,
        cfg.cds_down,
    )?;
    if cfg.translate && !matches!(range, Range::None) {
        return Err("translate is incompatible with neighborhood extraction (cds_range/kb_range/whole_contig)".into());
    }
    let strand = match cfg.strand.as_deref() {
        Some("f") => Some(1),
        Some("r") => Some(-1),
        Some(other) => return Err(format!("strand must be 'f' or 'r', got {other:?}")),
        None => None,
    };
    let allowed_taxids = compile_allowed_taxids(
        &cfg.include_taxids,
        &cfg.exclude_taxids,
        cfg.taxonomy_expr.as_deref(),
        cfg.ncbi_taxonomy_path.as_deref(),
    )?;
    let decoys = parse_name_list(&cfg.decoys, cfg.decoys_file.as_deref())
        .map_err(|e| format!("reading decoys: {e}"))?;
    let opts = DomainSearchOptions {
        z: cfg.z.unwrap_or(1000.0),
        evalue: cfg.evalue.unwrap_or(0.001),
        min_evalue: cfg.min_evalue.unwrap_or(0.0),
        max_overlap: cfg.max_overlap.unwrap_or(1.0),
        add_annotations: cfg.add_annotations,
        max_hits: cfg.max_hits,
        max_hits_per_contig: cfg.max_hits_per_contig,
        batch_size: cfg.batch_size.unwrap_or(10000),
        fasta_type: cfg.fasta_type.clone().unwrap_or_else(|| "protein".into()),
        cpu: cfg.cpu.unwrap_or(0),
        range,
        normalize_direction: !cfg.keep_direction,
        strand,
        max_region_overlap: cfg.max_region_overlap.unwrap_or(1.0),
        translate: cfg.translate,
        max_mode: cfg.max_mode,
        allowed_taxids,
        decoys,
    };
    let hits = domain_search(std::slice::from_ref(&input), &dbs, &opts).map_err(|e| format!("domain_search: {e}"))?;
    write_genbank(&output, &hits).map_err(|e| format!("writing GenBank: {e}"))?;
    Ok((
        ref_inputs(&input, &cfg.references),
        vec![IoRef { path: output, role: "genbank".into() }],
        format!("wrote {} hit contigs", hits.len()),
    ))
}

fn run_domainate(params: Value) -> Result<RunOutput, String> {
    let cfg: DomainateConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    let input = cfg.input.clone().ok_or("missing input sequences")?;
    let output = cfg.output.clone().ok_or("missing output (GenBank)")?;
    if cfg.references.is_empty() {
        return Err("select at least one HMM reference".into());
    }
    let fasta_type = cfg.fasta_type.clone().unwrap_or_else(|| "protein".into());
    let records = load_sequence(&input, &fasta_type)?;
    let dbs = domainate::read_hmm_references(&cfg.references).map_err(|e| format!("reading references: {e}"))?;
    if dbs.has_nucleotide() && records.iter().any(|r| r.is_protein()) {
        return Err("cannot search protein inputs with a nucleotide reference".into());
    }
    let max_domains = match cfg.max_domains {
        Some(0) | None => usize::MAX,
        Some(n) => n,
    };
    let allowed_taxids = compile_allowed_taxids(
        &cfg.include_taxids,
        &cfg.exclude_taxids,
        cfg.taxonomy_expr.as_deref(),
        cfg.ncbi_taxonomy_path.as_deref(),
    )?;
    let opts = DomainateOptions {
        z: cfg.z.unwrap_or(1000.0),
        evalue: cfg.evalue.unwrap_or(0.001),
        min_evalue: cfg.min_evalue.unwrap_or(0.0),
        max_hits: max_domains,
        max_overlap: cfg.max_overlap.unwrap_or(1.0),
        no_annotations: cfg.no_annotations,
        hits_only: cfg.hits_only,
        overlap_by_db: cfg.overlap_by_db,
        max_mode: cfg.max_mode,
        allowed_taxids,
        cpu: cfg.cpu.unwrap_or(0),
        ..Default::default()
    };
    let annotated = domainate(records, &dbs, &opts);
    write_genbank(&output, &annotated).map_err(|e| format!("writing GenBank: {e}"))?;
    Ok((
        ref_inputs(&input, &cfg.references),
        vec![IoRef { path: output, role: "genbank".into() }],
        format!("annotated {} records", annotated.len()),
    ))
}

fn run_seq_dist(params: Value) -> Result<RunOutput, String> {
    let cfg: SeqDistConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    let input = cfg.input.clone().ok_or("missing input sequences")?;
    let output = cfg.sparse.clone().ok_or("missing output (matrix .hdf5)")?;
    let input_kind = detect_kind(Path::new(&input)).ok_or_else(|| format!("could not detect input kind: {input}"))?;
    let ref_kind = match cfg.reference.as_ref() {
        Some(p) => Some(detect_kind(Path::new(p)).ok_or_else(|| format!("could not detect reference kind: {p}"))?),
        None => None,
    };
    let opts = SeqDistOptions {
        algorithm: cfg.algorithm.clone().unwrap_or_else(|| "hmmer".into()),
        mode: cfg.mode.clone().unwrap_or_else(|| "score".into()),
        lb: cfg.lb.unwrap_or(0.0),
        k: cfg.k,
        mst_knn: cfg.mst_knn,
        knn: cfg.knn,
        params: domainator_core::tools::seq_dist::parse_diamond_params(
            cfg.params.as_deref().unwrap_or(""),
        )?,
        cpu: cfg.cpu.unwrap_or(0),
    };
    let matrix = seq_dist(&input, input_kind, cfg.reference.as_deref(), ref_kind, &opts)
        .map_err(|e| format!("seq_dist: {e}"))?;
    hdf5_io::write_sparse(&output, &matrix).map_err(|e| format!("writing matrix: {e}"))?;
    let mut inputs = vec![IoRef { path: input, role: "sequence".into() }];
    if let Some(r) = cfg.reference {
        inputs.push(IoRef { path: r, role: "reference".into() });
    }
    Ok((inputs, vec![IoRef { path: output, role: "matrix".into() }], format!("wrote {}×{} matrix", matrix.nrows(), matrix.ncols())))
}

fn run_compare_contigs(params: Value) -> Result<RunOutput, String> {
    let cfg: CompareContigsConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    if cfg.input.is_empty() {
        return Err("select at least one GenBank file".into());
    }
    let output = cfg.sparse.clone().ok_or("missing output (matrix .hdf5)")?;
    let ji = cfg.ji.unwrap_or(0.5);
    let ai = cfg.ai.unwrap_or(0.5);
    if ji == 0.0 && ai == 0.0 {
        return Err("set a non-zero Jaccard or Adjacency weight".into());
    }
    let opts = CompareContigsOptions {
        ji_weight: ji,
        ai_weight: ai,
        databases: if cfg.databases.is_empty() { None } else { Some(cfg.databases.iter().cloned().collect()) },
        k: cfg.k,
        lb: cfg.lb.unwrap_or(0.0),
        mst_knn: cfg.mst_knn,
        knn: cfg.knn,
        cpu: cfg.cpu.unwrap_or(0),
    };
    let matrix = compare_contigs(&cfg.input, &opts).map_err(|e| format!("compare_contigs: {e}"))?;
    hdf5_io::write_sparse(&output, &matrix).map_err(|e| format!("writing matrix: {e}"))?;
    let inputs = cfg.input.iter().map(|p| IoRef { path: p.clone(), role: "genbank".into() }).collect();
    Ok((inputs, vec![IoRef { path: output, role: "matrix".into() }], format!("wrote {}×{} matrix", matrix.nrows(), matrix.ncols())))
}

/// Load and concatenate all records from multiple sequence files (FASTA → protein).
fn load_inputs(paths: &[String]) -> Result<Vec<SeqRecord>, String> {
    let mut all = Vec::new();
    for p in paths {
        all.extend(load_sequence(p, "protein")?);
    }
    Ok(all)
}

/// Retain only records whose LOCUS name is in the contig name-list (if any).
fn filter_contigs(
    records: &mut Vec<SeqRecord>,
    contigs: &[String],
    contigs_file: Option<&str>,
) -> Result<(), String> {
    if let Some(set) =
        parse_name_list(contigs, contigs_file).map_err(|e| format!("reading contigs: {e}"))?
    {
        records.retain(|r| set.contains(&r.name));
    }
    Ok(())
}

/// One `f`/`r` strand selector into the internal ±1 (mirrors the CLI).
fn parse_strand(s: Option<&str>) -> Result<Option<i32>, String> {
    match s {
        Some("f") => Ok(Some(1)),
        Some("r") => Ok(Some(-1)),
        None | Some("") => Ok(None),
        Some(other) => Err(format!("strand must be 'f' or 'r', got {other:?}")),
    }
}

fn run_select_by_cds(params: Value) -> Result<RunOutput, String> {
    let cfg: SelectByCdsConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    if cfg.input.is_empty() {
        return Err("select at least one input sequence file".into());
    }
    let output = cfg.output.clone().ok_or("missing output (GenBank)")?;
    let mut records = load_inputs(&cfg.input)?;
    let target_contigs =
        parse_name_list(&cfg.contigs, cfg.contigs_file.as_deref()).map_err(|e| e.to_string())?;
    if let Some(set) = &target_contigs {
        records.retain(|r| set.contains(&r.name));
    }
    let target_cdss = parse_name_list(&cfg.cds, cfg.cds_file.as_deref()).map_err(|e| e.to_string())?;
    let target_domains =
        parse_name_list(&cfg.domains, cfg.domains_file.as_deref()).map_err(|e| e.to_string())?;
    let range = validate_range(
        cfg.whole_contig, cfg.kb_range, cfg.kb_up, cfg.kb_down, cfg.cds_range, cfg.cds_up, cfg.cds_down,
    )?;
    let strand = parse_strand(cfg.strand.as_deref())?;
    let opts = SelectByCdsOptions {
        target_cdss,
        target_domains,
        domain_expr: cfg.domain_expr,
        range,
        normalize_direction: !cfg.keep_direction,
        evalue: cfg.evalue.unwrap_or(100_000_000.0),
        invert: cfg.invert,
        search_hits: cfg.search_hits,
        max_region_overlap: cfg.max_region_overlap.unwrap_or(1.0),
        strand,
        databases: if cfg.databases.is_empty() { None } else { Some(cfg.databases.iter().cloned().collect()) },
        unannotated: cfg.unannotated,
        include_nucleic_acids: cfg.include_nucleic_acids,
    };
    let mut regions = select_by_cds(&records, &opts);
    if !cfg.skip_deduplicate {
        let mut seen = HashSet::new();
        regions.retain(|r| seen.insert(r.record.name.clone()));
    }
    if cfg.pad {
        pad_records(&mut regions);
    }
    let out_records: Vec<SeqRecord> = regions.into_iter().map(|r| r.record).collect();
    write_genbank(&output, &out_records).map_err(|e| format!("writing GenBank: {e}"))?;
    let inputs = cfg.input.iter().map(|p| IoRef { path: p.clone(), role: "sequence".into() }).collect();
    Ok((
        inputs,
        vec![IoRef { path: output, role: "genbank".into() }],
        format!("selected {} regions", out_records.len()),
    ))
}

fn run_select_by_contig(params: Value) -> Result<RunOutput, String> {
    let cfg: SelectByContigConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    if cfg.input.is_empty() {
        return Err("select at least one input sequence file".into());
    }
    let output = cfg.output.clone().ok_or("missing output")?;
    let records = load_inputs(&cfg.input)?;
    let target_contigs =
        parse_name_list(&cfg.contigs, cfg.contigs_file.as_deref()).map_err(|e| e.to_string())?;
    let target_domains =
        parse_name_list(&cfg.domains, cfg.domains_file.as_deref()).map_err(|e| e.to_string())?;
    let needs_tax = !cfg.include_taxids.is_empty()
        || !cfg.exclude_taxids.is_empty()
        || cfg.taxonomy_expr.as_deref().is_some_and(|e| !e.trim().is_empty());
    let tax = if needs_tax {
        let path = cfg
            .ncbi_taxonomy_path
            .as_deref()
            .filter(|p| !p.trim().is_empty())
            .ok_or("taxonomy filters require ncbi_taxonomy_path (a taxdump directory)")?;
        Some(NCBITaxonomy::load(Path::new(path)).map_err(|e| format!("loading taxonomy: {e}"))?)
    } else {
        None
    };
    let opts = SelectByContigOptions {
        target_domains,
        domain_expr: cfg.domain_expr,
        domain_evalue: cfg.evalue.unwrap_or(100_000_000.0),
        first: cfg.first,
        length_lb: cfg.length_lb,
        length_ub: cfg.length_ub,
        cds_count_lb: cfg.cds_count_lb,
        cds_count_ub: cfg.cds_count_ub,
        definition_regex: cfg.definition_regex,
        sequence_regex: cfg.sequence_regex,
        target_contigs,
        contigs_regex: cfg.contigs_regex,
        invert: cfg.invert,
        include_taxids: if cfg.include_taxids.is_empty() { None } else { Some(cfg.include_taxids.iter().copied().collect()) },
        exclude_taxids: if cfg.exclude_taxids.is_empty() { None } else { Some(cfg.exclude_taxids.iter().copied().collect()) },
        taxonomy_expr: cfg.taxonomy_expr,
        databases: if cfg.databases.is_empty() { None } else { Some(cfg.databases.iter().cloned().collect()) },
        unannotated: cfg.unannotated,
        search_score_lb: cfg.search_score_lb,
        search_score_ub: cfg.search_score_ub,
        domain_type: cfg.domain_type.unwrap_or_else(|| "domain".into()),
    };
    let selected = select_by_contig(records, tax.as_ref(), &opts).map_err(|e| e.to_string())?;
    let role = if cfg.fasta_out { "fasta" } else { "genbank" };
    if cfg.fasta_out {
        write_fasta(&output, &selected).map_err(|e| format!("writing FASTA: {e}"))?;
    } else {
        write_genbank(&output, &selected).map_err(|e| format!("writing GenBank: {e}"))?;
    }
    let inputs = cfg.input.iter().map(|p| IoRef { path: p.clone(), role: "sequence".into() }).collect();
    Ok((
        inputs,
        vec![IoRef { path: output, role: role.into() }],
        format!("selected {} contigs", selected.len()),
    ))
}

fn run_filter_domains(params: Value) -> Result<RunOutput, String> {
    let cfg: FilterDomainsConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    if cfg.input.is_empty() {
        return Err("select at least one input sequence file".into());
    }
    let output = cfg.output.clone().ok_or("missing output (GenBank)")?;
    let mut records = load_inputs(&cfg.input)?;
    filter_contigs(&mut records, &cfg.contigs, cfg.contigs_file.as_deref())?;
    let selected_domains =
        parse_name_list(&cfg.domains, cfg.domains_file.as_deref()).map_err(|e| e.to_string())?;
    let annotation_type =
        AnnotationType::parse(cfg.annotation_type.as_deref().unwrap_or("all")).map_err(|e| e.to_string())?;
    let opts = FilterDomainsOptions {
        evalue: cfg.evalue.unwrap_or(100_000_000.0),
        max_overlap: cfg.max_overlap.unwrap_or(1.0),
        databases_keep: if cfg.databases_keep.is_empty() { None } else { Some(cfg.databases_keep.iter().cloned().collect()) },
        databases_filter: if cfg.databases_filter.is_empty() { None } else { Some(cfg.databases_filter.iter().cloned().collect()) },
        selected_domains,
        annotation_type,
    };
    let out = filter_domains(records, &opts);
    write_genbank(&output, &out).map_err(|e| format!("writing GenBank: {e}"))?;
    let inputs = cfg.input.iter().map(|p| IoRef { path: p.clone(), role: "sequence".into() }).collect();
    Ok((
        inputs,
        vec![IoRef { path: output, role: "genbank".into() }],
        format!("wrote {} records", out.len()),
    ))
}

fn run_hmmer_select(params: Value) -> Result<RunOutput, String> {
    let cfg: HmmerSelectConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    if cfg.input.is_empty() {
        return Err("select at least one input .hmm file".into());
    }
    let output = cfg.output.clone().ok_or("missing output (.hmm)")?;
    let fields = Fields::from_field(cfg.field.as_deref().unwrap_or("all")).map_err(|e| e.to_string())?;
    let opts = SelectOptions {
        regex: cfg.regex,
        exact: cfg.exact,
        contains: cfg.contains,
        fields,
        case_sensitive: cfg.case_sensitive,
    };
    let mut f = std::io::BufWriter::new(
        std::fs::File::create(&output).map_err(|e| format!("creating {output}: {e}"))?,
    );
    let found = hmmer_select(&cfg.input, &opts, &mut f).map_err(|e| e.to_string())?;
    std::io::Write::flush(&mut f).map_err(|e| format!("flushing: {e}"))?;
    let inputs = cfg.input.iter().map(|p| IoRef { path: p.clone(), role: "hmm".into() }).collect();
    Ok((
        inputs,
        vec![IoRef { path: output, role: "hmm".into() }],
        format!("selected {found} profiles"),
    ))
}

fn run_hmmer_build(params: Value) -> Result<RunOutput, String> {
    let cfg: HmmerBuildConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    let input = cfg.input.clone().ok_or("missing input MSA (Stockholm)")?;
    let output = cfg.output.clone().ok_or("missing output (.hmm)")?;
    let name = cfg
        .name
        .clone()
        .filter(|n| !n.trim().is_empty())
        .ok_or("a profile name is required")?;
    let opts = HmmerBuildOptions {
        name,
        acc: cfg.acc.clone().filter(|s| !s.trim().is_empty()),
        desc: cfg.desc.clone().filter(|s| !s.trim().is_empty()),
        alphabet: cfg.alphabet.clone().filter(|s| !s.trim().is_empty()),
        window_length: cfg.window_length,
        window_beta: cfg.window_beta,
    };
    let hmm = hmmer_build(Path::new(&input), &opts).map_err(|e| e.to_string())?;
    let mut f = std::io::BufWriter::new(
        std::fs::File::create(&output).map_err(|e| format!("creating {output}: {e}"))?,
    );
    write_hmm_out(&hmm, &mut f).map_err(|e| e.to_string())?;
    std::io::Write::flush(&mut f).map_err(|e| format!("flushing: {e}"))?;
    Ok((
        vec![IoRef { path: input, role: "msa".into() }],
        vec![IoRef { path: output, role: "hmm".into() }],
        "built 1 profile".into(),
    ))
}

/// Write records as FASTA using the record **name** as the id (Biopython `.id`),
/// matching the CLI's `write_fasta_bp_out` (used by genbank_to_fasta/extract_*/clean).
fn write_fasta_bp_path(path: &str, records: &[SeqRecord]) -> Result<(), String> {
    let mut w = std::io::BufWriter::new(
        std::fs::File::create(path).map_err(|e| format!("creating {path}: {e}"))?,
    );
    write_fasta_bp(&mut w, records).map_err(|e| format!("writing FASTA: {e}"))?;
    std::io::Write::flush(&mut w).map_err(|e| e.to_string())
}

fn set_of(v: &[String]) -> Option<HashSet<String>> {
    if v.is_empty() { None } else { Some(v.iter().cloned().collect()) }
}

fn run_genbank_to_fasta(params: Value) -> Result<RunOutput, String> {
    let cfg: GenbankToFastaConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    if cfg.input.is_empty() {
        return Err("select at least one input file".into());
    }
    let output = cfg.output.clone().ok_or("missing output (FASTA)")?;
    let records = load_inputs(&cfg.input)?;
    write_fasta_bp_path(&output, &records)?;
    let inputs = cfg.input.iter().map(|p| IoRef { path: p.clone(), role: "sequence".into() }).collect();
    Ok((inputs, vec![IoRef { path: output, role: "fasta".into() }], format!("wrote {} sequences", records.len())))
}

fn run_clean_sequences(params: Value) -> Result<RunOutput, String> {
    let cfg: CleanSequencesConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    if cfg.input.is_empty() {
        return Err("select at least one input file".into());
    }
    let output = cfg.output.clone().ok_or("missing output")?;
    let fasta_type = cfg.fasta_type.clone().unwrap_or_else(|| "protein".into());
    let mut records = Vec::new();
    for p in &cfg.input {
        records.extend(load_sequence(p, &fasta_type)?);
    }
    let opts = clean_sequences::CleanSequencesOptions {
        fasta_type,
        clean_name: cfg.clean_name,
        deduplicate_names: cfg.deduplicate_names,
        strip: cfg.strip,
        filter_by_aa: cfg.filter_by_aa,
    };
    let mut stats = clean_sequences::CleaningStats::default();
    let out = clean_sequences::clean_sequences(records, &opts, &mut stats);
    let role = if cfg.fasta_out { "fasta" } else { "genbank" };
    if cfg.fasta_out {
        write_fasta_bp_path(&output, &out)?;
    } else {
        write_genbank(&output, &out).map_err(|e| format!("writing GenBank: {e}"))?;
    }
    let inputs = cfg.input.iter().map(|p| IoRef { path: p.clone(), role: "sequence".into() }).collect();
    Ok((inputs, vec![IoRef { path: output, role: role.into() }], format!("{} in → {} out", stats.input_count, stats.output_count)))
}

fn run_sort_contigs(params: Value) -> Result<RunOutput, String> {
    let cfg: SortContigsConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    if cfg.input.is_empty() {
        return Err("select at least one input file".into());
    }
    let output = cfg.output.clone().ok_or("missing output (GenBank)")?;
    let records = load_inputs(&cfg.input)?;
    let max_hits = match cfg.max_hits {
        Some(0) | None => None,
        Some(n) => Some(n),
    };
    let domains = set_of(&cfg.domains);
    let databases = if domains.is_none() { None } else { set_of(&cfg.databases) };
    let opts = sort_contigs::SortContigsOptions { max_hits, domains, databases };
    let out = sort_contigs::sort_contigs(records, &opts).map_err(|e| e.to_string())?;
    write_genbank(&output, &out).map_err(|e| format!("writing GenBank: {e}"))?;
    let inputs = cfg.input.iter().map(|p| IoRef { path: p.clone(), role: "sequence".into() }).collect();
    Ok((inputs, vec![IoRef { path: output, role: "genbank".into() }], format!("sorted {} contigs", out.len())))
}

fn run_select_by_coord(params: Value) -> Result<RunOutput, String> {
    let cfg: SelectByCoordConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    if cfg.input.is_empty() {
        return Err("select at least one input file".into());
    }
    let output = cfg.output.clone().ok_or("missing output (GenBank)")?;
    let coords_path = cfg.coords.clone().ok_or("a coordinate table is required")?;
    let table = std::fs::read_to_string(&coords_path).map_err(|e| format!("reading coordinate table: {e}"))?;
    let selections = select_by_coord::parse_coordinate_table(&table).map_err(|e| e.to_string())?;
    let mut records: Vec<(String, SeqRecord)> = Vec::new();
    for p in &cfg.input {
        let base = basename(p);
        for rec in load_sequence(p, "protein")? {
            records.push((base.clone(), rec));
        }
    }
    let out = select_by_coord::select_by_coord(&records, &selections).map_err(|e| e.to_string())?;
    write_genbank(&output, &out).map_err(|e| format!("writing GenBank: {e}"))?;
    let inputs = cfg.input.iter().map(|p| IoRef { path: p.clone(), role: "sequence".into() }).collect();
    Ok((inputs, vec![IoRef { path: output, role: "genbank".into() }], format!("selected {} regions", out.len())))
}

fn run_trim_contigs(params: Value) -> Result<RunOutput, String> {
    let cfg: TrimContigsConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    if cfg.input.is_empty() {
        return Err("select at least one input file".into());
    }
    let output = cfg.output.clone().ok_or("missing output (GenBank)")?;
    let mut records = load_inputs(&cfg.input)?;
    filter_contigs(&mut records, &cfg.contigs, cfg.contigs_file.as_deref())?;
    let (kb_truncation, cds_truncation) = trim_contigs::validate_range(
        cfg.kb_both, cfg.kb_left, cfg.kb_right, cfg.cds_both, cfg.cds_left, cfg.cds_right,
    ).map_err(|e| e.to_string())?;
    let strip_domains = parse_name_list(&cfg.domains, cfg.domains_file.as_deref()).map_err(|e| e.to_string())?;
    let opts = trim_contigs::TrimContigsOptions {
        kb_truncation,
        cds_truncation,
        strip_domains,
        domain_expr: cfg.domain_expr.clone(),
        no_domain: cfg.no_domain,
    };
    let out = trim_contigs::trim_contigs(records, &opts).map_err(|e| e.to_string())?;
    write_genbank(&output, &out).map_err(|e| format!("writing GenBank: {e}"))?;
    let inputs = cfg.input.iter().map(|p| IoRef { path: p.clone(), role: "sequence".into() }).collect();
    Ok((inputs, vec![IoRef { path: output, role: "genbank".into() }], format!("trimmed {} contigs", out.len())))
}

fn run_extract_domains(params: Value) -> Result<RunOutput, String> {
    let cfg: ExtractDomainsConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    if cfg.input.is_empty() {
        return Err("select at least one input file".into());
    }
    let output = cfg.output.clone().ok_or("missing output")?;
    let mut records = load_inputs(&cfg.input)?;
    filter_contigs(&mut records, &cfg.contigs, cfg.contigs_file.as_deref())?;
    let domains = parse_name_list(&cfg.domains, cfg.domains_file.as_deref()).map_err(|e| e.to_string())?;
    let opts = extract_domains::ExtractDomainsOptions {
        evalue: cfg.evalue,
        score: cfg.score,
        domains,
        pad_up: cfg.pad_up.unwrap_or(0),
        pad_down: cfg.pad_down.unwrap_or(0),
        combine: cfg.combine,
        keep_direction: cfg.keep_direction,
        keep_name: cfg.keep_name,
        search_hits: cfg.search_hits,
        databases: set_of(&cfg.databases),
        splice: cfg.splice,
    };
    let out = extract_domains::extract_domains(&records, &opts).map_err(|e| e.to_string())?;
    let role = if cfg.fasta_out { "fasta" } else { "genbank" };
    if cfg.fasta_out {
        write_fasta_bp_path(&output, &out)?;
    } else {
        write_genbank(&output, &out).map_err(|e| format!("writing GenBank: {e}"))?;
    }
    let inputs = cfg.input.iter().map(|p| IoRef { path: p.clone(), role: "sequence".into() }).collect();
    Ok((inputs, vec![IoRef { path: output, role: role.into() }], format!("extracted {} domains", out.len())))
}

fn run_extract_peptides(params: Value) -> Result<RunOutput, String> {
    let cfg: ExtractPeptidesConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    if cfg.input.is_empty() {
        return Err("select at least one input file".into());
    }
    let output = cfg.output.clone().ok_or("missing output")?;
    let mut records = load_inputs(&cfg.input)?;
    filter_contigs(&mut records, &cfg.contigs, cfg.contigs_file.as_deref())?;
    let target_domains = parse_name_list(&cfg.domains, cfg.domains_file.as_deref()).map_err(|e| e.to_string())?;
    let target_cdss = parse_name_list(&cfg.cdss, cfg.cdss_file.as_deref()).map_err(|e| e.to_string())?;
    let mut invert = cfg.invert;
    let extract_all = target_domains.is_none() && target_cdss.is_none() && !cfg.unannotated && !cfg.search_hits;
    if extract_all {
        invert = false;
    }
    let opts = extract_peptides::ExtractPeptidesOptions {
        evalue: cfg.evalue.unwrap_or(100_000_000.0),
        target_domains,
        target_cdss,
        search_hits: cfg.search_hits,
        strand: cfg.strand.clone().filter(|s| !s.is_empty()),
        unannotated: cfg.unannotated,
        invert,
        databases: set_of(&cfg.databases),
        keep_name: cfg.keep_name,
        name_field: cfg.name_field.clone(),
        extract_all,
    };
    let out = extract_peptides::extract_peptides(&records, &opts);
    let role = if cfg.fasta_out { "fasta" } else { "genbank" };
    if cfg.fasta_out {
        write_fasta_bp_path(&output, &out)?;
    } else {
        write_genbank(&output, &out).map_err(|e| format!("writing GenBank: {e}"))?;
    }
    let inputs = cfg.input.iter().map(|p| IoRef { path: p.clone(), role: "sequence".into() }).collect();
    Ok((inputs, vec![IoRef { path: output, role: role.into() }], format!("extracted {} peptides", out.len())))
}

fn run_extract_unannotated(params: Value) -> Result<RunOutput, String> {
    let cfg: ExtractUnannotatedConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    if cfg.input.is_empty() {
        return Err("select at least one input file".into());
    }
    let output = cfg.output.clone().ok_or("missing output")?;
    let mut records = load_inputs(&cfg.input)?;
    filter_contigs(&mut records, &cfg.contigs, cfg.contigs_file.as_deref())?;
    let opts = extract_unannotated::ExtractUnannotatedOptions {
        evalue: cfg.evalue,
        score: cfg.score,
        keep_direction: cfg.keep_direction,
        keep_name: cfg.keep_name,
        search_hits: cfg.search_hits,
        databases: set_of(&cfg.databases),
        largest: cfg.largest,
        lb: cfg.lb,
    };
    let out = extract_unannotated::extract_unannotated(&records, &opts).map_err(|e| e.to_string())?;
    let role = if cfg.fasta_out { "fasta" } else { "genbank" };
    if cfg.fasta_out {
        write_fasta_bp_path(&output, &out)?;
    } else {
        write_genbank(&output, &out).map_err(|e| format!("writing GenBank: {e}"))?;
    }
    let inputs = cfg.input.iter().map(|p| IoRef { path: p.clone(), role: "sequence".into() }).collect();
    Ok((inputs, vec![IoRef { path: output, role: role.into() }], format!("extracted {} regions", out.len())))
}

fn run_color_genbank(params: Value) -> Result<RunOutput, String> {
    let cfg: ColorGenbankConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    if cfg.input.is_empty() {
        return Err("select at least one input file".into());
    }
    let output = cfg.output.clone().ok_or("missing output (GenBank)")?;
    let mut records = load_inputs(&cfg.input)?;
    filter_contigs(&mut records, &cfg.contigs, cfg.contigs_file.as_deref())?;
    let targets = color_genbank::ColorTargets::parse(cfg.mode.as_deref().unwrap_or("cdss")).map_err(|e| e.to_string())?;
    let color_feature = if cfg.domain_database { "database" } else { "name" }.to_string();
    let mut color_map = match &cfg.color_table {
        Some(path) if !path.trim().is_empty() => {
            let text = std::fs::read_to_string(path).map_err(|e| format!("reading color table: {e}"))?;
            let table = color_genbank::read_color_table(&text).map_err(|e| e.to_string())?;
            color_genbank::ColorMap::from_table(table)
        }
        _ => color_genbank::ColorMap::auto(),
    };
    let search_hit_color = match cfg.search_hit_color.as_deref().filter(|c| !c.trim().is_empty()) {
        Some(c) => Some(color_genbank::normalize_color_hex(c).map_err(|e| e.to_string())?),
        None => None,
    };
    let opts = color_genbank::ColorGenbankOptions {
        targets,
        search_hit_color,
        clear: cfg.clear,
        databases: set_of(&cfg.databases),
        color_feature,
    };
    let out = color_genbank::color_genbank(records, &mut color_map, &opts);
    write_genbank(&output, &out).map_err(|e| format!("writing GenBank: {e}"))?;
    let mut outputs = vec![IoRef { path: output, role: "genbank".into() }];
    if let Some(path) = cfg.color_table_out.as_deref().filter(|p| !p.trim().is_empty()) {
        use std::fmt::Write as _;
        let mut t = String::new();
        for (annotation, color) in color_map.items() {
            let _ = writeln!(t, "{annotation}\t{color}");
        }
        std::fs::write(path, t).map_err(|e| format!("writing color table: {e}"))?;
        outputs.push(IoRef { path: path.to_string(), role: "table".into() });
    }
    let inputs = cfg.input.iter().map(|p| IoRef { path: p.clone(), role: "sequence".into() }).collect();
    Ok((inputs, outputs, format!("colored {} records", out.len())))
}

fn run_color_table_to_legend(params: Value) -> Result<RunOutput, String> {
    let cfg: ColorTableToLegendConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    if cfg.input.is_empty() {
        return Err("select at least one color-table file".into());
    }
    let svg_path = cfg.svg.clone().ok_or("missing output (SVG)")?;
    let mut table: Vec<(String, String)> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for p in &cfg.input {
        let text = std::fs::read_to_string(p).map_err(|e| format!("reading {p}: {e}"))?;
        let entries = color_genbank::read_color_table(&text).map_err(|e| e.to_string())?;
        for (k, v) in entries {
            if let Some(&i) = index.get(&k) {
                table[i].1 = v;
            } else {
                index.insert(k.clone(), table.len());
                table.push((k, v));
            }
        }
    }
    let title = cfg.title.as_deref().filter(|t| !t.trim().is_empty()).unwrap_or("Legend");
    let svg = color_table_to_legend::color_table_to_legend(&table, title);
    std::fs::write(&svg_path, svg).map_err(|e| format!("writing {svg_path}: {e}"))?;
    let inputs = cfg.input.iter().map(|p| IoRef { path: p.clone(), role: "table".into() }).collect();
    Ok((inputs, vec![IoRef { path: svg_path, role: "svg".into() }], format!("legend with {} entries", table.len())))
}

fn run_deduplicate(params: Value) -> Result<RunOutput, String> {
    let cfg: DeduplicateConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    if cfg.input.is_empty() {
        return Err("select at least one input file".into());
    }
    let output = cfg.output.clone().ok_or("missing output")?;
    if cfg.prefix_count && cfg.suffix_count {
        return Err("prefix_count and suffix_count are mutually exclusive".into());
    }
    let id = cfg.id.ok_or("an identity threshold (id) is required")?;
    let fasta_type = cfg.fasta_type.clone().unwrap_or_else(|| "protein".into());
    let mut records = Vec::new();
    for p in &cfg.input {
        records.extend(load_sequence(p, &fasta_type)?);
    }
    let opts = deduplicate_genbank::DeduplicateOptions {
        algorithm: cfg.algorithm.clone().unwrap_or_else(|| "cd-hit".into()),
        id,
        both_strands: cfg.both_strands,
        fasta_type: fasta_type.clone(),
        cpu: cfg.cpu.unwrap_or(0),
        bin_path: cfg.bin_path.clone().filter(|s| !s.trim().is_empty()),
        params: Vec::new(),
    };
    let result = deduplicate_genbank::deduplicate(&records, &opts).map_err(|e| e.to_string())?;
    let count_by_rep: HashMap<usize, usize> = result.clusters.iter().map(|(r, m)| (*r, m.len())).collect();
    let mut final_name: HashMap<usize, String> = HashMap::new();
    let mut out_records: Vec<SeqRecord> = Vec::new();
    for &i in &result.representatives {
        let mut rec = records[i].clone();
        let base = rec.name.clone();
        let count = count_by_rep.get(&i).copied().unwrap_or(1);
        let newname = if cfg.prefix_count {
            format!("{count}-{base}")
        } else if cfg.suffix_count {
            format!("{base}-{count}")
        } else {
            base
        };
        rec.name = newname.clone();
        if cfg.fasta_out {
            rec.id = newname.clone();
        }
        final_name.insert(i, newname);
        out_records.push(rec);
    }
    let role = if cfg.fasta_out { "fasta" } else { "genbank" };
    if cfg.fasta_out {
        write_fasta_bp_path(&output, &out_records)?;
    } else {
        write_genbank(&output, &out_records).map_err(|e| format!("writing GenBank: {e}"))?;
    }
    let mut outputs = vec![IoRef { path: output, role: role.into() }];
    if let Some(table_path) = cfg.cluster_table.as_deref().filter(|p| !p.trim().is_empty()) {
        let sep = cfg.cluster_sep.clone().unwrap_or_else(|| " ; ".into());
        let mut t = String::from("representative\tcontigs\n");
        for (rep, members) in &result.clusters {
            let rep_name = final_name.get(rep).cloned().unwrap_or_default();
            let names: Vec<String> = members.iter().map(|&m| records[m].name.clone()).collect();
            t.push_str(&format!("{rep_name}\t{}\n", names.join(&sep)));
        }
        std::fs::write(table_path, t).map_err(|e| format!("writing cluster table: {e}"))?;
        outputs.push(IoRef { path: table_path.to_string(), role: "table".into() });
    }
    let inputs = cfg.input.iter().map(|p| IoRef { path: p.clone(), role: "sequence".into() }).collect();
    Ok((inputs, outputs, format!("{} in → {} representatives", records.len(), out_records.len())))
}

fn run_hmmer_compare(params: Value) -> Result<RunOutput, String> {
    let cfg: HmmerCompareConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    if cfg.input.is_empty() || cfg.reference.is_empty() {
        return Err("select query and reference HMM file(s)".into());
    }
    let output = cfg.output.clone().ok_or("missing output")?;
    let opts = hmmer_compare_m::HmmerCompareOptions {
        score_cutoff: cfg.score_cutoff.unwrap_or(0.0),
        alignments: cfg.alignments,
        k: cfg.k,
        cpu: cfg.cpu.unwrap_or(0),
        max_output_gb: Some(cfg.max_output_gb.unwrap_or(25.0)),
    };
    let mut buf: Vec<u8> = Vec::new();
    hmmer_compare_m::hmmer_compare(&cfg.input, &cfg.reference, &opts, &mut buf).map_err(|e| e.to_string())?;
    std::fs::write(&output, &buf).map_err(|e| format!("writing {output}: {e}"))?;
    let mut inputs: Vec<IoRef> = cfg.input.iter().map(|p| IoRef { path: p.clone(), role: "hmm".into() }).collect();
    inputs.extend(cfg.reference.iter().map(|p| IoRef { path: p.clone(), role: "reference".into() }));
    Ok((inputs, vec![IoRef { path: output, role: "table".into() }], "wrote comparison table".into()))
}

fn run_hmmer_search(params: Value) -> Result<RunOutput, String> {
    let cfg: HmmerSearchConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    if cfg.input.is_empty() || cfg.reference.is_empty() {
        return Err("select query and reference HMM file(s)".into());
    }
    let output = cfg.output.clone().ok_or("missing output (.hmm)")?;
    let opts = hmmer_search_m::HmmerSearchOptions {
        score_cutoff: cfg.score_cutoff.unwrap_or(10.0),
        max_hits: cfg.max_hits,
        cpu: cfg.cpu.unwrap_or(0),
        max_output_gb: cfg.max_output_gb,
    };
    let mut f = std::io::BufWriter::new(
        std::fs::File::create(&output).map_err(|e| format!("creating {output}: {e}"))?,
    );
    let found = hmmer_search_m::hmmer_search(&cfg.input, &cfg.reference, &opts, &mut f).map_err(|e| e.to_string())?;
    std::io::Write::flush(&mut f).map_err(|e| e.to_string())?;
    let mut inputs: Vec<IoRef> = cfg.input.iter().map(|p| IoRef { path: p.clone(), role: "hmm".into() }).collect();
    inputs.extend(cfg.reference.iter().map(|p| IoRef { path: p.clone(), role: "reference".into() }));
    Ok((inputs, vec![IoRef { path: output, role: "hmm".into() }], format!("kept {found} query profiles")))
}

fn run_hmmer_report(params: Value) -> Result<RunOutput, String> {
    let cfg: HmmerReportConfig = serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    if cfg.input.is_empty() {
        return Err("select at least one input .hmm file".into());
    }
    let output = cfg.output.clone().ok_or("missing output")?;
    // The declarative columns → core analyses (leading `name` is always emitted).
    let mut analyses = Vec::new();
    for c in &cfg.columns {
        analyses.push(c.to_analysis().map_err(|e| e.to_string())?);
    }
    let format = if cfg.json.is_some() { hmmer_report_m::OutputFormat::Json } else { hmmer_report_m::OutputFormat::Tsv };
    let records = hmmer_report_m::read_hmms_to_iterators(&cfg.input).map_err(|e| e.to_string())?;
    let col_names = cfg.column_names.as_deref();
    let mut f = std::io::BufWriter::new(
        std::fs::File::create(&output).map_err(|e| format!("creating {output}: {e}"))?,
    );
    hmmer_report_m::hmmer_report(&records, &analyses, col_names, format, &mut f).map_err(|e| e.to_string())?;
    std::io::Write::flush(&mut f).map_err(|e| e.to_string())?;
    let n_profiles: usize = records.iter().map(|(_, h)| h.len()).sum();
    let inputs = cfg.input.iter().map(|p| IoRef { path: p.clone(), role: "hmm".into() }).collect();
    Ok((
        inputs,
        vec![IoRef { path: output, role: "table".into() }],
        format!("reported {n_profiles} profiles × {} columns", analyses.len() + 1),
    ))
}

fn str_param<'a>(params: &'a Value, key: &str) -> Result<&'a str, String> {
    params.get(key).and_then(|v| v.as_str()).ok_or_else(|| format!("missing {key}"))
}

fn basename(path: &str) -> String {
    Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn run_export_enum_report(params: Value) -> Result<RunOutput, String> {
    let input = str_param(&params, "input")?.to_string();
    let output = str_param(&params, "output")?.to_string();
    let (by, default_names): (By, &[&str]) = match params.get("by").and_then(|v| v.as_str()) {
        Some("contig") => (By::Contig, &["length", "cds_count", "domains"]),
        Some("domain") => (By::Domain, &["start", "end", "strand", "score", "identity"]),
        _ => (By::Cds, &["domains", "architecture", "length"]),
    };
    // Honor an explicit `columns` selection from the form; else the per-`by` default.
    let chosen: Vec<String> = params
        .get("columns")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let names: Vec<String> = if chosen.is_empty() {
        default_names.iter().map(|s| s.to_string()).collect()
    } else {
        chosen
    };
    let analyses: Vec<_> = names.iter().filter_map(|n| static_analysis(n)).collect();
    if analyses.is_empty() {
        return Err("no valid columns selected".into());
    }
    let records = load_sequence(&input, "protein")?;
    let fname = basename(&input);
    let recs: Vec<_> = records.into_iter().map(|r| (r, fname.clone())).collect();
    let mut buf: Vec<u8> = Vec::new();
    enum_report(recs, by, &analyses, None, None, None, OutputFormat::Tsv, &mut buf)?;
    std::fs::write(&output, &buf).map_err(|e| format!("writing report: {e}"))?;
    Ok((
        vec![IoRef { path: input, role: "sequence".into() }],
        vec![IoRef { path: output, role: "report".into() }],
        format!("wrote enum report ({} bytes)", buf.len()),
    ))
}

fn run_export_matrix_report(params: Value) -> Result<RunOutput, String> {
    let input = str_param(&params, "input")?.to_string();
    let output = str_param(&params, "output")?.to_string();
    let m = read_matrix(&input).map_err(|e| format!("reading matrix: {e}"))?;
    let report = matrix_report(&m, DEFAULT_MERGE_IMPACT_METRIC, DEFAULT_MAX_MERGE);
    std::fs::write(&output, render_text(&report)).map_err(|e| format!("writing report: {e}"))?;
    Ok((
        vec![IoRef { path: input, role: "matrix".into() }],
        vec![IoRef { path: output, role: "report".into() }],
        format!("wrote matrix report ({} nodes, {} edges)", report.n_nodes, report.n_edges),
    ))
}

fn run_transform_matrix(params: Value) -> Result<RunOutput, String> {
    let cfg: TransformMatrixConfig =
        serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    let input = cfg.input.clone().ok_or("missing input matrix")?;
    let opts = resolve_transform_matrix(&cfg).map_err(|e| e.to_string())?;
    let m = read_matrix(&input).map_err(|e| format!("reading matrix: {e}"))?;
    let m = transform_matrix(m, &opts).map_err(|e| format!("transform_matrix: {e}"))?;

    let mut outputs = Vec::new();
    if let Some(p) = &cfg.dense {
        hdf5_io::write_dense(p, &m).map_err(|e| format!("writing dense: {e}"))?;
        outputs.push(IoRef { path: p.clone(), role: "dense".into() });
    }
    if let Some(p) = &cfg.dense_text {
        text::write_dense_text(p, &m).map_err(|e| format!("writing dense_text: {e}"))?;
        outputs.push(IoRef { path: p.clone(), role: "dense_text".into() });
    }
    if let Some(p) = &cfg.sparse {
        hdf5_io::write_sparse(p, &m).map_err(|e| format!("writing sparse: {e}"))?;
        outputs.push(IoRef { path: p.clone(), role: "sparse".into() });
    }
    if outputs.is_empty() {
        return Err("no output specified (set dense, dense_text, or sparse)".into());
    }
    let inputs = vec![IoRef { path: input, role: "matrix".into() }];
    Ok((inputs, outputs, format!("wrote {}×{} matrix", m.nrows(), m.ncols())))
}

fn run_build_ssn(params: Value) -> Result<RunOutput, String> {
    let cfg: BuildSsnConfig =
        serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    let input = cfg.input.clone().ok_or("missing input matrix")?;
    let xgmml = cfg.xgmml.clone().ok_or("missing output (--xgmml path)")?;
    if cfg.mst && cfg.mst_knn.is_some() {
        return Err("mst and mst_knn are mutually exclusive".into());
    }
    // Resolve the optional colour table (name→color TSV); subset files aren't
    // exposed in the GUI form yet, so pass none for those.
    let color_table = match cfg.color_table.as_deref().filter(|p| !p.trim().is_empty()) {
        Some(path) => {
            let text = std::fs::read_to_string(path).map_err(|e| format!("reading color table: {e}"))?;
            let table = color_genbank::read_color_table(&text).map_err(|e| e.to_string())?;
            Some(table.into_iter().collect::<std::collections::HashMap<_, _>>())
        }
        None => None,
    };
    let opts = resolve_build_ssn(&cfg, None, color_table);
    let m = read_matrix(&input).map_err(|e| format!("reading matrix: {e}"))?;
    let result = build_ssn(&m, &opts).map_err(|e| format!("build_ssn: {e}"))?;

    let name =
        Path::new(&xgmml).file_stem().and_then(|s| s.to_str()).unwrap_or("network").to_string();
    let mut buf: Vec<u8> = Vec::new();
    cytoscape::write_xgmml_to(&mut buf, &name, &result.table, &result.edges, result.node_fills.as_deref())
        .map_err(|e| format!("writing XGMML: {e}"))?;
    std::fs::write(&xgmml, &buf).map_err(|e| format!("writing {xgmml}: {e}"))?;
    let mut outputs = vec![IoRef { path: xgmml, role: "xgmml".into() }];

    if let Some(path) = &cfg.cluster_tsv {
        let labels = result.cluster_labels.as_ref().ok_or("no cluster labels (set cluster)")?;
        let mut out = String::new();
        if !cfg.no_cluster_header {
            out.push_str("contig\tcluster\n");
        }
        for (name, id) in result.table.names.iter().zip(labels) {
            out.push_str(name);
            out.push('\t');
            out.push_str(&id.to_string());
            out.push('\n');
        }
        std::fs::write(path, out).map_err(|e| format!("writing cluster tsv: {e}"))?;
        outputs.push(IoRef { path: path.clone(), role: "cluster_tsv".into() });
    }

    if let Some(path) = cfg.color_table_out.as_deref().filter(|p| !p.trim().is_empty()) {
        let ct = result.resolved_color_table.as_ref().ok_or("no colour table resolved (set color_by)")?;
        let mut items: Vec<(&String, &String)> = ct.iter().collect();
        items.sort_by(|a, b| a.0.cmp(b.0));
        let mut out = String::new();
        for (k, v) in items {
            out.push_str(k);
            out.push('\t');
            out.push_str(v);
            out.push('\n');
        }
        std::fs::write(path, out).map_err(|e| format!("writing color table: {e}"))?;
        outputs.push(IoRef { path: path.to_string(), role: "table".into() });
    }

    let inputs = vec![IoRef { path: input, role: "matrix".into() }];
    Ok((inputs, outputs, format!("wrote {} nodes, {} edges", result.table.names.len(), result.edges.len())))
}

fn run_build_ssn_viewer(params: Value) -> Result<RunOutput, String> {
    let cfg: BuildSsnViewerConfig =
        serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    let input = cfg.input.clone().ok_or("missing input matrix")?;
    let output = cfg.output.clone().ok_or("missing output (.dsnv path)")?;
    let name = Path::new(&output)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("SSN")
        .to_string();
    let opts = resolve_build_ssn_viewer(&cfg, name, None);
    let m = read_matrix(&input).map_err(|e| format!("reading matrix: {e}"))?;
    let bundle = build_ssn_viewer_bundle(&m, &opts).map_err(|e| format!("building bundle: {e}"))?;
    let compressed = gzip(&serialize_bundle(&bundle));
    std::fs::write(&output, &compressed).map_err(|e| format!("writing .dsnv: {e}"))?;

    let inputs = vec![IoRef { path: input, role: "matrix".into() }];
    let outputs = vec![IoRef { path: output, role: "dsnv".into() }];
    Ok((inputs, outputs, format!("wrote {}-node .dsnv bundle", m.nrows())))
}
