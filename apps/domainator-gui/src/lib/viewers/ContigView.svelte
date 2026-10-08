<script lang="ts">
  import { addFeature, extractSelection, getContigSequences, loadContigView, removeFeature, saveBinaryFile, saveTextFile, setFeatureColor, type ExtractItem } from "../api";
  import { cached, invalidatePath } from "../cache";
  import { ws } from "../state.svelte";
  import { validateFilename } from "../filetypes";
  import NameModal from "../NameModal.svelte";
  import type { ContigFileView, ContigView as Contig, FeatureView } from "../types";
  import { itemKey, toggleItem, type SelItem } from "../selection";

  let { path, active = true }: { path: string; active?: boolean } = $props();

  // ── data / load ──────────────────────────────────────────────────────────
  let data = $state<ContigFileView | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(true);
  let focused = $state<number | null>(null);
  let fitted = false; // one-time initial Fit once the viewport width is known

  // ── editing (GenBank only) ─────────────────────────────────────────────────
  const canEdit = $derived(/\.(gb|gbk|genbank|gbff)$/i.test(path));
  // The right-clicked feature: drives the pinned detail panel (all file types)
  // and, for GenBank, the color/remove edit controls at the bottom of it. Apart
  // from the selection, which can hold many features at once.
  let inspected = $state<{ contigIndex: number; feat: FeatureView } | null>(null);
  let addOpen = $state(false);
  let addForm = $state({ contigIndex: 0, kind: "misc_feature", start: 0, end: 100, strand: 1, note: "" });
  let editMsg = $state<string | null>(null);

  // ── view controls ──────────────────────────────────────────────────────────
  let pxPerBp = $state(0.2);
  let enabledKinds = $state<Record<string, boolean>>({});
  let enabledDbs = $state<Record<string, boolean>>({});
  let maxEvalueExp = $state(0);
  let hasEvalue = $state(false);

  let tooltip = $state<{ x: number; y: number; feature: FeatureView } | null>(null);
  let nameTooltip = $state<{ x: number; y: number; name: string } | null>(null);

  // ── geometry constants ─────────────────────────────────────────────────────
  let GUTTER = $state(134);
  const SEQ_H = 16; // nucleotide/residue letter track
  const SEQBAR_H = 7; // collapsed sequence bar (zoomed out)
  const TRANS_H = 16; // translation track (nucleotide contigs)
  const LETTERS_MIN = 4.5; // px/bp at/above which residue letters render (~1 zoom step earlier)
  const CONTIG_PAD_TOP = 4;

  // ── user-adjustable appearance ─────────────────────────────────────────────
  // Everything here feeds both the live canvas and the SVG/PNG export, which
  // share one `renderScene`; changing any of them re-derives `vlayout` (track
  // heights) and repaints.
  let optsOpen = $state(false);
  let laneH = $state(20); // vertical pitch of one feature lane
  let contigGap = $state(16); // blank space between stacked contigs
  let rulerFontSize = $state(10); // bp ruler tick labels
  let labelFontSize = $state(10); // feature labels
  let rulerUnits = $state<"auto" | "bp" | "kb">("auto");
  let sharedRuler = $state(false); // one pinned ruler instead of one per contig
  let labelPlacement = $state<"inside" | "outside" | "both">("inside");
  let annotH = $state(14); // requested feature-bar height
  // Overlapping lanes are offset by part of a bar instead of a full lane pitch,
  // so later lanes partly cover earlier ones (CDS on top, its domains below).
  let stackAnnotations = $state(false);

  // Drawn bar height: never shorter than the label text it has to hold.
  const featH = $derived(Math.max(labelFontSize, annotH));
  // Spaced-lane pitch, grown when the bar would not otherwise fit.
  const lanePitch = $derived(Math.max(laneH, featH + 2));
  // Vertical offset between consecutive lanes in stacked mode.
  const stackShift = $derived(Math.max(3, Math.round(featH / 2)));

  // Height of a ruler band (the tick label plus its tick mark and padding).
  const rulerBandH = $derived(rulerFontSize + 7);
  // Per-contig ruler height — zero when a single shared ruler is pinned on top.
  const perContigRulerH = $derived(sharedRuler ? 0 : rulerBandH);
  // Outside labels need their own strip above each feature box.
  // Stacked lanes have no room for per-lane strips, so they use inside labels only.
  const outsideLabelH = $derived(labelPlacement === "inside" || stackAnnotations ? 0 : labelFontSize + 3);
  // Full vertical pitch of a lane slot (label strip + feature row).
  const laneSlotH = $derived(lanePitch + outsideLabelH);

  function resetViewOptions() {
    laneH = 20;
    contigGap = 16;
    rulerFontSize = 10;
    labelFontSize = 10;
    rulerUnits = "auto";
    sharedRuler = false;
    labelPlacement = "inside";
    annotH = 14;
    stackAnnotations = false;
  }

  // ── lane-stacking (port of plot_contigs.py add_z_order) ─────────────────────
  const PRECEDENCE = [
    "source", "CDS", "exon", "repeat_region", "ncRNA", "rRNA", "tRNA",
    "Domainator", "Domain_Search",
  ];
  const SQUASH = new Set(["CDS", "exon"]);
  const kindOrder = (k: string) => {
    const i = PRECEDENCE.indexOf(k);
    return i < 0 ? PRECEDENCE.length + 1 : i;
  };

  type Placed = { f: FeatureView; lane: number };
  // `byLane` is `placed` stably sorted by lane: the paint order in stacked mode,
  // where a higher lane must always cover the lane it overlaps.
  type Layout = { placed: Placed[]; byLane: Placed[]; maxLane: number };

  // Greedy interval lane-assignment (port of plot_contigs.py add_z_order), but
  // O(n·depth) via a per-lane running-max-end array instead of the old O(n²)
  // all-pairs scan — so we can lay out every contig in a 20k-contig file without
  // freezing (needed for genome-scale annotation stacks + dynamic row heights).
  function layout(features: FeatureView[]): Layout {
    const sorted = [...features].sort((a, b) => kindOrder(a.kind) - kindOrder(b.kind) || a.start - b.start);
    const placed: Placed[] = [];
    const laneEnd: number[] = []; // max feature-end currently in each lane
    let squashLane = -1; // CDS/exon all collapse onto one shared lane
    for (const f of sorted) {
      let lane: number;
      if (SQUASH.has(f.kind)) {
        if (squashLane === -1) {
          squashLane = laneEnd.length;
          laneEnd.push(f.end);
        } else {
          laneEnd[squashLane] = Math.max(laneEnd[squashLane], f.end);
        }
        lane = squashLane;
      } else {
        lane = -1;
        for (let i = 0; i < laneEnd.length; i++) {
          if (i === squashLane) continue;
          if (laneEnd[i] <= f.start) { lane = i; break; } // free: nothing overlaps
        }
        if (lane === -1) {
          lane = laneEnd.length;
          laneEnd.push(f.end);
        } else {
          laneEnd[lane] = f.end;
        }
      }
      placed.push({ f, lane });
    }
    const maxLane = Math.max(0, laneEnd.length - 1);
    const byLane = [...placed].sort((a, b) => a.lane - b.lane);
    return { placed, byLane, maxLane };
  }

  function visible(f: FeatureView): boolean {
    if (enabledKinds[f.kind] === false) return false;
    if (f.database !== null && enabledDbs[f.database] === false) return false;
    if (hasEvalue && f.evalue !== null && f.evalue > Math.pow(10, maxEvalueExp)) return false;
    return true;
  }

  // Memoize per-contig layout; recompute only when the filter signature changes
  // (layout is zoom-independent — lanes are assigned by bp overlap).
  const filterSig = $derived(JSON.stringify([enabledKinds, enabledDbs, maxEvalueExp, hasEvalue]));
  let layoutCache = new Map<number, { sig: string; layout: Layout }>();
  function contigLayout(c: Contig): Layout {
    const hit = layoutCache.get(c.index);
    if (hit && hit.sig === filterSig) return hit.layout;
    const l = layout(c.features.filter(visible));
    layoutCache.set(c.index, { sig: filterSig, layout: l });
    return l;
  }

  // ── vertical layout: per-contig track offsets + cumulative y offsets ─────────
  const showLetters = $derived(pxPerBp >= LETTERS_MIN);
  function seqTrackH(): number {
    return showLetters ? SEQ_H : SEQBAR_H;
  }
  function transTrackH(c: Contig): number {
    return showLetters && !c.is_protein ? TRANS_H : 0;
  }
  // Height of a contig's whole feature-lane track.
  function lanesTrackH(lay: Layout): number {
    return stackAnnotations ? featH + lay.maxLane * stackShift + 2 : (lay.maxLane + 1) * laneSlotH;
  }
  // Top of a lane's feature bar, relative to the frame's `laneBaseY`. Spaced lanes
  // draw lane 0 at the bottom; stacked lanes draw it on top, later lanes below.
  function laneTopOffset(lay: Layout, lane: number): number {
    if (stackAnnotations) return 1 + lane * stackShift;
    return (lay.maxLane - lane) * laneSlotH + outsideLabelH + (lanePitch - featH) / 2;
  }

  function contigHeight(c: Contig, lay: Layout): number {
    return (
      CONTIG_PAD_TOP +
      perContigRulerH +
      seqTrackH() +
      transTrackH(c) +
      lanesTrackH(lay) +
      contigGap
    );
  }

  // Cumulative content-y offset per contig + total height. Every contig is sized
  // to its real lane count (no clipping, even at tens of thousands of contigs —
  // the layout is O(n·depth) and cached). Depends on data, filters, and zoom
  // (letters change track heights) — NOT on scroll.
  const vlayout = $derived.by(() => {
    if (!data) return { offsets: [] as number[], total: 0 };
    void filterSig;
    void pxPerBp;
    void featH;
    void laneSlotH;
    void stackShift;
    void stackAnnotations;
    const contigs = data.contigs;
    const offsets: number[] = new Array(contigs.length + 1);
    // A pinned shared ruler occupies the top of the viewport, so the content
    // starts below it (it then scrolls underneath, sticky-header style).
    offsets[0] = sharedRuler ? rulerBandH : 0;
    for (let i = 0; i < contigs.length; i++) {
      offsets[i + 1] = offsets[i] + contigHeight(contigs[i], contigLayout(contigs[i]));
    }
    return { offsets, total: offsets[contigs.length] };
  });

  const maxContigLen = $derived(data ? data.contigs.reduce((m, c) => Math.max(m, c.length), 1) : 1);

  // ── sequence lazy-load (server-cached, batched by range with overscan) ────────
  let seqCache = $state<Record<number, string>>({});
  const seqPending = new Set<number>();
  const SEQ_OVERSCAN = 8; // prefetch this many contigs above/below the viewport

  // Fetch the missing contiguous sub-range within [a, b] (± overscan) in ONE call.
  function ensureSeqRange(a: number, b: number) {
    if (!data) return;
    const lo = Math.max(0, a - SEQ_OVERSCAN);
    const hi = Math.min(data.contigs.length - 1, b + SEQ_OVERSCAN);
    let need0 = -1;
    let need1 = -1;
    for (let i = lo; i <= hi; i++) {
      if (seqCache[i] === undefined && !seqPending.has(i)) {
        if (need0 < 0) need0 = i;
        need1 = i;
      }
    }
    if (need0 < 0) return;
    for (let i = need0; i <= need1; i++) seqPending.add(i);
    const s0 = need0;
    getContigSequences(path, need0, need1 + 1)
      .then((seqs) => {
        const next = { ...seqCache };
        seqs.forEach((s, k) => (next[s0 + k] = s));
        seqCache = next;
      })
      .catch(() => {})
      .finally(() => {
        for (let i = s0; i <= need1; i++) seqPending.delete(i);
      });
  }
  // A single contig (e.g. for copy of a clicked feature not yet loaded).
  function ensureSeq(ci: number) {
    ensureSeqRange(ci, ci);
  }

  // ── viewport state ──────────────────────────────────────────────────────────
  let scrollEl = $state<HTMLDivElement>();
  let canvas = $state<HTMLCanvasElement>();
  // Horizontal is JS-managed (viewLeft, in track px) rather than native scroll:
  // a genome zoomed to letter level needs a track far wider than the browser's
  // max DOM element width, so we can't lean on a physical spacer for the x axis.
  // Vertical uses native scroll (viewTop = scrollTop; the spacer is only as tall
  // as the stacked contigs, well within limits).
  let viewLeft = $state(0);
  let viewTop = $state(0);
  let viewW = $state(1000);
  let viewH = $state(600);

  const trackW = $derived(maxContigLen * pxPerBp); // full track width (excl. gutter)
  const trackViewW = $derived(Math.max(1, viewW - GUTTER)); // visible track width
  const maxViewLeft = $derived(Math.max(0, trackW - trackViewW));
  function setViewLeft(v: number) {
    viewLeft = Math.max(0, Math.min(maxViewLeft, v));
  }

  function screenX(bp: number): number {
    return GUTTER + bp * pxPerBp - viewLeft;
  }
  function bpAt(clientX: number): number {
    if (!scrollEl) return 0;
    const rect = scrollEl.getBoundingClientRect();
    const contentX = viewLeft + (clientX - rect.left);
    return (contentX - GUTTER) / pxPerBp;
  }

  // Which contigs intersect the vertical viewport, with resolved track offsets.
  type Frame = {
    ci: number;
    c: Contig;
    lay: Layout;
    yTop: number;
    rulerY: number; // -1 when a single shared ruler is pinned instead
    seqY: number;
    transY: number; // -1 if no translation track
    laneBaseY: number; // y of lane 0's top
    height: number;
    winL: number;
    winR: number;
  };
  // Frames for the contigs intersecting an arbitrary window. Parameterized (not
  // closing over the live viewport) so the exporter can lay out a window the
  // screen never shows — e.g. every contig at full track width.
  function computeFrames(vTop: number, vH: number, vLeft: number, vW: number): Frame[] {
    if (!data) return [];
    const { offsets } = vlayout;
    const contigs = data.contigs;
    // binary search first contig whose bottom > vTop
    let lo = 0,
      hi = contigs.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (offsets[mid + 1] <= vTop) lo = mid + 1;
      else hi = mid;
    }
    const out: Frame[] = [];
    for (let ci = lo; ci < contigs.length && offsets[ci] < vTop + vH; ci++) {
      const c = contigs[ci];
      const lay = contigLayout(c);
      const yTop = offsets[ci] + CONTIG_PAD_TOP;
      const rulerY = perContigRulerH > 0 ? yTop : -1;
      const seqY = yTop + perContigRulerH;
      const th = transTrackH(c);
      const transY = th > 0 ? seqY + seqTrackH() : -1;
      const laneBaseY = seqY + seqTrackH() + th;
      const winL = Math.max(0, vLeft / pxPerBp);
      const winR = Math.min(c.length, (vLeft + vW - GUTTER) / pxPerBp);
      out.push({
        ci, c, lay, yTop, rulerY, seqY, transY, laneBaseY,
        height: offsets[ci + 1] - offsets[ci],
        winL, winR,
      });
    }
    return out;
  }

  const frame = $derived.by((): Frame[] => computeFrames(viewTop, viewH, viewLeft, viewW));

  // ── genetic code + colors ──────────────────────────────────────────────────
  const CODON_TABLE: Record<string, string> = buildCodonTable();
  function buildCodonTable(): Record<string, string> {
    const b = "TCAG";
    const aa = "FFLLSSSSYY**CC*WLLLLPPPPHHQQRRRRIIIMTTTTNNKKSSRRVVVVAAAADDEEGGGG";
    const t: Record<string, string> = {};
    let i = 0;
    for (const x of b) for (const y of b) for (const z of b) t[x + y + z] = aa[i++];
    return t;
  }
  const NT_COLOR: Record<string, string> = {
    A: "#2BA02B", C: "#2F6FE0", G: "#E0A020", T: "#E03030", U: "#E03030",
  };
  // Light per-residue background (property-based); letters drawn dark on top.
  function aaBg(aa: string): string {
    if ("AVLIMFWPC".includes(aa)) return "#e8eefc"; // hydrophobic
    if ("GSTYNQ".includes(aa)) return "#e6f5ea"; // polar
    if ("KRH".includes(aa)) return "#fde9e9"; // basic
    if ("DE".includes(aa)) return "#f7e6f7"; // acidic
    if (aa === "*") return "#333333";
    return "#f0f0f0";
  }

  // Coding map for a CDS: peptide + per-coding-nucleotide genomic bp (reading
  // order), so codons can be placed at genomic coordinates for both strands and
  // multi-part locations. Cached per feature; cleared when the contig seq changes.
  let codingCache = new Map<string, { pep: string; pos: Int32Array }>();
  function codingMap(ci: number, f: FeatureView, seq: string): { pep: string; pos: Int32Array } | null {
    const key = ci + ":" + f.feat_index;
    const hit = codingCache.get(key);
    if (hit) return hit;
    const positions: number[] = [];
    const nts: string[] = [];
    const comp: Record<string, string> = { A: "T", T: "A", G: "C", C: "G", U: "A" };
    for (const p of f.parts) {
      if (p.strand === -1) {
        for (let bp = p.end - 1; bp >= p.start; bp--) {
          positions.push(bp);
          nts.push(comp[seq[bp]] ?? "N");
        }
      } else {
        for (let bp = p.start; bp < p.end; bp++) {
          positions.push(bp);
          nts.push(seq[bp] ?? "N");
        }
      }
    }
    // Honor /codon_start (1|2|3): the reading frame may begin 1–2 nt into the CDS.
    const cs = f.qualifiers.find(([k]) => k === "codon_start")?.[1];
    const off = Math.max(0, (cs ? parseInt(cs, 10) : 1) - 1) || 0;
    let pep = "";
    for (let i = off; i + 3 <= nts.length; i += 3) pep += CODON_TABLE[nts[i] + nts[i + 1] + nts[i + 2]] ?? "X";
    const rec = { pep, pos: Int32Array.from(off > 0 ? positions.slice(off) : positions) };
    codingCache.set(key, rec);
    return rec;
  }

  // ── selection ───────────────────────────────────────────────────────────────
  // Any number of items at once -- whole contigs, features, bp ranges -- kept in
  // `ws.viewSelection` (`selection.ts`) so the Enum Report tab shares it. A plain
  // click replaces the selection; Ctrl/Cmd+click adds to it (or removes a
  // feature or contig already in it). The last item is the "primary": the one the
  // coordinate readout, Fit selection, zoom centering and shift-click work on.
  const items = $derived(ws.viewSelection[path]?.items ?? []);
  function setItems(next: SelItem[]) {
    ws.setViewSelection(path, next, "contig");
  }
  // Features by `ci:feat_index`, to resolve feature items.
  const featByKey = $derived.by(() => {
    const m = new Map<string, FeatureView>();
    for (const c of data?.contigs ?? []) for (const f of c.features) m.set(`${c.index}:${f.feat_index}`, f);
    return m;
  });
  const featOf = (ci: number, feat: number) => featByKey.get(`${ci}:${feat}`);
  // The bp span an item covers (0-based half-open), or null if it no longer exists.
  function itemRange(it: SelItem): { ci: number; lo: number; hi: number } | null {
    const c = data?.contigs[it.ci];
    if (!c) return null;
    if (it.kind === "contig") return { ci: it.ci, lo: 0, hi: c.length };
    if (it.kind === "range") return { ci: it.ci, lo: Math.min(it.a, it.b), hi: Math.max(it.a, it.b) };
    const f = featOf(it.ci, it.feat);
    return f ? { ci: it.ci, lo: f.start, hi: f.end } : null;
  }
  const primary = $derived<SelItem | null>(items.length ? items[items.length - 1] : null);
  const primaryFeat = $derived(primary?.kind === "feature" ? (featOf(primary.ci, primary.feat) ?? null) : null);
  // Selected features (outlined) and contigs (gutter highlight), by key / index.
  const selFeatKeys = $derived(new Set(items.filter((it) => it.kind === "feature").map((it) => `${it.ci}:${it.feat}`)));
  const contigSel = $derived(new Set(items.filter((it) => it.kind === "contig").map((it) => it.ci)));
  // Highlight spans per contig, for drawing.
  const spansByContig = $derived.by(() => {
    const m = new Map<number, { lo: number; hi: number }[]>();
    for (const it of items) {
      const r = itemRange(it);
      if (!r || r.hi <= r.lo) continue;
      const list = m.get(r.ci);
      if (list) list.push(r);
      else m.set(r.ci, [r]);
    }
    return m;
  });
  // A click on empty track sets a caret (no selection); a drag from it, or a
  // later shift-click, makes a range.
  let caret = $state<{ ci: number; a: number } | null>(null);
  let dragging = false;
  // The range item a drag is growing (an index into `items`), or -1 until the
  // pointer has moved off the caret.
  let dragIdx = -1;
  let contigAnchor = -1; // last plainly-clicked contig (shift-range base)
  // Live cursor coordinate (bp within `cursorContig`), for the toolbar readout and
  // the canvas guide line. Null when the pointer is off the track.
  let cursorBp = $state<number | null>(null);
  let cursorContig = $state<number | null>(null);
  let copyMsg = $state<string | null>(null);
  // Shown in the copy buttons' tooltips so the shortcut is discoverable rather
  // than folklore. Cmd on macOS, Ctrl elsewhere.
  const COPY_HINT =
    typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform ?? "")
      ? "\u2318\u21E7C"
      : "Ctrl+Shift+C";

  // The primary item's span: what Fit selection, zoom centering and the readout use.
  const selRange = $derived(primary ? itemRange(primary) : null);
  const hasSel = $derived(items.some((it) => {
    const r = itemRange(it);
    return !!r && r.hi > r.lo;
  }));
  // Every item is a CDS on a nucleotide contig → offer the translation copy.
  const allNucCds = $derived(
    items.length > 0 &&
      items.every((it) => it.kind === "feature" && featOf(it.ci, it.feat)?.kind === "CDS" && !data?.contigs[it.ci].is_protein),
  );
  // Some item is on a nucleotide contig → offer the peptide extract.
  const anyNuc = $derived(items.some((it) => !!data && !data.contigs[it.ci]?.is_protein));
  // Coordinates are displayed 1-based inclusive — the GenBank/annotation
  // convention — while the model is 0-based half-open.
  const selCoords = $derived(
    selRange ? `${(selRange.lo + 1).toLocaleString()}:${selRange.hi.toLocaleString()}` : null,
  );
  const selUnit = $derived(selRange && data ? (data.contigs[selRange.ci].is_protein ? "aa" : "bp") : "bp");
  const caretLabel = $derived(caret && !items.length ? (caret.a + 1).toLocaleString() : null);
  const cursorLabel = $derived(cursorBp === null ? null : (Math.floor(cursorBp) + 1).toLocaleString());

  async function writeClip(text: string, msg: string) {
    try {
      await navigator.clipboard.writeText(text);
      copyMsg = msg;
    } catch (e) {
      copyMsg = "copy failed: " + String(e);
    }
    setTimeout(() => (copyMsg = null), 2500);
  }

  // The selected items' sequences, or null (and a fetch started) while any
  // contig's sequence is still loading. `translate` gives CDS translations.
  function selectedSeqs(translate: boolean): { name: string; seq: string }[] | null {
    if (!data) return null;
    const missing = [...new Set(items.map((it) => it.ci))].filter((ci) => seqCache[ci] === undefined);
    if (missing.length) {
      ensureSeqRange(Math.min(...missing), Math.max(...missing));
      return null;
    }
    const out: { name: string; seq: string }[] = [];
    for (const it of items) {
      const r = itemRange(it);
      if (!r || r.hi <= r.lo) continue;
      const c = data.contigs[it.ci];
      const id = c.id || c.name;
      const seq = seqCache[it.ci];
      if (translate) {
        const f = it.kind === "feature" ? featOf(it.ci, it.feat) : undefined;
        const cm = f ? codingMap(it.ci, f, seq) : null;
        if (cm) out.push({ name: `${f!.label || id}`, seq: cm.pep });
      } else {
        const whole = it.kind === "contig";
        out.push({ name: whole ? id : `${id}_${r.lo + 1}:${r.hi}`, seq: seq.slice(r.lo, r.hi) });
      }
    }
    return out;
  }

  // One item copies its bare sequence; several copy as FASTA.
  async function copySelection(translate: boolean) {
    const seqs = selectedSeqs(translate);
    if (seqs === null) {
      copyMsg = "loading sequence…";
      return;
    }
    if (!seqs.length) return;
    if (seqs.length === 1) {
      const unit = translate || data?.contigs[items[0].ci].is_protein ? "aa" : "bp";
      await writeClip(seqs[0].seq, `copied ${seqs[0].seq.length.toLocaleString()} ${unit}${translate ? " (translation)" : ""}`);
    } else {
      const fasta = seqs.map((s) => `>${s.name}\n${s.seq}\n`).join("");
      await writeClip(fasta, `copied ${seqs.length} sequences as FASTA`);
    }
  }

  // ── extract the selection to a new file, one record per item ─────────────────
  let showSelExtract = $state(false);
  let selExtractKind = $state<"nt" | "aa">("nt");
  const selExtractDefault = $derived.by(() => {
    const suffix = selExtractKind === "aa" ? "_peptides" : "";
    if (items.length === 1 && selRange && data) {
      const c = data.contigs[selRange.ci];
      const id = (c.id || c.name || "seq").replace(/[^\w.-]+/g, "_");
      // Both are GenBank: NT keeps the region's features; peptide runs the region's
      // CDSs through extract_peptides → protein GenBank.
      if (items[0].kind === "contig") return `${id}${suffix}.gb`;
      return `${id}_${selRange.lo + 1}_${selRange.hi}${suffix}.gb`;
    }
    if (items.every((it) => it.kind === "contig")) return `contigs_subset${suffix}.gb`;
    const base = (path.split(/[\\/]/).pop() ?? "selection").replace(/\.[^.]+$/, "");
    return `${base}_selection${suffix}.gb`;
  });

  function openSelExtract(kind: "nt" | "aa") {
    selExtractKind = kind;
    showSelExtract = true;
  }
  // Throws on failure so the modal shows it inline; closes on success. Slicing and
  // the peptide conversion both happen server-side. A minus-strand feature is
  // extracted reverse-complemented, so it reads forward in the new record.
  async function doSelExtract(name: string) {
    const dir = ws.currentDir;
    if (!data || !dir || !items.length) return;
    const req: ExtractItem[] = [];
    for (const it of items) {
      if (it.kind === "contig") {
        req.push({ contig: it.ci });
        continue;
      }
      const r = itemRange(it);
      if (!r || r.hi <= r.lo) continue;
      const strand = it.kind === "feature" && featOf(it.ci, it.feat)?.strand === -1 ? -1 : 1;
      req.push({ contig: it.ci, start: r.lo, end: r.hi, strand });
    }
    const p = await extractSelection(dir, name, path, req, selExtractKind === "aa");
    await ws.refreshAndOpen(p);
    showSelExtract = false;
  }

  // ── hit-testing ─────────────────────────────────────────────────────────────
  function frameAtY(y: number): Frame | null {
    for (const fr of frame) if (y >= fr.yTop && y < fr.yTop + fr.height) return fr;
    return null;
  }
  function featureAt(clientX: number, clientY: number): { ci: number; f: FeatureView } | null {
    if (!scrollEl) return null;
    const rect = scrollEl.getBoundingClientRect();
    const sx = clientX - rect.left;
    if (sx < GUTTER) return null;
    const contentY = viewTop + (clientY - rect.top);
    const fr = frameAtY(contentY);
    if (!fr) return null;
    const laneRegion = contentY - fr.laneBaseY;
    if (laneRegion < 0) return null;
    const bp = bpAt(clientX);
    if (stackAnnotations) {
      // Lanes overlap: the last-painted bar under the pointer wins.
      const ps = fr.lay.byLane;
      for (let i = ps.length - 1; i >= 0; i--) {
        const p = ps[i];
        const top = laneTopOffset(fr.lay, p.lane);
        if (laneRegion < top || laneRegion >= top + featH) continue;
        for (const part of p.f.parts) {
          if (bp >= part.start && bp <= part.end) return { ci: fr.ci, f: p.f };
        }
      }
      return null;
    }
    const laneFromTop = Math.floor(laneRegion / laneSlotH);
    const lane = fr.lay.maxLane - laneFromTop; // lane 0 drawn at bottom
    // topmost matching feature in that lane
    for (const p of fr.lay.placed) {
      if (p.lane !== lane) continue;
      for (const part of p.f.parts) {
        if (bp >= part.start && bp <= part.end) return { ci: fr.ci, f: p.f };
      }
    }
    return null;
  }

  // ── pointer interaction ─────────────────────────────────────────────────────
  const bpClamp = (clientX: number, len: number) =>
    Math.max(0, Math.min(len, Math.round(bpAt(clientX))));

  // Grow the range a drag is making to the bp under `clientX`. The range item is
  // created on the first move off the caret, so a click without a drag only sets
  // the caret.
  function dragTo(clientX: number) {
    if (!caret || !data) return;
    const b = bpClamp(clientX, data.contigs[caret.ci]?.length ?? 0);
    const range: SelItem = { kind: "range", ci: caret.ci, a: caret.a, b };
    if (dragIdx < 0) {
      if (b === caret.a) return;
      dragIdx = items.length;
      setItems([...items, range]);
    } else {
      const next = [...items];
      next[dragIdx] = range;
      setItems(next);
    }
  }

  function onPointerDown(e: PointerEvent) {
    if (!scrollEl || e.button !== 0) return; // right-click is the detail panel
    const rect0 = scrollEl.getBoundingClientRect();
    const additive = e.ctrlKey || e.metaKey;
    // The pinned shared ruler floats over the top of the viewport; a click on it
    // would otherwise select whatever scrolled underneath.
    if (sharedRuler && e.clientY - rect0.top < rulerBandH) return;
    // Click in the left margin (name gutter): select the whole contig. Ctrl/Cmd
    // toggles it in the selection, Shift adds the contigs from the last one clicked.
    if (e.clientX - rect0.left < GUTTER) {
      const fr = frameAtY(viewTop + (e.clientY - rect0.top));
      if (fr) {
        const ci = fr.ci;
        caret = null;
        if (e.shiftKey && contigAnchor >= 0) {
          const [lo, hi] = contigAnchor < ci ? [contigAnchor, ci] : [ci, contigAnchor];
          const have = new Set(items.map(itemKey));
          const add: SelItem[] = [];
          for (let i = lo; i <= hi; i++) {
            const it: SelItem = { kind: "contig", ci: i };
            if (!have.has(itemKey(it))) add.push(it);
          }
          setItems([...items, ...add]);
        } else {
          const it: SelItem = { kind: "contig", ci };
          setItems(additive ? toggleItem(items, it) : [it]);
          contigAnchor = ci;
        }
        ensureSeq(ci);
        scheduleDraw();
      }
      return;
    }
    // Shift-click in the track extends the primary item (or the caret) on the
    // clicked contig into a range ending at the clicked bp — the way to select a
    // span wider than the viewport: click one end, scroll, shift-click the other.
    if (e.shiftKey) {
      const fr = frameAtY(viewTop + (e.clientY - rect0.top));
      if (fr) {
        let anchor: number | null = null;
        let replace = false;
        if (primary && primary.ci === fr.ci) {
          const r = itemRange(primary);
          anchor = primary.kind === "range" ? primary.a : (r?.lo ?? null);
          replace = anchor !== null;
        } else if (caret && caret.ci === fr.ci) {
          anchor = caret.a;
        }
        if (anchor !== null) {
          caret = { ci: fr.ci, a: anchor };
          dragIdx = replace ? items.length - 1 : -1;
          dragTo(e.clientX);
          ensureSeq(fr.ci);
          dragging = true;
          dragClientX = e.clientX;
          scrollEl.setPointerCapture?.(e.pointerId);
          scheduleDraw();
          return;
        }
      }
    }
    const hit = featureAt(e.clientX, e.clientY);
    if (hit) {
      // Select the feature (its span highlights and can be copied or extracted).
      const it: SelItem = { kind: "feature", ci: hit.ci, feat: hit.f.feat_index };
      setItems(additive ? toggleItem(items, it) : [it]);
      caret = null;
      ensureSeq(hit.ci); // load sequence so Copy is ready
      scheduleDraw();
      return;
    }
    // Plain click sets a caret (anchor) at this bp and clears the selection; a
    // drag from it makes a range. With Ctrl/Cmd the range is added instead.
    const fr = frameAtY(viewTop + (e.clientY - rect0.top));
    if (!fr) return;
    caret = { ci: fr.ci, a: bpClamp(e.clientX, fr.c.length) };
    if (!additive && items.length) setItems([]);
    dragIdx = -1;
    ensureSeq(fr.ci);
    dragging = true;
    dragClientX = e.clientX;
    scrollEl.setPointerCapture?.(e.pointerId);
    scheduleDraw();
  }

  // Right-click on a feature pins its details (and, for GenBank, its edit
  // controls) in the side panel; on anything else it closes the panel.
  function onContextMenu(e: MouseEvent) {
    e.preventDefault();
    const hit = featureAt(e.clientX, e.clientY);
    inspected = hit ? { contigIndex: hit.ci, feat: hit.f } : null;
    scheduleDraw();
  }
  // Track the bp under the cursor so the toolbar can show a live coordinate and
  // the canvas can draw a guide line at it.
  function updateCursorBp(e: PointerEvent) {
    if (!scrollEl) return;
    const rect = scrollEl.getBoundingClientRect();
    if (e.clientX - rect.left < GUTTER) {
      cursorBp = null;
      cursorContig = null;
      return;
    }
    const fr = frameAtY(viewTop + (e.clientY - rect.top));
    if (!fr) {
      cursorBp = null;
      cursorContig = null;
      return;
    }
    const bp = bpAt(e.clientX);
    cursorBp = bp < 0 || bp > fr.c.length ? null : bp;
    cursorContig = cursorBp === null ? null : fr.ci;
  }

  function onPointerMove(e: PointerEvent) {
    updateCursorBp(e);
    if (dragging && caret && scrollEl) {
      dragClientX = e.clientX;
      dragTo(e.clientX);
      maybeAutoScroll(); // slide the viewport if dragging past an edge
      scheduleDraw();
      return;
    }
    const rect = scrollEl?.getBoundingClientRect();
    const fr = rect && e.clientX - rect.left < GUTTER
      ? frameAtY(viewTop + e.clientY - rect.top) : null;
    nameTooltip = fr ? {
      x: e.clientX + 12, y: e.clientY + 12,
      name: fr.c.name && fr.c.name !== fr.c.id ? `${fr.c.id} (${fr.c.name})` : fr.c.id || fr.c.name,
    } : null;
    const hit = featureAt(e.clientX, e.clientY);
    tooltip = hit ? { x: e.clientX + 12, y: e.clientY + 12, feature: hit.f } : null;
    scheduleDraw(); // the hover guide follows the cursor
  }
  function onPointerUp() {
    dragging = false;
    // A finished range anchors later shift-clicks itself; the caret is only for
    // a click that made none.
    if (dragIdx >= 0) caret = null;
    dragIdx = -1;
    stopAutoScroll();
    scheduleDraw();
  }
  function onPointerLeave() {
    tooltip = null;
    nameTooltip = null;
    cursorBp = null;
    cursorContig = null;
    scheduleDraw();
  }

  function resizeGutter(e: PointerEvent) {
    e.preventDefault();
    e.stopPropagation();
    const handle = e.currentTarget as HTMLElement;
    handle.setPointerCapture(e.pointerId);
    const startX = e.clientX;
    const startWidth = GUTTER;
    handle.onpointermove = (move) => {
      move.stopPropagation();
      GUTTER = Math.max(80, Math.min(viewW - 80, startWidth + move.clientX - startX));
      setViewLeft(viewLeft);
      scheduleDraw();
    };
    handle.onpointerup = handle.onpointercancel = () => {
      handle.onpointermove = null;
      handle.onpointerup = null;
      handle.onpointercancel = null;
    };
  }

  // ── drag auto-scroll (extend selection past the viewport edge) ────────────────
  let dragClientX = 0;
  let autoRaf = 0;
  function edgeVelocity(): number {
    if (!scrollEl) return 0;
    const rect = scrollEl.getBoundingClientRect();
    const M = 44; // px from an edge that triggers auto-scroll
    const left = rect.left + GUTTER;
    if (dragClientX > rect.right - M) return Math.min(36, 4 + ((dragClientX - (rect.right - M)) / M) * 32);
    if (dragClientX < left + M) return -Math.min(36, 4 + ((left + M - dragClientX) / M) * 32);
    return 0;
  }
  function autoScrollTick() {
    const v = edgeVelocity();
    if (v === 0 || !dragging || !caret) {
      autoRaf = 0;
      return;
    }
    setViewLeft(viewLeft + v);
    dragTo(dragClientX);
    scheduleDraw();
    autoRaf = requestAnimationFrame(autoScrollTick);
  }
  function maybeAutoScroll() {
    if (!autoRaf && dragging && edgeVelocity() !== 0) autoRaf = requestAnimationFrame(autoScrollTick);
  }
  function stopAutoScroll() {
    if (autoRaf) cancelAnimationFrame(autoRaf);
    autoRaf = 0;
  }

  // ── zoom (relative to cursor) ────────────────────────────────────────────────
  let lastCursorX = 334; // screen px within the scroll box (for +/- buttons)
  // Rescale by `factor`, keeping genomic position `bp` pinned at screen x `screenPx`.
  function zoomKeeping(bp: number, screenPx: number, factor: number) {
    const next = Math.min(20, Math.max(0.002, pxPerBp * factor));
    if (next === pxPerBp) return;
    pxPerBp = next;
    setViewLeft(bp * pxPerBp - (screenPx - GUTTER));
    scheduleDraw();
  }
  function zoomAround(screenPx: number, factor: number) {
    // bp currently under the cursor (using the OLD scale).
    zoomKeeping((viewLeft + screenPx - GUTTER) / pxPerBp, screenPx, factor);
  }
  function zoomBtn(factor: number) {
    // With a selection active, keep it centered in the viewport across zooms;
    // otherwise anchor on the last cursor position.
    if (selRange) {
      centerOnSelection();
      zoomKeeping((selRange.lo + selRange.hi) / 2, GUTTER + trackViewW / 2, factor);
    } else {
      zoomAround(lastCursorX, factor);
    }
  }
  function fit() {
    if (!data) return;
    pxPerBp = Math.min(2, Math.max(0.002, (viewW - GUTTER) / maxContigLen));
    setViewLeft(0);
    scheduleDraw();
  }
  // Zoom so the current selection fills the track width (with a little margin),
  // and scroll it into view vertically.
  function fitSelection() {
    if (!selRange || !data) return;
    const span = Math.max(1, selRange.hi - selRange.lo);
    pxPerBp = Math.min(20, Math.max(0.002, (trackViewW * 0.92) / span));
    centerOnSelection();
  }
  // Pan (horizontally, and vertically if needed) so the selection is centered.
  function centerOnSelection() {
    if (!selRange || !scrollEl) return;
    const mid = (selRange.lo + selRange.hi) / 2;
    setViewLeft(mid * pxPerBp - trackViewW / 2);
    const off = vlayout.offsets[selRange.ci];
    if (off !== undefined) {
      const target = off + (vlayout.offsets[selRange.ci + 1] - off) / 2 - viewH / 2;
      scrollEl.scrollTop = Math.max(0, target);
    }
    scheduleDraw();
  }
  function onWheel(e: WheelEvent) {
    if (!scrollEl) return;
    if (e.ctrlKey || e.metaKey) {
      e.preventDefault();
      const rect = scrollEl.getBoundingClientRect();
      zoomAround(e.clientX - rect.left, e.deltaY < 0 ? 1.2 : 1 / 1.2);
      return;
    }
    // Horizontal intent (shift, or a trackpad's dominant deltaX) pans the track;
    // otherwise let native vertical scroll happen.
    const dx = e.shiftKey ? e.deltaY : e.deltaX;
    if (dx !== 0 && (e.shiftKey || Math.abs(e.deltaX) > Math.abs(e.deltaY))) {
      e.preventDefault();
      setViewLeft(viewLeft + dx);
      scheduleDraw();
    }
  }

  // Custom horizontal scrollbar: drag the thumb (or click the track) to pan.
  function onHbarDown(e: PointerEvent) {
    const bar = e.currentTarget as HTMLElement;
    const rect = bar.getBoundingClientRect();
    const thumbW = Math.max(24, (trackViewW / trackW) * rect.width);
    const apply = (clientX: number) => {
      const rel = clientX - rect.left - thumbW / 2;
      const span = Math.max(1, rect.width - thumbW);
      setViewLeft((rel / span) * maxViewLeft);
      scheduleDraw();
    };
    apply(e.clientX);
    const move = (ev: PointerEvent) => apply(ev.clientX);
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }

  // ── ruler ticks ───────────────────────────────────────────────────────────────
  // Tick positions for the window [vLeft, vLeft + vW - GUTTER) in track px, as
  // 1-based residue numbers (base 1, then every multiple of `step`) -- the same
  // numbering the selection readout and the detail panel use. Base n spans
  // [n-1, n) in the 0-based track coordinates, so its tick is drawn at n - 0.5.
  function ticks(length: number, vLeft: number, vW: number): { pos: number[]; step: number } {
    const spanBp = Math.min(length, (vW - GUTTER) / pxPerBp);
    const n = Math.max(3, Math.floor((vW - GUTTER) / 110));
    const raw = spanBp / n;
    const mag = Math.pow(10, Math.floor(Math.log10(Math.max(1, raw))));
    const step = Math.max(1, Math.round(raw / mag) * mag);
    const hi = Math.min(length, (vLeft + vW - GUTTER) / pxPerBp + 1);
    let first = Math.floor(Math.max(0, vLeft / pxPerBp) / step) * step;
    const pos: number[] = [];
    if (first < step) {
      if (length >= 1 && step > 1) pos.push(1);
      first = step;
    }
    for (let x = first; x <= hi; x += step) pos.push(x);
    return { pos, step };
  }

  // Tick label in the chosen unit. `auto` switches to kb once the tick spacing is
  // at least 1 kb, which is where raw bp counts stop being readable.
  function tickLabel(bp: number, step: number): string {
    const kb = rulerUnits === "kb" || (rulerUnits === "auto" && step >= 1000);
    // The leading base-1 tick would round to "0 kb", which reads as 0-based.
    if (!kb || bp === 1) return bp.toLocaleString();
    const v = bp / 1000;
    // Enough decimals to keep consecutive ticks distinct.
    const dp = step >= 1000 ? 0 : step >= 100 ? 1 : 2;
    return `${v.toLocaleString(undefined, { minimumFractionDigits: dp, maximumFractionDigits: dp })} kb`;
  }
  // Unit suffix for the shared ruler's gutter caption.
  const rulerUnitLabel = $derived(rulerUnits === "kb" ? "kb" : rulerUnits === "bp" ? "bp" : "bp/kb");

  // ── canvas draw ───────────────────────────────────────────────────────────────
  // The 2D subset both backends implement (real canvas, and the SVG shim below).
  type Ctx2D = CanvasRenderingContext2D;

  function drawArrow(ctx: Ctx2D, x: number, y: number, w: number, h: number, mode: "box" | "right" | "left") {
    const head = Math.min(Math.round((h * 4) / 7), Math.max(0, w)); // 8px at the default 14px bar
    ctx.beginPath();
    if (mode === "right") {
      const b = x + w - head;
      ctx.moveTo(x, y); ctx.lineTo(b, y); ctx.lineTo(x + w, y + h / 2); ctx.lineTo(b, y + h); ctx.lineTo(x, y + h);
    } else if (mode === "left") {
      ctx.moveTo(x + w, y); ctx.lineTo(x + head, y); ctx.lineTo(x, y + h / 2); ctx.lineTo(x + head, y + h); ctx.lineTo(x + w, y + h);
    } else {
      ctx.moveTo(x, y); ctx.lineTo(x + w, y); ctx.lineTo(x + w, y + h); ctx.lineTo(x, y + h);
    }
    ctx.closePath();
  }

  type Palette = {
    text: string; dim: string; border: string; accent: string; panel: string; bg: string;
  };
  function palette(): Palette {
    const css = scrollEl ? getComputedStyle(scrollEl) : null;
    const get = (n: string, d: string) => css?.getPropertyValue(n).trim() || d;
    return {
      text: get("--text", "#111"),
      dim: get("--text-dim", "#888"),
      border: get("--border", "#ccc"),
      accent: get("--accent", "#3b82f6"),
      panel: get("--bg-panel", "#fff"),
      bg: get("--bg", "#ffffff"),
    };
  }

  // One window of the view, drawn into any Ctx2D. Everything positional is derived
  // from `scene` rather than the live viewport, so the exporter can render a window
  // the screen never shows (all contigs, full track width) with identical output.
  type Scene = {
    frames: Frame[];
    vLeft: number;
    vTop: number;
    vW: number;
    vH: number;
    pal: Palette;
    interactive: boolean; // selection highlight, caret, hover guide
    opaque: boolean; // paint a background (exports need one; the live canvas is layered)
  };

  function renderScene(ctx: Ctx2D, scene: Scene) {
    if (!data) return;
    const { frames, vLeft, vTop, vW, vH, pal } = scene;
    const cText = pal.text, cDim = pal.dim, cBorder = pal.border, cAccent = pal.accent, cPanel = pal.panel;
    const cSeqBar = cDim;
    const sx = (bp: number) => GUTTER + bp * pxPerBp - vLeft;

    if (scene.opaque) {
      ctx.fillStyle = pal.bg;
      ctx.fillRect(0, 0, vW, vH);
    }

    // Font tracks cell width (≈0.6em glyph advance) so letters fit even at the
    // lower zoom threshold without overlapping.
    const letterFont = `${Math.min(13, Math.max(6, pxPerBp * 1.15))}px ui-monospace, monospace`;
    const rulerFont = `${rulerFontSize}px ui-monospace, monospace`;
    const featFont = `${labelFontSize}px system-ui, sans-serif`;

    // Draw one ruler band: baseline at `ry`, ticks over [0, length].
    const drawRuler = (ry: number, length: number) => {
      ctx.strokeStyle = cBorder;
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(Math.max(GUTTER, sx(0)), ry + 0.5);
      ctx.lineTo(sx(length), ry + 0.5);
      ctx.stroke();
      ctx.fillStyle = cDim;
      ctx.font = rulerFont;
      ctx.textAlign = "left";
      ctx.textBaseline = "alphabetic";
      const { pos, step } = ticks(length, vLeft, vW);
      for (const t of pos) {
        const tx = sx(t - 0.5);
        ctx.beginPath();
        ctx.moveTo(tx + 0.5, ry - 4);
        ctx.lineTo(tx + 0.5, ry);
        ctx.stroke();
        ctx.fillText(tickLabel(t, step), tx + 2, ry - 6);
      }
    };

    // Clip tracks to the right of the gutter.
    ctx.save();
    ctx.beginPath();
    ctx.rect(GUTTER, 0, vW - GUTTER, vH);
    ctx.clip();

    for (const fr of frames) {
      const sYtop = fr.yTop - vTop;
      const selH = fr.height - contigGap;
      // selection highlights, and the caret, on this contig
      if (scene.interactive) {
        ctx.fillStyle = "rgba(59,130,246,0.18)";
        for (const r of spansByContig.get(fr.ci) ?? []) {
          const x1 = sx(r.lo);
          ctx.fillRect(x1, sYtop, Math.max(1, sx(r.hi) - x1), selH);
        }
        // focus border of the primary range (the edge a shift-click / drag moves)
        // or a lone caret
        const edge =
          primary?.kind === "range" && primary.ci === fr.ci ? primary.b
          : caret && caret.ci === fr.ci ? caret.a
          : null;
        const cx = edge === null ? NaN : sx(edge);
        if (cx >= GUTTER - 1 && cx <= vW) {
          ctx.fillStyle = cAccent;
          ctx.fillRect(cx - 0.75, sYtop, 1.5, selH);
        }
      }

      // per-contig ruler (absent when a single shared ruler is pinned on top)
      if (fr.rulerY >= 0) drawRuler(fr.rulerY - vTop + perContigRulerH - 1, fr.c.length);

      // sequence track
      const seq = seqCache[fr.ci];
      const seqTop = fr.seqY - vTop;
      if (showLetters && seq !== undefined) {
        ctx.font = letterFont;
        ctx.textAlign = "center";
        ctx.textBaseline = "middle";
        const lo = Math.max(0, Math.floor(fr.winL));
        const hi = Math.min(fr.c.length, Math.ceil(fr.winR));
        const isProt = fr.c.is_protein;
        for (let bp = lo; bp < hi; bp++) {
          const ch = seq[bp] ?? "N";
          const cx = sx(bp) + pxPerBp / 2;
          if (isProt) {
            ctx.fillStyle = aaBg(ch);
            ctx.fillRect(sx(bp), seqTop, pxPerBp, SEQ_H);
            ctx.fillStyle = "#111";
          } else {
            ctx.fillStyle = NT_COLOR[ch] ?? "#777";
          }
          ctx.fillText(ch, cx, seqTop + SEQ_H / 2 + 0.5);
        }
      } else {
        // collapsed sequence bar
        ctx.fillStyle = cSeqBar;
        const x1 = Math.max(GUTTER, sx(0));
        const x2 = sx(fr.c.length);
        ctx.globalAlpha = 0.35;
        ctx.fillRect(x1, seqTop + 1, Math.max(1, x2 - x1), SEQBAR_H - 2);
        ctx.globalAlpha = 1;
      }

      // translation track (nucleotide contigs, CDS codons)
      if (fr.transY >= 0 && seq !== undefined) {
        const ty = fr.transY - vTop;
        ctx.font = letterFont;
        ctx.textAlign = "center";
        ctx.textBaseline = "middle";
        for (const p of fr.lay.placed) {
          if (p.f.kind !== "CDS") continue;
          if (p.f.end < fr.winL || p.f.start > fr.winR) continue;
          const cm = codingMap(fr.ci, p.f, seq);
          if (!cm) continue;
          for (let k = 0; k < cm.pep.length; k++) {
            const g0 = cm.pos[3 * k];
            const g2 = cm.pos[3 * k + 2];
            if (g0 === undefined || g2 === undefined) break;
            const lo = Math.min(g0, g2);
            const hi = Math.max(g0, g2) + 1;
            if (hi < fr.winL || lo > fr.winR) continue;
            const aa = cm.pep[k];
            const cx = sx((lo + hi) / 2);
            const w = (hi - lo) * pxPerBp;
            ctx.fillStyle = aaBg(aa);
            ctx.fillRect(sx(lo), ty, w, TRANS_H);
            ctx.fillStyle = aa === "*" ? "#fff" : "#111";
            if (w >= 5) ctx.fillText(aa, cx, ty + TRANS_H / 2 + 0.5);
          }
        }
      }

      // feature lanes
      ctx.textAlign = "left";
      ctx.textBaseline = "middle";
      ctx.font = featFont;
      // Rightmost x already claimed by an outside label, per lane. `placed` is in
      // left-to-right order within a lane (the layout sorts by start), so one
      // running value per lane is enough.
      const labelRight = new Map<number, number>();
      for (const p of stackAnnotations ? fr.lay.byLane : fr.lay.placed) {
        const f = p.f;
        if (f.end < fr.winL || f.start > fr.winR) continue;
        const laneTop = fr.laneBaseY - vTop + laneTopOffset(fr.lay, p.lane);
        // parts + connectors
        const sp = [...f.parts].sort((a, b) => a.start - b.start);
        let term = -1;
        if (f.strand === 1) {
          let maxEnd = -Infinity;
          sp.forEach((pt, i) => { if (pt.end > maxEnd) { maxEnd = pt.end; term = i; } });
        } else if (f.strand === -1) term = 0;
        // connectors
        ctx.strokeStyle = cDim;
        ctx.lineWidth = 1;
        for (let i = 0; i < sp.length - 1; i++) {
          const gap = sp[i + 1].start - sp[i].end;
          if (gap > 0 && gap < fr.c.length * 0.5) {
            ctx.beginPath();
            ctx.moveTo(sx(sp[i].end), laneTop + featH / 2);
            ctx.lineTo(sx(sp[i + 1].start), laneTop + featH / 2);
            ctx.stroke();
          }
        }
        const isSel =
          scene.interactive &&
          (selFeatKeys.has(`${fr.ci}:${f.feat_index}`) ||
            (inspected?.contigIndex === fr.ci && inspected?.feat.feat_index === f.feat_index));
        for (let i = 0; i < sp.length; i++) {
          const x = sx(sp[i].start);
          const w = Math.max(1.5, (sp[i].end - sp[i].start) * pxPerBp);
          const mode = i === term ? (f.strand === 1 ? "right" : "left") : "box";
          drawArrow(ctx, x, laneTop, w, featH, mode);
          ctx.fillStyle = f.color;
          ctx.fill();
          ctx.lineWidth = isSel ? 2 : 0.5;
          ctx.strokeStyle = isSel ? cAccent : "rgba(0,0,0,0.35)";
          ctx.stroke();
        }
        const fw = (f.end - f.start) * pxPerBp;
        // inside label: clipped to the feature box, only when it has room
        if ((labelPlacement !== "outside" || stackAnnotations) && fw > 34) {
          const lx = Math.max(GUTTER + 2, sx(f.start) + 4);
          ctx.fillStyle = labelColor(f.color);
          ctx.textAlign = "left";
          ctx.save();
          ctx.beginPath();
          ctx.rect(sx(f.start), laneTop, fw, featH);
          ctx.clip();
          ctx.fillText(f.label, lx, laneTop + featH / 2 + 0.5);
          ctx.restore();
        }
        // outside label: centered over the feature in its own strip, unclipped.
        // Neighbors in the same lane would otherwise overprint each other, so a
        // label is skipped when it would collide with the last one drawn there.
        if (labelPlacement !== "inside" && !stackAnnotations && fw > 2) {
          // Center on the visible part of the feature, so a feature running off
          // the edge still labels the part you can see.
          const l = Math.max(sx(f.start), GUTTER);
          const r = Math.min(sx(f.end), vW);
          const mid = (l + r) / 2;
          ctx.textAlign = "center";
          ctx.textBaseline = "alphabetic";
          const halfW = ctx.measureText(f.label).width / 2;
          const taken = labelRight.get(p.lane) ?? -Infinity;
          if (mid - halfW >= taken) {
            ctx.fillStyle = cText;
            ctx.fillText(f.label, mid, laneTop - 2);
            labelRight.set(p.lane, mid + halfW + 4);
          }
          ctx.textBaseline = "middle";
          ctx.textAlign = "left";
        }
      }
    }

    // hover guide: a vertical line at the cursor's bp, so the readout has a mark
    if (scene.interactive && cursorBp !== null && !dragging) {
      const gx = sx(cursorBp);
      if (gx >= GUTTER && gx <= vW) {
        ctx.strokeStyle = cAccent;
        ctx.globalAlpha = 0.5;
        ctx.lineWidth = 1;
        ctx.beginPath();
        ctx.moveTo(gx + 0.5, 0);
        ctx.lineTo(gx + 0.5, vH);
        ctx.stroke();
        ctx.globalAlpha = 1;
      }
    }
    ctx.restore();

    // ── gutter (pinned left column) ──
    ctx.fillStyle = cPanel;
    ctx.fillRect(0, 0, GUTTER, vH);
    ctx.strokeStyle = cBorder;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(GUTTER + 0.5, 0);
    ctx.lineTo(GUTTER + 0.5, vH);
    ctx.stroke();
    ctx.textAlign = "left";
    ctx.textBaseline = "alphabetic";
    for (const fr of frames) {
      // Multi-select highlight for the whole contig row in the gutter.
      if (scene.interactive && contigSel.has(fr.ci)) {
        ctx.fillStyle = "rgba(59,130,246,0.18)";
        ctx.fillRect(0, fr.yTop - CONTIG_PAD_TOP - vTop, GUTTER, fr.height - contigGap);
      }
      const yid = fr.seqY - vTop + seqTrackH() / 2 + 3;
      ctx.fillStyle = cText;
      ctx.font = "600 11px ui-monospace, monospace";
      ctx.fillText(clip(`${fr.ci + 1}. ${fr.c.id || fr.c.name}`, GUTTER - 8, ctx), 6, yid);
      if (fr.transY >= 0) {
        ctx.fillStyle = cDim;
        ctx.font = "10px system-ui, sans-serif";
        ctx.fillText("Translation", 6, fr.transY - vTop + TRANS_H / 2 + 4);
      }
    }

    // ── shared ruler (pinned band across the top, all contigs at once) ──
    if (sharedRuler) {
      ctx.fillStyle = cPanel;
      ctx.fillRect(0, 0, vW, rulerBandH);
      ctx.save();
      ctx.beginPath();
      ctx.rect(GUTTER, 0, vW - GUTTER, rulerBandH);
      ctx.clip();
      drawRuler(rulerBandH - 1, maxContigLen);
      ctx.restore();
      ctx.fillStyle = cDim;
      ctx.font = "10px system-ui, sans-serif";
      ctx.textAlign = "left";
      ctx.textBaseline = "alphabetic";
      ctx.fillText(rulerUnitLabel, 6, rulerBandH - 3);
      ctx.strokeStyle = cBorder;
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(0, rulerBandH + 0.5);
      ctx.lineTo(vW, rulerBandH + 0.5);
      ctx.stroke();
    }
  }

  function draw() {
    const cv = canvas;
    if (!cv || !data || !scrollEl) return;
    const dpr = window.devicePixelRatio || 1;
    cv.width = Math.round(viewW * dpr);
    cv.height = Math.round(viewH * dpr);
    cv.style.width = viewW + "px";
    cv.style.height = viewH + "px";
    cv.style.left = "0px"; // fills the viewport width; x is handled by viewLeft in-draw
    cv.style.top = viewTop + "px"; // follows native vertical scroll
    const ctx = cv.getContext("2d")!;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, viewW, viewH);
    renderScene(ctx, {
      frames: frame,
      vLeft: viewLeft, vTop: viewTop, vW: viewW, vH: viewH,
      pal: palette(),
      interactive: true,
      opaque: false,
    });
  }

  function clip(s: string, maxW: number, ctx: CanvasRenderingContext2D): string {
    if (ctx.measureText(s).width <= maxW) return s;
    let t = s;
    while (t.length > 1 && ctx.measureText(t + "…").width > maxW) t = t.slice(0, -1);
    return t + "…";
  }
  function labelColor(bg: string): string {
    const m = /^#([0-9a-f]{6})$/i.exec(bg);
    if (!m) return "#111";
    const n = parseInt(m[1], 16);
    const r = (n >> 16) & 255, g = (n >> 8) & 255, b = n & 255;
    return 0.299 * r + 0.587 * g + 0.114 * b > 150 ? "#111" : "#fff";
  }

  // ── SVG backend ──────────────────────────────────────────────────────────────
  // `renderScene` is written against the canvas API; this records the same calls
  // as SVG elements instead. Implementing the drawing *interface* rather than
  // writing a second renderer is what keeps the export honest — there is exactly
  // one description of what the view looks like, so an SVG can never drift from
  // the pixels on screen. Only the subset `renderScene` uses is implemented.
  const xmlEsc = (s: string) =>
    s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
  const n2 = (v: number) => (Math.round(v * 100) / 100).toString();

  // Canvas accepts `rgba(...)`; SVG 1.1 renderers (Inkscape, Illustrator) want the
  // alpha as a separate attribute, so split it out.
  function splitColor(c: string): { color: string; alpha: number } {
    const m = /^rgba\(\s*([\d.]+)\s*,\s*([\d.]+)\s*,\s*([\d.]+)\s*,\s*([\d.]+)\s*\)$/i.exec(c);
    if (m) return { color: `rgb(${m[1]},${m[2]},${m[3]})`, alpha: parseFloat(m[4]) };
    return { color: c, alpha: 1 };
  }
  function parseFont(f: string): { weight: string; size: number; family: string } {
    const m = /^\s*(?:(\d+)\s+)?([\d.]+)px\s+(.+)$/.exec(f);
    if (!m) return { weight: "", size: 10, family: "sans-serif" };
    return { weight: m[1] ?? "", size: parseFloat(m[2]), family: m[3].trim() };
  }

  type Rect = { x: number; y: number; w: number; h: number };

  class SvgContext {
    fillStyle = "#000";
    strokeStyle = "#000";
    lineWidth = 1;
    font = "10px sans-serif";
    textAlign: CanvasTextAlign = "left";
    textBaseline: CanvasTextBaseline = "alphabetic";
    globalAlpha = 1;

    private body: string[] = [];
    private defs: string[] = [];
    private path = "";
    private rects: Rect[] = [];
    private openGroups = 0;
    private clipSeq = 0;
    private stack: { style: string[]; groups: number }[] = [];
    private meas: CanvasRenderingContext2D;

    constructor(meas: CanvasRenderingContext2D) {
      this.meas = meas;
    }

    // no-ops: the SVG is authored in CSS pixels, with no device-pixel scaling
    setTransform() {}
    clearRect() {}

    beginPath() {
      this.path = "";
      this.rects = [];
    }
    moveTo(x: number, y: number) {
      this.path += `M${n2(x)} ${n2(y)}`;
    }
    lineTo(x: number, y: number) {
      this.path += `L${n2(x)} ${n2(y)}`;
    }
    closePath() {
      this.path += "Z";
    }
    rect(x: number, y: number, w: number, h: number) {
      this.rects.push({ x, y, w, h });
      this.path += `M${n2(x)} ${n2(y)}H${n2(x + w)}V${n2(y + h)}H${n2(x)}Z`;
    }

    private paint(attrs: string) {
      this.body.push(attrs);
    }
    fill() {
      if (!this.path) return;
      const { color, alpha } = splitColor(this.fillStyle);
      const a = alpha * this.globalAlpha;
      this.paint(
        `<path d="${this.path}" fill="${xmlEsc(color)}"${a < 1 ? ` fill-opacity="${n2(a)}"` : ""}/>`,
      );
    }
    stroke() {
      if (!this.path) return;
      const { color, alpha } = splitColor(this.strokeStyle);
      const a = alpha * this.globalAlpha;
      this.paint(
        `<path d="${this.path}" fill="none" stroke="${xmlEsc(color)}" stroke-width="${n2(this.lineWidth)}"` +
          `${a < 1 ? ` stroke-opacity="${n2(a)}"` : ""}/>`,
      );
    }
    fillRect(x: number, y: number, w: number, h: number) {
      if (w <= 0 || h <= 0) return;
      const { color, alpha } = splitColor(this.fillStyle);
      const a = alpha * this.globalAlpha;
      this.paint(
        `<rect x="${n2(x)}" y="${n2(y)}" width="${n2(w)}" height="${n2(h)}" fill="${xmlEsc(color)}"` +
          `${a < 1 ? ` fill-opacity="${n2(a)}"` : ""}/>`,
      );
    }
    clip() {
      const id = `c${this.clipSeq++}`;
      const rs = this.rects
        .map((r) => `<rect x="${n2(r.x)}" y="${n2(r.y)}" width="${n2(r.w)}" height="${n2(r.h)}"/>`)
        .join("");
      this.defs.push(`<clipPath id="${id}">${rs}</clipPath>`);
      this.body.push(`<g clip-path="url(#${id})">`);
      this.openGroups++;
    }
    fillText(text: string, x: number, y: number) {
      if (!text) return;
      const { weight, size, family } = parseFont(this.font);
      const { color, alpha } = splitColor(this.fillStyle);
      const a = alpha * this.globalAlpha;
      const anchor =
        this.textAlign === "center" ? "middle" : this.textAlign === "right" || this.textAlign === "end" ? "end" : "start";
      // Canvas "middle" is the em-box center; SVG's nearest equivalent is `central`.
      const base = this.textBaseline === "middle" ? ` dominant-baseline="central"` : "";
      this.paint(
        `<text x="${n2(x)}" y="${n2(y)}" font-family="${xmlEsc(family)}" font-size="${n2(size)}"` +
          `${weight ? ` font-weight="${weight}"` : ""} text-anchor="${anchor}"${base}` +
          ` fill="${xmlEsc(color)}"${a < 1 ? ` fill-opacity="${n2(a)}"` : ""}>${xmlEsc(text)}</text>`,
      );
    }
    measureText(t: string): TextMetrics {
      this.meas.font = this.font;
      return this.meas.measureText(t);
    }
    save() {
      this.stack.push({
        style: [this.fillStyle, this.strokeStyle, String(this.lineWidth), this.font, this.textAlign, this.textBaseline, String(this.globalAlpha)],
        groups: this.openGroups,
      });
    }
    restore() {
      const s = this.stack.pop();
      if (!s) return;
      while (this.openGroups > s.groups) {
        this.body.push("</g>");
        this.openGroups--;
      }
      [this.fillStyle, this.strokeStyle] = [s.style[0], s.style[1]];
      this.lineWidth = Number(s.style[2]);
      this.font = s.style[3];
      this.textAlign = s.style[4] as CanvasTextAlign;
      this.textBaseline = s.style[5] as CanvasTextBaseline;
      this.globalAlpha = Number(s.style[6]);
    }

    toSvg(w: number, h: number): string {
      while (this.openGroups > 0) {
        this.body.push("</g>");
        this.openGroups--;
      }
      return (
        `<?xml version="1.0" encoding="UTF-8"?>\n` +
        `<svg xmlns="http://www.w3.org/2000/svg" width="${n2(w)}" height="${n2(h)}" ` +
        `viewBox="0 0 ${n2(w)} ${n2(h)}">` +
        (this.defs.length ? `<defs>${this.defs.join("")}</defs>` : "") +
        this.body.join("") +
        `</svg>\n`
      );
    }
  }

  // ── image export (SVG / PNG) ─────────────────────────────────────────────────
  let exportOpen = $state(false);
  let exportFormat = $state<"svg" | "png">("svg");
  let exportScope = $state<"view" | "all">("view");
  let exportBg = $state<"theme" | "white" | "none">("white");
  let exportScale = $state(2);
  let exporting = $state(false);
  let exportMsg = $state<string | null>(null);

  // The window to render. "view" is exactly what is on screen; "all" is every
  // contig at the current zoom, over the full track width.
  const exportGeom = $derived.by(() => {
    if (exportScope === "all") {
      return {
        vLeft: 0,
        vTop: 0,
        vW: Math.max(1, Math.ceil(GUTTER + trackW)),
        vH: Math.max(1, Math.ceil(vlayout.total)),
      };
    }
    return { vLeft: viewLeft, vTop: viewTop, vW: Math.round(viewW), vH: Math.round(viewH) };
  });
  const exportPxW = $derived(Math.round(exportGeom.vW * (exportFormat === "png" ? exportScale : 1)));
  const exportPxH = $derived(Math.round(exportGeom.vH * (exportFormat === "png" ? exportScale : 1)));
  // Browsers refuse to allocate a canvas past ~16k on a side (and past a total
  // area cap); SVG has no such limit, so only PNG is blocked.
  const pngTooBig = $derived(
    exportFormat === "png" && (exportPxW > 16384 || exportPxH > 16384 || exportPxW * exportPxH > 1.2e8),
  );
  // Letters only draw for contigs whose sequence has been fetched (they load
  // lazily as you scroll), so a whole-file export at letter zoom may miss some.
  const exportSeqWarning = $derived(
    exportScope === "all" && showLetters && !!data && data.contigs.some((_, i) => seqCache[i] === undefined),
  );

  function exportPalette(): Palette {
    if (exportBg === "theme") return palette();
    const p = palette();
    return { ...p, text: "#111111", dim: "#666666", border: "#cccccc", panel: "#ffffff", bg: "#ffffff" };
  }
  function exportScene(): Scene {
    const g = exportGeom;
    return {
      frames: computeFrames(g.vTop, g.vH, g.vLeft, g.vW),
      vLeft: g.vLeft,
      vTop: g.vTop,
      vW: g.vW,
      vH: g.vH,
      pal: exportPalette(),
      interactive: false,
      opaque: exportBg !== "none",
    };
  }
  function exportBaseName(): string {
    const base = (path.split(/[\\/]/).pop() ?? "contigs").replace(/\.[^.]+$/, "");
    return `${base}_contigs`;
  }

  async function doExport() {
    if (!data || exporting || pngTooBig) return;
    exporting = true;
    exportMsg = null;
    try {
      const g = exportGeom;
      const scene = exportScene();
      if (exportFormat === "svg") {
        const meas = document.createElement("canvas").getContext("2d")!;
        const sctx = new SvgContext(meas);
        renderScene(sctx as unknown as Ctx2D, scene);
        const saved = await saveTextFile(`${exportBaseName()}.svg`, "svg", "SVG image", sctx.toSvg(g.vW, g.vH));
        exportMsg = saved ? `saved ${saved}` : null;
      } else {
        const cv = document.createElement("canvas");
        cv.width = exportPxW;
        cv.height = exportPxH;
        const ctx = cv.getContext("2d");
        if (!ctx) throw new Error("could not allocate a canvas that large");
        ctx.setTransform(exportScale, 0, 0, exportScale, 0, 0);
        renderScene(ctx, scene);
        const url = cv.toDataURL("image/png");
        const saved = await saveBinaryFile(
          `${exportBaseName()}.png`, "png", "PNG image", url.slice(url.indexOf(",") + 1),
        );
        exportMsg = saved ? `saved ${saved}` : null;
      }
      if (exportMsg) {
        exportOpen = false;
        const done = exportMsg;
        setTimeout(() => exportMsg === done && (exportMsg = null), 4000);
      }
    } catch (e) {
      exportMsg = "export failed: " + String(e);
    } finally {
      exporting = false;
    }
  }

  // ── redraw plumbing ───────────────────────────────────────────────────────────
  let rafPending = false;
  function scheduleDraw() {
    if (rafPending) return;
    rafPending = true;
    requestAnimationFrame(() => {
      rafPending = false;
      draw();
    });
  }
  function syncScroll() {
    if (!scrollEl) return;
    viewTop = scrollEl.scrollTop; // vertical is native; horizontal is viewLeft (JS)
    viewW = scrollEl.clientWidth;
    viewH = scrollEl.clientHeight;
    setViewLeft(viewLeft); // re-clamp if the width changed
  }
  function onScroll() {
    syncScroll();
    scheduleDraw();
  }

  // Repaint on any reactive input (frame folds scroll/zoom/filters; add the rest),
  // and lazily fetch sequences for any now-visible contig that needs letters.
  $effect(() => {
    const frames = frame; // depend on layout/scroll/zoom/filters
    void seqCache;
    void spansByContig;
    void selFeatKeys;
    void contigSel;
    void caret;
    void inspected;
    // Appearance options that change only the painting, never the track heights,
    // do not reach us through `frame` — `renderScene` runs inside a rAF callback,
    // so its own reads are not tracked. Without these the ruler would keep
    // showing bp after a switch to kb until something else forced a repaint.
    void rulerUnits;
    void labelFontSize;
    void rulerFontSize;
    void labelPlacement;
    void sharedRuler;
    void featH;
    void stackAnnotations;
    // Batch-fetch the visible window's sequences in one call (with overscan) so
    // scrolling doesn't fire one backend round-trip per contig.
    if (showLetters && frames.length) ensureSeqRange(frames[0].ci, frames[frames.length - 1].ci);
    scheduleDraw();
  });

  $effect(() => {
    if (!scrollEl) return;
    syncScroll();
    if (!fitted && data) {
      fit(); // now that the viewport width is known
      fitted = true;
    }
    const ro = new ResizeObserver(() => {
      syncScroll();
      scheduleDraw();
    });
    ro.observe(scrollEl);
    return () => ro.disconnect();
  });

  // ── editing actions ───────────────────────────────────────────────────────────
  async function refetch() {
    invalidatePath(path);
    layoutCache = new Map();
    codingCache = new Map();
    seqCache = {};
    try {
      data = await cached(`contig:${path}`, () => loadContigView(path));
    } catch (e) {
      error = String(e);
    }
  }
  async function applyColor(hex: string) {
    if (!inspected) return;
    editMsg = null;
    try {
      await setFeatureColor(path, inspected.contigIndex, inspected.feat.feat_index, hex);
      inspected = null;
      await refetch();
    } catch (e) {
      editMsg = String(e);
    }
  }
  async function removeInspected() {
    if (!inspected) return;
    editMsg = null;
    try {
      await removeFeature(path, inspected.contigIndex, inspected.feat.feat_index);
      inspected = null;
      // Feature indices after the removed one shift; a selection holding them
      // would now name other features.
      setItems([]);
      await refetch();
    } catch (e) {
      editMsg = String(e);
    }
  }
  async function submitAdd() {
    editMsg = null;
    try {
      const q: [string, string][] = addForm.note ? [["note", addForm.note]] : [];
      await addFeature(path, addForm.contigIndex, addForm.kind, Number(addForm.start), Number(addForm.end), Number(addForm.strand), q);
      addOpen = false;
      setItems([]); // as for a removal: indices may have moved
      await refetch();
    } catch (e) {
      editMsg = String(e);
    }
  }

  async function load() {
    loading = true;
    error = null;
    fitted = false;
    caret = null;
    inspected = null;
    layoutCache = new Map();
    codingCache = new Map();
    seqCache = {};
    try {
      const d = await cached(`contig:${path}`, () => loadContigView(path));
      data = d;
      enabledKinds = Object.fromEntries(d.feature_kinds.map((k) => [k, true]));
      enabledDbs = Object.fromEntries(d.databases.map((k) => [k, true]));
      hasEvalue = d.evalue_range !== null;
      if (d.evalue_range) maxEvalueExp = Math.ceil(Math.log10(Math.max(d.evalue_range[1], 1e-300)));
      // The selection outlives this view (it is shared with the Enum Report and
      // may have been made before this load); drop anything the file lacks.
      const kept = items.filter((it) => itemRange(it) !== null);
      if (kept.length !== items.length) setItems(kept);
      fit();
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void path;
    load();
  });

  // Take a pending "show this contig" request (SSN node → this contig). An
  // effect rather than part of `load`: the view stays mounted across tab
  // switches, so a request can arrive long after the load. Only the visible
  // view consumes it.
  $effect(() => {
    if (!active || !data) return;
    const requested = ws.focusContig[path];
    if (requested !== undefined) {
      focused = requested;
      delete ws.focusContig[path];
    }
  });

  // When the selection was changed by another view (the Enum Report's row
  // checkboxes) while this one was hidden, show its primary item on the way in.
  let seenRev = 0;
  $effect(() => {
    if (!active || !data || !scrollEl) return;
    const cur = ws.viewSelection[path];
    if (!cur || cur.rev === seenRev) return;
    seenRev = cur.rev;
    if (cur.from === "contig" || !cur.items.length) return;
    caret = null;
    ensureSeq(cur.items[cur.items.length - 1].ci);
    // After the tab is shown (and any first-visit Fit), so the zoom uses the
    // real viewport size.
    requestAnimationFrame(() => fitSelection());
  });

  // Scroll a focused contig into view.
  $effect(() => {
    if (focused === null || !data || !scrollEl) return;
    const idx = focused;
    const off = vlayout.offsets[idx];
    if (off !== undefined) {
      requestAnimationFrame(() => scrollEl && (scrollEl.scrollTop = Math.max(0, off - 40)));
    }
    const timer = setTimeout(() => (focused = null), 2200);
    return () => clearTimeout(timer);
  });

  // Copy the selected bases/residues (use the Protein button for a CDS
  // translation).
  //
  // **Shift is load-bearing.** This used to be a plain Ctrl/Cmd+C, which stopped
  // being viable once the native menu landed: Edit → Copy registers a real OS
  // accelerator for Cmd/Ctrl+C, and menu accelerators are dispatched before the
  // focused widget sees the key (AppKit checks `performKeyEquivalent:` ahead of
  // the responder chain; GTK3 runs `gtk_window_activate_key` ahead of
  // `gtk_window_propagate_key_event`). So the old binding would not have
  // double-copied so much as silently stopped firing on macOS and Linux —
  // `preventDefault` cannot help, because the key never reaches JS at all.
  //
  // The bp selection here is canvas state, not a DOM selection, so the webview's
  // own Copy could never have served it anyway. The visible ⧉ buttons below are
  // the primary affordance; this is the shortcut for them.
  function onKeydown(e: KeyboardEvent) {
    // Hidden views stay mounted (ViewerDock's keep-alive); only the visible one
    // owns the shortcut.
    if (!active) return;
    if ((e.ctrlKey || e.metaKey) && e.shiftKey && e.key.toLowerCase() === "c" && hasSel) {
      // WebView2 binds Ctrl+Shift+C to the DevTools element picker when dev tools
      // are enabled (debug builds); swallow it so the shortcut behaves the same
      // in dev as in release.
      e.preventDefault();
      copySelection(false);
    }
  }
</script>

<svelte:window onkeydown={onKeydown} />

<div class="cv">
  {#if loading}
    <div class="msg">Loading…</div>
  {:else if error}
    <div class="msg err">{error}</div>
  {:else if data}
    <div class="controls">
      <div class="zoom">
        <button onclick={() => zoomBtn(1 / 1.5)} aria-label="Zoom out">−</button>
        <button onclick={fit}>Fit</button>
        <button onclick={() => zoomBtn(1.5)} aria-label="Zoom in">+</button>
        <button onclick={fitSelection} disabled={!selRange || selRange.hi <= selRange.lo} title="Zoom to the last item selected">Fit selection</button>
        <span class="dim">{data.contigs.length} contig{data.contigs.length === 1 ? "" : "s"}</span>
        {#if items.length > 0}
          <span class="sel-badge" title="Ctrl/Cmd+click adds to the selection">{items.length} selected</span>
          {#if hasSel}
            {#if allNucCds}
              <button class="copy" onclick={() => copySelection(false)} title="Copy CDS nucleotides ({COPY_HINT})">⧉ NT</button>
              <button class="copy" onclick={() => copySelection(true)} title="Copy CDS translations">⧉ Protein</button>
            {:else}
              <button class="copy" onclick={() => copySelection(false)} title="Copy the selected sequence{items.length > 1 ? 's as FASTA' : ''} ({COPY_HINT})">
                ⧉ Copy{#if items.length === 1 && selRange}&nbsp;{(selRange.hi - selRange.lo).toLocaleString()} {selUnit}{/if}
              </button>
            {/if}
            <button class="extract" onclick={() => openSelExtract("nt")} title="Extract the selection to a new file, each item as its own contig">
              ⤓ Extract
            </button>
            {#if anyNuc}
              <button class="extract" onclick={() => openSelExtract("aa")} title="Extract the CDS peptides in the selection to a protein GenBank">⤓ Protein</button>
            {/if}
          {/if}
          <button onclick={() => setItems([])}>clear</button>
        {/if}
        {#if copyMsg}<span class="dim">{copyMsg}</span>{/if}
        {#if canEdit}
          <button onclick={() => (addOpen = !addOpen)}>{addOpen ? "Cancel add" : "Add annotation"}</button>
        {/if}
        <div class="popwrap">
          <button onclick={() => { optsOpen = !optsOpen; exportOpen = false; }} title="Appearance options">View ▾</button>
          {#if optsOpen}
            <div class="pop">
              <div class="pop-head">View options<button class="pop-x" onclick={() => (optsOpen = false)} aria-label="Close">×</button></div>
              <label class="orow">
                Lane spacing
                <input type="range" min="12" max="48" step="1" bind:value={laneH} disabled={stackAnnotations} />
                <span class="onum">{lanePitch}px</span>
              </label>
              <label class="orow" title={featH > annotH ? "Held at the label font size" : undefined}>
                Annotation height
                <input type="range" min="4" max="40" step="1" bind:value={annotH} />
                <span class="onum">{featH}px</span>
              </label>
              <label class="orow">
                Contig spacing
                <input type="range" min="0" max="64" step="1" bind:value={contigGap} />
                <span class="onum">{contigGap}px</span>
              </label>
              <label class="orow">
                Ruler font
                <input type="range" min="7" max="18" step="1" bind:value={rulerFontSize} />
                <span class="onum">{rulerFontSize}px</span>
              </label>
              <label class="orow">
                Label font
                <input type="range" min="7" max="18" step="1" bind:value={labelFontSize} />
                <span class="onum">{labelFontSize}px</span>
              </label>
              <label class="orow">
                Ruler units
                <select bind:value={rulerUnits}>
                  <option value="auto">auto</option>
                  <option value="bp">bp</option>
                  <option value="kb">kb</option>
                </select>
              </label>
              <label class="orow">
                Feature labels
                <select bind:value={labelPlacement}>
                  <option value="inside">inside</option>
                  <option value="outside" disabled={stackAnnotations}>outside</option>
                  <option value="both" disabled={stackAnnotations}>both</option>
                </select>
              </label>
              <label class="orow check">
                <input type="checkbox" bind:checked={sharedRuler} />
                Single shared ruler
              </label>
              <label class="orow check">
                <input type="checkbox" bind:checked={stackAnnotations} />
                Stack annotations
              </label>
              <div class="pop-foot"><button onclick={resetViewOptions}>Reset</button></div>
            </div>
          {/if}
        </div>
        <div class="popwrap">
          <button onclick={() => { exportOpen = !exportOpen; optsOpen = false; }} title="Export the view as an image">Export ▾</button>
          {#if exportOpen}
            <div class="pop">
              <div class="pop-head">Export image<button class="pop-x" onclick={() => (exportOpen = false)} aria-label="Close">×</button></div>
              <label class="orow">
                Format
                <select bind:value={exportFormat}>
                  <option value="svg">SVG (vector)</option>
                  <option value="png">PNG (raster)</option>
                </select>
              </label>
              <label class="orow">
                Region
                <select bind:value={exportScope}>
                  <option value="view">Current view</option>
                  <option value="all">All contigs, full width</option>
                </select>
              </label>
              <label class="orow">
                Background
                <select bind:value={exportBg}>
                  <option value="white">White</option>
                  <option value="theme">Match theme</option>
                  <option value="none">Transparent</option>
                </select>
              </label>
              {#if exportFormat === "png"}
                <label class="orow">
                  Scale
                  <select bind:value={exportScale}>
                    <option value={1}>1×</option>
                    <option value={2}>2×</option>
                    <option value={3}>3×</option>
                    <option value={4}>4×</option>
                  </select>
                </label>
              {/if}
              <div class="ohint">{exportPxW.toLocaleString()} × {exportPxH.toLocaleString()} px</div>
              {#if pngTooBig}
                <div class="ohint warn">Too large for a PNG — lower the scale, zoom out, or export SVG.</div>
              {/if}
              {#if exportSeqWarning}
                <div class="ohint warn">Sequence letters render only for contigs already loaded; scroll through the file first, or zoom out.</div>
              {/if}
              <div class="pop-foot">
                <button class="apply" onclick={doExport} disabled={exporting || pngTooBig}>
                  {exporting ? "Exporting…" : "Save…"}
                </button>
              </div>
            </div>
          {/if}
        </div>
        {#if exportMsg}<span class="dim">{exportMsg}</span>{/if}
      </div>

      <div class="coords">
        {#if selCoords && selRange && selRange.hi > selRange.lo}
          <span class="coord"><b>sel</b> {selCoords} <span class="dim">({(selRange!.hi - selRange!.lo).toLocaleString()} {selUnit})</span></span>
        {:else if caretLabel}
          <span class="coord"><b>caret</b> {caretLabel}</span>
        {/if}
        {#if cursorLabel}
          <span class="coord"><b>pos</b> {cursorLabel}</span>
        {/if}
      </div>

      <div class="filters">
        {#each data.feature_kinds as kind (kind)}
          <label class="chip"><input type="checkbox" bind:checked={enabledKinds[kind]} /> {kind}</label>
        {/each}
      </div>

      {#if data.databases.length}
        <div class="filters">
          <span class="dim">Databases:</span>
          {#each data.databases as db (db)}
            <label class="chip"><input type="checkbox" bind:checked={enabledDbs[db]} /> {db}</label>
          {/each}
        </div>
      {/if}

      {#if hasEvalue && data.evalue_range}
        <div class="evalue">
          <span class="dim">max e-value 1e{maxEvalueExp}</span>
          <input
            type="range"
            min={Math.floor(Math.log10(Math.max(data.evalue_range[0], 1e-300)))}
            max={Math.ceil(Math.log10(Math.max(data.evalue_range[1], 1e-300)))}
            step="1"
            bind:value={maxEvalueExp}
          />
        </div>
      {/if}
    </div>

    {#if addOpen}
      <div class="editrow">
        <span class="dim">Add:</span>
        <select bind:value={addForm.contigIndex}>
          {#each data.contigs as c (c.index)}<option value={c.index}>{c.id || c.name}</option>{/each}
        </select>
        <input class="k" type="text" bind:value={addForm.kind} placeholder="kind" />
        <input class="n" type="number" bind:value={addForm.start} placeholder="start" />
        <input class="n" type="number" bind:value={addForm.end} placeholder="end" />
        <select bind:value={addForm.strand}>
          <option value={1}>+</option><option value={-1}>−</option><option value={0}>.</option>
        </select>
        <input class="note" type="text" bind:value={addForm.note} placeholder="note (optional)" />
        <button class="apply" onclick={submitAdd}>Add</button>
      </div>
    {/if}
    {#if editMsg}<div class="editmsg err">{editMsg}</div>{/if}

    <div
      class="scroll"
      bind:this={scrollEl}
      onscroll={onScroll}
      onwheel={onWheel}
      onpointerdown={onPointerDown}
      oncontextmenu={onContextMenu}
      onpointermove={(e) => { const r = scrollEl?.getBoundingClientRect(); if (r) lastCursorX = e.clientX - r.left; onPointerMove(e); }}
      onpointerup={onPointerUp}
      onpointerleave={onPointerLeave}
      role="application"
      aria-label="Contig annotation viewer"
    >
      <button type="button" class="gutter-resize" style="left:{GUTTER - 3}px; top:{viewTop}px; height:{viewH}px;" aria-label="Resize contig names" onpointerdown={resizeGutter} onkeydown={(e) => { if (e.key === "ArrowLeft" || e.key === "ArrowRight") { GUTTER = Math.max(80, Math.min(viewW - 80, GUTTER + (e.key === "ArrowRight" ? 10 : -10))); scheduleDraw(); } }}></button>
      <!-- Spacer is only as tall as the stacked contigs (vertical native scroll);
           the x axis is JS-managed (viewLeft) so it isn't bound by DOM width limits. -->
      <div class="spacer" style="height:{vlayout.total}px;">
        <canvas id="contig-canvas" bind:this={canvas}></canvas>
      </div>
    </div>
    <!-- Custom horizontal scrollbar (the track can be far wider than any DOM element). -->
    <div
      class="hbar"
      role="scrollbar"
      aria-controls="contig-canvas"
      aria-orientation="horizontal"
      aria-valuenow={Math.round(viewLeft)}
      tabindex="-1"
      onpointerdown={onHbarDown}
    >
      <div
        class="hthumb"
        style="left:{maxViewLeft > 0 ? (viewLeft / trackW) * 100 : 0}%; width:{Math.min(100, (trackViewW / trackW) * 100)}%;"
      ></div>
    </div>
  {/if}

  {#if inspected}
    <div class="editpanel">
      <div class="ep-head">
        <b>{inspected.feat.kind}</b> · {inspected.feat.label}
        <button class="ep-x" onclick={() => (inspected = null)} aria-label="Close">×</button>
      </div>
      <!-- The same fields the hover tooltip shows, pinned: clicking a feature to
           recolor it must not cost you the annotation you were reading. -->
      <div class="ep-detail">
        <div class="tt-row"><b>contig</b> {data?.contigs[inspected.contigIndex].id || data?.contigs[inspected.contigIndex].name}</div>
        <div class="tt-row"><b>location</b> {(inspected.feat.start + 1).toLocaleString()}:{inspected.feat.end.toLocaleString()} ({(inspected.feat.end - inspected.feat.start).toLocaleString()} bp)</div>
        <div class="tt-row"><b>strand</b> {inspected.feat.strand === -1 ? "−" : inspected.feat.strand === 1 ? "+" : "."}</div>
        {#if inspected.feat.parts.length > 1}
          <div class="tt-row"><b>parts</b> {inspected.feat.parts.map((p) => `${p.start + 1}:${p.end}`).join(", ")}</div>
        {/if}
        {#if inspected.feat.database !== null}<div class="tt-row"><b>database</b> {inspected.feat.database}</div>{/if}
        {#each inspected.feat.qualifiers as [k, v] (k)}
          <div class="tt-row"><b>{k}</b> {v}</div>
        {/each}
      </div>
      <div class="ep-body">
        {#if canEdit}
          <label>Color <input type="color" value={inspected.feat.color} onchange={(e) => applyColor(e.currentTarget.value)} /></label>
          <button class="ep-remove" onclick={removeInspected}>Remove</button>
        {:else}
          <span class="dim">Read-only (GenBank files can be edited)</span>
        {/if}
      </div>
    </div>
  {/if}

  {#if tooltip}
    <div class="tooltip" style="left: {tooltip.x}px; top: {tooltip.y}px;">
      <div class="tt-title">{tooltip.feature.kind}</div>
      <div class="tt-row"><b>start</b> {tooltip.feature.start + 1}</div>
      <div class="tt-row"><b>end</b> {tooltip.feature.end}</div>
      <div class="tt-row"><b>strand</b> {tooltip.feature.strand === -1 ? "−" : tooltip.feature.strand === 1 ? "+" : "."}</div>
      {#each tooltip.feature.qualifiers as [k, v] (k)}
        <div class="tt-row"><b>{k}</b> {v}</div>
      {/each}
    </div>
  {/if}
  {#if nameTooltip}
    <div class="tooltip" style="left: {nameTooltip.x}px; top: {nameTooltip.y}px;">{nameTooltip.name}</div>
  {/if}

  {#if showSelExtract}
    <NameModal
      title="Extract {items.length === 1 ? 'selection' : `${items.length} items`} ({selExtractKind === 'aa' ? 'CDS peptides → protein GenBank' : 'one contig each → GenBank'})"
      defaultName={selExtractDefault}
      submitLabel="Extract"
      validate={validateFilename}
      onsubmit={(name) => doSelExtract(name)}
      oncancel={() => (showSelExtract = false)}
    />
  {/if}
</div>

<style>
  .cv {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    position: relative;
  }
  .controls {
    display: flex;
    flex-wrap: wrap;
    gap: 8px 16px;
    align-items: center;
    padding: 6px 12px;
    border-bottom: 1px solid var(--border);
    background: var(--bg-panel);
  }
  .zoom {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .copy,
  .extract {
    background: var(--accent);
    color: #fff;
    border-color: var(--accent);
  }
  .sel-badge {
    color: var(--accent);
    font-size: 12px;
  }
  .filters {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-size: 12px;
    padding: 1px 6px;
    border: 1px solid var(--border);
    border-radius: 10px;
    background: var(--bg-elevated);
    user-select: none;
  }
  .chip input {
    margin: 0;
  }
  .evalue {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .dim {
    color: var(--text-dim);
    font-size: 12px;
  }
  .editrow {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    padding: 6px 12px;
    border-bottom: 1px solid var(--border);
    background: var(--bg-elevated);
  }
  .editrow input,
  .editrow select {
    padding: 3px 5px;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg);
    color: var(--text);
    font-size: 12px;
  }
  .editrow .k { width: 110px; }
  .editrow .n { width: 72px; }
  .editrow .note { width: 160px; }
  .apply {
    background: var(--accent);
    color: #fff;
    border-color: var(--accent);
  }
  .editmsg {
    padding: 4px 12px;
    font-size: 12px;
  }
  .editmsg.err {
    color: #e5484d;
  }
  .scroll {
    flex: 1;
    overflow-x: hidden;
    overflow-y: auto;
    position: relative;
    min-height: 0;
  }
  .gutter-resize {
    position: absolute;
    width: 6px;
    z-index: 3;
    cursor: col-resize;
    touch-action: none;
    border: 0;
    padding: 0;
    background: transparent;
  }
  .gutter-resize:hover { background: var(--accent); opacity: 0.5; }
  .spacer {
    position: relative;
    width: 100%;
  }
  canvas {
    position: absolute;
    top: 0;
    left: 0;
    display: block;
  }
  .hbar {
    flex: none;
    height: 12px;
    position: relative;
    background: var(--bg-panel);
    border-top: 1px solid var(--border);
    cursor: pointer;
  }
  .hthumb {
    position: absolute;
    top: 2px;
    height: 8px;
    min-width: 24px;
    border-radius: 4px;
    background: var(--border);
  }
  .hthumb:hover {
    background: var(--text-dim);
  }
  .editpanel {
    position: absolute;
    right: 16px;
    bottom: 16px;
    z-index: 40;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: 8px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.3);
    padding: 8px 10px;
    min-width: 200px;
    max-width: 440px;
  }
  .ep-detail {
    max-height: 40vh;
    overflow: auto;
    font-size: 12px;
    margin-bottom: 8px;
    padding-bottom: 6px;
    border-bottom: 1px solid var(--border);
  }
  .coords {
    display: flex;
    align-items: center;
    gap: 12px;
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }
  .coord b {
    color: var(--text-dim);
    font-weight: 600;
    margin-right: 4px;
  }
  .popwrap {
    position: relative;
  }
  .pop {
    position: absolute;
    top: 100%;
    left: 0;
    margin-top: 4px;
    z-index: 30;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: 6px;
    box-shadow: 0 6px 20px rgba(0, 0, 0, 0.3);
    padding: 8px 10px;
    min-width: 250px;
  }
  .pop-head {
    display: flex;
    align-items: center;
    font-weight: 600;
    font-size: 12.5px;
    margin-bottom: 6px;
  }
  .pop-x {
    margin-left: auto;
    border: none;
    background: none;
    color: var(--text-dim);
    font-size: 15px;
    padding: 0 3px;
  }
  .orow {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
    margin: 4px 0;
  }
  .orow input[type="range"] {
    flex: 1;
    min-width: 80px;
  }
  .orow select {
    margin-left: auto;
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 2px 4px;
    font-size: 12px;
  }
  .orow.check {
    gap: 6px;
  }
  .onum {
    width: 40px;
    text-align: right;
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .ohint {
    font-size: 11.5px;
    color: var(--text-dim);
    margin-top: 6px;
  }
  .ohint.warn {
    color: #d08700;
  }
  .pop-foot {
    display: flex;
    justify-content: flex-end;
    margin-top: 8px;
  }
  .ep-head {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 6px;
    font-size: 12.5px;
  }
  .ep-x {
    margin-left: auto;
    border: none;
    background: none;
    color: var(--text-dim);
    font-size: 15px;
    padding: 0 3px;
  }
  .ep-body {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .ep-body label {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: 12px;
  }
  .ep-remove {
    color: #e5484d;
    border-color: #e5484d55;
  }
  .tooltip {
    position: fixed;
    z-index: 50;
    max-width: 420px;
    max-height: 60vh;
    overflow: auto;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 8px 10px;
    font-size: 12px;
    box-shadow: 0 6px 20px rgba(0, 0, 0, 0.25);
    pointer-events: none;
  }
  .tt-title {
    font-weight: 700;
    color: #aa2222;
    margin-bottom: 3px;
  }
  .tt-row {
    line-height: 1.4;
    word-break: break-word;
  }
  .tt-row b {
    color: var(--text-dim);
    margin-right: 4px;
  }
  .msg {
    padding: 24px;
    color: var(--text-dim);
  }
  .msg.err {
    color: #e5484d;
  }
</style>
