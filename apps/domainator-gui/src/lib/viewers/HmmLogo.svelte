<script lang="ts">
  import { listHmmProfiles, loadHmmLogo, saveTextFile } from "../api";
  import { cached } from "../cache";
  import type { HmmLogo, HmmSummary, LogoPosition } from "../types";

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

  // ── Geometry ───────────────────────────────────────────────────────────────
  // Column width is user-adjustable (zoom); everything else is fixed.
  let colW = $state(26);
  const MIN_COL = 4;
  const MAX_COL = 60;
  // Below this per-column width, letters aren't legible — draw colored bars
  // instead (skylign's render_with_rects trick: far cheaper, still informative).
  const RECT_MODE_BELOW = 11;

  const AXIS_W = 42;
  const PLOT_H = 190;
  const OCC_H = 28;
  const LABEL_H = 22;
  const TOP = 10;
  const HEIGHT = TOP + PLOT_H + OCC_H + LABEL_H;
  const BASE_Y = TOP + PLOT_H; // y of the 0-bits baseline

  // Residue colouring — matches skylign's per-residue palette exactly
  // (hmm_logo_js `aa_colors` / `dna_colors`).
  const AA_COLORS: Record<string, string> = {
    A: "#FF9966", C: "#009999", D: "#FF0000", E: "#CC0033", F: "#00FF00",
    G: "#f2f20c", H: "#660033", I: "#CC9933", K: "#663300", L: "#FF9933",
    M: "#CC99CC", N: "#336666", P: "#0099FF", Q: "#6666CC", R: "#990000",
    S: "#0000FF", T: "#00FFFF", V: "#FFCC33", W: "#66CC66", Y: "#006600",
  };
  const NT_COLORS: Record<string, string> = {
    A: "#cbf751", C: "#5ec0cc", G: "#ffdf59", T: "#b51f16", U: "#b51f16",
  };
  const colorOf = (ch: string) =>
    (data?.alphabet === "amino" ? AA_COLORS[ch] : NT_COLORS[ch]) ?? "#888";

  const contentW = $derived(data ? data.length * colW : 0);
  const yScale = $derived(data && data.max_height_bits > 0 ? PLOT_H / data.max_height_bits : 1);

  // ── Glyph bitmap cache ───────────────────────────────────────────────────
  // Each (char,color) letter is rasterized ONCE into an offscreen canvas whose
  // pixels are the letter's ink box, tightly cropped. Placing a letter is then a
  // single scaled drawImage into the target rect — crisp, cheap, and it stretches
  // the way a sequence logo should (independent x/y scale). This is the core
  // perf/appearance win over per-glyph SVG <text> nodes.
  const GLYPH_PX = 128; // rasterization resolution (backing store)
  const GLYPH_FS = 96; // font size the metrics are measured at
  const GLYPH_FAMILY = 'Arial, "Helvetica Neue", Helvetica, sans-serif';
  const GLYPH_FONT = `bold ${GLYPH_FS}px ${GLYPH_FAMILY}`;

  // Ink-box metrics for one character, at GLYPH_FS, measured once and cached.
  // Both the canvas rasterizer and the SVG exporter use these to stretch a glyph
  // so its ink exactly fills a target rect.
  type Metrics = { left: number; asc: number; inkW: number; inkH: number };
  const metricCache = new Map<string, Metrics>();
  function metricsOf(ch: string): Metrics {
    const hit = metricCache.get(ch);
    if (hit) return hit;
    const probe = document.createElement("canvas").getContext("2d")!;
    probe.font = GLYPH_FONT;
    probe.textBaseline = "alphabetic";
    probe.textAlign = "left";
    const m = probe.measureText(ch);
    const left = m.actualBoundingBoxLeft ?? 0;
    const right = m.actualBoundingBoxRight ?? m.width;
    const asc = m.actualBoundingBoxAscent ?? 70;
    const desc = m.actualBoundingBoxDescent ?? 0;
    const met = { left, asc, inkW: Math.max(1, left + right), inkH: Math.max(1, asc + desc) };
    metricCache.set(ch, met);
    return met;
  }

  type Rasterized = { canvas: HTMLCanvasElement; w: number; h: number };
  const glyphCache = new Map<string, Rasterized>();

  function rasterize(ch: string, color: string): Rasterized {
    const key = ch + color;
    const hit = glyphCache.get(key);
    if (hit) return hit;

    const { left, asc, inkW, inkH } = metricsOf(ch);
    // Render into a canvas sized to the ink box (scaled up to GLYPH_PX tall for
    // resolution), positioned so the glyph fills it edge-to-edge.
    const scale = GLYPH_PX / inkH;
    const cw = Math.ceil(inkW * scale);
    const chh = Math.ceil(inkH * scale);
    const canvas = document.createElement("canvas");
    canvas.width = cw;
    canvas.height = chh;
    const ctx = canvas.getContext("2d")!;
    ctx.scale(scale, scale);
    ctx.font = GLYPH_FONT;
    ctx.textBaseline = "alphabetic";
    ctx.textAlign = "left";
    ctx.fillStyle = color;
    ctx.fillText(ch, left, asc);

    const r = { canvas, w: cw, h: chh };
    glyphCache.set(key, r);
    return r;
  }

  // ── Canvas rendering ───────────────────────────────────────────────────────
  let scrollEl = $state<HTMLDivElement>();
  let plotEl = $state<HTMLDivElement>();
  let mainCanvas = $state<HTMLCanvasElement>();
  let axisCanvas = $state<HTMLCanvasElement>();
  let viewW = $state(900);
  let viewLeft = 0;

  // Clicked column: the LogoPosition to detail in the popup, plus its screen
  // placement (relative to the .plot box).
  let selected = $state<LogoPosition | null>(null);
  let popupLeft = $state(0);
  let popupTop = $state(0);

  function cssVar(name: string, fallback: string): string {
    if (typeof getComputedStyle === "undefined" || !scrollEl) return fallback;
    const v = getComputedStyle(scrollEl).getPropertyValue(name).trim();
    return v || fallback;
  }

  // Redraw the fixed left axis (bits scale, gridline labels, occ/pos labels).
  function drawAxis() {
    const cv = axisCanvas;
    if (!cv || !data) return;
    const dpr = window.devicePixelRatio || 1;
    cv.width = Math.round(AXIS_W * dpr);
    cv.height = Math.round(HEIGHT * dpr);
    const ctx = cv.getContext("2d")!;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, AXIS_W, HEIGHT);

    const dim = cssVar("--text-dim", "#888");
    ctx.fillStyle = dim;
    ctx.strokeStyle = cssVar("--border", "#ccc");
    ctx.font = '10px ui-monospace, monospace';
    ctx.textBaseline = "middle";
    ctx.textAlign = "right";

    const maxB = Math.ceil(data.max_height_bits);
    for (let b = 0; b <= maxB; b++) {
      const y = BASE_Y - b * yScale;
      ctx.fillText(String(b), AXIS_W - 6, y);
    }
    // rotated "bits" label
    ctx.save();
    ctx.translate(11, TOP + PLOT_H / 2);
    ctx.rotate(-Math.PI / 2);
    ctx.textAlign = "center";
    ctx.fillText("bits", 0, 0);
    ctx.restore();

    ctx.textAlign = "right";
    ctx.fillText("occ", AXIS_W - 6, BASE_Y + OCC_H / 2 + 2);
  }

  // Redraw the visible slice of the plot into the viewport-sized main canvas.
  // Only the ~viewport-width columns are touched, so this is cheap enough to run
  // every scroll frame — no DOM diffing, unlike the old SVG approach.
  function draw() {
    const cv = mainCanvas;
    if (!cv || !data) return;
    const dpr = window.devicePixelRatio || 1;
    const w = Math.max(1, Math.min(viewW, contentW - viewLeft));
    // Only resize the backing store when it actually changes (resize clears it).
    const bw = Math.round(w * dpr);
    const bh = Math.round(HEIGHT * dpr);
    if (cv.width !== bw) cv.width = bw;
    if (cv.height !== bh) cv.height = bh;
    cv.style.width = w + "px";
    cv.style.height = HEIGHT + "px";
    cv.style.left = viewLeft + "px";

    const ctx = cv.getContext("2d")!;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, HEIGHT);
    // Translate so column i sits at (i*colW - viewLeft).
    ctx.translate(-viewLeft, 0);

    const gridColor = cssVar("--border", "#ccc");
    const dim = cssVar("--text-dim", "#888");
    const accent = cssVar("--accent", "#3b82f6");

    // gridlines across the visible width
    ctx.strokeStyle = gridColor;
    ctx.globalAlpha = 0.4;
    ctx.lineWidth = 1;
    const maxB = Math.ceil(data.max_height_bits);
    for (let b = 0; b <= maxB; b++) {
      const y = Math.round(BASE_Y - b * yScale) + 0.5;
      ctx.beginPath();
      ctx.moveTo(viewLeft, y);
      ctx.lineTo(viewLeft + w, y);
      ctx.stroke();
    }
    ctx.globalAlpha = 1;

    const first = Math.max(0, Math.floor(viewLeft / colW));
    const last = Math.min(data.length, Math.ceil((viewLeft + w) / colW));

    // Highlight the clicked column (skylign's ffdede/ff8888 selection box).
    if (selected) {
      const sx = (selected.index - 1) * colW;
      if (sx + colW >= viewLeft && sx <= viewLeft + w) {
        ctx.fillStyle = "rgba(255,120,120,0.16)";
        ctx.fillRect(sx, TOP, colW, PLOT_H + OCC_H);
        ctx.strokeStyle = "rgba(255,110,110,0.85)";
        ctx.strokeRect(sx + 0.5, TOP + 0.5, colW - 1, PLOT_H + OCC_H);
      }
    }

    const rectMode = colW < RECT_MODE_BELOW;
    const gap = colW > 8 ? 1 : 0;

    ctx.textBaseline = "alphabetic";
    ctx.textAlign = "center";
    ctx.font = '10px ui-monospace, monospace';

    for (let i = first; i < last; i++) {
      const pos = data.positions[i];
      if (!pos) continue;
      const x = i * colW;
      let cum = 0; // bits accumulated from the baseline upward

      for (const l of pos.letters) {
        const hPx = l.height * yScale;
        cum += l.height;
        if (hPx <= 0.4) continue;
        const yTop = BASE_Y - cum * yScale;
        if (rectMode) {
          ctx.fillStyle = colorOf(l.char);
          ctx.fillRect(x + gap, yTop, colW - gap * 2, hPx);
        } else {
          const g = rasterize(l.char, colorOf(l.char));
          ctx.drawImage(g.canvas, x + gap, yTop, colW - gap * 2, hPx);
        }
      }

      // occupancy strip
      const occH = (OCC_H - 6) * pos.occupancy;
      ctx.fillStyle = accent;
      ctx.globalAlpha = 0.55;
      ctx.fillRect(x + 3, BASE_Y + 4 + (OCC_H - 6) - occH, colW - 6, occH);
      ctx.globalAlpha = 1;

      // column-number label (thin out when zoomed in far or out)
      const showLabel =
        pos.index === 1 ||
        pos.index === data.length ||
        (colW >= 22 ? pos.index % 5 === 0 : pos.index % 10 === 0);
      if (showLabel) {
        ctx.fillStyle = dim;
        ctx.fillText(String(pos.index), x + colW / 2, HEIGHT - 6);
      }
    }

    // baseline
    ctx.strokeStyle = gridColor;
    ctx.beginPath();
    ctx.moveTo(viewLeft, BASE_Y + 0.5);
    ctx.lineTo(viewLeft + w, BASE_Y + 0.5);
    ctx.stroke();
  }

  let rafPending = false;
  function scheduleDraw() {
    if (rafPending) return;
    rafPending = true;
    requestAnimationFrame(() => {
      rafPending = false;
      draw();
    });
  }

  function onScroll() {
    if (scrollEl) viewLeft = scrollEl.scrollLeft;
    scheduleDraw();
  }

  // Click a column → open its detail popup (residue frequencies + metadata),
  // positioned near the cursor within the plot box.
  function onCanvasClick(e: MouseEvent) {
    if (!data || !mainCanvas || !plotEl) return;
    const contentX = viewLeft + e.offsetX;
    const idx0 = Math.floor(contentX / colW);
    const pos = data.positions[idx0];
    if (!pos) return;
    if (selected && selected.index === pos.index) {
      selected = null; // toggle off
    } else {
      selected = pos;
      const rect = plotEl.getBoundingClientRect();
      const PW = 300;
      popupLeft = Math.max(4, Math.min(rect.width - PW - 4, e.clientX - rect.left + 12));
      popupTop = Math.max(4, e.clientY - rect.top - 10);
    }
    scheduleDraw();
  }

  // Descending-by-probability residue rows for the popup table.
  const popupResidues = $derived(
    selected ? [...selected.letters].sort((a, b) => b.prob - a.prob) : [],
  );

  // ── SVG export ──────────────────────────────────────────────────────────────
  // Vector export of the FULL logo (all columns) — glyphs stretched to fill their
  // rects via the same ink-box metrics the canvas uses, so it matches on screen.
  const esc = (s: string) =>
    s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

  function buildSvg(): string {
    if (!data) return "";
    const W = AXIS_W + data.length * colW + 8;
    const gridC = "#d0d0d0";
    const dimC = "#555";
    const accentC = "#3b82f6";
    const parts: string[] = [];
    parts.push(
      `<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${HEIGHT}" viewBox="0 0 ${W} ${HEIGHT}" font-family='monospace'>`,
    );
    parts.push(`<rect width="${W}" height="${HEIGHT}" fill="#ffffff"/>`);

    // y-axis gridlines + tick labels
    const maxB = Math.ceil(data.max_height_bits);
    for (let b = 0; b <= maxB; b++) {
      const y = BASE_Y - b * yScale;
      parts.push(
        `<line x1="${AXIS_W}" y1="${y.toFixed(1)}" x2="${W}" y2="${y.toFixed(1)}" stroke="${gridC}" stroke-width="0.5"/>`,
      );
      parts.push(
        `<text x="${AXIS_W - 6}" y="${(y + 3).toFixed(1)}" font-size="10" fill="${dimC}" text-anchor="end">${b}</text>`,
      );
    }
    parts.push(
      `<text transform="translate(11,${TOP + PLOT_H / 2}) rotate(-90)" font-size="10" fill="${dimC}" text-anchor="middle">bits</text>`,
    );
    parts.push(
      `<line x1="${AXIS_W}" y1="${BASE_Y}" x2="${W}" y2="${BASE_Y}" stroke="#999" stroke-width="1"/>`,
    );
    parts.push(
      `<text x="${AXIS_W - 6}" y="${BASE_Y + OCC_H / 2 + 4}" font-size="10" fill="${dimC}" text-anchor="end">occ</text>`,
    );

    // columns
    for (let i = 0; i < data.length; i++) {
      const pos = data.positions[i];
      if (!pos) continue;
      const x = AXIS_W + i * colW;
      let cum = 0;
      for (const l of pos.letters) {
        const hPx = l.height * yScale;
        cum += l.height;
        if (hPx <= 0.4) continue;
        const yTop = BASE_Y - cum * yScale;
        const { left, asc, inkW, inkH } = metricsOf(l.char);
        const sxf = (colW - 2) / inkW;
        const syf = hPx / inkH;
        parts.push(
          `<g transform="translate(${(x + 1).toFixed(2)},${yTop.toFixed(2)}) scale(${sxf.toFixed(4)},${syf.toFixed(4)})">` +
            `<text x="${left.toFixed(2)}" y="${asc.toFixed(2)}" font-family="${GLYPH_FAMILY}" font-weight="bold" font-size="${GLYPH_FS}" fill="${colorOf(l.char)}">${esc(l.char)}</text></g>`,
        );
      }
      // occupancy strip
      const occH = (OCC_H - 6) * pos.occupancy;
      parts.push(
        `<rect x="${x + 3}" y="${(BASE_Y + 4 + (OCC_H - 6) - occH).toFixed(2)}" width="${colW - 6}" height="${occH.toFixed(2)}" fill="${accentC}" fill-opacity="0.55"/>`,
      );
      const showLabel =
        pos.index === 1 || pos.index === data.length || pos.index % 5 === 0;
      if (showLabel) {
        parts.push(
          `<text x="${x + colW / 2}" y="${HEIGHT - 6}" font-size="10" fill="${dimC}" text-anchor="middle">${pos.index}</text>`,
        );
      }
    }
    parts.push("</svg>");
    return parts.join("\n");
  }

  let exporting = $state(false);
  async function exportSvg() {
    if (!data || exporting) return;
    exporting = true;
    try {
      const name = (data.name || "hmm_logo").replace(/[^\w.-]+/g, "_");
      await saveTextFile(`${name}.svg`, "svg", "SVG image", buildSvg());
    } catch (e) {
      error = String(e);
    } finally {
      exporting = false;
    }
  }

  // Redraw whenever data, zoom, or size changes. Reading these makes the effect
  // reactive; glyphCache persists across redraws (it's keyed by char+color).
  $effect(() => {
    void data;
    void colW;
    void viewW;
    void selected;
    if (scrollEl) viewLeft = scrollEl.scrollLeft;
    drawAxis();
    draw();
  });

  // Clear any open popup when the profile changes.
  $effect(() => {
    void selectedIndex;
    void path;
    selected = null;
  });

  $effect(() => {
    if (!scrollEl) return;
    viewW = scrollEl.clientWidth;
    const ro = new ResizeObserver(() => {
      if (scrollEl) viewW = scrollEl.clientWidth;
    });
    ro.observe(scrollEl);
    // Redraw on DPR changes (e.g. window moved to another monitor).
    return () => ro.disconnect();
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
        <span class="zoom">
          <button class="zbtn" onclick={() => (colW = Math.max(MIN_COL, colW - 4))} title="Zoom out" aria-label="Zoom out">−</button>
          <input
            class="zslider"
            type="range"
            min={MIN_COL}
            max={MAX_COL}
            step="1"
            bind:value={colW}
            aria-label="Column width"
          />
          <button class="zbtn" onclick={() => (colW = Math.min(MAX_COL, colW + 4))} title="Zoom in" aria-label="Zoom in">+</button>
          <button class="ebtn" onclick={exportSvg} disabled={exporting} title="Export as SVG">
            {exporting ? "Exporting…" : "Export SVG"}
          </button>
        </span>
      </div>
      <div class="plot" bind:this={plotEl}>
        <canvas class="axis" bind:this={axisCanvas} style="width:{AXIS_W}px; height:{HEIGHT}px;"></canvas>
        <div class="scroll" bind:this={scrollEl} onscroll={onScroll}>
          <!-- Full-width spacer carries the horizontal scrollbar; the canvas is
               only viewport-sized and follows scrollLeft, so the browser never
               paints a model-wide (10k+ px) layer. -->
          <div class="spacer" style="width:{contentW}px; height:{HEIGHT}px;">
            <canvas class="viewport" bind:this={mainCanvas} onclick={onCanvasClick}></canvas>
          </div>
        </div>

        {#if selected}
          <div class="popup" style="left:{popupLeft}px; top:{popupTop}px;">
            <div class="phead">
              <b>Column {selected.index}</b>
              <button class="pclose" onclick={() => (selected = null)} aria-label="Close">×</button>
            </div>
            <div class="pmeta">
              <span>Occupancy <b>{selected.occupancy.toFixed(2)}</b></span>
              <span>Insert prob. <b>{selected.insert_prob.toFixed(2)}</b></span>
              <span>Insert len. <b>{Number.isFinite(selected.insert_length) ? selected.insert_length.toFixed(1) : "∞"}</b></span>
              <span>Delete prob. <b>{selected.delete_prob.toFixed(2)}</b></span>
            </div>
            <div class="pgrid">
              {#each popupResidues as l (l.char)}
                <div class="pcell">
                  <span class="swatch" style="background:{colorOf(l.char)}"></span>
                  <span class="pres">{l.char}</span>
                  <span class="pnum">{l.prob.toFixed(3)}</span>
                </div>
              {/each}
            </div>
          </div>
        {/if}
      </div>
      <p class="legend dim">
        Letter height = information content (bits); lower strip = match-state occupancy.
        {#if colW < RECT_MODE_BELOW}Zoom in to see residue letters.{/if}
      </p>
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
    flex: 1;
  }
  .dim {
    color: var(--text-dim);
  }
  .zoom {
    margin-left: auto;
    display: flex;
    align-items: center;
    gap: 4px;
    flex: none;
  }
  .zbtn {
    width: 20px;
    height: 20px;
    line-height: 1;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg-elevated);
    color: var(--text);
    cursor: pointer;
    font-size: 13px;
    padding: 0;
  }
  .zbtn:hover {
    background: var(--row-hover);
  }
  .zslider {
    width: 90px;
  }
  .ebtn {
    margin-left: 6px;
    padding: 2px 8px;
    height: 20px;
    line-height: 1;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg-elevated);
    color: var(--text);
    cursor: pointer;
    font-size: 11px;
    white-space: nowrap;
  }
  .ebtn:hover:not(:disabled) {
    background: var(--row-hover);
  }
  .ebtn:disabled {
    opacity: 0.6;
    cursor: default;
  }
  .plot {
    flex: 1;
    display: flex;
    min-height: 0;
    overflow: hidden;
    position: relative;
  }
  canvas.viewport {
    cursor: pointer;
  }
  .popup {
    position: absolute;
    z-index: 10;
    width: 300px;
    max-height: calc(100% - 8px);
    overflow: auto;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: 6px;
    box-shadow: 0 6px 24px rgba(0, 0, 0, 0.28);
    padding: 8px 10px 10px;
    font-size: 12px;
  }
  .phead {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 6px;
  }
  .pclose {
    border: none;
    background: none;
    color: var(--text-dim);
    cursor: pointer;
    font-size: 16px;
    line-height: 1;
    padding: 0 2px;
  }
  .pclose:hover {
    color: var(--text);
  }
  .pmeta {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 2px 12px;
    color: var(--text-dim);
    margin-bottom: 8px;
  }
  .pmeta b {
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }
  .pgrid {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: 1px 12px;
  }
  .pcell {
    display: flex;
    align-items: center;
    gap: 5px;
  }
  .swatch {
    width: 9px;
    height: 9px;
    border-radius: 2px;
    flex: none;
    display: inline-block;
  }
  .pres {
    font-family: var(--mono);
    font-weight: 600;
    width: 12px;
  }
  .pnum {
    margin-left: auto;
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  canvas.axis {
    flex: none;
    display: block;
    margin-top: 8px;
    z-index: 1;
  }
  .scroll {
    flex: 1;
    overflow-x: auto;
    overflow-y: hidden;
    padding: 8px 0;
    min-width: 0;
  }
  .spacer {
    position: relative;
  }
  canvas.viewport {
    position: absolute;
    top: 0;
    display: block;
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
