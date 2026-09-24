<script lang="ts">
  import ResizableBody from "./ResizableBody.svelte";
  import { downloadExternalTool, listExternalTools } from "./api";
  import type { ExternalToolStatus } from "./types";
  import { ui } from "./ui.svelte";
  import { flashPanel } from "./panelFlash";

  let expanded = $state(false);

  // Tools menu → expand, outline for a few seconds, and clear the request.
  let panelEl = $state<HTMLDivElement>();
  $effect(() => {
    if (ui.focusPanel === "external") {
      expanded = true;
      ui.focusPanel = null;
      flashPanel(panelEl);
    }
  });
  let tools = $state<ExternalToolStatus[]>([]);
  let busy = $state<Record<string, boolean>>({});
  let errors = $state<Record<string, string>>({});
  let listError = $state<string | null>(null);
  let loading = $state(true);

  async function refresh() {
    try {
      tools = await listExternalTools();
      listError = null;
    } catch (e) {
      // Swallowing this made the panel report "0" tools as though none existed.
      listError = String(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    refresh();
  });

  async function download(id: string) {
    busy[id] = true;
    delete errors[id];
    try {
      await downloadExternalTool(id);
      await refresh();
    } catch (e) {
      errors[id] = String(e);
    } finally {
      busy[id] = false;
    }
  }

  const installedCount = $derived(tools.filter((t) => t.installed).length);
</script>

<div class="ext" bind:this={panelEl}>
  <div class="header">
    <button class="twisty" onclick={() => (expanded = !expanded)} aria-label="Toggle external tools">
      {expanded ? "▾" : "▸"}
    </button>
    <span class="title">External tools</span>
    <span class="count">{installedCount}/{tools.length}</span>
  </div>
  {#if expanded}
    <ResizableBody key="external" defaultMax={220}>
    {#if loading}
      <div class="empty">Checking…</div>
    {:else if listError}
      <div class="err">{listError}</div>
    {/if}
    {#each tools as t (t.id)}
      <div class="tool">
        <div class="row1">
          <span class="name" title={t.description}>{t.id} <span class="ver">{t.version}</span></span>
          {#if t.installed}
            <span class="ok" title={t.path ?? ""}>installed</span>
          {:else if !t.available}
            <span class="na">n/a on {t.platform}</span>
          {:else}
            <button class="dl" disabled={busy[t.id]} onclick={() => download(t.id)}>
              {busy[t.id] ? "downloading…" : "Download"}
            </button>
          {/if}
        </div>
        {#if errors[t.id]}<div class="err">{errors[t.id]}</div>{/if}
      </div>
    {/each}
    {#if tools.length === 0 && !loading && !listError}
      <div class="empty">No managed external tools.</div>
    {/if}
    </ResizableBody>
  {/if}
</div>

<style>
  .ext {
    border-top: 1px solid var(--border);
    font-size: 12.5px;
    flex: none;
  }
  .header {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 4px 8px;
    background: var(--bg-panel);
  }
  .twisty {
    border: none;
    background: none;
    color: var(--text-dim);
    padding: 0 2px;
    font-size: 10px;
  }
  .twisty:hover {
    background: none;
  }
  .title {
    font-weight: 600;
  }
  .count {
    color: var(--text-dim);
    margin-left: auto;
  }
  .tool {
    padding: 4px 8px 4px 12px;
    border-bottom: 1px solid var(--border);
  }
  .row1 {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .name {
    font-family: var(--mono);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ver {
    color: var(--text-dim);
    font-size: 11px;
  }
  .ok {
    margin-left: auto;
    color: #06842b;
    font-size: 11px;
  }
  .na {
    margin-left: auto;
    color: var(--text-dim);
    font-size: 11px;
  }
  .dl {
    margin-left: auto;
    padding: 1px 8px;
    font-size: 11px;
  }
  .err {
    color: #e5484d;
    font-size: 11px;
    margin-top: 2px;
  }
  .empty {
    padding: 6px 12px;
    color: var(--text-dim);
  }
</style>
