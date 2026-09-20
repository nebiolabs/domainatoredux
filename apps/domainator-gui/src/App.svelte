<script lang="ts">
  import { ws } from "./lib/state.svelte";
  import {
    forgetRecentWorkspace,
    listTools,
    pickDirectory,
    startupInfo,
    toolSchemaCached,
  } from "./lib/api";
  import FolderTree from "./lib/FolderTree.svelte";
  import FileTable from "./lib/FileTable.svelte";
  import ViewerDock from "./lib/ViewerDock.svelte";
  import ToolDialog from "./lib/ToolDialog.svelte";
  import DatabasesPanel from "./lib/DatabasesPanel.svelte";
  import JobsPanel from "./lib/JobsPanel.svelte";
  import ExternalToolsPanel from "./lib/ExternalToolsPanel.svelte";
  import ContigView from "./lib/viewers/ContigView.svelte";
  import HmmLogo from "./lib/viewers/HmmLogo.svelte";
  import FileWindow from "./lib/FileWindow.svelte";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { standaloneParams } from "./lib/api";
  import type { JobInfo, ToolMeta } from "./lib/types";

  // Detached window mode: `?view=contig` (a single sequence), `?view=hmmlogo` (one
  // popped-out HMM logo), or `?view=file` (a whole file with its viewer sub-tabs)
  // bypasses the shell.
  const standaloneView = new URLSearchParams(window.location.search).get("view");
  const isStandalone =
    standaloneView === "contig" || standaloneView === "hmmlogo" || standaloneView === "file";
  let standalonePath = $state<string | null>(null);
  let standaloneIndex = $state(0);
  let standaloneErr = $state<string | null>(null);

  $effect(() => {
    if (!isStandalone) return;
    standaloneParams(getCurrentWindow().label)
      .then((p) => {
        if (p) {
          standaloneIndex = p[1];
          if (standaloneView === "contig") ws.focusContig[p[0]] = p[1];
          // Give the detached window a workspace folder (the file's parent) so its
          // in-viewer Extract / New-file actions have a destination.
          const dir = p[0].replace(/[\\/][^\\/]*$/, "");
          if (dir && dir !== p[0]) {
            ws.root = dir;
            ws.selectedFolder = dir;
          }
          standalonePath = p[0];
        } else {
          standaloneErr = "window parameters not found";
        }
      })
      .catch((e) => (standaloneErr = String(e)));
  });

  // Load the tool list and prefetch every tool's JSON Schema ONCE at startup —
  // before any workspace/GenBank viewer is open, so the main thread is idle and
  // the fetches can't stall. Opening a tool dialog then needs no backend call,
  // which is what previously left dialogs stuck at "Loading…" while a large
  // GenBank contig view was rendering.
  $effect(() => {
    if (isStandalone) return;
    listTools()
      .then((t) => {
        tools = t;
        for (const m of t) void toolSchemaCached(m.id);
      })
      // Without this the Tools menu just renders empty and the rejection only
      // reaches the console.
      .catch((e) => (ws.error = `Could not load the tool list: ${e}`));
  });

  // Recent workspaces, and the one reopened at launch. The app used to start cold
  // every time, with no memory of where you were working.
  let recent = $state<string[]>([]);
  let restoring = $state(true);

  $effect(() => {
    if (isStandalone) {
      restoring = false;
      return;
    }
    startupInfo()
      .then(async (info) => {
        recent = info.recent;
        if (info.reopen) await ws.open(info.reopen);
      })
      .catch((e) => (ws.error = String(e)))
      .finally(() => (restoring = false));
  });

  /// Open `dir` and move it to the front of the recent list.
  async function openWorkspace(dir: string): Promise<void> {
    await ws.open(dir);
    recent = [dir, ...recent.filter((r) => r !== dir)];
  }

  async function forgetRecent(dir: string): Promise<void> {
    try {
      recent = await forgetRecentWorkspace(dir);
    } catch (e) {
      ws.error = String(e);
    }
  }

  const shortName = (p: string) => p.split(/[\\/]/).filter(Boolean).pop() ?? p;

  // Live job status from the backend JobRegistry (main window only).
  $effect(() => {
    if (isStandalone) return;
    // `disposed`: this effect re-runs when the workspace changes (ws.loadJobs
    // reads ws.root), and without it a re-run before the listen promise resolves
    // leaks the old handler — every job update would then be applied twice.
    let disposed = false;
    let unlisten: (() => void) | undefined;
    listen<JobInfo>("job-update", (e) => ws.upsertJob(e.payload)).then((u) => {
      unlisten = u;
      if (disposed) u();
    });
    ws.loadJobs();
    return () => {
      disposed = true;
      unlisten?.();
    };
  });

  // Panel sizes persist per machine, like the file table's columns and sort.
  // Reads are guarded: storage can be unavailable or hold junk.
  function storedNumber(key: string, fallback: number): number {
    try {
      const v = Number(localStorage.getItem(key));
      return Number.isFinite(v) && v > 0 ? v : fallback;
    } catch {
      return fallback;
    }
  }
  let leftWidth = $state(storedNumber("shell.leftWidth", 260));
  let tableHeight = $state(storedNumber("shell.tableHeight", 240));
  $effect(() => {
    try {
      localStorage.setItem("shell.leftWidth", String(leftWidth));
      localStorage.setItem("shell.tableHeight", String(tableHeight));
    } catch {
      /* storage unavailable (private window, blocked site data) — sizes just
         won't persist; nothing else depends on it */
    }
  });
  let tools = $state<ToolMeta[]>([]);
  let toolsOpen = $state(false);
  let activeTool = $state<string | null>(null);
  let toolInitial = $state<Record<string, unknown> | null>(null);
  let newFolderOpen = $state(false);
  let newFolderName = $state("");

  // Open the tool dialog on request (Tools menu → no initial; job "clone" → params).
  $effect(() => {
    const req = ws.requestedTool;
    if (!req) return;
    activeTool = req.id;
    toolInitial = req.initial ?? null;
    ws.requestedTool = null;
  });

  // Cross-window refresh: when any window creates a file (e.g. Extract from a
  // popped-out window), refresh the browser if the changed dir is the open folder.
  $effect(() => {
    if (isStandalone) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    listen<string>("files-changed", (e) => {
      if (e.payload === ws.selectedFolder) ws.refreshFolder();
    }).then((u) => {
      unlisten = u;
      if (disposed) u();
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  });

  async function submitNewFolder() {
    const parent = ws.selectedFolder ?? ws.root;
    if (parent && newFolderName.trim()) {
      await ws.createSubfolder(parent, newFolderName.trim());
    }
    newFolderName = "";
    newFolderOpen = false;
  }

  let opening = $state(false);
  async function chooseWorkspace() {
    if (opening) return;
    opening = true;
    try {
      const dir = await pickDirectory();
      if (dir) await openWorkspace(dir);
    } catch (e) {
      ws.error = String(e);
    } finally {
      opening = false;
    }
  }

  async function toggleTools() {
    toolsOpen = !toolsOpen;
    if (toolsOpen && tools.length === 0) tools = await listTools();
  }

  // Generic drag helper: while the pointer is down, feed clientX/clientY to `apply`.
  function startDrag(apply: (e: PointerEvent) => void) {
    return (e: PointerEvent) => {
      e.preventDefault();
      const move = (ev: PointerEvent) => apply(ev);
      const up = () => {
        window.removeEventListener("pointermove", move);
        window.removeEventListener("pointerup", up);
      };
      window.addEventListener("pointermove", move);
      window.addEventListener("pointerup", up);
    };
  }

  const dragLeft = startDrag((e) => {
    leftWidth = Math.min(Math.max(160, e.clientX), 560);
  });
  const dragTable = startDrag((e) => {
    // 44px accounts for the toolbar height.
    tableHeight = Math.min(Math.max(120, e.clientY - 44), window.innerHeight - 160);
  });
</script>

{#if isStandalone}
  <div class="standalone">
    <div class="standalone-body">
      {#if standalonePath}
        {#key standalonePath}
          {#if standaloneView === "hmmlogo"}
            <HmmLogo path={standalonePath} index={standaloneIndex} />
          {:else if standaloneView === "file"}
            <FileWindow path={standalonePath} />
          {:else}
            <ContigView path={standalonePath} />
          {/if}
        {/key}
      {:else if standaloneErr}
        <p class="empty">{standaloneErr}</p>
      {:else}
        <p class="empty">Loading…</p>
      {/if}
    </div>
    {#if ws.error}
      <!-- Detached windows had no error surface at all, so a failed extract or
           rename in one was completely invisible. -->
      <div class="errbar standalone-err" role="alert">
        <span class="errtext">{ws.error}</span>
        <button class="errclose" onclick={() => (ws.error = null)} aria-label="Dismiss">×</button>
      </div>
    {/if}
  </div>
{:else}
<div class="shell" style="--left-width: {leftWidth}px; --table-height: {tableHeight}px;">
  <header class="toolbar">
    <span class="brand">Domainator</span>
    <button onclick={chooseWorkspace} disabled={opening}>
      {opening ? "Opening…" : "Open workspace…"}
    </button>
    <div class="menu">
      <button onclick={toggleTools} disabled={!ws.root}>Tools ▾</button>
      {#if toolsOpen}
        <div class="dropdown" role="menu">
          {#each tools as t (t.id)}
            <button
              class="menu-item"
              role="menuitem"
              onclick={() => {
                activeTool = t.id;
                toolInitial = null;
                toolsOpen = false;
              }}
              title={t.description}>{t.label}</button
            >
          {/each}
        </div>
      {/if}
    </div>
    <span class="root-path" title={ws.root ?? ""}>{ws.root ?? "No workspace open"}</span>
  </header>

  {#if ws.error}
    <!-- Selectable (so the text can be copied into a bug report) and dismissible;
         it used to be a squeezed toolbar <span> that never cleared. -->
    <div class="errbar" role="alert">
      <span class="errtext">{ws.error}</span>
      <button class="errclose" onclick={() => (ws.error = null)} aria-label="Dismiss">×</button>
    </div>
  {/if}

  {#if activeTool}
    <ToolDialog
      tool={activeTool}
      meta={tools.find((m) => m.id === activeTool) ?? null}
      initial={toolInitial}
      onclose={() => {
        activeTool = null;
        toolInitial = null;
      }}
    />
  {/if}

  <aside class="left">
    {#if ws.tree}
      <div class="left-header">
        <span class="lh-title">Folders</span>
        <button class="lh-add" title="New subfolder" onclick={() => (newFolderOpen = !newFolderOpen)}>＋</button>
      </div>
      {#if newFolderOpen}
        <div class="newfolder">
          <input
            type="text"
            placeholder="subfolder in {ws.selectedFolder?.split(/[\\/]/).pop() ?? 'workspace'}"
            bind:value={newFolderName}
            onkeydown={(e) => {
              if (e.key === "Enter") submitNewFolder();
              if (e.key === "Escape") {
                newFolderOpen = false;
                newFolderName = "";
              }
            }}
          />
          <button onclick={submitNewFolder}>Create</button>
        </div>
      {/if}
    {/if}
    <div class="tree-scroll">
      {#if ws.tree}
        <FolderTree node={ws.tree} depth={0} />
      {:else if restoring}
        <p class="empty">Restoring your last workspace…</p>
      {:else if recent.length > 0}
        <div class="recent">
          <p class="empty">Open a workspace directory to browse its folders.</p>
          <div class="recent-title">Recent</div>
          {#each recent as r (r)}
            <div class="recent-row">
              <button class="recent-open" title={r} onclick={() => openWorkspace(r)}>
                <span class="recent-name">{shortName(r)}</span>
                <span class="recent-path">{r}</span>
              </button>
              <button
                class="recent-forget"
                title="Remove from this list"
                aria-label="Remove from recent list"
                onclick={() => forgetRecent(r)}>×</button
              >
            </div>
          {/each}
        </div>
      {:else}
        <p class="empty">Open a workspace directory to browse its folders.</p>
      {/if}
    </div>
    <JobsPanel />
    <DatabasesPanel />
    <ExternalToolsPanel />
  </aside>

  <div class="left-resizer" onpointerdown={dragLeft} role="separator" aria-label="Resize sidebar" tabindex="-1"></div>

  <section class="main">
    <div class="table-pane">
      <FileTable />
    </div>
    <div class="row-resizer" onpointerdown={dragTable} role="separator" aria-label="Resize viewer" tabindex="-1"></div>
    <div class="viewer-pane">
      <ViewerDock />
    </div>
  </section>
</div>
{/if}

<style>
  .standalone-err {
    flex: none;
  }
  .standalone {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .standalone-body {
    flex: 1;
    min-height: 0;
  }
  .shell {
    display: grid;
    grid-template-columns: var(--left-width) 4px 1fr;
    /* The error row is `auto`, so it takes no space at all when there is no error. */
    grid-template-rows: 44px auto 1fr;
    grid-template-areas:
      "toolbar toolbar toolbar"
      "errbar  errbar  errbar"
      "left    lres    main";
    height: 100%;
  }
  .toolbar {
    grid-area: toolbar;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 12px;
    background: var(--bg-panel);
    border-bottom: 1px solid var(--border);
  }
  .brand {
    font-weight: 700;
    letter-spacing: 0.02em;
  }
  .menu {
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
    z-index: 60;
    min-width: 180px;
    padding: 4px;
  }
  .menu-item {
    display: block;
    width: 100%;
    text-align: left;
    border: none;
    background: none;
    padding: 6px 10px;
    border-radius: 4px;
  }
  .menu-item:hover {
    background: var(--row-hover);
  }
  .root-path {
    color: var(--text-dim);
    font-family: var(--mono);
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
  }
  .errbar {
    grid-area: errbar;
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 6px 12px;
    background: color-mix(in srgb, #e5484d 12%, var(--bg-panel));
    border-bottom: 1px solid color-mix(in srgb, #e5484d 40%, var(--border));
    font-size: 12px;
  }
  .errtext {
    color: #e5484d;
    flex: 1;
    /* Selectable and wrapping: the whole point is being able to read and copy it. */
    user-select: text;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    max-height: 6.5em;
    overflow-y: auto;
  }
  .errclose {
    border: none;
    background: none;
    color: #e5484d;
    cursor: pointer;
    font-size: 16px;
    line-height: 1;
    padding: 0 2px;
    flex: none;
  }
  .recent {
    padding-bottom: 8px;
  }
  .recent-title {
    padding: 4px 10px;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
  }
  .recent-row {
    display: flex;
    align-items: center;
  }
  .recent-row:hover {
    background: var(--row-hover);
  }
  .recent-open {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 1px;
    border: none;
    background: none;
    color: var(--text);
    text-align: left;
    padding: 4px 10px;
    cursor: pointer;
    border-radius: 0;
  }
  .recent-name {
    font-size: 12px;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .recent-path {
    font-size: 10.5px;
    color: var(--text-dim);
    direction: rtl;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .recent-forget {
    border: none;
    background: none;
    color: var(--text-dim);
    cursor: pointer;
    font-size: 14px;
    line-height: 1;
    padding: 0 8px;
    opacity: 0;
  }
  .recent-row:hover .recent-forget {
    opacity: 0.7;
  }
  .recent-forget:hover {
    color: #e5484d;
    opacity: 1;
  }
  .left {
    grid-area: left;
    background: var(--bg-panel);
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }
  .left-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 4px 8px;
    border-bottom: 1px solid var(--border);
  }
  .lh-title {
    font-weight: 600;
    font-size: 12px;
    color: var(--text-dim);
  }
  .lh-add {
    border: none;
    background: none;
    color: var(--accent);
    font-size: 15px;
    line-height: 1;
    padding: 0 4px;
  }
  .lh-add:hover {
    background: none;
  }
  .newfolder {
    display: flex;
    gap: 4px;
    padding: 4px 8px;
    border-bottom: 1px solid var(--border);
  }
  .newfolder input {
    flex: 1;
    min-width: 0;
    padding: 3px 5px;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg-elevated);
    color: var(--text);
    font-size: 12px;
  }
  .tree-scroll {
    flex: 1;
    overflow: auto;
    padding: 6px 0;
    min-height: 0;
  }
  .empty {
    color: var(--text-dim);
    padding: 12px;
    line-height: 1.5;
  }
  .left-resizer {
    grid-area: lres;
    cursor: col-resize;
    background: var(--border);
  }
  .left-resizer:hover {
    background: var(--accent);
  }
  .main {
    grid-area: main;
    display: grid;
    grid-template-rows: var(--table-height) 4px 1fr;
    min-width: 0;
    min-height: 0;
  }
  .table-pane {
    overflow: auto;
    min-height: 0;
  }
  .row-resizer {
    cursor: row-resize;
    background: var(--border);
  }
  .row-resizer:hover {
    background: var(--accent);
  }
  .viewer-pane {
    overflow: hidden;
    min-height: 0;
    background: var(--bg);
  }
</style>
