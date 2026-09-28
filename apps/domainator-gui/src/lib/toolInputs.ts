// Which files a tool's input fields accept. Shared by `ToolDialog` (its input
// dropdowns) and the file table's tool menus (`toolMenu.ts`), so a tool a menu
// offers for the selection is one whose dialog will actually list those files.
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

/// What `assignFiles` needs of a file.
export interface TypedPath {
  path: string;
  file_type: string;
}

/// Place every one of `files` in one of `tool`'s input fields, or `null` when
/// they cannot all go in at once. An `input_file` field holds one file, an
/// `input_files` field any number. The result is in the dialog's pre-fill shape:
/// a path for `input_file`, a list for `input_files`.
///
/// Counting capacity, not just types, is what keeps the file table's Tools menu
/// honest: two Stockholm files do not offer hmmer-build (one input), because its
/// dialog could only take one of them.
///
/// Files of a type no list field takes are matched to single fields first (a
/// small bipartite matching, so seq-dist's two `[sequence, hmm]` inputs take two
/// FASTAs). Each remaining type goes to the first list field that takes it,
/// unless it is a lone file and a free single field comes earlier in the spec --
/// so for build-ssn an .hdf5 + .tsv fill `input` and `metadata`, not the
/// cluster-table field after them.
export function assignFiles(tool: ToolMeta, files: TypedPath[]): Record<string, string | string[]> | null {
  if (!files.length) return null;
  const inputs = tool.fields.filter((f) => isInputRole(f.role));
  const singles = inputs.filter((f) => f.role === "input_file");
  const multiIndex = (t: string) => inputs.findIndex((f) => f.role === "input_files" && fieldAccepts(f, t));

  // Single fields -> the file placed there.
  const taken = new Map<string, TypedPath>();
  const hard = files.filter((f) => multiIndex(f.file_type) < 0);
  // Kuhn's augmenting paths; `seen` is per top-level attempt. A free field is
  // tried before displacing anyone, so files land in selection order when they
  // can (seq-dist: first file -> `input`, second -> `reference`).
  const place = (file: TypedPath, seen: Set<string>): boolean => {
    const free = singles.find((f) => !taken.has(f.name) && fieldAccepts(f, file.file_type));
    if (free) {
      taken.set(free.name, file);
      return true;
    }
    for (const field of singles) {
      if (seen.has(field.name) || !fieldAccepts(field, file.file_type)) continue;
      seen.add(field.name);
      const holder = taken.get(field.name);
      if (!holder || place(holder, seen)) {
        taken.set(field.name, file);
        return true;
      }
    }
    return false;
  };
  for (const file of hard) if (!place(file, new Set())) return null;

  const out: Record<string, string | string[]> = {};
  for (const [name, file] of taken) out[name] = file.path;

  const byType = new Map<string, TypedPath[]>();
  for (const f of files) {
    if (hard.includes(f)) continue;
    byType.set(f.file_type, [...(byType.get(f.file_type) ?? []), f]);
  }
  for (const [type, group] of byType) {
    const mi = multiIndex(type);
    if (group.length === 1) {
      const si = inputs.findIndex((f) => f.role === "input_file" && !(f.name in out) && fieldAccepts(f, type));
      if (si >= 0 && si < mi) {
        out[inputs[si].name] = group[0].path;
        continue;
      }
    }
    const name = inputs[mi].name;
    out[name] = [...((out[name] as string[] | undefined) ?? []), ...group.map((f) => f.path)];
  }
  return out;
}
