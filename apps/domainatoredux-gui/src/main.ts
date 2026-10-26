import { mount } from "svelte";
import App from "./App.svelte";
import "./app.css";

/// The `<input>` types that edit text (`HTMLInputElement.type` is lowercased and
/// defaults to `text`).
const TEXT_INPUT_TYPES = new Set(["text", "search", "url", "tel", "email", "password", "number"]);

/// Whether the raw webview context menu should be allowed through.
///
/// The browser menu is useless in a desktop app *except* for text editing, where
/// it is the only thing that offers a working Paste — the clipboard read a custom
/// menu would need is not dependably available in these webviews. So editable
/// fields and live selections keep it, and everything else gets ours.
function allowsNativeMenu(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false;

  // Tool parameter fields, filename prompts, the editable TSV grid, the text
  // editor: these need Cut/Copy/Paste/Undo. Only inputs that hold text, though:
  // a checkbox, radio, slider or button has nothing to paste into.
  const field = target.closest("input, textarea, [contenteditable=''], [contenteditable='true']");
  if (field) {
    return !(field instanceof HTMLInputElement) || TEXT_INPUT_TYPES.has(field.type);
  }

  // An explicit opt-out for anything added later.
  if (target.closest("[data-native-menu]")) return true;

  // Text the user has actually selected — notably the error bar, which is
  // deliberately selectable so it can be copied into a bug report.
  const sel = document.getSelection();
  if (sel && !sel.isCollapsed && sel.anchorNode && target.contains(sel.anchorNode)) {
    return true;
  }

  return false;
}

// Bubble phase, deliberately: component handlers run first, `openContextMenu`
// calls preventDefault(), and this sees `defaultPrevented` and stands down. That
// keeps the two composable without any ordering fragility, and means a surface
// with no menu of its own simply gets nothing rather than the browser's.
//
// Applies uniformly in dev and release — "Inspect Element" is not worth a
// behavioral difference between the build you test and the build you ship.
window.addEventListener("contextmenu", (e) => {
  if (e.defaultPrevented) return;
  if (allowsNativeMenu(e.target)) return;
  e.preventDefault();
});

const app = mount(App, { target: document.getElementById("app")! });

export default app;
