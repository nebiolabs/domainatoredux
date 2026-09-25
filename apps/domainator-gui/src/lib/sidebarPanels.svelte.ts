// Heights of the sidebar panels under the folder tree (Jobs, Databases,
// Reference databases, External tools), shared between each panel's body
// (`ResizableBody`) and the seam above it (`PanelSeam`).
//
// The seam above a panel is what resizes it, as in any split view: it moves the
// boundary between that panel and whatever sits above it. That neighbor is the
// panel before it in `PANEL_ORDER` when that one is expanded; otherwise it is the
// folder tree, which is the flexible part of the sidebar and absorbs the change.

/// Top-to-bottom order of the panels in `App.svelte`'s sidebar.
export const PANEL_ORDER = ["jobs", "databases", "refdbs", "external"] as const;
export type PanelKey = (typeof PANEL_ORDER)[number];

export const MIN_PANEL_HEIGHT = 40;

const storageKey = (key: PanelKey) => `sidebar.height.${key}`;

function stored(key: PanelKey): number | null {
  try {
    const v = Number(localStorage.getItem(storageKey(key)));
    return Number.isFinite(v) && v >= MIN_PANEL_HEIGHT ? v : null;
  } catch {
    return null;
  }
}

/// A panel's dragged height, or `null` for "as tall as its content, up to its
/// default maximum".
export const panelHeights = $state(
  Object.fromEntries(PANEL_ORDER.map((k) => [k, stored(k)])) as Record<PanelKey, number | null>,
);

export function persistHeight(key: PanelKey): void {
  try {
    const h = panelHeights[key];
    if (h === null) localStorage.removeItem(storageKey(key));
    else localStorage.setItem(storageKey(key), String(h));
  } catch {
    /* sizes are a convenience; storage can be unavailable */
  }
}

/// The body element of each expanded panel. A collapsed panel has no body, so
/// it is absent here and a seam below it resizes against the folder tree.
const bodies = new Map<PanelKey, HTMLElement>();

export function registerBody(key: PanelKey, el: HTMLElement): () => void {
  bodies.set(key, el);
  return () => {
    if (bodies.get(key) === el) bodies.delete(key);
  };
}

export function panelBody(key: PanelKey): HTMLElement | undefined {
  return bodies.get(key);
}

/// The expanded panel directly above `key`, if there is one.
export function neighborAbove(key: PanelKey): PanelKey | null {
  const i = PANEL_ORDER.indexOf(key);
  const above = i > 0 ? PANEL_ORDER[i - 1] : null;
  return above && bodies.has(above) ? above : null;
}
