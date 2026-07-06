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

export interface ContigView {
  index: number;
  id: string;
  name: string;
  description: string;
  length: number;
  topology: "linear" | "circular";
  is_protein: boolean;
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
  format: "fasta" | "genbank" | "hmm";
  molecule: "protein" | "nucleic" | "unknown";
  shards: number;
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

export interface ToolField {
  name: string;
  label: string;
  role: "input_file" | "input_files" | "output_file" | "param" | "hmm_columns" | "enum_columns";
  file_type: string | null;
  required: boolean;
}

export interface ToolMeta {
  id: string;
  label: string;
  description: string;
  fields: ToolField[];
}

export interface RunResult {
  job_id: string;
  outputs: string[];
  message: string;
}

export interface JobInfo {
  job_id: string;
  tool: string;
  status: "running" | "completed" | "failed" | "cancelled";
  message: string;
  started_at_ms: string;
  outputs: string[];
  params: Record<string, unknown>;
}

// Minimal JSON-Schema shape we render forms from (draft-07 subset).
export interface JsonSchema {
  title?: string;
  description?: string;
  type?: string;
  properties?: Record<string, JsonSchemaProp>;
  required?: string[];
}

export interface JsonSchemaProp {
  type?: string | string[];
  description?: string;
  default?: unknown;
  enum?: (string | null)[];
  items?: { type?: string };
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
