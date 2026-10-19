//! The menu bar: the Electron app's 47 items with their shortcuts
//! (src/main.ts:1375-1758), plus a View menu and a Tools menu (UX audit X11).
//!
//! - macOS: the standard app, Edit and Window menus; Open Recent is the system's.
//! - Windows and Linux: the menu bar sits in the window; File has Open Recent (built from
//!   the app's own list), Close and Quit; Help has About. Shortcuts are the Mac ones mapped
//!   by `tandem_protocol::keys::PC` (Ctrl, never Ctrl+Alt, which is AltGr).
//!
//! Each item's id is the action the core runs (see `Action::from_menu_id`).

use tandem_protocol::ui::View;
use tauri::menu::{
    AboutMetadata, Menu, MenuItem, MenuItemBuilder, PredefinedMenuItem, Submenu, SubmenuBuilder,
};
use tauri::{AppHandle, Runtime};

/// This OS's accelerator for a Mac one from the tables below.
pub fn accel(mac: &str) -> Option<String> {
    if cfg!(target_os = "macos") {
        Some(mac.to_string())
    } else {
        tandem_protocol::keys::pc_accelerator(mac).map(str::to_string)
    }
}

/// One custom menu item: (id, label, accelerator).
type Item = (&'static str, &'static str, Option<&'static str>);

pub const FILE: &[Item] = &[
    ("open", "Open…", Some("CmdOrCtrl+O")),
    ("new", "New", Some("CmdOrCtrl+N")), // no dialog follows: no ellipsis
    (
        "edit:new-from-images",
        "New from Images…",
        Some("Alt+CmdOrCtrl+N"),
    ),
    ("-", "", None),
    ("save:in-place", "Save", Some("CmdOrCtrl+S")),
    ("save:as", "Save As…", Some("CmdOrCtrl+Shift+S")),
    ("save:copy", "Save a Copy…", Some("Alt+Shift+CmdOrCtrl+S")),
    ("-", "", None),
    ("print", "Print…", Some("CmdOrCtrl+P")),
    ("edit:export", "Export…", Some("Shift+CmdOrCtrl+E")),
    ("edit:batch", "Batch…", Some("Alt+CmdOrCtrl+B")),
    ("-", "", None),
    ("edit:security", "Security…", Some("Alt+CmdOrCtrl+S")),
    ("edit:cert-sign-open", "Sign Digitally…", None), // the chooser's words
    ("edit:verify-open", "Verify Signatures…", None),
    ("edit:compare", "Compare with…", Some("Alt+CmdOrCtrl+K")),
];

pub const EDIT_TAIL: &[Item] = &[("edit:content-edit", "Existing Content…", None)];

pub const REDACT: &[Item] = &[
    ("edit:redact-area", "Add Redaction Box", None),
    ("edit:redact-search", "Find Text to Redact…", None),
    ("edit:redact-apply-open", "Redact Marked Areas…", None),
];

pub const INSERT: &[Item] = &[
    ("edit:note", "Note", Some("Shift+CmdOrCtrl+N")),
    (
        "edit:highlight",
        "Highlight Match",
        Some("Shift+CmdOrCtrl+H"),
    ),
    ("edit:freetext", "Text Box", Some("Shift+CmdOrCtrl+L")),
    ("edit:text-add-open", "Text as Page Content…", None),
    ("-", "", None),
    ("edit:bookmark", "Bookmark…", Some("Shift+CmdOrCtrl+B")),
    ("edit:link", "Link to Page…", Some("Shift+CmdOrCtrl+K")),
    ("-", "", None),
    ("edit:square", "Square", Some("Alt+Shift+CmdOrCtrl+G")), // ⇧⌘G is Find Previous (Mac standard)
    ("edit:circle", "Circle", Some("Shift+CmdOrCtrl+O")),
    ("edit:line", "Line", Some("Shift+CmdOrCtrl+\\")),
    ("edit:ink", "Ink", Some("Shift+CmdOrCtrl+I")),
    ("edit:stamp", "Stamp…", Some("Shift+CmdOrCtrl+M")),
    (
        "edit:sign-place",
        "Add My Signature",
        Some("Shift+CmdOrCtrl+J"),
    ),
];

pub const PAGE: &[Item] = &[
    (
        "edit:rotate-right",
        "Rotate Right",
        Some("Shift+CmdOrCtrl+]"),
    ),
    ("edit:rotate-left", "Rotate Left", Some("Shift+CmdOrCtrl+[")),
    ("edit:move-up", "Move Page Up", Some("Alt+CmdOrCtrl+Up")),
    (
        "edit:move-down",
        "Move Page Down",
        Some("Alt+CmdOrCtrl+Down"),
    ),
    (
        "edit:delete-page",
        "Delete Page",
        Some("Shift+CmdOrCtrl+Backspace"),
    ),
    ("-", "", None),
    (
        "edit:insert",
        "Insert Pages from File…",
        Some("Alt+CmdOrCtrl+I"),
    ),
    ("edit:extract", "Extract Pages…", Some("Alt+CmdOrCtrl+E")),
    ("-", "", None),
    ("edit:watermark", "Watermark…", Some("Alt+CmdOrCtrl+W")),
    (
        "edit:header-footer",
        "Header and Footer…",
        Some("Alt+CmdOrCtrl+F"),
    ),
    ("edit:labels", "Page Labels…", Some("Alt+CmdOrCtrl+L")),
];

/// New in the Rust app (X11): zoom, panels and the tools, all from the menu bar.
pub const VIEW: &[Item] = &[
    ("view:zoom-in", "Zoom In", Some("CmdOrCtrl+=")),
    ("view:zoom-out", "Zoom Out", Some("CmdOrCtrl+-")),
    ("view:actual-size", "Actual Size", Some("CmdOrCtrl+0")),
    ("view:fit-width", "Fit Width", Some("CmdOrCtrl+9")),
    ("view:fit-page", "Fit Page", Some("CmdOrCtrl+8")),
    ("-", "", None),
    (
        "view:sidebar",
        "Show or Hide Sidebar",
        Some("Alt+CmdOrCtrl+1"),
    ),
    ("view:panel", "Show or Hide Panel", Some("Alt+CmdOrCtrl+2")),
    ("-", "", None),
    ("view:find", "Find…", Some("CmdOrCtrl+F")),
    ("view:find-next", "Find Next", Some("CmdOrCtrl+G")),
    (
        "view:find-previous",
        "Find Previous",
        Some("Shift+CmdOrCtrl+G"),
    ),
    ("-", "", None),
    (
        "view:prev-page",
        "Previous Page",
        Some("Alt+CmdOrCtrl+Left"),
    ),
    ("view:next-page", "Next Page", Some("Alt+CmdOrCtrl+Right")),
];

/// The tool strip's tools (§3.2), in strip order, with ⌃⌘ digits.
pub const TOOLS: &[Item] = &[
    ("tool:select", "Select", Some("Ctrl+CmdOrCtrl+1")),
    ("tool:highlight", "Highlight", Some("Ctrl+CmdOrCtrl+2")),
    ("tool:draw", "Draw", Some("Ctrl+CmdOrCtrl+3")),
    ("tool:note", "Note", Some("Ctrl+CmdOrCtrl+4")),
    ("tool:text", "Text", Some("Ctrl+CmdOrCtrl+5")),
    ("tool:fill-sign", "Fill & Sign", Some("Ctrl+CmdOrCtrl+6")),
    ("tool:redact", "Redact", Some("Ctrl+CmdOrCtrl+7")),
    ("tool:pages", "Pages", Some("Ctrl+CmdOrCtrl+8")),
];

/// ⌘? is set natively after the menu is built (`macos::help_key`): the accelerator parser
/// knows key codes only, and "?" is a shifted "/".
pub const HELP: &[Item] = &[("edit:help", HELP_LABEL, None)];
pub const HELP_LABEL: &str = "Tandem PDF Help";

fn add_items<'a, R: Runtime>(
    mut builder: SubmenuBuilder<'a, R, AppHandle<R>>,
    app: &AppHandle<R>,
    items: &[Item],
) -> tauri::Result<SubmenuBuilder<'a, R, AppHandle<R>>> {
    for (id, label, accel) in items {
        if *id == "-" {
            builder = builder.separator();
            continue;
        }
        let mut item = MenuItemBuilder::with_id(*id, *label);
        if let Some(accel) = accel.and_then(self::accel) {
            item = item.accelerator(accel);
        }
        builder = builder.item(&item.build(app)?);
    }
    Ok(builder)
}

fn undo_redo<'a, R: Runtime>(
    builder: SubmenuBuilder<'a, R, AppHandle<R>>,
    app: &AppHandle<R>,
) -> tauri::Result<SubmenuBuilder<'a, R, AppHandle<R>>> {
    let mut builder = builder;
    for (id, label, mac) in [
        ("edit:undo", "Undo", "CmdOrCtrl+Z"),
        ("edit:redo", "Redo", "Shift+CmdOrCtrl+Z"),
    ] {
        let mut item = MenuItemBuilder::with_id(id, label);
        if let Some(a) = accel(mac) {
            item = item.accelerator(a);
        }
        builder = builder.item(&item.build(app)?);
    }
    Ok(builder)
}

/// Windows and Linux: File ▸ Close and Quit (Exit on Windows) are the app's own items
/// (the predefined ones do not exist on Linux), and Help ▸ Tandem PDF Help is F1.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub const PC_FILE_END: &[Item] = &[
    ("app:close", "Close Window", Some("Ctrl+W")),
    (
        "app:quit",
        if cfg!(windows) { "Exit" } else { "Quit" },
        if cfg!(windows) { None } else { Some("Ctrl+Q") },
    ),
];
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub const PC_HELP_KEY: &str = "F1";

/// The Open Recent submenu's id; its items are `recent:<n>` and `recent:clear`.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub const RECENT_ID: &str = "file:recent";

/// Builds the whole menu bar (Windows and Linux).
#[cfg(not(target_os = "macos"))]
pub fn build<R: Runtime>(app: &AppHandle<R>, about: AboutMetadata<'_>) -> tauri::Result<Menu<R>> {
    let recent = SubmenuBuilder::with_id(app, RECENT_ID, "Open Recent")
        .item(
            &MenuItemBuilder::with_id("recent:none", "No Recent Files")
                .enabled(false)
                .build(app)?,
        )
        .build()?;
    let file = add_items(SubmenuBuilder::new(app, "File"), app, &FILE[..1])?.item(&recent);
    let file = add_items(file, app, &FILE[1..])?.separator();
    let mut file = file;
    for (id, label, key) in PC_FILE_END {
        let mut item = MenuItemBuilder::with_id(*id, *label);
        if let Some(key) = key {
            item = item.accelerator(*key);
        }
        file = file.item(&item.build(app)?);
    }
    let file = file.build()?;

    let redact = add_items(SubmenuBuilder::new(app, "Redact"), app, REDACT)?.build()?;
    let edit = undo_redo(SubmenuBuilder::new(app, "Edit"), app)?
        .separator()
        .item(&PredefinedMenuItem::cut(app, None)?)
        .item(&PredefinedMenuItem::copy(app, None)?)
        .item(&PredefinedMenuItem::paste(app, None)?)
        .item(&PredefinedMenuItem::select_all(app, None)?)
        .separator();
    let edit = add_items(edit, app, EDIT_TAIL)?.item(&redact).build()?;
    let view = add_items(SubmenuBuilder::new(app, "View"), app, VIEW)?.build()?;
    let tools = add_items(SubmenuBuilder::new(app, "Tools"), app, TOOLS)?.build()?;
    let insert = add_items(SubmenuBuilder::new(app, "Insert"), app, INSERT)?.build()?;
    let page = add_items(SubmenuBuilder::new(app, "Page"), app, PAGE)?.build()?;
    let help = SubmenuBuilder::new(app, "Help")
        .item(
            &MenuItemBuilder::with_id("edit:help", HELP_LABEL)
                .accelerator(PC_HELP_KEY)
                .build(app)?,
        )
        .separator()
        .item(&PredefinedMenuItem::about(
            app,
            Some("About Tandem PDF"),
            Some(about),
        )?)
        .build()?;
    Menu::with_items(app, &[&file, &edit, &view, &tools, &insert, &page, &help])
}

/// Windows and Linux: File ▸ Open Recent from the app's list (the start screen's), rebuilt
/// when the list changes. Kept in the app's state.
#[cfg(not(target_os = "macos"))]
pub struct RecentMenu {
    shown: std::sync::Mutex<Option<Vec<String>>>,
}

#[cfg(not(target_os = "macos"))]
impl RecentMenu {
    pub fn new() -> Self {
        RecentMenu {
            shown: std::sync::Mutex::new(None),
        }
    }

    /// The path of `recent:<n>`.
    pub fn path(&self, id: &str) -> Option<String> {
        let n: usize = id.strip_prefix("recent:")?.parse().ok()?;
        self.shown.lock().ok()?.as_ref()?.get(n).cloned()
    }

    /// Shows `files` (paths and names) when they differ from what is shown.
    pub fn update<R: Runtime>(&self, app: &AppHandle<R>, files: &[(String, String)]) {
        let paths: Vec<String> = files.iter().map(|(p, _)| p.clone()).collect();
        let Ok(mut shown) = self.shown.lock() else {
            return;
        };
        if shown.as_ref() == Some(&paths) {
            return;
        }
        let Some(menu) = app.menu() else {
            return;
        };
        let Some(sub) = find_submenu(&menu, RECENT_ID) else {
            return;
        };
        if let Ok(items) = sub.items() {
            for item in items {
                let _ = sub.remove(&item);
            }
        }
        if files.is_empty() {
            if let Ok(item) = MenuItemBuilder::with_id("recent:none", "No Recent Files")
                .enabled(false)
                .build(app)
            {
                let _ = sub.append(&item);
            }
        } else {
            for (n, (path, name)) in files.iter().enumerate() {
                // The file name, and its folder when two files share a name.
                let twin = files.iter().filter(|(_, other)| other == name).count() > 1;
                let label = if twin { path.clone() } else { name.clone() };
                if let Ok(item) = MenuItemBuilder::with_id(format!("recent:{n}"), label).build(app)
                {
                    let _ = sub.append(&item);
                }
            }
            if let Ok(sep) = PredefinedMenuItem::separator(app) {
                let _ = sub.append(&sep);
            }
            if let Ok(item) = MenuItemBuilder::with_id("recent:clear", "Clear Menu").build(app) {
                let _ = sub.append(&item);
            }
        }
        *shown = Some(paths);
    }
}

#[cfg(not(target_os = "macos"))]
fn find_submenu<R: Runtime>(menu: &Menu<R>, id: &str) -> Option<Submenu<R>> {
    fn walk<R: Runtime>(sub: &Submenu<R>, id: &str) -> Option<Submenu<R>> {
        for kind in sub.items().ok()? {
            if let Some(inner) = kind.as_submenu() {
                if inner.id().as_ref() == id {
                    return Some(inner.clone());
                }
                if let Some(found) = walk(inner, id) {
                    return Some(found);
                }
            }
        }
        None
    }
    for kind in menu.items().ok()? {
        if let Some(sub) = kind.as_submenu() {
            if sub.id().as_ref() == id {
                return Some(sub.clone());
            }
            if let Some(found) = walk(sub, id) {
                return Some(found);
            }
        }
    }
    None
}

/// Builds the whole menu bar (macOS).
#[cfg(target_os = "macos")]
pub fn build<R: Runtime>(app: &AppHandle<R>, about: AboutMetadata<'_>) -> tauri::Result<Menu<R>> {
    let app_menu = SubmenuBuilder::new(app, "Tandem PDF")
        .item(&PredefinedMenuItem::about(
            app,
            Some("About Tandem PDF"),
            Some(about),
        )?)
        .separator()
        .item(&PredefinedMenuItem::services(app, None)?)
        .separator()
        .item(&PredefinedMenuItem::hide(app, None)?)
        .item(&PredefinedMenuItem::hide_others(app, None)?)
        .item(&PredefinedMenuItem::show_all(app, None)?)
        .separator()
        .item(&PredefinedMenuItem::quit(app, None)?)
        .build()?;

    let file = add_items(SubmenuBuilder::new(app, "File"), app, FILE)?
        .separator()
        .item(&PredefinedMenuItem::close_window(app, None)?)
        .build()?;

    let redact = add_items(SubmenuBuilder::new(app, "Redact"), app, REDACT)?.build()?;
    let edit = undo_redo(SubmenuBuilder::new(app, "Edit"), app)?
        .separator()
        .item(&PredefinedMenuItem::cut(app, None)?)
        .item(&PredefinedMenuItem::copy(app, None)?)
        .item(&PredefinedMenuItem::paste(app, None)?)
        .item(&PredefinedMenuItem::select_all(app, None)?)
        .separator();
    let edit = add_items(edit, app, EDIT_TAIL)?.item(&redact).build()?;

    let view = add_items(SubmenuBuilder::new(app, "View"), app, VIEW)?.build()?;
    let tools = add_items(SubmenuBuilder::new(app, "Tools"), app, TOOLS)?.build()?;
    let insert = add_items(SubmenuBuilder::new(app, "Insert"), app, INSERT)?.build()?;
    let page = add_items(SubmenuBuilder::new(app, "Page"), app, PAGE)?.build()?;
    let window: Submenu<R> = SubmenuBuilder::new(app, "Window")
        .item(&PredefinedMenuItem::minimize(app, None)?)
        .item(&PredefinedMenuItem::maximize(app, None)?)
        .separator()
        .item(&PredefinedMenuItem::fullscreen(app, None)?)
        .build()?;
    let help = add_items(SubmenuBuilder::new(app, "Help"), app, HELP)?.build()?;

    Menu::with_items(
        app,
        &[
            &app_menu, &file, &edit, &view, &tools, &insert, &page, &window, &help,
        ],
    )
}

/// Every custom item of the menu bar by id, with the enabled state last set, so a state
/// update touches only the items that change (each change is a trip to the main thread).
pub struct MenuItems<R: Runtime> {
    items: std::sync::Mutex<std::collections::HashMap<String, (MenuItem<R>, bool)>>,
}

impl<R: Runtime> MenuItems<R> {
    /// Collects the plain items of `menu` (`Menu::get` sees only the top level).
    pub fn collect(menu: &Menu<R>) -> Self {
        let mut items = std::collections::HashMap::new();
        if let Ok(kinds) = menu.items() {
            for kind in kinds {
                if let Some(sub) = kind.as_submenu() {
                    collect_in(sub, &mut items);
                }
            }
        }
        MenuItems {
            items: std::sync::Mutex::new(items),
        }
    }

    /// Enables or disables items by id; unknown ids are ignored.
    pub fn set_enabled<'a>(&self, wanted: impl IntoIterator<Item = (&'a str, bool)>) {
        let Ok(mut items) = self.items.lock() else {
            return;
        };
        for (id, on) in wanted {
            if let Some((item, last)) = items.get_mut(id)
                && *last != on
                && item.set_enabled(on).is_ok()
            {
                *last = on;
            }
        }
    }
}

fn collect_in<R: Runtime>(
    sub: &Submenu<R>,
    out: &mut std::collections::HashMap<String, (MenuItem<R>, bool)>,
) {
    let Ok(kinds) = sub.items() else {
        return;
    };
    for kind in kinds {
        if let Some(item) = kind.as_menuitem() {
            let on = item.is_enabled().unwrap_or(true);
            out.insert(kind.id().as_ref().to_string(), (item.clone(), on));
        } else if let Some(inner) = kind.as_submenu() {
            collect_in(inner, out);
        }
    }
}

/// Which items the document allows (UX: a menu item that cannot act is greyed, not
/// silently ignored). The Save items are the core's (`Platform::set_document_state`).
pub fn enabled_for(view: &View) -> Vec<(&'static str, bool)> {
    let doc = view.doc.as_ref().filter(|_| view.screen == "editor");
    let editor = doc.is_some();
    let tool = |id: &str| {
        editor
            && view
                .tool
                .available
                .iter()
                .find(|a| a.id == id)
                .is_none_or(|a| a.enabled)
    };
    let pages = doc.map_or(0, |d| d.page_count);
    let mut out = vec![
        ("edit:undo", doc.is_some_and(|d| d.can_undo)),
        ("edit:redo", doc.is_some_and(|d| d.can_redo)),
        ("view:prev-page", editor && view.page > 0),
        ("view:next-page", editor && view.page + 1 < pages),
        ("view:find-next", editor && !view.search.query.is_empty()),
        (
            "view:find-previous",
            editor && !view.search.query.is_empty(),
        ),
        ("edit:content-edit", tool("text")),
        ("edit:text-add-open", tool("text")),
        ("edit:note", tool("note")),
        ("edit:freetext", tool("note")),
        ("edit:highlight", tool("highlight")),
    ];
    for id in [
        "edit:square",
        "edit:circle",
        "edit:line",
        "edit:ink",
        "edit:stamp",
    ] {
        out.push((id, tool("draw")));
    }
    for (id, ..) in REDACT {
        out.push((id, tool("redact")));
    }
    for (id, ..) in PAGE.iter().filter(|i| i.0 != "-") {
        out.push((
            id,
            if *id == "edit:extract" {
                editor
            } else {
                tool("pages")
            },
        ));
    }
    for (id, ..) in TOOLS {
        out.push((id, tool(&id[5..])));
    }
    // Everything else that needs a document.
    for id in [
        "print",
        "edit:export",
        "edit:security",
        "edit:cert-sign-open",
        "edit:verify-open",
        "edit:compare",
        "edit:bookmark",
        "edit:link",
        "edit:sign-place",
    ] {
        out.push((id, editor));
    }
    for (id, ..) in VIEW.iter().filter(|i| i.0 != "-") {
        if !out.iter().any(|(o, _)| o == id) {
            out.push((id, editor));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Electron app's 47 items: 14 File (with Close), 7 Edit, 3 Redact,
    /// 12 Insert, 10 Page, 1 Help. Custom ones here; the role items (Close,
    /// Cut, Copy, Paste, Select All) are added in `build`, Undo/Redo too.
    #[test]
    fn forty_seven_items() {
        let custom = |items: &[Item]| items.iter().filter(|i| i.0 != "-").count();
        let file = custom(FILE) + 1; // + Close
        let edit = 2 + 4 + custom(EDIT_TAIL); // Undo, Redo, cut/copy/paste/selectAll, Existing Content
        let total = file + edit + custom(REDACT) + custom(INSERT) + custom(PAGE) + custom(HELP);
        assert_eq!(
            (
                file,
                edit,
                custom(REDACT),
                custom(INSERT),
                custom(PAGE),
                custom(HELP)
            ),
            (14, 7, 3, 12, 10, 1)
        );
        assert_eq!(total, 47);
    }

    /// Every Mac shortcut has a decided Windows/Linux one (or none), and those are unique
    /// with Close, Quit and F1 too, and never Ctrl+Alt (AltGr).
    #[test]
    fn pc_shortcuts_are_decided_and_unique() {
        use tandem_protocol::keys::{chord, pc_accelerator, pc_listed};
        let mut seen = std::collections::BTreeSet::new();
        let tables: [&[Item]; 8] = [FILE, EDIT_TAIL, REDACT, INSERT, PAGE, VIEW, TOOLS, HELP];
        let mut macs: Vec<&str> = vec!["CmdOrCtrl+Z", "Shift+CmdOrCtrl+Z"];
        for items in tables {
            macs.extend(items.iter().filter_map(|(_, _, a)| *a));
        }
        for mac in macs {
            assert!(
                pc_listed(mac),
                "{mac} has no Windows/Linux decision in keys::PC"
            );
            if let Some(pc) = pc_accelerator(mac) {
                assert!(seen.insert(chord(pc)), "{pc} used twice");
            }
        }
        for (_, _, key) in PC_FILE_END {
            if let Some(key) = key {
                assert!(seen.insert(chord(key)), "{key} used twice");
            }
        }
        assert!(seen.insert(chord(PC_HELP_KEY)));
        for c in &seen {
            assert!(!(c.ctrl && c.alt), "{c:?} is AltGr");
        }
        // Windows and Linux reserve or expect these elsewhere.
        for reserved in [
            "Alt+F4",
            "Ctrl+Alt+Delete",
            "Ctrl+Shift+Escape",
            "F10",
            "F11",
        ] {
            assert!(!seen.contains(&chord(reserved)), "{reserved} is reserved");
        }
    }

    #[test]
    fn shortcuts_are_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for items in [FILE, EDIT_TAIL, REDACT, INSERT, PAGE, VIEW, TOOLS, HELP] {
            for (_, label, accel) in items.iter() {
                if let Some(a) = accel {
                    assert!(seen.insert(*a), "{a} used twice ({label})");
                }
            }
        }
        // macOS reserves these (docs/gotchas.md, Electron/macOS).
        for reserved in [
            "CmdOrCtrl+H",
            "Alt+CmdOrCtrl+H",
            "CmdOrCtrl+M",
            "Alt+CmdOrCtrl+M",
            "CmdOrCtrl+Q",
            "CmdOrCtrl+W",
            "CmdOrCtrl+,",
        ] {
            assert!(!seen.contains(reserved), "{reserved} is reserved by macOS");
        }
    }
}
