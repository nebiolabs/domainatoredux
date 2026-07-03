<script lang="ts">
  import { runEnumReport, type EnumRow } from "../api";

  let { path }: { path: string } = $props();

  let by = $state("cds");
  let rows = $state<EnumRow[]>([]);
  let error = $state<string | null>(null);
  let loading = $state(true);
  let sortCol = $state<string | null>(null);
  let sortDir = $state<1 | -1>(1);

  $effect(() => {
    void path;
    void by;
    loading = true;
    error = null;
    sortCol = null;
    runEnumReport(path, by)
      .then((r) => (rows = r))
      .catch((e) => (error = String(e)))
      .finally(() => (loading = false));
  });

  const columns = $derived(rows.length ? Object.keys(rows[0]) : []);
  const isNumericCol = (c: string) => rows.some((r) => typeof r[c] === "number");

  const sorted = $derived.by(() => {
    if (!sortCol) return rows;
    const c = sortCol;
    const dir = sortDir;
    return [...rows].sort((a, b) => {
      const av = a[c];
      const bv = b[c];
      if (av === null || av === undefined) return 1;
      if (bv === null || bv === undefined) return -1;
      if (typeof av === "number" && typeof bv === "number") return (av - bv) * dir;
      return String(av).localeCompare(String(bv)) * dir;
    });
  });

  function toggleSort(c: string) {
    if (sortCol === c) sortDir = sortDir === 1 ? -1 : 1;
    else {
      sortCol = c;
      sortDir = 1;
    }
  }
</script>

<div class="er">
  <div class="controls">
    <label>
      By
      <select bind:value={by}>
        <option value="contig">contig</option>
        <option value="cds">cds</option>
        <option value="domain">domain</option>
      </select>
    </label>
    <span class="dim">{rows.length} row{rows.length === 1 ? "" : "s"}</span>
  </div>

  {#if loading}
    <div class="msg">Computing…</div>
  {:else if error}
    <div class="msg err">{error}</div>
  {:else if rows.length === 0}
    <div class="msg">No rows.</div>
  {:else}
    <div class="scroll">
      <table>
        <thead>
          <tr>
            {#each columns as c (c)}
              <th class:num={isNumericCol(c)} onclick={() => toggleSort(c)}>
                {c}{#if sortCol === c}<span class="arrow">{sortDir === 1 ? " ▲" : " ▼"}</span>{/if}
              </th>
            {/each}
          </tr>
        </thead>
        <tbody>
          {#each sorted as row, i (i)}
            <tr>
              {#each columns as c (c)}
                <td class:num={isNumericCol(c)}>{row[c] ?? ""}</td>
              {/each}
            </tr>
          {/each}
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
    padding: 3px 12px;
    border-bottom: 1px solid var(--border);
    white-space: nowrap;
  }
  th {
    position: sticky;
    top: 0;
    background: var(--bg-panel);
    cursor: pointer;
    user-select: none;
    font-weight: 600;
  }
  th:hover {
    background: var(--row-hover);
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
