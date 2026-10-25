import { invoke } from "@tauri-apps/api/core";
import type {
  ContigFileView,
  DatabaseEntry,
  DatabaseRow,
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
  EnumReportView,
  ExportOptions,
  ExportOutcome,
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

/// Native "Save As" picker that only chooses a path (null if canceled).
export const pickSavePath = (
  defaultName: string,
  filterName: string,
  extensions: string[],
): Promise<string | null> => invoke("pick_save_path", { defaultName, filterName, extensions });

/// Copy `paths` to `dest` (a directory, or the zip archive for `zip` packaging).
/// Without `overwrite`, an export that would replace existing files writes
/// nothing and returns `{kind: "conflicts"}` with their names.
export const exportFiles = (
  paths: string[],
  options: ExportOptions,
  dest: string,
  overwrite: boolean,
): Promise<ExportOutcome> => invoke("export_files", { paths, options, dest, overwrite });

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

/// `cmd_workspace::MoveReport`: `[old, new]` per moved file, `[path, reason]` per failure.
export interface MoveReport {
  moved: [string, string][];
  failed: [string, string][];
}

/// Move data files (with their sidecars and `.didx` indexes) into `destDir`.
export const moveFiles = (workspaceRoot: string, paths: string[], destDir: string): Promise<MoveReport> =>
  invoke("move_files", { workspaceRoot, paths, destDir });

export const loadContigView = (path: string): Promise<ContigFileView> =>
  invoke("load_contig_view", { path });

export const loadMatrixReport = (path: string): Promise<MatrixReport> =>
  invoke("load_matrix_report", { path });

export const loadHmmLogo = (path: string, index?: number): Promise<HmmLogo> =>
  invoke("load_hmm_logo", { path, index: index ?? null });

export const listHmmProfiles = (path: string): Promise<HmmSummary[]> =>
  invoke("list_hmm_profiles", { path });

/// Uppercased sequences for the contigs at these file indices, in that order ("" for
/// one past the end). Server-cached, and batched by the viewer's visible window so
/// scrolling doesn't re-parse the file per contig.
export const getContigSequences = (path: string, indices: number[]): Promise<string[]> =>
  invoke("get_contig_sequences", { path, indices });

/// Read a text file (TSV/CSV) and write it back in place (the Text / Table editors).
/// A text file's contents, normalized to LF, plus the line terminator it used.
/// Pass `eol` back to `writeTextFile` / `createTextFile` to round-trip the file
/// instead of rewriting every line ending in it.
export interface TextFile {
  text: string;
  eol: string;
}
export const readTextFile = (path: string): Promise<TextFile> =>
  invoke("read_text_file", { path });
export const writeTextFile = (path: string, contents: string, eol?: string): Promise<void> =>
  invoke("write_text_file", { path, contents, eol });

/// Extract selected HMM profiles (0-based indices) into a new file
/// `dir/name`; returns the new path.
export const extractHmms = (
  dir: string,
  name: string,
  source: string,
  indices: number[],
): Promise<string> => invoke("extract_hmms", { dir, name, source, indices });

/// One item for `extractSelection`: contig `contig` whole, or its region
/// [start, end) read on `strand` (-1 = reverse complement).
export interface ExtractItem {
  contig: number;
  start?: number;
  end?: number;
  strand?: number;
}
/// Write each item, in order, as its own record of a new file `dir/name` (FASTA or
/// GenBank by extension); `translate` makes it a protein GenBank of their CDSs.
export const extractSelection = (
  dir: string,
  name: string,
  source: string,
  items: ExtractItem[],
  translate: boolean,
): Promise<string> => invoke("extract_selection", { dir, name, source, items, translate });

/// Create a new text file `dir/name` with `contents`; returns the new path.
export const createTextFile = (
  dir: string,
  name: string,
  contents: string,
  eol?: string,
): Promise<string> => invoke("create_text_file", { dir, name, contents, eol });

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

/// `columns` are column objects, `{type, ...parameters}` (`columns.ts`'s
/// `toColumns`); `ncbiTaxonomyPath` is needed only by the taxonomy columns.
/// Row `i`'s source: `[contig index, feat_index | null]` (indices as in
/// `ContigView`/`FeatureView`), `null` for a contig row or a protein's CDS row.
export type EnumRowTarget = [number, number | null];
export interface EnumReportResult {
  rows: EnumRow[];
  targets: EnumRowTarget[];
}

export const runEnumReport = (
  path: string,
  by: string,
  columns: string[],
  ncbiTaxonomyPath: string | null,
): Promise<EnumReportResult> => invoke("run_enum_report", { path, by, columns, ncbiTaxonomyPath });

export const runSummaryStats = (path: string, ncbiTaxonomyPath: string | null): Promise<SummaryStats> =>
  invoke("run_summary_stats", { path, ncbiTaxonomyPath });

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

/// File -> Quit: ask every window to close (each still guards its unsaved edits).
export const quitApp = (): Promise<void> => invoke("quit_app");

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
// One registry shared by every workspace (`cmd_db`). `workspaceRoot`, when given,
// folds that workspace's old per-workspace list in first.
export const listDatabases = (workspaceRoot: string | null): Promise<DatabaseRow[]> =>
  invoke("list_databases", { workspaceRoot });
export const registerDatabase = (path: string): Promise<DatabaseEntry> =>
  invoke("register_database", { path });
/// Take a database off the list; its files are untouched.
export const unregisterDatabase = (path: string): Promise<void> =>
  invoke("unregister_database", { path });
/// The data files, shards and indexes a database consists of.
export const listDatabaseFiles = (path: string): Promise<string[]> =>
  invoke("list_database_files", { path });
export const renameDatabase = (path: string, newName: string): Promise<DatabaseEntry> =>
  invoke("rename_database", { path, newName });
/// Delete a database's files from disk and take it off the list.
export const deleteDatabase = (path: string): Promise<void> => invoke("delete_database", { path });
export const buildDatabaseIndex = (path: string): Promise<number> =>
  invoke("build_database_index", { path });
/// Stop a `buildDatabaseIndex` run; it then rejects with `INDEXING_CANCELED`.
export const cancelDatabaseIndex = (path: string): Promise<void> =>
  invoke("cancel_database_index", { path });
/// Mirrors `cmd_db::INDEXING_CANCELED`.
export const INDEXING_CANCELED = "indexing canceled";
export const revealDatabase = (path: string): Promise<void> => invoke("reveal_database", { path });

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

/// The Enum Report tab's remembered granularity and columns for a file (`null`
/// when none, or the file is in no workspace), and saving them.
export const getEnumReportView = (path: string): Promise<EnumReportView | null> =>
  invoke("get_enum_report_view", { path });
export const setEnumReportView = (path: string, view: EnumReportView): Promise<void> =>
  invoke("set_enum_report_view", { path, view });

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

export const downloadReferenceDb = (id: string): Promise<InstalledDb> =>
  invoke("download_reference_db", { id });

export const cancelReferenceDb = (id: string): Promise<void> =>
  invoke("cancel_reference_db", { id });

export const removeReferenceDb = (id: string): Promise<void> =>
  invoke("remove_reference_db", { id });

/// Make sure every installed reference database is in the shared registry
/// (copies installed before it was shared were registered per workspace).
export const syncReferenceDbs = (): Promise<number> => invoke("sync_reference_dbs");

/// What the native menu needs in order to gray items out. Sent from the main
/// window only; `app_menu.rs` ignores it from any other label.
export interface MenuState {
  has_workspace: boolean;
  has_folder: boolean;
  has_tab: boolean;
  can_save: boolean;
  has_closed_tab: boolean;
}

export const setMenuState = (state: MenuState): Promise<void> =>
  invoke("set_menu_state", { state });
