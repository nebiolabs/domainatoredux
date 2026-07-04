<script lang="ts">
  import { addFeature, loadContigView, removeFeature, setFeatureColor } from "../api";
  import { cached, invalidatePath } from "../cache";
  import { ws } from "../state.svelte";
  import type { ContigFileView, FeatureView } from "../types";

  let { path }: { path: string } = $props();

  let focused = $state<number | null>(null);

  // --- editing (GenBank only) ---
  const canEdit = $derived(/\.(gb|gbk|genbank|gbff)$/i.test(path));
  let selected = $state<{ contigIndex: number; feat: FeatureView } | null>(null);
  let addOpen = $state(false);
  let addForm = $state({ contigIndex: 0, kind: "misc_feature", start: 0, end: 100, strand: 1, note: "" });
  let editMsg = $state<string | null>(null);

  async function refetch() {
    // The file changed on disk (edit) — drop cached views for it, then reload.
    invalidatePath(path);
    try {
      data = await cached(`contig:${path}`, () => loadContigView(path));
    } catch (e) {
      error = String(e);
    }
  }
  function selectFeature(contigIndex: number, feat: FeatureView) {
    if (canEdit) selected = { contigIndex, feat };
  }
  const isSelected = (contigIndex: number, featIndex: number) =>
    selected?.contigIndex === contigIndex && selected?.feat.feat_index === featIndex;
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

  let data = $state<ContigFileView | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(true);

  // View controls
  let pxPerBp = $state(0.2);
  let enabledKinds = $state<Record<string, boolean>>({});
  let enabledDbs = $state<Record<string, boolean>>({});
  let maxEvalueExp = $state(0); // log10 threshold; features with evalue>10^exp hidden
  let hasEvalue = $state(false);

  let tooltip = $state<{ x: number; y: number; feature: FeatureView } | null>(null);

  // --- viewport (for virtualization): the visible horizontal slice ---
  let scrollEl = $state<HTMLDivElement>();
  let viewLeft = $state(0);
  let viewWidth = $state(1000);
  let viewTop = $state(0);
  let viewH = $state(600);
  let rafPending = false;
  // Max features drawn in detail (arrows + labels) per contig; above this the
  // visible window collapses to a density strip.
  const DETAIL_CAP = 800;
  const SVG_MARGIN = 12; // must match svg margin-left in CSS
  // Vertical virtualization: for files with many contigs, render only the
  // vertically-visible ones in fixed-height slots (handles 100k+ contigs).
  const VVIRT_THRESHOLD = 150;
  const VSLOT = 96;
  const VOVERSCAN = 4;

  function onScroll() {
    if (rafPending) return;
    rafPending = true;
    requestAnimationFrame(() => {
      rafPending = false;
      if (scrollEl) {
        viewLeft = scrollEl.scrollLeft;
        viewWidth = scrollEl.clientWidth;
        viewTop = scrollEl.scrollTop;
        viewH = scrollEl.clientHeight;
      }
    });
  }

  // --- lane-stacking (port of plot_contigs.py add_z_order) ---
  const PRECEDENCE = [
    "source",
    "CDS",
    "exon",
    "repeat_region",
    "ncRNA",
    "rRNA",
    "tRNA",
    "Domainator",
    "Domain_Search",
  ];
  const SQUASH = new Set(["CDS", "exon"]);
  const LANE_H = 22;
  const FEAT_H = 15;
  const RULER_H = 22;

  const kindOrder = (k: string) => {
    const i = PRECEDENCE.indexOf(k);
    return i < 0 ? PRECEDENCE.length + 1 : i;
  };
  const overlaps = (a: FeatureView, b: FeatureView) => a.start < b.end && b.start < a.end;

  function layout(features: FeatureView[]): { placed: { f: FeatureView; lane: number }[]; maxLane: number } {
    const sorted = [...features].sort((a, b) => kindOrder(a.kind) - kindOrder(b.kind) || a.start - b.start);
    const placed: { f: FeatureView; lane: number }[] = [];
    for (const f of sorted) {
      const occupied = new Set<number>();
      let override: number | null = null;
      for (const p of placed) {
        if (SQUASH.has(f.kind) && SQUASH.has(p.f.kind)) {
          override = p.lane;
          break;
        }
        if (overlaps(f, p.f)) occupied.add(p.lane);
      }
      let lane = override;
      if (lane === null) {
        lane = 0;
        while (occupied.has(lane)) lane++;
      }
      placed.push({ f, lane });
    }
    const maxLane = placed.reduce((m, p) => Math.max(m, p.lane), 0);
    return { placed, maxLane };
  }

  function visible(f: FeatureView): boolean {
    if (enabledKinds[f.kind] === false) return false;
    if (f.database !== null && enabledDbs[f.database] === false) return false;
    if (hasEvalue && f.evalue !== null && f.evalue > Math.pow(10, maxEvalueExp)) return false;
    return true;
  }

  type Placed = { f: FeatureView; lane: number };
  type Run = { x: number; w: number; color: string };
  type Plan = {
    contig: ContigFileView["contigs"][number];
    mode: "detail" | "overview";
    placed: Placed[];
    maxLane: number;
    bins: Run[];
  };

  function planFor(c: ContigFileView["contigs"][number], bpL: number, bpRraw: number): Plan {
    const feats = c.features.filter(visible);
    if (feats.length <= DETAIL_CAP) {
      const { placed, maxLane } = layout(feats);
      return { contig: c, mode: "detail", placed, maxLane, bins: [] };
    }
    const bpR = Math.min(c.length, bpRraw);
    const margin = (bpR - bpL) * 0.5;
    const wL = Math.max(0, bpL - margin);
    const wR = Math.min(c.length, bpR + margin);
    const windowFeats = feats.filter((f) => f.end > wL && f.start < wR);
    if (windowFeats.length <= DETAIL_CAP) {
      const { placed, maxLane } = layout(windowFeats);
      return { contig: c, mode: "detail", placed, maxLane, bins: [] };
    }
    return { contig: c, mode: "overview", placed: [], maxLane: 0, bins: densityBins(feats, bpL, bpR) };
  }

  // Doubly virtualized layout:
  //  - VERTICAL: with many contigs, only the vertically-visible ones (fixed VSLOT
  //    slots) are laid out/rendered; `topPad`/`bottomPad` preserve the scrollbar.
  //  - HORIZONTAL (per contig): ≤DETAIL_CAP features → draw all; denser → window to
  //    the horizontal viewport; very dense window → a binned density strip.
  const laidOut = $derived.by(() => {
    if (!data) return { plans: [] as Plan[], topPad: 0, bottomPad: 0, virt: false };
    void enabledKinds;
    void enabledDbs;
    void maxEvalueExp;
    const bpL = Math.max(0, (viewLeft - SVG_MARGIN) / pxPerBp);
    const bpRraw = (viewLeft + viewWidth - SVG_MARGIN) / pxPerBp;
    const contigs = data.contigs;
    const virt = contigs.length > VVIRT_THRESHOLD;
    if (!virt) {
      return { plans: contigs.map((c) => planFor(c, bpL, bpRraw)), topPad: 0, bottomPad: 0, virt };
    }
    const first = Math.max(0, Math.floor(viewTop / VSLOT) - VOVERSCAN);
    const last = Math.min(contigs.length, first + Math.ceil(viewH / VSLOT) + 2 * VOVERSCAN);
    const plans: Plan[] = [];
    for (let ci = first; ci < last; ci++) plans.push(planFor(contigs[ci], bpL, bpRraw));
    return { plans, topPad: first * VSLOT, bottomPad: (contigs.length - last) * VSLOT, virt };
  });

  // --- geometry ---
  const xOf = (bp: number) => bp * pxPerBp;

  // Density strip for the visible window: fill ~1 bin per pixel with the color of
  // the highest-precedence feature covering it, then merge into colored runs.
  function densityBins(feats: FeatureView[], bpL: number, bpR: number): Run[] {
    const spanBp = Math.max(1e-9, bpR - bpL);
    const nbins = Math.max(1, Math.min(2000, Math.ceil(spanBp * pxPerBp)));
    const colors: (string | null)[] = new Array(nbins).fill(null);
    const ordered = [...feats].sort((a, b) => kindOrder(a.kind) - kindOrder(b.kind));
    for (const f of ordered) {
      for (const p of f.parts) {
        if (p.end <= bpL || p.start >= bpR) continue;
        const a = Math.max(0, Math.floor(((p.start - bpL) / spanBp) * nbins));
        const b = Math.min(nbins, Math.ceil(((p.end - bpL) / spanBp) * nbins));
        for (let i = a; i < b; i++) colors[i] = f.color;
      }
    }
    const runs: Run[] = [];
    let i = 0;
    while (i < nbins) {
      if (colors[i] === null) {
        i++;
        continue;
      }
      const col = colors[i]!;
      let j = i;
      while (j < nbins && colors[j] === col) j++;
      const startBp = bpL + (i / nbins) * spanBp;
      const endBp = bpL + (j / nbins) * spanBp;
      runs.push({ x: xOf(startBp), w: Math.max(1, xOf(endBp) - xOf(startBp)), color: col });
      i = j;
    }
    return runs;
  }

  const OVERVIEW_H = RULER_H + LANE_H + 4;
  const planHeight = (p: Plan) => (p.mode === "overview" ? OVERVIEW_H : contigHeight(p.maxLane));
  const overviewY = RULER_H + (LANE_H - FEAT_H) / 2;

  // One part's shape at absolute x. `mode`: box (interior part), or right/left
  // arrow for the terminal part (in the feature's reading direction).
  function partPath(x: number, w: number, mode: "box" | "right" | "left"): string {
    const h = FEAT_H;
    const head = Math.min(8, w);
    if (mode === "right") {
      const b = x + w - head;
      return `M${x},0 L${b},0 L${x + w},${h / 2} L${b},${h} L${x},${h} Z`;
    }
    if (mode === "left") {
      return `M${x + w},0 L${x + head},0 L${x},${h / 2} L${x + head},${h} L${x + w},${h} Z`;
    }
    return `M${x},0 L${x + w},0 L${x + w},${h} L${x},${h} Z`;
  }

  // Segments + connector lines for a (possibly multi-part / origin-wrapping)
  // feature. Only the terminal part gets the arrowhead; intron connectors join
  // adjacent parts, but a huge gap (origin wrap on a circular contig) is skipped.
  function featureParts(f: FeatureView, contigLen: number) {
    const sp = [...f.parts].sort((a, b) => a.start - b.start);
    let term = -1;
    if (f.strand === 1) {
      let maxEnd = -Infinity;
      sp.forEach((p, i) => {
        if (p.end > maxEnd) {
          maxEnd = p.end;
          term = i;
        }
      });
    } else if (f.strand === -1) {
      term = 0; // smallest start after ascending sort
    }
    const segs = sp.map((p, i) => ({
      d: partPath(
        xOf(p.start),
        Math.max(1.5, (p.end - p.start) * pxPerBp),
        i === term ? (f.strand === 1 ? "right" : "left") : "box",
      ),
    }));
    const conns: { x1: number; x2: number }[] = [];
    for (let i = 0; i < sp.length - 1; i++) {
      const gapBp = sp[i + 1].start - sp[i].end;
      if (gapBp > 0 && gapBp < contigLen * 0.5) {
        conns.push({ x1: xOf(sp[i].end), x2: xOf(sp[i + 1].start) });
      }
    }
    return { segs, conns };
  }

  const contigHeight = (maxLane: number) => RULER_H + (maxLane + 1) * LANE_H + 4;
  const laneY = (lane: number, maxLane: number) => RULER_H + (maxLane - lane) * LANE_H + (LANE_H - FEAT_H) / 2;

  // bp ruler ticks (~8 evenly spaced)
  function ticks(length: number): number[] {
    const n = 8;
    const raw = length / n;
    const mag = Math.pow(10, Math.floor(Math.log10(raw)));
    const step = Math.max(1, Math.round(raw / mag) * mag);
    const out: number[] = [];
    for (let x = 0; x <= length; x += step) out.push(x);
    return out;
  }

  const labelFits = (f: FeatureView) => (f.end - f.start) * pxPerBp > 34;

  // --- controls ---
  function zoom(factor: number) {
    pxPerBp = Math.min(20, Math.max(0.002, pxPerBp * factor));
  }
  function fit() {
    if (!data) return;
    const maxLen = Math.max(1, ...data.contigs.map((c) => c.length));
    pxPerBp = Math.min(2, Math.max(0.002, 900 / maxLen));
  }

  function showTip(e: PointerEvent, feature: FeatureView) {
    tooltip = { x: e.clientX + 12, y: e.clientY + 12, feature };
  }
  function hideTip() {
    tooltip = null;
  }

  async function load() {
    loading = true;
    error = null;
    try {
      const d = await cached(`contig:${path}`, () => loadContigView(path));
      data = d;
      enabledKinds = Object.fromEntries(d.feature_kinds.map((k) => [k, true]));
      enabledDbs = Object.fromEntries(d.databases.map((k) => [k, true]));
      hasEvalue = d.evalue_range !== null;
      if (d.evalue_range) maxEvalueExp = Math.ceil(Math.log10(Math.max(d.evalue_range[1], 1e-300)));
      fit();
      // Honor a cross-view focus request (SSN node → this contig).
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

  // Track the scroll viewport width (drives virtualization window).
  $effect(() => {
    if (!scrollEl) return;
    viewWidth = scrollEl.clientWidth;
    const ro = new ResizeObserver(() => {
      if (scrollEl) viewWidth = scrollEl.clientWidth;
    });
    ro.observe(scrollEl);
    return () => ro.disconnect();
  });

  // Scroll to (and briefly highlight) a focused contig once it has rendered.
  $effect(() => {
    if (focused === null || !data) return;
    const idx = focused;
    requestAnimationFrame(() => {
      document.getElementById(`contig-${idx}`)?.scrollIntoView({ behavior: "smooth", block: "center" });
    });
    const timer = setTimeout(() => (focused = null), 2200);
    return () => clearTimeout(timer);
  });
</script>

<div class="cv">
  {#if loading}
    <div class="msg">Loading…</div>
  {:else if error}
    <div class="msg err">{error}</div>
  {:else if data}
    <div class="controls">
      <div class="zoom">
        <button onclick={() => zoom(1 / 1.5)} aria-label="Zoom out">−</button>
        <button onclick={fit}>Fit</button>
        <button onclick={() => zoom(1.5)} aria-label="Zoom in">+</button>
        <span class="dim">{data.contigs.length} contig{data.contigs.length === 1 ? "" : "s"}</span>
        {#if canEdit}
          <button onclick={() => (addOpen = !addOpen)}>{addOpen ? "Cancel add" : "Add annotation"}</button>
        {/if}
      </div>

      <div class="filters">
        {#each data.feature_kinds as kind (kind)}
          <label class="chip">
            <input type="checkbox" bind:checked={enabledKinds[kind]} />
            {kind}
          </label>
        {/each}
      </div>

      {#if data.databases.length}
        <div class="filters">
          <span class="dim">Databases:</span>
          {#each data.databases as db (db)}
            <label class="chip">
              <input type="checkbox" bind:checked={enabledDbs[db]} />
              {db}
            </label>
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
          <option value={1}>+</option>
          <option value={-1}>−</option>
          <option value={0}>.</option>
        </select>
        <input class="note" type="text" bind:value={addForm.note} placeholder="note (optional)" />
        <button class="apply" onclick={submitAdd}>Add</button>
      </div>
    {/if}
    {#if editMsg}<div class="editmsg err">{editMsg}</div>{/if}

    <div class="scroll" bind:this={scrollEl} onscroll={onScroll}>
      {#if laidOut.topPad > 0}<div style="height: {laidOut.topPad}px"></div>{/if}
      {#each laidOut.plans as plan (plan.contig.index)}
        <div
          class="contig"
          class:virt={laidOut.virt}
          id={"contig-" + plan.contig.index}
          class:focused={focused === plan.contig.index}
        >
          <div class="contig-head">
            <span class="cid" title={plan.contig.description}>{plan.contig.id || plan.contig.name}</span>
            <span class="dim">{plan.contig.length.toLocaleString()} {plan.contig.is_protein ? "aa" : "bp"}</span>
            {#if plan.contig.topology === "circular"}<span class="topo">circular</span>{/if}
            {#if plan.mode === "overview"}<span class="ov">overview · zoom in for detail</span>{/if}
            {#if plan.contig.description}<span class="cdesc" title={plan.contig.description}>{plan.contig.description}</span>{/if}
          </div>
          <svg
            width={Math.max(200, xOf(plan.contig.length))}
            height={planHeight(plan)}
            role="img"
            aria-label={"contig " + plan.contig.id}
          >
            <!-- ruler -->
            <line x1="0" y1={RULER_H - 1} x2={xOf(plan.contig.length)} y2={RULER_H - 1} class="axis" />
            {#each ticks(plan.contig.length) as t (t)}
              <line x1={xOf(t)} y1={RULER_H - 6} x2={xOf(t)} y2={RULER_H - 1} class="axis" />
              <text x={xOf(t) + 2} y={RULER_H - 8} class="tick">{t.toLocaleString()}</text>
            {/each}

            {#if plan.mode === "overview"}
              <!-- density strip (windowed) -->
              {#each plan.bins as run, bi (bi)}
                <rect x={run.x} y={overviewY} width={run.w} height={FEAT_H} fill={run.color} />
              {/each}
            {:else}
              <!-- features (arrows + labels) -->
              {#each plan.placed as { f, lane } (f.feat_index)}
                {@const fp = featureParts(f, plan.contig.length)}
                {@const sel = isSelected(plan.contig.index, f.feat_index)}
                <g
                  transform="translate(0,{laneY(lane, plan.maxLane)})"
                  onpointerenter={(e) => showTip(e, f)}
                  onpointermove={(e) => showTip(e, f)}
                  onpointerleave={hideTip}
                  onclick={() => selectFeature(plan.contig.index, f)}
                  role="presentation"
                >
                  {#each fp.conns as c, ci (ci)}
                    <line x1={c.x1} y1={FEAT_H / 2} x2={c.x2} y2={FEAT_H / 2} class="intron" />
                  {/each}
                  {#each fp.segs as s, si (si)}
                    <path
                      d={s.d}
                      fill={f.color}
                      stroke={sel ? "var(--accent)" : "rgba(0,0,0,0.35)"}
                      stroke-width={sel ? 2 : 0.5}
                      style={canEdit ? "cursor: pointer" : ""}
                    />
                  {/each}
                  {#if labelFits(f)}
                    <text x={xOf(f.start) + 4} y={FEAT_H / 2 + 4} class="flabel">{f.label}</text>
                  {/if}
                </g>
              {/each}
            {/if}
          </svg>
        </div>
      {/each}
      {#if laidOut.bottomPad > 0}<div style="height: {laidOut.bottomPad}px"></div>{/if}
    </div>
  {/if}

  {#if selected}
    <div class="editpanel">
      <div class="ep-head">
        <b>{selected.feat.kind}</b> · {selected.feat.label}
        <button class="ep-x" onclick={() => (selected = null)} aria-label="Close">×</button>
      </div>
      <div class="ep-body">
        <label>
          Color
          <input type="color" value={selected.feat.color} onchange={(e) => applyColor(e.currentTarget.value)} />
        </label>
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
  .editrow .k {
    width: 110px;
  }
  .editrow .n {
    width: 72px;
  }
  .editrow .note {
    width: 160px;
  }
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
  .scroll {
    flex: 1;
    overflow: auto;
    padding: 8px 0;
    position: relative;
  }
  .contig {
    margin-bottom: 12px;
    border-radius: 4px;
    transition: background 0.4s;
  }
  .contig.virt {
    margin-bottom: 0;
    height: 96px; /* == VSLOT */
    overflow: hidden;
  }
  .contig.focused {
    background: var(--accent-soft);
  }
  .contig-head {
    display: flex;
    align-items: baseline;
    gap: 10px;
    padding: 0 12px 2px;
    position: sticky;
    left: 0;
  }
  .cid {
    font-weight: 600;
    font-family: var(--mono);
  }
  .cdesc {
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .topo {
    font-size: 11px;
    color: #b5179e;
    border: 1px solid #b5179e55;
    border-radius: 8px;
    padding: 0 6px;
  }
  .ov {
    font-size: 11px;
    color: var(--text-dim);
    font-style: italic;
  }
  svg {
    display: block;
    margin-left: 12px;
  }
  .axis {
    stroke: var(--border);
    stroke-width: 1;
  }
  .tick {
    fill: var(--text-dim);
    font-size: 10px;
    font-family: var(--mono);
  }
  .flabel {
    fill: #111;
    font-size: 10px;
    pointer-events: none;
  }
  .intron {
    stroke: var(--text-dim);
    stroke-width: 1;
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
