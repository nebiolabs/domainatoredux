// Mirrors the serde DTOs in domainator-core::app_support (workspace + contig_view)
// and the backend command return types.

export interface FolderNode {
  path: string;
  name: string;
  children: FolderNode[];
}

export interface Settings {
  default_cpu: number;
}

export interface WorkspaceInfo {
  root: string;
  tree: FolderNode;
  settings: Settings;
}

export type FileType =
  | "genbank"
  | "fasta"
  | "hmm"
  | "hdf5"
  | "cm"
  | "ssn_viewer"
  | "table"
  | "text"
  | "svg";

/// `app_support::export`: how exported files are packaged.
export type ExportPackaging = "plain" | "gzip" | "zip";

export interface ExportOptions {
  packaging: ExportPackaging;
  split_records: boolean;
}

/// `app_support::export::ExportOutcome`.
export type ExportOutcome =
  | { kind: "conflicts"; names: string[] }
  | { kind: "done"; written: number; dest: string };

export interface FileEntry {
  path: string;
  name: string;
  file_type: FileType;
  description: string;
  size: number;
  created_ms: number | null;
  modified_ms: number | null;
  created_by_job: string | null;
}

export interface PartView {
  start: number;
  end: number;
  strand: number;
  before: boolean;
  after: boolean;
}

// One sequence source file a network was traced back to (SSN "Extract selected").
export interface SourceFile {
  path: string;
  present: boolean;
  modified_since: boolean;
}

export interface SourceTrace {
  has_provenance: boolean;
  sources: SourceFile[];
}

export interface FeatureView {
  feat_index: number;
  kind: string;
  label: string;
  parts: PartView[];
  start: number;
  end: number;
  strand: number;
  color: string;
  color_source: "qualifier" | "default";
  database: string | null;
  evalue: number | null;
  score: number | null;
  identity: number | null;
  cds_id: string | null;
  qualifiers: [string, string][];
}

// One row of a record's header summary; `detail` rows (references, comments) are
// shown only in the right-click inspector, not the hover.
export interface InfoRow {
  key: string;
  value: string;
  detail: boolean;
}

export interface ContigView {
  index: number;
  id: string;
  name: string; // VERSION (or ACCESSION) for GenBank; the id for FASTA
  description: string;
  definition: string; // description without a leading copy of the id
  accession: string | null;
  source: string | null; // SOURCE, else ORGANISM
  length: number;
  topology: "linear" | "circular";
  is_protein: boolean;
  info: InfoRow[];
  features: FeatureView[];
}

export interface ContigFileView {
  path: string;
  molecule_type: string | null;
  contigs: ContigView[];
  databases: string[];
  feature_kinds: string[];
  evalue_range: [number, number] | null;
}

// A file opened in the lower viewer dock, plus which view tab is active.
export interface OpenTab {
  id: string;
  file: FileEntry;
}

export interface LogoLetter {
  char: string;
  height: number;
  prob: number;
}

export interface LogoPosition {
  index: number;
  total_ic: number;
  occupancy: number;
  insert_prob: number;
  delete_prob: number;
  insert_length: number;
  letters: LogoLetter[];
}

export interface HmmSummary {
  index: number;
  name: string;
  acc: string | null;
  desc: string | null;
  length: number;
  alphabet: "amino" | "dna" | "rna";
  /** `NSEQ`: sequences in the training alignment. */
  nseq: number | null;
  /** `EFFN`: effective number of sequences after relative weighting. */
  effn: number | null;
  /** Pfam cutoffs (`GA`/`TC`/`NC`) as `[per-sequence, per-domain]` bit scores. */
  ga: [number, number] | null;
  tc: [number, number] | null;
  nc: [number, number] | null;
}

export interface HmmLogo {
  name: string;
  acc: string | null;
  desc: string | null;
  alphabet: "amino" | "dna" | "rna";
  length: number;
  max_height_bits: number;
  positions: LogoPosition[];
}

export interface NameCount {
  name: string;
  count: number;
}

export interface TaxNode {
  name: string;
  count: number;
  children: TaxNode[];
}

export interface SummaryStats {
  n_contigs: number;
  is_protein: boolean;
  total_residues: number;
  length_min: number;
  length_max: number;
  length_mean: number;
  length_median: number;
  length_histogram: [number, number][];
  n_cds: number;
  /** CDSs with a `<`/`>` partial end. */
  n_partial_cds: number;
  /** Protein records that are fragments (UniProt `(Fragment)`, `NON_TER`, a partial CDS/Protein feature). */
  n_fragment_proteins: number;
  n_domain_features: number;
  distinct_domains: number;
  top_domains: NameCount[];
  databases: NameCount[];
  feature_kinds: NameCount[];
  top_organisms: NameCount[];
  n_with_taxonomy: number;
  taxonomy: TaxNode;
}

export interface SuggestedThreshold {
  threshold: number | null;
  threshold_label: string;
  cluster_count: number;
  merge_impact: number | null;
}

// Payload from `load_matrix_report`: the parity-locked `to_json` fields plus the
// GUI-only histogram / matrix-shape extras.
export interface DatabaseEntry {
  path: string;
  name: string;
  format: "fasta" | "genbank" | "hmm" | "taxonomy";
  molecule: "protein" | "nucleic" | "unknown";
  shards: number;
}

/// `registry::IndexStatus`: whether a database has its `.didx` offset index.
export type IndexStatus = "indexed" | "stale" | "missing" | "unindexable" | "not_applicable";

/// One row of the Databases panel (`cmd_db::DatabaseRow`): the registry entry
/// plus what is true of it on disk right now.
export interface DatabaseRow extends DatabaseEntry {
  index: IndexStatus;
  /// A managed reference download (Reference Databases panel), not user-added.
  managed: boolean;
  /// That download's id (`reference_dbs.rs`), named by a spec's `select: database:<id>`.
  managed_id: string | null;
  on_disk: boolean;
}

export interface ExternalToolStatus {
  id: string;
  bin_name: string;
  version: string;
  description: string;
  installed: boolean;
  path: string | null;
  available: boolean;
  platform: string;
}

// A managed reference database (UniProt / Pfam), mirroring `cmd_refdb.rs`.

export interface InstalledDb {
  id: string;
  /// The publisher's release string as of the download (`2026_03`, `38.2`).
  version: string;
  path: string;
  bytes: number;
  installed_at_ms: string;
}

export interface ReferenceDbStatus {
  id: string;
  name: string;
  description: string;
  kind: "fasta" | "hmm" | "taxonomy";
  file_name: string;
  url: string;
  download_bytes: number;
  disk_bytes: number;
  /// What must be free to install: `disk_bytes` plus the copy already on disk when
  /// this is an update, since the old file survives until the closing rename.
  required_bytes: number;
  /// False only when the shortfall is certain — an unreadable free-space figure
  /// reports true rather than blocking the download.
  enough_space: boolean;
  needs_confirmation: boolean;
  installed: InstalledDb | null;
  latest_version: string | null;
  checked_at_ms: string | null;
  update_available: boolean;
  downloading: boolean;
}

export interface ReferenceDbList {
  databases: ReferenceDbStatus[];
  /// Free space on the volume holding the managed directory, or null if unknown.
  free_bytes: number | null;
  dir: string;
}

/// The `refdb-progress` event payload.
export interface RefDbProgress {
  id: string;
  /// `downloading` | `indexing` | `registering` | `done` | `failed` | `canceled`.
  phase: string;
  done_bytes: number;
  total_bytes: number;
}

export interface ToolField {
  name: string;
  label: string;
  /// The hint: the spec's `help`, else the manifest parameter's.
  help: string | null;
  role: "input_file" | "input_files" | "output_file" | "param" | "columns";
  /// Inputs: the file types the dropdown offers (the spec's `file_type`); empty otherwise.
  file_types: string[];
  required: boolean;
  /// What the dialog seeds: `tools/<id>.yml`'s `gui_default`, else `cli_default`.
  default: unknown;
  /// The CLI's default (the manifest's), shown as a hint when `default` differs.
  cli_default: unknown;
  /// The manifest parameter's kind.
  kind: "value" | "flag" | "path" | "columns";
  /// Its JSON type: of each value, for a multiple parameter.
  value_type: "string" | "integer" | "number" | "boolean";
  /// It takes a list of values.
  multiple: boolean;
  /// The values it allows, when they are limited.
  choices: unknown[];
  min: number | null;
  max: number | null;
  exclusive_min: number | null;
  /// `columns`: the report's column catalog.
  columns: ColumnInfo[];
  /// Inputs: which group of the dropdown comes first.
  sources: "databases_first" | "workspace_first" | null;
  /// Inputs: `first` | `none` | `database:<managed id>`, for when no selected
  /// file was placed in the field.
  select: string | null;
  /// Inputs: selected files fill inputs in descending priority, ties in spec order.
  prefill_priority: number;
  /// Outputs: the seeded file name, `{stem}` being the primary input's stem.
  output_name: string | null;
  /// `columns`: the spec's per-type overrides for the column builder.
  column_types: Record<string, ColumnTypeSpec>;
}

/// One entry of a manifest `columns` catalog.
export interface ColumnInfo {
  name: string;
  /// Argument names; with `repeat_last`, the last one takes one or more values.
  args: string[];
  repeat_last: boolean;
  help: string;
}

/// `tool_spec_types::ColumnTypeSpec`: how the builder shows one column.
export interface ColumnTypeSpec {
  label: string | null;
  help: string | null;
  show: boolean;
  params: Record<string, ColumnParamSpec>;
}

export interface ColumnParamSpec {
  label: string | null;
  help: string | null;
  default: unknown;
  choices: unknown[];
}

export interface ToolMeta {
  id: string;
  label: string;
  description: string;
  fields: ToolField[];
  /// Menu group, one of `cmd_tools::CATEGORY_LABELS`' labels; `list_tools` returns the
  /// tools sorted by it.
  category: string;
}

export interface RunResult {
  job_id: string;
  outputs: string[];
  message: string;
}

export interface JobInfo {
  job_id: string;
  tool: string;
  /// `queued` until one of `Prefs.max_concurrent_jobs` slots is free. `canceling`
  /// is set the moment the user clicks cancel and lasts while the worker process
  /// is killed and its partial outputs removed (milliseconds), then settles as
  /// `canceled`.
  status: "queued" | "running" | "canceling" | "completed" | "failed" | "canceled";
  message: string;
  started_at_ms: string;
  outputs: string[];
  params: Record<string, unknown>;
}

export interface MatrixReport {
  n_nodes: number;
  n_edges: number;
  stats: { mean: number; median: number; min: number; max: number };
  cluster_count_by_threshold: [number | null, number][];
  edges_by_threshold: [number, number | null][];
  suggested_thresholds: SuggestedThreshold[];
  histogram: [number, number][];
  n_rows: number;
  n_cols: number;
  data_type: string;
  symmetric: boolean;
}

/// Color scheme preference. Mirrors `app_support::prefs::Theme`.
export type Theme = "system" | "light" | "dark";

/// Mirrors `app_support::prefs::Prefs`.
export interface Prefs {
  schema_version: number;
  recent_workspaces: string[];
  reopen_last_workspace: boolean;
  theme: Theme;
  default_cpu: number;
  max_concurrent_jobs: number;
  check_updates_on_launch: boolean;
  last_update_check_ms: number | null;
  last_seen_release: string | null;
  enum_report_columns: EnumReportColumns;
}

/// Mirrors `app_support::prefs::EnumReportColumns`: the Enum Report's default
/// columns per granularity, `null` for the built-in set.
export interface EnumReportColumns {
  contig: string[] | null;
  cds: string[] | null;
  domain: string[] | null;
}

/// Mirrors `app_support::workspace::EnumReportView`: what the Enum Report tab
/// remembers for one file.
export interface EnumReportView {
  by: string | null;
  /// Column tokens per granularity.
  columns: Record<string, string[]>;
}

/// A partial settings update. Omitted fields are left alone.
export interface PrefsPatch {
  reopen_last_workspace?: boolean;
  theme?: Theme;
  default_cpu?: number;
  max_concurrent_jobs?: number;
  check_updates_on_launch?: boolean;
  enum_report_columns?: EnumReportColumns;
}

/// Mirrors `app_support::version_check::UpdateCheck`.
export interface UpdateCheck {
  current: string;
  latest: { tag: string; url: string; published: string | null } | null;
  newer: boolean;
  /// Set when the check could not complete (offline, rate-limited, no releases).
  /// Always shown inline — never on the app-wide error bar.
  note: string | null;
}
