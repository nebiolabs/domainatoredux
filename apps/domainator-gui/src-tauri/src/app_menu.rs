//! Building the native menu and routing its clicks.
//!
//! Native rather than an HTML menu bar because macOS is not opt-out: Tauri
//! installs `Menu::default()` there when none is set, so an HTML bar would mean
//! two menu bars unless the default is disabled — which deletes the app's only
//! Quit, Hide and Services. Native also gives the only clipboard items that
//! actually work in a webview (`PredefinedMenuItem::{cut,copy,paste,select_all}`)
//! and accelerators that still fire when focus is inside a `<canvas>` (the contig
//! and logo viewers) or the SSN `<iframe>` — which is most of this app.

use std::collections::HashMap;
use std::sync::Mutex;

use tauri::menu::{
    Menu, MenuBuilder, MenuItemBuilder, PredefinedMenuItem, Submenu, SubmenuBuilder,
    HELP_SUBMENU_ID,
};
use tauri::{AppHandle, Emitter, EventTarget, Manager, Wry};

use crate::menu_ids::{
    visible_items, MenuCommand, COMMANDS, DYNAMIC_PREFIXES, LAYOUT, MACOS_HIDDEN, SEPARATOR,
};

/// Items we need to reach again to enable/disable or check.
#[derive(Default)]
pub struct MenuRegistry(pub Mutex<Registry>);

#[derive(Default)]
pub struct Registry {
    pub items: HashMap<String, tauri::menu::MenuItem<Wry>>,
}

fn lock(reg: &MenuRegistry) -> std::sync::MutexGuard<'_, Registry> {
    // A panic while toggling one item must not disable the whole menu for the
    // session; the map is plain data with no half-applied invariant.
    reg.0.lock().unwrap_or_else(|e| e.into_inner())
}

fn spec(id: &str) -> &'static crate::menu_ids::MenuCommandSpec {
    COMMANDS
        .iter()
        .find(|c| c.id == id)
        .expect("LAYOUT references an id not in COMMANDS")
}

/// The recent-workspace list, filtered to what still exists — the same source the
/// welcome screen uses, so the two cannot disagree.
fn recent_workspaces(app: &AppHandle) -> Vec<String> {
    let Ok(path) = crate::cmd_prefs::prefs_path(app) else {
        return Vec::new();
    };
    domainator_core::app_support::prefs::load_prefs(&path)
        .existing_recent()
        .into_iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}

/// Last path component, for a menu label.
fn short_name(path: &str) -> String {
    path.rsplit(['/', '\\'])
        .find(|s| !s.is_empty())
        .unwrap_or(path)
        .to_string()
}

/// File → Open Recent, from the same prefs list the welcome screen shows.
fn open_recent_submenu(app: &AppHandle) -> tauri::Result<Submenu<Wry>> {
    let recent = recent_workspaces(app);
    let mut rs = SubmenuBuilder::new(app, "Open Recent");
    if recent.is_empty() {
        rs = rs.item(
            &MenuItemBuilder::with_id("file.open_recent:", "(none)")
                .enabled(false)
                .build(app)?,
        );
    } else {
        for path in &recent {
            rs = rs.item(
                &MenuItemBuilder::with_id(format!("file.open_recent:{path}"), short_name(path))
                    .build(app)?,
            );
        }
    }
    rs.build()
}

/// Build the main window's menu from [`LAYOUT`].
///
/// Panics on a bad accelerator rather than dropping the item: a typo like
/// `"Ctrl+Pluss"` should stop the app at launch, where it is obvious, not leave a
/// menu entry mysteriously missing. `menu_ids`' shape test catches most of these
/// first.
pub fn build_main(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let reg = app.state::<MenuRegistry>();
    let mut menu = MenuBuilder::new(app);

    // macOS puts Quit/Hide/Services in an app submenu named after the app, and
    // setting any menu replaces Tauri's default wholesale -- so without this the
    // Mac build loses Cmd+Q entirely.
    #[cfg(target_os = "macos")]
    {
        let app_name = app.package_info().name.clone();
        let sub = SubmenuBuilder::new(app, &app_name)
            .item(&PredefinedMenuItem::about(
                app,
                Some("About Domainator"),
                None,
            )?)
            .separator()
            .services()
            .separator()
            .hide()
            .hide_others()
            .show_all()
            .separator()
            .quit()
            .build()?;
        menu = menu.item(&sub);
    }

    // macOS keeps Quit in the app submenu above; elsewhere it is ours, in File.
    let hidden: &[&str] = if cfg!(target_os = "macos") {
        MACOS_HIDDEN
    } else {
        &[]
    };

    for (title, items) in LAYOUT {
        let mut sub = SubmenuBuilder::new(app, *title);

        for item in visible_items(items, hidden) {
            if item == SEPARATOR {
                sub = sub.separator();
                continue;
            }
            let s = spec(item);
            let mut b = MenuItemBuilder::with_id(s.id, s.label);
            if let Some(a) = s.accel {
                b = b.accelerator(a);
            }
            let built = b.build(app)?;
            lock(&reg).items.insert(s.id.to_string(), built.clone());
            sub = sub.item(&built);
            // Open Recent sits beside Open Workspace, where users look for it.
            if s.id == "file.open_workspace" {
                sub = sub.item(&open_recent_submenu(app)?);
            }
        }

        // Per-submenu extras that are dynamic or predefined.
        match *title {
            "Tools" => {
                // Built straight from the GUI tool registry, so the menu can never
                // drift from what `run_tool` can actually dispatch. 34 flat entries
                // is unusable, so they are grouped by the registry's category.
                let tools = tauri::async_runtime::block_on(crate::cmd_tools::list_tools());
                sub = sub.separator();
                // Grouped in TOOL_CATEGORIES order, not first-appearance order, so
                // the menu's shape is a property of that table rather than of the
                // registry's internal ordering.
                for cat in crate::cmd_tools::TOOL_CATEGORIES {
                    let in_cat: Vec<_> = tools.iter().filter(|t| t.category == cat).collect();
                    if in_cat.is_empty() {
                        continue;
                    }
                    let mut cs = SubmenuBuilder::new(app, cat);
                    for t in in_cat {
                        cs = cs.item(
                            &MenuItemBuilder::with_id(format!("tools.run:{}", t.id), &t.label)
                                .build(app)?,
                        );
                    }
                    sub = sub.item(&cs.build()?);
                }
                // A tool whose category is missing from the table would otherwise
                // be silently unreachable; `every_tool_has_a_known_category` is
                // the real guard, this is the runtime safety net.
                let orphans: Vec<_> = tools
                    .iter()
                    .filter(|t| !crate::cmd_tools::TOOL_CATEGORIES.contains(&t.category.as_str()))
                    .collect();
                if !orphans.is_empty() {
                    let mut cs = SubmenuBuilder::new(app, "Other");
                    for t in orphans {
                        cs = cs.item(
                            &MenuItemBuilder::with_id(format!("tools.run:{}", t.id), &t.label)
                                .build(app)?,
                        );
                    }
                    sub = sub.item(&cs.build()?);
                }
            }
            "Edit" => {
                // Predefined: the ONLY clipboard items that work in a webview.
                // Undo/Redo are omitted deliberately -- they drive the webview's
                // text-field undo stack, not `cmd_edit.rs`'s contig edits, so on
                // a contig view they would silently do nothing to the document
                // the user is looking at.
                sub = sub
                    .separator()
                    .item(&PredefinedMenuItem::cut(app, None)?)
                    .item(&PredefinedMenuItem::copy(app, None)?)
                    .item(&PredefinedMenuItem::paste(app, None)?)
                    .item(&PredefinedMenuItem::select_all(app, None)?);
            }
            _ => {}
        }

        // AppKit wires its automatic help search onto this id.
        let built = match *title {
            "Help" => sub.id(HELP_SUBMENU_ID).build()?,
            _ => sub.build()?,
        };
        menu = menu.item(&built);
    }

    menu.build()
}

/// A reduced menu for detached viewer windows: no workspace or tool commands,
/// which belong to the main window's state. Just the clipboard items, which only
/// work in a webview as predefined menu items; the window itself is managed from
/// its title bar.
///
/// Only used off macOS — `Window::set_menu` is unsupported there, so detached
/// windows share the app menu, which is the platform norm. The frontend
/// dispatcher is the backstop either way.
#[cfg(not(target_os = "macos"))]
pub fn build_detached(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let edit = SubmenuBuilder::new(app, "Edit")
        .item(&PredefinedMenuItem::cut(app, None)?)
        .item(&PredefinedMenuItem::copy(app, None)?)
        .item(&PredefinedMenuItem::paste(app, None)?)
        .item(&PredefinedMenuItem::select_all(app, None)?)
        .build()?;
    MenuBuilder::new(app).item(&edit).build()
}

/// Route a menu click to the focused window's frontend.
///
/// **This runs on the main thread, so it must only emit.** Creating a window or
/// showing an `rfd` dialog from here re-enters WebView2's IPC dispatch and wedges
/// the event loop on Windows — the failure documented at length in
/// `cmd_windows::spawn_window`. Everything that needs a window or a dialog goes
/// `emit → frontend → invoke(async command)` instead.
///
/// Note this is registered once, globally. `WebviewWindow::on_menu_event` looks
/// like the way to scope per window, but Tauri's dispatch loop calls *every*
/// registered window listener for *every* menu event, so two of them would run
/// each command twice.
pub fn route(app: &AppHandle, event: tauri::menu::MenuEvent) {
    let raw = event.id().as_ref();
    let cmd = MenuCommand::parse(raw);

    // A parameterized id whose prefix is not declared means the builder invented
    // an id no frontend handler routes -- worth saying out loud rather than
    // emitting a command that silently does nothing.
    if cmd.arg.is_some() && !DYNAMIC_PREFIXES.contains(&cmd.id.as_str()) {
        eprintln!("ignoring menu command with an undeclared dynamic prefix: {raw}");
        return;
    }
    // `Manager::get_focused_window` would be the obvious call, but it is behind
    // Tauri's `unstable` feature; `webview_windows()` + `is_focused()` is the
    // stable equivalent. Falling back to "main" keeps a command working even if
    // nothing reports focus (which happens briefly during window transitions).
    let target = app
        .webview_windows()
        .into_iter()
        .find(|(_, w)| w.is_focused().unwrap_or(false))
        .map(|(label, _)| label)
        .unwrap_or_else(|| "main".to_string());

    if let Err(e) = app.emit_to(EventTarget::webview_window(&target), "menu", cmd) {
        eprintln!("could not deliver menu command: {e}");
    }
}

/// What the frontend reports so the menu can gray items out.
#[derive(Debug, Default, serde::Deserialize)]
pub struct MenuState {
    pub has_workspace: bool,
    pub has_folder: bool,
    pub has_tab: bool,
    pub can_save: bool,
    pub has_closed_tab: bool,
}

impl MenuState {
    fn allows(&self, needs: crate::menu_ids::Needs) -> bool {
        use crate::menu_ids::Needs::*;
        match needs {
            Nothing => true,
            Workspace => self.has_workspace,
            Folder => self.has_folder,
            Tab => self.has_tab,
            Savable => self.can_save,
            ClosedTab => self.has_closed_tab,
        }
    }
}

/// Apply the frontend's state to the menu.
///
/// `async` so `set_enabled` (which internally posts to the event loop) is called
/// off the main thread. Only the main window publishes — a detached window's view
/// of "is a tab open" is not the main window's.
#[tauri::command]
pub async fn set_menu_state(
    window: tauri::WebviewWindow,
    reg: tauri::State<'_, MenuRegistry>,
    state: MenuState,
) -> Result<(), String> {
    if window.label() != "main" {
        return Ok(());
    }
    let guard = lock(&reg);
    for c in COMMANDS {
        if let Some(item) = guard.items.get(c.id) {
            let _ = item.set_enabled(state.allows(c.needs));
        }
    }
    Ok(())
}
