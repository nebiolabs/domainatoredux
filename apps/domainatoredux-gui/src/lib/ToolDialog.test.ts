// @vitest-environment jsdom
// The dialog renders for every kind of field. Its first render comes before the
// effect that seeds the form, so a field whose widget cannot take an unset value
// throws there, and the dead dialog's backdrop then swallows every click: the app
// looks frozen, with nothing in the log a user would see.
import { flushSync, mount, unmount } from "svelte";
import { expect, it, vi } from "vitest";
import type { ToolField, ToolMeta } from "./types";

vi.mock("@tauri-apps/api/core", () => ({ invoke: async () => null }));

const { default: ToolDialog } = await import("./ToolDialog.svelte");
const { ws } = await import("./state.svelte");

function field(name: string, over: Partial<ToolField>): ToolField {
  return {
    name,
    label: name,
    help: null,
    role: "param",
    file_types: [],
    required: false,
    default: null,
    cli_default: null,
    kind: "value",
    value_type: "string",
    multiple: false,
    choices: [],
    min: null,
    max: null,
    exclusive_min: null,
    columns: [],
    sources: null,
    select: null,
    prefill_priority: 0,
    output_name: null,
    column_types: {},
    ...over,
  };
}

const report: ToolMeta = {
  id: "enum_report",
  label: "Enum report",
  description: "",
  category: "",
  fields: [
    field("input", { role: "input_files", kind: "path", multiple: true, file_types: ["sequence"], select: "first" }),
    field("reference", { role: "input_file", kind: "path", file_types: ["hmm"] }),
    field("tsv_out", { role: "output_file", kind: "path", output_name: "{stem}.enum.tsv" }),
    field("by", { choices: ["contig", "cds"], default: "contig", cli_default: "contig" }),
    field("columns", {
      role: "columns",
      kind: "columns",
      multiple: true,
      default: ["domains", "qualifier:CDS:product"],
      columns: [
        { name: "domains", args: [], repeat_last: false, help: "" },
        { name: "qualifier", args: ["feature", "qualifier"], repeat_last: false, help: "" },
      ],
    }),
    field("evalue", { value_type: "number", default: 10 }),
    field("verbose_names", { kind: "flag", value_type: "boolean", default: false }),
    field("names", { multiple: true, default: ["a", "b"] }),
  ],
};

it("renders a dialog with every kind of field, the column builder included", () => {
  ws.root = "/w";
  ws.selectedFolder = "/w";
  ws.selectedFolders = ["/w"];
  ws.files = [{ path: "/w/contigs.gb", name: "contigs.gb", file_type: "genbank" } as never];
  const target = document.createElement("div");
  document.body.append(target);
  const dialog = mount(ToolDialog, { target, props: { tool: report.id, meta: report, onclose: () => {} } });
  flushSync();
  expect(target.querySelectorAll(".builder .row")).toHaveLength(2);
  expect(target.querySelector<HTMLInputElement>('input[type="text"]')?.value).toBe("contigs.enum.tsv");
  unmount(dialog);
});
