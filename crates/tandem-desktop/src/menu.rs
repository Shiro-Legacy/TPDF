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

/// A label as the menu shows it: Windows and Linux read "&" as an underlined access key
/// ("Fill & Sign"), so it is doubled there.
pub fn label_text(label: &str) -> String {
    if cfg!(target_os = "macos") {
        label.to_string()
    } else {
        label.replace('&', "&&")
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
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
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
        let mut item = MenuItemBuilder::with_id(*id, label_text(label));
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
///
/// As with [`MenuItems`], `shown` is never held across a menu call (Windows' page keys read
/// it on the main thread); `apply` keeps rebuilds in order, off the main thread.
pub struct RecentMenu {
    shown: std::sync::Mutex<Option<Vec<String>>>,
    apply: std::sync::Mutex<()>,
}

#[cfg(not(target_os = "macos"))]
impl RecentMenu {
    pub fn new() -> Self {
        RecentMenu {
            shown: std::sync::Mutex::new(None),
            apply: std::sync::Mutex::new(()),
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
        let Ok(_order) = self.apply.lock() else {
            return;
        };
        if self
            .shown
            .lock()
            .map_or(true, |s| s.as_ref() == Some(&paths))
        {
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
                let label = label_text(if twin { path } else { name });
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
        if let Ok(mut shown) = self.shown.lock() {
            *shown = Some(paths);
        }
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

/// Windows and Linux: every menu item with a shortcut, as (accelerator, id).
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn pc_shortcuts() -> Vec<(&'static str, &'static str)> {
    use tandem_protocol::keys::pc_accelerator;
    let mut out = Vec::new();
    for (id, mac) in [
        ("edit:undo", "CmdOrCtrl+Z"),
        ("edit:redo", "Shift+CmdOrCtrl+Z"),
    ] {
        out.extend(pc_accelerator(mac).map(|pc| (pc, id)));
    }
    for items in [FILE, EDIT_TAIL, REDACT, INSERT, PAGE, VIEW, TOOLS] {
        for (id, _, mac) in items {
            out.extend(mac.and_then(pc_accelerator).map(|pc| (pc, *id)));
        }
    }
    for (id, _, key) in PC_FILE_END {
        out.extend(key.map(|k| (k, *id)));
    }
    out.push((PC_HELP_KEY, "edit:help"));
    out
}

/// Windows: whether holding an item's shortcut down repeats it (paging, zoom, undo).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn repeats(id: &str) -> bool {
    matches!(
        id,
        "view:prev-page"
            | "view:next-page"
            | "view:zoom-in"
            | "view:zoom-out"
            | "edit:undo"
            | "edit:redo"
    )
}

/// Windows: the menu item for a key pressed in the page (`vk`, a Win32 virtual-key code,
/// with the modifiers held), if a menu item has that shortcut.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn pc_item_for_key(vk: u32, ctrl: bool, shift: bool, alt: bool) -> Option<&'static str> {
    use tandem_protocol::keys::chord;
    let key = match vk {
        0x30..=0x39 | 0x41..=0x5A => char::from_u32(vk)?.to_string(),
        0x70..=0x87 => format!("F{}", vk - 0x6F),
        0x21 => "PageUp".into(),
        0x22 => "PageDown".into(),
        0x25 => "Left".into(),
        0x26 => "Up".into(),
        0x27 => "Right".into(),
        0x28 => "Down".into(),
        0x08 => "Backspace".into(),
        // The = and - keys, and the number pad's + and -.
        0xBB | 0x6B => "=".into(),
        0xBD | 0x6D => "-".into(),
        0xDB => "[".into(),
        0xDC => "\\".into(),
        0xDD => "]".into(),
        _ => return None,
    };
    // Ctrl++ as people type it (Shift and =) zooms in like Ctrl+=.
    let shift = shift && !(key == "=" && ctrl && !alt);
    pc_shortcuts()
        .into_iter()
        .find(|(accel, _)| {
            let c = chord(accel);
            !c.cmd
                && (c.ctrl, c.alt, c.shift) == (ctrl, alt, shift)
                && c.key.eq_ignore_ascii_case(&key)
        })
        .map(|(_, id)| id)
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
///
/// Never hold `items` across a menu call: off the main thread each one waits for the main
/// thread, which itself reads `items` for Windows' page keys ([`MenuItems::enabled`]).
/// `apply` keeps updates in order instead; it is only taken off the main thread.
pub struct MenuItems<R: Runtime> {
    items: std::sync::Mutex<std::collections::HashMap<String, (MenuItem<R>, bool)>>,
    apply: std::sync::Mutex<()>,
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
            apply: std::sync::Mutex::new(()),
        }
    }

    /// Whether the item `id` is enabled as last set (unknown ids: no).
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn enabled(&self, id: &str) -> bool {
        self.items
            .lock()
            .ok()
            .and_then(|items| items.get(id).map(|(_, on)| *on))
            .unwrap_or(false)
    }

    /// Enables or disables items by id; unknown ids are ignored. Not on the main thread.
    pub fn set_enabled<'a>(&self, wanted: impl IntoIterator<Item = (&'a str, bool)>) {
        let Ok(_order) = self.apply.lock() else {
            return;
        };
        let changes: Vec<(String, MenuItem<R>, bool)> = {
            let Ok(items) = self.items.lock() else {
                return;
            };
            wanted
                .into_iter()
                .filter_map(|(id, on)| {
                    let (item, last) = items.get(id)?;
                    (*last != on).then(|| (id.to_string(), item.clone(), on))
                })
                .collect()
        };
        for (id, item, on) in changes {
            if item.set_enabled(on).is_ok()
                && let Ok(mut items) = self.items.lock()
                && let Some((_, last)) = items.get_mut(&id)
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

    /// Windows: key presses in the page find their menu items (virtual-key codes).
    #[test]
    fn page_keys_find_menu_items() {
        assert_eq!(pc_item_for_key(0x4F, true, false, false), Some("open")); // Ctrl+O
        assert_eq!(pc_item_for_key(0x53, true, true, false), Some("save:as")); // Ctrl+Shift+S
        assert_eq!(pc_item_for_key(0x5A, true, false, false), Some("edit:undo"));
        assert_eq!(
            pc_item_for_key(0x70, false, false, false),
            Some("edit:help")
        ); // F1
        assert_eq!(pc_item_for_key(0x57, true, false, false), Some("app:close")); // Ctrl+W
        assert_eq!(
            pc_item_for_key(0xBB, true, false, false),
            Some("view:zoom-in")
        );
        assert_eq!(pc_item_for_key(0xDC, true, true, false), Some("edit:line")); // Ctrl+Shift+\\
        assert_eq!(
            pc_item_for_key(0x31, true, true, false),
            Some("tool:select")
        );
        assert_eq!(
            pc_item_for_key(0x22, true, false, false),
            Some("view:next-page")
        );
        assert_eq!(
            pc_item_for_key(0x21, true, true, false),
            Some("edit:move-up")
        );
        assert_eq!(
            pc_item_for_key(0x08, true, true, false),
            Some("edit:delete-page")
        );
        // Ctrl++ (Shift+=) and the number pad zoom too.
        assert_eq!(
            pc_item_for_key(0xBB, true, true, false),
            Some("view:zoom-in")
        );
        assert_eq!(
            pc_item_for_key(0x6B, true, false, false),
            Some("view:zoom-in")
        );
        assert_eq!(
            pc_item_for_key(0x6D, true, false, false),
            Some("view:zoom-out")
        );
        // AltGr is Ctrl+Alt: never a shortcut. Plain letters and Ctrl+C stay the page's.
        assert_eq!(pc_item_for_key(0x45, true, false, true), None);
        assert_eq!(pc_item_for_key(0x4F, false, false, false), None);
        assert_eq!(pc_item_for_key(0x43, true, false, false), None);
        // Every shortcut is reachable from some key.
        assert!(pc_shortcuts().len() > 40);
        let fill = if cfg!(target_os = "macos") {
            "Fill & Sign"
        } else {
            "Fill && Sign"
        };
        assert_eq!(label_text("Fill & Sign"), fill);
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
