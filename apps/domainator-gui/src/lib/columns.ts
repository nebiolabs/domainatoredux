// The report column builder's model (role `columns`). The column types and their
// parameters come from the tool's schema: `columns.items.oneOf` has one entry per
// type, generated in `domainator-config` from the same enum that deserializes the
// run (`column_schemas_match_their_enums` keeps them in step). The spec's
// `column_types` (`tools/<id>.yml`) only relabels, hides and seeds them.
import type { JsonSchemaProp, ToolField } from "./types";

/// One parameter box of a column type.
export interface ColumnParam {
  name: string;
  label: string;
  help: string;
  /// `list`: comma-separated text sent as an array; `enum`: a dropdown.
  kind: "list" | "enum" | "number" | "text";
  /// `enum`: the allowed values.
  options: string[];
  /// Suggested values: chips for a list, a suggestion list for text.
  choices: string[];
  /// `list`: the JSON type of each entry.
  itemType: string;
  /// What a new column of this type starts with, as box text.
  initial: string;
  required: boolean;
}

/// One column type the builder offers.
export interface ColumnType {
  type: string;
  label: string;
  help: string;
  show: boolean;
  params: ColumnParam[];
}

/// A builder row: a type, and each parameter as the text in its box.
export interface ColumnRow {
  type: string;
  params: Record<string, string>;
}

const firstType = (p: JsonSchemaProp): string =>
  (Array.isArray(p.type) ? p.type.find((t) => t !== "null") : p.type) ?? "string";

const humanize = (name: string) => {
  const s = name.replace(/_/g, " ");
  return s.charAt(0).toUpperCase() + s.slice(1);
};

/// A value as box text: lists comma-separated, missing as blank.
export function textOf(v: unknown): string {
  if (v === null || v === undefined) return "";
  if (Array.isArray(v)) return v.map(String).join(", ");
  return String(v);
}

/// The column types of `field`, from its schema property `prop`, in schema order.
export function columnTypes(field: ToolField, prop: JsonSchemaProp | undefined): ColumnType[] {
  return (prop?.items?.oneOf ?? []).map((variant) => {
    const type = variant.properties?.type?.const ?? "";
    const over = field.column_types?.[type];
    const required = new Set(variant.required ?? []);
    const params = Object.entries(variant.properties ?? {})
      .filter(([name]) => name !== "type")
      .map(([name, p]): ColumnParam => {
        const po = over?.params?.[name];
        const t = firstType(p);
        const kind = p.enum ? "enum" : t === "array" ? "list" : t === "integer" || t === "number" ? "number" : "text";
        const options = (p.enum ?? []).filter((x) => x !== null).map(String);
        return {
          name,
          label: po?.label ?? humanize(name),
          help: po?.help ?? p.description ?? "",
          kind,
          options,
          choices: (po?.choices ?? []).map(String),
          itemType: p.items?.type ?? "string",
          initial: textOf(po?.default ?? (kind === "enum" ? options[0] : "")),
          required: required.has(name),
        };
      });
    return {
      type,
      label: over?.label ?? variant.title ?? type,
      help: over?.help ?? variant.description ?? "",
      show: over?.show ?? true,
      params,
    };
  });
}

export function newRow(t: ColumnType): ColumnRow {
  return { type: t.type, params: Object.fromEntries(t.params.map((p) => [p.name, p.initial])) };
}

/// Switch `row` to `type`, keeping the text of any parameter both types share.
export function retype(row: ColumnRow, type: string, types: ColumnType[]): void {
  const t = types.find((x) => x.type === type);
  if (!t) return;
  row.type = type;
  row.params = Object.fromEntries(t.params.map((p) => [p.name, row.params[p.name] ?? p.initial]));
}

/// Rows from a column list: the spec's `gui_default`, or a cloned job's saved
/// `columns`. A bare string is a parameterless column (enum_report's old
/// checklist saved its columns that way).
export function toRows(list: unknown, types: ColumnType[]): ColumnRow[] {
  if (!Array.isArray(list)) return [];
  return list.map((item) => {
    const obj: Record<string, unknown> = typeof item === "string" ? { type: item } : { ...(item as object) };
    const type = String(obj.type ?? "");
    const t = types.find((x) => x.type === type);
    const params: Record<string, string> = {};
    for (const p of t?.params ?? []) params[p.name] = p.name in obj ? textOf(obj[p.name]) : p.initial;
    return { type, params };
  });
}

/// The rows as the column objects the run takes: each type's parameters only,
/// since the column enums are `deny_unknown_fields`. Throws, with the column's
/// number, on a type the schema lacks or an empty required list.
export function toColumns(rows: ColumnRow[], types: ColumnType[]): Record<string, unknown>[] {
  return rows.map((row, i) => {
    const t = types.find((x) => x.type === row.type);
    if (!t) throw new Error(`Column ${i + 1}: unknown column type "${row.type}".`);
    const out: Record<string, unknown> = { type: row.type };
    for (const p of t.params) {
      const raw = (row.params[p.name] ?? "").trim();
      if (p.kind === "list") {
        const items = raw.split(",").map((s) => s.trim()).filter(Boolean);
        if (p.required && items.length === 0) {
          throw new Error(`Column ${i + 1} (${t.label}): "${p.label}" needs at least one value.`);
        }
        out[p.name] = p.itemType === "integer" || p.itemType === "number" ? items.map(Number) : items;
      } else if (p.kind === "number") {
        if (raw !== "") out[p.name] = Number(raw);
      } else {
        out[p.name] = raw;
      }
    }
    return out;
  });
}

/// A list parameter's text with `value` added, or removed if it is already there.
export function toggleListValue(text: string, value: string): string {
  const items = text.split(",").map((s) => s.trim()).filter(Boolean);
  const next = items.includes(value) ? items.filter((x) => x !== value) : [...items, value];
  return next.join(", ");
}

export const listHas = (text: string, value: string) =>
  text.split(",").map((s) => s.trim()).includes(value);
