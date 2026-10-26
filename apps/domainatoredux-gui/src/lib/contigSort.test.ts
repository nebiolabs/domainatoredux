// Contig View's view-only sort: the display permutation and its keys.
import { describe, expect, it } from "vitest";
import { contigSortValue, domainNames, hasDomainSearch, missingCount, sortOrder } from "./contigSort";

const hit = (kind: string, score: number | null, name?: string) => ({
  kind,
  score,
  qualifiers: name ? ([["name", name]] as [string, string][]) : [],
});

// 0: no hits; 1: Domain_Search 50 + PF1 10/30; 2: Domain_Search 80 + PF2 99;
// 3: Domain_Search 50 + PF1 30 (ties contig 1 on both).
const contigs = [
  { length: 300, features: [hit("CDS", null)] },
  { length: 100, features: [hit("Domain_Search", 50), hit("Domainator", 10, "PF1"), hit("Domainator", 30, "PF1")] },
  { length: 200, features: [hit("Domain_Search", 80), hit("Domainator", 99, "PF2")] },
  { length: 100, features: [hit("Domain_Search", 50), hit("Domainator", 30, "PF1")] },
];

describe("sortOrder", () => {
  it("is the identity in file order, and its reverse descending", () => {
    expect(sortOrder(contigs, { by: "file" }, false)).toEqual([0, 1, 2, 3]);
    expect(sortOrder(contigs, { by: "file" }, true)).toEqual([3, 2, 1, 0]);
  });

  it("sorts by length with ties in file order", () => {
    expect(sortOrder(contigs, { by: "length" }, true)).toEqual([0, 2, 1, 3]);
    expect(sortOrder(contigs, { by: "length" }, false)).toEqual([1, 3, 2, 0]);
  });

  it("puts contigs without a value last in both directions", () => {
    expect(sortOrder(contigs, { by: "domain_search" }, true)).toEqual([2, 1, 3, 0]);
    expect(sortOrder(contigs, { by: "domain_search" }, false)).toEqual([1, 3, 2, 0]);
    expect(sortOrder(contigs, { by: "domain", name: "PF2" }, false)).toEqual([2, 0, 1, 3]);
  });

  it("scores a named domain by its best hit only", () => {
    expect(contigSortValue(contigs[1], { by: "domain", name: "PF1" })).toBe(30);
    expect(contigSortValue(contigs[2], { by: "domain", name: "PF1" })).toBeNull();
    expect(sortOrder(contigs, { by: "domain", name: "PF1" }, true)).toEqual([1, 3, 0, 2]);
  });
});

describe("helpers", () => {
  it("counts the contigs that sort last", () => {
    expect(missingCount(contigs, { by: "domain_search" })).toBe(1);
    expect(missingCount(contigs, { by: "domain", name: "PF1" })).toBe(2);
    expect(missingCount(contigs, { by: "file" })).toBe(0);
  });

  it("lists distinct domain names and notices Domain_Search hits", () => {
    expect(domainNames(contigs)).toEqual(["PF1", "PF2"]);
    expect(hasDomainSearch(contigs)).toBe(true);
    expect(hasDomainSearch([contigs[0]])).toBe(false);
  });
});
