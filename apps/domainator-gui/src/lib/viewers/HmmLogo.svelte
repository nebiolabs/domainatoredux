<script lang="ts">
  import { listHmmProfiles, loadHmmLogo } from "../api";
  import { cached } from "../cache";
  import type { HmmLogo, HmmSummary } from "../types";

  // `index` set → single-profile mode (a popped-out logo): show just that profile,
  // no browser list.
  let { path, index }: { path: string; index?: number } = $props();

  const single = $derived(index !== undefined);
  let profiles = $state<HmmSummary[]>([]);
  // Initialized to 0; the load effect sets it from `index` in single-profile mode.
  let selectedIndex = $state(0);
  let filter = $state("");
  let data = $state<HmmLogo | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(true);
  let listError = $state<string | null>(null);

  const LIST_CAP = 500; // cap rendered list items (files can have 25k profiles)

  // Load the profile list (cached) once per file — skipped in single-profile mode.
  $effect(() => {
    void path;
    if (single) {
      selectedIndex = index ?? 0;
      return;
    }
    selectedIndex = 0;
    filter = "";
    listError = null;
    cached(`hmmprofiles:${path}`, () => listHmmProfiles(path))
      .then((p) => (profiles = p))
      .catch((e) => (listError = String(e)));
  });

  // Load the selected profile's logo (cached per index).
  $effect(() => {
    const idx = selectedIndex;
    loading = true;
    error = null;
    data = null;
    cached(`hmmlogo:${path}:${idx}`, () => loadHmmLogo(path, idx))
      .then((d) => (data = d))
      .catch((e) => (error = String(e)))
      .finally(() => (loading = false));
  });

  const filtered = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    const list = q ? profiles.filter((p) => p.name.toLowerCase().includes(q)) : profiles;
    return list;
  });

  // Geometry
  const COL_W = 26;
  const PLOT_H = 170;
  const AXIS_W = 34;
  const OCC_H = 26;
  const LABEL_H = 22;
  const TOP = 8;

  // Residue colouring (amino by chemistry; nucleotides standard).
  const AA_COLORS: Record<string, string> = {
    A: "#20a0f0", V: "#20a0f0", L: "#20a0f0", I: "#20a0f0", M: "#20a0f0",
    F: "#20a0f0", W: "#20a0f0", // hydrophobic
    G: "#f09000", P: "#f09000", // special
    S: "#20c050", T: "#20c050", C: "#20c050", Y: "#20c050", N: "#20c050", Q: "#20c050", // polar
    K: "#e03030", R: "#e03030", H: "#e03030", // basic
    D: "#e030e0", E: "#e030e0", // acidic
  };
  const NT_COLORS: Record<string, string> = { A: "#20c050", C: "#20a0f0", G: "#f09000", T: "#e03030", U: "#e03030" };
  const colorOf = (ch: string) =>
    (data?.alphabet === "amino" ? AA_COLORS[ch] : NT_COLORS[ch]) ?? "#888";

  const width = $derived(data ? AXIS_W + data.length * COL_W + 12 : 0);
  const height = TOP + PLOT_H + OCC_H + LABEL_H;
  const yScale = $derived(data ? PLOT_H / data.max_height_bits : 1);

  // Nominal monospace glyph metrics (font-size 100) used to stretch letters.
  const GLYPH_W = 60;
  const GLYPH_CAP = 72;
  const BASE_Y = TOP + PLOT_H;

  function axisTicks(max: number): number[] {
    const out: number[] = [];
    for (let b = 0; b <= Math.ceil(max); b++) out.push(b);
    return out;
  }

  // Horizontal virtualization: only render columns in the scroll viewport.
  let scrollEl = $state<HTMLDivElement>();
  let viewLeft = $state(0);
  let viewW = $state(900);
  let rafPending = false;
  function onScroll() {
    if (rafPending) return;
    rafPending = true;
    requestAnimationFrame(() => {
      rafPending = false;
      if (scrollEl) {
        viewLeft = scrollEl.scrollLeft;
        viewW = scrollEl.clientWidth;
      }
    });
  }
  $effect(() => {
    if (!scrollEl) return;
    viewW = scrollEl.clientWidth;
    const ro = new ResizeObserver(() => {
      if (scrollEl) viewW = scrollEl.clientWidth;
    });
    ro.observe(scrollEl);
    return () => ro.disconnect();
  });

  // Viewport SVG width: only as wide as what's visible, clamped to remaining
  // content so the absolute SVG never overflows the (full-width) spacer.
  const vw = $derived(Math.max(1, Math.min(viewW, width - viewLeft)));

  type Glyph = { char: string; color: string; x: number; yTop: number; sx: number; sy: number };
  type Col = { index: number; glyphs: Glyph[]; occTop: number; occH: number; labelX: number; showLabel: boolean };

  // Precompute per-column glyph geometry ONCE (avoids the O(n²) cumulative-sum in
  // the template) and only for the visible column window.
  const columns = $derived.by((): Col[] => {
    if (!data) return [];
    void viewLeft;
    void viewW;
    const len = data.length;
    const first = Math.max(0, Math.floor((viewLeft - AXIS_W) / COL_W) - 4);
    const last = Math.min(len, Math.ceil((viewLeft + viewW - AXIS_W) / COL_W) + 4);
    const sx = (COL_W - 2) / GLYPH_W;
    const out: Col[] = [];
    for (let i = first; i < last; i++) {
      const pos = data.positions[i];
      if (!pos) continue;
      const x = AXIS_W + i * COL_W;
      const glyphs: Glyph[] = [];
      let cum = 0; // running height from the bottom (bits)
      for (const l of pos.letters) {
        const hPx = l.height * yScale;
        cum += l.height;
        if (hPx <= 0.4) continue;
        glyphs.push({
          char: l.char,
          color: colorOf(l.char),
          x: x + 1,
          yTop: BASE_Y - cum * yScale,
          sx,
          sy: hPx / GLYPH_CAP,
        });
      }
      out.push({
        index: pos.index,
        glyphs,
        occTop: BASE_Y + 4 + (OCC_H - 6) * (1 - pos.occupancy),
        occH: (OCC_H - 6) * pos.occupancy,
        labelX: x + COL_W / 2,
        showLabel: pos.index === 1 || pos.index === len || pos.index % 10 === 0,
      });
    }
    return out;
  });
</script>

<div class="hmm">
  {#if !single && profiles.length > 1}
    <div class="profile-list">
      <input class="pfilter" placeholder="filter {profiles.length.toLocaleString()} profiles…" bind:value={filter} />
      <div class="plist">
        {#each filtered.slice(0, LIST_CAP) as p (p.index)}
          <button
            class="pitem"
            class:active={p.index === selectedIndex}
            onclick={() => (selectedIndex = p.index)}
            title={p.desc ?? p.name}
          >
            <span class="pname">{p.name}</span><span class="plen">{p.length}</span>
          </button>
        {/each}
        {#if filtered.length > LIST_CAP}
          <div class="pmore">…{(filtered.length - LIST_CAP).toLocaleString()} more — refine the filter</div>
        {/if}
      </div>
    </div>
  {/if}

  <div class="logo-panel">
    {#if listError}
      <div class="msg err">{listError}</div>
    {:else if loading}
    <div class="msg">Loading…</div>
  {:else if error}
    <div class="msg err">{error}</div>
  {:else if data}
    <div class="head">
      <b>{data.name}</b>
      {#if data.acc}<span class="dim">{data.acc}</span>{/if}
      <span class="dim">· {data.length} positions · {data.alphabet}</span>
      {#if data.desc}<span class="desc" title={data.desc}>{data.desc}</span>{/if}
    </div>
    <div class="scroll" bind:this={scrollEl} onscroll={onScroll}>
      <!-- Full-width spacer carries the scrollbar; the SVG is only viewport-sized
           and follows scrollLeft (via CSS left + viewBox), so the browser never
           has to paint a model-wide (10k+ px) SVG layer while scrolling. -->
      <div class="spacer" style="width:{width}px; height:{height}px;">
      <svg
        class="viewport"
        width={vw}
        {height}
        viewBox="{viewLeft} 0 {vw} {height}"
        style="left:{viewLeft}px;"
        role="img"
        aria-label="HMM sequence logo"
      >
        <!-- y-axis (bits) -->
        {#each axisTicks(data.max_height_bits) as b (b)}
          <line x1={AXIS_W} y1={BASE_Y - b * yScale} x2={width} y2={BASE_Y - b * yScale} class="grid" />
          <text x={AXIS_W - 5} y={BASE_Y - b * yScale + 3} class="tick" text-anchor="end">{b}</text>
        {/each}
        <text class="axis-label" transform="translate(10,{TOP + PLOT_H / 2}) rotate(-90)" text-anchor="middle">bits</text>

        {#each columns as col (col.index)}
          {#each col.glyphs as g, gi (gi)}
            <g transform="translate({g.x},{g.yTop}) scale({g.sx},{g.sy})">
              <text x="0" y={GLYPH_CAP} font-size="100" class="glyph" fill={g.color}>{g.char}</text>
            </g>
          {/each}
          <rect x={col.labelX - COL_W / 2 + 3} y={col.occTop} width={COL_W - 6} height={col.occH} class="occ" />
          {#if col.showLabel}
            <text x={col.labelX} y={height - 6} class="tick" text-anchor="middle">{col.index}</text>
          {/if}
        {/each}
        <line x1={AXIS_W} y1={BASE_Y} x2={width} y2={BASE_Y} class="axis" />
        <text x={AXIS_W - 5} y={BASE_Y + OCC_H} class="tick" text-anchor="end">occ</text>
      </svg>
      </div>
    </div>
    <p class="legend dim">Letter height = information content (bits); lower strip = match-state occupancy.</p>
    {/if}
  </div>
</div>

<style>
  .hmm {
    display: flex;
    height: 100%;
    min-height: 0;
  }
  .profile-list {
    width: 240px;
    flex: none;
    display: flex;
    flex-direction: column;
    border-right: 1px solid var(--border);
    background: var(--bg-panel);
    min-height: 0;
  }
  .pfilter {
    margin: 6px;
    padding: 4px 6px;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg-elevated);
    color: var(--text);
    font-size: 12px;
  }
  .plist {
    flex: 1;
    overflow: auto;
    min-height: 0;
  }
  .pitem {
    display: flex;
    justify-content: space-between;
    gap: 8px;
    width: 100%;
    border: none;
    background: none;
    text-align: left;
    padding: 4px 8px;
    border-radius: 0;
    font-size: 12px;
  }
  .pitem:hover {
    background: var(--row-hover);
  }
  .pitem.active {
    background: var(--row-selected);
  }
  .pname {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .plen {
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .pmore {
    padding: 6px 8px;
    color: var(--text-dim);
    font-size: 11px;
  }
  .logo-panel {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }
  .head {
    display: flex;
    gap: 10px;
    align-items: baseline;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
    background: var(--bg-panel);
  }
  .desc {
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dim {
    color: var(--text-dim);
  }
  .scroll {
    flex: 1;
    overflow: auto;
    padding: 8px 0;
  }
  .spacer {
    position: relative;
  }
  svg.viewport {
    position: absolute;
    top: 0;
    display: block;
  }
  .glyph {
    font-family: "SF Mono", "Cascadia Code", Menlo, Consolas, monospace;
    font-weight: 700;
  }
  .grid {
    stroke: var(--border);
    stroke-width: 0.5;
    opacity: 0.5;
  }
  .axis {
    stroke: var(--border);
    stroke-width: 1;
  }
  .occ {
    fill: var(--accent);
    opacity: 0.55;
  }
  .tick {
    fill: var(--text-dim);
    font-size: 10px;
    font-family: var(--mono);
  }
  .axis-label {
    fill: var(--text-dim);
    font-size: 10px;
  }
  .legend {
    padding: 4px 12px 8px;
    font-size: 11px;
  }
  .msg {
    padding: 24px;
    color: var(--text-dim);
  }
  .msg.err {
    color: #e5484d;
  }
</style>
