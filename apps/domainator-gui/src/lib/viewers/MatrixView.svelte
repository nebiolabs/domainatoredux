<script lang="ts">
  import { loadMatrixReport } from "../api";
  import type { MatrixReport } from "../types";

  let { path }: { path: string } = $props();

  let data = $state<MatrixReport | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(true);
  let selected = $state<number | null>(null); // selected threshold value

  $effect(() => {
    void path;
    loading = true;
    error = null;
    data = null;
    selected = null;
    loadMatrixReport(path)
      .then((d) => (data = d))
      .catch((e) => (error = String(e)))
      .finally(() => (loading = false));
  });

  function fmt(x: number): string {
    if (x === 0) return "0";
    const a = Math.abs(x);
    return a >= 1000 || a < 0.01 ? x.toExponential(1) : x.toFixed(2);
  }

  // Chart geometry (viewBox units; scales to container width).
  const W = 820;
  const H = 200;
  const M = { l: 48, r: 12, t: 10, b: 28 };
  const plotW = W - M.l - M.r;
  const plotH = H - M.t - M.b;

  // --- histogram ---
  const hist = $derived(data?.histogram ?? []);
  const histMax = $derived(Math.max(1, ...hist.map((b) => b[1])));
  const histMin = $derived(hist.length ? hist[0][0] : 0);
  const histBinW = $derived(hist.length > 1 ? hist[1][0] - hist[0][0] : 1);
  const histSpan = $derived(hist.length ? hist[hist.length - 1][0] + histBinW - histMin || 1 : 1);
  const hx = (v: number) => M.l + ((v - histMin) / histSpan) * plotW;
  const hy = (c: number) => M.t + plotH - (c / histMax) * plotH;

  // --- cluster-count vs threshold curve ---
  const curve = $derived(
    (data?.cluster_count_by_threshold ?? [])
      .filter((p): p is [number, number] => p[0] !== null)
      .sort((a, b) => a[0] - b[0]),
  );
  const tMin = $derived(curve.length ? curve[0][0] : 0);
  const tMax = $derived(curve.length ? curve[curve.length - 1][0] : 1);
  const tSpan = $derived(tMax - tMin || 1);
  const cMax = $derived(Math.max(1, ...curve.map((p) => p[1])));
  const cx = (t: number) => M.l + ((t - tMin) / tSpan) * plotW;
  const cy = (c: number) => M.t + plotH - (c / cMax) * plotH;
  const curvePts = $derived(curve.map((p) => `${cx(p[0])},${cy(p[1])}`).join(" "));

  // Cluster count that results from cutting at the selected threshold: the count
  // recorded at the nearest threshold in the curve.
  const selectedClusters = $derived.by(() => {
    if (selected === null || !curve.length) return null;
    let best = curve[0];
    let bestD = Infinity;
    for (const p of curve) {
      const d = Math.abs(p[0] - selected);
      if (d < bestD) {
        bestD = d;
        best = p;
      }
    }
    return best[1];
  });

  function pickThreshold(e: PointerEvent, svg: SVGSVGElement) {
    const rect = svg.getBoundingClientRect();
    const px = ((e.clientX - rect.left) / rect.width) * W;
    const t = tMin + ((px - M.l) / plotW) * tSpan;
    selected = Math.min(tMax, Math.max(tMin, t));
  }
</script>

<div class="mv">
  {#if loading}
    <div class="msg">Computing report…</div>
  {:else if error}
    <div class="msg err">{error}</div>
  {:else if data}
    <div class="scroll">
      <div class="stats">
        <div><span class="k">nodes</span>{data.n_nodes.toLocaleString()}</div>
        <div><span class="k">edges</span>{data.n_edges.toLocaleString()}</div>
        <div><span class="k">matrix</span>{data.n_rows}×{data.n_cols} {data.symmetric ? "sym" : "asym"}</div>
        <div><span class="k">type</span>{data.data_type}</div>
        <div><span class="k">mean</span>{fmt(data.stats.mean)}</div>
        <div><span class="k">median</span>{fmt(data.stats.median)}</div>
        <div><span class="k">min</span>{fmt(data.stats.min)}</div>
        <div><span class="k">max</span>{fmt(data.stats.max)}</div>
      </div>

      <h3>Edge-score distribution</h3>
      {#if hist.length}
        <svg viewBox="0 0 {W} {H}" class="chart" role="img" aria-label="Edge score histogram">
          <line x1={M.l} y1={M.t + plotH} x2={W - M.r} y2={M.t + plotH} class="axis" />
          {#each hist as [lo, count] (lo)}
            <rect
              x={hx(lo) + 0.5}
              y={hy(count)}
              width={Math.max(0.5, (plotW / hist.length) - 1)}
              height={M.t + plotH - hy(count)}
              class="bar"
            />
          {/each}
          <text x={M.l} y={H - 8} class="tick">{fmt(histMin)}</text>
          <text x={W - M.r} y={H - 8} class="tick" text-anchor="end">{fmt(histMin + histSpan)}</text>
          <text x={M.l - 6} y={M.t + 8} class="tick" text-anchor="end">{histMax}</text>
        </svg>
      {:else}
        <p class="msg">Not enough edges to plot.</p>
      {/if}

      <h3>Clusters vs. threshold <span class="dim">(click the chart to pick a cutoff)</span></h3>
      {#if curve.length}
        <svg
          viewBox="0 0 {W} {H}"
          class="chart pick"
          role="img"
          aria-label="Cluster count versus threshold"
          onpointerdown={(e) => pickThreshold(e, e.currentTarget)}
          onpointermove={(e) => e.buttons === 1 && pickThreshold(e, e.currentTarget)}
        >
          <line x1={M.l} y1={M.t + plotH} x2={W - M.r} y2={M.t + plotH} class="axis" />
          <line x1={M.l} y1={M.t} x2={M.l} y2={M.t + plotH} class="axis" />
          <polyline points={curvePts} class="curve" />
          {#if selected !== null}
            <line x1={cx(selected)} y1={M.t} x2={cx(selected)} y2={M.t + plotH} class="marker" />
          {/if}
          <text x={M.l} y={H - 8} class="tick">{fmt(tMin)}</text>
          <text x={W - M.r} y={H - 8} class="tick" text-anchor="end">{fmt(tMax)}</text>
          <text x={M.l - 6} y={M.t + 8} class="tick" text-anchor="end">{cMax}</text>
        </svg>
        {#if selected !== null}
          <p class="selection">
            threshold <b>{fmt(selected)}</b> → <b>{selectedClusters}</b> clusters
          </p>
        {/if}
      {:else}
        <p class="msg">No threshold curve available.</p>
      {/if}

      {#if data.suggested_thresholds.length}
        <h3>Suggested cutoffs</h3>
        <table class="cuts">
          <thead>
            <tr><th>Threshold</th><th>Clusters</th><th>Merge impact</th></tr>
          </thead>
          <tbody>
            {#each data.suggested_thresholds as s, i (i)}
              <tr onclick={() => (selected = s.threshold)} class:clickable={s.threshold !== null}>
                <td>{s.threshold_label}</td>
                <td>{s.cluster_count}</td>
                <td>{s.merge_impact === null ? "—" : fmt(s.merge_impact)}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </div>
  {/if}
</div>

<style>
  .mv {
    height: 100%;
    min-height: 0;
  }
  .scroll {
    height: 100%;
    overflow: auto;
    padding: 12px 16px;
  }
  .stats {
    display: flex;
    flex-wrap: wrap;
    gap: 6px 18px;
    margin-bottom: 8px;
  }
  .stats .k {
    color: var(--text-dim);
    margin-right: 6px;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  h3 {
    margin: 16px 0 6px;
    font-size: 13px;
  }
  .dim {
    color: var(--text-dim);
    font-weight: 400;
    font-size: 12px;
  }
  .chart {
    width: 100%;
    max-width: 900px;
    height: auto;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: 6px;
  }
  .chart.pick {
    cursor: crosshair;
  }
  .axis {
    stroke: var(--border);
    stroke-width: 1;
  }
  .bar {
    fill: var(--accent);
    opacity: 0.8;
  }
  .curve {
    fill: none;
    stroke: var(--accent);
    stroke-width: 1.5;
  }
  .marker {
    stroke: #e5484d;
    stroke-width: 1.5;
    stroke-dasharray: 4 3;
  }
  .tick {
    fill: var(--text-dim);
    font-size: 10px;
    font-family: var(--mono);
  }
  .selection {
    margin: 6px 0;
  }
  .cuts {
    border-collapse: collapse;
    font-size: 12.5px;
    min-width: 320px;
  }
  .cuts th,
  .cuts td {
    text-align: left;
    padding: 3px 14px 3px 0;
    border-bottom: 1px solid var(--border);
  }
  .cuts th {
    color: var(--text-dim);
    font-weight: 600;
  }
  .cuts tr.clickable {
    cursor: pointer;
  }
  .cuts tr.clickable:hover td {
    background: var(--row-hover);
  }
  .msg {
    padding: 8px 0;
    color: var(--text-dim);
  }
  .msg.err {
    color: #e5484d;
  }
</style>
