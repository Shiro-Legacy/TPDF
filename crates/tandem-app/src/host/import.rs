//! Bringing over the Electron app's stores, once.
//!
//! The Electron app kept its recent files, trust list and drawn signature in its user-data
//! folder. A packaged Electron app used the product name (`Tandem PDF`), which is the Rust
//! app's own folder, so those carry over as they are (the formats are the same). Development
//! and early builds used the package name, `pdf4all-p0`, beside it:
//! macOS `~/Library/Application Support/pdf4all-p0`, Windows `%APPDATA%\pdf4all-p0`,
//! Linux `~/.config/pdf4all-p0`.
//!
//! On the first start with the default folder the app copies each store its own folder lacks.
//! It never overwrites, never changes or removes Electron's files, and runs only once (a
//! marker file), so a trust list the person later empties is not brought back.

use super::util;
use std::io;
use std::path::{Path, PathBuf};

/// The stores and how they are named to people.
pub const STORES: [(&str, &str); 3] = [
    ("recent-files.json", "recent files"),
    ("trusted-certs.pem", "trusted certificates"),
    ("drawn-signature.json", "saved signature"),
];
/// Written once the import has run (whatever it found).
pub const MARKER: &str = "electron-import.json";
/// The Electron development builds' folder name.
pub const ELECTRON_FOLDER: &str = "pdf4all-p0";

/// The Electron folder beside the default user-data folder.
pub fn electron_folder() -> PathBuf {
    super::app_data_root().join(ELECTRON_FOLDER)
}

/// Copies the stores `to` lacks from `from`, once per `to`: the names of the stores copied.
/// Nothing in `from` is changed. A store that cannot be read is skipped.
pub fn import_once(from: &Path, to: &Path) -> io::Result<Vec<&'static str>> {
    let marker = to.join(MARKER);
    if marker.exists() {
        return Ok(Vec::new());
    }
    std::fs::create_dir_all(to)?;
    let mut copied = Vec::new();
    for (name, _) in STORES {
        let target = to.join(name);
        if std::fs::symlink_metadata(&target).is_ok() {
            continue;
        }
        let source = from.join(name);
        // Only a plain file (never through a link), and only a store-sized one.
        let Ok(meta) = std::fs::symlink_metadata(&source) else {
            continue;
        };
        if !meta.is_file() || meta.len() > 16 * 1024 * 1024 {
            continue;
        }
        let Ok(bytes) = std::fs::read(&source) else {
            continue;
        };
        if util::write_store(&target, &bytes, true, None).is_ok() {
            copied.push(name);
        }
    }
    let record = serde_json::json!({"from": from.to_string_lossy(), "copied": copied});
    util::write_store(&marker, format!("{record}\n").as_bytes(), true, None)
        .map_err(|e| io::Error::other(format!("{e:?}")))?;
    Ok(copied)
}

/// What to tell the person, if anything was copied.
pub fn note(copied: &[&str]) -> Option<String> {
    let words: Vec<&str> = STORES
        .iter()
        .filter(|(name, _)| copied.contains(name))
        .map(|(_, words)| *words)
        .collect();
    let list = match words.as_slice() {
        [] => return None,
        [one] => (*one).to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    };
    Some(format!(
        "Brought over your {list} from the earlier version of Tandem PDF."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folders(tag: &str) -> (PathBuf, PathBuf) {
        let base = util::session_scratch(tag);
        let (from, to) = (base.join("pdf4all-p0"), base.join("Tandem PDF"));
        std::fs::create_dir_all(&from).unwrap();
        (from, to)
    }

    #[test]
    fn copies_what_is_missing_once_and_never_overwrites() {
        let (from, to) = folders("import");
        std::fs::write(from.join("recent-files.json"), b"[\"/a.pdf\"]").unwrap();
        std::fs::write(from.join("trusted-certs.pem"), b"electron pem").unwrap();
        std::fs::write(from.join("drawn-signature.json"), b"{\"electron\":1}").unwrap();
        std::fs::create_dir_all(&to).unwrap();
        std::fs::write(to.join("trusted-certs.pem"), b"mine").unwrap();
        let copied = import_once(&from, &to).unwrap();
        assert_eq!(copied, ["recent-files.json", "drawn-signature.json"]);
        assert_eq!(
            std::fs::read(to.join("trusted-certs.pem")).unwrap(),
            b"mine"
        );
        assert_eq!(
            std::fs::read(to.join("recent-files.json")).unwrap(),
            b"[\"/a.pdf\"]"
        );
        // Electron's files are left as they were.
        assert_eq!(
            std::fs::read(from.join("trusted-certs.pem")).unwrap(),
            b"electron pem"
        );
        assert_eq!(std::fs::read_dir(&from).unwrap().count(), 3);
        // Once: a store removed later is not brought back.
        std::fs::remove_file(to.join("trusted-certs.pem")).unwrap();
        assert!(import_once(&from, &to).unwrap().is_empty());
        assert!(!to.join("trusted-certs.pem").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(to.join("drawn-signature.json"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0, "copied stores are private");
        }
        let _ = std::fs::remove_dir_all(from.parent().unwrap());
    }

    #[test]
    fn nothing_to_bring_over_says_nothing() {
        let (from, to) = folders("import-none");
        assert!(import_once(&from, &to).unwrap().is_empty());
        assert!(to.join(MARKER).exists());
        let missing = from.parent().unwrap().join("no-such-folder");
        let (_, to2) = folders("import-missing");
        assert!(import_once(&missing, &to2).unwrap().is_empty());
        assert_eq!(note(&[]), None);
        let _ = std::fs::remove_dir_all(from.parent().unwrap());
        let _ = std::fs::remove_dir_all(to2.parent().unwrap());
    }

    #[test]
    fn the_note_names_what_came_over() {
        assert_eq!(
            note(&["recent-files.json"]).unwrap(),
            "Brought over your recent files from the earlier version of Tandem PDF."
        );
        assert_eq!(
            note(&[
                "drawn-signature.json",
                "recent-files.json",
                "trusted-certs.pem"
            ])
            .unwrap(),
            "Brought over your recent files, trusted certificates and saved signature from the earlier version of Tandem PDF."
        );
    }

    #[cfg(unix)]
    #[test]
    fn links_are_not_followed() {
        let (from, to) = folders("import-link");
        let outside = from.parent().unwrap().join("outside.json");
        std::fs::write(&outside, b"secret").unwrap();
        std::os::unix::fs::symlink(&outside, from.join("recent-files.json")).unwrap();
        assert!(import_once(&from, &to).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(from.parent().unwrap());
    }
}
