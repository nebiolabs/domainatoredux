// The file table's tool menus: the tools that can take every selected file at
// once, and how picking one pre-fills its dialog. Shared by the row context
// menu's Tools submenu and the toolbar's Tools button so the two cannot offer
// different lists.
import { ws } from "./state.svelte";
import { assignFiles } from "./toolInputs";
import type { FileEntry, ToolMeta } from "./types";

export interface ToolChoice {
  tool: ToolMeta;
  /// Opens the dialog with `files` placed in its inputs.
  open: () => void;
}

/// The tools that fit `files`, grouped by category in `tools`' order
/// (`list_tools` returns them sorted by category).
export function toolChoices(tools: ToolMeta[], files: FileEntry[]): Map<string, ToolChoice[]> {
  const groups = new Map<string, ToolChoice[]>();
  for (const tool of tools) {
    if (!assignFiles(tool, files)) continue;
    const choice = { tool, open: () => ws.openTool(tool.id, undefined, files) };
    groups.set(tool.category, [...(groups.get(tool.category) ?? []), choice]);
  }
  return groups;
}
