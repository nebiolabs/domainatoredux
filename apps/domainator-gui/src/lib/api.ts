import { invoke } from "@tauri-apps/api/core";
import type {
  ContigFileView,
  DatabaseEntry,
  FileEntry,
  HmmLogo,
  MatrixReport,
  RunResult,
  ToolMeta,
  WorkspaceInfo,
} from "./types";

export const pickDirectory = (): Promise<string | null> =>
  invoke("pick_directory");

export const openWorkspace = (root: string): Promise<WorkspaceInfo> =>
  invoke("open_workspace", { root });

export const listFolder = (dir: string): Promise<FileEntry[]> =>
  invoke("list_folder", { dir });

export const loadContigView = (path: string): Promise<ContigFileView> =>
  invoke("load_contig_view", { path });

export const loadMatrixReport = (path: string): Promise<MatrixReport> =>
  invoke("load_matrix_report", { path });

export const loadHmmLogo = (path: string, index?: number): Promise<HmmLogo> =>
  invoke("load_hmm_logo", { path, index: index ?? null });

export type EnumRow = Record<string, string | number | null>;

export const runEnumReport = (path: string, by: string): Promise<EnumRow[]> =>
  invoke("run_enum_report", { path, by });

export const renderSsnHtml = (path: string): Promise<string> =>
  invoke("render_ssn_html", { path });

export interface ResolvedRecord {
  path: string;
  file_type: string;
  contig_index: number;
  contig_id: string;
}

export const resolveRecord = (
  workspaceRoot: string,
  label: string,
): Promise<ResolvedRecord | null> => invoke("resolve_record", { workspaceRoot, label });

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

export const runTool = (
  tool: string,
  params: Record<string, unknown>,
  workspaceRoot: string,
): Promise<RunResult> => invoke("run_tool", { tool, params, workspaceRoot });
