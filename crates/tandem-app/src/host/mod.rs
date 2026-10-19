//! What Electron's main process did (port of `src/main.ts`'s IPC handlers and
//! state): the one window's open file, opening and saving files, New, the
//! recent list, printing, web links, the insert/compare source, images, the
//! batch folder, the trust list, the drawn signature, certificate signing and
//! verification. `Host` is owned by the document session (`session.host`);
//! every IPC handler is a method with plain Rust arguments and results.
//!
//! Main → renderer messages (`contents.send('document' | 'open-error' |
//! 'edit-request' …)`) are queued as [`Event`]s: whoever calls into the host
//! drains them with [`Host::take_events`] and hands each to the session, in
//! order (the drive harness does that after its own host calls; the session
//! does it after its own).
//!
//! Native dialogs and the OS go through [`crate::platform::Platform`]; in a
//! smoke or drive run that is [`crate::platform::Scripted`] and the `smoke`
//! rules of main.ts apply (no discard or pending-boxes question, memory-only
//! stores, `P2_LINK` lines instead of opening links).
//!
//! Electron-only concerns are gone: there is one window (no per-sender maps),
//! no `isTrustedOpen` frame check (nothing untrusted can call in), and IPC
//! argument type checks are Rust types. "Busy" flags stay where a re-entrant
//! call is still possible from a UI.

pub mod blank;
pub mod drawn;
pub mod recent;
pub mod sign;
pub mod trust;
pub mod util;

use crate::platform::{
    BoxKind, Filter, MessageBox, PDF_FILTER, Pick, Platform, PrintOutcome, Question, SavePurpose,
};
use crate::safe_write::{self, FileId, Inject, MAX_SAVE_BYTES, SaveFail};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use util::basename;

/// The open ceiling (`renderer/limits.ts` MAX_PDF_BYTES, 120 MiB).
pub const MAX_PDF_BYTES: u64 = 120 * 1024 * 1024;
/// One image of New from Images / a stamp image (M:264).
pub const IMAGE_MAX_BYTES: u64 = 32 * 1024 * 1024;
pub const IMAGE_MAX_FILES: usize = 50;
pub const BATCH_MAX_FILES: usize = 200;
/// The batch output folder beside the inputs (M:91).
pub const BATCH_OUT: &str = "pdf4all-out";
/// `MAX_LINK_URI` (preload.cts:25).
pub const MAX_LINK_URI: usize = 2048;
/// A one-shot open is valid this long (M:468).
const OPEN_TOKEN_TTL: Duration = Duration::from_secs(30);

/// The packaged sample documents (`isFixture`, policy.ts:61).
pub const FIXTURES: [&str; 3] = ["hello.pdf", "form.pdf", "large.pdf"];

/// `isFixture(name)`.
pub fn is_fixture(name: &str) -> bool {
    FIXTURES.contains(&name)
}

/// Where a packaged sample lives: the app bundle's resources, beside the
/// binary, the working folder (repo runs), or this source tree (dev builds).
pub fn fixture_path(name: &str) -> Option<PathBuf> {
    if !is_fixture(name) {
        return None;
    }
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join("../Resources/fixtures").join(name));
        candidates.push(dir.join("fixtures").join(name));
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("fixtures").join(name));
    }
    candidates.push(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(name),
    );
    candidates.into_iter().find(|p| p.is_file())
}

/// The user-data folder when no `--user-data-dir` is given: Electron's
/// `app.getPath('userData')` for the product name "Tandem PDF".
pub fn default_user_data() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(util::temp_dir);
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Tandem PDF")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"))
            .join("Tandem PDF")
    }
}

/// The single-instance lock: an exclusive lock on `<userData>/SingletonLock`
/// held for the process lifetime (Electron's `requestSingleInstanceLock`). A
/// second instance on the same user-data folder fails to take it and exits 3
/// without touching anything else there (M:584-585).
#[derive(Debug)]
pub struct SingletonLock {
    path: PathBuf,
    file: Option<fs::File>,
}

impl SingletonLock {
    /// Takes the lock; `Ok(None)` when another live process holds it.
    pub fn acquire(user_data: &Path) -> std::io::Result<Option<SingletonLock>> {
        fs::create_dir_all(user_data)?;
        let path = user_data.join("SingletonLock");
        let mut file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        match file.try_lock() {
            Ok(()) => {}
            Err(fs::TryLockError::WouldBlock) => return Ok(None),
            Err(fs::TryLockError::Error(e)) => return Err(e),
        }
        // The holder's pid, for people looking at the folder (nothing reads it).
        let _ = file.set_len(0);
        let _ = write!(file, "{}", std::process::id());
        Ok(Some(SingletonLock {
            path,
            file: Some(file),
        }))
    }

    /// Removes the lock file and lets the lock go (before `process::exit`).
    pub fn release(&mut self) {
        if let Some(file) = self.file.take() {
            let _ = fs::remove_file(&self.path);
            let _ = file.unlock();
        }
    }
}

impl Drop for SingletonLock {
    fn drop(&mut self) {
        self.release();
    }
}

/// How a `Host` is set up.
#[derive(Clone, Debug)]
pub struct HostConfig {
    /// `--user-data-dir` (or [`default_user_data`]).
    pub user_data: PathBuf,
    /// A `--smoke` or `--drive` run: memory-only stores, no questions, P2_LINK lines.
    pub smoke: bool,
    /// Print `P2_LINK` lines on stdout (off in unit tests).
    pub echo_stdout: bool,
}

/// `document {name, size, url, untitled?}`: a file the session should open,
/// reading its bytes once with [`Host::read_open`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Document {
    pub name: String,
    pub size: u64,
    /// An opaque one-shot token (`app://pdf4all/open/<uuid>`); also the
    /// document's identity for `save_file`.
    pub url: String,
    /// A New document (Save asks for a path first).
    pub untitled: bool,
}

/// `open-error {reason, name?, limit?}`; `reason` is `cancelled`, `unreadable` or `too-large`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenError {
    pub reason: &'static str,
    pub name: Option<String>,
    pub limit: Option<u64>,
}

impl OpenError {
    pub fn cancelled() -> Self {
        OpenError {
            reason: "cancelled",
            name: None,
            limit: None,
        }
    }
    pub fn unreadable(name: Option<String>) -> Self {
        OpenError {
            reason: "unreadable",
            name,
            limit: None,
        }
    }
    pub fn too_large(name: String) -> Self {
        OpenError {
            reason: "too-large",
            name: Some(name),
            limit: Some(MAX_PDF_BYTES),
        }
    }
    /// The IPC payload (`{reason, name?, limit?}`).
    pub fn to_json(&self) -> serde_json::Value {
        let mut v = serde_json::json!({"reason": self.reason});
        if let Some(name) = &self.name {
            v["name"] = name.clone().into();
        }
        if let Some(limit) = self.limit {
            v["limit"] = limit.into();
        }
        v
    }
}

/// `save-request`: none = Save a Copy, `{inPlace:true}` = Save, `{as:true}` = Save As.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveRequest {
    Copy,
    InPlace,
    As,
}

/// A main → renderer message (preload.cts:83-195).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Document(Document),
    OpenError(OpenError),
    SaveRequest(SaveRequest),
    /// `edit-request {op, contents?}` (contents already within the preload caps).
    EditRequest {
        op: String,
        contents: Option<String>,
    },
    /// `print-request {range?}`: `None` opens the print dialog.
    PrintRequest {
        range: Option<String>,
    },
}

/// The window's bound file (`OpenRecord`, M:62).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenRecord {
    pub file: FileId,
    pub url: String,
    pub name: String,
}

/// What a redacted copy or original's open does to "Open original" (M:503).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Original {
    /// `undefined`: keep the current one.
    Keep,
    /// `null`: forget it.
    Forget,
    /// The file the redacted copy came from.
    Set(PathBuf),
}

#[derive(Debug)]
struct Token {
    path: PathBuf,
    size: u64,
    expires: Instant,
}

/// The pending-boxes question's answer (M:522).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Boxes {
    Redact,
    Discard,
    Cancel,
    None,
}

/// `save-file` options (M:816-831).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SaveOptions {
    pub in_place: bool,
    pub filled: bool,
    pub rebind: bool,
    pub suggested_name: Option<String>,
}

/// A save that landed (`SaveResult` ok, preload.cts:4-6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveOk {
    pub name: String,
    /// `source-changed` or `durability-uncertain`.
    pub warning: Option<&'static str>,
    /// Save As onto the open file itself (a plain Save).
    pub same_file: bool,
}

/// `save-new` kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveKind {
    Pdf,
    Png,
    Jpeg,
    Txt,
}

impl SaveKind {
    pub fn parse(text: &str) -> Option<SaveKind> {
        match text {
            "pdf" => Some(SaveKind::Pdf),
            "png" => Some(SaveKind::Png),
            "jpeg" => Some(SaveKind::Jpeg),
            "txt" => Some(SaveKind::Txt),
            _ => None,
        }
    }
    fn ext(self) -> &'static str {
        match self {
            SaveKind::Pdf => ".pdf",
            SaveKind::Png => ".png",
            SaveKind::Jpeg => ".jpg",
            SaveKind::Txt => ".txt",
        }
    }
    fn filter(self) -> Filter {
        match self {
            SaveKind::Pdf => PDF_FILTER,
            SaveKind::Png => Filter {
                name: "PNG",
                extensions: &["png"],
            },
            SaveKind::Jpeg => Filter {
                name: "JPEG",
                extensions: &["jpg", "jpeg"],
            },
            SaveKind::Txt => Filter {
                name: "Text",
                extensions: &["txt"],
            },
        }
    }
}

/// A `save-new` that landed: `url`/`original` for a redacted copy the session opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedNew {
    pub name: String,
    pub url: Option<String>,
    /// Whether the copy has an original to go back to.
    pub original: Option<bool>,
}

/// A one-shot open the host issued directly (`issueOpen`, M:490).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issued {
    pub name: String,
    pub url: String,
}

/// A file picked and read (`insert-source`, one of `pick-images`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickedFile {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// `pick-folder`'s answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderListing {
    pub folder: PathBuf,
    pub out: PathBuf,
    /// `(name, size in bytes)`, in the batch order.
    pub files: Vec<(String, u64)>,
}

/// `PrintResult` (M:63): `Err` is `cancelled`, `busy`, `failed` or `no-printer`.
pub type PrintResult = Result<(), &'static str>;

/// `Result<_, reason>` for the handlers whose failures are a bare reason token.
pub type Reason = &'static str;

/// Main's state for the one window, and the IPC handlers.
pub struct Host {
    pub smoke: bool,
    echo_stdout: bool,
    user_data: PathBuf,
    journal: PathBuf,
    pub platform: Box<dyn Platform>,
    pub recent: recent::RecentStore,
    pub trust: trust::TrustStore,
    pub signature: drawn::SignatureStore,
    open: Option<OpenRecord>,
    untitled: HashSet<PathBuf>,
    redact_original: Option<PathBuf>,
    redact_pending: Option<(u32, bool)>,
    pending_bind: Option<(OpenRecord, Original)>,
    tokens: HashMap<String, Token>,
    dirty: bool,
    save_as_enabled: bool,
    open_dialog: bool,
    save_busy: bool,
    print_busy: bool,
    batch: Option<(PathBuf, Vec<String>)>,
    pub(crate) cert: sign::CertState,
    events: Vec<Event>,
    last_p2_link: Option<(String, bool)>,
    last_print: Option<PrintResult>,
    /// Lines written to stdout (`P2_LINK …`), also kept here.
    pub stdout_lines: Vec<String>,
}

impl std::fmt::Debug for Host {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Host")
            .field("smoke", &self.smoke)
            .field("open", &self.open)
            .finish_non_exhaustive()
    }
}

impl Host {
    /// M:605-610: the stores (memory-only in a smoke/drive run) and the save
    /// journal, whose orphaned temps are removed now. The single-instance lock
    /// must already be held ([`SingletonLock::acquire`]).
    pub fn new(config: HostConfig, platform: Box<dyn Platform>) -> Host {
        let _ = fs::create_dir_all(&config.user_data);
        let journal = config.user_data.join("pending-saves.json");
        let file = |name: &str| (!config.smoke).then(|| config.user_data.join(name));
        let j = Some(journal.clone());
        safe_write::cleanup_orphans(&journal);
        Host {
            smoke: config.smoke,
            echo_stdout: config.echo_stdout,
            recent: recent::RecentStore::new(file("recent-files.json"), j.clone()),
            trust: trust::TrustStore::new(file("trusted-certs.pem"), j.clone()),
            signature: drawn::SignatureStore::new(file("drawn-signature.json"), j),
            user_data: config.user_data,
            journal,
            platform,
            open: None,
            untitled: HashSet::new(),
            redact_original: None,
            redact_pending: None,
            pending_bind: None,
            tokens: HashMap::new(),
            dirty: false,
            save_as_enabled: false,
            open_dialog: false,
            save_busy: false,
            print_busy: false,
            batch: None,
            cert: sign::CertState::default(),
            events: Vec::new(),
            last_p2_link: None,
            last_print: None,
            stdout_lines: Vec::new(),
        }
    }

    pub fn user_data(&self) -> &Path {
        &self.user_data
    }

    /// The save journal (`<userData>/pending-saves.json`).
    pub fn journal(&self) -> &Path {
        &self.journal
    }

    /// The window's bound file.
    pub fn open_record(&self) -> Option<&OpenRecord> {
        self.open.as_ref()
    }

    /// The last `open-link` of this smoke/drive run (`p2Link` in the report).
    pub fn last_p2_link(&self) -> Option<(&str, bool)> {
        self.last_p2_link.as_ref().map(|(u, a)| (u.as_str(), *a))
    }

    /// The last print's result, taken (`settleSmokePrint`, M:339).
    pub fn take_print_result(&mut self) -> Option<PrintResult> {
        self.last_print.take()
    }

    /// Main → renderer messages queued since the last call, in order.
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    fn send(&mut self, event: Event) {
        self.events.push(event);
    }

    fn stdout(&mut self, line: String) {
        if self.echo_stdout {
            let mut out = std::io::stdout().lock();
            let _ = writeln!(out, "{line}");
            let _ = out.flush();
        }
        self.stdout_lines.push(line);
    }

    /// A packaged sample's bytes (`fetch('/fixtures/<name>')`).
    pub fn fixture_bytes(&self, name: &str) -> Option<Vec<u8>> {
        fs::read(fixture_path(name)?).ok()
    }

    /// Housekeeping a UI calls now and then: drops a held certificate key after
    /// five idle minutes and forgets expired open tokens.
    pub fn tick(&mut self) {
        self.cert.expire_idle();
        let now = Instant::now();
        self.tokens.retain(|_, t| t.expires > now);
    }

    // ---------------------------------------------------------------- window state

    /// `setSaveEnabled` (M:388): Save As… and Save a Copy… follow an open document.
    fn set_save_enabled(&mut self, enabled: bool) {
        self.save_as_enabled = enabled;
        self.sync_menus();
    }

    /// `setWindowDirty` (M:395).
    fn set_window_dirty(&mut self, dirty: bool) {
        self.dirty = dirty;
        self.sync_menus();
    }

    fn sync_menus(&mut self) {
        let save = self.dirty && self.open.is_some();
        self.platform
            .set_document_state(self.dirty, save, self.save_as_enabled);
    }

    fn represent(&mut self) {
        match &self.open {
            Some(r) => {
                let (path, name) = (r.file.path.clone(), r.name.clone());
                let untitled = self.untitled.contains(&path);
                self.platform
                    .set_represented_file((!untitled).then_some(path.as_path()), &name);
            }
            None => self.platform.set_represented_file(None, "Tandem PDF"),
        }
    }

    /// Whether the window has unsaved changes (as the session last said).
    pub fn dirty(&self) -> bool {
        self.dirty
    }

    /// `dirty` (M:936): the session's unsaved-changes mark.
    pub fn set_dirty(&mut self, dirty: bool) {
        self.set_window_dirty(dirty);
    }

    /// `redact-pending` (M:940): unsaved redaction boxes; `memory` when they are
    /// a signed file's, which exist only while the file is open.
    pub fn set_redact_pending(&mut self, count: u32, memory: bool) {
        self.redact_pending = (count > 0).then_some((count, memory));
    }

    /// `askPendingBoxes` (M:522): unsaved boxes ask before the window closes or
    /// another file replaces them. "Redact Now…" also queues
    /// `edit-request {op:'redact-apply-open'}`.
    fn ask_pending_boxes(&mut self, close: bool) -> Boxes {
        let Some((count, memory)) = self.redact_pending.filter(|(c, _)| *c >= 1) else {
            return Boxes::None;
        };
        if self.smoke {
            return Boxes::None;
        }
        let question = pending_question(count, memory, close);
        let choice = self.platform.ask(&question);
        match choice {
            1 => Boxes::Discard,
            0 => {
                self.send(Event::EditRequest {
                    op: "redact-apply-open".into(),
                    contents: None,
                });
                Boxes::Redact
            }
            _ => Boxes::Cancel,
        }
    }

    /// The window's close button (M:759): `true` when it may close.
    pub fn close_requested(&mut self) -> bool {
        if self.smoke {
            return true;
        }
        match self.ask_pending_boxes(true) {
            Boxes::Discard => {
                self.set_window_dirty(false);
                self.redact_pending = None;
                return true;
            }
            Boxes::None => {}
            _ => return false,
        }
        if !self.dirty {
            return true;
        }
        if self.platform.ask(&discard_box()) != 0 {
            return false;
        }
        self.set_window_dirty(false);
        true
    }

    // ---------------------------------------------------------------- opening

    /// `inspectOpen` (M:380): the `--open` / `--insert-from` pre-check.
    pub fn inspect_open(path: &str) -> Option<OpenError> {
        let name = basename(Path::new(path));
        if path.is_empty() || !Path::new(path).is_absolute() {
            return Some(OpenError::unreadable(Some(name)));
        }
        match FileId::of(Path::new(path)) {
            None => Some(OpenError::unreadable(Some(name))),
            Some(meta) if meta.size > MAX_PDF_BYTES => Some(OpenError::too_large(name)),
            Some(_) => None,
        }
    }

    /// `open-file` (M:776): File ▸ Open… with the native sheet.
    pub fn open_file(&mut self) {
        self.deliver_open(None, false);
    }

    /// `deliverOpen(path?, {skipDiscard})` (M:409): asks what must be asked,
    /// then queues `document` or `open-error`.
    pub fn deliver_open(&mut self, file_path: Option<&Path>, skip_discard: bool) {
        // Another open is already asking: this one is answered "cancelled".
        if self.open_dialog {
            self.send(Event::OpenError(OpenError::cancelled()));
            return;
        }
        self.open_dialog = true;
        self.deliver_open_inner(file_path, skip_discard);
        self.open_dialog = false;
    }

    fn deliver_open_inner(&mut self, file_path: Option<&Path>, skip_discard: bool) {
        // D6: the signed copy carries the window's unsaved edits, so opening it asks neither the discard
        // question nor the pending-boxes one for marks; a signed file's in-memory boxes still ask.
        let boxes = if asks_boxes_on_open(skip_discard, self.redact_pending) {
            self.ask_pending_boxes(false)
        } else {
            Boxes::None
        };
        if matches!(boxes, Boxes::Redact | Boxes::Cancel) {
            self.send(Event::OpenError(OpenError::cancelled()));
            return;
        }
        if !self.smoke
            && !skip_discard
            && boxes == Boxes::None
            && self.dirty
            && self.platform.ask(&discard_box()) != 0
        {
            self.send(Event::OpenError(OpenError::cancelled()));
            return;
        }
        let path = match file_path.filter(|p| !p.as_os_str().is_empty()) {
            Some(p) => p.to_path_buf(),
            None => match self.platform.pick_file(Pick::Open, &[PDF_FILTER]) {
                Some(p) => p,
                None => {
                    self.send(Event::OpenError(OpenError::cancelled()));
                    return;
                }
            },
        };
        let name = basename(&path);
        let Some(meta) = FileId::of(&path) else {
            self.recent.remove(&path); // a recent file that is gone leaves the start screen's list
            self.send(Event::OpenError(OpenError::unreadable(Some(name))));
            return;
        };
        if meta.size > MAX_PDF_BYTES {
            self.send(Event::OpenError(OpenError::too_large(name)));
            return;
        }
        let url = self.issue_token(&path, meta.size);
        let size = meta.size;
        self.open = Some(OpenRecord {
            file: meta,
            url: url.clone(),
            name: name.clone(),
        });
        // Review L4: a file opened by hand is not a redacted copy; "Open original" no longer applies.
        self.redact_original = None;
        self.redact_pending = None;
        self.pending_bind = None;
        let untitled = self.untitled.contains(&path);
        // M9: an Untitled document's Save asks for a path first.
        self.send(Event::Document(Document {
            name,
            size,
            url,
            untitled,
        }));
        if !untitled {
            self.recent.add(&path);
            if !self.smoke {
                self.platform.add_recent_document(&path);
            }
        }
        self.set_window_dirty(false);
        self.set_save_enabled(true);
        self.represent();
    }

    fn issue_token(&mut self, path: &Path, size: u64) -> String {
        let now = Instant::now();
        self.tokens.retain(|_, t| t.expires > now);
        let token = util::uuid();
        self.tokens.insert(
            token.clone(),
            Token {
                path: path.to_path_buf(),
                size,
                expires: now + OPEN_TOKEN_TTL,
            },
        );
        format!("app://pdf4all/open/{token}")
    }

    /// `serveOpen` (M:354): the bytes behind a one-shot open URL, once, within
    /// 30 s, and only while the file still has the size it was issued with.
    pub fn read_open(&mut self, url: &str) -> Option<Vec<u8>> {
        let token = url.strip_prefix("app://pdf4all/open/")?;
        let issued = self.tokens.remove(token)?;
        if Instant::now() > issued.expires || issued.size == 0 {
            return None;
        }
        let meta = FileId::of(&issued.path)?;
        if meta.size != issued.size {
            return None;
        }
        let bytes = safe_write::read_capped(&issued.path, issued.size).ok()?;
        (bytes.len() as u64 == issued.size).then_some(bytes)
    }

    /// `issueOpen` (M:490): for the redacted copy and "Open original" the host
    /// hands the URL straight back; the session opens it, then calls `bind_open`.
    pub fn issue_open(&mut self, path: &Path, original: Original) -> Result<Issued, Reason> {
        let meta = FileId::of(path).ok_or("unreadable")?;
        if meta.size > MAX_PDF_BYTES {
            return Err("too-large");
        }
        let name = basename(path);
        let url = self.issue_token(path, meta.size);
        self.pending_bind = Some((
            OpenRecord {
                file: meta,
                url: url.clone(),
                name: name.clone(),
            },
            original,
        ));
        Ok(Issued { name, url })
    }

    /// `bind-open` (M:505): the window now shows the issued file: Save, the
    /// dirty mark and "Open original" follow it.
    pub fn bind_open(&mut self, url: &str) -> bool {
        let Some((record, original)) = self.pending_bind.take() else {
            return false;
        };
        if record.url != url {
            self.pending_bind = Some((record, original));
            return false;
        }
        let path = record.file.path.clone();
        self.open = Some(record);
        match original {
            Original::Set(p) => self.redact_original = Some(p),
            Original::Forget => self.redact_original = None,
            Original::Keep => {}
        }
        self.redact_pending = None;
        self.set_window_dirty(false);
        self.set_save_enabled(true);
        // UX audit X4: the redacted copy (or the original reopened from its banner) is a recent file too.
        self.recent.add(&path);
        if !self.smoke {
            self.platform.add_recent_document(&path);
        }
        self.represent();
        true
    }

    /// `open-original` (M:947): the redacted copy's banner reopens the file it came from.
    pub fn open_original(&mut self) -> Result<Issued, Reason> {
        let path = self.redact_original.clone().ok_or("unreadable")?;
        let boxes = self.ask_pending_boxes(false);
        if matches!(boxes, Boxes::Redact | Boxes::Cancel) {
            return Err("cancelled");
        }
        if !self.smoke
            && boxes == Boxes::None
            && self.dirty
            && self.platform.ask(&discard_box()) != 0
        {
            return Err("cancelled");
        }
        self.issue_open(&path, Original::Forget)
    }

    /// `new-blank` / the drive `new` step (M:538): a blank one-page PDF written
    /// as `Untitled.pdf` in a fresh temp folder, opened like any file. `size` is
    /// `None` for JS `undefined` (Letter); see [`blank::page_size_of`].
    // ponytail: each New leaves its temp folder behind (the OS temp cleaner owns it).
    pub fn new_blank(&mut self, size: Option<&serde_json::Value>) {
        let name = "Untitled.pdf".to_string();
        let Some((w, h)) = blank::page_size_of(size) else {
            self.send(Event::OpenError(OpenError::unreadable(Some(name))));
            return;
        };
        let written = util::mkdtemp(&util::temp_dir().join("pdf4all-new-"))
            .ok()
            .and_then(|dir| {
                let path = dir.join(&name);
                safe_write::write_new(
                    Some(&path),
                    &blank::blank_pdf(w, h),
                    Some(&self.journal),
                    &Inject::default(),
                )
                .ok()
                .map(|_| path)
            });
        let Some(path) = written else {
            self.send(Event::OpenError(OpenError::unreadable(Some(name))));
            return;
        };
        self.untitled.insert(path.clone());
        self.deliver_open(Some(&path), false);
    }

    /// `recent-list` (M:785).
    pub fn recent_list(&mut self) -> Vec<recent::RecentEntry> {
        self.recent.list()
    }

    /// `recent-clear` (M:786).
    pub fn recent_clear(&mut self) {
        self.recent.clear();
        if !self.smoke {
            self.platform.clear_recent_documents();
        }
    }

    /// `recent-open` (M:791): an entry no longer listed opens nothing.
    pub fn recent_open(&mut self, id: &str) {
        match self.recent.path(id) {
            Some(path) => self.deliver_open(Some(&path), false),
            None => self.send(Event::OpenError(OpenError::unreadable(None))),
        }
    }

    /// `open-path` (M:798): a dropped file, by absolute path.
    pub fn open_path(&mut self, path: &str) {
        if !Path::new(path).is_absolute() {
            let name = (!path.is_empty()).then(|| basename(Path::new(path)));
            self.send(Event::OpenError(OpenError::unreadable(name)));
            return;
        }
        self.deliver_open(Some(Path::new(path)), false);
    }

    // ---------------------------------------------------------------- saving

    /// `save-file` (M:808): Save, Save As and Save a Copy of the open document.
    pub fn save_file(
        &mut self,
        bytes: &[u8],
        document_url: &str,
        options: &SaveOptions,
    ) -> Result<SaveOk, SaveFail> {
        if self.save_busy {
            return Err(SaveFail::Busy);
        }
        if document_url.is_empty() {
            return Err(SaveFail::DocumentChanged);
        }
        let current = match &self.open {
            Some(r) if r.url == document_url => r.clone(),
            recorded => {
                return Err(if options.in_place && recorded.is_none() {
                    SaveFail::Unreadable
                } else {
                    SaveFail::DocumentChanged
                });
            }
        };
        if bytes.len() > MAX_SAVE_BYTES {
            return Err(SaveFail::TooLarge);
        }
        if bytes.len() < 5 {
            return Err(SaveFail::Unreadable);
        }
        self.save_busy = true;
        let result = self.save_file_inner(bytes, current, options);
        self.save_busy = false;
        result
    }

    fn save_file_inner(
        &mut self,
        bytes: &[u8],
        current: OpenRecord,
        options: &SaveOptions,
    ) -> Result<SaveOk, SaveFail> {
        let SaveOptions {
            in_place,
            filled,
            rebind,
            ..
        } = *options;
        let suggested = suggested_kind_name(options.suggested_name.as_deref(), SaveKind::Pdf);
        let journal = self.journal.clone();
        let untitled = in_place && self.untitled.contains(&current.file.path);
        let mut as_copy = in_place && (untitled || filled);
        // UX item 8: the first Save of a filled form asks once: over the original, or as a new file
        // (a drive line's `filled` pre-answers it). Review fix 5: Return answers "Save as New File…".
        if in_place && filled && !untitled {
            match self.platform.ask(&filled_box(&current.name)) {
                0 => as_copy = false,
                1 => {}
                _ => return Err(SaveFail::Cancelled),
            }
        }
        if in_place && !as_copy {
            let saved = safe_write::save_in_place(
                bytes,
                &current.file,
                Some(&journal),
                &Inject::default(),
                &|| true,
            )?;
            return Ok(self.saved_in_place(saved, &current, false));
        }
        let is_untitled = self.untitled.contains(&current.file.path);
        let default_path = if untitled || (rebind && is_untitled) {
            PathBuf::from("Untitled.pdf")
        } else if as_copy {
            let base = basename(&current.file.path);
            let stem = strip_pdf_ext(&base);
            current
                .file
                .path
                .parent()
                .unwrap_or(Path::new("/"))
                .join(format!("{stem} (filled).pdf"))
        } else if rebind {
            current.file.path.clone()
        } else {
            PathBuf::from(suggested.unwrap_or_else(|| safe_write::copy_basename(&current.name)))
        };
        let dest = self
            .platform
            .pick_save(SavePurpose::Document, &default_path, &[PDF_FILTER])
            .ok_or(SaveFail::Cancelled)?;
        // Save As onto the open file itself is a plain Save, whatever the spelling (same path, a symbolic
        // link, another case on a case-insensitive disk). A hard link is a name of its own.
        if rebind && !is_untitled && same_file_name(&dest, &current.file) {
            let saved = safe_write::save_in_place(
                bytes,
                &current.file,
                Some(&journal),
                &Inject::default(),
                &|| true,
            )?;
            return Ok(self.saved_in_place(saved, &current, true));
        }
        let copied = safe_write::save_copy(
            Some(&dest),
            bytes,
            &current.file,
            Some(&journal),
            rebind,
            &Inject::default(),
            &|| true,
        )?;
        if as_copy || rebind {
            // Save As (and Save on Untitled or filled): the window now edits the written file, bound to the
            // identity the write took right after its rename.
            let Some(file) = copied.file.clone() else {
                // Review F5: never report a Save As the window was not rebound to.
                return Err(SaveFail::WriteFailed);
            };
            let path = file.path.clone();
            self.open = Some(OpenRecord {
                file,
                url: current.url.clone(),
                name: copied.name.clone(),
            });
            self.untitled.remove(&current.file.path);
            self.redact_original = None;
            // Review fix 15: the file the window now edits is a recent file too.
            self.recent.add(&path);
            if !self.smoke {
                self.platform.add_recent_document(&path);
            }
            self.represent();
        }
        Ok(SaveOk {
            name: copied.name,
            warning: copied.warning,
            same_file: false,
        })
    }

    fn saved_in_place(
        &mut self,
        saved: safe_write::Saved,
        current: &OpenRecord,
        same_file: bool,
    ) -> SaveOk {
        if let Some(file) = saved.file {
            self.open = Some(OpenRecord {
                file,
                url: current.url.clone(),
                name: current.name.clone(),
            });
        }
        SaveOk {
            name: saved.name,
            warning: saved.warning,
            same_file,
        }
    }

    /// `save-new` (M:1007): a new file (export, extracted pages, a redacted copy,
    /// new from images). `redacted` (PDF only) refuses the open file as the
    /// destination and issues the copy's open; `open_after` opens a PDF after
    /// writing it (not in a smoke run).
    pub fn save_new(
        &mut self,
        bytes: &[u8],
        suggested_name: Option<&str>,
        kind: SaveKind,
        open_after: bool,
        redacted: bool,
    ) -> Result<SavedNew, SaveFail> {
        if self.save_busy {
            return Err(SaveFail::Busy);
        }
        if bytes.is_empty() {
            return Err(SaveFail::Unreadable);
        }
        if bytes.len() > MAX_SAVE_BYTES {
            return Err(SaveFail::TooLarge);
        }
        let redacted = redacted && kind == SaveKind::Pdf;
        self.save_busy = true;
        let result = (|| {
            let default_path = suggested_kind_name(suggested_name, kind)
                .unwrap_or_else(|| format!("untitled{}", kind.ext()));
            let dest = self
                .platform
                .pick_save(SavePurpose::New, Path::new(&default_path), &[kind.filter()])
                .ok_or(SaveFail::Cancelled)?;
            // Review H1: the redacted copy never replaces the file it was made from (same path, link or inode).
            let source = self.open.clone();
            if redacted
                && source
                    .as_ref()
                    .is_some_and(|s| safe_write::dest_conflicts_source(&dest, &s.file, false))
            {
                return Err(SaveFail::SameFile);
            }
            let written =
                safe_write::write_new(Some(&dest), bytes, Some(&self.journal), &Inject::default())
                    // M10 review: a read-only folder or target and a full disk keep their own text.
                    .map_err(|e| {
                        if e == SaveFail::WriteFailed {
                            SaveFail::Unreadable
                        } else {
                            e
                        }
                    })?;
            if redacted {
                let original = source
                    .as_ref()
                    .map_or(Original::Forget, |s| Original::Set(s.file.path.clone()));
                return Ok(match self.issue_open(&dest, original) {
                    Ok(issued) => SavedNew {
                        name: written.name,
                        url: Some(issued.url),
                        original: Some(source.is_some()),
                    },
                    Err(_) => SavedNew {
                        name: written.name,
                        url: None,
                        original: None,
                    },
                });
            }
            if kind == SaveKind::Pdf && open_after && !self.smoke {
                self.deliver_open(Some(&dest), false);
            }
            Ok(SavedNew {
                name: written.name,
                url: None,
                original: None,
            })
        })();
        self.save_busy = false;
        result
    }

    // ---------------------------------------------------------------- sources

    /// `insert-source` (M:980): the PDF for Insert Pages and Compare.
    pub fn insert_source(&mut self) -> Result<PickedFile, Reason> {
        let path = self
            .platform
            .pick_file(Pick::InsertSource, &[PDF_FILTER])
            .ok_or("cancelled")?;
        let name = basename(&path);
        if !path.is_absolute() {
            return Err("unreadable");
        }
        let meta = FileId::of(&path).ok_or("unreadable")?;
        if meta.size > MAX_PDF_BYTES {
            return Err("too-large");
        }
        let bytes = safe_write::read_capped(&path, MAX_PDF_BYTES).map_err(|_| "unreadable")?;
        Ok(PickedFile { name, bytes })
    }

    /// `pick-images` (M:1227): at most 50 files, each at most 32 MiB, all within the PDF ceiling.
    pub fn pick_images(&mut self) -> Result<Vec<PickedFile>, Reason> {
        let paths = self.platform.pick_images();
        if paths.is_empty() {
            return Err("cancelled");
        }
        if paths.len() > IMAGE_MAX_FILES {
            return Err("too-many");
        }
        let mut images = Vec::new();
        let mut total = 0u64;
        for path in paths {
            if !path.is_absolute() {
                return Err("unreadable");
            }
            let meta = FileId::of(&path).ok_or("unreadable")?;
            if meta.size > IMAGE_MAX_BYTES || total + meta.size > MAX_PDF_BYTES {
                return Err("too-large");
            }
            let bytes =
                safe_write::read_capped(&path, IMAGE_MAX_BYTES).map_err(|_| "unreadable")?;
            total += bytes.len() as u64;
            images.push(PickedFile {
                name: basename(&path),
                bytes,
            });
        }
        Ok(images)
    }

    /// `pick-folder` (M:1262): the batch folder's PDFs, in the batch order.
    pub fn pick_folder(&mut self) -> Result<FolderListing, Reason> {
        let folder = self.platform.pick_folder().ok_or("cancelled")?;
        if !folder.is_absolute() {
            return Err("unreadable");
        }
        let entries = fs::read_dir(&folder).map_err(|_| "unreadable")?;
        let mut names: Vec<String> = entries
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.to_lowercase().ends_with(".pdf"))
            .collect();
        // One order for the queue and the Bates numbers (§9.8).
        names.sort_by(|a, b| util::natural_cmp(a, b));
        if names.len() > BATCH_MAX_FILES {
            return Err("too-many-files");
        }
        let files: Vec<(String, u64)> = names
            .into_iter()
            .filter_map(|name| FileId::of(&folder.join(&name)).map(|m| (name, m.size)))
            .collect();
        self.batch = Some((
            folder.clone(),
            files.iter().map(|(n, _)| n.clone()).collect(),
        ));
        Ok(FolderListing {
            out: folder.join(BATCH_OUT),
            folder,
            files,
        })
    }

    fn batch_listing(&self) -> Result<(PathBuf, Vec<String>), Reason> {
        self.batch.clone().ok_or("not-listed")
    }

    /// `batch-io read` (M:1298): a listed input's bytes.
    pub fn batch_read(&mut self, name: &str) -> Result<Vec<u8>, Reason> {
        let (folder, names) = self.batch_listing()?;
        if !names.iter().any(|n| n == name) {
            return Err("not-listed");
        }
        let path = folder.join(name);
        let meta = FileId::of(&path).ok_or("unreadable")?;
        if meta.size > MAX_PDF_BYTES {
            return Err("too-large");
        }
        safe_write::read_capped(&path, MAX_PDF_BYTES).map_err(|_| "unwritable")
    }

    /// `batch-io exists` (M:1313): whether `pdf4all-out/<name>` is taken.
    pub fn batch_exists(&mut self, name: &str) -> Result<bool, Reason> {
        let (folder, names) = self.batch_listing()?;
        let safe = batch_out_name(name, &names).ok_or("bad-name")?;
        Ok(fs::symlink_metadata(folder.join(BATCH_OUT).join(safe)).is_ok())
    }

    /// `batch-io write` (M:1314): writes `pdf4all-out/<name>`, never through a
    /// symlinked folder and never over an existing file.
    pub fn batch_write(&mut self, name: &str, bytes: &[u8]) -> Result<String, Reason> {
        let (folder, names) = self.batch_listing()?;
        let safe = batch_out_name(name, &names).ok_or("bad-name")?.to_string();
        if bytes.is_empty() {
            return Err("unreadable");
        }
        if bytes.len() > MAX_SAVE_BYTES {
            return Err("too-large");
        }
        let out = folder.join(BATCH_OUT);
        let dest = out.join(&safe);
        match fs::create_dir(&out) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err("unwritable"),
        }
        let dir = fs::symlink_metadata(&out).map_err(|_| "unwritable")?;
        if !dir.is_dir() || dir.file_type().is_symlink() {
            return Err("unwritable");
        }
        if fs::symlink_metadata(&dest).is_ok() {
            return Err("exists");
        }
        match safe_write::write_new(Some(&dest), bytes, Some(&self.journal), &Inject::default()) {
            Ok(_) => Ok(safe),
            Err(SaveFail::TooLarge) => Err("too-large"),
            Err(_) => Err("unwritable"),
        }
    }

    // ---------------------------------------------------------------- links and printing

    /// `open-link` (M:960): only http, https and mailto, asked first. A smoke or
    /// drive run logs `P2_LINK {"uri","allowed"}` instead and opens nothing.
    pub fn open_link(&mut self, uri: &str) {
        if uri.is_empty() || uri.encode_utf16().count() > MAX_LINK_URI {
            return;
        }
        let allowed = link_allowed(uri);
        if self.smoke {
            self.last_p2_link = Some((uri.to_string(), allowed));
            let line = format!(
                "P2_LINK {}",
                serde_json::json!({"uri": uri, "allowed": allowed})
            );
            self.stdout(line);
            return;
        }
        if !allowed {
            return;
        }
        let question = MessageBox {
            question: Question::OpenLink,
            kind: BoxKind::Question,
            message: format!("Open {uri} in your browser?"),
            detail: None,
            buttons: vec!["Open".into(), "Cancel".into()],
            default: 0,
            cancel: 1,
        };
        if self.platform.ask(&question) == 0 {
            self.platform.open_external(uri);
        }
    }

    /// `print` (M:1327): the session hands over the print as a PDF. A smoke with
    /// `--print-to` (or a drive `print-to` step) writes it there; otherwise the
    /// system print dialog, and with no printer a save-as-PDF sheet.
    pub fn print(&mut self, pdf: &[u8]) -> PrintResult {
        if self.print_busy {
            return Err("busy");
        }
        self.print_busy = true;
        let result = self.print_inner(pdf);
        self.print_busy = false;
        self.last_print = Some(result);
        result
    }

    fn print_inner(&mut self, pdf: &[u8]) -> PrintResult {
        let print_to = if self.smoke {
            self.platform.answers().and_then(|a| a.print_to.clone())
        } else {
            None
        };
        if let Some(target) = print_to {
            if !target.is_absolute() {
                return Err("failed");
            }
            return safe_write::write_new(
                Some(&target),
                pdf,
                Some(&self.journal),
                &Inject::default(),
            )
            .map(|_| ())
            .map_err(|_| "failed");
        }
        let name = self
            .open
            .as_ref()
            .map_or("document.pdf".to_string(), |r| r.name.clone());
        match self.platform.print(pdf, &name) {
            PrintOutcome::Printed => Ok(()),
            PrintOutcome::Cancelled => Err("cancelled"),
            PrintOutcome::Failed => Err("failed"),
            PrintOutcome::NoPrinter => {
                let default_path = PathBuf::from(format!("{}-print.pdf", strip_pdf_ext(&name)));
                let picked = self
                    .platform
                    .pick_save(SavePurpose::Print, &default_path, &[PDF_FILTER])
                    .ok_or("cancelled")?;
                // Never over the open file.
                if self.open.as_ref().is_some_and(|r| r.file.path == picked) {
                    return Err("failed");
                }
                safe_write::write_new(Some(&picked), pdf, Some(&self.journal), &Inject::default())
                    .map(|_| ())
                    .map_err(|_| "failed")
            }
        }
    }
}

/// `asksBoxesOnOpen` (redact-ui.ts:51).
fn asks_boxes_on_open(skip_discard: bool, pending: Option<(u32, bool)>) -> bool {
    match pending {
        Some((count, memory)) if count >= 1 => !skip_discard || memory,
        _ => false,
    }
}

/// `DISCARD_BOX` (M:95, MSG.main.discard).
pub fn discard_box() -> MessageBox {
    MessageBox {
        question: Question::Discard,
        kind: BoxKind::Warning,
        message: "Discard unsaved changes?".into(),
        detail: Some("Your changes since the last save will be lost.".into()),
        buttons: vec!["Discard Changes".into(), "Cancel".into()],
        default: 1,
        cancel: 1,
    }
}

/// `MSG.filled.box(name)` (messages.ts:232): Return never overwrites the original; Esc cancels.
pub fn filled_box(name: &str) -> MessageBox {
    MessageBox {
        question: Question::Filled,
        kind: BoxKind::Question,
        message: format!("Save over “{name}” or as a new file?"),
        detail: Some("You filled in this form. Saving over it replaces the original; a new file keeps the original empty.".into()),
        buttons: vec!["Save Over".into(), "Save as New File…".into(), "Cancel".into()],
        default: 1,
        cancel: 2,
    }
}

/// `MSG.redact.pendingQuestion(count, memory, action)` (messages.ts:90) as main's box (M:526).
pub fn pending_question(count: u32, memory: bool, close: bool) -> MessageBox {
    let verb = if close {
        "Closing"
    } else {
        "Opening another file"
    };
    let areas = if count == 1 {
        "1 area".to_string()
    } else {
        format!("{count} areas")
    };
    let detail = if memory {
        format!(
            "These boxes are kept only while the file is open. {verb} now discards them. Nothing has been removed from the file."
        )
    } else {
        format!(
            "{verb} now discards the boxes and any other unsaved changes. Nothing has been removed from the file."
        )
    };
    MessageBox {
        question: Question::PendingBoxes,
        kind: BoxKind::Warning,
        message: format!(
            "{areas} {} marked but not redacted yet.",
            if count == 1 { "is" } else { "are" }
        ),
        detail: Some(detail),
        buttons: vec![
            "Redact Now…".into(),
            if close {
                "Close Without Redacting"
            } else {
                "Discard the Boxes"
            }
            .into(),
            "Cancel".into(),
        ],
        default: 0,
        cancel: 2,
    }
}

/// `linkAllowed` (M:168): `new URL(uri).protocol` is `http:`, `https:` or
/// `mailto:`. Like the URL parser, leading/trailing spaces and controls are
/// ignored and an http(s) URL needs a host.
pub fn link_allowed(uri: &str) -> bool {
    let text = uri.trim_matches(|c: char| c <= ' ');
    let Some((scheme, rest)) = text.split_once(':') else {
        return false;
    };
    let valid_scheme = scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    if !valid_scheme {
        return false;
    }
    match scheme.to_ascii_lowercase().as_str() {
        "mailto" => true,
        "http" | "https" => {
            let after = rest.trim_start_matches(['/', '\\']);
            let authority = after.split(['/', '\\', '?', '#']).next().unwrap_or("");
            let host = authority.rsplit('@').next().unwrap_or("");
            let host = host.split(':').next().unwrap_or("");
            !host.is_empty()
        }
        _ => false,
    }
}

fn strip_pdf_ext(name: &str) -> &str {
    if name.len() >= 4
        && name.is_char_boundary(name.len() - 4)
        && name[name.len() - 4..].eq_ignore_ascii_case(".pdf")
    {
        &name[..name.len() - 4]
    } else {
        name
    }
}

/// `suggestedKindName` (M:271; `suggestedPdfName` M:206 is the PDF case): a
/// bare file name of at most 255 characters, with the kind's extension added
/// when missing.
pub fn suggested_kind_name(raw: Option<&str>, kind: SaveKind) -> Option<String> {
    let raw = raw?;
    let len = |s: &str| s.encode_utf16().count();
    if raw.is_empty() || len(raw) > 255 || basename(Path::new(raw)) != raw {
        return None;
    }
    let lower = raw.to_lowercase();
    let name =
        if lower.ends_with(kind.ext()) || (kind == SaveKind::Jpeg && lower.ends_with(".jpeg")) {
            raw.to_string()
        } else {
            format!("{raw}{}", kind.ext())
        };
    (len(&name) <= 255).then_some(name)
}

/// `batchOutName` (policy.ts:76): a listed file name or `report.txt`, never a path.
pub fn batch_out_name<'a>(name: &'a str, listed: &[String]) -> Option<&'a str> {
    if name.is_empty() || name.encode_utf16().count() > 255 || name == "." || name == ".." {
        return None;
    }
    if name.contains(['/', '\\', '\0']) {
        return None;
    }
    (name == "report.txt" || listed.iter().any(|n| n == name)).then_some(name)
}

/// `sameFileName` (M:562): whether `dest` names the open file itself (same
/// device and inode, links followed) by the same real path, or by the same
/// real path in another letter case on a disk that ignores case. Two names of
/// one file (hard links) are not the same name.
pub fn same_file_name(dest: &Path, current: &FileId) -> bool {
    use std::os::unix::fs::MetadataExt;
    let (Ok(chosen), Ok(real), Ok(open)) = (
        fs::metadata(dest),
        fs::canonicalize(dest),
        fs::canonicalize(&current.path),
    ) else {
        return false;
    };
    if chosen.dev() != current.dev || chosen.ino() != current.ino {
        return false;
    }
    if real == open {
        return true;
    }
    if real.to_string_lossy().to_lowercase() != open.to_string_lossy().to_lowercase() {
        return false;
    }
    let name = basename(&open);
    if !name.chars().any(|c| c.is_ascii_alphabetic()) {
        return true;
    }
    let swapped: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() {
                c.to_ascii_uppercase()
            } else {
                c.to_ascii_lowercase()
            }
        })
        .collect();
    let other = open.parent().unwrap_or(Path::new("/")).join(swapped);
    fs::metadata(other).is_ok_and(|m| m.dev() == current.dev && m.ino() == current.ino)
}

#[cfg(test)]
mod tests;
