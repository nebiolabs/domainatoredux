<script lang="ts">
  import { ws } from "./lib/state.svelte";
  import { listTools, pickDirectory, toolSchemaCached } from "./lib/api";
  import FolderTree from "./lib/FolderTree.svelte";
  import FileTable from "./lib/FileTable.svelte";
  import ViewerDock from "./lib/ViewerDock.svelte";
  import ToolDialog from "./lib/ToolDialog.svelte";
  import DatabasesPanel from "./lib/DatabasesPanel.svelte";
  import JobsPanel from "./lib/JobsPanel.svelte";
  import ExternalToolsPanel from "./lib/ExternalToolsPanel.svelte";
  import ContigView from "./lib/viewers/ContigView.svelte";
  import HmmLogo from "./lib/viewers/HmmLogo.svelte";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { standaloneParams } from "./lib/api";
  import type { JobInfo, ToolMeta } from "./lib/types";

  // Detached window mode: a window opened with `?view=contig` (a single sequence)
  // or `?view=hmmlogo` (a single popped-out HMM profile logo) bypasses the shell.
  const standaloneView = new URLSearchParams(window.location.search).get("view");
  const isStandalone = standaloneView === "contig" || standaloneView === "hmmlogo";
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
    listTools().then((t) => {
      tools = t;
      for (const m of t) void toolSchemaCached(m.id);
    });
  });

  // Live job status from the backend JobRegistry (main window only).
  $effect(() => {
    if (isStandalone) return;
    let unlisten: (() => void) | undefined;
    listen<JobInfo>("job-update", (e) => ws.upsertJob(e.payload)).then((u) => (unlisten = u));
    ws.loadJobs();
    return () => unlisten?.();
  });

  let leftWidth = $state(260);
  let tableHeight = $state(240);
  let tools = $state<ToolMeta[]>([]);
  let toolsOpen = $state(false);
  let activeTool = $state<string | null>(null);
  let newFolderOpen = $state(false);
  let newFolderName = $state("");

  async function submitNewFolder() {
    const parent = ws.selectedFolder ?? ws.root;
    if (parent && newFolderName.trim()) {
      await ws.createSubfolder(parent, newFolderName.trim());
    }
    newFolderName = "";
    newFolderOpen = false;
  }

  async function chooseWorkspace() {
    const dir = await pickDirectory();
    if (dir) await ws.open(dir);
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
    {#if standalonePath}
      {#key standalonePath}
        {#if standaloneView === "hmmlogo"}
          <HmmLogo path={standalonePath} index={standaloneIndex} />
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
{:else}
<div class="shell" style="--left-width: {leftWidth}px; --table-height: {tableHeight}px;">
  <header class="toolbar">
    <span class="brand">Domainator</span>
    <button onclick={chooseWorkspace}>Open workspace…</button>
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
                toolsOpen = false;
              }}
              title={t.description}>{t.label}</button
            >
          {/each}
        </div>
      {/if}
    </div>
    <span class="root-path" title={ws.root ?? ""}>{ws.root ?? "No workspace open"}</span>
    {#if ws.error}
      <span class="err">{ws.error}</span>
    {/if}
  </header>

  {#if activeTool}
    <ToolDialog
      tool={activeTool}
      meta={tools.find((m) => m.id === activeTool) ?? null}
      onclose={() => (activeTool = null)}
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
  .standalone {
    height: 100%;
    min-height: 0;
  }
  .shell {
    display: grid;
    grid-template-columns: var(--left-width) 4px 1fr;
    grid-template-rows: 44px 1fr;
    grid-template-areas:
      "toolbar toolbar toolbar"
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
  .err {
    color: #e5484d;
    font-size: 12px;
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
