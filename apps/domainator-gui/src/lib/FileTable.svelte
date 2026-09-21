<script lang="ts">
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { ws } from "./state.svelte";
  import { countRecords, createTextFile, pickFiles } from "./api";
  import { validateFilename } from "./filetypes";
  import NameModal from "./NameModal.svelte";
  import ConfirmDialog from "./ConfirmDialog.svelte";
  import { openContextMenu, SEP, type CtxItem } from "./ctxmenu.svelte";
  import { ui } from "./ui.svelte";
  import { openFileWindow, revealInFileManager } from "./api";
  import { copyText } from "./clipboard";
  import type { FileEntry } from "./types";

  const TYPE_LABEL: Record<string, string> = {
    genbank: "GenBank",
    fasta: "FASTA",
    hmm: "HMM",
    hdf5: "Matrix",
    cm: "CM",
    ssn_viewer: "SSN",
    table: "Table",
    stockholm: "Stockholm",
  };

  // ── column model ────────────────────────────────────────────────────────────
  type ColKey = "name" | "type" | "description" | "size" | "created" | "modified" | "records";
  const COLUMNS: { key: ColKey; label: string; width: string; align?: "right" }[] = [
    { key: "name", label: "Name", width: "minmax(140px, 2fr)" },
    { key: "type", label: "Type", width: "88px" },
    { key: "description", label: "Description", width: "minmax(140px, 3fr)" },
    { key: "size", label: "Size", width: "78px", align: "right" },
    { key: "records", label: "Records", width: "76px", align: "right" },
    { key: "created", label: "Created", width: "132px" },
    { key: "modified", label: "Modified", width: "132px" },
  ];
  const TOGGLEABLE = COLUMNS.filter((c) => c.key !== "name"); // name is always shown

  function loadCols(): Set<ColKey> {
    try {
      const s = localStorage.getItem("ft.cols");
      if (s) {
        const a = JSON.parse(s);
        if (Array.isArray(a)) return new Set<ColKey>([...(a as ColKey[]), "name"]);
      }
    } catch { /* ignore */ }
    return new Set<ColKey>(["name", "type", "description", "size"]);
  }
  function loadSort(): { key: ColKey; dir: 1 | -1 } {
    try {
      const s = localStorage.getItem("ft.sort");
      if (s) {
        const o = JSON.parse(s);
        if (o && typeof o.key === "string") return { key: o.key, dir: o.dir === -1 ? -1 : 1 };
      }
    } catch { /* ignore */ }
    return { key: "name", dir: 1 };
  }

  let visibleCols = $state<Set<ColKey>>(loadCols());
  let sortKey = $state<ColKey>(loadSort().key);
  let sortDir = $state<1 | -1>(loadSort().dir);
  let colMenuOpen = $state(false);

  const visibleColDefs = $derived(COLUMNS.filter((c) => visibleCols.has(c.key)));
  const gridTemplate = $derived(visibleColDefs.map((c) => c.width).join(" ") + " 52px");

  function toggleCol(k: ColKey) {
    const s = new Set(visibleCols);
    s.has(k) ? s.delete(k) : s.add(k);
    visibleCols = s;
  }
  function setSort(k: ColKey) {
    if (sortKey === k) sortDir = sortDir === 1 ? -1 : 1;
    else {
      sortKey = k;
      sortDir = 1;
    }
  }

  $effect(() => {
    try { localStorage.setItem("ft.cols", JSON.stringify([...visibleCols])); } catch { /* ignore */ }
  });
  $effect(() => {
    try { localStorage.setItem("ft.sort", JSON.stringify({ key: sortKey, dir: sortDir })); } catch { /* ignore */ }
  });

  // ── record counts (lazy; only when the column is shown) ──────────────────────
  let recordCounts = $state<Record<string, number | null>>({});
  let countPending = false;
  $effect(() => {
    const files = ws.files;
    if (!visibleCols.has("records") || countPending) return;
    const missing = files.filter((f) => !(f.path in recordCounts)).map((f) => f.path);
    if (!missing.length) return;
    countPending = true;
    countRecords(missing)
      .then((counts) => {
        const next = { ...recordCounts };
        missing.forEach((p, i) => (next[p] = counts[i]));
        recordCounts = next;
      })
      // Surfaced rather than swallowed: the column otherwise sits at "…" forever
      // with no hint why.
      .catch((e) => (ws.error = `Could not count records: ${e}`))
      .finally(() => (countPending = false));
  });

  // ── sorting ─────────────────────────────────────────────────────────────────
  function cmp(a: FileEntry, b: FileEntry, k: ColKey): number {
    switch (k) {
      case "name": return a.name.toLowerCase().localeCompare(b.name.toLowerCase());
      case "type": return a.file_type.localeCompare(b.file_type) || a.name.localeCompare(b.name);
      case "description": return a.description.localeCompare(b.description);
      case "size": return a.size - b.size;
      case "created": return (a.created_ms ?? 0) - (b.created_ms ?? 0);
      case "modified": return (a.modified_ms ?? 0) - (b.modified_ms ?? 0);
      case "records": return (recordCounts[a.path] ?? -1) - (recordCounts[b.path] ?? -1);
      default: return 0;
    }
  }
  const sortedFiles = $derived.by(() => {
    const files = [...ws.files];
    const k = sortKey;
    const dir = sortDir;
    files.sort((a, b) => cmp(a, b, k) * dir || a.name.localeCompare(b.name));
    return files;
  });

  // ── formatting ──────────────────────────────────────────────────────────────
  function fmtSize(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    return `${(n / 1024 / 1024).toFixed(1)} MB`;
  }
  const pad = (n: number) => String(n).padStart(2, "0");
  function fmtDate(ms: number | null): string {
    if (ms == null) return "—";
    const d = new Date(ms);
    return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
  }
  function recordText(f: FileEntry): string {
    if (!(f.path in recordCounts)) return "…";
    const c = recordCounts[f.path];
    return c == null ? "—" : c.toLocaleString();
  }

  // ── open / edit / import ─────────────────────────────────────────────────────
  function activate(f: FileEntry) {
    ws.openFile(f);
  }
  // The file pending deletion, shown in ConfirmDialog. Was a raw window.confirm(),
  // which is unstyled (a destructive action looked like a benign one) and not
  // dependable in these webviews -- the same reason NameModal exists.
  let deleteFor = $state<FileEntry | null>(null);
  function remove(e: MouseEvent, f: FileEntry) {
    e.stopPropagation();
    deleteFor = f;
  }

  // ── context menus ───────────────────────────────────────────────────────────
  /// Copy, surfacing a failure. These rows are `user-select: none`, so if the
  /// clipboard write fails there is no other way to get the text out.
  async function copy(text: string) {
    if (!(await copyText(text))) ws.error = "Could not copy to the clipboard.";
  }


  // Copy Name / Copy Full Path are not conveniences: `.row` sets
  // `user-select: none` (rows are click-to-open, so selecting text in them fights
  // the click), which left filenames and paths with no way at all to be copied.
  function rowMenu(e: MouseEvent, f: FileEntry) {
    const items: CtxItem[] = [
      { label: "Open", action: () => activate(f) },
      {
        label: "Open in New Window",
        action: () => openFileWindow(f.path, f.name),
      },
      SEP,
      {
        label: "Rename…",
        action: () => {
          renameFor = f;
        },
      },
      SEP,
      { label: "Copy Name", action: () => copy(f.name) },
      { label: "Copy Full Path", action: () => copy(f.path) },
      { label: "Show in File Manager", action: () => revealInFileManager(ws.root, f.path) },
      SEP,
      {
        label: "Delete…",
        danger: true,
        action: () => {
          deleteFor = f;
        },
      },
    ];
    openContextMenu(e, items);
  }

  function blankMenu(e: MouseEvent) {
    const items: CtxItem[] = [
      {
        label: "New File…",
        disabled: !ws.currentDir,
        action: () => {
          newOpen = true;
        },
      },
      {
        label: "Import Files…",
        disabled: !ws.selectedFolder || ws.importing > 0,
        action: importViaDialog,
      },
      SEP,
      { label: "Refresh", disabled: !ws.selectedFolder, action: () => ws.refreshFolder() },
      {
        label: "Show Folder in File Manager",
        disabled: !ws.selectedFolder,
        action: () => revealInFileManager(ws.root, ws.selectedFolder!),
      },
      SEP,
      {
        label: "Columns",
        submenu: TOGGLEABLE.map((c) => ({
          label: c.label,
          checked: visibleCols.has(c.key),
          action: () => toggleCol(c.key),
        })),
      },
      {
        label: "Sort By",
        submenu: COLUMNS.map((c) => ({
          label: c.label,
          checked: sortKey === c.key,
          action: () => setSort(c.key),
        })),
      },
    ];
    openContextMenu(e, items);
  }

  const folderName = $derived(ws.selectedFolder?.split(/[\\/]/).pop() ?? "workspace");

  async function importViaDialog() {
    if (!ws.selectedFolder) return;
    try {
      const files = await pickFiles();
      if (files && files.length) await ws.importInto(files);
    } catch (e) {
      ws.error = String(e);
    }
  }

  let newOpen = $state(false);
  // File → New File… lives in the native menu but the modal lives here.
  $effect(() => {
    if (ui.newFileRequest > 0 && ws.currentDir) newOpen = true;
  });
  let renameFor = $state<FileEntry | null>(null);

  // Both throw on failure so the modal shows it inline and keeps the pasted text;
  // they close only on success.
  async function createNew(name: string, body: string) {
    const dir = ws.currentDir;
    if (!dir) return;
    const p = await createTextFile(dir, name, body);
    await ws.refreshAndOpen(p);
    newOpen = false;
  }
  function startRename(e: MouseEvent, f: FileEntry) {
    e.stopPropagation();
    renameFor = f;
  }
  async function doRename(name: string) {
    const f = renameFor;
    if (!f) return;
    await ws.renameFile(f.path, name);
    renameFor = null;
  }

  // ── Drag-and-drop import (webview event carries absolute paths) ───────────────
  let rootEl = $state<HTMLDivElement>();
  let dragActive = $state(false);
  function isOver(pos: { x: number; y: number }): boolean {
    if (!rootEl || !ws.selectedFolder) return false;
    const dpr = window.devicePixelRatio || 1;
    const r = rootEl.getBoundingClientRect();
    const x = pos.x / dpr;
    const y = pos.y / dpr;
    return x >= r.left && x <= r.right && y >= r.top && y <= r.bottom;
  }
  $effect(() => {
    // `disposed` matters: if this effect re-runs before the listen promise
    // resolves, `unlisten` is still undefined and the old listener would leak.
    let disposed = false;
    let unlisten: (() => void) | undefined;
    getCurrentWebview()
      .onDragDropEvent((e) => {
        const p = e.payload;
        if (p.type === "enter" || p.type === "over") dragActive = isOver(p.position);
        else if (p.type === "leave") dragActive = false;
        else if (p.type === "drop") {
          const over = isOver(p.position);
          dragActive = false;
          if (over && p.paths.length) void ws.importInto(p.paths);
        }
      })
      .then((u) => {
        unlisten = u;
        if (disposed) u();
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
  });
</script>

<div
  class="table"
  role="table"
  bind:this={rootEl}
  class:dragover={dragActive}
  style="--grid: {gridTemplate};"
  oncontextmenu={blankMenu}
>
  <div class="actionbar">
    <span class="ab-folder" title={ws.selectedFolder ?? ""}>{folderName}</span>
    <div class="colmenu">
      <button class="import" onclick={() => (colMenuOpen = !colMenuOpen)} title="Show/hide columns">Columns ▾</button>
      {#if colMenuOpen}
        <div class="dropdown" role="menu">
          {#each TOGGLEABLE as c (c.key)}
            <label class="ditem">
              <input type="checkbox" checked={visibleCols.has(c.key)} onchange={() => toggleCol(c.key)} />
              {c.label}
            </label>
          {/each}
        </div>
      {/if}
    </div>
    <button class="import" onclick={() => (newOpen = true)} disabled={!ws.currentDir} title="Create a new file from pasted text">
      ＋ New file…
    </button>
    <button
      class="import"
      onclick={importViaDialog}
      disabled={!ws.selectedFolder || ws.importing > 0}
      title="Copy files into this folder"
    >
      {#if ws.importing > 0}
        Importing {ws.importing} file{ws.importing === 1 ? "" : "s"}…
      {:else}
        ⬆ Import files…
      {/if}
    </button>
  </div>

  <div class="head" role="row" style="grid-template-columns: var(--grid);">
    {#each visibleColDefs as c (c.key)}
      <button
        class="colhead"
        class:right={c.align === "right"}
        role="columnheader"
        onclick={() => setSort(c.key)}
        title="Sort by {c.label}"
      >
        {c.label}{#if sortKey === c.key}<span class="arrow">{sortDir === 1 ? " ▲" : " ▼"}</span>{/if}
      </button>
    {/each}
    <div class="colhead" role="columnheader"></div>
  </div>

  {#if ws.loadingFiles}
    <div class="note">Loading…</div>
  {:else if !ws.selectedFolder}
    <div class="note">Select a folder.</div>
  {:else if ws.files.length === 0}
    <div class="note">No recognized files in this folder.</div>
  {:else}
    {#each sortedFiles as f (f.path)}
      <div
        class="row"
        class:active={ws.activeTabId === f.path}
        role="row"
        tabindex="0"
        style="grid-template-columns: var(--grid);"
        onclick={() => activate(f)}
        onkeydown={(e) => e.key === "Enter" && activate(f)}
        oncontextmenu={(e) => rowMenu(e, f)}
      >
        {#each visibleColDefs as c (c.key)}
          {#if c.key === "name"}
            <div class="cell cname" role="cell" title={f.name}>{f.name}</div>
          {:else if c.key === "type"}
            <div class="cell" role="cell"><span class="badge badge-{f.file_type}">{TYPE_LABEL[f.file_type] ?? f.file_type}</span></div>
          {:else if c.key === "description"}
            <div class="cell cdesc" role="cell" title={f.description}>{f.description}</div>
          {:else if c.key === "size"}
            <div class="cell num" role="cell">{fmtSize(f.size)}</div>
          {:else if c.key === "records"}
            <div class="cell num" role="cell">{recordText(f)}</div>
          {:else if c.key === "created"}
            <div class="cell mono dim" role="cell">{fmtDate(f.created_ms)}</div>
          {:else if c.key === "modified"}
            <div class="cell mono dim" role="cell">{fmtDate(f.modified_ms)}</div>
          {/if}
        {/each}
        <div class="cell c-act" role="cell">
          <button class="act" title="Rename file" aria-label="Rename file" onclick={(e) => startRename(e, f)}>✎</button>
          <button class="del" title="Delete file" aria-label="Delete file" onclick={(e) => remove(e, f)}>🗑</button>
        </div>
      </div>
    {/each}
  {/if}

  {#if dragActive}
    <div class="dropzone" role="presentation">
      <div class="dz-inner">Drop files to import into <b>{folderName}</b></div>
    </div>
  {/if}
</div>

{#if newOpen}
  <NameModal
    title="New file in {folderName}"
    defaultName="pasted.fasta"
    withBody={true}
    bodyPlaceholder="Paste file contents here. The extension sets the type: .fasta/.gb (sequences), .hmm, .tsv/.csv (table), .dsnv…"
    submitLabel="Create"
    validate={validateFilename}
    onsubmit={createNew}
    oncancel={() => (newOpen = false)}
  />
{/if}
{#if deleteFor}
  <ConfirmDialog
    title="Delete “{deleteFor.name}”?"
    body="This permanently removes the file from disk. It cannot be undone."
    confirmLabel="Delete"
    danger={true}
    onconfirm={async () => {
      const f = deleteFor;
      if (f) await ws.deleteFile(f.path);
      deleteFor = null;
    }}
    oncancel={() => (deleteFor = null)}
  />
{/if}
{#if renameFor}
  <NameModal
    title="Rename file"
    defaultName={renameFor.name}
    submitLabel="Rename"
    validate={validateFilename}
    onsubmit={(name) => doRename(name)}
    oncancel={() => (renameFor = null)}
  />
{/if}

<style>
  .table {
    display: flex;
    flex-direction: column;
    min-width: 0;
    font-size: 12.5px;
    position: relative;
  }
  .actionbar {
    position: sticky;
    top: 0;
    z-index: 3;
    display: flex;
    align-items: center;
    gap: 8px;
    height: 32px;
    padding: 0 12px;
    background: var(--bg-panel);
    border-bottom: 1px solid var(--border);
  }
  .ab-folder {
    font-weight: 600;
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
  }
  .import {
    border: 1px solid var(--border);
    background: var(--bg-elevated);
    color: var(--text);
    border-radius: 4px;
    padding: 3px 9px;
    font-size: 12px;
    cursor: pointer;
    white-space: nowrap;
  }
  .import:hover:not(:disabled) {
    background: var(--row-hover);
    border-color: var(--accent);
  }
  .import:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .colmenu {
    position: relative;
  }
  .dropdown {
    position: absolute;
    top: 100%;
    left: 0;
    margin-top: 4px;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: 6px;
    box-shadow: 0 6px 20px rgba(0, 0, 0, 0.3);
    z-index: 20;
    min-width: 150px;
    padding: 4px;
  }
  .ditem {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px;
    font-size: 12px;
    border-radius: 4px;
    cursor: pointer;
    user-select: none;
  }
  .ditem:hover {
    background: var(--row-hover);
  }
  .head,
  .row {
    display: grid;
    align-items: center;
    gap: 8px;
    padding: 0 12px;
  }
  .head {
    position: sticky;
    top: 32px;
    height: 28px;
    background: var(--bg-panel);
    border-bottom: 1px solid var(--border);
    color: var(--text-dim);
    font-weight: 600;
    z-index: 2;
  }
  .colhead {
    border: none;
    background: none;
    color: var(--text-dim);
    font: inherit;
    font-weight: 600;
    text-align: left;
    padding: 0;
    cursor: pointer;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .colhead.right {
    text-align: right;
  }
  .colhead:hover {
    color: var(--text);
    background: none;
  }
  .arrow {
    color: var(--accent);
  }
  .row {
    height: 26px;
    cursor: default;
    user-select: none;
    border-bottom: 1px solid transparent;
  }
  .row:hover {
    background: var(--row-hover);
  }
  .row.active {
    background: var(--row-selected);
  }
  .cell {
    overflow: hidden;
  }
  .cname,
  .cdesc {
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .num {
    text-align: right;
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .mono {
    font-family: var(--mono);
    white-space: nowrap;
  }
  .dim {
    color: var(--text-dim);
  }
  .c-act {
    text-align: right;
    display: flex;
    justify-content: flex-end;
    gap: 2px;
    overflow: visible;
  }
  .del,
  .act {
    border: none;
    background: none;
    cursor: pointer;
    opacity: 0;
    font-size: 12px;
    padding: 2px;
    line-height: 1;
  }
  .row:hover .del,
  .row:hover .act {
    opacity: 0.6;
  }
  .del:hover,
  .act:hover {
    opacity: 1;
  }
  .badge {
    display: inline-block;
    padding: 1px 7px;
    border-radius: 10px;
    font-size: 11px;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    color: var(--text-dim);
  }
  .badge-genbank {
    color: #06842b;
    border-color: #06842b55;
  }
  .badge-fasta {
    color: #2f6feb;
    border-color: #2f6feb55;
  }
  .badge-ssn_viewer {
    color: #b5179e;
    border-color: #b5179e55;
  }
  .note {
    padding: 16px 12px;
    color: var(--text-dim);
  }
  .dropzone {
    position: absolute;
    inset: 32px 0 0 0;
    z-index: 5;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(59, 130, 246, 0.12);
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    border: 2px dashed var(--accent);
    border-radius: 6px;
    pointer-events: none;
  }
  .dz-inner {
    padding: 10px 18px;
    border-radius: 6px;
    background: var(--bg-elevated);
    border: 1px solid var(--accent);
    color: var(--text);
    font-size: 13px;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.25);
  }
</style>
