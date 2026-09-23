<script lang="ts">
  // One honest home for "what is the app doing?".
  //
  // Before this, `ws.importing`, `ws.busy` and `ws.loadingFiles` were each
  // rendered in a different panel or not at all, and the workspace path was an
  // ellipsized span squeezed into the toolbar. Reads only existing store fields —
  // no new backend.
  import { ws } from "./state.svelte";

  const running = $derived(
    ws.jobs.filter((j) => j.status === "running" || j.status === "canceling").length,
  );
  const dirty = $derived(ws.dirtyPaths.length);

  const activity = $derived.by(() => {
    if (ws.importing > 0) return `Importing ${ws.importing} file${ws.importing === 1 ? "" : "s"}…`;
    if (ws.loadingFiles) return "Loading folder…";
    if (ws.busy) return "Working…";
    if (running > 0) return `${running} job${running === 1 ? "" : "s"} running`;
    return null;
  });
</script>

<div class="status">
  <span class="root" title={ws.root ?? ""}>{ws.root ?? "No workspace open"}</span>
  {#if ws.selectedFolder}
    <span class="sep">·</span>
    <span>{ws.files.length} file{ws.files.length === 1 ? "" : "s"}</span>
  {/if}
  {#if dirty > 0}
    <span class="sep">·</span>
    <span class="dirty" title="Unsaved editor buffers">
      {dirty} unsaved
    </span>
  {/if}
  <span class="spacer"></span>
  {#if activity}<span class="activity">{activity}</span>{/if}
</div>

<style>
  .status {
    grid-area: status;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 12px;
    background: var(--bg-panel);
    border-top: 1px solid var(--border);
    font-size: 11.5px;
    color: var(--text-dim);
    overflow: hidden;
    white-space: nowrap;
  }
  .root {
    font-family: var(--mono);
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 55%;
    /* The path is the thing people copy into a terminal. */
    user-select: text;
  }
  .sep {
    opacity: 0.5;
  }
  .dirty {
    color: var(--accent);
  }
  .spacer {
    flex: 1;
  }
  .activity {
    color: var(--text);
  }
</style>
