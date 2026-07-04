<script lang="ts">
  import { untrack } from "svelte";
  import { runEnumReport, type EnumRow } from "../api";
  import { cached } from "../cache";

  let { path }: { path: string } = $props();

  // All selectable analyses (valid static analyses in enum_report).
  const ALL_COLUMNS = [
    "domains", "domain_descriptions", "architecture", "architecture_detailed",
    "length", "start", "end", "strand", "topology", "score", "identity",
    "definition", "cds_count", "domain_count", "sequence", "taxid_lineage", "taxname_lineage",
  ];
  function defaultColumns(b: string): string[] {
    if (b === "contig") return ["length", "cds_count", "domains"];
    if (b === "domain") return ["start", "end", "strand", "score", "identity"];
    return ["domains", "architecture", "length"];
  }

  let by = $state("cds");
  let selectedColumns = $state<string[]>(defaultColumns("cds"));
  let colsOpen = $state(false);
  let rows = $state<EnumRow[]>([]);
  let error = $state<string | null>(null);
  let loading = $state(true);
  let sortCol = $state<string | null>(null);
  let sortDir = $state<1 | -1>(1);
  let filters = $state<Record<string, string>>({});

  function onByChange() {
    selectedColumns = defaultColumns(by);
  }
  function toggleColumn(c: string) {
    selectedColumns = selectedColumns.includes(c)
      ? selectedColumns.filter((x) => x !== c)
      : [...selectedColumns, c];
  }

  // Virtualization: only rows in the scroll viewport are in the DOM; spacer rows
  // preserve the scrollbar's full extent.
  const ROW_H = 24;
  const OVERSCAN = 12;
  let scrollEl = $state<HTMLDivElement>();
  let scrollTop = $state(0);
  let clientH = $state(400);
  let rafPending = false;

  function onScroll() {
    if (rafPending) return;
    rafPending = true;
    requestAnimationFrame(() => {
      rafPending = false;
      if (scrollEl) {
        scrollTop = scrollEl.scrollTop;
        clientH = scrollEl.clientHeight;
      }
    });
  }

  // Reset columns to the granularity defaults when the file changes.
  $effect(() => {
    void path;
    selectedColumns = defaultColumns(untrack(() => by));
  });

  $effect(() => {
    void path;
    void by;
    void selectedColumns;
    loading = true;
    error = null;
    sortCol = null;
    filters = {};
    scrollTop = 0;
    const cols = selectedColumns;
    cached(`enum:${by}:${cols.join(",")}:${path}`, () => runEnumReport(path, by, cols))
      .then((r) => (rows = r))
      .catch((e) => (error = String(e)))
      .finally(() => (loading = false));
  });

  $effect(() => {
    if (scrollEl) {
      clientH = scrollEl.clientHeight;
      const ro = new ResizeObserver(() => {
        if (scrollEl) clientH = scrollEl.clientHeight;
      });
      ro.observe(scrollEl);
      return () => ro.disconnect();
    }
  });

  const columns = $derived(rows.length ? Object.keys(rows[0]) : []);
  const numericCols = $derived(
    new Set(columns.filter((c) => rows.some((r) => typeof r[c] === "number"))),
  );
  const hasFilters = $derived(Object.values(filters).some((v) => v.trim() !== ""));

  const filtered = $derived.by(() => {
    if (!hasFilters) return rows;
    const active = Object.entries(filters)
      .filter(([, v]) => v.trim() !== "")
      .map(([c, v]) => [c, v.trim().toLowerCase()] as const);
    return rows.filter((r) => active.every(([c, q]) => String(r[c] ?? "").toLowerCase().includes(q)));
  });

  const sorted = $derived.by(() => {
    if (!sortCol) return filtered;
    const c = sortCol;
    const dir = sortDir;
    return [...filtered].sort((a, b) => {
      const av = a[c];
      const bv = b[c];
      if (av === null || av === undefined) return 1;
      if (bv === null || bv === undefined) return -1;
      if (typeof av === "number" && typeof bv === "number") return (av - bv) * dir;
      return String(av).localeCompare(String(bv)) * dir;
    });
  });

  const total = $derived(sorted.length);
  const firstIdx = $derived(Math.max(0, Math.floor(scrollTop / ROW_H) - OVERSCAN));
  const lastIdx = $derived(Math.min(total, firstIdx + Math.ceil(clientH / ROW_H) + 2 * OVERSCAN));
  const visibleRows = $derived(sorted.slice(firstIdx, lastIdx));
  const topPad = $derived(firstIdx * ROW_H);
  const bottomPad = $derived(Math.max(0, (total - lastIdx) * ROW_H));

  function toggleSort(c: string) {
    if (sortCol === c) sortDir = sortDir === 1 ? -1 : 1;
    else {
      sortCol = c;
      sortDir = 1;
    }
    if (scrollEl) scrollEl.scrollTop = 0;
  }
</script>

<div class="er">
  <div class="controls">
    <label>
      By
      <select bind:value={by} onchange={onByChange}>
        <option value="contig">contig</option>
        <option value="cds">cds</option>
        <option value="domain">domain</option>
      </select>
    </label>
    <div class="colpick">
      <button onclick={() => (colsOpen = !colsOpen)}>Columns ({selectedColumns.length}) ▾</button>
      {#if colsOpen}
        <div class="colmenu" role="menu">
          {#each ALL_COLUMNS as c (c)}
            <label class="colitem">
              <input type="checkbox" checked={selectedColumns.includes(c)} onchange={() => toggleColumn(c)} />
              {c}
            </label>
          {/each}
        </div>
      {/if}
    </div>
    <span class="dim">
      {#if hasFilters}{total.toLocaleString()} of {rows.length.toLocaleString()}{:else}{rows.length.toLocaleString()}{/if}
      row{total === 1 ? "" : "s"}
    </span>
  </div>

  {#if loading}
    <div class="msg">Computing…</div>
  {:else if error}
    <div class="msg err">{error}</div>
  {:else if rows.length === 0}
    <div class="msg">No rows.</div>
  {:else}
    <div class="scroll" bind:this={scrollEl} onscroll={onScroll}>
      <table>
        <thead>
          <tr>
            {#each columns as c (c)}
              <th class:num={numericCols.has(c)} onclick={() => toggleSort(c)}>
                {c}{#if sortCol === c}<span class="arrow">{sortDir === 1 ? " ▲" : " ▼"}</span>{/if}
              </th>
            {/each}
          </tr>
          <tr class="filterrow">
            {#each columns as c (c)}
              <th>
                <input
                  type="text"
                  placeholder="filter"
                  value={filters[c] ?? ""}
                  oninput={(e) => (filters[c] = e.currentTarget.value)}
                />
              </th>
            {/each}
          </tr>
        </thead>
        <tbody>
          {#if topPad > 0}<tr style="height: {topPad}px"><td colspan={columns.length}></td></tr>{/if}
          {#each visibleRows as row, i (firstIdx + i)}
            <tr>
              {#each columns as c (c)}
                <td class:num={numericCols.has(c)}>{row[c] ?? ""}</td>
              {/each}
            </tr>
          {/each}
          {#if bottomPad > 0}<tr style="height: {bottomPad}px"><td colspan={columns.length}></td></tr>{/if}
        </tbody>
      </table>
    </div>
  {/if}
</div>

<style>
  .er {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .controls {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 6px 12px;
    border-bottom: 1px solid var(--border);
    background: var(--bg-panel);
  }
  .controls select {
    margin-left: 4px;
    background: var(--bg-elevated);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 2px 4px;
  }
  .dim {
    color: var(--text-dim);
  }
  .colpick {
    position: relative;
  }
  .colmenu {
    position: absolute;
    top: 100%;
    left: 0;
    margin-top: 4px;
    z-index: 20;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: 6px;
    box-shadow: 0 6px 20px rgba(0, 0, 0, 0.3);
    padding: 6px;
    max-height: 320px;
    overflow: auto;
    min-width: 180px;
  }
  .colitem {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 2px 6px;
    font-size: 12px;
    white-space: nowrap;
  }
  .colitem:hover {
    background: var(--row-hover);
  }
  .scroll {
    flex: 1;
    overflow: auto;
  }
  table {
    border-collapse: collapse;
    font-size: 12.5px;
    width: 100%;
  }
  th,
  td {
    text-align: left;
    padding: 0 12px;
    border-bottom: 1px solid var(--border);
    white-space: nowrap;
    height: 24px;
    box-sizing: border-box;
  }
  thead th {
    position: sticky;
    background: var(--bg-panel);
    cursor: pointer;
    user-select: none;
    font-weight: 600;
  }
  thead tr:first-child th {
    top: 0;
  }
  thead tr.filterrow th {
    top: 24px;
    cursor: auto;
    padding: 2px 6px;
  }
  thead tr.filterrow input {
    width: 100%;
    min-width: 60px;
    padding: 1px 4px;
    border: 1px solid var(--border);
    border-radius: 3px;
    background: var(--bg-elevated);
    color: var(--text);
    font-size: 11px;
    font-weight: 400;
  }
  td.num,
  th.num {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  tbody tr:hover {
    background: var(--row-hover);
  }
  .arrow {
    color: var(--accent);
  }
  .msg {
    padding: 20px;
    color: var(--text-dim);
  }
  .msg.err {
    color: #e5484d;
  }
</style>
