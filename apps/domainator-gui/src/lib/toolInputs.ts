// Which files a tool's input fields accept. Shared by `ToolDialog` (its input
// dropdowns) and the toolbar's context-sensitive Tools button, so a tool the
// button offers for a file is one whose dialog will actually list that file.
import type { ToolField, ToolMeta } from "./types";

/// Whether a file of `fileType` fits a field that accepts `ft`, one entry of the
/// field's `file_type` in its `tools/<id>.yml` spec.
export function typeMatches(fileType: string, ft: string): boolean {
  if (ft === "sequence") return fileType === "genbank" || fileType === "fasta";
  // domainate/domain_search accept HMM or FASTA references (protein or nucleotide).
  if (ft === "reference") return fileType === "hmm" || fileType === "fasta";
  return fileType === ft;
}

export const isInputRole = (role: string) => role === "input_file" || role === "input_files";

/// Whether an input field accepts a file of `fileType` (any of its `file_types`).
export const fieldAccepts = (field: ToolField, fileType: string) =>
  field.file_types.some((ft) => typeMatches(fileType, ft));

/// Whether any of `tool`'s input fields accepts a file of `fileType`.
export function toolAcceptsFile(tool: ToolMeta, fileType: string): boolean {
  return tool.fields.some((f) => isInputRole(f.role) && fieldAccepts(f, fileType));
}
