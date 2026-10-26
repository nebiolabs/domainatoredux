// The Contig View gutter label: fields, their order, and the row number.
import { describe, expect, it } from "vitest";
import { DEFAULT_GUTTER_FIELDS, gutterLines } from "./contigLabel";

const gb = {
  id: "BX548174_369054:361090rc",
  name: "BX548174.1",
  definition: "Prochlorococcus marinus MED4 complete genome",
  accession: "BX548174",
  source: "Prochlorococcus marinus MED4",
};
const fasta = { id: "seq1", name: "seq1", definition: "", accession: null, source: null };
const all = { locus: true, accession: true, definition: true, source: true };

describe("gutterLines", () => {
  it("defaults to today's label: file number and LOCUS", () => {
    expect(gutterLines(gb, DEFAULT_GUTTER_FIELDS, "file", 4, 0)).toEqual(["5. BX548174_369054:361090rc"]);
  });

  it("stacks every ticked field in a fixed order, numbering the first line", () => {
    expect(gutterLines(gb, all, "rank", 4, 0)).toEqual([
      "1. BX548174_369054:361090rc",
      "BX548174.1",
      "Prochlorococcus marinus MED4 complete genome",
      "Prochlorococcus marinus MED4",
    ]);
    expect(gutterLines(gb, { ...all, locus: false }, "none", 4, 0)[0]).toBe("BX548174.1");
  });

  it("skips empty fields and never draws a blank row", () => {
    expect(gutterLines(fasta, all, "none", 0, 0)).toEqual(["seq1"]);
    expect(gutterLines(fasta, { ...all, locus: false }, "file", 0, 0)).toEqual(["1. seq1"]);
  });

  it("falls back to the primary accession when there is no VERSION", () => {
    const rec = { ...gb, name: gb.id };
    expect(gutterLines(rec, { ...all, locus: false, definition: false, source: false }, "none", 0, 0)).toEqual([
      "BX548174",
    ]);
  });
});
