<script lang="ts">
  import { ws } from "./state.svelte";
  import { openFileWindow } from "./api";
  import { viewsFor } from "./viewers/registry";
  import type { OpenTab } from "./types";
  import { openContextMenu, SEP, type CtxItem } from "./ctxmenu.svelte";
  import { copyText } from "./clipboard";

  // A new window takes a visible moment to appear; without this the ⧉ click
  // looked like it did nothing.
  let popping = $state(false);
  async function popOut(tab: OpenTab) {
    if (popping) return;
    popping = true;
    try {
      await openFileWindow(tab.file.path, tab.file.name);
    } catch (e) {
      ws.error = String(e);
    } finally {
      popping = false;
    }
  }

  function tabMenu(e: MouseEvent, tab: OpenTab) {
    const items: CtxItem[] = [
      { label: "Activate", action: () => ws.setActive(tab.id) },
      { label: "Open in New Window", action: () => popOut(tab) },
      SEP,
      { label: "Close", action: () => ws.closeTab(tab.id) },
      {
        label: "Close Others",
        disabled: ws.tabs.length < 2,
        action: () => ws.closeOtherTabs(tab.id),
      },
      { label: "Close All", action: () => ws.closeAllTabs() },
      {
        label: "Reopen Closed Tab",
        disabled: ws.closedTabs.length === 0,
        action: () => ws.reopenClosedTab(),
      },
      SEP,
      {
        label: "Copy Full Path",
        action: async () => {
          if (!(await copyText(tab.file.path))) ws.error = "Could not copy to the clipboard.";
        },
      },
    ];
    openContextMenu(e, items);
  }

  const active = $derived(ws.tabs.find((t) => t.id === ws.activeTabId) ?? null);
  const views = $derived(active ? viewsFor(active.file.file_type) : []);

  // The selected sub-view per open file lives on the store, so a viewer can
  // switch the dock to a sibling tab (HmmReport's "logo" button → the Logo view).
  const currentViewId = $derived(active ? (ws.subActive[active.id] ?? views[0]?.id ?? "") : "");
  const currentView = $derived(views.find((v) => v.id === currentViewId) ?? views[0] ?? null);
  const activeKey = $derived(active && currentView ? paneKey(active.id, currentView.id) : null);

  // Keep-alive: every view that has been shown stays mounted until its tab
  // closes, so zoom, scroll, selection and filters survive switching tabs (and
  // the SSN iframe does not reload and re-run its layout). Views mount lazily,
  // on first show, so opening many tabs does not load many files.
  //
  // `visited` is in first-shown order and is what the panes are rendered from,
  // never `ws.tabs`: a keyed each moves DOM nodes when its order changes, and
  // moving an iframe reloads it.
  type Pane = { key: string; tabId: string; viewId: string };
  let visited = $state<Pane[]>([]);

  function paneKey(tabId: string, viewId: string): string {
    return tabId + "\u0000" + viewId;
  }

  $effect(() => {
    const key = activeKey;
    if (key && active && currentView && !visited.some((p) => p.key === key)) {
      visited.push({ key, tabId: active.id, viewId: currentView.id });
    }
  });

  // A closed tab's panes are dropped, which destroys their viewers: reopening
  // the file starts from a fresh view.
  $effect(() => {
    const open = new Set(ws.tabs.map((t) => t.id));
    if (visited.some((p) => !open.has(p.tabId))) {
      visited = visited.filter((p) => open.has(p.tabId));
    }
  });

  function paneView(p: Pane) {
    const tab = ws.tabs.find((t) => t.id === p.tabId);
    const view = tab ? viewsFor(tab.file.file_type).find((v) => v.id === p.viewId) : undefined;
    return tab && view ? { path: tab.file.path, component: view.component } : null;
  }
</script>

<div class="dock">
  <div class="tabstrip" role="tablist">
    {#each ws.tabs as tab (tab.id)}
      <div
        class="tab"
        class:active={tab.id === ws.activeTabId}
        role="tab"
        aria-selected={tab.id === ws.activeTabId}
        oncontextmenu={(e) => tabMenu(e, tab)}
        tabindex="-1"
      >
        <button class="tab-label" onclick={() => ws.setActive(tab.id)} title={tab.file.path}>
          {tab.file.name}
        </button>
        <button class="tab-pop" onclick={() => popOut(tab)} disabled={popping} title="Open in a new window" aria-label="Open in new window">⧉</button>
        <button class="tab-close" onclick={() => ws.closeTab(tab.id)} aria-label="Close">×</button>
      </div>
    {/each}
  </div>

  {#if active && views.length > 1}
    <div class="subtabs" role="tablist">
      {#each views as v (v.id)}
        <button
          class="subtab"
          class:active={v.id === currentViewId}
          role="tab"
          aria-selected={v.id === currentViewId}
          onclick={() => ws.showSubView(active.id, v.id)}
        >
          {v.label}
        </button>
      {/each}
    </div>
  {/if}

  <div class="tab-body">
    {#each visited as p (p.key)}
      {@const pv = paneView(p)}
      {#if pv}
        {@const View = pv.component}
        <div class="pane" class:hidden={p.key !== activeKey} inert={p.key !== activeKey}>
          <!-- A file rewritten on disk (a job output, a rename) remounts its view. -->
          {#key ws.fileRev[pv.path] ?? 0}
            <View path={pv.path} active={p.key === activeKey} />
          {/key}
        </div>
      {/if}
    {/each}
    {#if !active}
      <div class="placeholder">Open a file to view it here.</div>
    {:else if !currentView}
      <div class="placeholder">
        No viewer yet for <b>{active.file.file_type}</b> files. (Coming in a later phase.)
      </div>
    {/if}
  </div>
</div>

<style>
  .dock {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .tabstrip {
    display: flex;
    gap: 2px;
    background: var(--bg-panel);
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
    min-height: 30px;
  }
  .tab {
    display: flex;
    align-items: center;
    border-right: 1px solid var(--border);
    background: var(--bg-panel);
    max-width: 220px;
  }
  .tab.active {
    background: var(--bg);
  }
  .tab-label {
    border: none;
    background: none;
    padding: 6px 8px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tab-label:hover {
    background: none;
  }
  .tab.active .tab-label {
    font-weight: 600;
  }
  .tab-pop {
    border: none;
    background: none;
    color: var(--text-dim);
    padding: 2px 4px;
    font-size: 12px;
    line-height: 1;
  }
  .tab-pop:hover {
    background: none;
    color: var(--accent);
  }
  .tab-close {
    border: none;
    background: none;
    color: var(--text-dim);
    padding: 2px 6px;
    font-size: 14px;
    line-height: 1;
  }
  .tab-close:hover {
    background: none;
    color: #e5484d;
  }
  .subtabs {
    display: flex;
    gap: 0;
    background: var(--bg);
    border-bottom: 1px solid var(--border);
  }
  .subtab {
    border: none;
    border-bottom: 2px solid transparent;
    border-radius: 0;
    background: none;
    color: var(--text-dim);
    padding: 5px 12px;
  }
  .subtab:hover {
    background: var(--row-hover);
  }
  .subtab.active {
    color: var(--text);
    border-bottom-color: var(--accent);
  }
  .tab-body {
    flex: 1;
    min-height: 0;
    overflow: hidden;
    position: relative;
  }
  /* Hidden panes use visibility, not display: none, so they keep their size and
     scroll offsets — a 0×0 box would re-clamp the canvases and the virtualized
     tables, and drop native scroll positions. */
  .pane {
    position: absolute;
    inset: 0;
  }
  .pane.hidden {
    visibility: hidden;
    pointer-events: none;
  }
  .placeholder {
    padding: 24px;
    color: var(--text-dim);
  }
</style>
