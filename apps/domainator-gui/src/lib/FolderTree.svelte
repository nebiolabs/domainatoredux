<script lang="ts">
  import { untrack } from "svelte";
  import { ws } from "./state.svelte";
  import type { FolderNode } from "./types";
  import Self from "./FolderTree.svelte";

  let { node, depth }: { node: FolderNode; depth: number } = $props();
  // Auto-expand the top level once; `depth` is fixed per node instance.
  let expanded = $state(untrack(() => depth < 1));

  const selected = $derived(ws.selectedFolder === node.path);

  function select() {
    ws.selectFolder(node.path);
    if (node.children.length) expanded = true;
  }
</script>

<div
  class="row"
  class:selected
  style="padding-left: {depth * 14 + 8}px"
  onclick={select}
  role="treeitem"
  aria-selected={selected}
  aria-expanded={node.children.length ? expanded : undefined}
  tabindex="0"
  onkeydown={(e) => e.key === "Enter" && select()}
>
  {#if node.children.length}
    <button
      class="twisty"
      onclick={(e) => {
        e.stopPropagation();
        expanded = !expanded;
      }}
      aria-label={expanded ? "Collapse" : "Expand"}>{expanded ? "▾" : "▸"}</button
    >
  {:else}
    <span class="twisty spacer"></span>
  {/if}
  <span class="icon">🗀</span>
  <span class="name" title={node.name}>{node.name}</span>
</div>

{#if expanded}
  {#each node.children as child (child.path)}
    <Self node={child} depth={depth + 1} />
  {/each}
{/if}

<style>
  .row {
    display: flex;
    align-items: center;
    gap: 4px;
    height: 24px;
    padding-right: 8px;
    cursor: default;
    user-select: none;
    white-space: nowrap;
  }
  .row:hover {
    background: var(--row-hover);
  }
  .row.selected {
    background: var(--row-selected);
  }
  .twisty {
    width: 16px;
    height: 16px;
    padding: 0;
    border: none;
    background: none;
    color: var(--text-dim);
    font-size: 10px;
    line-height: 1;
  }
  .twisty:hover {
    background: none;
    color: var(--text);
  }
  .spacer {
    display: inline-block;
  }
  .icon {
    font-size: 12px;
    opacity: 0.8;
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
