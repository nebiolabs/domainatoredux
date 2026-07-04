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
    schema, BuildSsnConfig, BuildSsnViewerConfig, CompareContigsConfig, DomainSearchConfig,
    DomainateConfig, SeqDistConfig, TransformMatrixConfig,
};
use domainator_core::app_support::provenance::{new_job_id, now_ms, write_job, IoRef, JobRecord};
use domainator_core::io::fasta::read_fasta;
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
use domainator_core::tools::matrix_report::{
    matrix_report, render_text, DEFAULT_MAX_MERGE, DEFAULT_MERGE_IMPACT_METRIC,
};
use domainator_core::tools::neighborhood::{validate_range, Range};
use domainator_core::tools::select_common::parse_name_list;
use domainator_core::tools::seq_dist::{detect_kind, seq_dist, SeqDistOptions};
use domainator_core::seq::taxonomy::NCBITaxonomy;
use domainator_core::tools::transform_matrix::transform_matrix;
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter};

/// Live state of a tool run, tracked in the [`JobRegistry`] managed state and
/// broadcast to the UI via the `job-update` event.
#[derive(Serialize, Clone)]
pub struct JobInfo {
    pub job_id: String,
    pub tool: String,
    /// `running` | `completed` | `failed` | `cancelled`.
    pub status: String,
    pub message: String,
    pub started_at_ms: String, // u128 as string (JSON-safe)
    pub outputs: Vec<String>,
}

/// Managed state: all jobs seen this session + a cancel flag set.
#[derive(Default)]
pub struct JobRegistry {
    jobs: Mutex<HashMap<String, JobInfo>>,
    cancelled: Mutex<std::collections::HashSet<String>>,
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
#[tauri::command]
pub fn list_tools() -> Vec<ToolMeta> {
    vec![
        ToolMeta {
            id: "domain-search".into(),
            label: "Domain search".into(),
            description: "Search sequences with HMM references and extract hit CDSs or their genomic neighborhoods (step 1).".into(),
            fields: vec![
                f("input", "Input sequences", "input_file", Some("sequence"), true),
                f("references", "HMM references", "input_files", Some("hmm"), true),
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
                f("decoys_file", "Decoys file", "param", None, false),
                f("include_taxids", "Include taxids", "param", None, false),
                f("exclude_taxids", "Exclude taxids", "param", None, false),
                f("taxonomy_expr", "Taxonomy expr", "param", None, false),
                f("ncbi_taxonomy_path", "Taxdump dir", "param", None, false),
            ],
        },
        ToolMeta {
            id: "domainate".into(),
            label: "Domainate".into(),
            description: "Annotate sequences with domains from HMM references (step 2).".into(),
            fields: vec![
                f("input", "Input sequences", "input_file", Some("sequence"), true),
                f("references", "HMM references", "input_files", Some("hmm"), true),
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
                f("dense", "Output (matrix)", "output_file", None, true),
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
                f("dense", "Output (matrix)", "output_file", None, true),
                f("ji", "Jaccard weight", "param", None, false),
                f("ai", "Adjacency weight", "param", None, false),
                f("k", "kNN k", "param", None, false),
                f("lb", "Lower bound", "param", None, false),
                f("mst_knn", "MST-kNN k", "param", None, false),
            ],
        },
        ToolMeta {
            id: "enum-report".into(),
            label: "Enum report (export)".into(),
            description: "Export a per-contig/cds/domain TSV report from a sequence file.".into(),
            fields: vec![
                f("input", "Input sequences", "input_file", Some("sequence"), true),
                f("output", "Output (TSV)", "output_file", None, true),
                f("by", "By", "param", None, false),
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
                f("dense", "Output (matrix)", "output_file", None, true),
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
                f("cluster", "Cluster", "param", None, false),
                f("cluster_tsv", "Cluster TSV", "output_file", None, false),
                f("lb", "Lower bound", "param", None, false),
                f("mst", "MST only", "param", None, false),
                f("mst_knn", "MST-kNN k", "param", None, false),
            ],
        },
        ToolMeta {
            id: "build-ssn-viewer".into(),
            label: "Build SSN viewer".into(),
            description: "Build a .ssnv bundle for the interactive network viewer.".into(),
            fields: vec![
                f("input", "Input matrix", "input_file", Some("hdf5"), true),
                f("output", "Output (.ssnv)", "output_file", None, true),
                f("color_by", "Color by", "param", None, false),
                f("label_by", "Label by", "param", None, false),
                f("merge_impact_metric", "Merge metric", "param", None, false),
                f("max_merge_events", "Max merge events", "param", None, false),
            ],
        },
    ]
}

/// The draft-07 JSON Schema for a tool's parameters (drives the form).
#[tauri::command]
pub fn tool_schema(tool: String) -> Result<String, String> {
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
    };
    match outcome {
        _ if cancelled => {
            info.status = "cancelled".into();
            info.message = "cancelled".into();
            set_job(&registry, &app, info.clone());
            return Err("job cancelled".into());
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

/// Record a job's state and broadcast it to the UI.
fn set_job(registry: &JobRegistry, app: &AppHandle, info: JobInfo) {
    registry.jobs.lock().unwrap().insert(info.job_id.clone(), info.clone());
    let _ = app.emit("job-update", &info);
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
    let hits = domain_search(&[input.clone()], &dbs, &opts).map_err(|e| format!("domain_search: {e}"))?;
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
    let output = cfg.dense.clone().ok_or("missing output (matrix .hdf5)")?;
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
        cpu: cfg.cpu.unwrap_or(0),
    };
    let matrix = seq_dist(&input, input_kind, cfg.reference.as_deref(), ref_kind, &opts)
        .map_err(|e| format!("seq_dist: {e}"))?;
    hdf5_io::write_dense(&output, &matrix).map_err(|e| format!("writing matrix: {e}"))?;
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
    let output = cfg.dense.clone().ok_or("missing output (matrix .hdf5)")?;
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
        cpu: cfg.cpu.unwrap_or(0),
    };
    let matrix = compare_contigs(&cfg.input, &opts).map_err(|e| format!("compare_contigs: {e}"))?;
    hdf5_io::write_dense(&output, &matrix).map_err(|e| format!("writing matrix: {e}"))?;
    let inputs = cfg.input.iter().map(|p| IoRef { path: p.clone(), role: "genbank".into() }).collect();
    Ok((inputs, vec![IoRef { path: output, role: "matrix".into() }], format!("wrote {}×{} matrix", matrix.nrows(), matrix.ncols())))
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
    let (by, names): (By, &[&str]) = match params.get("by").and_then(|v| v.as_str()) {
        Some("contig") => (By::Contig, &["length", "cds_count", "domains"]),
        Some("domain") => (By::Domain, &["start", "end", "strand", "score", "identity"]),
        _ => (By::Cds, &["domains", "architecture", "length"]),
    };
    let analyses: Vec<_> = names.iter().filter_map(|n| static_analysis(n)).collect();
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
    // GUI form doesn't expose subset/color-table files yet; pass none.
    let opts = resolve_build_ssn(&cfg, None, None);
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

    let inputs = vec![IoRef { path: input, role: "matrix".into() }];
    Ok((inputs, outputs, format!("wrote {} nodes, {} edges", result.table.names.len(), result.edges.len())))
}

fn run_build_ssn_viewer(params: Value) -> Result<RunOutput, String> {
    let cfg: BuildSsnViewerConfig =
        serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
    let input = cfg.input.clone().ok_or("missing input matrix")?;
    let output = cfg.output.clone().ok_or("missing output (.ssnv path)")?;
    let name = Path::new(&output)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("SSN")
        .to_string();
    let opts = resolve_build_ssn_viewer(&cfg, name, None);
    let m = read_matrix(&input).map_err(|e| format!("reading matrix: {e}"))?;
    let bundle = build_ssn_viewer_bundle(&m, &opts).map_err(|e| format!("building bundle: {e}"))?;
    let compressed = gzip(&serialize_bundle(&bundle));
    std::fs::write(&output, &compressed).map_err(|e| format!("writing .ssnv: {e}"))?;

    let inputs = vec![IoRef { path: input, role: "matrix".into() }];
    let outputs = vec![IoRef { path: output, role: "ssnv".into() }];
    Ok((inputs, outputs, format!("wrote {}-node .ssnv bundle", m.nrows())))
}
