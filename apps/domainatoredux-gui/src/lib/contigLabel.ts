// The Contig View gutter label: which record fields it shows (one per line) and
// how the row is numbered.
import type { ContigView } from "./types";

export type GutterField = "locus" | "accession" | "definition" | "source";
export type GutterNumber = "file" | "rank" | "none";

/// The checkboxes in View options, in the order the lines are drawn.
export const GUTTER_FIELDS: { field: GutterField; label: string; title: string }[] = [
  { field: "locus", label: "LOCUS name", title: "The record id (LOCUS, or the FASTA id)" },
  { field: "accession", label: "Accession", title: "VERSION, else the primary ACCESSION" },
  { field: "definition", label: "Definition", title: "DEFINITION (the FASTA description)" },
  { field: "source", label: "Source", title: "SOURCE, else ORGANISM" },
];

export const DEFAULT_GUTTER_FIELDS: Record<GutterField, boolean> = {
  locus: true,
  accession: false,
  definition: false,
  source: false,
};

type LabelContig = Pick<ContigView, "id" | "name" | "definition" | "accession" | "source">;

function fieldValue(c: LabelContig, f: GutterField): string {
  switch (f) {
    case "locus":
      return c.id;
    case "accession":
      // `name` is the VERSION (or ACCESSION); for FASTA it is just the id again.
      return c.name && c.name !== c.id ? c.name : (c.accession ?? "");
    case "definition":
      return c.definition;
    case "source":
      return c.source ?? "";
  }
}

/// The gutter lines for one contig: each ticked, non-empty field in
/// `GUTTER_FIELDS` order, the first prefixed with its number. Falls back to the id
/// so a row is never blank. `ci` is the file index, `pos` the display position.
export function gutterLines(
  c: LabelContig,
  fields: Record<GutterField, boolean>,
  num: GutterNumber,
  ci: number,
  pos: number,
): string[] {
  const lines = GUTTER_FIELDS.filter(({ field }) => fields[field])
    .map(({ field }) => fieldValue(c, field).trim())
    .filter((v) => v !== "");
  if (lines.length === 0) lines.push(c.id || c.name);
  if (num !== "none") lines[0] = `${(num === "file" ? ci : pos) + 1}. ${lines[0]}`;
  return lines;
}
