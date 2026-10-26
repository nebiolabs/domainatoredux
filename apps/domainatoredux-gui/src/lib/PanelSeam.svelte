<script lang="ts">
  // The draggable seam on a sidebar panel's top border. Dragging it moves the
  // boundary between this panel and the one above it (or the folder tree, when
  // the panel above is collapsed or there is none): up grows this panel and
  // shrinks that one. Double-click returns this panel to its automatic size.
  // Inactive while the panel is collapsed, since there is no body to size.
  import {
    MIN_PANEL_HEIGHT as MIN,
    neighborAbove,
    panelBody,
    panelHeights,
    persistHeight,
    type PanelKey,
  } from "./sidebarPanels.svelte";

  let { key, active }: { key: PanelKey; active: boolean } = $props();

  function startDrag(e: PointerEvent) {
    const body = panelBody(key);
    if (!active || !body) return;
    e.preventDefault();
    const above = neighborAbove(key);
    const aboveBody = above ? panelBody(above) : undefined;
    const startY = e.clientY;
    const startH = body.getBoundingClientRect().height;
    const startAbove = aboveBody?.getBoundingClientRect().height ?? 0;
    // Against a panel, the two trade height and neither goes below MIN. Against
    // the folder tree, the tree keeps its own floor and the sidebar scrolls
    // past that, so only a window-sized cap applies.
    const maxGrow = above ? startAbove - MIN : Math.max(0, window.innerHeight - 160 - startH);
    const move = (ev: PointerEvent) => {
      const d = Math.round(Math.min(Math.max(startY - ev.clientY, MIN - startH), maxGrow));
      panelHeights[key] = startH + d;
      if (above) panelHeights[above] = startAbove - d;
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      persistHeight(key);
      if (above) persistHeight(above);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }

  function reset() {
    if (!active) return;
    panelHeights[key] = null;
    persistHeight(key);
  }
</script>

<div class="seam-anchor">
  <div
    class="seam"
    class:active
    role="separator"
    aria-orientation="horizontal"
    aria-label="Resize panel"
    title={active ? "Drag to resize · double-click to fit the contents" : undefined}
    tabindex="-1"
    onpointerdown={startDrag}
    ondblclick={reset}
  ></div>
</div>

<style>
  /* Zero-height, so the seam overlays the panel's top border instead of adding
     a row of its own. */
  .seam-anchor {
    position: relative;
    height: 0;
  }
  .seam {
    position: absolute;
    left: 0;
    right: 0;
    top: -4px;
    height: 7px;
    z-index: 2;
  }
  .seam.active {
    cursor: row-resize;
  }
  .seam.active:hover {
    background: var(--accent);
    opacity: 0.5;
  }
</style>
