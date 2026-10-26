//! The macOS App Sandbox (the App Store build). There the app may read and write only
//! the files a person chose (open and save panels, Finder, drag and drop), and only until
//! it quits. Two things follow:
//! - [`lent_folder`]: a save's temp cannot go beside the file, so it goes in a folder the
//!   system lends on the same volume (`NSItemReplacementDirectory`), then is renamed over;
//! - [`Bookmarks`]: Open Recent and the start screen open files by path, which the
//!   sandbox refuses after a relaunch, so each recent file keeps a security-scoped
//!   bookmark that wins the access back.
//!
//! Outside the sandbox (the .dmg build, development) none of this is used.
#![allow(unsafe_code)]

use objc2::rc::Retained;
use objc2::runtime::Bool;
use objc2_foundation::{
    NSData, NSFileManager, NSSearchPathDirectory, NSSearchPathDomainMask, NSString, NSURL,
    NSURLBookmarkCreationOptions, NSURLBookmarkResolutionOptions,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Bookmarks kept: more than Open Recent shows, so a file that drops off and comes back
/// still opens.
const KEPT: usize = 32;

/// True when this process runs in the App Sandbox.
pub fn active() -> bool {
    std::env::var_os("APP_SANDBOX_CONTAINER_ID").is_some()
}

/// A new empty folder on `dest`'s volume that this app may write (`safe_write`'s temp
/// folder); `None` leaves the temp beside `dest`.
pub fn lent_folder(dest: &Path) -> Option<PathBuf> {
    let url = file_url(dest);
    NSFileManager::defaultManager()
        .URLForDirectory_inDomain_appropriateForURL_create_error(
            NSSearchPathDirectory::ItemReplacementDirectory,
            NSSearchPathDomainMask::UserDomainMask,
            Some(&url),
            true,
        )
        .ok()
        .and_then(|folder| path_of(&folder))
}

fn file_url(path: &Path) -> Retained<NSURL> {
    NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()))
}

fn path_of(url: &NSURL) -> Option<PathBuf> {
    url.path().map(|p| PathBuf::from(p.to_string()))
}

/// The recent files' bookmarks, newest first, in `<user data>/bookmarks.json`.
pub struct Bookmarks {
    file: PathBuf,
    entries: Vec<(PathBuf, Vec<u8>)>,
    /// Files whose access was won back this run (and is held until the app quits).
    started: HashSet<PathBuf>,
}

impl Bookmarks {
    pub fn load(user_data: &Path) -> Bookmarks {
        let file = user_data.join("bookmarks.json");
        let entries = std::fs::read(&file)
            .ok()
            .and_then(|b| serde_json::from_slice::<Vec<(String, String)>>(&b).ok())
            .unwrap_or_default()
            .into_iter()
            .map(|(path, data)| {
                (
                    PathBuf::from(path),
                    tandem_app::host::util::base64_decode_lenient(&data),
                )
            })
            .filter(|(_, data)| !data.is_empty())
            .collect();
        Bookmarks {
            file,
            entries,
            started: HashSet::new(),
        }
    }

    /// Keeps a bookmark for `path`, which this run may reach now (it was just opened or saved).
    pub fn remember(&mut self, path: &Path) {
        let Ok(data) = file_url(path)
            .bookmarkDataWithOptions_includingResourceValuesForKeys_relativeToURL_error(
                NSURLBookmarkCreationOptions::WithSecurityScope,
                None,
                None,
            )
        else {
            return;
        };
        self.entries.retain(|(p, _)| p != path);
        self.entries.insert(0, (path.to_path_buf(), data.to_vec()));
        self.entries.truncate(KEPT);
        self.save();
    }

    /// Wins back access to `path` from its bookmark, if it has one and this run has not
    /// already. (A file that moved since is not followed; opening `path` says it is gone.)
    pub fn regain(&mut self, path: &Path) {
        if self.started.contains(path) {
            return;
        }
        let Some((_, data)) = self.entries.iter().find(|(p, _)| p == path) else {
            return;
        };
        let data = NSData::with_bytes(data);
        let mut stale = Bool::NO;
        // SAFETY: `stale` outlives the call, which writes it once.
        let url = unsafe {
            NSURL::URLByResolvingBookmarkData_options_relativeToURL_bookmarkDataIsStale_error(
                &data,
                NSURLBookmarkResolutionOptions::WithSecurityScope,
                None,
                &mut stale,
            )
        };
        let Ok(url) = url else {
            return;
        };
        // SAFETY: a plain Foundation call; the access is held for the rest of the run.
        if unsafe { url.startAccessingSecurityScopedResource() } {
            self.started.insert(path.to_path_buf());
            if stale.as_bool() {
                self.remember(path);
            }
        }
    }

    /// Clear Menu: forgets every bookmark.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.save();
    }

    fn save(&self) {
        let list: Vec<(String, String)> = self
            .entries
            .iter()
            .map(|(p, d)| {
                (
                    p.to_string_lossy().into_owned(),
                    tandem_app::host::util::base64_encode(d),
                )
            })
            .collect();
        let bytes = serde_json::to_vec(&list).unwrap_or_default();
        let _ = tandem_app::host::util::write_store(&self.file, &bytes, true, None);
    }
}
