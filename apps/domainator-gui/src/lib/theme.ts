import type { Theme } from "./types";

/// Apply a theme choice to the document.
///
/// "system" *removes* the attribute rather than setting it, so the
/// `prefers-color-scheme` media query in `app.css` decides — which is also what
/// renders before this runs, so there is no flash on launch.
export function applyTheme(theme: Theme): void {
  const root = document.documentElement;
  if (theme === "system") delete root.dataset.theme;
  else root.dataset.theme = theme;
}
