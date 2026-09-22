// Dispatch for native-menu clicks.
//
// `app_menu.rs` runs on the main thread and therefore only *emits*; everything
// that needs a window or a native dialog happens here, on the webview thread,
// through the ordinary async commands. (Creating a window or showing an rfd
// dialog from the menu handler re-enters WebView2's IPC dispatch and wedges the
// event loop on Windows — see the comment in `cmd_windows::spawn_window`.)
//
// `HANDLERS` is typed `Record<MenuId, Handler>` deliberately: TypeScript then
// makes a missing handler a `npm run check` error, which is the frontend half of
// the cross-language guard in `menu_ids.rs`. **Do not** relax it to
// `Partial<Record<…>>` — that is exactly what would let a menu item ship doing
// nothing.
import { listen } from "@tauri-apps/api/event";
import { ws } from "./state.svelte";
import { ui } from "./ui.svelte";
import {
  openExternalUrl,
  openFileWindow,
  pickDirectory,
  pickFiles,
  appDiagnostics,
} from "./api";
import { MENU_PREFIXES, type MenuCommand, type MenuId } from "./menuIds";

type Handler = (arg: string | null) => void | Promise<void>;

/// Open a workspace directory and remember it at the front of the recent list.
async function chooseWorkspace() {
  const dir = await pickDirectory();
  if (dir) await ws.open(dir);
}

export const HANDLERS: Record<MenuId, Handler> = {
  "file.new_file": () => {
    ui.newFileRequest++;
  },
  "file.new_folder": () => {
    ws.requestNewFolder = ws.selectedFolder ?? ws.root;
  },
  "file.open_workspace": chooseWorkspace,
  "file.import": async () => {
    const files = await pickFiles();
    if (files && files.length) await ws.importInto(files);
  },
  "file.save": () => {
    ui.saveRequest++;
  },
  "file.export_rows": () => {
    ui.exportRequest++;
  },
  "file.close_tab": () => {
    if (ws.activeTabId) ws.closeTab(ws.activeTabId);
  },
  "file.reopen_tab": () => ws.reopenClosedTab(),

  "edit.find": () => {
    ui.findRequest++;
  },
  "edit.preferences": () => {
    ui.preferencesRequest++;
  },

  "view.refresh": () => ws.refreshFolder(),
  "view.toggle_sidebar": () => {
    ui.sidebarVisible = !ui.sidebarVisible;
  },
  "view.toggle_table": () => {
    ui.tableVisible = !ui.tableVisible;
  },

  "tools.databases": () => {
    ui.focusPanel = "databases";
  },
  "tools.reference_dbs": () => {
    ui.focusPanel = "refdbs";
  },
  "tools.external": () => {
    ui.focusPanel = "external";
  },
  "tools.clear_jobs": () => {
    ui.clearJobsRequest++;
  },

  "window.pop_out": async () => {
    const tab = ws.tabs.find((t) => t.id === ws.activeTabId);
    if (tab) await openFileWindow(tab.file.path, tab.file.name);
  },

  "help.check_updates": () => {
    // Opens Preferences on the Updates section rather than a separate dialog:
    // one place that talks about versions is enough.
    ui.updateCheckRequest++;
    ui.preferencesRequest++;
  },
  "help.about": () => {
    ui.aboutRequest++;
  },
  "help.project": async () => {
    const d = await appDiagnostics();
    await openExternalUrl(d.project_url);
  },
  "help.issues": async () => {
    const d = await appDiagnostics();
    await openExternalUrl(d.issues_url);
  },
};

/// Handlers for parameterised ids (`<prefix>:<arg>`), keyed by prefix.
const PREFIX_HANDLERS: Record<(typeof MENU_PREFIXES)[number], Handler> = {
  "tools.run": (id) => {
    if (id) ws.openTool(id);
  },
  "file.open_recent": async (path) => {
    if (path) await ws.open(path);
  },
  // Focusing a window is handled in Rust (a cheap runtime message that creates
  // nothing); nothing to do here, but the prefix must still route.
  "window.focus": () => {},
  "view.col": () => {},
  "view.sort": () => {},
};

/// Commands that only make sense with the shell's workspace state. A detached
/// viewer window runs the same App.svelte, and on macOS shares the app menu
/// outright, so it must refuse these rather than half-apply them.
const SHELL_ONLY = new Set<string>([
  "file.new_file",
  "file.new_folder",
  "file.open_workspace",
  "file.import",
  "file.export_rows",
  "file.close_tab",
  "file.reopen_tab",
  "view.refresh",
  "view.toggle_sidebar",
  "view.toggle_table",
  "tools.databases",
  "tools.reference_dbs",
  "tools.external",
  "tools.clear_jobs",
  "window.pop_out",
  "tools.run",
  "file.open_recent",
]);

/// Start listening for menu commands. Returns an unlisten function.
export async function installMenuListener(isStandalone: boolean): Promise<() => void> {
  return listen<MenuCommand>("menu", (e) => {
    const { id, arg } = e.payload;

    if (isStandalone && SHELL_ONLY.has(id)) {
      ws.error = "That command is not available in a detached window.";
      return;
    }

    const run = (h: Handler | undefined) => {
      if (!h) {
        // Should be unreachable: the Rust guard pins the id set and the Record
        // type pins the handler set. Say so rather than failing silently.
        ws.error = `Unhandled menu command: ${id}`;
        return;
      }
      Promise.resolve(h(arg)).catch((err) => (ws.error = String(err)));
    };

    if (arg !== null) {
      run(PREFIX_HANDLERS[id as (typeof MENU_PREFIXES)[number]]);
    } else {
      run(HANDLERS[id as MenuId]);
    }
  });
}
