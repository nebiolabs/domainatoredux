<script lang="ts">
  // A multiple sequence alignment (aligned FASTA): the residue grid, colored by residue
  // class or by agreement with the consensus, under tracks of each column's
  // conservation and occupancy, with rows selectable for export.
  //
  // The grid is a canvas drawn from blocks of rows fetched as they scroll into view
  // (`alignment_window`), so a large alignment opens without loading every residue
  // into the page. Sequence ids, sizes and the tracks come up front (`alignment_info`).
  import { alignmentInfo, alignmentWindow, exportAlignmentRows } from "../api";
  import type { AlignmentInfo } from "../api";
  import { fileTypeOf, validateFilename } from "../filetypes";
  import { ws } from "../state.svelte";
  import NameModal from "../NameModal.svelte";

  let { path }: { path: string } = $props();

  const ROW_H = 16;
  const COL_W = 10;
  const NAME_W = 200;
  const TRACK_H = 46; // ruler 14 + conservation 20 + consensus 12
  const BLOCK = 64; // rows fetched at a time

  let info = $state<AlignmentInfo | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(true);
  let scheme = $state<"residue" | "consensus" | "none">("residue");
  let filter = $state("");
  let selected = $state<Set<number>>(new Set());
  let anchor: number | null = null;
  let exporting = $state<null | { gapped: boolean }>(null);
  let note = $state<{ text: string; err: boolean } | null>(null);

  let scroller = $state<HTMLDivElement | null>(null);
  let grid = $state<HTMLCanvasElement | null>(null);
  let names = $state<HTMLCanvasElement | null>(null);
  let tracks = $state<HTMLCanvasElement | null>(null);
  let viewW = $state(0);
  let viewH = $state(0);
  let scrollX = $state(0);
  let scrollY = $state(0);

  // Row blocks by block index; `pending` keeps a block from being fetched twice.
  let blocks = new Map<number, string[]>();
  const pending = new Set<number>();
  let redraws = $state(0);

  const stem = $derived(
    (path.split(/[\\/]/).pop() ?? "alignment").replace(/\.(afa|afasta|fasta|fa|fna|faa|pep)(\.gz)?$/i, ""),
  );

  // The rows shown, as indices into the alignment: all of them, or those whose id
  // contains the filter text.
  const rows = $derived.by(() => {
    if (!info) return [] as number[];
    const f = filter.trim().toLowerCase();
    const all = info.ids.map((_, i) => i);
    return f ? all.filter((i) => info!.ids[i].toLowerCase().includes(f)) : all;
  });

  $effect(() => {
    void path;
    loading = true;
    error = null;
    info = null;
    selected = new Set();
    blocks = new Map();
    pending.clear();
    alignmentInfo(path)
      .then((i) => (info = i))
      .catch((e) => (error = String(e)))
      .finally(() => (loading = false));
  });

  // Track the scroller's size.
  $effect(() => {
    if (!scroller) return;
    const ro = new ResizeObserver(() => {
      viewW = scroller!.clientWidth;
      viewH = scroller!.clientHeight;
    });
    ro.observe(scroller);
    return () => ro.disconnect();
  });

  // Redraw when the theme changes, since canvas colors are read from CSS.
  $effect(() => {
    const mo = new MutationObserver(() => redraws++);
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = () => redraws++;
    mq.addEventListener("change", onChange);
    return () => {
      mo.disconnect();
      mq.removeEventListener("change", onChange);
    };
  });

  function onScroll() {
    if (!scroller) return;
    scrollX = scroller.scrollLeft;
    scrollY = scroller.scrollTop;
  }

  function fetchBlock(b: number) {
    if (!info || blocks.has(b) || pending.has(b)) return;
    pending.add(b);
    const at = path;
    alignmentWindow(path, b * BLOCK, BLOCK, 0, info.length)
      .then((seqs) => {
        if (at !== path) return;
        blocks.set(b, seqs);
        // Keep the cache bounded on a very tall alignment.
        if (blocks.size > 256) blocks.delete(blocks.keys().next().value!);
        redraws++;
      })
      .catch((e) => (error = String(e)))
      .finally(() => pending.delete(b));
  }

  function residueAt(row: number): string | null {
    const block = blocks.get(Math.floor(row / BLOCK));
    return block ? block[row % BLOCK] : null;
  }

  // Clustal X's residue classes, and the usual nucleotide colors.
  const PROTEIN: Record<string, string> = {};
  for (const c of "AILMFWVC") PROTEIN[c] = "#80a0f0";
  for (const c of "KR") PROTEIN[c] = "#f01505";
  for (const c of "DE") PROTEIN[c] = "#c048c0";
  for (const c of "NQST") PROTEIN[c] = "#15c015";
  for (const c of "HY") PROTEIN[c] = "#15a4a4";
  PROTEIN.G = "#f09048";
  PROTEIN.P = "#c0c000";
  const NUCLEOTIDE: Record<string, string> = { A: "#64f73f", C: "#ffb340", G: "#eb413c", T: "#3c88ee", U: "#3c88ee" };

  function cellColor(ch: string, col: number): string | null {
    if (ch === "-" || ch === ".") return null;
    const up = ch.toUpperCase();
    if (scheme === "residue") return (info?.alphabet === "nucleotide" ? NUCLEOTIDE : PROTEIN)[up] ?? null;
    if (scheme === "consensus" && info && info.consensus[col] === up) {
      // Darker the more of the column agrees.
      const a = 0.25 + 0.75 * info.conservation[col];
      return `rgba(47, 111, 235, ${a.toFixed(2)})`;
    }
    return null;
  }

  function prepare(canvas: HTMLCanvasElement, w: number, h: number) {
    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.max(1, Math.floor(w * dpr));
    canvas.height = Math.max(1, Math.floor(h * dpr));
    canvas.style.width = `${w}px`;
    canvas.style.height = `${h}px`;
    const ctx = canvas.getContext("2d")!;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    return ctx;
  }

  // Draw the visible part of the grid, the ids beside it and the tracks above it.
  $effect(() => {
    void redraws;
    void scheme;
    void selected;
    if (!info || !grid || !names || !tracks || viewW === 0) return;
    const style = getComputedStyle(scroller!);
    const fg = style.getPropertyValue("--text").trim() || "#000";
    const dim = style.getPropertyValue("--text-dim").trim() || "#666";
    const selBg = style.getPropertyValue("--row-selected").trim() || "#d6e4ff";
    const accent = style.getPropertyValue("--accent").trim() || "#2f6feb";

    const firstRow = Math.floor(scrollY / ROW_H);
    const lastRow = Math.min(rows.length, Math.ceil((scrollY + viewH) / ROW_H));
    const firstCol = Math.floor(scrollX / COL_W);
    const lastCol = Math.min(info.length, Math.ceil((scrollX + viewW) / COL_W));
    const offX = firstCol * COL_W - scrollX;
    const offY = firstRow * ROW_H - scrollY;

    const g = prepare(grid, viewW, viewH);
    g.font = "12px monospace";
    g.textAlign = "center";
    g.textBaseline = "middle";
    const n = prepare(names, NAME_W, viewH);
    n.font = "12px monospace";
    n.textBaseline = "middle";
    for (let r = firstRow; r < lastRow; r++) {
      const row = rows[r];
      const y = offY + (r - firstRow) * ROW_H;
      if (selected.has(row)) {
        g.fillStyle = selBg;
        g.fillRect(0, y, viewW, ROW_H);
        n.fillStyle = selBg;
        n.fillRect(0, y, NAME_W, ROW_H);
      }
      n.fillStyle = fg;
      n.fillText(info.ids[row], 6, y + ROW_H / 2, NAME_W - 10);
      const seq = residueAt(row);
      if (seq === null) {
        fetchBlock(Math.floor(row / BLOCK));
        continue;
      }
      for (let c = firstCol; c < lastCol; c++) {
        const ch = seq[c];
        const x = offX + (c - firstCol) * COL_W;
        const color = cellColor(ch, c);
        if (color) {
          g.fillStyle = color;
          g.fillRect(x, y, COL_W, ROW_H);
          g.fillStyle = "#111";
        } else {
          g.fillStyle = ch === "-" || ch === "." ? dim : fg;
        }
        g.fillText(ch, x + COL_W / 2, y + ROW_H / 2 + 1);
      }
    }

    const t = prepare(tracks, viewW, TRACK_H);
    t.font = "10px monospace";
    t.textBaseline = "top";
    t.fillStyle = dim;
    for (let c = firstCol; c < lastCol; c++) {
      const x = offX + (c - firstCol) * COL_W;
      if ((c + 1) % 10 === 0) {
        t.textAlign = "right";
        t.fillText(String(c + 1), x + COL_W, 1);
      }
      // Conservation as a bar, occupancy as its faint background.
      t.globalAlpha = 0.25;
      t.fillStyle = dim;
      t.fillRect(x + 1, 34 - 18 * info.occupancy[c], COL_W - 2, 18 * info.occupancy[c]);
      t.globalAlpha = 1;
      t.fillStyle = accent;
      t.fillRect(x + 1, 34 - 18 * info.conservation[c], COL_W - 2, 18 * info.conservation[c]);
      t.fillStyle = fg;
      t.textAlign = "center";
      t.fillText(info.consensus[c], x + COL_W / 2, 35);
      t.fillStyle = dim;
    }
  });

  // Select a row by its id: a plain click selects it alone, Ctrl/Cmd toggles it and
  // Shift extends the selection from the last row clicked.
  function onNameClick(e: MouseEvent) {
    if (!names || !info) return;
    const rect = names.getBoundingClientRect();
    const r = Math.floor((e.clientY - rect.top + scrollY) / ROW_H);
    if (r < 0 || r >= rows.length) return;
    const row = rows[r];
    const next = new Set(e.ctrlKey || e.metaKey || e.shiftKey ? selected : []);
    if (e.shiftKey && anchor !== null) {
      const a = rows.indexOf(anchor);
      const [lo, hi] = a < r ? [a, r] : [r, a];
      for (let i = Math.max(lo, 0); i <= hi; i++) next.add(rows[i]);
    } else if ((e.ctrlKey || e.metaKey) && next.has(row)) {
      next.delete(row);
    } else {
      next.add(row);
    }
    anchor = row;
    selected = next;
  }

  function selectShown() {
    selected = new Set(rows);
  }

  async function doExport(name: string) {
    if (!info || !exporting) return;
    const dir = ws.currentDir;
    if (!dir) throw new Error("no workspace folder to write into");
    const ids = [...selected].sort((a, b) => a - b).map((i) => info!.ids[i]);
    const out = await exportAlignmentRows(path, dir, name, ids, exporting.gapped);
    exporting = null;
    note = { text: `Wrote ${ids.length} sequence${ids.length === 1 ? "" : "s"} to ${name}.`, err: false };
    await ws.refreshFolder();
    ws.openPath(out, fileTypeOf(name) ?? "fasta");
  }
</script>

<div class="aln">
  {#if loading}
    <div class="msg">Reading the alignment…</div>
  {:else if error}
    <div class="msg err">{error}</div>
  {:else if info}
    <div class="bar">
      <span class="dim">
        {info.ids.length.toLocaleString()} sequences × {info.length.toLocaleString()} columns · {info.alphabet}
      </span>
      <label>
        Color
        <select bind:value={scheme}>
          <option value="residue">by residue</option>
          <option value="consensus">by consensus</option>
          <option value="none">none</option>
        </select>
      </label>
      <input type="search" placeholder="Filter ids…" bind:value={filter} />
      <span class="spacer"></span>
      {#if selected.size > 0}
        <span>{selected.size.toLocaleString()} selected</span>
        <button onclick={() => (exporting = { gapped: true })} disabled={!ws.currentDir}>Export aligned…</button>
        <button onclick={() => (exporting = { gapped: false })} disabled={!ws.currentDir}>Export sequences…</button>
        <button onclick={() => (selected = new Set())}>Clear</button>
      {:else}
        <button onclick={selectShown} disabled={rows.length === 0}>Select shown</button>
      {/if}
    </div>
    {#if note}
      <div class="bar">
        <span class:err={note.err}>{note.text}</span>
        <button onclick={() => (note = null)}>Dismiss</button>
      </div>
    {/if}
    <div class="frame">
      <div class="corner" style="width: {NAME_W}px; height: {TRACK_H}px">
        <span class="dim">conservation / consensus</span>
      </div>
      <canvas class="tracks" bind:this={tracks} style="left: {NAME_W}px"></canvas>
      <canvas
        class="names"
        bind:this={names}
        style="top: {TRACK_H}px"
        onclick={onNameClick}
        title="Click to select; Ctrl/Cmd-click to toggle; Shift-click for a range"
      ></canvas>
      <div
        class="scroller"
        bind:this={scroller}
        onscroll={onScroll}
        style="left: {NAME_W}px; top: {TRACK_H}px"
      >
        <!-- At least the viewport's size, so the sticky canvas never stretches it. -->
        <div
          class="spacer-box"
          style="width: {Math.max(info.length * COL_W, viewW)}px; height: {Math.max(rows.length * ROW_H, viewH)}px"
        >
          <canvas class="grid" bind:this={grid}></canvas>
        </div>
      </div>
    </div>
  {/if}

  {#if exporting}
    <NameModal
      title="Export {selected.size} selected {exporting.gapped ? 'aligned sequence' : 'sequence'}{selected.size === 1 ? '' : 's'}"
      defaultName={`${stem}_selected.${exporting.gapped ? "afa" : "fasta"}`}
      submitLabel="Export"
      validate={validateFilename}
      onsubmit={(name) => doExport(name)}
      oncancel={() => (exporting = null)}
    />
  {/if}
</div>

<style>
  .aln {
    height: 100%;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 5px 12px;
    background: var(--bg-panel);
    border-bottom: 1px solid var(--border);
    font-size: 12.5px;
  }
  .bar .spacer {
    flex: 1;
  }
  .dim {
    color: var(--text-dim);
  }
  .err {
    color: #e5484d;
  }
  .frame {
    position: relative;
    flex: 1;
    min-height: 0;
    overflow: hidden;
    background: var(--bg);
  }
  .corner {
    position: absolute;
    left: 0;
    top: 0;
    display: flex;
    align-items: flex-end;
    padding: 0 6px 3px;
    box-sizing: border-box;
    font-size: 10.5px;
    border-right: 1px solid var(--border);
    border-bottom: 1px solid var(--border);
  }
  .tracks {
    position: absolute;
    top: 0;
    border-bottom: 1px solid var(--border);
  }
  .names {
    position: absolute;
    left: 0;
    border-right: 1px solid var(--border);
    cursor: pointer;
  }
  .scroller {
    position: absolute;
    right: 0;
    bottom: 0;
    overflow: auto;
  }
  .spacer-box {
    position: relative;
  }
  .grid {
    position: sticky;
    left: 0;
    top: 0;
    display: block;
  }
  .msg {
    padding: 24px;
    color: var(--text-dim);
  }
  .msg.err {
    color: #e5484d;
  }
</style>
