import { tick } from "svelte";

/// How long a panel stays outlined after a Tools-menu request for it.
export const PANEL_FLASH_MS = 5000;

const timers = new WeakMap<HTMLElement, number>();

/// Scroll a sidebar panel into view and outline it (the global `.panel-flash`
/// class) for `PANEL_FLASH_MS`. Its menu item otherwise does nothing visible when
/// the panel is already expanded. A repeat request restarts the 5 s. Waits a tick
/// so a panel that was just expanded is measured at its full height.
export async function flashPanel(el: HTMLElement | undefined) {
  if (!el) return;
  await tick();
  el.scrollIntoView({ block: "nearest" });
  el.classList.add("panel-flash");
  clearTimeout(timers.get(el));
  timers.set(el, window.setTimeout(() => el.classList.remove("panel-flash"), PANEL_FLASH_MS));
}
