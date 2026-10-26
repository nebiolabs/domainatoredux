// Enum Report column defaults: the built-in set per granularity, the global
// defaults from Preferences that replace it, and the columns the builder offers.
// Shared by the Enum Report tab and the Preferences dialog.
import { listTools } from "./api";
import { cached } from "./cache";
import { columnTypes, type ColumnType } from "./columns";
import type { EnumBy } from "./selection";
import type { EnumReportView, Prefs, ToolField } from "./types";

/// A column token, as `--columns` takes it (`qualifier:CDS:product`).
export type Column = string;

/// This session's remembered views by path: the fallback for a file in no
/// workspace, which has no sidecar to keep them in.
export const sessionViews = new Map<string, EnumReportView>();

/// The columns a granularity starts with when neither the file nor Preferences
/// says otherwise. Mirrors `default_columns` in `cmd_files.rs`.
const BUILTIN: Record<EnumBy, Column[]> = {
  contig: ["length", "cds_count", "domains"],
  cds: ["domains", "architecture", "length"],
  domain: ["start", "end", "strand", "score", "identity"],
};

export function builtinColumns(by: EnumBy): Column[] {
  return [...BUILTIN[by]];
}

/// The global default for `by`: the Preferences setting, else the built-in set.
export function defaultColumns(by: EnumBy, prefs: Prefs | null): Column[] {
  return prefs?.enum_report_columns?.[by] ?? builtinColumns(by);
}

/// The columns, from enum_report's manifest entry and its spec's `column_types`
/// (`tools/enum_report.yml`), exactly as in the export dialog.
export async function loadEnumColumnTypes(): Promise<ColumnType[]> {
  const tools = await cached("tools", listTools);
  const field: ToolField | undefined = tools
    .find((t) => t.id === "enum_report")
    ?.fields.find((f) => f.role === "columns");
  return field ? columnTypes(field) : [];
}
