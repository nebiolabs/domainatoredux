// `placeFiles` / `assignFiles`: where the selected files land in a tool dialog.
// The tools below copy the input fields of their `tools/<id>.yml` specs (role,
// file_type, prefill_priority, in spec order); nothing else of a spec matters here.
import { describe, expect, it } from "vitest";
import { assignFiles, placeFiles, type TypedPath } from "./toolInputs";
import type { ToolField, ToolMeta } from "./types";

type Role = "input_file" | "input_files";

function input(name: string, role: Role, file_types: string[], prefill_priority = 0): ToolField {
  return {
    name,
    label: name,
    help: null,
    role,
    file_types,
    required: false,
    default: null,
    cli_default: null,
    kind: "path",
    value_type: "string",
    multiple: role === "input_files",
    choices: [],
    min: null,
    max: null,
    exclusive_min: null,
    columns: [],
    sources: "workspace_first",
    select: "first",
    prefill_priority,
    output_name: null,
    column_types: {},
  };
}

function tool(id: string, fields: ToolField[]): ToolMeta {
  return { id, label: id, description: "", category: "", fields };
}

const domainSearch = tool("domain_search", [
  input("input", "input_file", ["sequence"]),
  input("references", "input_files", ["reference"], 1),
]);
const seqDist = tool("seq_dist", [
  input("input", "input_file", ["sequence", "hmm"]),
  input("reference", "input_file", ["sequence", "hmm"]),
]);
const buildSsn = tool("build_ssn", [
  input("input", "input_file", ["hdf5"]),
  input("metadata", "input_files", ["table"]),
  input("color_table", "input_file", ["table"]),
]);
const hmmerBuild = tool("hmmer_build", [input("input", "input_file", ["stockholm"])]);

const file = (path: string, file_type: string): TypedPath => ({ path, file_type });
const fa = file("a.fasta", "fasta");
const fa2 = file("b.fasta", "fasta");
const hmm = file("p.hmm", "hmm");
const gb = file("c.gb", "genbank");
const hdf5 = file("m.hdf5", "hdf5");
const tsv = file("meta.tsv", "table");
const tsv2 = file("colors.tsv", "table");
const sto = file("x.sto", "stockholm");
const sto2 = file("y.sto", "stockholm");

describe("placeFiles", () => {
  it.each([
    // domain_search's references outrank its input, so FASTAs are references;
    // `input` is left for its `select` (the first database).
    ["domain_search: lone FASTA", domainSearch, [fa], { references: ["a.fasta"] }, []],
    ["domain_search: HMM + FASTA", domainSearch, [hmm, fa], { references: ["p.hmm", "a.fasta"] }, []],
    // GenBank fits only `input`, whatever the priority says.
    ["domain_search: HMM + FASTA + GenBank", domainSearch, [hmm, fa, gb], { input: "c.gb", references: ["p.hmm", "a.fasta"] }, []],
    ["domain_search: lone GenBank", domainSearch, [gb], { input: "c.gb" }, []],
    // Two single fields of the same types fill in selection order.
    ["seq_dist: two FASTAs", seqDist, [fa, fa2], { input: "a.fasta", reference: "b.fasta" }, []],
    ["seq_dist: FASTA + HMM", seqDist, [fa, hmm], { input: "a.fasta", reference: "p.hmm" }, []],
    // A lone table takes the list field that comes before the single one.
    ["build_ssn: hdf5 + tsv", buildSsn, [hdf5, tsv], { input: "m.hdf5", metadata: ["meta.tsv"] }, []],
    ["build_ssn: two tables", buildSsn, [tsv, tsv2], { metadata: ["meta.tsv", "colors.tsv"] }, []],
    // Best effort: what fits is placed, the rest is left out in selection order.
    ["hmmer_build: two Stockholm files", hmmerBuild, [sto, sto2], { input: "x.sto" }, ["y.sto"]],
    ["seq_dist: three FASTAs", seqDist, [fa, fa2, file("z.fasta", "fasta")], { input: "a.fasta", reference: "b.fasta" }, ["z.fasta"]],
    ["domain_search: a stray table", domainSearch, [tsv, fa, gb], { input: "c.gb", references: ["a.fasta"] }, ["meta.tsv"]],
    ["hmmer_build: nothing fits", hmmerBuild, [fa, gb], {}, ["a.fasta", "c.gb"]],
  ])("%s", (_name, t, files, assigned, left) => {
    const got = placeFiles(t, files);
    expect(got.assigned).toEqual(assigned);
    expect(got.left.map((f) => f.path)).toEqual(left);
  });

  it("without priorities, a lone file prefers the earlier single field", () => {
    const noPriority = tool("domain_search", [
      input("input", "input_file", ["sequence"]),
      input("references", "input_files", ["reference"]),
    ]);
    expect(placeFiles(noPriority, [fa]).assigned).toEqual({ input: "a.fasta" });
  });

  it("ties in priority keep spec order", () => {
    const tied = tool("t", [input("first", "input_file", ["fasta"], 2), input("second", "input_file", ["fasta"], 2)]);
    expect(placeFiles(tied, [fa]).assigned).toEqual({ first: "a.fasta" });
  });
});

describe("assignFiles", () => {
  it("offers a tool only when every file fits", () => {
    expect(assignFiles(hmmerBuild, [sto])).toEqual({ input: "x.sto" });
    expect(assignFiles(hmmerBuild, [sto, sto2])).toBeNull();
    expect(assignFiles(domainSearch, [tsv, fa])).toBeNull();
  });

  it("offers nothing for an empty selection", () => {
    expect(assignFiles(domainSearch, [])).toBeNull();
  });
});
