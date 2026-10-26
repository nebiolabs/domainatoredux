// The column builder's tokens must be what the CLI's `--columns` parser reads
// (`domainator_config::columns::parse_column`): the last argument keeps its ':',
// a repeating argument splits on every ':'.
import { describe, expect, it } from "vitest";
import { columnTypes, toColumns, toRows } from "./columns";
import type { ToolField } from "./types";

const field = {
  columns: [
    { name: "length", args: [], repeat_last: false, help: "" },
    { name: "qualifier", args: ["feature", "qualifier"], repeat_last: false, help: "" },
    { name: "taxid", args: ["rank"], repeat_last: true, help: "" },
    { name: "append", args: ["name", "type", "value"], repeat_last: false, help: "" },
  ],
  column_types: { taxid: { label: null, help: null, show: true, params: { rank: { label: null, help: null, default: ["species"], choices: [] } } } },
} as unknown as ToolField;
const types = columnTypes(field);

describe("column tokens", () => {
  it("round-trip through builder rows", () => {
    const tokens = ["length", "qualifier:CDS:product", "taxid:genus:species", "append:note:str:a:b"];
    const rows = toRows(tokens, types);
    expect(rows[2].params.rank).toBe("genus, species");
    expect(rows[3].params.value).toBe("a:b");
    expect(toColumns(rows, types)).toEqual(tokens);
  });

  it("seed a new argument from the spec's default", () => {
    expect(toRows(["taxid"], types)[0].params.rank).toBe("species");
  });

  it("refuse an empty argument, naming the column", () => {
    const rows = toRows(["qualifier:CDS:"], types);
    expect(() => toColumns(rows, types)).toThrow(/Column 1 .*needs a value/);
  });

  it("refuse a ':' the token cannot hold", () => {
    const rows = [{ type: "qualifier", params: { feature: "a:b", qualifier: "x" } }];
    expect(() => toColumns(rows, types)).toThrow(/cannot contain ':'/);
  });
});
