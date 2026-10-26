// Shared state and API for the app's right-click menus.
//
// Named `ctxmenu`, not `contextmenu`: macOS and Windows have case-insensitive
// filesystems, so `import ... from "./contextmenu.svelte"` resolves to the
// sibling component `ContextMenu.svelte` there and the component imports
// itself — a build that is green on Linux and fails on the other two. The
// guard is the name; keep the two spellings distinct.
//
// The app used to have none at all, so every right-click fell through to the
// webview's own menu (Reload / Save as / Inspect Element) — see the global
// suppressor in `main.ts`. This is the replacement.
//
// Drawn in HTML rather than with `@tauri-apps/api/menu` on purpose: the items are
// built per row, per job, per tab and per cell, so a native menu would construct a
// fresh `Menu` plus N `MenuItem` resources on every right-click (two IPC round
// trips before it paints, and each item is a resource that leaks unless closed),
// and every action here is a plain frontend call on the store anyway.

/// One entry in a context menu. A item with no `label` renders as a separator.
export interface CtxItem {
  label?: string;
  /// May be async and may throw; a rejection is surfaced through `ws.error`.
  action?: () => void | Promise<void>;
  disabled?: boolean;
  /// Renders in the error color — destructive actions (Delete, Clear history).
  danger?: boolean;
  /// Renders a checkmark; makes the item a `menuitemcheckbox`.
  checked?: boolean;
  /// Display-only accelerator hint. Never bound to a key handler here — real
  /// accelerators belong to the native menu.
  accel?: string;
  /// One level of nesting only. Deeper menus are unusable at this size.
  submenu?: CtxItem[];
}

/// The single open menu. One instance app-wide, rendered by `ContextMenu.svelte`.
export const ctx = $state({
  open: false,
  x: 0,
  y: 0,
  items: [] as CtxItem[],
  /// Focus is returned here on close, so right-clicking a row and pressing Escape
  /// leaves the keyboard where it was.
  returnFocus: null as HTMLElement | null,
  /// The button a dropdown hangs from (`toggleMenuBelow`). A press on it is left to
  /// the button, which toggles, rather than dismissing the menu only for the
  /// same click to reopen it.
  anchor: null as HTMLElement | null,
});

/// Open a context menu at the event's position.
///
/// Calls `preventDefault()` first, which is also what tells the global suppressor
/// in `main.ts` to stand down — it skips any event a component already claimed.
export function openContextMenu(e: MouseEvent, items: CtxItem[]): void {
  if (items.length === 0) return;
  e.preventDefault();
  e.stopPropagation();
  ctx.returnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  ctx.x = e.clientX;
  ctx.y = e.clientY;
  ctx.items = items;
  ctx.anchor = null;
  ctx.open = true;
}

/// Open a menu as a dropdown under `anchor` (a toolbar button), or close it if
/// that same button's menu is already open.
export function toggleMenuBelow(anchor: HTMLElement, items: CtxItem[]): void {
  if (ctx.open && ctx.anchor === anchor) {
    closeContextMenu();
    return;
  }
  if (items.length === 0) return;
  const r = anchor.getBoundingClientRect();
  ctx.returnFocus = anchor;
  ctx.x = r.left;
  ctx.y = r.bottom + 2;
  ctx.items = items;
  ctx.anchor = anchor;
  ctx.open = true;
}

export function closeContextMenu(): void {
  if (!ctx.open) return;
  ctx.open = false;
  ctx.items = [];
  ctx.anchor = null;
  const back = ctx.returnFocus;
  ctx.returnFocus = null;
  back?.focus?.();
}

/// A separator, for readability at call sites.
export const SEP: CtxItem = {};
