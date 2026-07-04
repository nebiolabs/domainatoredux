<script lang="ts">
  import { listHmmProfiles, loadHmmLogo } from "../api";
  import { cached } from "../cache";
  import type { HmmLogo, HmmSummary } from "../types";

  let { path }: { path: string } = $props();

  let profiles = $state<HmmSummary[]>([]);
  let selectedIndex = $state(0);
  let filter = $state("");
  let data = $state<HmmLogo | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(true);
  let listError = $state<string | null>(null);

  const LIST_CAP = 500; // cap rendered list items (files can have 25k profiles)

  // Load the profile list (cached) once per file.
  $effect(() => {
    void path;
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

  function axisTicks(max: number): number[] {
    const out: number[] = [];
    for (let b = 0; b <= Math.ceil(max); b++) out.push(b);
    return out;
  }
  const showLabel = (i: number, len: number) => i === 1 || i === len || i % 10 === 0;
</script>

<div class="hmm">
  {#if profiles.length > 1}
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
    <div class="scroll">
      <svg {width} {height} role="img" aria-label="HMM sequence logo">
        <!-- y-axis (bits) -->
        {#each axisTicks(data.max_height_bits) as b (b)}
          <line x1={AXIS_W} y1={TOP + PLOT_H - b * yScale} x2={width} y2={TOP + PLOT_H - b * yScale} class="grid" />
          <text x={AXIS_W - 5} y={TOP + PLOT_H - b * yScale + 3} class="tick" text-anchor="end">{b}</text>
        {/each}
        <text class="axis-label" transform="translate(10,{TOP + PLOT_H / 2}) rotate(-90)" text-anchor="middle">bits</text>

        {#each data.positions as pos (pos.index)}
          {@const x = AXIS_W + (pos.index - 1) * COL_W}
          <!-- stacked letters (ascending height, so tallest ends on top) -->
          {@const _base = TOP + PLOT_H}
          {#each pos.letters as letter, li (li)}
            {@const hPx = letter.height * yScale}
            {@const yTop = _base - pos.letters.slice(0, li + 1).reduce((s, l) => s + l.height, 0) * yScale}
            {#if hPx > 0.4}
              <g transform="translate({x + 1},{yTop}) scale({(COL_W - 2) / GLYPH_W},{hPx / GLYPH_CAP})">
                <text x="0" y={GLYPH_CAP} font-size="100" class="glyph" fill={colorOf(letter.char)}>{letter.char}</text>
              </g>
            {/if}
          {/each}
          <!-- occupancy bar -->
          {@const occTop = TOP + PLOT_H + 4 + (OCC_H - 6) * (1 - pos.occupancy)}
          <rect x={x + 3} y={occTop} width={COL_W - 6} height={(OCC_H - 6) * pos.occupancy} class="occ" />
          <!-- position label -->
          {#if showLabel(pos.index, data.length)}
            <text x={x + COL_W / 2} y={height - 6} class="tick" text-anchor="middle">{pos.index}</text>
          {/if}
        {/each}
        <line x1={AXIS_W} y1={TOP + PLOT_H} x2={width} y2={TOP + PLOT_H} class="axis" />
        <text x={AXIS_W - 5} y={TOP + PLOT_H + OCC_H} class="tick" text-anchor="end">occ</text>
      </svg>
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
  svg {
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
