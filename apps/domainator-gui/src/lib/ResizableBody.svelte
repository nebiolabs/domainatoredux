<script lang="ts">
  // The body of a sidebar panel (Jobs, Databases, Reference databases, External
  // tools): scrolls when its content spills over, with a grip underneath to drag
  // its height. Until dragged it is as tall as its content, up to `defaultMax`;
  // a dragged height sticks per panel (localStorage, like the other panel sizes)
  // and double-clicking the grip returns to that automatic size.
  import type { Snippet } from "svelte";

  let {
    key,
    defaultMax = 220,
    children,
  }: { key: string; defaultMax?: number; children: Snippet } = $props();

  const MIN = 40;
  // svelte-ignore state_referenced_locally
  const storageKey = `sidebar.height.${key}`;

  function stored(): number | null {
    try {
      const v = Number(localStorage.getItem(storageKey));
      return Number.isFinite(v) && v >= MIN ? v : null;
    } catch {
      return null;
    }
  }

  let height = $state<number | null>(stored());
  let el = $state<HTMLDivElement>();

  function startDrag(e: PointerEvent) {
    if (!el) return;
    e.preventDefault();
    const startY = e.clientY;
    const startH = el.getBoundingClientRect().height;
    const max = Math.max(MIN, window.innerHeight - 160);
    const move = (ev: PointerEvent) => {
      height = Math.round(Math.min(Math.max(MIN, startH + ev.clientY - startY), max));
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      try {
        if (height !== null) localStorage.setItem(storageKey, String(height));
      } catch {
        /* sizes are a convenience; storage can be unavailable */
      }
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }

  function reset() {
    height = null;
    try {
      localStorage.removeItem(storageKey);
    } catch {
      /* see above */
    }
  }
</script>

<div
  class="rb"
  bind:this={el}
  style={height !== null ? `height: ${height}px` : `max-height: ${defaultMax}px`}
>
  {@render children()}
</div>
<div
  class="grip"
  role="separator"
  aria-orientation="horizontal"
  aria-label="Resize panel"
  title="Drag to resize · double-click to fit the contents"
  tabindex="-1"
  onpointerdown={startDrag}
  ondblclick={reset}
></div>

<style>
  .rb {
    overflow-y: auto;
    overflow-x: hidden;
  }
  .grip {
    height: 5px;
    cursor: row-resize;
    border-top: 1px solid transparent;
  }
  .grip:hover {
    background: var(--accent);
    opacity: 0.5;
  }
</style>
