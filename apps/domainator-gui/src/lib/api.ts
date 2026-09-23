import { invoke } from "@tauri-apps/api/core";
import type {
  ContigFileView,
  DatabaseEntry,
  ExternalToolStatus,
  FileEntry,
  FolderNode,
  HmmLogo,
  HmmSummary,
  JobInfo,
  MatrixReport,
  ReferenceDbList,
  InstalledDb,
  RunResult,
  SourceTrace,
  SummaryStats,
  ToolMeta,
  WorkspaceInfo,
  Prefs,
  PrefsPatch,
  UpdateCheck,
} from "./types";

export const pickDirectory = (): Promise<string | null> =>
  invoke("pick_directory");

/// Native multi-file picker for import (null if canceled).
export const pickFiles = (): Promise<string[] | null> => invoke("pick_files");

/// Copy `sources` into workspace folder `dir`; returns the imported paths.
export const importFiles = (
  workspaceRoot: string,
  dir: string,
  sources: string[],
): Promise<string[]> => invoke("import_files", { workspaceRoot, dir, sources });

export const openWorkspace = (root: string): Promise<WorkspaceInfo> =>
  invoke("open_workspace", { root });

export const listFolder = (workspaceRoot: string, dir: string): Promise<FileEntry[]> =>
  invoke("list_folder", { workspaceRoot, dir });

export const rescanTree = (root: string): Promise<FolderNode> =>
  invoke("rescan_tree", { root });

export const createFolder = (parent: string, name: string): Promise<string> =>
  invoke("create_folder", { parent, name });

export const deleteFile = (workspaceRoot: string, path: string): Promise<void> =>
  invoke("delete_file", { workspaceRoot, path });

export const loadContigView = (path: string): Promise<ContigFileView> =>
  invoke("load_contig_view", { path });

export const loadMatrixReport = (path: string): Promise<MatrixReport> =>
  invoke("load_matrix_report", { path });

export const loadHmmLogo = (path: string, index?: number): Promise<HmmLogo> =>
  invoke("load_hmm_logo", { path, index: index ?? null });

export const listHmmProfiles = (path: string): Promise<HmmSummary[]> =>
  invoke("list_hmm_profiles", { path });

/// Uppercased sequences for contigs [start, end) (server-cached; batched by the
/// viewer's visible window so scrolling doesn't re-parse the file per contig).
export const getContigSequences = (
  path: string,
  start: number,
  end: number,
): Promise<string[]> => invoke("get_contig_sequences", { path, start, end });

/// Read a text file (TSV/CSV) as a string; write it back in place (table editor).
export const readTextFile = (path: string): Promise<string> =>
  invoke("read_text_file", { path });
export const writeTextFile = (path: string, contents: string): Promise<void> =>
  invoke("write_text_file", { path, contents });

/// Extract selected contigs / HMM profiles (0-based indices) into a new file
/// `dir/name`; returns the new path.
export const extractContigs = (
  dir: string,
  name: string,
  source: string,
  indices: number[],
): Promise<string> => invoke("extract_contigs", { dir, name, source, indices });
export const extractHmms = (
  dir: string,
  name: string,
  source: string,
  indices: number[],
): Promise<string> => invoke("extract_hmms", { dir, name, source, indices });

/// Extract a coordinate region [start, end) of contig `index` to a new GenBank
/// file (preserves the features in the region, like select-by-coord). With
/// `translate`, the region's CDSs are converted to a protein GenBank via
/// extract_peptides.
export const extractRegion = (
  dir: string,
  name: string,
  source: string,
  index: number,
  start: number,
  end: number,
  translate: boolean,
): Promise<string> => invoke("extract_region", { dir, name, source, index, start, end, translate });

/// Create a new text file `dir/name` with `contents`; returns the new path.
export const createTextFile = (dir: string, name: string, contents: string): Promise<string> =>
  invoke("create_text_file", { dir, name, contents });

/// Record counts for the given files (lazy; cached server-side). null = no
/// meaningful count for that type (matrix/table) or unreadable.
export const countRecords = (paths: string[]): Promise<(number | null)[]> =>
  invoke("count_records", { paths });

/// Rename a data file in place; returns the new path.
export const renameFile = (
  workspaceRoot: string,
  path: string,
  newName: string,
): Promise<string> => invoke("rename_file", { workspaceRoot, path, newName });

/// Native "Save As" dialog + write text to disk. Returns the saved path, or null
/// if the user canceled.
export const saveTextFile = (
  defaultName: string,
  extension: string,
  filterName: string,
  contents: string,
): Promise<string | null> =>
  invoke("save_text_file", { defaultName, extension, filterName, contents });

/// Native "Save As" dialog + write binary data to disk. `contentsBase64` is the
/// payload as standard base64 (no data: prefix) — the IPC bridge is JSON, so a
/// raw byte array would cost ~4x the bytes and a lot of parse time for a
/// multi-megabyte PNG. Returns the saved path, or null if the user canceled.
export const saveBinaryFile = (
  defaultName: string,
  extension: string,
  filterName: string,
  contentsBase64: string,
): Promise<string | null> =>
  invoke("save_binary_file", { defaultName, extension, filterName, contentsBase64 });

export type EnumRow = Record<string, string | number | null>;

export const runEnumReport = (
  path: string,
  by: string,
  columns: string[],
): Promise<EnumRow[]> => invoke("run_enum_report", { path, by, columns });

export const runSummaryStats = (path: string): Promise<SummaryStats> =>
  invoke("run_summary_stats", { path });

export const renderSsnHtml = (path: string): Promise<string> =>
  invoke("render_ssn_html", { path });

export interface ResolvedRecord {
  path: string;
  file_type: string;
  contig_index: number;
  contig_id: string;
}

/// Resolve many SSN node labels to their source records, searching ONLY `files`
/// (the network's provenance-traced sequence sources). Aligned to `labels` (null
/// for any not found).
export const resolveRecords = (
  files: string[],
  labels: string[],
): Promise<(ResolvedRecord | null)[]> => invoke("resolve_records", { files, labels });

/// Trace a network artifact (.dsnv/matrix) back to the sequence files that
/// supplied its node labels (provenance DAG walk).
export const traceNetworkSources = (path: string): Promise<SourceTrace> =>
  invoke("trace_network_sources", { path });

// --- detached sequence windows (SSN node -> new window) ---
export const openSequenceWindow = (
  path: string,
  contigIndex: number,
  title: string,
): Promise<void> => invoke("open_sequence_window", { path, contigIndex, title });

export const standaloneParams = (label: string): Promise<[string, number] | null> =>
  invoke("standalone_params", { label });

// Pop a whole file (its full viewer + sub-tabs) out into a detached window.
export const openFileWindow = (path: string, title: string): Promise<void> =>
  invoke("open_file_window", { path, title });

// Pop out a single HMM profile's logo in a detached window.
export const openHmmLogoWindow = (
  path: string,
  index: number,
  title: string,
): Promise<void> => invoke("open_hmm_logo_window", { path, index, title });

// Build a single GenBank/FASTA of the selected records as a real workspace file
// `dir/name` (SSN "Extract selected"). Returns the new path.
export const extractRecords = (
  dir: string,
  name: string,
  records: { path: string; contig_index: number }[],
): Promise<string> => invoke("extract_records", { dir, name, records });

// --- databases ---
export const pickFile = (): Promise<string | null> => invoke("pick_file");
export const listDatabases = (workspaceRoot: string): Promise<DatabaseEntry[]> =>
  invoke("list_databases", { workspaceRoot });
export const registerDatabase = (workspaceRoot: string, path: string): Promise<DatabaseEntry> =>
  invoke("register_database", { workspaceRoot, path });
export const unregisterDatabase = (workspaceRoot: string, path: string): Promise<void> =>
  invoke("unregister_database", { workspaceRoot, path });

// --- contig annotation editing (GenBank write-back) ---
export const setFeatureColor = (
  path: string,
  contigIndex: number,
  featIndex: number,
  hex: string,
): Promise<void> => invoke("set_feature_color", { path, contigIndex, featIndex, hex });

export const removeFeature = (
  path: string,
  contigIndex: number,
  featIndex: number,
): Promise<void> => invoke("remove_feature", { path, contigIndex, featIndex });

export const addFeature = (
  path: string,
  contigIndex: number,
  kind: string,
  start: number,
  end: number,
  strand: number,
  qualifiers: [string, string][],
): Promise<void> => invoke("add_feature", { path, contigIndex, kind, start, end, strand, qualifiers });

export const listTools = (): Promise<ToolMeta[]> => invoke("list_tools");

export const toolSchema = (tool: string): Promise<string> =>
  invoke("tool_schema", { tool });

// Tool JSON Schemas are static per tool, so cache them in a module singleton that
// (unlike the viewer cache) is NEVER cleared. Prefetched at app startup — before
// any heavy GenBank viewer is mounted — so opening a tool dialog needs no backend
// round-trip and can't stall behind main-thread work. A rejected fetch is dropped
// so it can be retried.
const schemaCache = new Map<string, Promise<string>>();
export function toolSchemaCached(tool: string): Promise<string> {
  let p = schemaCache.get(tool);
  if (!p) {
    p = toolSchema(tool).catch((e) => {
      schemaCache.delete(tool);
      throw e;
    });
    schemaCache.set(tool, p);
  }
  return p;
}

export const runTool = (
  tool: string,
  params: Record<string, unknown>,
  workspaceRoot: string,
): Promise<RunResult> => invoke("run_tool", { tool, params, workspaceRoot });

export const listJobs = (): Promise<JobInfo[]> => invoke("list_jobs");
export const cancelJob = (jobId: string): Promise<void> => invoke("cancel_job", { jobId });
/// Load persisted job history for a workspace (also sets where new jobs are saved).
export const loadJobs = (workspaceRoot: string): Promise<JobInfo[]> =>
  invoke("load_jobs", { workspaceRoot });
export const deleteJob = (jobId: string): Promise<void> => invoke("delete_job", { jobId });
export const clearJobs = (): Promise<void> => invoke("clear_jobs");

export const listExternalTools = (): Promise<ExternalToolStatus[]> => invoke("list_external_tools");
export const downloadExternalTool = (id: string): Promise<string> =>
  invoke("download_external_tool", { id });

// --- app preferences (recent workspaces / reopen-on-launch) ---

/// What the shell needs on launch: the recent workspaces that still exist, and
/// the one to reopen automatically (null if reopening is off or none are left).
export interface StartupInfo {
  recent: string[];
  reopen: string | null;
}
export const startupInfo = (): Promise<StartupInfo> => invoke("startup_info");
export const forgetRecentWorkspace = (root: string): Promise<string[]> =>
  invoke("forget_recent_workspace", { root });
/// The full preferences. (There was no getter at all before the Preferences
/// dialog existed.)
export const getPrefs = (): Promise<Prefs> => invoke("get_prefs");

/// Apply a partial settings update; returns the saved preferences.
export const setPrefs = (patch: PrefsPatch): Promise<Prefs> => invoke("set_prefs", { patch });

/// Ask GitHub whether a newer release exists. Never rejects for "offline" —
/// that comes back as `note`.
export const checkForUpdates = (): Promise<UpdateCheck> => invoke("check_for_updates");

/// One managed external binary as the About box lists it.
export interface ToolLine {
  name: string;
  version: string;
  installed: boolean;
}

/// Everything the About dialog shows. `diagnostics` is the pre-rendered blob a
/// bug report should carry — the frontend never re-derives it, so what the user
/// pastes always matches what the dialog displays.
export interface AppDiagnostics {
  product: string;
  version: string;
  upstream_version: string;
  upstream_commit: string;
  tauri_version: string;
  webview_version: string;
  os: string;
  arch: string;
  profile: string;
  app_data_dir: string;
  prefs_path: string;
  managed_tools_dir: string;
  external_tools: ToolLine[];
  diagnostics: string;
  project_url: string;
  issues_url: string;
  license: string;
}

export const appDiagnostics = (): Promise<AppDiagnostics> => invoke("app_diagnostics");

/// Open one of the app's own links in the user's browser. The Rust side refuses
/// anything not on its allow-list, so this cannot open an arbitrary URL.
export const openExternalUrl = (url: string): Promise<void> =>
  invoke("open_external_url", { url });

/// Show a file or folder in the OS file manager. Bounded on the Rust side to the
/// open workspace and the app's own data directory.
export const revealInFileManager = (
  workspaceRoot: string | null,
  path: string,
): Promise<void> => invoke("reveal_in_file_manager", { workspaceRoot, path });

export const openAppDataDir = (): Promise<void> => invoke("open_app_data_dir");

// Managed reference databases (UniProt / Pfam). `listReferenceDbs` never touches
// the network; `checkReferenceDbVersion` is the only call that does so on its own,
// and only because the user asked.
export const listReferenceDbs = (): Promise<ReferenceDbList> =>
  invoke("list_reference_dbs");

export const checkReferenceDbVersion = (id: string): Promise<string> =>
  invoke("check_reference_db_version", { id });

export const downloadReferenceDb = (
  id: string,
  workspaceRoot: string | null,
): Promise<InstalledDb> => invoke("download_reference_db", { id, workspaceRoot });

export const cancelReferenceDb = (id: string): Promise<void> =>
  invoke("cancel_reference_db", { id });

export const removeReferenceDb = (
  id: string,
  workspaceRoot: string | null,
): Promise<void> => invoke("remove_reference_db", { id, workspaceRoot });

/// Re-register every installed reference database into a workspace being opened:
/// they live in the app data dir and are shared, but each workspace keeps its own
/// registry file.
export const syncReferenceDbsToWorkspace = (
  workspaceRoot: string,
): Promise<number> => invoke("sync_reference_dbs_to_workspace", { workspaceRoot });

/// What the native menu needs in order to gray items out. Sent from the main
/// window only; `app_menu.rs` ignores it from any other label.
export interface MenuState {
  has_workspace: boolean;
  has_folder: boolean;
  has_tab: boolean;
  can_save: boolean;
  can_find: boolean;
  can_export: boolean;
  has_closed_tab: boolean;
}

export const setMenuState = (state: MenuState): Promise<void> =>
  invoke("set_menu_state", { state });
