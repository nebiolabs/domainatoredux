// The file table's tool menus: the tools that can take every selected file at
// once, and how picking one pre-fills its dialog. Shared by the row context
// menu's Tools submenu and the toolbar's Tools button so the two cannot offer
// different lists.
import { ws } from "./state.svelte";
import { assignFiles, type TypedPath } from "./toolInputs";
import type { ToolMeta } from "./types";

export interface ToolChoice {
  tool: ToolMeta;
  /// Opens the dialog with the files in the fields `assignFiles` chose.
  open: () => void;
}

/// The tools that fit `files`, grouped by category in `tools`' order
/// (`list_tools` returns them sorted by category).
export function toolChoices(tools: ToolMeta[], files: TypedPath[]): Map<string, ToolChoice[]> {
  const groups = new Map<string, ToolChoice[]>();
  for (const tool of tools) {
    const assignment = assignFiles(tool, files);
    if (!assignment) continue;
    const choice = { tool, open: () => ws.openTool(tool.id, assignment, { seedActive: false }) };
    groups.set(tool.category, [...(groups.get(tool.category) ?? []), choice]);
  }
  return groups;
}
