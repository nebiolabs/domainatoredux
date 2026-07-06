<script lang="ts">
  // A single popped-out file: the same sub-view tabs as ViewerDock, but for one
  // file in its own window (no tab strip). The file type is classified from the
  // path, so the detached window needs nothing but the path.
  import { viewsFor } from "./viewers/registry";
  import { fileTypeOf } from "./filetypes";

  let { path }: { path: string } = $props();

  const fileType = $derived(fileTypeOf(path));
  const views = $derived(fileType ? viewsFor(fileType) : []);
  let activeId = $state("");
  const currentId = $derived(activeId || views[0]?.id || "");
  const current = $derived(views.find((v) => v.id === currentId) ?? views[0] ?? null);
</script>

<div class="fw">
  {#if views.length > 1}
    <div class="subtabs" role="tablist">
      {#each views as v (v.id)}
        <button
          class="subtab"
          class:active={v.id === currentId}
          role="tab"
          aria-selected={v.id === currentId}
          onclick={() => (activeId = v.id)}
        >
          {v.label}
        </button>
      {/each}
    </div>
  {/if}
  <div class="body">
    {#if current}
      {@const View = current.component}
      {#key currentId}
        <View {path} />
      {/key}
    {:else}
      <div class="placeholder">No viewer for this file type.</div>
    {/if}
  </div>
</div>

<style>
  .fw {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .subtabs {
    display: flex;
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
  .body {
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }
  .placeholder {
    padding: 24px;
    color: var(--text-dim);
  }
</style>
