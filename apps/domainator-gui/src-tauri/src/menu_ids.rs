//! The application menu, as data.
//!
//! **No Tauri types live here on purpose.** Every guard below is then a plain
//! unit test needing no `AppHandle` and no display, which matters because this
//! crate has no parity gate — `cargo test` is its entire safety net — and because
//! GitHub CI does not build the GUI crate at all (webkit2gtk is too expensive for
//! that job), so these run under `scripts/sync_upstream.sh` and locally.
//!
//! `app_menu.rs` builds the real menu *from* [`LAYOUT`], so the menu a user sees
//! cannot contain a command this table does not describe, or omit one it does.
//!
//! ## Id scheme
//!
//! Two kinds:
//!
//! * **static** — `^[a-z]+\.[a-z0-9_]+$`, e.g. `file.open_workspace`. A closed set,
//!   pinned to the frontend by [`tests::every_menu_id_is_known_to_the_frontend`].
//! * **parameterised** — `<prefix>:<arg>`, e.g. `tools.run:domain-search`,
//!   `file.open_recent:3`, `window.focus:file7`. `:` is reserved as the separator
//!   and therefore forbidden in a static id.

/// What must be true for a command to be enabled.
///
/// Evaluated in the frontend (which owns the state) and sent back through
/// `set_menu_state`; the variants exist so the table documents the condition
/// rather than leaving it implicit in two places.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Needs {
    /// Always enabled.
    Nothing,
    /// A workspace is open.
    Workspace,
    /// A folder is selected inside the workspace.
    Folder,
    /// At least one file tab is open.
    Tab,
    /// The active view can save (a dirty editor).
    Savable,
    /// The active view has a filter box to focus.
    Findable,
    /// The active view is a table whose rows can be exported.
    Exportable,
    /// Something has been closed this session.
    ClosedTab,
}

/// One menu command.
#[derive(Debug, Clone, Copy)]
pub struct MenuCommandSpec {
    pub id: &'static str,
    pub label: &'static str,
    /// muda accelerator string, e.g. `"CmdOrCtrl+O"`. Only shape-checked here —
    /// see [`tests::menu_accelerators_are_well_formed`] for why.
    pub accel: Option<&'static str>,
    pub needs: Needs,
}

const fn cmd(
    id: &'static str,
    label: &'static str,
    accel: Option<&'static str>,
    needs: Needs,
) -> MenuCommandSpec {
    MenuCommandSpec { id, label, accel, needs }
}

/// Every statically-known command the menu can emit.
pub const COMMANDS: &[MenuCommandSpec] = &[
    // File
    cmd("file.new_file", "New File…", Some("CmdOrCtrl+N"), Needs::Folder),
    cmd("file.new_folder", "New Folder…", Some("CmdOrCtrl+Shift+N"), Needs::Workspace),
    cmd("file.open_workspace", "Open Workspace…", Some("CmdOrCtrl+O"), Needs::Nothing),
    cmd("file.import", "Import Files…", Some("CmdOrCtrl+I"), Needs::Folder),
    cmd("file.save", "Save", Some("CmdOrCtrl+S"), Needs::Savable),
    cmd("file.export_rows", "Export Visible Rows…", Some("CmdOrCtrl+Shift+E"), Needs::Exportable),
    cmd("file.close_tab", "Close Tab", Some("CmdOrCtrl+W"), Needs::Tab),
    cmd("file.reopen_tab", "Reopen Closed Tab", Some("CmdOrCtrl+Shift+T"), Needs::ClosedTab),
    // Edit (clipboard items are predefined and carry no id)
    cmd("edit.find", "Find…", Some("CmdOrCtrl+F"), Needs::Findable),
    cmd("edit.preferences", "Preferences…", Some("CmdOrCtrl+Comma"), Needs::Nothing),
    // View
    cmd("view.refresh", "Refresh Folder", Some("F5"), Needs::Folder),
    cmd("view.toggle_sidebar", "Show Sidebar", Some("CmdOrCtrl+1"), Needs::Nothing),
    cmd("view.toggle_table", "Show File Table", Some("CmdOrCtrl+2"), Needs::Nothing),
    // Tools
    cmd("tools.databases", "Manage Databases…", None, Needs::Workspace),
    cmd("tools.external", "External Tools…", None, Needs::Nothing),
    cmd("tools.clear_jobs", "Clear Job History…", None, Needs::Workspace),
    // Window
    cmd("window.pop_out", "Pop Out Current Tab", Some("CmdOrCtrl+Shift+O"), Needs::Tab),
    // Help
    cmd("help.check_updates", "Check for Updates…", None, Needs::Nothing),
    cmd("help.about", "About Domainator", None, Needs::Nothing),
    cmd("help.project", "Project on GitHub", None, Needs::Nothing),
    cmd("help.issues", "Report an Issue", None, Needs::Nothing),
];

/// Prefixes for parameterised ids. The router splits on the first `:`.
pub const DYNAMIC_PREFIXES: &[&str] =
    &["tools.run", "file.open_recent", "window.focus", "view.col", "view.sort"];

/// `"-"` in a [`LAYOUT`] item list renders a separator.
pub const SEPARATOR: &str = "-";

/// Submenu title → its items, in order. `app_menu.rs` renders exactly this.
///
/// Dynamic sections (Open Recent, the tool list, the window list) are appended by
/// the builder and so do not appear here.
pub const LAYOUT: &[(&str, &[&str])] = &[
    (
        "File",
        &[
            "file.new_file",
            "file.new_folder",
            SEPARATOR,
            "file.open_workspace",
            "file.import",
            SEPARATOR,
            "file.save",
            "file.export_rows",
            SEPARATOR,
            "file.close_tab",
            "file.reopen_tab",
        ],
    ),
    ("Edit", &["edit.find", SEPARATOR, "edit.preferences"]),
    ("View", &["view.refresh", SEPARATOR, "view.toggle_sidebar", "view.toggle_table"]),
    ("Tools", &["tools.databases", "tools.external", SEPARATOR, "tools.clear_jobs"]),
    ("Window", &["window.pop_out"]),
    ("Help", &["help.check_updates", SEPARATOR, "help.about", SEPARATOR, "help.project", "help.issues"]),
];

/// What the menu sends to the frontend. `arg` is the part after the `:`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MenuCommand {
    pub id: String,
    pub arg: Option<String>,
}

impl MenuCommand {
    /// Split a raw muda id into `(id, arg)`.
    pub fn parse(raw: &str) -> Self {
        match raw.split_once(':') {
            Some((id, arg)) => {
                Self { id: id.to_string(), arg: Some(arg.to_string()) }
            }
            None => Self { id: raw.to_string(), arg: None },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Ids as the frontend declares them, extracted from `MENU_IDS = [...]`.
    ///
    /// `include_str!` rather than `fs::read` on purpose: it makes cargo's
    /// fingerprint depend on the TypeScript file, so editing `menuIds.ts` alone
    /// re-runs this test, and it is independent of the working directory.
    fn frontend_ids() -> HashSet<String> {
        const TS: &str = include_str!("../../src/lib/menuIds.ts");
        let start = TS.find("MENU_IDS = [").expect("menuIds.ts has no `MENU_IDS = [`");
        let rest = &TS[start..];
        let end = rest.find(']').expect("menuIds.ts has no closing `]`");
        rest[..end]
            .split('"')
            .skip(1)
            .step_by(2)
            .map(|s| s.to_string())
            .collect()
    }

    /// The headline guard, and the direct analogue of
    /// `every_offered_tool_is_dispatched_and_has_a_schema`: two hand-maintained
    /// lists, in two languages, in two files.
    ///
    /// An id Rust emits with no frontend entry is a menu item that does nothing
    /// when clicked; an id the frontend lists that Rust never emits is dead code
    /// that looks live. Both directions are checked.
    #[test]
    fn every_menu_id_is_known_to_the_frontend() {
        let rust: HashSet<String> = COMMANDS.iter().map(|c| c.id.to_string()).collect();
        let ts = frontend_ids();

        let missing: Vec<_> = rust.difference(&ts).cloned().collect();
        assert!(
            missing.is_empty(),
            "emitted by Rust but absent from src/lib/menuIds.ts (clicking these would \
             do nothing): {missing:?}"
        );

        let extra: Vec<_> = ts.difference(&rust).cloned().collect();
        assert!(
            extra.is_empty(),
            "listed in src/lib/menuIds.ts but no Rust menu item emits them (dead \
             handlers): {extra:?}"
        );
    }

    #[test]
    fn menu_ids_are_well_formed_and_unique() {
        let mut seen = HashSet::new();
        for c in COMMANDS {
            assert!(seen.insert(c.id), "duplicate menu id {:?}", c.id);
            assert!(
                !c.id.contains(':'),
                "{:?} contains ':', which is reserved as the argument separator",
                c.id
            );
            let (prefix, rest) = c.id.split_once('.').unwrap_or_else(|| {
                panic!("{:?} is not `<group>.<name>`", c.id)
            });
            assert!(
                !prefix.is_empty() && prefix.chars().all(|ch| ch.is_ascii_lowercase()),
                "{:?} has a non-lowercase group",
                c.id
            );
            assert!(
                !rest.is_empty()
                    && rest.chars().all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_'),
                "{:?} has a malformed name",
                c.id
            );
            assert!(!c.label.trim().is_empty(), "{:?} has an empty label", c.id);
        }
    }

    /// The router does `split_once(':')`, so a dynamic prefix that is also a
    /// static id (or a prefix of one) would make a click ambiguous.
    #[test]
    fn dynamic_prefixes_do_not_shadow_static_ids() {
        let statics: HashSet<&str> = COMMANDS.iter().map(|c| c.id).collect();
        let mut seen = HashSet::new();
        for p in DYNAMIC_PREFIXES {
            assert!(seen.insert(p), "duplicate dynamic prefix {p:?}");
            assert!(!p.contains(':'), "dynamic prefix {p:?} must not contain ':'");
            assert!(
                !statics.contains(p),
                "{p:?} is both a dynamic prefix and a static id"
            );
        }
    }

    /// Accelerators are global once the menu is installed, so a duplicate means
    /// one of the two commands silently never fires.
    ///
    /// The webview's own clipboard bindings are included because the predefined
    /// Edit items use them, and `CmdOrCtrl+Shift+W` because that is what
    /// `PredefinedMenuItem::close_window` is given explicitly (its default is
    /// `CmdOrCtrl+W`, which would collide with Close Tab).
    #[test]
    fn menu_accelerators_are_unique() {
        const RESERVED: &[&str] = &[
            "CmdOrCtrl+C",
            "CmdOrCtrl+V",
            "CmdOrCtrl+X",
            "CmdOrCtrl+A",
            "CmdOrCtrl+Q",
            "CmdOrCtrl+M",
            "CmdOrCtrl+Shift+W",
            // The contig viewer's "copy the selected bases" shortcut
            // (`ContigView.svelte`). It moved here off a plain Cmd/Ctrl+C once
            // Edit -> Copy claimed that: a menu accelerator is dispatched before
            // the webview sees the key, so the old binding would have gone quietly
            // dead rather than merely double-copying. Reserved so no menu command
            // can take it back.
            "CmdOrCtrl+Shift+C",
            "F11",
        ];
        let norm = |a: &str| a.to_ascii_lowercase();
        let mut seen: HashSet<String> = RESERVED.iter().map(|a| norm(a)).collect();
        for c in COMMANDS {
            if let Some(a) = c.accel {
                assert!(
                    seen.insert(norm(a)),
                    "{:?} reuses accelerator {a:?}; one of the two would never fire",
                    c.id
                );
            }
        }
    }

    /// Shape only — `muda::accelerator::Accelerator: FromStr` exists but
    /// `tauri::menu` does not re-export it, and `MenuItemBuilder::build()` only
    /// surfaces a parse error with a live `AppHandle`. So this catches typos in
    /// the modifiers and obvious key nonsense, and `app_menu::build_main`
    /// deliberately panics on a real parse failure at startup rather than
    /// silently dropping the item.
    #[test]
    fn menu_accelerators_are_well_formed() {
        const MODS: &[&str] = &["CmdOrCtrl", "Cmd", "Command", "Ctrl", "Control", "Alt", "Shift", "Super"];
        for c in COMMANDS {
            let Some(a) = c.accel else { continue };
            let parts: Vec<&str> = a.split('+').collect();
            assert!(!parts.is_empty(), "{:?} has an empty accelerator", c.id);
            let (key, mods) = parts.split_last().unwrap();
            for m in mods {
                assert!(MODS.contains(m), "{:?}: {m:?} is not a modifier (in {a:?})", c.id);
            }
            let ok = (key.len() == 1
                && key.chars().next().is_some_and(|k| k.is_ascii_alphanumeric()))
                || (key.starts_with('F') && key[1..].parse::<u8>().is_ok_and(|n| (1..=24).contains(&n)))
                || ["Plus", "Minus", "Equal", "Comma", "Period", "Space", "Enter", "Delete", "Backspace"]
                    .contains(key);
            assert!(ok, "{:?}: {key:?} is not a recognized key (in {a:?})", c.id);
        }
    }

    /// `app_menu.rs` renders `LAYOUT`, so an id missing from it is a command the
    /// guards above bless and no user can ever reach; one listed twice appears in
    /// two menus.
    #[test]
    fn every_command_appears_in_exactly_one_layout_slot() {
        let mut placed: Vec<&str> = Vec::new();
        for (_, items) in LAYOUT {
            for it in *items {
                if *it != SEPARATOR {
                    placed.push(it);
                }
            }
        }
        let placed_set: HashSet<&str> = placed.iter().copied().collect();
        assert_eq!(placed.len(), placed_set.len(), "an id appears in two submenus: {placed:?}");

        let declared: HashSet<&str> = COMMANDS.iter().map(|c| c.id).collect();
        let unreachable: Vec<_> = declared.difference(&placed_set).collect();
        assert!(unreachable.is_empty(), "declared but in no submenu: {unreachable:?}");
        let unknown: Vec<_> = placed_set.difference(&declared).collect();
        assert!(unknown.is_empty(), "in a submenu but not declared in COMMANDS: {unknown:?}");
    }

    /// A stray separator at the top or bottom of a menu, or two in a row, is the
    /// classic symptom of an item that was removed or conditionally compiled out.
    #[test]
    fn no_submenu_is_empty_or_has_a_dangling_separator() {
        for (title, items) in LAYOUT {
            assert!(!items.is_empty(), "submenu {title:?} is empty");
            assert_ne!(items.first(), Some(&SEPARATOR), "{title:?} starts with a separator");
            assert_ne!(items.last(), Some(&SEPARATOR), "{title:?} ends with a separator");
            for w in items.windows(2) {
                assert!(
                    !(w[0] == SEPARATOR && w[1] == SEPARATOR),
                    "{title:?} has two separators in a row"
                );
            }
        }
    }

    #[test]
    fn parsing_splits_only_on_the_first_colon() {
        let m = MenuCommand::parse("tools.run:domain-search");
        assert_eq!(m.id, "tools.run");
        assert_eq!(m.arg.as_deref(), Some("domain-search"));

        let plain = MenuCommand::parse("help.about");
        assert_eq!(plain.id, "help.about");
        assert_eq!(plain.arg, None);

        // A Windows path as an argument keeps its drive colon.
        let path = MenuCommand::parse("file.open_recent:C:\\data\\ws");
        assert_eq!(path.id, "file.open_recent");
        assert_eq!(path.arg.as_deref(), Some("C:\\data\\ws"));
    }
}
