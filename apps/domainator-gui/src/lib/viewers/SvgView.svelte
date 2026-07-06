<script lang="ts">
  import { readTextFile } from "../api";
  import { cached } from "../cache";

  let { path }: { path: string } = $props();

  let raw = $state<string | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(true);
  let zoom = $state(1);

  // Inline the SVG markup directly into the DOM instead of pointing an <img> or
  // <iframe> at a file:// URL. Browsers treat every file:// document as its own
  // opaque origin, so an SVG loaded that way is blocked from resolving anything
  // (even itself) — the "unsafe attempt to load URL file://… 'file:' URLs are
  // treated as unique security origins" error. Inlining sidesteps that entirely.
  $effect(() => {
    void path;
    loading = true;
    error = null;
    raw = null;
    zoom = 1;
    cached(`svgtext:${path}`, () => readTextFile(path))
      .then((t) => (raw = t))
      .catch((e) => (error = String(e)))
      .finally(() => (loading = false));
  });

  // Strip an XML prolog / DOCTYPE — those are processing instructions that don't
  // belong when the markup is injected into an HTML container.
  const markup = $derived(
    (raw ?? "").replace(/^﻿?\s*<\?xml[^>]*\?>\s*/i, "").replace(/^\s*<!DOCTYPE[^>]*>\s*/i, ""),
  );
</script>

<div class="svgview">
  <div class="bar">
    <button onclick={() => (zoom = Math.max(0.1, zoom / 1.25))} title="Zoom out">−</button>
    <span class="pct">{Math.round(zoom * 100)}%</span>
    <button onclick={() => (zoom = Math.min(16, zoom * 1.25))} title="Zoom in">＋</button>
    <button onclick={() => (zoom = 1)} title="Reset zoom">Reset</button>
  </div>
  {#if loading}
    <div class="msg">Loading…</div>
  {:else if error}
    <div class="msg err">{error}</div>
  {:else}
    <div class="canvas">
      <div class="scale" style="transform: scale({zoom}); transform-origin: top left;">
        <!-- eslint-disable-next-line svelte/no-at-html-tags -->
        {@html markup}
      </div>
    </div>
  {/if}
</div>

<style>
  .svgview {
    height: 100%;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    background: var(--bg-panel);
    border-bottom: 1px solid var(--border);
    font-size: 12.5px;
    flex: none;
  }
  .pct {
    min-width: 44px;
    text-align: center;
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .canvas {
    flex: 1;
    min-height: 0;
    overflow: auto;
    background: #ffffff;
    /* light checkerboard so transparent SVGs are legible in dark mode too */
    background-image:
      linear-gradient(45deg, #eee 25%, transparent 25%),
      linear-gradient(-45deg, #eee 25%, transparent 25%),
      linear-gradient(45deg, transparent 75%, #eee 75%),
      linear-gradient(-45deg, transparent 75%, #eee 75%);
    background-size: 20px 20px;
    background-position: 0 0, 0 10px, 10px -10px, -10px 0;
    padding: 12px;
  }
  .scale :global(svg) {
    display: block;
  }
  .msg {
    padding: 24px;
    color: var(--text-dim);
  }
  .msg.err {
    color: #e5484d;
  }
</style>
