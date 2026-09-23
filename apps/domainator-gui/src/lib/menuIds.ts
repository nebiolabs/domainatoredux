// Mirror of `src-tauri/src/menu_ids.rs`'s COMMANDS.
//
// Pinned to it by the Rust test `every_menu_id_is_known_to_the_frontend`, which
// reads THIS FILE with `include_str!` and asserts set-equality in both
// directions. Add an id here and in COMMANDS together, or `cargo test` fails.
//
// The other half of the guard is `menu.ts`'s `HANDLERS: Record<MenuId, Handler>`:
// TypeScript makes a missing handler a `npm run check` error. Do not relax that
// to `Partial<Record<…>>` — it is what stops a menu item from silently doing
// nothing.
export const MENU_IDS = [
  "file.new_file",
  "file.new_folder",
  "file.open_workspace",
  "file.import",
  "file.save",
  "file.close_tab",
  "file.reopen_tab",
  "edit.preferences",
  "view.refresh",
  "view.toggle_sidebar",
  "view.toggle_table",
  "tools.databases",
  "tools.reference_dbs",
  "tools.external",
  "tools.clear_jobs",
  "window.pop_out",
  "help.check_updates",
  "help.about",
  "help.project",
  "help.issues",
] as const;

export type MenuId = (typeof MENU_IDS)[number];

/// Prefixes for parameterized ids (`<prefix>:<arg>`), mirroring DYNAMIC_PREFIXES.
export const MENU_PREFIXES = [
  "tools.run",
  "file.open_recent",
] as const;

export type MenuPrefix = (typeof MENU_PREFIXES)[number];

/// The payload `app_menu.rs` emits on the "menu" event.
export interface MenuCommand {
  id: string;
  arg: string | null;
}
