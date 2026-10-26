// Contig View's display order. A view-only sort: it permutes how contigs are
// drawn and never touches the file (the Sort Contigs tool writes a new one).
//
// Score keys mirror `sort_contigs.rs`'s `best_score`: the best Domain_Search hit,
// or the best Domainator hit with a given /name (from any database). Unlike the
// tool, a contig without one is not an error; it sorts last in either direction.
import type { ContigView, FeatureView } from "./types";

export type SortKey =
  | { by: "file" }
  | { by: "length" }
  | { by: "domain_search" }
  | { by: "domain"; name: string };

type SortFeature = Pick<FeatureView, "kind" | "score" | "qualifiers">;
type SortContig = Pick<ContigView, "length"> & { features: SortFeature[] };

const DOMAIN = "Domainator";
const DOMAIN_SEARCH = "Domain_Search";

function nameOf(f: SortFeature): string | undefined {
  return f.qualifiers.find(([k]) => k === "name")?.[1];
}

function bestScore(c: SortContig, keep: (f: SortFeature) => boolean): number | null {
  let best: number | null = null;
  for (const f of c.features) {
    if (f.score !== null && keep(f) && (best === null || f.score > best)) best = f.score;
  }
  return best;
}

/// The value a contig sorts by, or `null` when it has none (no matching hit).
export function contigSortValue(c: SortContig, k: SortKey): number | null {
  switch (k.by) {
    case "file":
      return null;
    case "length":
      return c.length;
    case "domain_search":
      return bestScore(c, (f) => f.kind === DOMAIN_SEARCH);
    case "domain":
      return bestScore(c, (f) => f.kind === DOMAIN && nameOf(f) === k.name);
  }
}

/// Display position → file index. File order (optionally reversed) for
/// `{by: "file"}`; otherwise by value, contigs without one last, ties in file order.
export function sortOrder(contigs: SortContig[], k: SortKey, desc: boolean): number[] {
  const idx = contigs.map((_, i) => i);
  if (k.by === "file") return desc ? idx.reverse() : idx;
  const vals = contigs.map((c) => contigSortValue(c, k));
  return idx.sort((a, b) => {
    const va = vals[a];
    const vb = vals[b];
    if (va === null || vb === null) {
      if (va === vb) return a - b;
      return va === null ? 1 : -1;
    }
    return (desc ? vb - va : va - vb) || a - b;
  });
}

/// How many contigs have no value for `k` (they are drawn last).
export function missingCount(contigs: SortContig[], k: SortKey): number {
  if (k.by === "file") return 0;
  return contigs.reduce((n, c) => n + (contigSortValue(c, k) === null ? 1 : 0), 0);
}

/// The distinct Domainator /name values in the file, sorted: the "Domain score" choices.
export function domainNames(contigs: SortContig[]): string[] {
  const names = new Set<string>();
  for (const c of contigs) {
    for (const f of c.features) {
      if (f.kind !== DOMAIN) continue;
      const n = nameOf(f);
      if (n) names.add(n);
    }
  }
  return [...names].sort((a, b) => a.localeCompare(b));
}

/// Whether any contig carries a scored Domain_Search hit.
export function hasDomainSearch(contigs: SortContig[]): boolean {
  return contigs.some((c) => c.features.some((f) => f.kind === DOMAIN_SEARCH && f.score !== null));
}
