<script lang="ts">
  import { ws } from "./lib/state.svelte";
  import {
    forgetRecentWorkspace,
    listTools,
    pickDirectory,
    startupInfo,
  } from "./lib/api";
  import FolderTree from "./lib/FolderTree.svelte";
  import FileTable from "./lib/FileTable.svelte";
  import ViewerDock from "./lib/ViewerDock.svelte";
  import ToolDialog from "./lib/ToolDialog.svelte";
  import DatabasesPanel from "./lib/DatabasesPanel.svelte";
  import JobsPanel from "./lib/JobsPanel.svelte";
  import ExternalToolsPanel from "./lib/ExternalToolsPanel.svelte";
  import ReferenceDbsPanel from "./lib/ReferenceDbsPanel.svelte";
  import ContigView from "./lib/viewers/ContigView.svelte";
  import HmmLogo from "./lib/viewers/HmmLogo.svelte";
  import FileWindow from "./lib/FileWindow.svelte";
  import AboutDialog from "./lib/AboutDialog.svelte";
  import PreferencesDialog from "./lib/PreferencesDialog.svelte";
  import ExportDialog from "./lib/ExportDialog.svelte";
  import ConfirmDialog from "./lib/ConfirmDialog.svelte";
  import ContextMenu from "./lib/ContextMenu.svelte";
  import StatusBar from "./lib/StatusBar.svelte";
  import ToolsButton from "./lib/ToolsButton.svelte";
  import { ui } from "./lib/ui.svelte";
  import { installMenuListener } from "./lib/menu";
  import { setMenuState, getPrefs, checkForUpdates } from "./lib/api";
  import { applyTheme } from "./lib/theme";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { standaloneParams } from "./lib/api";
  import type { FileEntry, JobInfo, ToolMeta } from "./lib/types";

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
            ws.selectedFolders = [dir];
            ws.selectedFolder = dir;
          }
          standalonePath = p[0];
        } else {
          standaloneErr = "window parameters not found";
        }
      })
      .catch((e) => (standaloneErr = String(e)));
  });

  // Load the tool list ONCE at startup -- before any workspace/GenBank viewer is
  // open, so the main thread is idle and the fetch can't stall. Each tool's fields
  // carry their manifest parameters, so opening a tool dialog then needs no backend
  // call, which is what previously left dialogs stuck at "Loading…" while a large
  // GenBank contig view was rendering.
  $effect(() => {
    if (isStandalone) return;
    listTools()
      .then((t) => {
        tools = t;
      })
      // Without this the Tools menu just renders empty and the rejection only
      // reaches the console.
      .catch((e) => (ws.error = `Could not load the tool list: ${e}`));
  });

  // The shared database list: independent of any workspace, so it is loaded
  // once at launch rather than on open (which only folds in a legacy list).
  $effect(() => {
    if (isStandalone) return;
    void ws.initDatabases();
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
  let activeTool = $state<string | null>(null);
  let toolInitial = $state<Record<string, unknown> | null>(null);
  let toolFiles = $state<FileEntry[]>([]);
  let aboutOpen = $state(false);
  let prefsOpen = $state(false);
  let pendingClose = $state<string[] | null>(null);
  let updateBanner = $state<string | null>(null);
  let newFolderOpen = $state(false);
  let newFolderName = $state("");

  // Open the tool dialog on request (a menu → the files to place; job "clone" → params).
  $effect(() => {
    const req = ws.requestedTool;
    if (!req) return;
    activeTool = req.id;
    toolInitial = req.initial ?? null;
    toolFiles = req.files ?? [];
    ws.requestedTool = null;
  });

  // Same channel for the folder tree's "New Subfolder…" context item: the inline
  // form lives here, so the tree asks rather than rendering its own.
  $effect(() => {
    if (ws.requestNewFolder === null) return;
    newFolderOpen = true;
    newFolderName = "";
    ws.requestNewFolder = null;
  });

  // Native-menu commands. Registered in detached windows too: on macOS they share
  // the app menu outright, so the dispatcher has to refuse shell-only commands
  // rather than half-apply them (`menu.ts`'s SHELL_ONLY).
  $effect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    installMenuListener(isStandalone).then((u) => {
      unlisten = u;
      if (disposed) u();
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  });

  // Menu requests that this component owns.
  $effect(() => {
    if (ui.aboutRequest > 0) aboutOpen = true;
  });
  $effect(() => {
    if (ui.preferencesRequest > 0) prefsOpen = true;
  });

  // Optional launch-time update check. Off by default, throttled by the stored
  // last-checked stamp, and completely silent on failure -- being offline must
  // never look like an app problem. Main window only.
  $effect(() => {
    if (isStandalone) return;
    getPrefs()
      .then(async (p) => {
        if (!p.check_updates_on_launch) return;
        const DAY = 24 * 60 * 60 * 1000;
        if (p.last_update_check_ms && Date.now() - p.last_update_check_ms < DAY) return;
        const res = await checkForUpdates();
        if (res.newer && res.latest && res.latest.tag !== p.last_seen_release) {
          updateBanner = res.latest.tag;
        }
      })
      .catch(() => {
        /* silent by design */
      });
  });

  // Theme, applied as early as possible. `app.css` already renders the right
  // colors for a "system" user via prefers-color-scheme, so this only matters
  // for an explicit light/dark choice -- and runs in detached windows too, which
  // load the same entry point.
  $effect(() => {
    getPrefs()
      .then((p) => applyTheme(p.theme))
      .catch(() => {
        /* defaults to the OS preference; not worth surfacing */
      });
  });

  // Window title: "file — workspace — Domainatoredux", with a bullet when the active
  // buffer is unsaved. Needs `core:window:allow-set-title`.
  $effect(() => {
    const tab = ws.tabs.find((t) => t.id === ws.activeTabId);
    const parts: string[] = [];
    if (tab) parts.push((ws.dirty[tab.id] ? "• " : "") + tab.file.name);
    if (ws.root) parts.push(shortName(ws.root));
    parts.push("Domainatoredux");
    getCurrentWindow()
      .setTitle(parts.join(" — "))
      .catch(() => {
        /* a title is cosmetic; never surface a failure */
      });
  });

  // Guard unsaved editor buffers on window close.
  //
  // NOTE: registering this handler is what makes `core:window:allow-destroy`
  // mandatory -- the JS helper calls destroy() itself whenever the handler does
  // not preventDefault(), so without that permission even a clean close throws
  // and the window never closes.
  $effect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    getCurrentWindow()
      .onCloseRequested((e) => {
        const dirty = ws.dirtyPaths;
        if (dirty.length === 0) return; // falls through; the helper destroys it
        e.preventDefault();
        pendingClose = dirty;
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


  // Publish what the menu needs to gray items out. Diffed before sending so a
  // pointer move that touches the store does not spam IPC; only the main window
  // publishes, since a detached window's "is a tab open" is not the shell's.
  let lastMenuState = "";
  $effect(() => {
    if (isStandalone) return;
    const state = {
      has_workspace: !!ws.root,
      has_folder: !!ws.selectedFolder,
      has_tab: !!ws.activeTabId,
      // File → Save writes the active tab's editor only.
      can_save: !!ws.activeTabId && !!ws.dirty[ws.activeTabId],
      has_closed_tab: ws.closedTabs.length > 0,
      // File → Export… acts on the file table's selection.
      has_selection: ws.selectedFiles.length > 0,
    };
    const json = JSON.stringify(state);
    if (json === lastMenuState) return;
    lastMenuState = json;
    setMenuState(state).catch(() => {
      /* the menu just keeps its previous enabled state; not worth an error bar */
    });
  });

  // Cross-window refresh: when any window creates a file (e.g. Extract from a
  // popped-out window), refresh the browser if the changed dir is a listed folder.
  $effect(() => {
    if (isStandalone) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    listen<string>("files-changed", (e) => {
      if (ws.selectedFolders.includes(e.payload)) ws.refreshFolder();
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
  let mainEl = $state<HTMLElement>();
  const dragTable = startDrag((e) => {
    // Measured from the pane itself, so a toolbar or error banner above it is
    // accounted for; the 124px left over matches the CSS cap on the row.
    const r = mainEl?.getBoundingClientRect();
    if (!r) return;
    tableHeight = Math.min(Math.max(120, e.clientY - r.top), r.height - 124);
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
    <ContextMenu />
    {#if pendingClose}
      <ConfirmDialog
        title="Close with unsaved changes?"
        body={`These files have unsaved edits:\n\n${pendingClose.map((p) => p.split(/[\\/]/).pop()).join("\n")}`}
        confirmLabel="Save and close"
        altLabel="Discard and close"
        onconfirm={async () => {
          await ws.saveAllEditors();
          ws.dirty = {};
          pendingClose = null;
          await getCurrentWindow().destroy();
        }}
        onalt={async () => {
          ws.dirty = {};
          pendingClose = null;
          await getCurrentWindow().destroy();
        }}
        oncancel={() => (pendingClose = null)}
      />
    {/if}
    <!-- Closing a tab with an unsaved buffer. The editor is about to be
         unmounted, so this is the last chance to keep the edits. -->
    {#if ws.pendingDirty}
      {@const pd = ws.pendingDirty}
      <ConfirmDialog
        title="Save changes to {pd.path.split(/[\\/]/).pop()}?"
        body="This file has unsaved edits. Closing it without saving discards them."
        confirmLabel="Save"
        altLabel="Discard"
        onconfirm={async () => {
          await ws.saveEditor(pd.path);
          ws.pendingDirty = null;
          pd.proceed();
        }}
        onalt={() => {
          ws.clearDirty(pd.path);
          ws.pendingDirty = null;
          pd.proceed();
        }}
        oncancel={() => (ws.pendingDirty = null)}
      />
    {/if}
  </div>
{:else}
<div
  class="shell"
  style="--left-width: {ui.sidebarVisible ? leftWidth : 0}px; --table-height: {ui.tableVisible ? tableHeight : 0}px;"
>
  <header class="toolbar">
    <div class="tb-left">
      <span class="brand">Domainatoredux</span>
      <ToolsButton {tools} />
      <button onclick={chooseWorkspace} disabled={opening}>
        {opening ? "Opening…" : "Open workspace…"}
      </button>
      <span class="root-path" title={ws.root ?? ""}>{ws.root ?? "No workspace open"}</span>
    </div>
    <div class="tb-right">
      <button class="about" title="Version and diagnostics" onclick={() => (aboutOpen = true)}>About</button>
    </div>
  </header>

  {#if updateBanner}
    <div class="updatebar" role="status">
      <span>Version {updateBanner} is available.</span>
      <button onclick={() => { updateBanner = null; prefsOpen = true; ui.updateCheckRequest++; }}>
        Details
      </button>
      <button class="errclose" onclick={() => (updateBanner = null)} aria-label="Dismiss">×</button>
    </div>
  {/if}

  {#if ws.error}
    <!-- Selectable (so the text can be copied into a bug report) and dismissible;
         it used to be a squeezed toolbar <span> that never cleared. -->
    <div class="errbar" role="alert">
      <span class="errtext">{ws.error}</span>
      <button class="errclose" onclick={() => (ws.error = null)} aria-label="Dismiss">×</button>
    </div>
  {/if}

  {#if aboutOpen}
    <AboutDialog onclose={() => (aboutOpen = false)} />
  {/if}

  {#if prefsOpen}
    <PreferencesDialog onclose={() => (prefsOpen = false)} />
  {/if}

  {#if ui.exportFor}
    <ExportDialog paths={ui.exportFor} onclose={() => (ui.exportFor = null)} />
  {/if}

  {#if activeTool}
    <ToolDialog
      tool={activeTool}
      meta={tools.find((m) => m.id === activeTool) ?? null}
      initial={toolInitial}
      files={toolFiles}
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
    <ReferenceDbsPanel />
    <ExternalToolsPanel />
  </aside>

  <div class="left-resizer" onpointerdown={dragLeft} role="separator" aria-label="Resize sidebar" tabindex="-1"></div>

  <section class="main" bind:this={mainEl}>
    <div class="table-pane">
      <FileTable {tools} />
    </div>
    <div class="row-resizer" onpointerdown={dragTable} role="separator" aria-label="Resize viewer" tabindex="-1"></div>
    <div class="viewer-pane">
      <ViewerDock />
    </div>
  </section>

  <StatusBar />

  <ContextMenu />

  {#if pendingClose}
    <ConfirmDialog
      title="Close with unsaved changes?"
      body={`These files have unsaved edits:\n\n${pendingClose.map((p) => p.split(/[\\/]/).pop()).join("\n")}`}
      confirmLabel="Save and close"
      altLabel="Discard and close"
      onconfirm={async () => {
        await ws.saveAllEditors();
        ws.dirty = {};
        pendingClose = null;
        await getCurrentWindow().destroy();
      }}
      onalt={async () => {
        ws.dirty = {};
        pendingClose = null;
        await getCurrentWindow().destroy();
      }}
      oncancel={() => (pendingClose = null)}
    />
  {/if}
  <!-- Closing a tab with an unsaved buffer. The editor is about to be
       unmounted, so this is the last chance to keep the edits. -->
  {#if ws.pendingDirty}
    {@const pd = ws.pendingDirty}
    <ConfirmDialog
      title="Save changes to {pd.path.split(/[\\/]/).pop()}?"
      body="This file has unsaved edits. Closing it without saving discards them."
      confirmLabel="Save"
      altLabel="Discard"
      onconfirm={async () => {
        await ws.saveEditor(pd.path);
        ws.pendingDirty = null;
        pd.proceed();
      }}
      onalt={() => {
        ws.clearDirty(pd.path);
        ws.pendingDirty = null;
        pd.proceed();
      }}
      oncancel={() => (ws.pendingDirty = null)}
    />
  {/if}
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
    grid-template-rows: 44px auto 1fr 22px;
    grid-template-areas:
      "toolbar toolbar toolbar"
      "errbar  errbar  errbar"
      "left    lres    main"
      "status  status  status";
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
  .tb-left {
    display: flex;
    align-items: center;
    gap: 12px;
    min-width: 0;
    flex: 1;
  }
  .tb-left > :not(.root-path) {
    flex: none;
  }
  .tb-right {
    display: flex;
    justify-content: flex-end;
  }
  .brand {
    font-weight: 700;
    letter-spacing: 0.02em;
  }
  .about {
    flex: none;
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
  .updatebar {
    grid-area: errbar;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 12px;
    background: var(--accent-soft);
    border-bottom: 1px solid var(--border);
    font-size: 12px;
  }
  .updatebar .errclose {
    margin-left: auto;
    color: var(--text-dim);
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
  /* The panels under the folder tree size themselves (each has a drag grip);
     the tree takes what is left, down to a floor, and past that the sidebar
     scrolls as a whole rather than clipping a panel. */
  .left {
    grid-area: left;
    background: var(--bg-panel);
    overflow-x: hidden;
    overflow-y: auto;
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
    min-height: 80px;
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
    /* The stored table height is capped by the space there is now, so the viewer
       always keeps at least 120px. Without the cap a height dragged in a taller
       window (another monitor, or before an error banner appeared) left the viewer
       a zero-height row, and opening a file seemed to do nothing. */
    grid-template-rows: min(var(--table-height), max(0px, calc(100% - 124px))) 4px 1fr;
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
