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

/// Which keys a person has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keys {
    Mac,
    /// Windows and Linux.
    Pc,
}

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
}
