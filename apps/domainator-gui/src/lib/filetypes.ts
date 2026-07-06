import type { FileType } from "./types";

// Recognized data-file extensions — mirrors the backend `classify_file`
// (crates/domainator-core/src/app_support/workspace.rs). Used to validate
// new/renamed file names in the UI before hitting the backend, so the modal can
// show an inline error and keep the user's input.

export const ACCEPTED_EXTS = [
  "gb", "gbk", "genbank", "gbff",
  "fasta", "fa", "fna", "faa", "pep",
  "hmm", "hmmer",
  "hdf5", "hdf", "h5",
  "cm",
  "sto", "stockholm", "sth",
  "tsv", "csv", "tab",
  "txt", "text", "list",
  "svg",
  "ssnv", "ssnview",
];

export const ACCEPTED_HINT =
  "Accepted: .gb/.gbk/.genbank/.gbff, .fasta/.fa/.fna/.faa/.pep, .hmm, .hdf5/.h5, " +
  ".cm, .sto/.stockholm, .tsv/.csv/.tab, .txt, .svg, .ssnv (optionally with a .gz suffix).";

export function isRecognizedFilename(name: string): boolean {
  let n = name.trim().toLowerCase().replace(/\.(gz|bgz|bgzf)$/, "");
  const dot = n.lastIndexOf(".");
  if (dot <= 0) return false;
  return ACCEPTED_EXTS.includes(n.slice(dot + 1));
}

/// Returns an error message for an unrecognized filename, else null.
export function validateFilename(name: string): string | null {
  return isRecognizedFilename(name) ? null : `Unrecognized file type. ${ACCEPTED_HINT}`;
}

/// Map a filename to its viewer `FileType` (null if it has no viewer). Mirrors the
/// backend `classify_file` for the types the GUI renders.
export function fileTypeOf(name: string): FileType | null {
  const n = name.trim().toLowerCase().replace(/\.(gz|bgz|bgzf)$/, "");
  const dot = n.lastIndexOf(".");
  if (dot <= 0) return null;
  const ext = n.slice(dot + 1);
  if (["gb", "gbk", "genbank", "gbff"].includes(ext)) return "genbank";
  if (["fasta", "fa", "fna", "faa", "pep"].includes(ext)) return "fasta";
  if (["hmm", "hmmer"].includes(ext)) return "hmm";
  if (["hdf5", "hdf", "h5"].includes(ext)) return "hdf5";
  if (ext === "cm") return "cm";
  if (["ssnv", "ssnview"].includes(ext)) return "ssn_viewer";
  if (["tsv", "csv", "tab"].includes(ext)) return "table";
  if (["txt", "text", "list"].includes(ext)) return "text";
  if (ext === "svg") return "svg";
  return null;
}
