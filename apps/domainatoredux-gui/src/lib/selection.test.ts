// Which Enum Report granularity a Contig View selection belongs to, and the
// row ↔ selection-item mapping the row checkboxes use.
import { describe, expect, it } from "vitest";
import { fittingBys, itemKey, targetItem, toggleItem, type SelItem } from "./selection";

// Contig 0 is a nucleotide record (feature 1 a CDS, feature 2 a domain, feature 3
// a gene); contig 1 is a protein.
const kinds: Record<string, string> = { "0:1": "CDS", "0:2": "Domainator", "0:3": "gene" };
const featKind = (ci: number, feat: number) => kinds[`${ci}:${feat}`];
const isProtein = (ci: number) => ci === 1;
const fits = (items: SelItem[]) => fittingBys(items, featKind, isProtein);

const cds: SelItem = { kind: "feature", ci: 0, feat: 1 };
const domain: SelItem = { kind: "feature", ci: 0, feat: 2 };
const gene: SelItem = { kind: "feature", ci: 0, feat: 3 };
const nucContig: SelItem = { kind: "contig", ci: 0 };
const protContig: SelItem = { kind: "contig", ci: 1 };
const range: SelItem = { kind: "range", ci: 0, a: 5, b: 2 };

describe("fittingBys", () => {
  it("names the one granularity a homogeneous selection fits", () => {
    expect(fits([cds])).toEqual(["cds"]);
    expect(fits([domain, domain])).toEqual(["domain"]);
    expect(fits([nucContig])).toEqual(["contig"]);
  });

  it("lets a protein record stand for its own CDS row", () => {
    expect(fits([protContig])).toEqual(["contig", "cds"]);
    expect(fits([protContig, cds])).toEqual(["cds"]);
    expect(fits([protContig, nucContig])).toEqual(["contig"]);
  });

  it("fits nothing for a mixed selection, a range or another feature kind", () => {
    expect(fits([cds, domain])).toEqual([]);
    expect(fits([cds, nucContig])).toEqual([]);
    expect(fits([range])).toEqual([]);
    expect(fits([gene])).toEqual([]);
  });

  it("fits everything when empty", () => {
    expect(fits([])).toEqual(["contig", "cds", "domain"]);
  });
});

describe("row items", () => {
  it("maps a report target to the item Contig View selects", () => {
    expect(targetItem([0, 1])).toEqual(cds);
    expect(targetItem([1, null])).toEqual(protContig);
  });

  it("keys a range by its span, whichever end the drag started from", () => {
    expect(itemKey(range)).toBe(itemKey({ kind: "range", ci: 0, a: 2, b: 5 }));
    expect(itemKey(cds)).not.toBe(itemKey(domain));
  });

  it("toggles an item in and out", () => {
    const once = toggleItem([cds], domain);
    expect(once).toEqual([cds, domain]);
    expect(toggleItem(once, { kind: "feature", ci: 0, feat: 1 })).toEqual([domain]);
  });
});
