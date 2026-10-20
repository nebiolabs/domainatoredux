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
//! * **parameterized** — `<prefix>:<arg>`, e.g. `tools.run:domain-search`,
//!   `file.open_recent:3`. `:` is reserved as the separator
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
    /// Something has been closed this session.
    ClosedTab,
    /// At least one file is selected in the file table.
    Selection,
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
    MenuCommandSpec {
        id,
        label,
        accel,
        needs,
    }
}

/// Every statically-known command the menu can emit.
pub const COMMANDS: &[MenuCommandSpec] = &[
    // File
    cmd(
        "file.new_file",
        "New File…",
        Some("CmdOrCtrl+N"),
        Needs::Folder,
    ),
    cmd(
        "file.new_folder",
        "New Folder…",
        Some("CmdOrCtrl+Shift+N"),
        Needs::Workspace,
    ),
    cmd(
        "file.open_workspace",
        "Open Workspace…",
        Some("CmdOrCtrl+O"),
        Needs::Nothing,
    ),
    cmd(
        "file.import",
        "Import Files…",
        Some("CmdOrCtrl+I"),
        Needs::Folder,
    ),
    cmd("file.export", "Export…", None, Needs::Selection),
    cmd("file.save", "Save", Some("CmdOrCtrl+S"), Needs::Savable),
    cmd(
        "file.close_tab",
        "Close Tab",
        Some("CmdOrCtrl+W"),
        Needs::Tab,
    ),
    cmd(
        "file.reopen_tab",
        "Reopen Closed Tab",
        Some("CmdOrCtrl+Shift+T"),
        Needs::ClosedTab,
    ),
    // Our own item rather than `PredefinedMenuItem::quit`: on Windows that one is
    // a bare `PostQuitMessage`, which ends the event loop without any window
    // seeing a close request -- so it skipped the unsaved-changes prompt.
    cmd("file.quit", "Quit", Some("CmdOrCtrl+Q"), Needs::Nothing),
    // Edit (clipboard items are predefined and carry no id)
    cmd(
        "edit.preferences",
        "Preferences…",
        Some("CmdOrCtrl+Comma"),
        Needs::Nothing,
    ),
    // View
    cmd("view.refresh", "Refresh Folder", Some("F5"), Needs::Folder),
    cmd(
        "view.toggle_sidebar",
        "Show Sidebar",
        Some("CmdOrCtrl+1"),
        Needs::Nothing,
    ),
    cmd(
        "view.toggle_table",
        "Show File Table",
        Some("CmdOrCtrl+2"),
        Needs::Nothing,
    ),
    // Tools
    cmd(
        "tools.databases",
        "Manage Databases…",
        None,
        Needs::Workspace,
    ),
    cmd(
        "tools.reference_dbs",
        "Reference Databases…",
        None,
        Needs::Nothing,
    ),
    cmd("tools.external", "External Tools…", None, Needs::Nothing),
    cmd(
        "tools.clear_jobs",
        "Clear Job History…",
        None,
        Needs::Workspace,
    ),
    // Help
    cmd(
        "help.check_updates",
        "Check for Updates…",
        None,
        Needs::Nothing,
    ),
    cmd("help.about", "About Domainator", None, Needs::Nothing),
    cmd("help.project", "Project on GitHub", None, Needs::Nothing),
    cmd("help.issues", "Report an Issue", None, Needs::Nothing),
];

/// Prefixes for parameterized ids. The router splits on the first `:`.
pub const DYNAMIC_PREFIXES: &[&str] = &["tools.run", "file.open_recent"];

/// `"-"` in a [`LAYOUT`] item list renders a separator.
pub const SEPARATOR: &str = "-";

/// Submenu title → its items, in order. `app_menu.rs` renders exactly this.
///
/// Dynamic sections (Open Recent, the tool list) are inserted by the builder and
/// so do not appear here.
///
/// There is deliberately no Window menu: minimize/maximize are on the title bar,
/// popping a tab out is the ⧉ button on the tab itself, and muda's predefined
/// Fullscreen item is a silent no-op on Windows and Linux.
pub const LAYOUT: &[(&str, &[&str])] = &[
    (
        "File",
        &[
            "file.new_file",
            "file.new_folder",
            SEPARATOR,
            "file.open_workspace",
            "file.import",
            "file.export",
            SEPARATOR,
            "file.save",
            SEPARATOR,
            "file.close_tab",
            "file.reopen_tab",
            SEPARATOR,
            "file.quit",
        ],
    ),
    ("Edit", &["edit.preferences"]),
    (
        "View",
        &[
            "view.refresh",
            SEPARATOR,
            "view.toggle_sidebar",
            "view.toggle_table",
        ],
    ),
    (
        "Tools",
        &[
            "tools.databases",
            "tools.reference_dbs",
            "tools.external",
            SEPARATOR,
            "tools.clear_jobs",
        ],
    ),
    (
        "Help",
        &[
            "help.check_updates",
            SEPARATOR,
            "help.about",
            SEPARATOR,
            "help.project",
            "help.issues",
        ],
    ),
];

/// Ids left out of the menu on macOS, where the app submenu already carries the
/// platform's own Quit.
pub const MACOS_HIDDEN: &[&str] = &["file.quit"];

/// `items` without the ids in `hidden`, and without the separators that hiding
/// them would leave dangling (leading, trailing or doubled).
pub fn visible_items<'a>(items: &[&'a str], hidden: &[&str]) -> Vec<&'a str> {
    let mut out: Vec<&'a str> = Vec::new();
    for it in items.iter().copied().filter(|it| !hidden.contains(it)) {
        if it == SEPARATOR && out.last().is_none_or(|l| *l == SEPARATOR) {
            continue;
        }
        out.push(it);
    }
    if out.last() == Some(&SEPARATOR) {
        out.pop();
    }
    out
}

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
            Some((id, arg)) => Self {
                id: id.to_string(),
                arg: Some(arg.to_string()),
            },
            None => Self {
                id: raw.to_string(),
                arg: None,
            },
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
        let start = TS
            .find("MENU_IDS = [")
            .expect("menuIds.ts has no `MENU_IDS = [`");
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

    /// The gap the id guard above cannot see: a handler that exists but whose
    /// effect nothing observes. Most handlers only bump a request counter or set
    /// a flag on the `ui` store (`ui.newFileRequest++`) for some component to
    /// react to, so the id is known, the handler is typed, `npm run check` passes
    /// -- and
    /// the item still does nothing if no component reads that field. File ->
    /// Export Visible Rows and Edit -> Find both shipped exactly that way.
    ///
    /// Every `ui.<field>` written in `menu.ts` must be read by some other file
    /// under `src/` (the store's own declaration in `ui.svelte.ts` excluded).
    #[test]
    fn every_ui_request_a_menu_handler_writes_has_a_reader() {
        const MENU_TS: &str = include_str!("../../src/lib/menu.ts");
        let written: HashSet<String> = MENU_TS
            .match_indices("ui.")
            // `ui.` preceded by an identifier character is some other name
            // (`gui.`), not the store.
            .filter(|(i, _)| {
                !MENU_TS[..*i].ends_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
            })
            .map(|(i, _)| {
                MENU_TS[i + 3..]
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect::<String>()
            })
            .filter(|f| !f.is_empty() && f != "svelte")
            .collect();
        assert!(
            !written.is_empty(),
            "found no `ui.` writes in menu.ts; has the store moved?"
        );

        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src");
        let mut others = String::new();
        let mut stack = vec![src];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("read frontend src dir") {
                let path = entry.expect("dir entry").path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                let frontend = name.ends_with(".svelte") || name.ends_with(".ts");
                if frontend && name != "menu.ts" && name != "ui.svelte.ts" {
                    others.push_str(&std::fs::read_to_string(&path).expect("read frontend file"));
                    others.push('\n');
                }
            }
        }

        let unread: Vec<_> = written
            .iter()
            .filter(|f| {
                let needle = format!("ui.{f}");
                !others.match_indices(&needle).any(|(i, _)| {
                    !others[i + needle.len()..]
                        .starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
                })
            })
            .collect();
        assert!(
            unread.is_empty(),
            "menu.ts writes these `ui` fields but no component reads them, so their \
             menu items do nothing when clicked: {unread:?}"
        );
    }

    /// Every route that closes a tab must ask about an unsaved buffer first.
    ///
    /// Closing a tab destroys its editor component (`ViewerDock` drops the panes
    /// of a tab that leaves `ws.tabs`), and the `$effect` teardown that clears the
    /// dirty flag goes with it, so even the window-close guard could not see the
    /// edits that had just been discarded. `Workspace.guardDirty` is the one
    /// chokepoint, and this asserts nothing reassigns `this.tabs` outside it: a new
    /// method (`closeTabsToTheRight`, say) reintroduces the data loss with no other
    /// symptom, because the buffer is gone before anything can report it.
    ///
    /// Only removal needs the guard. Switching tabs (`this.activeTabId =`) keeps
    /// every visited view mounted, and appending a tab (`[...this.tabs, ...]`)
    /// destroys nothing.
    #[test]
    fn every_tab_mutation_goes_through_the_unsaved_changes_guard() {
        const STATE_TS: &str = include_str!("../../src/lib/state.svelte.ts");

        // Character ranges covered by a `this.guardDirty( ... )` call, by paren
        // matching from each call site.
        let mut guarded: Vec<(usize, usize)> = Vec::new();
        for (start, _) in STATE_TS.match_indices("this.guardDirty(") {
            let open = start + "this.guardDirty".len();
            let mut depth = 0usize;
            let mut end = open;
            for (off, ch) in STATE_TS[open..].char_indices() {
                match ch {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = open + off;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            assert!(end > open, "unbalanced parens after a guardDirty call");
            guarded.push((start, end));
        }
        assert!(
            !guarded.is_empty(),
            "no `this.guardDirty(` calls in state.svelte.ts -- has the guard been removed?"
        );

        let mut unguarded = Vec::new();
        for needle in ["this.tabs ="] {
            for (i, _) in STATE_TS.match_indices(needle) {
                let rhs = &STATE_TS[i + needle.len()..];
                // `===` / `==` are comparisons, not assignments; an append removes
                // nothing.
                if rhs.starts_with('=') || rhs.trim_start().starts_with("[...this.tabs,") {
                    continue;
                }
                if !guarded.iter().any(|&(lo, hi)| i > lo && i < hi) {
                    let line = STATE_TS[..i].lines().count();
                    unguarded.push(format!("{needle} at line {line}"));
                }
            }
        }
        assert!(
            unguarded.is_empty(),
            "these remove open tabs without going through `guardDirty`, so an \
             unsaved editor buffer is discarded with no prompt: {unguarded:?}"
        );
    }

    /// File -> Save is wired straight to the editor rather than through a `ui`
    /// request counter, so the sibling test above cannot see it. Both ends have to
    /// exist: a handler calling `ws.saveEditor()` with nothing registering a save
    /// hook is a menu item that silently does nothing.
    #[test]
    fn file_save_reaches_a_registered_editor() {
        const MENU_TS: &str = include_str!("../../src/lib/menu.ts");
        const STATE_TS: &str = include_str!("../../src/lib/state.svelte.ts");
        assert!(
            MENU_TS.contains("ws.saveEditor()"),
            "the file.save handler no longer calls ws.saveEditor()"
        );
        assert!(
            STATE_TS.contains("async saveEditor("),
            "Workspace.saveEditor is gone; File -> Save would be a no-op"
        );

        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/lib/viewers");
        let registrars: Vec<String> = std::fs::read_dir(&src)
            .expect("read viewers dir")
            .filter_map(|e| {
                let path = e.expect("dir entry").path();
                let body = std::fs::read_to_string(&path).ok()?;
                body.contains("ws.registerEditor(")
                    .then(|| path.file_name()?.to_str().map(str::to_owned))?
            })
            .collect();
        assert!(
            !registrars.is_empty(),
            "no viewer calls ws.registerEditor(), so File -> Save can never save anything"
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
            let (prefix, rest) =
                c.id.split_once('.')
                    .unwrap_or_else(|| panic!("{:?} is not `<group>.<name>`", c.id));
            assert!(
                !prefix.is_empty() && prefix.chars().all(|ch| ch.is_ascii_lowercase()),
                "{:?} has a non-lowercase group",
                c.id
            );
            assert!(
                !rest.is_empty()
                    && rest
                        .chars()
                        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_'),
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
            assert!(
                !p.contains(':'),
                "dynamic prefix {p:?} must not contain ':'"
            );
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
    /// Edit items use them.
    #[test]
    fn menu_accelerators_are_unique() {
        const RESERVED: &[&str] = &[
            "CmdOrCtrl+C",
            "CmdOrCtrl+V",
            "CmdOrCtrl+X",
            "CmdOrCtrl+A",
            // The contig viewer's "copy the selected bases" shortcut
            // (`ContigView.svelte`). It moved here off a plain Cmd/Ctrl+C once
            // Edit -> Copy claimed that: a menu accelerator is dispatched before
            // the webview sees the key, so the old binding would have gone quietly
            // dead rather than merely double-copying. Reserved so no menu command
            // can take it back.
            "CmdOrCtrl+Shift+C",
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
        const MODS: &[&str] = &[
            "CmdOrCtrl",
            "Cmd",
            "Command",
            "Ctrl",
            "Control",
            "Alt",
            "Shift",
            "Super",
        ];
        for c in COMMANDS {
            let Some(a) = c.accel else { continue };
            let parts: Vec<&str> = a.split('+').collect();
            assert!(!parts.is_empty(), "{:?} has an empty accelerator", c.id);
            let (key, mods) = parts.split_last().unwrap();
            for m in mods {
                assert!(
                    MODS.contains(m),
                    "{:?}: {m:?} is not a modifier (in {a:?})",
                    c.id
                );
            }
            let ok = (key.len() == 1
                && key
                    .chars()
                    .next()
                    .is_some_and(|k| k.is_ascii_alphanumeric()))
                || (key.starts_with('F')
                    && key[1..].parse::<u8>().is_ok_and(|n| (1..=24).contains(&n)))
                || [
                    "Plus",
                    "Minus",
                    "Equal",
                    "Comma",
                    "Period",
                    "Space",
                    "Enter",
                    "Delete",
                    "Backspace",
                ]
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
        assert_eq!(
            placed.len(),
            placed_set.len(),
            "an id appears in two submenus: {placed:?}"
        );

        let declared: HashSet<&str> = COMMANDS.iter().map(|c| c.id).collect();
        let unreachable: Vec<_> = declared.difference(&placed_set).collect();
        assert!(
            unreachable.is_empty(),
            "declared but in no submenu: {unreachable:?}"
        );
        let unknown: Vec<_> = placed_set.difference(&declared).collect();
        assert!(
            unknown.is_empty(),
            "in a submenu but not declared in COMMANDS: {unknown:?}"
        );
    }

    /// A stray separator at the top or bottom of a menu, or two in a row, is the
    /// classic symptom of an item that was removed or conditionally compiled out.
    #[test]
    fn no_submenu_is_empty_or_has_a_dangling_separator() {
        for (title, items) in LAYOUT {
            assert!(!items.is_empty(), "submenu {title:?} is empty");
            assert_ne!(
                items.first(),
                Some(&SEPARATOR),
                "{title:?} starts with a separator"
            );
            assert_ne!(
                items.last(),
                Some(&SEPARATOR),
                "{title:?} ends with a separator"
            );
            for w in items.windows(2) {
                assert!(
                    !(w[0] == SEPARATOR && w[1] == SEPARATOR),
                    "{title:?} has two separators in a row"
                );
            }
        }
    }

    /// Hiding an item on one platform must not leave a separator hanging where
    /// it used to be -- the same symptom the test above guards on the full table.
    #[test]
    fn hiding_platform_items_leaves_no_dangling_separator() {
        for (title, items) in LAYOUT {
            let shown = visible_items(items, MACOS_HIDDEN);
            assert!(!shown.is_empty(), "{title:?} is empty on macOS");
            assert_ne!(
                shown.first(),
                Some(&SEPARATOR),
                "{title:?} starts with a separator on macOS"
            );
            assert_ne!(
                shown.last(),
                Some(&SEPARATOR),
                "{title:?} ends with a separator on macOS"
            );
            assert!(shown
                .windows(2)
                .all(|w| !(w[0] == SEPARATOR && w[1] == SEPARATOR)));
            assert!(shown.iter().all(|it| !MACOS_HIDDEN.contains(it)));
        }
        assert_eq!(visible_items(&["a", SEPARATOR, "b"], &["b"]), vec!["a"]);
        assert_eq!(
            visible_items(&["a", SEPARATOR, "b", SEPARATOR, "c"], &["b"]),
            vec!["a", SEPARATOR, "c"]
        );
        for id in MACOS_HIDDEN {
            assert!(
                COMMANDS.iter().any(|c| c.id == *id),
                "MACOS_HIDDEN names unknown id {id:?}"
            );
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
