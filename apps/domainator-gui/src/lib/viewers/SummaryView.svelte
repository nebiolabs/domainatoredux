<script lang="ts">
  import { runSummaryStats } from "../api";
  import { cached } from "../cache";
  import type { NameCount, SummaryStats, TaxNode } from "../types";

  let { path }: { path: string } = $props();

  let data = $state<SummaryStats | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(true);

  $effect(() => {
    void path;
    loading = true;
    error = null;
    data = null;
    cached(`summary:${path}`, () => runSummaryStats(path))
      .then((d) => (data = d))
      .catch((e) => (error = String(e)))
      .finally(() => (loading = false));
  });

  const unit = $derived(data?.is_protein ? "aa" : "bp");
  function fmt(n: number): string {
    return n.toLocaleString();
  }
  function fmt1(n: number): string {
    return n.toLocaleString(undefined, { maximumFractionDigits: 1 });
  }

  // Histogram geometry
  const W = 760;
  const H = 160;
  const M = { l: 40, r: 10, t: 8, b: 22 };
  const pw = W - M.l - M.r;
  const ph = H - M.t - M.b;
  const hist = $derived(data?.length_histogram ?? []);
  const histMax = $derived(Math.max(1, ...hist.map((b) => b[1])));
  const histLo = $derived(hist.length ? hist[0][0] : 0);
  const binW = $derived(hist.length > 1 ? hist[1][0] - hist[0][0] : 1);
  const histSpan = $derived(hist.length ? hist[hist.length - 1][0] + binW - histLo || 1 : 1);

  function maxCount(list: NameCount[]): number {
    return Math.max(1, ...list.map((c) => c.count));
  }

  // --- taxonomy icicle ---
  const COL = 150;
  const ICICLE_H = 440;
  const MIN_H = 0.6;
  type Rect = { x: number; y: number; w: number; h: number; name: string; count: number; color: string; label: boolean };
  const branchColor = (i: number) => `hsl(${(i * 53) % 360}, 52%, 70%)`;

  const icicle = $derived.by(() => {
    const rects: Rect[] = [];
    let maxDepth = 0;
    const root = data?.taxonomy;
    if (!root || root.count === 0) return { rects, width: COL };
    const total = root.count || 1;
    let y = 0;
    const walk = (node: TaxNode, depth: number, y0: number, h: number, color: string) => {
      if (h < MIN_H) return;
      maxDepth = Math.max(maxDepth, depth);
      rects.push({ x: depth * COL, y: y0, w: COL - 1, h, name: node.name, count: node.count, color, label: h >= 11 });
      let cy = y0;
      const t = node.count || 1;
      for (const ch of node.children) {
        const chh = h * (ch.count / t);
        walk(ch, depth + 1, cy, chh, color);
        cy += chh;
      }
    };
    root.children.forEach((child, i) => {
      const h = ICICLE_H * (child.count / total);
      walk(child, 0, y, h, branchColor(i));
      y += h;
    });
    return { rects, width: (maxDepth + 1) * COL };
  });
  function truncate(s: string, n: number): string {
    return s.length > n ? s.slice(0, n - 1) + "…" : s;
  }
</script>

<div class="sv">
  {#if loading}
    <div class="msg">Computing…</div>
  {:else if error}
    <div class="msg err">{error}</div>
  {:else if data}
    <div class="scroll">
      <div class="tiles">
        <div class="tile"><span class="v">{fmt(data.n_contigs)}</span><span class="k">contigs</span></div>
        <div class="tile"><span class="v">{fmt(data.total_residues)}</span><span class="k">total {unit}</span></div>
        <div class="tile"><span class="v">{fmt(data.n_cds)}</span><span class="k">CDS</span></div>
        <div class="tile"><span class="v">{fmt(data.n_domain_features)}</span><span class="k">domains</span></div>
        <div class="tile"><span class="v">{fmt(data.distinct_domains)}</span><span class="k">distinct domains</span></div>
        <div class="tile"><span class="v">{fmt1(data.length_mean)}</span><span class="k">mean len ({unit})</span></div>
        <div class="tile"><span class="v">{fmt(data.length_median)}</span><span class="k">median len</span></div>
        <div class="tile"><span class="v">{fmt(data.length_min)}–{fmt(data.length_max)}</span><span class="k">len range</span></div>
      </div>

      <h3>Contig length distribution</h3>
      {#if hist.length}
        <svg viewBox="0 0 {W} {H}" class="chart" role="img" aria-label="Length histogram">
          <line x1={M.l} y1={M.t + ph} x2={W - M.r} y2={M.t + ph} class="axis" />
          {#each hist as [lo, count] (lo)}
            {@const x = M.l + ((lo - histLo) / histSpan) * pw}
            {@const y = M.t + ph - (count / histMax) * ph}
            <rect x={x + 0.5} {y} width={Math.max(0.5, pw / hist.length - 1)} height={M.t + ph - y} class="bar" />
          {/each}
          <text x={M.l} y={H - 6} class="tick">{fmt(Math.round(histLo))}</text>
          <text x={W - M.r} y={H - 6} class="tick" text-anchor="end">{fmt(Math.round(histLo + histSpan))} {unit}</text>
          <text x={M.l - 5} y={M.t + 8} class="tick" text-anchor="end">{histMax}</text>
        </svg>
      {:else}
        <p class="msg">Not enough contigs to plot.</p>
      {/if}

      {#if data.n_with_taxonomy > 0 && icicle.rects.length > 0}
        <h3>Taxonomy <span class="dim">({fmt(data.n_with_taxonomy)} of {fmt(data.n_contigs)} contigs with lineage)</span></h3>
        <div class="icicle-scroll">
          <svg width={icicle.width} height={ICICLE_H} role="img" aria-label="Taxonomy icicle">
            {#each icicle.rects as r, ri (ri)}
              <g>
                <rect x={r.x} y={r.y} width={r.w} height={Math.max(0, r.h - 0.5)} fill={r.color} stroke="var(--bg)" stroke-width="0.5">
                  <title>{r.name} ({fmt(r.count)})</title>
                </rect>
                {#if r.label}
                  <text x={r.x + 4} y={r.y + r.h / 2 + 3} class="ilabel">{truncate(r.name, 20)}</text>
                {/if}
              </g>
            {/each}
          </svg>
        </div>
      {/if}

      <div class="cols">
        {#each [["Top domains", data.top_domains], ["Databases", data.databases], ["Feature kinds", data.feature_kinds], ["Organisms", data.top_organisms]] as [title, list] (title)}
          {#if (list as NameCount[]).length}
            {@const mx = maxCount(list as NameCount[])}
            <div class="barlist">
              <h4>{title}</h4>
              {#each list as NameCount[] as c (c.name)}
                <div class="brow" title={c.name}>
                  <span class="bname">{c.name}</span>
                  <span class="btrack"><span class="bfill" style="width: {(c.count / mx) * 100}%"></span></span>
                  <span class="bcount">{fmt(c.count)}</span>
                </div>
              {/each}
            </div>
          {/if}
        {/each}
      </div>
    </div>
  {/if}
</div>

<style>
  .sv {
    height: 100%;
    min-height: 0;
  }
  .scroll {
    height: 100%;
    overflow: auto;
    padding: 12px 16px;
  }
  .tiles {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
  }
  .tile {
    display: flex;
    flex-direction: column;
    min-width: 96px;
    padding: 8px 12px;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: 6px;
  }
  .tile .v {
    font-size: 18px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }
  .tile .k {
    color: var(--text-dim);
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.03em;
  }
  h3 {
    margin: 16px 0 6px;
    font-size: 13px;
  }
  h4 {
    margin: 0 0 6px;
    font-size: 12.5px;
  }
  .chart {
    width: 100%;
    max-width: 820px;
    height: auto;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: 6px;
  }
  .axis {
    stroke: var(--border);
  }
  .bar {
    fill: var(--accent);
    opacity: 0.8;
  }
  .tick {
    fill: var(--text-dim);
    font-size: 10px;
    font-family: var(--mono);
  }
  .dim {
    color: var(--text-dim);
    font-weight: 400;
    font-size: 12px;
  }
  .icicle-scroll {
    overflow-x: auto;
    max-width: 100%;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-elevated);
  }
  .icicle-scroll svg {
    display: block;
  }
  .ilabel {
    fill: #1c1f24;
    font-size: 11px;
    pointer-events: none;
  }
  .cols {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(260px, 1fr));
    gap: 16px;
    margin-top: 16px;
  }
  .barlist {
    min-width: 0;
  }
  .brow {
    display: grid;
    grid-template-columns: minmax(60px, 40%) 1fr 44px;
    align-items: center;
    gap: 8px;
    font-size: 12px;
    margin-bottom: 3px;
  }
  .bname {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .btrack {
    background: var(--bg-panel);
    border-radius: 3px;
    height: 12px;
    overflow: hidden;
  }
  .bfill {
    display: block;
    height: 100%;
    background: var(--accent);
    opacity: 0.7;
  }
  .bcount {
    text-align: right;
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .msg {
    padding: 20px;
    color: var(--text-dim);
  }
  .msg.err {
    color: #e5484d;
  }
</style>
