// The Contig View selection of one sequence file, shared with its Enum Report tab.
//
// A selection is a list of items, each a whole contig, one feature (by its
// `feat_index`), or a bp range. Contig View draws and edits it; the Enum Report
// shows it as row checkboxes whenever every item is the kind of thing its rows
// are (all contigs, all CDSs, or all domains).

export type SelItem =
  | { kind: "contig"; ci: number }
  | { kind: "feature"; ci: number; feat: number }
  | { kind: "range"; ci: number; a: number; b: number };

/// The Enum Report granularities.
export type EnumBy = "contig" | "cds" | "domain";
export const ENUM_BYS: EnumBy[] = ["contig", "cds", "domain"];

/// Identity of an item, for set membership. A range's key includes its span.
export function itemKey(it: SelItem): string {
  if (it.kind === "contig") return `c:${it.ci}`;
  if (it.kind === "feature") return `f:${it.ci}:${it.feat}`;
  return `r:${it.ci}:${Math.min(it.a, it.b)}:${Math.max(it.a, it.b)}`;
}

/// The item an Enum Report row stands for: its feature, or the whole record for a
/// contig row (and a protein record's CDS row, which is the record itself).
export function targetItem([ci, feat]: [number, number | null]): SelItem {
  return feat === null ? { kind: "contig", ci } : { kind: "feature", ci, feat };
}

/// Whether `it` is something an Enum Report row at granularity `by` describes.
/// `featKind(ci, feat)` is the feature's kind (`CDS`, `Domainator`, ...), and
/// `isProtein(ci)` says whether a record is a protein, whose CDS row is the record.
export function itemFits(
  it: SelItem,
  by: EnumBy,
  featKind: (ci: number, feat: number) => string | undefined,
  isProtein: (ci: number) => boolean,
): boolean {
  if (it.kind === "range") return false;
  if (it.kind === "contig") return by === "contig" || (by === "cds" && isProtein(it.ci));
  const k = featKind(it.ci, it.feat);
  return by === "cds" ? k === "CDS" : by === "domain" ? k === "Domainator" : false;
}

/// The granularities every item fits, in `ENUM_BYS` order. An empty selection
/// fits all of them; a mixed one (or one holding a range) fits none.
export function fittingBys(
  items: SelItem[],
  featKind: (ci: number, feat: number) => string | undefined,
  isProtein: (ci: number) => boolean,
): EnumBy[] {
  return ENUM_BYS.filter((by) => items.every((it) => itemFits(it, by, featKind, isProtein)));
}

/// `items` with `it` added, or removed if already present.
export function toggleItem(items: SelItem[], it: SelItem): SelItem[] {
  const k = itemKey(it);
  return items.some((x) => itemKey(x) === k) ? items.filter((x) => itemKey(x) !== k) : [...items, it];
}
