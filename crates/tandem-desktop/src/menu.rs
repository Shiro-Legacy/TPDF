//! The macOS menu bar: the Electron app's 47 items with their shortcuts
//! (src/main.ts:1375-1758), plus a View menu (UX audit X11) and the standard
//! app, Edit and Window items macOS users expect.
//!
//! Each item's id is the action the core runs (see `Action::from_menu_id`).

use tandem_protocol::ui::View;
use tauri::menu::{
    AboutMetadata, Menu, MenuItem, MenuItemBuilder, PredefinedMenuItem, Submenu, SubmenuBuilder,
};
use tauri::{AppHandle, Runtime};

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
        if let Some(accel) = accel {
            item = item.accelerator(*accel);
        }
        builder = builder.item(&item.build(app)?);
    }
    Ok(builder)
}

/// Builds the whole menu bar.
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
    let edit = SubmenuBuilder::new(app, "Edit")
        .item(
            &MenuItemBuilder::with_id("edit:undo", "Undo")
                .accelerator("CmdOrCtrl+Z")
                .build(app)?,
        )
        .item(
            &MenuItemBuilder::with_id("edit:redo", "Redo")
                .accelerator("Shift+CmdOrCtrl+Z")
                .build(app)?,
        )
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
