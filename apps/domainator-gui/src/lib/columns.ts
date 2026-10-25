// The report column builder's model (role `columns`). The columns and their
// arguments come from the tool's manifest (`domainatoredux describe`): the
// `columns` parameter lists one `{name, args, repeat_last, help}` entry per column,
// generated from the same catalog that parses the run's `--columns`. The spec's
// `column_types` (`tools/<id>.yml`) only relabels, hides and seeds them.
//
// A column is sent as a token, its name and arguments joined by ':'
// (`qualifier:CDS:product`). The last argument takes the rest of the token, so it
// may contain ':'; a column whose last argument repeats (`taxid:genus:species`)
// splits on every ':' instead.
import type { ToolField } from "./types";

/// One argument box of a column.
export interface ColumnParam {
  name: string;
  label: string;
  help: string;
  /// `list`: the repeating last argument, comma-separated text; `text`: one value.
  kind: "list" | "text";
  /// Suggested values: chips for a list, a suggestion list for text.
  choices: string[];
  /// What a new column starts with, as box text.
  initial: string;
  /// The last argument, which may contain ':'.
  last: boolean;
}

/// One column the builder offers.
export interface ColumnType {
  type: string;
  label: string;
  help: string;
  show: boolean;
  params: ColumnParam[];
}

/// A builder row: a column, and each argument as the text in its box.
export interface ColumnRow {
  type: string;
  params: Record<string, string>;
}

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

const listItems = (text: string) =>
  text
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);

/// The columns of `field`, from its manifest catalog, in catalog order.
export function columnTypes(field: ToolField): ColumnType[] {
  return (field.columns ?? []).map((c) => {
    const over = field.column_types?.[c.name];
    const params = c.args.map((name, i): ColumnParam => {
      const po = over?.params?.[name];
      const last = i === c.args.length - 1;
      return {
        name,
        label: po?.label ?? humanize(name),
        help: po?.help ?? "",
        kind: last && c.repeat_last ? "list" : "text",
        choices: (po?.choices ?? []).map(String),
        initial: textOf(po?.default),
        last,
      };
    });
    return {
      type: c.name,
      label: over?.label ?? humanize(c.name),
      help: over?.help ?? c.help ?? "",
      show: over?.show ?? true,
      params,
    };
  });
}

export function newRow(t: ColumnType): ColumnRow {
  return { type: t.type, params: Object.fromEntries(t.params.map((p) => [p.name, p.initial])) };
}

/// Switch `row` to `type`, keeping the text of any argument both columns share.
export function retype(row: ColumnRow, type: string, types: ColumnType[]): void {
  const t = types.find((x) => x.type === type);
  if (!t) return;
  row.type = type;
  row.params = Object.fromEntries(t.params.map((p) => [p.name, row.params[p.name] ?? p.initial]));
}

/// Rows from column tokens: the spec's `gui_default`, a cloned job's saved
/// `columns`, or a remembered Enum Report view.
export function toRows(list: unknown, types: ColumnType[]): ColumnRow[] {
  if (!Array.isArray(list)) return [];
  return list.map((item) => {
    const token = String(item);
    const cut = token.indexOf(":");
    const type = cut < 0 ? token : token.slice(0, cut);
    const rest = cut < 0 ? "" : token.slice(cut + 1);
    const t = types.find((x) => x.type === type);
    const params: Record<string, string> = {};
    const ps = t?.params ?? [];
    if (ps.length && rest) {
      if (ps[ps.length - 1].kind === "list") {
        // Only the repeating argument: every ':' separates two of its values.
        params[ps[0].name] = rest.split(":").join(", ");
      } else {
        const parts = rest.split(":");
        ps.forEach((p, i) => {
          params[p.name] = p.last ? parts.slice(i).join(":") : (parts[i] ?? "");
        });
      }
    }
    for (const p of ps) params[p.name] ??= p.initial;
    return { type, params };
  });
}

/// The rows as the tokens the run takes. Throws, with the column's number, on a
/// column the catalog lacks, an empty argument, or a ':' where the token cannot
/// hold one.
export function toColumns(rows: ColumnRow[], types: ColumnType[]): string[] {
  return rows.map((row, i) => {
    const t = types.find((x) => x.type === row.type);
    if (!t) throw new Error(`Column ${i + 1}: unknown column "${row.type}".`);
    const parts = [row.type];
    for (const p of t.params) {
      const raw = (row.params[p.name] ?? "").trim();
      const values = p.kind === "list" ? listItems(raw) : raw ? [raw] : [];
      if (values.length === 0) {
        throw new Error(`Column ${i + 1} (${t.label}): "${p.label}" needs a value.`);
      }
      if ((!p.last || p.kind === "list") && values.some((v) => v.includes(":"))) {
        throw new Error(`Column ${i + 1} (${t.label}): "${p.label}" cannot contain ':'.`);
      }
      parts.push(...values);
    }
    return parts.join(":");
  });
}

/// The column name of a token.
export const columnName = (token: string) => token.split(":", 1)[0];

/// A list parameter's text with `value` added, or removed if it is already there.
export function toggleListValue(text: string, value: string): string {
  const items = listItems(text);
  const next = items.includes(value) ? items.filter((x) => x !== value) : [...items, value];
  return next.join(", ");
}

export const listHas = (text: string, value: string) =>
  text.split(",").map((s) => s.trim()).includes(value);
