<script lang="ts">
  import { addFeature, extractContigs, extractRegion, getContigSequences, loadContigView, removeFeature, setFeatureColor } from "../api";
  import { cached, invalidatePath } from "../cache";
  import { ws } from "../state.svelte";
  import { validateFilename } from "../filetypes";
  import NameModal from "../NameModal.svelte";
  import type { ContigFileView, ContigView as Contig, FeatureView } from "../types";

  let { path }: { path: string } = $props();

  // ── data / load ──────────────────────────────────────────────────────────
  let data = $state<ContigFileView | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(true);
  let focused = $state<number | null>(null);
  let fitted = false; // one-time initial Fit once the viewport width is known

  // ── editing (GenBank only) ─────────────────────────────────────────────────
  const canEdit = $derived(/\.(gb|gbk|genbank|gbff)$/i.test(path));
  let selected = $state<{ contigIndex: number; feat: FeatureView } | null>(null);
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

  // ── geometry constants ─────────────────────────────────────────────────────
  const GUTTER = 134; // fixed left label column (pinned during horizontal scroll)
  const RULER_H = 16;
  const SEQ_H = 16; // nucleotide/residue letter track
  const SEQBAR_H = 7; // collapsed sequence bar (zoomed out)
  const TRANS_H = 16; // translation track (nucleotide contigs)
  const LANE_H = 20;
  const FEAT_H = 14;
  const CONTIG_GAP = 16;
  const LETTERS_MIN = 4.5; // px/bp at/above which residue letters render (~1 zoom step earlier)
  const CONTIG_PAD_TOP = 4;

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
  type Layout = { placed: Placed[]; maxLane: number };

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
    return { placed, maxLane };
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
  function contigHeight(c: Contig, lay: Layout): number {
    return (
      CONTIG_PAD_TOP + RULER_H + seqTrackH() + transTrackH(c) + (lay.maxLane + 1) * LANE_H + CONTIG_GAP
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
    const contigs = data.contigs;
    const offsets: number[] = new Array(contigs.length + 1);
    offsets[0] = 0;
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
    rulerY: number;
    seqY: number;
    transY: number; // -1 if no translation track
    laneBaseY: number; // y of lane 0's top
    height: number;
    winL: number;
    winR: number;
  };
  const frame = $derived.by((): Frame[] => {
    if (!data) return [];
    void viewLeft;
    void viewTop;
    void viewW;
    void viewH;
    const { offsets } = vlayout;
    const contigs = data.contigs;
    // binary search first contig whose bottom > viewTop
    let lo = 0,
      hi = contigs.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (offsets[mid + 1] <= viewTop) lo = mid + 1;
      else hi = mid;
    }
    const out: Frame[] = [];
    for (let ci = lo; ci < contigs.length && offsets[ci] < viewTop + viewH; ci++) {
      const c = contigs[ci];
      const lay = contigLayout(c);
      const yTop = offsets[ci] + CONTIG_PAD_TOP;
      const rulerY = yTop;
      const seqY = rulerY + RULER_H;
      const th = transTrackH(c);
      const transY = th > 0 ? seqY + seqTrackH() : -1;
      const laneBaseY = seqY + seqTrackH() + th;
      const winL = Math.max(0, viewLeft / pxPerBp);
      const winR = Math.min(c.length, (viewLeft + viewW - GUTTER) / pxPerBp);
      out.push({
        ci, c, lay, yTop, rulerY, seqY, transY, laneBaseY,
        height: offsets[ci + 1] - offsets[ci],
        winL, winR,
      });
    }
    return out;
  });

  // ── genetic code + colours ──────────────────────────────────────────────────
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
  // A bp range on one contig, drag-selected over the sequence/track area.
  let sel = $state<{ ci: number; a: number; b: number } | null>(null);
  // The feature a selection came from (if it was a feature click) — enables the
  // "copy nucleotides vs. translation" choice for a CDS on a nucleotide contig.
  let selFeat = $state<{ ci: number; feat: FeatureView } | null>(null);
  let dragging = false;
  let copyMsg = $state<string | null>(null);
  // Shown in the copy buttons' tooltips so the shortcut is discoverable rather
  // than folklore. Cmd on macOS, Ctrl elsewhere.
  const COPY_HINT =
    typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform ?? "")
      ? "\u2318\u21E7C"
      : "Ctrl+Shift+C";

  // Multi-contig selection (by contig index) for Extract → new sequence file.
  let contigSel = $state(new Set<number>());
  let contigAnchor = -1; // last plainly-clicked contig (shift-range base)
  let showExtract = $state(false);

  // Throws on failure so the modal shows it inline and stays open; closes on success.
  async function doExtractContigs(name: string) {
    const dir = ws.currentDir;
    if (!dir || contigSel.size === 0) return;
    const p = await extractContigs(dir, name, path, [...contigSel].sort((a, b) => a - b));
    await ws.refreshAndOpen(p);
    contigSel = new Set();
    showExtract = false;
  }

  const selRange = $derived(sel ? { ci: sel.ci, lo: Math.min(sel.a, sel.b), hi: Math.max(sel.a, sel.b) } : null);
  const hasSel = $derived(!!selRange && selRange.hi > selRange.lo);
  // A CDS on a nucleotide contig is selected → offer NT + translation copy.
  const cdsSelected = $derived(
    !!selFeat && selFeat.feat.kind === "CDS" && !!data && !data.contigs[selFeat.ci].is_protein,
  );

  async function writeClip(text: string, msg: string) {
    try {
      await navigator.clipboard.writeText(text);
      copyMsg = msg;
    } catch (e) {
      copyMsg = "copy failed: " + String(e);
    }
    setTimeout(() => (copyMsg = null), 2500);
  }

  async function copyNucleotides() {
    if (!selRange) return;
    const seq = seqCache[selRange.ci];
    if (seq === undefined) {
      ensureSeq(selRange.ci);
      copyMsg = "loading sequence…";
      return;
    }
    const sub = seq.slice(selRange.lo, selRange.hi);
    const unit = data?.contigs[selRange.ci].is_protein ? "aa" : "bp";
    await writeClip(sub, `copied ${sub.length.toLocaleString()} ${unit}`);
  }

  async function copyTranslation() {
    if (!selFeat) return;
    const seq = seqCache[selFeat.ci];
    if (seq === undefined) {
      ensureSeq(selFeat.ci);
      copyMsg = "loading sequence…";
      return;
    }
    const cm = codingMap(selFeat.ci, selFeat.feat, seq);
    if (!cm) return;
    await writeClip(cm.pep, `copied ${cm.pep.length.toLocaleString()} aa (translation)`);
  }

  // ── extract the current selection to a new FASTA file ─────────────────────────
  let showSelExtract = $state(false);
  let selExtractKind = $state<"nt" | "aa">("nt");
  const selIsNuc = $derived(!!selRange && !!data && !data.contigs[selRange.ci].is_protein);
  const selExtractDefault = $derived.by(() => {
    if (!selRange || !data) return "selection.gb";
    const c = data.contigs[selRange.ci];
    const id = (c.id || c.name || "seq").replace(/[^\w.-]+/g, "_");
    // Both are GenBank: NT keeps the region's features; peptide runs the region's
    // CDSs through extract_peptides → protein GenBank.
    const suffix = selExtractKind === "aa" ? "_peptides" : "";
    return `${id}_${selRange.lo + 1}_${selRange.hi}${suffix}.gb`;
  });

  function openSelExtract(kind: "nt" | "aa") {
    selExtractKind = kind;
    showSelExtract = true;
  }
  // Throws on failure so the modal shows it inline; closes on success. Both the
  // nucleotide slice and the peptide conversion happen server-side (the peptide
  // path: extract region → extract_peptides → protein GenBank).
  async function doSelExtract(name: string) {
    const r = selRange;
    const dir = ws.currentDir;
    if (!r || !data || !dir) return;
    const p = await extractRegion(dir, name, path, r.ci, r.lo, r.hi, selExtractKind === "aa");
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
    const laneFromTop = Math.floor(laneRegion / LANE_H);
    const lane = fr.lay.maxLane - laneFromTop; // lane 0 drawn at bottom
    const bp = bpAt(clientX);
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

  function onPointerDown(e: PointerEvent) {
    if (!scrollEl) return;
    const rect0 = scrollEl.getBoundingClientRect();
    // Click in the left margin (name gutter): select the whole sequence (copy) and
    // manage the multi-contig set for Extract (Ctrl/Cmd toggles, Shift ranges).
    if (e.clientX - rect0.left < GUTTER) {
      const fr = frameAtY(viewTop + (e.clientY - rect0.top));
      if (fr) {
        const ci = fr.ci;
        if (e.shiftKey && contigAnchor >= 0) {
          const [lo, hi] = contigAnchor < ci ? [contigAnchor, ci] : [ci, contigAnchor];
          const s = new Set(contigSel);
          for (let i = lo; i <= hi; i++) s.add(i);
          contigSel = s;
        } else if (e.ctrlKey || e.metaKey) {
          const s = new Set(contigSel);
          s.has(ci) ? s.delete(ci) : s.add(ci);
          contigSel = s;
          contigAnchor = ci;
        } else {
          contigSel = new Set([ci]);
          contigAnchor = ci;
        }
        sel = { ci, a: 0, b: fr.c.length }; // whole-sequence copy selection
        selFeat = null;
        ensureSeq(ci);
        scheduleDraw();
      }
      return;
    }
    // Shift-click in the track extends the current selection's focus border to the
    // clicked bp (on the same contig) — the way to select a span wider than the
    // viewport: click one end, scroll, shift-click the other.
    if (e.shiftKey && sel) {
      const fr = frameAtY(viewTop + (e.clientY - rect0.top));
      if (fr && fr.ci === sel.ci) {
        sel = { ci: sel.ci, a: sel.a, b: bpClamp(e.clientX, fr.c.length) };
        selFeat = null;
        ensureSeq(sel.ci);
        dragging = true;
        dragClientX = e.clientX;
        scrollEl.setPointerCapture?.(e.pointerId);
        scheduleDraw();
        return;
      }
    }
    const hit = featureAt(e.clientX, e.clientY);
    if (hit) {
      // Select the feature's underlying sequence span (so it highlights + can be
      // copied), and — for editable files — open the edit panel.
      sel = { ci: hit.ci, a: hit.f.start, b: hit.f.end };
      selFeat = { ci: hit.ci, feat: hit.f };
      ensureSeq(hit.ci); // load sequence so Copy is ready
      if (canEdit) selected = { contigIndex: hit.ci, feat: hit.f };
      scheduleDraw();
      return;
    }
    // Plain click sets a caret (anchor) at this bp; a drag extends the focus. The
    // caret persists (no clear on mouse-up) so a later shift-click can extend it.
    const fr = frameAtY(viewTop + (e.clientY - rect0.top));
    if (!fr) return;
    const bp = bpClamp(e.clientX, fr.c.length);
    sel = { ci: fr.ci, a: bp, b: bp };
    selFeat = null;
    ensureSeq(fr.ci);
    dragging = true;
    dragClientX = e.clientX;
    scrollEl.setPointerCapture?.(e.pointerId);
    scheduleDraw();
  }
  function onPointerMove(e: PointerEvent) {
    if (dragging && sel && scrollEl) {
      dragClientX = e.clientX;
      const c = data?.contigs[sel.ci];
      sel = { ...sel, b: bpClamp(e.clientX, c?.length ?? 0) };
      maybeAutoScroll(); // slide the viewport if dragging past an edge
      scheduleDraw();
      return;
    }
    const hit = featureAt(e.clientX, e.clientY);
    tooltip = hit ? { x: e.clientX + 12, y: e.clientY + 12, feature: hit.f } : null;
  }
  function onPointerUp() {
    dragging = false;
    stopAutoScroll();
    scheduleDraw();
  }
  function onPointerLeave() {
    tooltip = null;
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
    if (v === 0 || !dragging || !sel) {
      autoRaf = 0;
      return;
    }
    setViewLeft(viewLeft + v);
    const c = data?.contigs[sel.ci];
    sel = { ...sel, b: bpClamp(dragClientX, c?.length ?? 0) };
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
  let lastCursorX = GUTTER + 200; // screen px within the scroll box (for +/- buttons)
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
  function ticks(length: number): number[] {
    const spanBp = Math.min(length, (viewW - GUTTER) / pxPerBp);
    const n = Math.max(3, Math.floor((viewW - GUTTER) / 110));
    const raw = spanBp / n;
    const mag = Math.pow(10, Math.floor(Math.log10(Math.max(1, raw))));
    const step = Math.max(1, Math.round(raw / mag) * mag);
    const first = Math.floor(Math.max(0, viewLeft / pxPerBp) / step) * step;
    const out: number[] = [];
    for (let x = first; x <= Math.min(length, (viewLeft + viewW - GUTTER) / pxPerBp); x += step) out.push(x);
    return out;
  }

  // ── canvas draw ───────────────────────────────────────────────────────────────
  function drawArrow(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, mode: "box" | "right" | "left") {
    const head = Math.min(8, Math.max(0, w));
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

    const css = getComputedStyle(scrollEl);
    const cText = css.getPropertyValue("--text").trim() || "#111";
    const cDim = css.getPropertyValue("--text-dim").trim() || "#888";
    const cBorder = css.getPropertyValue("--border").trim() || "#ccc";
    const cAccent = css.getPropertyValue("--accent").trim() || "#3b82f6";
    const cPanel = css.getPropertyValue("--bg-panel").trim() || "#fff";
    const cSeqBar = cDim;

    // Font tracks cell width (≈0.6em glyph advance) so letters fit even at the
    // lower zoom threshold without overlapping.
    const letterFont = `${Math.min(13, Math.max(6, pxPerBp * 1.15))}px ui-monospace, monospace`;

    // Clip tracks to the right of the gutter.
    ctx.save();
    ctx.beginPath();
    ctx.rect(GUTTER, 0, viewW - GUTTER, viewH);
    ctx.clip();

    for (const fr of frame) {
      const sYtop = fr.rulerY - viewTop;
      const selH = fr.height - CONTIG_GAP;
      // selection highlight (range) or caret (collapsed) for this contig
      if (selRange && selRange.ci === fr.ci && selRange.hi > selRange.lo) {
        const x1 = screenX(selRange.lo);
        const x2 = screenX(selRange.hi);
        ctx.fillStyle = "rgba(59,130,246,0.18)";
        ctx.fillRect(x1, sYtop, Math.max(1, x2 - x1), selH);
        // focus-border caret (the edge a shift-click / drag moves)
        const fx = screenX(sel!.b);
        ctx.fillStyle = cAccent;
        ctx.fillRect(fx - 0.75, sYtop, 1.5, selH);
      } else if (sel && sel.ci === fr.ci) {
        const cx = screenX(sel.a);
        if (cx >= GUTTER - 1 && cx <= viewW) {
          ctx.fillStyle = cAccent;
          ctx.fillRect(cx - 0.75, sYtop, 1.5, selH);
        }
      }

      // ruler
      const ry = fr.rulerY - viewTop + RULER_H - 1;
      ctx.strokeStyle = cBorder;
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(Math.max(GUTTER, screenX(0)), ry + 0.5);
      ctx.lineTo(screenX(fr.c.length), ry + 0.5);
      ctx.stroke();
      ctx.fillStyle = cDim;
      ctx.font = "10px ui-monospace, monospace";
      ctx.textAlign = "left";
      ctx.textBaseline = "alphabetic";
      for (const t of ticks(fr.c.length)) {
        const tx = screenX(t);
        ctx.beginPath();
        ctx.moveTo(tx + 0.5, ry - 4);
        ctx.lineTo(tx + 0.5, ry);
        ctx.stroke();
        ctx.fillText(t.toLocaleString(), tx + 2, ry - 6);
      }

      // sequence track
      const seq = seqCache[fr.ci];
      const seqTop = fr.seqY - viewTop;
      if (showLetters && seq !== undefined) {
        ctx.font = letterFont;
        ctx.textAlign = "center";
        ctx.textBaseline = "middle";
        const lo = Math.max(0, Math.floor(fr.winL));
        const hi = Math.min(fr.c.length, Math.ceil(fr.winR));
        const isProt = fr.c.is_protein;
        for (let bp = lo; bp < hi; bp++) {
          const ch = seq[bp] ?? "N";
          const cx = screenX(bp) + pxPerBp / 2;
          if (isProt) {
            ctx.fillStyle = aaBg(ch);
            ctx.fillRect(screenX(bp), seqTop, pxPerBp, SEQ_H);
            ctx.fillStyle = "#111";
          } else {
            ctx.fillStyle = NT_COLOR[ch] ?? "#777";
          }
          ctx.fillText(ch, cx, seqTop + SEQ_H / 2 + 0.5);
        }
      } else {
        // collapsed sequence bar
        ctx.fillStyle = cSeqBar;
        const x1 = Math.max(GUTTER, screenX(0));
        const x2 = screenX(fr.c.length);
        ctx.globalAlpha = 0.35;
        ctx.fillRect(x1, seqTop + 1, Math.max(1, x2 - x1), SEQBAR_H - 2);
        ctx.globalAlpha = 1;
      }

      // translation track (nucleotide contigs, CDS codons)
      if (fr.transY >= 0 && seq !== undefined) {
        const ty = fr.transY - viewTop;
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
            const cx = screenX((lo + hi) / 2);
            const w = (hi - lo) * pxPerBp;
            ctx.fillStyle = aaBg(aa);
            ctx.fillRect(screenX(lo), ty, w, TRANS_H);
            ctx.fillStyle = aa === "*" ? "#fff" : "#111";
            if (w >= 5) ctx.fillText(aa, cx, ty + TRANS_H / 2 + 0.5);
          }
        }
      }

      // feature lanes
      ctx.textAlign = "left";
      ctx.textBaseline = "middle";
      ctx.font = "10px system-ui, sans-serif";
      for (const p of fr.lay.placed) {
        const f = p.f;
        if (f.end < fr.winL || f.start > fr.winR) continue;
        const laneTop = fr.laneBaseY - viewTop + (fr.lay.maxLane - p.lane) * LANE_H + (LANE_H - FEAT_H) / 2;
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
            ctx.moveTo(screenX(sp[i].end), laneTop + FEAT_H / 2);
            ctx.lineTo(screenX(sp[i + 1].start), laneTop + FEAT_H / 2);
            ctx.stroke();
          }
        }
        const isSel = selected?.contigIndex === fr.ci && selected?.feat.feat_index === f.feat_index;
        for (let i = 0; i < sp.length; i++) {
          const x = screenX(sp[i].start);
          const w = Math.max(1.5, (sp[i].end - sp[i].start) * pxPerBp);
          const mode = i === term ? (f.strand === 1 ? "right" : "left") : "box";
          drawArrow(ctx, x, laneTop, w, FEAT_H, mode);
          ctx.fillStyle = f.color;
          ctx.fill();
          ctx.lineWidth = isSel ? 2 : 0.5;
          ctx.strokeStyle = isSel ? cAccent : "rgba(0,0,0,0.35)";
          ctx.stroke();
        }
        // label if it fits
        const fw = (f.end - f.start) * pxPerBp;
        if (fw > 34) {
          const lx = Math.max(GUTTER + 2, screenX(f.start) + 4);
          ctx.fillStyle = labelColor(f.color);
          ctx.save();
          ctx.beginPath();
          ctx.rect(screenX(f.start), laneTop, fw, FEAT_H);
          ctx.clip();
          ctx.fillText(f.label, lx, laneTop + FEAT_H / 2 + 0.5);
          ctx.restore();
        }
      }
    }
    ctx.restore();

    // ── gutter (pinned left column) ──
    ctx.fillStyle = cPanel;
    ctx.fillRect(0, 0, GUTTER, viewH);
    ctx.strokeStyle = cBorder;
    ctx.beginPath();
    ctx.moveTo(GUTTER + 0.5, 0);
    ctx.lineTo(GUTTER + 0.5, viewH);
    ctx.stroke();
    ctx.textAlign = "left";
    ctx.textBaseline = "alphabetic";
    for (const fr of frame) {
      // Multi-select highlight for the whole contig row in the gutter.
      if (contigSel.has(fr.ci)) {
        ctx.fillStyle = "rgba(59,130,246,0.18)";
        ctx.fillRect(0, fr.yTop - CONTIG_PAD_TOP - viewTop, GUTTER, fr.height - CONTIG_GAP);
      }
      const yid = fr.seqY - viewTop + seqTrackH() / 2 + 3;
      ctx.fillStyle = cText;
      ctx.font = "600 11px ui-monospace, monospace";
      ctx.fillText(clip(`${fr.ci + 1}. ${fr.c.id || fr.c.name}`, GUTTER - 8, ctx), 6, yid);
      if (fr.transY >= 0) {
        ctx.fillStyle = cDim;
        ctx.font = "10px system-ui, sans-serif";
        ctx.fillText("Translation", 6, fr.transY - viewTop + TRANS_H / 2 + 4);
      }
    }
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
    void selRange;
    void selected;
    void contigSel;
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
    if (!selected) return;
    editMsg = null;
    try {
      await setFeatureColor(path, selected.contigIndex, selected.feat.feat_index, hex);
      selected = null;
      await refetch();
    } catch (e) {
      editMsg = String(e);
    }
  }
  async function removeSelected() {
    if (!selected) return;
    editMsg = null;
    try {
      await removeFeature(path, selected.contigIndex, selected.feat.feat_index);
      selected = null;
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
      await refetch();
    } catch (e) {
      editMsg = String(e);
    }
  }

  async function load() {
    loading = true;
    error = null;
    fitted = false;
    sel = null;
    selFeat = null;
    contigSel = new Set();
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
      fit();
      if (ws.focusContig[path] !== undefined) {
        focused = ws.focusContig[path];
        delete ws.focusContig[path];
      }
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

  // Scroll a focused contig into view (SSN node → this contig).
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
    if ((e.ctrlKey || e.metaKey) && e.shiftKey && e.key.toLowerCase() === "c" && hasSel) {
      // WebView2 binds Ctrl+Shift+C to the DevTools element picker when dev tools
      // are enabled (debug builds); swallow it so the shortcut behaves the same
      // in dev as in release.
      e.preventDefault();
      copyNucleotides();
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
        <button onclick={fitSelection} disabled={!hasSel} title="Zoom to the current selection">Fit selection</button>
        <span class="dim">{data.contigs.length} contig{data.contigs.length === 1 ? "" : "s"}</span>
        {#if contigSel.size > 0}
          <span class="sel-badge">{contigSel.size} selected</span>
          <button class="extract" onclick={() => (showExtract = true)}>Extract →</button>
          <button onclick={() => (contigSel = new Set())}>clear</button>
        {/if}
        {#if hasSel && selRange}
          {#if cdsSelected}
            <button class="copy" onclick={copyNucleotides} title="Copy CDS nucleotides ({COPY_HINT})">
              ⧉ NT ({(selRange.hi - selRange.lo).toLocaleString()} bp)
            </button>
            <button class="copy" onclick={copyTranslation} title="Copy CDS translation">⧉ Protein</button>
          {:else}
            <button class="copy" onclick={copyNucleotides} title="Copy selected sequence ({COPY_HINT})">
              ⧉ Copy {(selRange.hi - selRange.lo).toLocaleString()} {data.contigs[selRange.ci].is_protein ? "aa" : "bp"}
            </button>
          {/if}
          <button class="extract" onclick={() => openSelExtract("nt")} title="Extract selection to a new GenBank">
            ⤓ {selIsNuc ? "NT" : "File"}
          </button>
          {#if selIsNuc}
            <button class="extract" onclick={() => openSelExtract("aa")} title="Extract the region's CDS peptides to a protein GenBank">⤓ Protein</button>
          {/if}
        {/if}
        {#if copyMsg}<span class="dim">{copyMsg}</span>{/if}
        {#if canEdit}
          <button onclick={() => (addOpen = !addOpen)}>{addOpen ? "Cancel add" : "Add annotation"}</button>
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
      onpointermove={(e) => { const r = scrollEl?.getBoundingClientRect(); if (r) lastCursorX = e.clientX - r.left; onPointerMove(e); }}
      onpointerup={onPointerUp}
      onpointerleave={onPointerLeave}
      role="application"
      aria-label="Contig annotation viewer"
    >
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

  {#if selected}
    <div class="editpanel">
      <div class="ep-head">
        <b>{selected.feat.kind}</b> · {selected.feat.label}
        <button class="ep-x" onclick={() => (selected = null)} aria-label="Close">×</button>
      </div>
      <div class="ep-body">
        <label>Color <input type="color" value={selected.feat.color} onchange={(e) => applyColor(e.currentTarget.value)} /></label>
        <button class="ep-remove" onclick={removeSelected}>Remove</button>
      </div>
    </div>
  {/if}

  {#if tooltip}
    <div class="tooltip" style="left: {tooltip.x}px; top: {tooltip.y}px;">
      <div class="tt-title">{tooltip.feature.kind}</div>
      <div class="tt-row"><b>start</b> {tooltip.feature.start}</div>
      <div class="tt-row"><b>end</b> {tooltip.feature.end}</div>
      <div class="tt-row"><b>strand</b> {tooltip.feature.strand === -1 ? "−" : tooltip.feature.strand === 1 ? "+" : "."}</div>
      {#each tooltip.feature.qualifiers as [k, v] (k)}
        <div class="tt-row"><b>{k}</b> {v}</div>
      {/each}
    </div>
  {/if}

  {#if showExtract}
    <NameModal
      title="Extract {contigSel.size} contig{contigSel.size === 1 ? '' : 's'} to a new file"
      defaultName="contigs_subset.gb"
      submitLabel="Extract"
      validate={validateFilename}
      onsubmit={(name) => doExtractContigs(name)}
      oncancel={() => (showExtract = false)}
    />
  {/if}

  {#if showSelExtract}
    <NameModal
      title="Extract selection ({selExtractKind === 'aa' ? 'CDS peptides → protein GenBank' : 'nucleotides → GenBank'})"
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
