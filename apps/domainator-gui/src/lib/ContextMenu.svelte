<script lang="ts">
  // The single context-menu instance. Mounted once by App.svelte (in both the
  // shell and the detached-window branch), so there is one keyboard model, one
  // z-index, and no per-open mount cost.
  import { ctx, closeContextMenu, type CtxItem } from "./ctxmenu.svelte";
  import { ws } from "./state.svelte";

  let menuEl = $state<HTMLDivElement>();
  let subEl = $state<HTMLDivElement>();
  // Clamped position, computed after mount when the size is known.
  let pos = $state({ x: 0, y: 0 });
  let focused = $state(-1);
  let openSub = $state(-1);
  let subFocused = $state(-1);
  let subPos = $state({ x: 0, y: 0 });

  const items = $derived(ctx.items);
  const subItems = $derived(openSub >= 0 ? (items[openSub]?.submenu ?? []) : []);

  const selectable = (list: CtxItem[], i: number) =>
    !!list[i]?.label && !list[i]?.disabled;

  /// Seed the position from the click, then clamp/flip once we can measure.
  $effect(() => {
    if (!ctx.open) return;
    pos = { x: ctx.x, y: ctx.y };
    focused = -1;
    openSub = -1;
    // Measure after paint: getBoundingClientRect is only meaningful once mounted.
    requestAnimationFrame(() => {
      if (!menuEl) return;
      const r = menuEl.getBoundingClientRect();
      let x = ctx.x;
      let y = ctx.y;
      if (x + r.width > window.innerWidth) x = ctx.x - r.width;
      if (y + r.height > window.innerHeight) y = ctx.y - r.height;
      pos = { x: Math.max(4, x), y: Math.max(4, y) };
      menuEl.focus();
    });
  });

  /// Position a submenu beside its parent row, flipping left near the edge.
  function placeSub(i: number, rowEl: HTMLElement) {
    const r = rowEl.getBoundingClientRect();
    const m = menuEl?.getBoundingClientRect();
    subPos = { x: (m?.right ?? r.right) - 2, y: r.top - 4 };
    openSub = i;
    subFocused = -1;
    requestAnimationFrame(() => {
      if (!subEl) return;
      const s = subEl.getBoundingClientRect();
      let x = subPos.x;
      let y = subPos.y;
      if (x + s.width > window.innerWidth) x = (m?.left ?? r.left) - s.width + 2;
      if (y + s.height > window.innerHeight) y = window.innerHeight - s.height - 4;
      subPos = { x: Math.max(4, x), y: Math.max(4, y) };
    });
  }

  /// Run an item's action. Errors surface on the shared error bar rather than
  /// vanishing into the console — a context-menu action that silently fails is
  /// indistinguishable from one that did nothing.
  async function activate(item: CtxItem) {
    if (!item.label || item.disabled) return;
    if (item.submenu) return; // opening is handled by hover/ArrowRight
    closeContextMenu();
    try {
      await item.action?.();
    } catch (e) {
      ws.error = String(e);
    }
  }

  /// Move `focused` (or `subFocused`) to the next selectable row, wrapping.
  function step(list: CtxItem[], from: number, dir: 1 | -1): number {
    if (list.length === 0) return -1;
    let i = from;
    for (let n = 0; n < list.length; n++) {
      i = (i + dir + list.length) % list.length;
      if (selectable(list, i)) return i;
    }
    return -1;
  }

  function onKeydown(e: KeyboardEvent) {
    if (!ctx.open) return;
    const inSub = openSub >= 0;
    const list = inSub ? subItems : items;
    const cur = inSub ? subFocused : focused;
    const set = (v: number) => (inSub ? (subFocused = v) : (focused = v));

    switch (e.key) {
      case "Escape":
        e.preventDefault();
        if (inSub) openSub = -1;
        else closeContextMenu();
        return;
      case "ArrowDown":
        e.preventDefault();
        set(step(list, cur, 1));
        return;
      case "ArrowUp":
        e.preventDefault();
        set(step(list, cur, -1));
        return;
      case "Home":
        e.preventDefault();
        set(step(list, -1, 1));
        return;
      case "End":
        e.preventDefault();
        set(step(list, list.length, -1));
        return;
      case "ArrowRight": {
        if (inSub) return;
        const it = items[focused];
        if (it?.submenu && !it.disabled) {
          e.preventDefault();
          const row = menuEl?.querySelector<HTMLElement>(`[data-i="${focused}"]`);
          if (row) placeSub(focused, row);
          subFocused = step(it.submenu, -1, 1);
        }
        return;
      }
      case "ArrowLeft":
        if (inSub) {
          e.preventDefault();
          openSub = -1;
        }
        return;
      case "Enter":
      case " ":
        e.preventDefault();
        if (cur >= 0) void activate(list[cur]);
        return;
    }
  }

  // Dismissal. Registered only while open, and torn down together.
  //
  // `scroll` is capture-phase so a scroll inside a virtualized list (EnumReport,
  // HmmReport) counts too — those scroll a nested container, not the window, and
  // a menu left floating over moved content points at the wrong row.
  $effect(() => {
    if (!ctx.open) return;
    const onPointerDown = (e: PointerEvent) => {
      const t = e.target as Node | null;
      if (t && (menuEl?.contains(t) || subEl?.contains(t))) return;
      closeContextMenu();
    };
    const onScroll = () => closeContextMenu();
    const onBlur = () => closeContextMenu();
    window.addEventListener("pointerdown", onPointerDown, true);
    window.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", onScroll);
    window.addEventListener("blur", onBlur);
    return () => {
      window.removeEventListener("pointerdown", onPointerDown, true);
      window.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", onScroll);
      window.removeEventListener("blur", onBlur);
    };
  });
</script>

<svelte:window onkeydown={onKeydown} />

{#if ctx.open}
  <div
    class="menu"
    role="menu"
    tabindex="-1"
    bind:this={menuEl}
    style="left: {pos.x}px; top: {pos.y}px;"
  >
    {#each items as item, i (i)}
      {#if !item.label}
        <div class="sep" role="separator"></div>
      {:else if item.submenu}
        <div
          class="item has-sub"
          class:focused={focused === i}
          class:disabled={item.disabled}
          role="menuitem"
          tabindex="-1"
          aria-disabled={item.disabled}
          aria-haspopup="menu"
          aria-expanded={openSub === i}
          data-i={i}
          onmouseenter={(e) => {
            focused = i;
            if (!item.disabled) placeSub(i, e.currentTarget as HTMLElement);
          }}
        >
          <span class="label">{item.label}</span>
          <span class="arrow">▸</span>
        </div>
      {:else}
        <div
          class="item"
          class:focused={focused === i}
          class:disabled={item.disabled}
          class:danger={item.danger}
          role={item.checked === undefined ? "menuitem" : "menuitemcheckbox"}
          aria-checked={item.checked === undefined ? undefined : item.checked}
          tabindex="-1"
          aria-disabled={item.disabled}
          data-i={i}
          onmouseenter={() => {
            focused = i;
            openSub = -1;
          }}
          onclick={() => activate(item)}
        >
          <span class="check">{item.checked ? "✓" : ""}</span>
          <span class="label">{item.label}</span>
          {#if item.accel}<span class="accel">{item.accel}</span>{/if}
        </div>
      {/if}
    {/each}
  </div>

  {#if openSub >= 0 && subItems.length > 0}
    <div
      class="menu sub"
      role="menu"
      tabindex="-1"
      bind:this={subEl}
      style="left: {subPos.x}px; top: {subPos.y}px;"
    >
      {#each subItems as item, j (j)}
        {#if !item.label}
          <div class="sep" role="separator"></div>
        {:else}
          <div
            class="item"
            class:focused={subFocused === j}
            class:disabled={item.disabled}
            class:danger={item.danger}
            role={item.checked === undefined ? "menuitem" : "menuitemcheckbox"}
            aria-checked={item.checked === undefined ? undefined : item.checked}
            tabindex="-1"
            aria-disabled={item.disabled}
            onmouseenter={() => (subFocused = j)}
            onclick={() => activate(item)}
          >
            <span class="check">{item.checked ? "✓" : ""}</span>
            <span class="label">{item.label}</span>
            {#if item.accel}<span class="accel">{item.accel}</span>{/if}
          </div>
        {/if}
      {/each}
    </div>
  {/if}
{/if}

<style>
  .menu {
    position: fixed;
    z-index: 200;
    min-width: 180px;
    max-width: 320px;
    padding: 4px;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: 6px;
    box-shadow: 0 6px 20px rgba(0, 0, 0, 0.3);
    font-size: 12.5px;
    outline: none;
    user-select: none;
  }
  .item {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px 4px 4px;
    border-radius: 4px;
    cursor: pointer;
    white-space: nowrap;
  }
  .item.focused {
    background: var(--row-hover);
  }
  .item.disabled {
    opacity: 0.45;
    cursor: default;
  }
  .item.disabled.focused {
    background: none;
  }
  .item.danger .label {
    color: #e5484d;
  }
  .check {
    width: 1em;
    flex: none;
    color: var(--accent);
  }
  .label {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .accel,
  .arrow {
    color: var(--text-dim);
    font-size: 11.5px;
    flex: none;
  }
  .sep {
    height: 1px;
    margin: 4px 6px;
    background: var(--border);
  }
</style>
