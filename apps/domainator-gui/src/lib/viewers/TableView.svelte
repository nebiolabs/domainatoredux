<script lang="ts">
  import { createTextFile, readTextFile, writeTextFile } from "../api";
  import { ws } from "../state.svelte";
  import { validateFilename } from "../filetypes";
  import NameModal from "../NameModal.svelte";

  let { path }: { path: string } = $props();

  // Delimiter by extension: TSV/TAB → tab, CSV → comma.
  const isCsv = $derived(/\.csv$/i.test(path));
  const delim = $derived(isCsv ? "," : "\t");

  let rows = $state<string[][]>([]); // rows[0] is treated as the header
  let ncols = $state(0);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let dirty = $state(false);
  let saving = $state(false);
  let msg = $state<string | null>(null);
  let trailingNewline = true; // preserve a final newline on save

  const CAP = 5000; // editable-row cap (tables here are small; guard huge files)
  let truncated = $state(0);

  // Sort + row-selection state.
  let sortCol = $state(-1);
  let sortDir = $state<1 | -1>(1);
  let selected = $state(new Set<number>()); // indices into `rows` (≥1 = data rows)
  let rAnchor = -1;
  let showExtract = $state(false);

  // Minimal CSV field parse (quote-aware); TSV is a plain split.
  function parseLine(line: string, d: string): string[] {
    if (d === "\t" || !line.includes('"')) return line.split(d);
    const out: string[] = [];
    let cur = "";
    let q = false;
    for (let i = 0; i < line.length; i++) {
      const c = line[i];
      if (q) {
        if (c === '"') {
          if (line[i + 1] === '"') { cur += '"'; i++; } else q = false;
        } else cur += c;
      } else if (c === '"') q = true;
      else if (c === d) { out.push(cur); cur = ""; }
      else cur += c;
    }
    out.push(cur);
    return out;
  }
  function encodeField(v: string, d: string): string {
    if (d === "," && /[",\n]/.test(v)) return '"' + v.replace(/"/g, '""') + '"';
    return v;
  }
  const serialize = (data: string[][]) =>
    data.map((r) => r.map((v) => encodeField(v, delim)).join(delim)).join("\n");

  async function load() {
    loading = true;
    error = null;
    dirty = false;
    selected = new Set();
    sortCol = -1;
    try {
      const text = await readTextFile(path);
      trailingNewline = text.endsWith("\n");
      const lines = text.replace(/\r\n?/g, "\n").split("\n");
      if (trailingNewline && lines.length && lines[lines.length - 1] === "") lines.pop();
      truncated = Math.max(0, lines.length - CAP);
      const parsed = lines.slice(0, CAP).map((l) => parseLine(l, delim));
      ncols = parsed.reduce((m, r) => Math.max(m, r.length), 1);
      rows = parsed.map((r) => {
        const c = r.slice();
        while (c.length < ncols) c.push("");
        return c;
      });
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

  // ── edits ─────────────────────────────────────────────────────────────────
  function setCell(r: number, c: number, v: string) {
    rows[r][c] = v;
    dirty = true;
  }
  function addRow() {
    rows = [...rows, new Array(ncols).fill("")];
    dirty = true;
  }
  function deleteRow(r: number) {
    rows = rows.filter((_, i) => i !== r);
    selected = new Set(); // row indices shifted
    dirty = true;
  }
  function addColumn() {
    rows = rows.map((row) => [...row, ""]);
    ncols += 1;
    dirty = true;
  }
  function deleteColumn(c: number) {
    rows = rows.map((row) => row.filter((_, i) => i !== c));
    ncols = Math.max(0, ncols - 1);
    if (sortCol === c) sortCol = -1;
    else if (sortCol > c) sortCol -= 1;
    dirty = true;
  }
  function sortByColumn(c: number) {
    sortDir = sortCol === c ? (sortDir === 1 ? -1 : 1) : 1;
    sortCol = c;
    const dir = sortDir;
    const body = rows.slice(1).sort((a, b) => {
      const av = a[c] ?? "";
      const bv = b[c] ?? "";
      const an = parseFloat(av);
      const bn = parseFloat(bv);
      const numeric = av.trim() !== "" && bv.trim() !== "" && !isNaN(an) && !isNaN(bn);
      return (numeric ? an - bn : av.localeCompare(bv)) * dir;
    });
    rows = [rows[0], ...body];
    selected = new Set(); // order changed
    dirty = true;
  }

  // ── row selection (for Extract) ─────────────────────────────────────────────
  function rowSelect(e: MouseEvent, r: number) {
    if (e.shiftKey && rAnchor >= 1) {
      const [lo, hi] = rAnchor < r ? [rAnchor, r] : [r, rAnchor];
      const s = new Set(selected);
      for (let i = lo; i <= hi; i++) s.add(i);
      selected = s;
    } else if (e.ctrlKey || e.metaKey) {
      const s = new Set(selected);
      s.has(r) ? s.delete(r) : s.add(r);
      selected = s;
      rAnchor = r;
    } else {
      selected = new Set([r]);
      rAnchor = r;
    }
  }

  const baseName = $derived((path.split(/[\\/]/).pop() ?? "table").replace(/\.[^.]+$/, ""));
  const ext = $derived(isCsv ? "csv" : path.toLowerCase().endsWith(".tab") ? "tab" : "tsv");

  // Throws on failure so the modal shows it inline and stays open; closes on success.
  async function extractRows(name: string) {
    const dir = ws.currentDir;
    if (!dir || selected.size === 0) return;
    const idx = [...selected].sort((a, b) => a - b);
    const out = [rows[0], ...idx.map((i) => rows[i])];
    const body = serialize(out);
    const p = await createTextFile(dir, name, trailingNewline ? body + "\n" : body);
    await ws.refreshAndOpen(p);
    selected = new Set();
    showExtract = false;
  }

  // ── save ──────────────────────────────────────────────────────────────────
  async function save() {
    if (truncated > 0) {
      msg = "refusing to save: file was truncated for display";
      return;
    }
    saving = true;
    msg = null;
    try {
      const body = serialize(rows);
      await writeTextFile(path, trailingNewline ? body + "\n" : body);
      dirty = false;
      msg = "saved";
      setTimeout(() => (msg = null), 2000);
    } catch (e) {
      msg = "save failed: " + String(e);
    } finally {
      saving = false;
    }
  }

  const header = $derived(rows[0] ?? []);
</script>

<div class="tv">
  {#if loading}
    <div class="msg">Loading…</div>
  {:else if error}
    <div class="msg err">{error}</div>
  {:else}
    <div class="toolbar">
      <button onclick={save} disabled={!dirty || saving} class="save">{saving ? "Saving…" : "Save"}</button>
      <button onclick={addRow}>+ Row</button>
      <button onclick={addColumn}>+ Column</button>
      {#if selected.size > 0}
        <span class="sel-badge">{selected.size} selected</span>
        <button class="extract" onclick={() => (showExtract = true)}>Extract →</button>
        <button onclick={() => (selected = new Set())}>clear</button>
      {/if}
      <span class="dim">{Math.max(0, rows.length - 1).toLocaleString()} rows · {ncols} cols · {isCsv ? "CSV" : "TSV"}</span>
      {#if dirty}<span class="dot" title="Unsaved changes">●</span>{/if}
      {#if msg}<span class="dim">{msg}</span>{/if}
      {#if truncated > 0}<span class="warn">showing first {CAP.toLocaleString()} rows ({truncated.toLocaleString()} more — read-only)</span>{/if}
    </div>
    <div class="grid-scroll">
      <table>
        <thead>
          <tr>
            <th class="rownum"></th>
            {#each header as _, c (c)}
              <th>
                <div class="hcell">
                  <input value={header[c]} oninput={(e) => setCell(0, c, e.currentTarget.value)} />
                  <button class="hbtn" title="Sort by this column" onclick={() => sortByColumn(c)}>
                    {sortCol === c ? (sortDir === 1 ? "▲" : "▼") : "↕"}
                  </button>
                  <button class="hbtn del" title="Delete column" aria-label="Delete column" onclick={() => deleteColumn(c)}>×</button>
                </div>
              </th>
            {/each}
          </tr>
        </thead>
        <tbody>
          {#each rows.slice(1) as row, ri (ri)}
            {@const r = ri + 1}
            <tr class:sel={selected.has(r)}>
              <td class="rownum" onclick={(e) => rowSelect(e, r)} title="Click to select (Ctrl/Shift for multiple)">
                <span>{r}</span>
                <button class="del" title="Delete row" aria-label="Delete row" onclick={(e) => { e.stopPropagation(); deleteRow(r); }}>×</button>
              </td>
              {#each row as _, c (c)}
                <td><input value={row[c]} oninput={(e) => setCell(r, c, e.currentTarget.value)} /></td>
              {/each}
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
</div>

{#if showExtract}
  <NameModal
    title="Extract {selected.size} row{selected.size === 1 ? '' : 's'} to a new file"
    defaultName="{baseName}_subset.{ext}"
    submitLabel="Extract"
    validate={validateFilename}
    onsubmit={(name) => extractRows(name)}
    oncancel={() => (showExtract = false)}
  />
{/if}

<style>
  .tv {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
    border-bottom: 1px solid var(--border);
    background: var(--bg-panel);
  }
  .save,
  .extract {
    background: var(--accent);
    color: #fff;
    border-color: var(--accent);
  }
  .save:disabled {
    opacity: 0.5;
    cursor: default;
    background: var(--bg-elevated);
    color: var(--text);
    border-color: var(--border);
  }
  .sel-badge {
    color: var(--accent);
    font-size: 12px;
  }
  .dim {
    color: var(--text-dim);
    font-size: 12px;
  }
  .dot {
    color: var(--accent);
  }
  .warn {
    color: #d4880a;
    font-size: 12px;
  }
  .grid-scroll {
    flex: 1;
    overflow: auto;
    min-height: 0;
  }
  table {
    border-collapse: collapse;
    font-size: 12.5px;
    font-family: var(--mono);
  }
  th,
  td {
    border: 1px solid var(--border);
    padding: 0;
    vertical-align: middle;
  }
  thead th {
    position: sticky;
    top: 0;
    z-index: 1;
    background: var(--bg-panel);
  }
  .hcell {
    display: flex;
    align-items: center;
  }
  .hbtn {
    border: none;
    background: none;
    color: var(--text-dim);
    cursor: pointer;
    padding: 0 3px;
    font-size: 11px;
    line-height: 1;
  }
  .hbtn:hover {
    color: var(--text);
  }
  .hbtn.del:hover {
    color: #e5484d;
  }
  input {
    border: none;
    background: none;
    color: var(--text);
    padding: 3px 6px;
    width: 150px;
    font: inherit;
  }
  thead input {
    font-weight: 600;
    width: 120px;
  }
  input:focus {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
    background: var(--bg);
  }
  tbody tr.sel {
    background: var(--row-selected);
  }
  .rownum {
    position: sticky;
    left: 0;
    z-index: 1;
    background: var(--bg-panel);
    color: var(--text-dim);
    text-align: right;
    padding: 0 6px;
    white-space: nowrap;
    user-select: none;
    cursor: pointer;
  }
  tbody tr.sel .rownum {
    background: var(--row-selected);
  }
  thead .rownum {
    z-index: 2;
  }
  .rownum .del {
    border: none;
    background: none;
    color: var(--text-dim);
    cursor: pointer;
    opacity: 0;
    padding: 0 2px;
    font-size: 13px;
  }
  tr:hover .rownum .del {
    opacity: 0.7;
  }
  .rownum .del:hover {
    color: #e5484d;
    opacity: 1;
  }
  .msg {
    padding: 24px;
    color: var(--text-dim);
  }
  .msg.err {
    color: #e5484d;
  }
</style>
