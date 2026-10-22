// Enum Report column defaults: the built-in set per granularity, the global
// defaults from Preferences that replace it, and the column types the builder
// offers. Shared by the Enum Report tab and the Preferences dialog.
import { listTools, toolSchemaCached } from "./api";
import { cached } from "./cache";
import { columnTypes, type ColumnType } from "./columns";
import type { EnumBy } from "./selection";
import type { EnumReportView, JsonSchema, Prefs, ToolField } from "./types";

export type Column = Record<string, unknown>;

/// This session's remembered views by path: the fallback for a file in no
/// workspace, which has no sidecar to keep them in.
export const sessionViews = new Map<string, EnumReportView>();

/// The columns a granularity starts with when neither the file nor Preferences
/// says otherwise. Mirrors `default_columns` in `cmd_files.rs`.
const BUILTIN: Record<EnumBy, string[]> = {
  contig: ["length", "cds_count", "domains"],
  cds: ["domains", "architecture", "length"],
  domain: ["start", "end", "strand", "score", "identity"],
};

export function builtinColumns(by: EnumBy): Column[] {
  return BUILTIN[by].map((type) => ({ type }));
}

/// The global default for `by`: the Preferences setting, else the built-in set.
export function defaultColumns(by: EnumBy, prefs: Prefs | null): Column[] {
  return (prefs?.enum_report_columns?.[by] as Column[] | null | undefined) ?? builtinColumns(by);
}

/// The column types, from the enum_report schema and its spec's `column_types`
/// (`tools/enum_report.yml`), exactly as in the export dialog.
export async function loadEnumColumnTypes(): Promise<ColumnType[]> {
  const [schemaText, tools] = await Promise.all([toolSchemaCached("enum_report"), cached("tools", listTools)]);
  const schema = JSON.parse(schemaText) as JsonSchema;
  const field: ToolField | undefined = tools
    .find((t) => t.id === "enum_report")
    ?.fields.find((f) => f.role === "columns");
  return columnTypes(field ?? ({ column_types: {} } as unknown as ToolField), schema.properties?.columns);
}
