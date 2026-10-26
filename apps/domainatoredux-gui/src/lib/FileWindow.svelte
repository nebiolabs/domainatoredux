<script lang="ts">
  // A single popped-out file: the same sub-view tabs as ViewerDock, but for one
  // file in its own window (no tab strip). The file type is classified from the
  // path, so the detached window needs nothing but the path.
  import { viewsFor } from "./viewers/registry";
  import { fileTypeOf } from "./filetypes";
  import { ws } from "./state.svelte";

  let { path }: { path: string } = $props();

  const fileType = $derived(fileTypeOf(path));
  const views = $derived(fileType ? viewsFor(fileType) : []);
  // The active sub-view lives in `ws.subActive`, as in ViewerDock, so a view can
  // switch to a sibling (Enum Report → Contig View) with `ws.showSubView`.
  const currentId = $derived(ws.subActive[path] || views[0]?.id || "");
  const current = $derived(views.find((v) => v.id === currentId) ?? views[0] ?? null);

  // Keep-alive, as in ViewerDock: a sub-view stays mounted once shown, so
  // switching between them keeps each one's zoom, scroll and selection.
  let visited = $state<string[]>([]);
  $effect(() => {
    if (current && !visited.includes(current.id)) visited.push(current.id);
  });
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
          onclick={() => (ws.subActive[path] = v.id)}
        >
          {v.label}
        </button>
      {/each}
    </div>
  {/if}
  <div class="body">
    {#each visited as id (id)}
      {@const v = views.find((x) => x.id === id)}
      {#if v}
        {@const View = v.component}
        <div class="pane" class:hidden={id !== current?.id} inert={id !== current?.id}>
          <View {path} active={id === current?.id} />
        </div>
      {/if}
    {/each}
    {#if !current}
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
    position: relative;
  }
  /* visibility, not display: none — see ViewerDock. */
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
