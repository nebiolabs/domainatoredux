<script lang="ts">
  // The body of a sidebar panel (Jobs, Databases, Reference databases, External
  // tools): scrolls when its content spills over. Until resized it is as tall as
  // its content, up to `defaultMax`. The seam above the panel (`PanelSeam`) sets
  // its height; see `sidebarPanels.svelte.ts`.
  import type { Snippet } from "svelte";
  import { panelHeights, registerBody, type PanelKey } from "./sidebarPanels.svelte";

  let {
    key,
    defaultMax = 220,
    children,
  }: { key: PanelKey; defaultMax?: number; children: Snippet } = $props();

  let el = $state<HTMLDivElement>();
  $effect(() => {
    if (el) return registerBody(key, el);
  });

  const height = $derived(panelHeights[key]);
</script>

<div
  class="rb"
  bind:this={el}
  style={height !== null ? `height: ${height}px` : `max-height: ${defaultMax}px`}
>
  {@render children()}
</div>

<style>
  .rb {
    overflow-y: auto;
    overflow-x: hidden;
  }
</style>
