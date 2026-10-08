<script lang="ts">
  import { untrack } from "svelte";
  import {
    getEnumReportView,
    getPrefs,
    loadContigView,
    runEnumReport,
    setEnumReportView,
    type EnumRow,
    type EnumRowTarget,
  } from "../api";
  import { cached } from "../cache";
  import ColumnBuilder from "../ColumnBuilder.svelte";
  import { toColumns, toRows, type ColumnRow, type ColumnType } from "../columns";
  import { defaultColumns, loadEnumColumnTypes, sessionViews, type Column } from "../enumColumns";
  import { ws } from "../state.svelte";
  import { ENUM_BYS, fittingBys, itemKey, targetItem, toggleItem, type EnumBy, type SelItem } from "../selection";
  import type { ContigFileView, EnumReportView, Prefs } from "../types";

  let { path, active = true }: { path: string; active?: boolean } = $props();
  // Only for the "loading" hint: the backend decides what needs a taxonomy.
  const TAXONOMY_TYPES = new Set(["taxid", "taxname", "taxid_lineage", "taxname_lineage", "rank_lineage"]);

  let by = $state<EnumBy>("cds");
  /// The columns the grid shows. The builder edits `draft`, and Apply copies it
  /// here: every change reruns the whole report, so it isn't done per keystroke.
  let applied = $state<Column[]>(defaultColumns("cds", null));
  let draft = $state<ColumnRow[]>([]);
  let draftError = $state<string | null>(null);
  let colsOpen = $state(false);

  let types = $state<ColumnType[]>([]);
  $effect(() => {
    loadEnumColumnTypes()
      .then((t) => {
        types = t;
        draft = toRows(untrack(() => applied), types);
      })
      .catch((e) => (error = String(e)));
  });

  // ── remembered columns ─────────────────────────────────────────────────────
  // Each file remembers the granularity last shown and, per granularity, the
  // columns last applied (its workspace sidecar, `get/set_enum_report_view`).
  // A granularity it has nothing for starts from the Preferences default, and
  // failing that the built-in set. A file in no workspace has no sidecar, so it
  // is remembered for this session only (`sessionViews`).
  let saved: EnumReportView = { by: null, columns: {} };
  let prefs: Prefs | null = null;
  // Nothing runs until the file's remembered columns are in, so a file does not
  // first compute (and cache) the default report it is about to replace.
  let ready = $state(false);
  $effect(() => {
    const p = path;
    ready = false;
    Promise.all([getEnumReportView(p).catch(() => null), getPrefs().catch(() => null)]).then(([v, pr]) => {
      if (p !== path) return;
      saved = v ?? sessionViews.get(p) ?? { by: null, columns: {} };
      prefs = pr;
      by = ENUM_BYS.includes(saved.by as EnumBy) ? (saved.by as EnumBy) : "cds";
      setColumns(columnsFor(by));
      ready = true;
    });
  });
  function columnsFor(b: EnumBy): Column[] {
    return saved.columns[b] ?? defaultColumns(b, prefs);
  }
  function remember() {
    saved = { by, columns: { ...saved.columns } };
    sessionViews.set(path, saved);
    setEnumReportView(path, saved).catch(() => {});
  }

  // Registered taxonomy databases; the NCBI Taxonomy download first.
  const taxonomies = $derived(ws.databases.filter((d) => d.format === "taxonomy" && d.on_disk));
  let taxPath = $state<string>("");
  $effect(() => {
    if (taxPath && taxonomies.some((d) => d.path === taxPath)) return;
    taxPath = (taxonomies.find((d) => d.managed_id === "ncbi_taxonomy") ?? taxonomies[0])?.path ?? "";
  });
  const needsTaxonomy = $derived(applied.some((c) => TAXONOMY_TYPES.has(String(c.type))));
  // The taxonomy a run uses: none unless a column needs one, so switching it while
  // no taxonomy column is shown doesn't rerun the report (and reset the grid).
  const runTax = $derived(needsTaxonomy ? taxPath || null : null);

  function setColumns(cols: Column[]) {
    applied = cols;
    draft = toRows(cols, types);
    draftError = null;
  }
  function apply() {
    try {
      const cols = toColumns(draft, types);
      applied = cols;
      draftError = null;
      colsOpen = false;
      saved.columns[by] = cols;
      remember();
    } catch (e) {
      draftError = e instanceof Error ? e.message : String(e);
    }
  }
  let rows = $state<EnumRow[]>([]);
  // Each row's source record/feature, keyed by the row object itself so sorting
  // and filtering keep the link (and no hidden column reaches the grid).
  let targetOf = new Map<EnumRow, EnumRowTarget>();
  let error = $state<string | null>(null);
  let loading = $state(true);
  let sortCol = $state<string | null>(null);
  let sortDir = $state<1 | -1>(1);
  let filters = $state<Record<string, string>>({});

  function onByChange() {
    setColumns(columnsFor(by));
    remember();
  }
  // The builder's Defaults: back to the Preferences default, and forget this
  // file's own columns for the granularity so a later change of the default
  // reaches it.
  function resetColumns() {
    delete saved.columns[by];
    setColumns(defaultColumns(by, prefs));
    remember();
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

  $effect(() => {
    if (!ready) return;
    void path;
    void by;
    void applied;
    loading = true;
    error = null;
    sortCol = null;
    filters = {};
    scrollTop = 0;
    const cols = applied;
    const tax = runTax;
    cached(`enum:${by}:${JSON.stringify(cols)}:${tax ?? ""}:${path}`, () => runEnumReport(path, by, cols, tax))
      .then((r) => {
        targetOf = new Map(r.rows.map((row, i) => [row, r.targets[i]]));
        rows = r.rows;
      })
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

  // Column widths. The body is virtualized, so an auto-layout table would re-fit
  // its columns to whichever rows happen to be mounted and jump as you scroll.
  // Widths are instead fixed per column (seeded from the header and a sample of
  // rows, then drag-resizable), and long text is clipped with an ellipsis.
  const MIN_COL_W = 40;
  const MAX_AUTO_COL_W = 320;
  const CHAR_PX = 7.2; // ≈ one 12.5px system-ui glyph
  const WIDTH_SAMPLE = 200;
  // Keyed by column name, and kept across re-runs so a width the user set survives
  // toggling other columns or switching granularity.
  let colWidths = $state<Record<string, number>>({});

  $effect(() => {
    const cols = columns;
    const sample = rows.slice(0, WIDTH_SAMPLE);
    untrack(() => {
      const next = { ...colWidths };
      let changed = false;
      for (const c of cols) {
        if (next[c] !== undefined) continue;
        let chars = c.length + 2; // room for the sort arrow
        for (const r of sample) chars = Math.max(chars, String(r[c] ?? "").length);
        next[c] = Math.round(Math.min(MAX_AUTO_COL_W, Math.max(MIN_COL_W, chars * CHAR_PX + 26)));
        changed = true;
      }
      if (changed) colWidths = next;
    });
  });
  const CHECK_W = 26;
  const tableW = $derived(columns.reduce((sum, c) => sum + (colWidths[c] ?? 100), CHECK_W));

  // ── row checkboxes: the Contig View selection (`ws.viewSelection`) ──────────
  // A row is checked when its contig/CDS/domain is selected, but only while every
  // selected item is the kind of thing this granularity's rows are; a mixed
  // selection checks nothing, and checking a row then starts a new selection.
  // `from` records the granularity a selection made here was made at
  // (`enum:<by>`); one made in Contig View is classified by its features' kinds,
  // read from the Contig View data (`selection.ts`'s `fittingBys`).
  const viewSel = $derived(ws.viewSelection[path]);
  const selItems = $derived(viewSel?.items ?? []);
  const selKeys = $derived(new Set(selItems.map(itemKey)));
  let contigData = $state<ContigFileView | null>(null);
  // The same cache entry Contig View loads into, so this is normally free.
  function loadContigData(): Promise<ContigFileView> {
    return cached(`contig:${path}`, () => loadContigView(path)).then((d) => (contigData = d));
  }
  $effect(() => {
    if (viewSel?.from === "contig" && viewSel.items.length && !contigData) loadContigData().catch(() => {});
  });
  function fitsFor(items: SelItem[], from: string, d: ContigFileView | null): EnumBy[] {
    if (from.startsWith("enum:")) return [from.slice(5) as EnumBy];
    if (!d) return [];
    const kinds = new Map<string, string>();
    for (const c of d.contigs) for (const f of c.features) kinds.set(`${c.index}:${f.feat_index}`, f.kind);
    return fittingBys(
      items,
      (ci, feat) => kinds.get(`${ci}:${feat}`),
      (ci) => !!d.contigs[ci]?.is_protein,
    );
  }
  const selFits = $derived(
    selItems.length === 0 || !viewSel ? true : fitsFor(selItems, viewSel.from, contigData).includes(by),
  );
  function rowItem(row: EnumRow): SelItem | null {
    const t = targetOf.get(row);
    return t ? targetItem(t) : null;
  }
  function isChecked(row: EnumRow): boolean {
    if (!selFits) return false;
    const it = rowItem(row);
    return !!it && selKeys.has(itemKey(it));
  }
  function toggleRow(row: EnumRow) {
    const it = rowItem(row);
    if (!it) return;
    ws.setViewSelection(path, selFits ? toggleItem(selItems, it) : [it], `enum:${by}`);
  }
  // The header box checks or unchecks every row the filters leave.
  const filteredItems = $derived(filtered.map(rowItem).filter((x): x is SelItem => x !== null));
  const checkedCount = $derived(selFits ? rows.reduce((n, r) => n + (isChecked(r) ? 1 : 0), 0) : 0);
  const allChecked = $derived(
    selFits && filteredItems.length > 0 && filteredItems.every((it) => selKeys.has(itemKey(it))),
  );
  function toggleAll() {
    if (allChecked) {
      const drop = new Set(filteredItems.map(itemKey));
      ws.setViewSelection(path, selItems.filter((it) => !drop.has(itemKey(it))), `enum:${by}`);
    } else {
      const base = selFits ? selItems : [];
      const have = new Set(base.map(itemKey));
      ws.setViewSelection(path, [...base, ...filteredItems.filter((it) => !have.has(itemKey(it)))], `enum:${by}`);
    }
  }

  // When Contig View changed the selection while this tab was hidden, and every
  // item in it is a contig, a CDS or a domain, switch to that granularity so the
  // rows come up checked. A mixed selection leaves the granularity alone.
  let seenRev = 0;
  $effect(() => {
    if (!active) return;
    const cur = ws.viewSelection[path];
    if (!cur || cur.rev === seenRev) return;
    seenRev = cur.rev;
    if (cur.from !== "contig" || !cur.items.length) return;
    const items = cur.items;
    loadContigData()
      .then((d) => {
        const fits = fitsFor(items, "contig", d);
        if (fits.length && !fits.includes(untrack(() => by))) {
          by = fits[0];
          onByChange();
        }
      })
      .catch(() => {});
  });

  // Drag a header's right edge to resize that column. The click that ends the
  // drag lands on the header, so `resizing` stops it from also toggling the sort.
  let resizing = false;
  function startResize(e: PointerEvent, c: string) {
    e.preventDefault();
    e.stopPropagation();
    resizing = true;
    const startX = e.clientX;
    const startW = colWidths[c] ?? 100;
    const move = (ev: PointerEvent) => {
      colWidths[c] = Math.max(MIN_COL_W, Math.round(startW + ev.clientX - startX));
    };
    const up = () => {
      setTimeout(() => (resizing = false));
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }

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
      <button onclick={() => (colsOpen = !colsOpen)}>Columns ({applied.length}) ▾</button>
      {#if colsOpen}
        <div class="colmenu" role="dialog" aria-label="Columns">
          {#if types.length === 0}
            <div class="dim">Loading column types…</div>
          {:else}
            <ColumnBuilder {types} bind:rows={draft} />
          {/if}
          {#if draftError}<div class="colerr">{draftError}</div>{/if}
          <div class="colactions">
            <button onclick={resetColumns} title="The default for this granularity (Edit → Preferences)">Defaults</button>
            <span class="spacer"></span>
            <button onclick={() => { draft = toRows(applied, types); draftError = null; colsOpen = false; }}>Cancel</button>
            <button class="primary" onclick={apply} disabled={types.length === 0}>Apply</button>
          </div>
        </div>
      {/if}
    </div>
    {#if taxonomies.length > 0}
      <label title="The taxonomy the taxid, taxon-name and lineage columns use">
        Taxonomy
        <select bind:value={taxPath}>
          {#each taxonomies as d (d.path)}<option value={d.path}>{d.name}</option>{/each}
        </select>
      </label>
    {/if}
    <span class="dim">
      {#if hasFilters}{total.toLocaleString()} of {rows.length.toLocaleString()}{:else}{rows.length.toLocaleString()}{/if}
      row{total === 1 ? "" : "s"}{#if checkedCount > 0}, {checkedCount.toLocaleString()} selected{/if}
    </span>
  </div>

  {#if loading}
    <div class="msg">
      Computing…{#if needsTaxonomy} The first run with a taxonomy column also loads the taxonomy, which takes several seconds.{/if}
    </div>
  {:else if error}
    <div class="msg err">{error}</div>
  {:else if rows.length === 0}
    <div class="msg">No rows.</div>
  {:else}
    <div class="scroll" bind:this={scrollEl} onscroll={onScroll}>
      <table style="width: {tableW}px">
        <colgroup>
          <col style="width: {CHECK_W}px" />
          {#each columns as c (c)}
            <col style="width: {colWidths[c] ?? 100}px" />
          {/each}
        </colgroup>
        <thead>
          <tr>
            <th class="check-cell">
              <input
                type="checkbox"
                checked={allChecked}
                onchange={toggleAll}
                title={hasFilters ? "Select the rows the filters show" : "Select all rows"}
                aria-label="Select all rows"
              />
            </th>
            {#each columns as c (c)}
              <th class:num={numericCols.has(c)} onclick={() => !resizing && toggleSort(c)} title={c}>
                {c}{#if sortCol === c}<span class="arrow">{sortDir === 1 ? " ▲" : " ▼"}</span>{/if}
                <!-- svelte-ignore a11y_no_static_element_interactions -->
                <span class="resizer" onpointerdown={(e) => startResize(e, c)}></span>
              </th>
            {/each}
          </tr>
          <tr class="filterrow">
            <th class="check-cell"></th>
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
          {#if topPad > 0}<tr style="height: {topPad}px"><td colspan={columns.length + 1}></td></tr>{/if}
          {#each visibleRows as row, i (firstIdx + i)}
            <tr>
              <td class="check-cell">
                <input
                  type="checkbox"
                  checked={isChecked(row)}
                  onchange={() => toggleRow(row)}
                  title="Select in Contig View"
                  aria-label="Select in Contig View"
                />
              </td>
              {#each columns as c (c)}
                <td class:num={numericCols.has(c)} title={String(row[c] ?? "")}>{row[c] ?? ""}</td>
              {/each}
            </tr>
          {/each}
          {#if bottomPad > 0}<tr style="height: {bottomPad}px"><td colspan={columns.length + 1}></td></tr>{/if}
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
    padding: 8px;
    max-height: min(520px, 70vh);
    overflow: auto;
    width: min(480px, 90vw);
  }
  .colerr {
    color: #e5484d;
    font-size: 12px;
    margin-top: 6px;
  }
  .colactions {
    display: flex;
    gap: 6px;
    margin-top: 8px;
  }
  .colactions .spacer {
    flex: 1;
  }
  .colactions .primary {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }
  .scroll {
    flex: 1;
    overflow: auto;
  }
  table {
    border-collapse: collapse;
    font-size: 12.5px;
    table-layout: fixed;
  }
  th,
  td {
    text-align: left;
    padding: 0 12px;
    border-bottom: 1px solid var(--border);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    height: 24px;
    box-sizing: border-box;
  }
  .resizer {
    position: absolute;
    top: 0;
    right: 0;
    width: 6px;
    height: 100%;
    cursor: col-resize;
    user-select: none;
  }
  .resizer:hover {
    background: var(--accent);
    opacity: 0.5;
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
    min-width: 0;
    box-sizing: border-box;
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
  th.check-cell,
  td.check-cell {
    padding: 0;
    text-align: center;
    cursor: auto;
  }
  .check-cell input {
    margin: 0;
    vertical-align: middle;
    cursor: pointer;
  }
  .msg {
    padding: 20px;
    color: var(--text-dim);
  }
  .msg.err {
    color: #e5484d;
  }
</style>
