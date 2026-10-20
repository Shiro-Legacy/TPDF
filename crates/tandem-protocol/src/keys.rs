//! Keyboard shortcuts on each OS, in one place for the menu bar (tandem-desktop) and the
//! window's hints (tandem-ui).
//!
//! The menus are written with the Mac's accelerators (`Alt+CmdOrCtrl+K`). On Windows and
//! Linux [`pc_accelerator`] gives each its own: `CmdOrCtrl` becomes `Ctrl`, but a Mac
//! shortcut that uses Option (⌥) does not become `Ctrl+Alt`, because `Ctrl+Alt` is AltGr on
//! many keyboards (German AltGr+E is €, Polish AltGr+S is ś) and a menu accelerator would
//! swallow the character. Those get an F-key or `Ctrl+Shift` equivalent, or no shortcut (the
//! item stays in its menu).
//!
//! The window's hints are written with the Mac's symbols (`⌥⌘1`); [`hint`] shows them as
//! they are on a Mac and as the matching Windows/Linux shortcut elsewhere.

use serde::{Deserialize, Serialize};

/// Which keys a person has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keys {
    Mac,
    /// Windows and Linux.
    Pc,
}

/// The OS the window runs on. The core is built for each OS and says which
/// (`View::os`); the UI is WebAssembly and cannot tell by itself. It decides the keys
/// ([`Keys`]) and a few words (the file manager's name, "this Mac").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Os {
    #[default]
    Mac,
    Windows,
    Linux,
}

impl Os {
    /// The OS this code was built for. Only meaningful in native code (the core); the
    /// WebAssembly UI reads the core's answer instead.
    pub fn native() -> Os {
        if cfg!(target_os = "macos") {
            Os::Mac
        } else if cfg!(windows) {
            Os::Windows
        } else {
            Os::Linux
        }
    }

    pub fn keys(self) -> Keys {
        match self {
            Os::Mac => Keys::Mac,
            Os::Windows | Os::Linux => Keys::Pc,
        }
    }

    /// The button that shows a saved file in the file manager.
    pub fn show_in_folder(self) -> &'static str {
        match self {
            Os::Mac => "Show in Finder",
            Os::Windows => "Show in Explorer",
            Os::Linux => "Show in Folder",
        }
    }

    /// Words written for a Mac, for this OS: unchanged on a Mac; elsewhere key symbols in
    /// running text become key names ([`pc_text`]) and "this Mac" becomes "this computer".
    pub fn text(self, mac: &str) -> String {
        match self.keys() {
            Keys::Mac => mac.to_string(),
            Keys::Pc => pc_text(mac).replace("this Mac", "this computer"),
        }
    }

    /// A menu shortcut's hint (`"⌥⌘1"`) on this OS, or `None` when it has none here.
    pub fn hint(self, symbols: &str) -> Option<String> {
        hint(symbols, self.keys())
    }

    /// A tooltip naming a menu shortcut: `"Sidebar (⌥⌘1)"` on a Mac, `"Sidebar (F9)"` on
    /// Windows/Linux, just `"Sidebar"` where the item has no shortcut.
    pub fn titled(self, label: &str, symbols: &str) -> String {
        match self.hint(symbols) {
            Some(keys) => format!("{label} ({keys})"),
            None => label.to_string(),
        }
    }

    /// One row of keys as the help lists them (`["⌘O", "⌘N"]` joined by `sep`): as written
    /// on a Mac; elsewhere each menu shortcut as this OS's (one with none here is left
    /// out) and other keys ("Esc", "Tab") as they are. A space between shortcuts becomes
    /// ", " off the Mac, since PC names have spaces ("Ctrl+Page Up"). `None` when no key
    /// is left.
    pub fn help_keys(self, keys: &[&str], sep: &str) -> Option<String> {
        if self == Os::Mac {
            return Some(keys.join(sep));
        }
        let glyphs = |k: &str| k.contains(['⌘', '⌥', '⇧', '⌃']);
        let named: Vec<String> = keys
            .iter()
            .filter_map(|k| {
                if glyphs(k) {
                    self.hint(k)
                } else {
                    Some(self.text(k))
                }
            })
            .collect();
        let sep = if sep.trim().is_empty() { ", " } else { sep };
        (!named.is_empty()).then(|| named.join(sep))
    }

    /// Whether a key or pointer event has the command modifier: ⌘ on a Mac, Ctrl on
    /// Windows and Linux (where the Windows/Super key belongs to the OS).
    pub fn primary(self, meta: bool, ctrl: bool) -> bool {
        match self.keys() {
            Keys::Mac => meta,
            Keys::Pc => ctrl,
        }
    }

    /// The command modifier's name in `aria-keyshortcuts` ("Meta+F", "Control+F").
    pub fn aria_primary(self) -> &'static str {
        match self.keys() {
            Keys::Mac => "Meta",
            Keys::Pc => "Control",
        }
    }

    /// Whether a key press (the DOM's `KeyboardEvent.key` and modifiers) is one of this
    /// OS's menu shortcuts in [`PC`], which the native menu bar handles: the page must not
    /// act on it as well. Always false on a Mac (its pages keep their keys as they are).
    pub fn menu_chord(self, ctrl: bool, alt: bool, shift: bool, key: &str) -> bool {
        if self.keys() == Keys::Mac {
            return false;
        }
        let key = match key {
            "ArrowUp" => "Up".to_string(),
            "ArrowDown" => "Down".to_string(),
            "ArrowLeft" => "Left".to_string(),
            "ArrowRight" => "Right".to_string(),
            k if k.chars().count() == 1 => k.to_uppercase(),
            k => k.to_string(),
        };
        let pressed = Chord {
            cmd: false,
            ctrl,
            alt,
            shift,
            key,
        };
        PC.iter()
            .filter_map(|(_, pc)| *pc)
            .any(|pc| chord(pc) == pressed)
    }
}

/// The help's keyboard list: (keys as written on a Mac, how they are joined there, what
/// they do). [`Os::help_keys`] and [`Os::text`] give each row on the OS shown.
pub const HELP_KEYS: &[(&[&str], &str, &str)] = &[
    (&["⌘O", "⌘N", "⌘S", "⇧⌘S"], ", ", "Open, New, Save, Save As"),
    (&["⌘Z", "⇧⌘Z"], ", ", "Undo, Redo"),
    (&["⌘F", "⌘G", "⇧⌘G"], ", ", "Find, Find Next, Find Previous"),
    (
        &["⌘+", "⌘−", "⌘0", "⌘9", "⌘8"],
        ", ",
        "Zoom in, zoom out, actual size, fit width, fit page",
    ),
    (
        &["⌃⌘1", "⌃⌘8"],
        " … ",
        "Select, Highlight, Draw, Note, Text, Fill & Sign, Redact, Pages",
    ),
    (&["Esc"], "", "Close what is open, then back to Select"),
    (
        &["⌥⌘1", "⌥⌘2"],
        ", ",
        "Show or hide the sidebar, the side panel",
    ),
    (&["⌥⌘←", "⌥⌘→"], " ", "Previous page, next page"),
    (
        &["Delete", "arrows"],
        ", ",
        "Delete or nudge what is selected on the page (⇧ for bigger steps)",
    ),
    (&["Tab"], "", "Next form field while filling in"),
    (&["⌘?"], "", "This help"),
];

/// The Windows/Linux accelerator for each Mac accelerator the menus use (`None`: no shortcut
/// there). Every accelerator in the menus must be listed (tandem-desktop's menu tests).
pub const PC: &[(&str, Option<&str>)] = &[
    // File
    ("CmdOrCtrl+O", Some("Ctrl+O")),
    ("CmdOrCtrl+N", Some("Ctrl+N")),
    ("Alt+CmdOrCtrl+N", None),
    ("CmdOrCtrl+S", Some("Ctrl+S")),
    ("CmdOrCtrl+Shift+S", Some("Ctrl+Shift+S")),
    ("Alt+Shift+CmdOrCtrl+S", None),
    ("CmdOrCtrl+P", Some("Ctrl+P")),
    ("Shift+CmdOrCtrl+E", Some("Ctrl+Shift+E")),
    ("Alt+CmdOrCtrl+B", None),
    ("Alt+CmdOrCtrl+S", None),
    ("Alt+CmdOrCtrl+K", None),
    // Edit
    ("CmdOrCtrl+Z", Some("Ctrl+Z")),
    ("Shift+CmdOrCtrl+Z", Some("Ctrl+Shift+Z")),
    // Insert
    ("Shift+CmdOrCtrl+N", Some("Ctrl+Shift+N")),
    ("Shift+CmdOrCtrl+H", Some("Ctrl+Shift+H")),
    ("Shift+CmdOrCtrl+L", Some("Ctrl+Shift+L")),
    ("Shift+CmdOrCtrl+B", Some("Ctrl+Shift+B")),
    ("Shift+CmdOrCtrl+K", Some("Ctrl+Shift+K")),
    ("Alt+Shift+CmdOrCtrl+G", None),
    ("Shift+CmdOrCtrl+O", Some("Ctrl+Shift+O")),
    ("Shift+CmdOrCtrl+\\", Some("Ctrl+Shift+\\")),
    ("Shift+CmdOrCtrl+I", Some("Ctrl+Shift+I")),
    ("Shift+CmdOrCtrl+M", Some("Ctrl+Shift+M")),
    ("Shift+CmdOrCtrl+J", Some("Ctrl+Shift+J")),
    // Page
    ("Shift+CmdOrCtrl+]", Some("Ctrl+Shift+]")),
    ("Shift+CmdOrCtrl+[", Some("Ctrl+Shift+[")),
    ("Alt+CmdOrCtrl+Up", Some("Ctrl+Shift+PageUp")),
    ("Alt+CmdOrCtrl+Down", Some("Ctrl+Shift+PageDown")),
    ("Shift+CmdOrCtrl+Backspace", Some("Ctrl+Shift+Backspace")),
    ("Alt+CmdOrCtrl+I", None),
    ("Alt+CmdOrCtrl+E", None),
    ("Alt+CmdOrCtrl+W", None),
    ("Alt+CmdOrCtrl+F", None),
    ("Alt+CmdOrCtrl+L", None),
    // View
    ("CmdOrCtrl+=", Some("Ctrl+=")),
    ("CmdOrCtrl+-", Some("Ctrl+-")),
    ("CmdOrCtrl+0", Some("Ctrl+0")),
    ("CmdOrCtrl+9", Some("Ctrl+9")),
    ("CmdOrCtrl+8", Some("Ctrl+8")),
    ("Alt+CmdOrCtrl+1", Some("F9")),
    ("Alt+CmdOrCtrl+2", Some("Shift+F9")),
    ("CmdOrCtrl+F", Some("Ctrl+F")),
    ("CmdOrCtrl+G", Some("Ctrl+G")),
    ("Shift+CmdOrCtrl+G", Some("Ctrl+Shift+G")),
    ("Alt+CmdOrCtrl+Left", Some("Ctrl+PageUp")),
    ("Alt+CmdOrCtrl+Right", Some("Ctrl+PageDown")),
    // Tools (⌃⌘1–8 on a Mac)
    ("Ctrl+CmdOrCtrl+1", Some("Ctrl+Shift+1")),
    ("Ctrl+CmdOrCtrl+2", Some("Ctrl+Shift+2")),
    ("Ctrl+CmdOrCtrl+3", Some("Ctrl+Shift+3")),
    ("Ctrl+CmdOrCtrl+4", Some("Ctrl+Shift+4")),
    ("Ctrl+CmdOrCtrl+5", Some("Ctrl+Shift+5")),
    ("Ctrl+CmdOrCtrl+6", Some("Ctrl+Shift+6")),
    ("Ctrl+CmdOrCtrl+7", Some("Ctrl+Shift+7")),
    ("Ctrl+CmdOrCtrl+8", Some("Ctrl+Shift+8")),
    // Help: ⌘? on a Mac (set natively), F1 elsewhere.
    ("CmdOrCtrl+?", Some("F1")),
];

/// A shortcut's modifiers and key, whatever order it was written in.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Chord {
    pub cmd: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub key: String,
}

/// Parses an accelerator (`Alt+Shift+CmdOrCtrl+S`, `Ctrl+Shift+PageUp`, `F9`).
pub fn chord(accelerator: &str) -> Chord {
    let mut c = Chord {
        cmd: false,
        ctrl: false,
        alt: false,
        shift: false,
        key: String::new(),
    };
    let parts: Vec<&str> = accelerator.split('+').collect();
    // A trailing "+" key (`CmdOrCtrl++`) splits into two empty parts.
    let (mods, key) = match parts.as_slice() {
        [mods @ .., "", ""] => (mods, "+"),
        [mods @ .., key] => (mods, *key),
        [] => (&[][..], ""),
    };
    for m in mods {
        match m.to_ascii_lowercase().as_str() {
            "cmdorctrl" | "commandorcontrol" | "cmd" | "command" | "super" => c.cmd = true,
            "ctrl" | "control" => c.ctrl = true,
            "alt" | "option" => c.alt = true,
            "shift" => c.shift = true,
            _ => {}
        }
    }
    c.key = key.to_string();
    c
}

/// The Windows/Linux accelerator for a Mac one, as [`PC`] lists it (`None` when it has
/// none there, or is not listed).
pub fn pc_accelerator(mac: &str) -> Option<&'static str> {
    let want = chord(mac);
    PC.iter()
        .find(|(m, _)| chord(m) == want)
        .and_then(|(_, pc)| *pc)
}

/// Whether [`PC`] lists `mac`.
pub fn pc_listed(mac: &str) -> bool {
    let want = chord(mac);
    PC.iter().any(|(m, _)| chord(m) == want)
}

/// The Mac accelerator a hint in symbols names (`⌥⌘1` → `Alt+CmdOrCtrl+1`).
pub fn from_symbols(symbols: &str) -> String {
    let mut mods = Vec::new();
    let mut key = String::new();
    for ch in symbols.chars() {
        match ch {
            '⌃' => mods.push("Ctrl"),
            '⌥' => mods.push("Alt"),
            '⇧' => mods.push("Shift"),
            '⌘' => mods.push("CmdOrCtrl"),
            '←' => key.push_str("Left"),
            '→' => key.push_str("Right"),
            '↑' => key.push_str("Up"),
            '↓' => key.push_str("Down"),
            '⌫' => key.push_str("Backspace"),
            '↩' => key.push_str("Enter"),
            // A zoom hint's "+" and "−" are the = and - keys.
            '+' => key.push('='),
            '−' => key.push('-'),
            other => key.push(other),
        }
    }
    mods.push(&key);
    mods.join("+")
}

/// How an accelerator reads on Windows/Linux (`Ctrl+Shift+PageUp`).
pub fn pc_words(accelerator: &str) -> String {
    let c = chord(accelerator);
    let mut parts = Vec::new();
    if c.ctrl || c.cmd {
        parts.push("Ctrl".to_string());
    }
    if c.alt {
        parts.push("Alt".to_string());
    }
    if c.shift {
        parts.push("Shift".to_string());
    }
    parts.push(match c.key.as_str() {
        "=" => "+".to_string(),
        "PageUp" => "Page Up".to_string(),
        "PageDown" => "Page Down".to_string(),
        other => other.to_string(),
    });
    parts.join("+")
}

/// A menu shortcut's hint, written with the Mac's symbols (`"⌥⌘1"`): as it is on a Mac, the
/// Windows/Linux shortcut elsewhere, or `None` when that item has none there.
pub fn hint(symbols: &str, keys: Keys) -> Option<String> {
    match keys {
        Keys::Mac => Some(symbols.to_string()),
        Keys::Pc => pc_accelerator(&from_symbols(symbols)).map(pc_words),
    }
}

/// Mac key words in running text for Windows/Linux (`⌘-click` → `Ctrl-click`,
/// `⇧-click` → `Shift-click`, `⌘Return` → `Ctrl+Enter`, `⇧↩` → `Shift+Enter`). For
/// keys that are not menu shortcuts (clicks, Return in a field).
pub fn pc_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let modifier = |c: char| matches!(c, '⌘' | '⌥' | '⇧' | '⌃');
    let mut i = 0;
    while i < chars.len() {
        if !modifier(chars[i]) {
            match chars[i] {
                '↩' => out.push_str("Enter"),
                '⌫' => out.push_str("Backspace"),
                c => out.push(c),
            }
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && modifier(chars[i]) {
            i += 1;
        }
        let group = &chars[start..i];
        let names: Vec<&str> = [('⌘', "Ctrl"), ('⌃', "Ctrl"), ('⌥', "Alt"), ('⇧', "Shift")]
            .iter()
            .filter(|(g, _)| group.contains(g))
            .map(|(_, n)| *n)
            .fold(Vec::new(), |mut acc, n| {
                if !acc.contains(&n) {
                    acc.push(n);
                }
                acc
            });
        out.push_str(&names.join("+"));
        // "⌘-click" keeps its hyphen; "⌘Return" becomes "Ctrl+Return".
        if chars
            .get(i)
            .is_some_and(|c| !c.is_whitespace() && *c != '-')
        {
            out.push('+');
        }
    }
    out.replace("+Return", "+Enter")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_pc_shortcut_is_altgr() {
        let mut seen = std::collections::BTreeSet::new();
        for (mac, pc) in PC {
            if let Some(pc) = pc {
                let c = chord(pc);
                assert!(!(c.ctrl && c.alt), "{pc} (for {mac}) would be AltGr");
                assert!(!c.cmd, "{pc}: write Ctrl on Windows/Linux");
                assert!(seen.insert(c), "{pc} used twice");
            }
        }
    }

    #[test]
    fn orders_do_not_matter() {
        assert_eq!(chord("CmdOrCtrl+Shift+S"), chord("Shift+CmdOrCtrl+S"));
        assert_eq!(pc_accelerator("Shift+CmdOrCtrl+S"), Some("Ctrl+Shift+S"));
        assert_eq!(pc_accelerator("Alt+Shift+CmdOrCtrl+S"), None);
        assert!(pc_listed("Shift+Alt+CmdOrCtrl+S"));
        assert!(!pc_listed("CmdOrCtrl+Q"));
    }

    #[test]
    fn hints() {
        assert_eq!(hint("⌥⌘1", Keys::Mac).as_deref(), Some("⌥⌘1"));
        assert_eq!(hint("⌥⌘1", Keys::Pc).as_deref(), Some("F9"));
        assert_eq!(hint("⌘O", Keys::Pc).as_deref(), Some("Ctrl+O"));
        assert_eq!(hint("⇧⌘Z", Keys::Pc).as_deref(), Some("Ctrl+Shift+Z"));
        assert_eq!(hint("⌃⌘3", Keys::Pc).as_deref(), Some("Ctrl+Shift+3"));
        assert_eq!(hint("⌘+", Keys::Pc).as_deref(), Some("Ctrl++"));
        assert_eq!(hint("⌘−", Keys::Pc).as_deref(), Some("Ctrl+-"));
        assert_eq!(hint("⌥⌘↑", Keys::Pc).as_deref(), Some("Ctrl+Shift+Page Up"));
        assert_eq!(hint("⌥⌘←", Keys::Pc).as_deref(), Some("Ctrl+Page Up"));
        assert_eq!(hint("⌥⌘N", Keys::Pc), None);
        assert_eq!(hint("⌘?", Keys::Pc).as_deref(), Some("F1"));
    }

    #[test]
    fn running_text() {
        assert_eq!(pc_text("⌘-click adds a page"), "Ctrl-click adds a page");
        assert_eq!(
            pc_text("⇧-click selects a range"),
            "Shift-click selects a range"
        );
        assert_eq!(
            pc_text("Esc or ⌘Return to finish"),
            "Esc or Ctrl+Enter to finish"
        );
        assert_eq!(
            pc_text("Previous match (⇧↩)"),
            "Previous match (Shift+Enter)"
        );
        assert_eq!(pc_text("(⇧ for bigger steps)"), "(Shift for bigger steps)");
    }

    #[test]
    fn os_from_the_build() {
        let os = Os::native();
        #[cfg(target_os = "macos")]
        assert_eq!(os, Os::Mac);
        #[cfg(windows)]
        assert_eq!(os, Os::Windows);
        #[cfg(target_os = "linux")]
        assert_eq!(os, Os::Linux);
        assert_eq!(Os::Mac.keys(), Keys::Mac);
        assert_eq!(Os::Windows.keys(), Keys::Pc);
        assert_eq!(Os::Linux.keys(), Keys::Pc);
        // The wire words, and a missing field reads as a Mac.
        assert_eq!(serde_json::to_string(&Os::Windows).unwrap(), "\"windows\"");
        assert_eq!(serde_json::from_str::<Os>("\"linux\"").unwrap(), Os::Linux);
        assert_eq!(Os::default(), Os::Mac);
    }

    #[test]
    fn file_manager_names() {
        assert_eq!(Os::Mac.show_in_folder(), "Show in Finder");
        assert_eq!(Os::Windows.show_in_folder(), "Show in Explorer");
        assert_eq!(Os::Linux.show_in_folder(), "Show in Folder");
    }

    #[test]
    fn words_per_os() {
        let mac = "Read, fill, sign, edit and redact — everything stays on this Mac.";
        assert_eq!(Os::Mac.text(mac), mac);
        assert_eq!(
            Os::Windows.text(mac),
            "Read, fill, sign, edit and redact — everything stays on this computer."
        );
        let pages = "Drag pages to reorder. ⇧-click selects a range, ⌘-click adds a page.";
        assert_eq!(Os::Mac.text(pages), pages);
        assert_eq!(
            Os::Linux.text(pages),
            "Drag pages to reorder. Shift-click selects a range, Ctrl-click adds a page."
        );
        assert_eq!(
            Os::Windows.text("Type your note… (Esc or ⌘Return to finish)"),
            "Type your note… (Esc or Ctrl+Enter to finish)"
        );
        assert_eq!(
            Os::Windows.text("Previous match (⇧↩)"),
            "Previous match (Shift+Enter)"
        );
        assert_eq!(Os::Windows.text("Next match (↩)"), "Next match (Enter)");
    }

    #[test]
    fn tooltips() {
        assert_eq!(Os::Mac.titled("Sidebar", "⌥⌘1"), "Sidebar (⌥⌘1)");
        assert_eq!(Os::Windows.titled("Sidebar", "⌥⌘1"), "Sidebar (F9)");
        assert_eq!(Os::Linux.titled("Panel", "⌥⌘2"), "Panel (Shift+F9)");
        assert_eq!(Os::Windows.titled("Open…", "⌘O"), "Open… (Ctrl+O)");
        assert_eq!(Os::Windows.titled("Redo", "⇧⌘Z"), "Redo (Ctrl+Shift+Z)");
        assert_eq!(Os::Windows.titled("Zoom out", "⌘−"), "Zoom out (Ctrl+-)");
        assert_eq!(Os::Windows.titled("Zoom in", "⌘+"), "Zoom in (Ctrl++)");
        assert_eq!(Os::Windows.titled("Select", "⌃⌘1"), "Select (Ctrl+Shift+1)");
        assert_eq!(
            Os::Windows.titled("Move the selected pages one place earlier", "⌥⌘↑"),
            "Move the selected pages one place earlier (Ctrl+Shift+Page Up)"
        );
        // No shortcut there: the label alone.
        assert_eq!(
            Os::Windows.titled("New from images", "⌥⌘N"),
            "New from images"
        );
        assert_eq!(Os::Windows.hint("⌥⌘N"), None);
        assert_eq!(Os::Mac.hint("⌥⌘N").as_deref(), Some("⌥⌘N"));
    }

    #[test]
    fn help_rows() {
        let rows = |os: Os| -> Vec<(String, String)> {
            HELP_KEYS
                .iter()
                .filter_map(|(keys, sep, what)| Some((os.help_keys(keys, sep)?, os.text(what))))
                .collect()
        };
        let mac: Vec<(String, String)> = rows(Os::Mac);
        let mac_keys: Vec<&str> = mac.iter().map(|(k, _)| k.as_str()).collect();
        // Exactly as the help showed them before (the Mac app suite reads them).
        assert_eq!(
            mac_keys,
            [
                "⌘O, ⌘N, ⌘S, ⇧⌘S",
                "⌘Z, ⇧⌘Z",
                "⌘F, ⌘G, ⇧⌘G",
                "⌘+, ⌘−, ⌘0, ⌘9, ⌘8",
                "⌃⌘1 … ⌃⌘8",
                "Esc",
                "⌥⌘1, ⌥⌘2",
                "⌥⌘← ⌥⌘→",
                "Delete, arrows",
                "Tab",
                "⌘?",
            ]
        );
        assert_eq!(
            mac[8].1,
            "Delete or nudge what is selected on the page (⇧ for bigger steps)"
        );
        let pc = rows(Os::Windows);
        let pc_keys: Vec<&str> = pc.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            pc_keys,
            [
                "Ctrl+O, Ctrl+N, Ctrl+S, Ctrl+Shift+S",
                "Ctrl+Z, Ctrl+Shift+Z",
                "Ctrl+F, Ctrl+G, Ctrl+Shift+G",
                "Ctrl++, Ctrl+-, Ctrl+0, Ctrl+9, Ctrl+8",
                "Ctrl+Shift+1 … Ctrl+Shift+8",
                "Esc",
                "F9, Shift+F9",
                "Ctrl+Page Up, Ctrl+Page Down",
                "Delete, arrows",
                "Tab",
                "F1",
            ]
        );
        assert_eq!(
            pc[8].1,
            "Delete or nudge what is selected on the page (Shift for bigger steps)"
        );
        assert_eq!(rows(Os::Linux), pc);
        // A row of shortcuts that have none off the Mac goes away.
        assert_eq!(Os::Windows.help_keys(&["⌥⌘N", "⌥⌘K"], ", "), None);
        assert_eq!(
            Os::Windows.help_keys(&["⌘O", "⌥⌘N"], ", ").as_deref(),
            Some("Ctrl+O")
        );
    }

    #[test]
    fn command_modifier() {
        // ⌘ on a Mac (Control-click stays the Mac's context click)...
        assert!(Os::Mac.primary(true, false));
        assert!(!Os::Mac.primary(false, true));
        // ...Ctrl on Windows and Linux (the Windows/Super key is the OS's).
        assert!(Os::Windows.primary(false, true));
        assert!(!Os::Windows.primary(true, false));
        assert!(Os::Linux.primary(false, true));
        assert_eq!(Os::Mac.aria_primary(), "Meta");
        assert_eq!(Os::Windows.aria_primary(), "Control");
    }

    #[test]
    fn menu_chords_stay_with_the_menu() {
        let w = Os::Windows;
        // Delete Page (Ctrl+Shift+Backspace) and Move Page Up (Ctrl+Shift+Page Up) are the
        // menu's; plain Delete, Backspace and arrows are the page's.
        assert!(w.menu_chord(true, false, true, "Backspace"));
        assert!(w.menu_chord(true, false, true, "PageUp"));
        assert!(w.menu_chord(true, false, false, "PageDown"));
        assert!(w.menu_chord(true, false, false, "="));
        assert!(w.menu_chord(true, false, false, "o"));
        assert!(w.menu_chord(false, false, false, "F9"));
        assert!(w.menu_chord(false, false, true, "F9"));
        assert!(w.menu_chord(false, false, false, "F1"));
        assert!(!w.menu_chord(false, false, false, "Backspace"));
        assert!(!w.menu_chord(false, false, false, "Delete"));
        assert!(!w.menu_chord(false, false, true, "ArrowUp"));
        assert!(!w.menu_chord(true, false, false, "Enter"));
        // The Edit menu's Cut/Copy/Paste/Select All are not in the table: the page acts on them.
        assert!(!w.menu_chord(true, false, false, "c"));
        assert!(!w.menu_chord(true, false, false, "a"));
        assert!(Os::Linux.menu_chord(true, false, true, "Backspace"));
        // The Mac's pages keep every key they had.
        assert!(!Os::Mac.menu_chord(true, false, true, "Backspace"));
        assert!(!Os::Mac.menu_chord(false, false, false, "F9"));
    }
}
