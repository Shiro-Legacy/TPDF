//! Journaled safe writes (port of `src/safe-write.ts`).
//!
//! Every save goes to an exclusive 0600 temp file beside the target, is
//! fsynced, read back and compared by SHA-256, then renamed over the target.
//! The temp's name is written to a journal first, so a crash between create
//! and rename leaves a temp the next launch removes (`cleanup_orphans`).
//!
//! The macOS App Sandbox (the App Store build) lets the app write only the file a person
//! chose, never its folder, so there the temp is made in a folder the system lends on the
//! same volume ([`set_temp_folder`]) and renamed over the target from there.

use crate::fsx::{self, OpenMode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// The save and validate ceiling (engine `MAX_SAVE_BYTES`, 50 MiB).
pub const MAX_SAVE_BYTES: usize = 50 * 1024 * 1024;
const MIN_PDF_BYTES: usize = 5;

/// What identifies an opened file, to notice it changed underneath us.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileId {
    pub path: PathBuf,
    pub size: u64,
    pub mtime_ns: i128,
    pub dev: u64,
    pub ino: u64,
}

impl FileId {
    /// The identity of a regular file, or `None` (missing, a folder, unreadable).
    pub fn of(path: &Path) -> Option<FileId> {
        let meta = fs::metadata(path).ok()?;
        if !meta.is_file() {
            return None;
        }
        let key = fsx::file_key(path, &meta);
        Some(FileId {
            path: path.to_path_buf(),
            size: meta.len(),
            mtime_ns: fsx::mtime_ns(&meta),
            dev: key.0,
            ino: key.1,
        })
    }

    fn same_file(&self, other: &FileId) -> bool {
        self.size == other.size
            && self.mtime_ns == other.mtime_ns
            && self.dev == other.dev
            && self.ino == other.ino
    }
}

/// Why a save did not happen; the strings are the Electron app's reasons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveFail {
    Cancelled,
    TooLarge,
    DiskFull,
    Permission,
    Unreadable,
    SameFile,
    DocumentChanged,
    Busy,
    WriteFailed,
    SourceChanged,
    BrokenLink,
}

impl SaveFail {
    pub fn as_str(self) -> &'static str {
        match self {
            SaveFail::Cancelled => "cancelled",
            SaveFail::TooLarge => "too-large",
            SaveFail::DiskFull => "disk-full",
            SaveFail::Permission => "permission",
            SaveFail::Unreadable => "unreadable",
            SaveFail::SameFile => "same-file",
            SaveFail::DocumentChanged => "document-changed",
            SaveFail::Busy => "busy",
            SaveFail::WriteFailed => "write-failed",
            SaveFail::SourceChanged => "source-changed",
            SaveFail::BrokenLink => "broken-link",
        }
    }
}

/// A save that landed, possibly with a warning.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Saved {
    pub name: String,
    /// `source-changed` or `durability-uncertain`.
    pub warning: Option<&'static str>,
    /// The written file's identity, under the path the caller gave.
    pub file: Option<FileId>,
}

pub type SaveResult = Result<Saved, SaveFail>;

/// A replacement for the temp-file write.
pub type WriteHook = Box<dyn Fn(&mut File, &[u8]) -> io::Result<()>>;
/// A replacement for the temp file's read-back.
pub type ReopenHook = Box<dyn Fn(&Path) -> io::Result<Vec<u8>>>;
/// Picks the temp folder for a save to the given path (see [`set_temp_folder`]).
pub type TempFolder = fn(&Path) -> Option<PathBuf>;

static TEMP_FOLDER: OnceLock<TempFolder> = OnceLock::new();

/// Makes every later save put its temp in the folder `choose` gives for the target: a new
/// empty folder on the target's volume, which the save removes again. `None` from `choose`
/// means beside the target, as without it. Set once, at startup (the sandboxed Mac app).
pub fn set_temp_folder(choose: TempFolder) {
    let _ = TEMP_FOLDER.set(choose);
}

/// Test seams (`TANDEM_TEST_*` environment hooks map onto these).
#[derive(Default)]
pub struct Inject {
    /// Replaces the temp-file write (e.g. to fail with ENOSPC).
    pub write_all: Option<WriteHook>,
    /// Replaces the read-back of the temp file.
    pub reopen: Option<ReopenHook>,
    /// Fails the folder fsync.
    pub dir_sync_fails: bool,
    /// Replaces the [`set_temp_folder`] choice.
    pub temp_folder: Option<TempFolder>,
}

/// `name.pdf` → `name-copy.pdf`.
pub fn copy_basename(name: &str) -> String {
    let base = Path::new(name)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    if base.to_lowercase().ends_with(".pdf") {
        format!("{}-copy.pdf", &base[..base.len() - 4])
    } else {
        format!("{base}-copy.pdf")
    }
}

/// The OS error numbers [`classify`] reads: (disk full, permission, in use by another app).
#[cfg(target_os = "macos")]
const ERRNO: (&[i32], &[i32], &[i32]) = (&[28, 69], &[13, 1, 30], &[]); // ENOSPC EDQUOT; EACCES EPERM EROFS
#[cfg(all(unix, not(target_os = "macos")))]
const ERRNO: (&[i32], &[i32], &[i32]) = (&[28, 122], &[13, 1, 30], &[]); // ENOSPC EDQUOT; EACCES EPERM EROFS
/// ERROR_HANDLE_DISK_FULL, ERROR_DISK_FULL; ERROR_ACCESS_DENIED, ERROR_WRITE_PROTECT;
/// ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION (the file is open in another app).
#[cfg(windows)]
const ERRNO: (&[i32], &[i32], &[i32]) = (&[39, 112], &[5, 19], &[32, 33]);
#[cfg(not(any(unix, windows)))]
const ERRNO: (&[i32], &[i32], &[i32]) = (&[], &[], &[]);

/// Maps an I/O error onto the reasons a person can act on.
pub fn classify(error: &io::Error) -> SaveFail {
    let (full, denied, busy) = ERRNO;
    match error.raw_os_error() {
        Some(n) if full.contains(&n) => SaveFail::DiskFull,
        Some(n) if denied.contains(&n) => SaveFail::Permission,
        Some(n) if busy.contains(&n) => SaveFail::Busy,
        _ => match error.kind() {
            io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem => {
                SaveFail::Permission
            }
            io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded => SaveFail::DiskFull,
            _ => SaveFail::WriteFailed,
        },
    }
}

fn canonical_dest(dest: &Path) -> io::Result<PathBuf> {
    let parent = dest
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = dest
        .file_name()
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    Ok(fs::canonicalize(parent)?.join(name))
}

/// True when writing `dest` would write the source itself: the same real
/// path, a symbolic link, or (unless `other_names`) a hard link of it.
pub fn dest_conflicts_source(dest: &Path, source: &FileId, other_names: bool) -> bool {
    let Ok(dest_path) = canonical_dest(dest) else {
        return false;
    };
    let src_path = fs::canonicalize(&source.path).unwrap_or_else(|_| source.path.clone());
    if dest_path == src_path {
        return true;
    }
    if let Ok(meta) = fs::symlink_metadata(&dest_path) {
        if meta.file_type().is_symlink() {
            return true;
        }
        if !other_names && fsx::file_key(&dest_path, &meta) == (source.dev, source.ino) {
            return true;
        }
    }
    false
}

/// Save a copy to a new path (the open document keeps its own path).
pub fn save_copy(
    dest: Option<&Path>,
    bytes: &[u8],
    source: &FileId,
    journal: Option<&Path>,
    linked_name_is_other: bool,
    inject: &Inject,
    is_current: &dyn Fn() -> bool,
) -> SaveResult {
    let dest = dest.ok_or(SaveFail::Cancelled)?;
    if bytes.len() > MAX_SAVE_BYTES {
        return Err(SaveFail::TooLarge);
    }
    if bytes.len() < MIN_PDF_BYTES || !dest.is_absolute() {
        return Err(SaveFail::Unreadable);
    }
    write_copy(
        dest,
        bytes,
        Some(source),
        inject,
        is_current,
        journal,
        Mode::Copy,
        linked_name_is_other,
    )
}

/// Write a brand-new file (exports, extracted pages, redacted copies).
pub fn write_new(
    dest: Option<&Path>,
    bytes: &[u8],
    journal: Option<&Path>,
    inject: &Inject,
) -> SaveResult {
    write_fresh(dest, bytes, journal, inject, Mode::New)
}

/// [`write_new`] for a private file (a store of the person's own: drawn signature, recent
/// files): the new file is 0600 on Unix whatever mode the file it replaces had.
pub fn write_private(
    dest: Option<&Path>,
    bytes: &[u8],
    journal: Option<&Path>,
    inject: &Inject,
) -> SaveResult {
    write_fresh(dest, bytes, journal, inject, Mode::Private)
}

fn write_fresh(
    dest: Option<&Path>,
    bytes: &[u8],
    journal: Option<&Path>,
    inject: &Inject,
    mode: Mode,
) -> SaveResult {
    let dest = dest.ok_or(SaveFail::Cancelled)?;
    if bytes.len() > MAX_SAVE_BYTES {
        return Err(SaveFail::TooLarge);
    }
    if bytes.is_empty() || !dest.is_absolute() {
        return Err(SaveFail::Unreadable);
    }
    write_copy(dest, bytes, None, inject, &|| true, journal, mode, false)
}

/// Save over the opened file, refusing if it changed since it was opened.
pub fn save_in_place(
    bytes: &[u8],
    source: &FileId,
    journal: Option<&Path>,
    inject: &Inject,
    is_current: &dyn Fn() -> bool,
) -> SaveResult {
    if bytes.len() > MAX_SAVE_BYTES {
        return Err(SaveFail::TooLarge);
    }
    if bytes.len() < MIN_PDF_BYTES || !source.path.is_absolute() {
        return Err(SaveFail::Unreadable);
    }
    let dest = source.path.clone();
    write_copy(
        &dest,
        bytes,
        Some(source),
        inject,
        is_current,
        journal,
        Mode::InPlace,
        false,
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Copy,
    InPlace,
    New,
    /// A new file that is always 0600 (Unix), never the replaced file's mode.
    Private,
}

fn resolve_target(dest: &Path, mode: Mode) -> Result<PathBuf, SaveFail> {
    if mode == Mode::InPlace {
        return fs::canonicalize(dest).map_err(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                SaveFail::SourceChanged
            } else {
                classify(&e)
            }
        });
    }
    let link = match fs::symlink_metadata(dest) {
        Ok(meta) => meta.file_type().is_symlink(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => false,
        Err(e) => return Err(classify(&e)),
    };
    if !link {
        return canonical_dest(dest).map_err(|e| classify(&e));
    }
    fs::canonicalize(dest).map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound || e.raw_os_error() == Some(62) {
            SaveFail::BrokenLink
        } else {
            classify(&e)
        }
    })
}

fn source_changed(source: &FileId) -> Option<&'static str> {
    match FileId::of(&source.path) {
        Some(now) if now.same_file(source) => None,
        _ => Some("source-changed"),
    }
}

fn unique() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut h = Sha256::new();
    h.update(now.to_le_bytes());
    h.update(n.to_le_bytes());
    h.update(std::process::id().to_le_bytes());
    let d = h.finalize();
    u64::from_le_bytes(d[..8].try_into().unwrap_or([0; 8]))
}

fn random_hex(len: usize) -> String {
    let mut out = String::new();
    while out.len() < len {
        out.push_str(&format!("{:016x}", unique()));
    }
    out.truncate(len);
    out
}

#[allow(clippy::too_many_arguments)]
fn write_copy(
    dest: &Path,
    bytes: &[u8],
    source: Option<&FileId>,
    inject: &Inject,
    is_current: &dyn Fn() -> bool,
    journal: Option<&Path>,
    mode: Mode,
    other_names: bool,
) -> SaveResult {
    let dest_path = resolve_target(dest, mode)?;
    if mode == Mode::Copy
        && let Some(source) = source
        && dest_conflicts_source(&dest_path, source, other_names)
    {
        return Err(SaveFail::SameFile);
    }
    let mut warning = if mode == Mode::Copy {
        source.and_then(source_changed)
    } else {
        None
    };
    let expected = Sha256::digest(bytes);
    let dir = dest_path.parent().unwrap_or(Path::new("/")).to_path_buf();
    let base = dest_path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    // A lent folder (the App Sandbox) instead of the target's own; removed when done.
    let lent = inject
        .temp_folder
        .or(TEMP_FOLDER.get().copied())
        .and_then(|choose| choose(&dest_path))
        .filter(|folder| *folder != dir);
    let temp_dir = lent.clone().unwrap_or_else(|| dir.clone());

    let mut journaled: Vec<PathBuf> = Vec::new();
    let mut tmp: Option<PathBuf> = None;
    let mut committed = false;
    let result = (|| -> SaveResult {
        let (path, mut file) = exclusive_temp(&temp_dir, &base, |path| {
            if let Some(journal) = journal {
                journal_add(journal, path, &dest_path).map_err(|e| classify(&e))?;
                journaled.push(path.to_path_buf());
            }
            Ok(())
        })?;
        tmp = Some(path.clone());
        match &inject.write_all {
            Some(write) => write(&mut file, bytes),
            None => file.write_all(bytes),
        }
        .map_err(|e| classify(&e))?;
        file.sync_all().map_err(|e| classify(&e))?;
        drop(file);

        let disk = match &inject.reopen {
            Some(reopen) => reopen(&path),
            None => fs::read(&path),
        }
        .map_err(|e| classify(&e))?;
        if disk.len() != bytes.len() || Sha256::digest(&disk) != expected {
            return Err(SaveFail::WriteFailed);
        }
        if mode == Mode::Copy
            && let Some(source) = source
            && dest_conflicts_source(&dest_path, source, other_names)
        {
            return Err(SaveFail::SameFile);
        }
        if mode == Mode::InPlace {
            let source = source.ok_or(SaveFail::Unreadable)?;
            let now = fs::metadata(&dest_path).map_err(|_| SaveFail::SourceChanged)?;
            let id = FileId::of(&dest_path).ok_or(SaveFail::SourceChanged)?;
            if !now.is_file() || !id.same_file(source) {
                return Err(SaveFail::SourceChanged);
            }
            fsx::set_mode(&path, fsx::mode_of(&now) & 0o777).map_err(|e| classify(&e))?;
        } else {
            let existing = fs::metadata(&dest_path)
                .ok()
                .filter(|m| m.is_file())
                .map(|m| fsx::mode_of(&m) & 0o777);
            let bits = if mode == Mode::Private {
                0o600
            } else {
                existing.unwrap_or(0o666 & !fsx::umask())
            };
            fsx::set_mode(&path, bits).map_err(|e| classify(&e))?;
            if mode == Mode::Copy
                && let Some(source) = source
            {
                warning = source_changed(source).or(warning);
            }
        }
        if !is_current() {
            return Err(SaveFail::DocumentChanged);
        }
        if read_only_target(&dest_path) {
            return Err(SaveFail::Permission);
        }
        fs::rename(&path, &dest_path).map_err(|e| classify(&e))?;
        committed = true;
        let written = FileId::of(&dest_path);
        let synced = if inject.dir_sync_fails {
            Err(io::Error::other("injected"))
        } else if lent.is_some() {
            // The sandbox does not let the folder be opened; flushing the renamed file
            // (F_FULLFSYNC on macOS) is the most it allows.
            sync_dir(&dir).or_else(|_| File::open(&dest_path).and_then(|f| f.sync_all()))
        } else {
            sync_dir(&dir)
        };
        if synced.is_err() {
            warning = Some("durability-uncertain");
        }
        Ok(Saved {
            name: dest
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
            warning,
            file: written.map(|w| FileId {
                path: dest.to_path_buf(),
                ..w
            }),
        })
    })();
    if let Some(tmp) = &tmp
        && !committed
    {
        let _ = fs::remove_file(tmp);
    }
    if let Some(folder) = &lent {
        let _ = fs::remove_dir(folder);
    }
    if let Some(journal) = journal
        && !journaled.is_empty()
    {
        let _ = journal_drop(journal, &journaled);
    }
    result
}

/// True when `path` exists but this process may not write it.
fn read_only_target(path: &Path) -> bool {
    match OpenOptions::new().write(true).open(path) {
        Ok(_) => false,
        Err(e) => {
            matches!(classify(&e), SaveFail::Permission) && e.kind() != io::ErrorKind::NotFound
        }
    }
}

fn exclusive_temp(
    dir: &Path,
    base: &str,
    mut before_create: impl FnMut(&Path) -> Result<(), SaveFail>,
) -> Result<(PathBuf, File), SaveFail> {
    let safe: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "._-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .take(80)
        .collect();
    let safe = if safe.is_empty() {
        "pdf".to_string()
    } else {
        safe
    };
    for _ in 0..8 {
        let path = dir.join(format!(".{safe}.{}.tmp", random_hex(24)));
        before_create(&path)?;
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .with_mode(0o600)
            .open(&path)
        {
            Ok(file) => return Ok((path, file)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(classify(&e)),
        }
    }
    Err(SaveFail::WriteFailed)
}

#[derive(Serialize, Deserialize, Clone)]
struct JournalEntry {
    temp: String,
    dest: String,
    started: f64,
    /// The temp is in a lent folder ([`set_temp_folder`]), not beside `dest`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    apart: bool,
}

fn read_journal(path: &Path) -> Vec<JournalEntry> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(&text)
    else {
        return Vec::new();
    };
    items
        .into_iter()
        .filter_map(|v| serde_json::from_value(v).ok())
        .collect()
}

/// Writes `path.tmp` (0600, fsync), then renames it over `path`.
fn write_journal(path: &Path, entries: &[JournalEntry]) -> io::Result<()> {
    let tmp = PathBuf::from(format!("{}.tmp", path.display()));
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .with_mode(0o600)
        .open(&tmp)?;
    let written = fsx::set_file_mode(&file, 0o600)
        .and_then(|_| {
            file.write_all(
                serde_json::to_string(entries)
                    .unwrap_or_else(|_| "[]".into())
                    .as_bytes(),
            )
        })
        .and_then(|_| file.sync_all());
    if let Err(e) = written {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    fs::rename(&tmp, path)
}

fn journal_add(journal: &Path, temp: &Path, dest: &Path) -> io::Result<()> {
    let mut entries = read_journal(journal);
    let started = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0);
    entries.push(JournalEntry {
        temp: temp.display().to_string(),
        dest: dest.display().to_string(),
        started,
        apart: temp.parent() != dest.parent(),
    });
    write_journal(journal, &entries)
}

fn journal_drop(journal: &Path, temps: &[PathBuf]) -> io::Result<()> {
    let entries: Vec<JournalEntry> = read_journal(journal)
        .into_iter()
        .filter(|e| !temps.iter().any(|t| Path::new(&e.temp) == t))
        .collect();
    write_journal(journal, &entries)
}

fn temp_name_ok(name: &str) -> bool {
    // ^\.[^/]+\.[A-Za-z0-9_-]+\.tmp$
    let Some(rest) = name.strip_prefix('.') else {
        return false;
    };
    let Some(stem) = rest.strip_suffix(".tmp") else {
        return false;
    };
    let Some((head, tail)) = stem.rsplit_once('.') else {
        return false;
    };
    !head.is_empty()
        && !head.contains(['/', '\\'])
        && !tail.is_empty()
        && tail
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Removes temps a crashed save left behind; never fails startup.
pub fn cleanup_orphans(journal: &Path) {
    let mut remain = Vec::new();
    for entry in read_journal(journal) {
        let temp = Path::new(&entry.temp);
        // A lent folder is the system's temporary items, which it empties itself; a
        // journal entry is never reason enough to delete outside the target's folder.
        if entry.apart {
            continue;
        }
        match fs::symlink_metadata(temp) {
            Ok(meta) => {
                let name = temp
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let same_dir = temp.parent() == Path::new(&entry.dest).parent();
                // An unexpected file is kept; so is one that could not be removed.
                if meta.file_type().is_symlink()
                    || !meta.is_file()
                    || !temp_name_ok(&name)
                    || !same_dir
                    || fs::remove_file(temp).is_err()
                {
                    remain.push(entry);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(_) => remain.push(entry),
        }
    }
    if journal.exists() || !remain.is_empty() {
        let _ = write_journal(journal, &remain);
    }
}

/// Reads at most `limit` bytes of a regular file; `None` if it is not one.
pub fn read_capped(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    let mut file = File::open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    let mut out = Vec::with_capacity(meta.len().min(limit) as usize);
    Read::by_ref(&mut file).take(limit).read_to_end(&mut out)?;
    Ok(out)
}

/// Flushes a folder's entries (the rename) to disk. Windows cannot open a folder as a file
/// and flushes the rename with the file system's own journal, so there it is nothing.
fn sync_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        File::open(dir).and_then(|d| d.sync_all())
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tandem-sw-{tag}-{}", random_hex(8)));
        fs::create_dir_all(&d).unwrap();
        fs::canonicalize(d).unwrap()
    }

    #[test]
    fn copy_name() {
        assert_eq!(copy_basename("/a/b/Report.PDF"), "Report-copy.pdf");
        assert_eq!(copy_basename("x"), "x-copy.pdf");
    }

    #[test]
    fn save_in_place_keeps_mode_and_refuses_changed_source() {
        let d = tmpdir("inplace");
        let p = d.join("a.pdf");
        fs::write(&p, b"%PDF-1.7 old").unwrap();
        fsx::set_mode(&p, 0o640).unwrap();
        let id = FileId::of(&p).unwrap();
        let journal = d.join("pending-saves.json");
        let saved = save_in_place(
            b"%PDF-1.7 new!",
            &id,
            Some(&journal),
            &Inject::default(),
            &|| true,
        )
        .unwrap();
        assert_eq!(saved.name, "a.pdf");
        assert_eq!(fs::read(&p).unwrap(), b"%PDF-1.7 new!");
        // Windows keeps only the read-only flag.
        assert_eq!(
            fsx::mode_of(&fs::metadata(&p).unwrap()) & 0o777,
            if cfg!(unix) { 0o640 } else { 0o666 }
        );
        assert_eq!(read_journal(&journal).len(), 0);
        // The file changed since `id` was taken.
        assert_eq!(
            save_in_place(b"%PDF-1.7 x", &id, None, &Inject::default(), &|| true),
            Err(SaveFail::SourceChanged)
        );
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn copy_refuses_same_file_and_cleans_temp_on_failure() {
        let d = tmpdir("copy");
        let p = d.join("a.pdf");
        fs::write(&p, b"%PDF-1.7 a").unwrap();
        let id = FileId::of(&p).unwrap();
        assert_eq!(
            save_copy(
                Some(&p),
                b"%PDF-1.7 b",
                &id,
                None,
                false,
                &Inject::default(),
                &|| true
            ),
            Err(SaveFail::SameFile)
        );
        let inject = Inject {
            // This system's own disk-full code (ENOSPC, or ERROR_DISK_FULL on Windows).
            write_all: Some(Box::new(|_, _| {
                Err(io::Error::from_raw_os_error(ERRNO.0[0]))
            })),
            ..Default::default()
        };
        let q = d.join("b.pdf");
        assert_eq!(
            save_copy(Some(&q), b"%PDF-1.7 b", &id, None, false, &inject, &|| true),
            Err(SaveFail::DiskFull)
        );
        assert_eq!(fs::read_dir(&d).unwrap().count(), 1);
        let ok = save_copy(
            Some(&q),
            b"%PDF-1.7 b",
            &id,
            None,
            false,
            &Inject::default(),
            &|| true,
        )
        .unwrap();
        assert_eq!(ok.name, "b.pdf");
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn a_lent_temp_folder_is_used_and_removed() {
        let d = tmpdir("lent");
        let p = d.join("a.pdf");
        fs::write(&p, b"%PDF-1.7 old").unwrap();
        let id = FileId::of(&p).unwrap();
        let journal = d.join("pending-saves.json");
        fn lend(dest: &Path) -> Option<PathBuf> {
            let folder = dest.with_file_name("lent");
            fs::create_dir(&folder).ok()?;
            Some(folder)
        }
        let inject = Inject {
            temp_folder: Some(lend),
            reopen: Some(Box::new(|temp| {
                assert!(temp.parent().unwrap().ends_with("lent"));
                fs::read(temp)
            })),
            ..Default::default()
        };
        let saved = save_in_place(b"%PDF-1.7 new!", &id, Some(&journal), &inject, &|| true);
        assert_eq!(saved.unwrap().warning, None);
        assert_eq!(fs::read(&p).unwrap(), b"%PDF-1.7 new!");
        assert!(!d.join("lent").exists());
        assert_eq!(read_journal(&journal).len(), 0);
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn orphans_in_a_lent_folder_are_forgotten_not_removed() {
        let d = tmpdir("orphan-lent");
        let journal = d.join("pending-saves.json");
        fs::create_dir(d.join("lent")).unwrap();
        let temp = d.join("lent").join(".a.pdf.0123abcd.tmp");
        fs::write(&temp, b"x").unwrap();
        journal_add(&journal, &temp, &d.join("a.pdf")).unwrap();
        cleanup_orphans(&journal);
        assert!(temp.exists());
        assert!(read_journal(&journal).is_empty());
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn orphans_are_removed() {
        let d = tmpdir("orphan");
        let journal = d.join("pending-saves.json");
        let temp = d.join(".a.pdf.0123abcd.tmp");
        fs::write(&temp, b"x").unwrap();
        journal_add(&journal, &temp, &d.join("a.pdf")).unwrap();
        cleanup_orphans(&journal);
        assert!(!temp.exists());
        assert!(read_journal(&journal).is_empty());
        fs::remove_dir_all(d).unwrap();
    }
}
