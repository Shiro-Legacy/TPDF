//! File safety: journaled safe writes, open caps and the size ceilings (Step 6 port of the
//! Electron-era tests/p1-save.test.mjs, m10-save-failure.test.mjs, save-as-rebind.test.mjs,
//! p1-open.test.mjs and p0.test.mjs cases that called build/safe-write.js and build/policy.js).
//!
//! Every case checks that the file a person had is left byte-identical when a save does not
//! happen, that no temp is left beside it, and that the refusal has a plain sentence to show.
//! Symbolic links and permission bits are Unix-only here (the JS skipped them on Windows, and
//! when running as root, which ignores permission bits).
// Some helpers serve only the Unix tests (modes, links).
#![cfg_attr(not(unix), allow(unused_imports, dead_code))]

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tandem_app::fsx;
use tandem_app::host::{self, Document, Event, Host, HostConfig, SaveOptions, util};
use tandem_app::messages;
use tandem_app::platform::Scripted;
use tandem_app::safe_write::{
    self, FileId, Inject, MAX_SAVE_BYTES, SaveFail, cleanup_orphans, read_capped, save_copy,
    save_in_place, write_new,
};

// ------------------------------------------------------------------ helpers

/// A private scratch folder, removed (after making it writable again) when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let prefix = util::temp_dir().join(format!("tandem-sw-{tag}-{}-{n}-", std::process::id()));
        let dir = util::mkdtemp(&prefix).expect("scratch folder");
        Scratch(fs::canonicalize(dir).expect("canonical scratch"))
    }
    fn path(&self) -> &Path {
        &self.0
    }
    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fsx::set_mode(&self.0, 0o755);
        let _ = fs::remove_dir_all(&self.0);
    }
}

const BYTES: &[u8] = b"%PDF-n";
const ORIGINAL: &[u8] = b"%PDF-original-bytes";
const EDITED: &[u8] = b"%PDF-edited-bytes-longer";
#[cfg_attr(not(unix), allow(dead_code))]
const PRECIOUS: &[u8] = b"%PDF-precious-second";

/// p1-save's `scratch()`: a source file and its identity, and a destination beside it.
struct Save {
    dir: Scratch,
    source_path: PathBuf,
    source: FileId,
    dest: PathBuf,
}

fn scratch(tag: &str, contents: &[u8]) -> Save {
    let dir = Scratch::new(tag);
    let source_path = dir.join("src.pdf");
    fs::write(&source_path, contents).unwrap();
    let source = FileId::of(&source_path).expect("source identity");
    let dest = dir.join("out.pdf");
    Save {
        dir,
        source_path,
        source,
        dest,
    }
}

/// Save temps (`.name.<random>.tmp`) left in `dir`.
fn temps(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with('.') && n.ends_with(".tmp"))
        .collect();
    names.sort();
    names
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn journal(path: &Path) -> Vec<Value> {
    let text = fs::read_to_string(path).expect("journal file");
    match serde_json::from_str(&text).expect("journal json") {
        Value::Array(items) => items,
        other => panic!("journal is not a list: {other}"),
    }
}

fn none() -> Inject {
    Inject::default()
}

fn yes() -> bool {
    true
}

fn os_error(code: i32) -> io::Error {
    io::Error::from_raw_os_error(code)
}

const ENOSPC: i32 = 28;
const EACCES: i32 = 13;
const EIO: i32 = 5;

fn key(path: &Path) -> (u64, u64) {
    fsx::file_key(&fs::metadata(path).unwrap())
}

#[cfg_attr(not(unix), allow(dead_code))]
fn mode(path: &Path) -> u32 {
    fsx::mode_of(&fs::metadata(path).unwrap()) & 0o777
}

/// The mode a new file gets here (0666 minus the umask), measured independently of the app.
#[cfg(unix)]
fn new_file_mode(dir: &Path) -> u32 {
    use std::os::unix::fs::OpenOptionsExt;
    let probe = dir.join("umask-probe");
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o666)
        .open(&probe)
        .unwrap();
    let bits = fsx::mode_of(&file.metadata().unwrap()) & 0o777;
    drop(file);
    fs::remove_file(probe).unwrap();
    bits
}

/// Whether permission bits stop this process (false for root, which writes read-only files).
#[cfg(unix)]
fn perms_enforced() -> bool {
    use std::sync::OnceLock;
    static ENFORCED: OnceLock<bool> = OnceLock::new();
    *ENFORCED.get_or_init(|| {
        let dir = Scratch::new("perm-probe");
        let probe = dir.join("ro");
        fs::write(&probe, b"x").unwrap();
        fsx::set_mode(&probe, 0o444).unwrap();
        fs::OpenOptions::new().write(true).open(&probe).is_err()
    })
}

/// Skips a permission test (prints why) where permission bits do not stop us.
#[cfg(unix)]
macro_rules! need_perms {
    () => {
        if !perms_enforced() {
            eprintln!("skipped: needs POSIX permission bits and a non-root user");
            return;
        }
    };
}

#[cfg(unix)]
fn symlink(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).unwrap();
}

#[cfg(unix)]
fn retarget(link: &Path, target: &Path) {
    fs::remove_file(link).unwrap();
    symlink(target, link);
}

fn smoke_host(dir: &Scratch) -> Host {
    Host::new(
        HostConfig {
            user_data: dir.join("udata"),
            smoke: true,
            echo_stdout: false,
        },
        Box::new(Scripted::default()),
    )
}

fn open(host: &mut Host, path: &Path) -> Document {
    host.deliver_open(Some(path), false);
    match host.take_events().as_slice() {
        [Event::Document(doc)] => doc.clone(),
        other => panic!("expected a document, got {other:?}"),
    }
}

fn save_to(host: &mut Host, path: Option<PathBuf>) {
    host.platform.answers().expect("scripted answers").save_to = path;
}

/// Every reason a save can fail with. A new variant does not compile here until listed.
fn all_fails() -> Vec<SaveFail> {
    use SaveFail::*;
    let all = vec![
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
    ];
    for fail in &all {
        match fail {
            Cancelled | TooLarge | DiskFull | Permission | Unreadable | SameFile
            | DocumentChanged | Busy | WriteFailed | SourceChanged | BrokenLink => {}
        }
    }
    all
}

/// The text the window shows for a refusal (the session shows `messages::save::reason`).
fn shown(fail: SaveFail) -> String {
    let text = messages::save::reason(fail.as_str())
        .unwrap_or_else(|| panic!("no save reason text for {}", fail.as_str()));
    assert!(!text.trim().is_empty(), "empty text for {}", fail.as_str());
    text
}

/// `/[A-Z]{2,}[_-][A-Z]|E[A-Z]{3,}\b/`: something that looks like an error code.
fn has_error_code(text: &str) -> bool {
    let c: Vec<char> = text.chars().collect();
    let upper = |i: usize| c.get(i).is_some_and(char::is_ascii_uppercase);
    let word = |i: usize| {
        c.get(i)
            .is_some_and(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
    };
    for i in 0..c.len() {
        // A run of two or more capitals, then `_` or `-`, then a capital.
        let mut j = i;
        while upper(j) {
            j += 1;
        }
        if j - i >= 2 && matches!(c.get(j), Some('_' | '-')) && upper(j + 1) {
            return true;
        }
        // E and three or more capitals ending a word.
        if c[i] == 'E' {
            let mut k = i + 1;
            while upper(k) {
                k += 1;
            }
            if k - (i + 1) >= 3 && !word(k) {
                return true;
            }
        }
    }
    false
}

fn has_word(text: &str, word: &str) -> bool {
    text.split(|ch: char| !ch.is_ascii_alphanumeric())
        .any(|w| w == word)
}

// ------------------------------------------------------------------ p1-save: names and errors

// was tests/p1-save.test.mjs: 'classifyFsError maps ENOSPC/EACCES and the rest to write-failed'
#[test]
fn classify_maps_disk_full_and_permission_and_the_rest_to_write_failed() {
    use io::ErrorKind;
    assert_eq!(
        safe_write::classify(&io::Error::from(ErrorKind::StorageFull)),
        SaveFail::DiskFull
    );
    assert_eq!(
        safe_write::classify(&io::Error::from(ErrorKind::PermissionDenied)),
        SaveFail::Permission
    );
    assert_eq!(
        safe_write::classify(&io::Error::from(ErrorKind::ReadOnlyFilesystem)),
        SaveFail::Permission
    );
    assert_eq!(
        safe_write::classify(&io::Error::other("EIO")),
        SaveFail::WriteFailed
    );
    #[cfg(unix)]
    {
        assert_eq!(safe_write::classify(&os_error(ENOSPC)), SaveFail::DiskFull);
        assert_eq!(
            safe_write::classify(&os_error(EACCES)),
            SaveFail::Permission
        );
        assert_eq!(safe_write::classify(&os_error(EIO)), SaveFail::WriteFailed);
    }
}

// ------------------------------------------------------------------ p1-save: Save a Copy

// was tests/p1-save.test.mjs: 'success replaces a preexisting copy and leaves the source'
#[test]
fn success_replaces_a_preexisting_copy_and_leaves_the_source() {
    let s = scratch("replace", BYTES);
    fs::write(&s.dest, b"old-copy-bytes").unwrap();
    let before = FileId::of(&s.source_path).unwrap();
    let saved = save_copy(Some(&s.dest), BYTES, &s.source, None, false, &none(), &yes).unwrap();
    assert_eq!(saved.name, "out.pdf");
    assert_eq!(saved.warning, None);
    assert_eq!(fs::read(&s.dest).unwrap(), BYTES);
    assert_eq!(fs::read(&s.source_path).unwrap(), BYTES);
    let after = FileId::of(&s.source_path).unwrap();
    assert_eq!(after, before, "the source is not touched");
    #[cfg(unix)]
    assert_eq!(mode(&s.dest), new_file_mode(s.dir.path()));
    assert_eq!(temps(s.dir.path()), Vec::<String>::new());
}

// was tests/p1-save.test.mjs: 'cancel writes nothing'
#[test]
fn cancel_writes_nothing() {
    let s = scratch("cancel", BYTES);
    assert_eq!(
        save_copy(None, BYTES, &s.source, None, false, &none(), &yes),
        Err(SaveFail::Cancelled)
    );
    assert!(!s.dest.exists());
    assert_eq!(temps(s.dir.path()), Vec::<String>::new());
}

// was tests/p1-save.test.mjs: 'inject failures clean temp and leave dest/source'
// The Rust `Inject` has no rename hook: the copy's commit failure is a real one (the target is a
// non-empty folder, so the rename fails); the in-place commit failure is not reachable (P-blocked).
#[test]
fn inject_failures_clean_temp_and_leave_dest_and_source() {
    type Case = (&'static str, fn() -> Inject, SaveFail, bool);
    let cases: [Case; 3] = [
        (
            "ENOSPC",
            || Inject {
                write_all: Some(Box::new(|_, _| Err(os_error(ENOSPC)))),
                ..Inject::default()
            },
            SaveFail::DiskFull,
            true,
        ),
        (
            "EACCES",
            || Inject {
                write_all: Some(Box::new(|_, _| Err(os_error(EACCES)))),
                ..Inject::default()
            },
            SaveFail::Permission,
            false,
        ),
        (
            "reopen hash mismatch",
            || Inject {
                reopen: Some(Box::new(|_| Ok(vec![7; BYTES.len()]))),
                ..Inject::default()
            },
            SaveFail::WriteFailed,
            true,
        ),
    ];
    for in_place in [false, true] {
        for (label, inject, reason, dest_preexists) in cases {
            let tag = format!("{} {label}", if in_place { "in-place" } else { "copy" });
            let s = scratch("inject", BYTES);
            if !in_place && dest_preexists {
                fs::write(&s.dest, b"keep-me-please").unwrap();
            }
            let result = if in_place {
                save_in_place(BYTES, &s.source, None, &inject(), &yes)
            } else {
                save_copy(
                    Some(&s.dest),
                    BYTES,
                    &s.source,
                    None,
                    false,
                    &inject(),
                    &yes,
                )
            };
            assert_eq!(result, Err(reason), "{tag}");
            if !in_place {
                if dest_preexists {
                    assert_eq!(fs::read(&s.dest).unwrap(), b"keep-me-please", "{tag}");
                } else {
                    assert!(!s.dest.exists(), "{tag}");
                }
            }
            assert_eq!(fs::read(&s.source_path).unwrap(), BYTES, "{tag}");
            assert_eq!(temps(s.dir.path()), Vec::<String>::new(), "{tag}");
        }
    }
    // 'commit rename failure' (copy): the rename onto a folder fails after the temp was written.
    let s = scratch("inject-rename", BYTES);
    fs::create_dir(&s.dest).unwrap();
    fs::write(s.dest.join("keep"), b"keep-me-please").unwrap();
    assert_eq!(
        save_copy(Some(&s.dest), BYTES, &s.source, None, false, &none(), &yes),
        Err(SaveFail::WriteFailed)
    );
    assert_eq!(fs::read(s.dest.join("keep")).unwrap(), b"keep-me-please");
    assert_eq!(fs::read(&s.source_path).unwrap(), BYTES);
    assert_eq!(temps(s.dir.path()), Vec::<String>::new());
}

// was tests/p1-save.test.mjs: 'directory fsync failure is success with durability-uncertain'
#[test]
fn directory_fsync_failure_is_success_with_durability_uncertain() {
    let s = scratch("dirsync", BYTES);
    let inject = Inject {
        dir_sync_fails: true,
        ..Inject::default()
    };
    let saved = save_copy(Some(&s.dest), BYTES, &s.source, None, false, &inject, &yes).unwrap();
    assert_eq!(saved.name, "out.pdf");
    assert_eq!(saved.warning, Some("durability-uncertain"));
    // The written file's identity (Save As rebinds to it) rides along.
    let file = saved.file.expect("written identity");
    assert_eq!(file.path, s.dest);
    assert_eq!((file.dev, file.ino), key(&s.dest));
    assert_eq!(fs::read(&s.dest).unwrap(), BYTES);
    assert_eq!(temps(s.dir.path()), Vec::<String>::new());
}

// was tests/p1-save.test.mjs: 'source change still writes the copy with a warning'
#[test]
fn source_change_still_writes_the_copy_with_a_warning() {
    let s = scratch("srcchange", BYTES);
    let changed = [BYTES, b"x"].concat();
    fs::write(&s.source_path, &changed).unwrap();
    let saved = save_copy(Some(&s.dest), BYTES, &s.source, None, false, &none(), &yes).unwrap();
    assert_eq!(saved.name, "out.pdf");
    assert_eq!(saved.warning, Some("source-changed"));
    let file = saved.file.expect("written identity");
    assert_eq!(file.path, s.dest);
    assert_eq!((file.dev, file.ino), key(&s.dest));
    assert_eq!(fs::read(&s.dest).unwrap(), BYTES);
    assert_eq!(fs::read(&s.source_path).unwrap(), changed);
}

// was tests/p1-save.test.mjs: 'same path, hardlink, and a symlink to the source are same-file; a
// symlink elsewhere is written through'
#[test]
fn same_path_hard_link_and_a_link_to_the_source_are_same_file() {
    let s = scratch("samefile", BYTES);
    assert_eq!(
        save_copy(
            Some(&s.source_path),
            BYTES,
            &s.source,
            None,
            false,
            &none(),
            &yes
        ),
        Err(SaveFail::SameFile)
    );
    let hard = s.dir.join("hard.pdf");
    fs::hard_link(&s.source_path, &hard).unwrap();
    assert_eq!(
        save_copy(
            Some(&hard),
            b"%PDF-other",
            &s.source,
            None,
            false,
            &none(),
            &yes
        ),
        Err(SaveFail::SameFile)
    );
    assert_eq!(fs::read(&hard).unwrap(), BYTES);
    #[cfg(unix)]
    {
        let other = s.dir.join("other.pdf");
        fs::write(&other, b"not-the-source").unwrap();
        let alias = s.dir.join("alias.pdf");
        symlink(&other, &alias);
        // A link to another file is written through, as in-place Save writes through its link.
        let through =
            save_copy(Some(&alias), BYTES, &s.source, None, false, &none(), &yes).unwrap();
        assert_eq!(through.name, "alias.pdf");
        assert_eq!(fs::read(&other).unwrap(), BYTES);
        assert!(
            fs::symlink_metadata(&alias)
                .unwrap()
                .file_type()
                .is_symlink()
        );

        let self_link = s.dir.join("self.pdf");
        symlink(&s.source_path, &self_link);
        assert_eq!(
            save_copy(
                Some(&self_link),
                b"%PDF-other",
                &s.source,
                None,
                false,
                &none(),
                &yes
            ),
            Err(SaveFail::SameFile)
        );
        assert_eq!(fs::read(&s.source_path).unwrap(), BYTES);
    }
}

// was tests/p1-save.test.mjs: 'Open B during write aborts before rename and leaves dest'
#[test]
fn opening_another_document_during_the_write_aborts_before_rename() {
    let s = scratch("openb", BYTES);
    fs::write(&s.dest, b"B-source-bytes").unwrap();
    let current = Arc::new(AtomicBool::new(true));
    let flip = current.clone();
    let inject = Inject {
        write_all: Some(Box::new(move |file, data| {
            file.write_all(data)?;
            flip.store(false, Ordering::SeqCst);
            Ok(())
        })),
        ..Inject::default()
    };
    let is_current = || current.load(Ordering::SeqCst);
    assert_eq!(
        save_copy(
            Some(&s.dest),
            BYTES,
            &s.source,
            None,
            false,
            &inject,
            &is_current
        ),
        Err(SaveFail::DocumentChanged)
    );
    assert_eq!(fs::read(&s.dest).unwrap(), b"B-source-bytes");
    assert_eq!(temps(s.dir.path()), Vec::<String>::new());
}

// ------------------------------------------------------------------ p1-save: ceilings

/// Hooks that skip the write and fail the read-back: a save that got past the size gate.
fn past_the_gate() -> Inject {
    Inject {
        write_all: Some(Box::new(|_, _| Ok(()))),
        reopen: Some(Box::new(|_| Err(os_error(EIO)))),
        ..Inject::default()
    }
}

// was tests/p1-save.test.mjs: 'save ceiling is 50 MiB; open stays 120 MiB'
#[test]
fn save_ceiling_is_50_mib_and_open_stays_120_mib() {
    use tandem_protocol::limits;
    assert_eq!(MAX_SAVE_BYTES, 50 * 1024 * 1024);
    assert_eq!(limits::MAX_SAVE_BYTES, MAX_SAVE_BYTES);
    assert_eq!(host::MAX_PDF_BYTES, 120 * 1024 * 1024);
    assert_eq!(limits::MAX_PDF_BYTES as u64, host::MAX_PDF_BYTES);

    // Saving: exactly the ceiling passes the size gate, one byte more is too large.
    let s = scratch("ceiling", BYTES);
    let at = vec![0u8; MAX_SAVE_BYTES];
    let over = vec![0u8; MAX_SAVE_BYTES + 1];
    let dest = s.dir.join("big.pdf");
    assert_eq!(
        write_new(Some(&dest), &at, None, &past_the_gate()),
        Err(SaveFail::WriteFailed)
    );
    assert_eq!(
        save_copy(
            Some(&dest),
            &at,
            &s.source,
            None,
            false,
            &past_the_gate(),
            &yes
        ),
        Err(SaveFail::WriteFailed)
    );
    assert_eq!(
        write_new(Some(&dest), &over, None, &none()),
        Err(SaveFail::TooLarge)
    );
    assert_eq!(
        save_copy(Some(&dest), &over, &s.source, None, false, &none(), &yes),
        Err(SaveFail::TooLarge)
    );
    assert_eq!(
        save_in_place(&over, &s.source, None, &none(), &yes),
        Err(SaveFail::TooLarge)
    );
    drop((at, over));
    assert!(!dest.exists());
    assert_eq!(fs::read(&s.source_path).unwrap(), BYTES);
    assert_eq!(temps(s.dir.path()), Vec::<String>::new());

    // Opening: a (sparse) file of exactly the ceiling may open; one byte more is too large.
    let sparse = s.dir.join("huge.pdf");
    File::create(&sparse)
        .unwrap()
        .set_len(host::MAX_PDF_BYTES)
        .unwrap();
    assert_eq!(Host::inspect_open(&sparse.to_string_lossy()), None);
    File::options()
        .write(true)
        .open(&sparse)
        .unwrap()
        .set_len(host::MAX_PDF_BYTES + 1)
        .unwrap();
    let too_large = host::OpenError::too_large("huge.pdf".into());
    assert_eq!(
        Host::inspect_open(&sparse.to_string_lossy()),
        Some(too_large.clone())
    );
    assert_eq!(too_large.limit, Some(host::MAX_PDF_BYTES));
    let mut host = smoke_host(&s.dir);
    host.deliver_open(Some(&sparse), false);
    assert_eq!(host.take_events(), vec![Event::OpenError(too_large)]);
}

// was tests/p1-save.test.mjs: 'relative dest is unreadable; too-large uses the save ceiling'
#[test]
fn relative_dest_is_unreadable_and_too_large_uses_the_save_ceiling() {
    let s = scratch("relative", BYTES);
    assert_eq!(
        save_copy(
            Some(Path::new("out.pdf")),
            BYTES,
            &s.source,
            None,
            false,
            &none(),
            &yes
        ),
        Err(SaveFail::Unreadable)
    );
    let huge = vec![0u8; MAX_SAVE_BYTES + 1];
    assert_eq!(
        save_copy(
            Some(&s.dir.join("x.pdf")),
            &huge,
            &s.source,
            None,
            false,
            &none(),
            &yes
        ),
        Err(SaveFail::TooLarge)
    );
    assert_eq!(names_in(s.dir.path()), vec!["src.pdf"]);
}

// was tests/p1-save.test.mjs: 'writeNew refuses a relative dest and oversize, writes 0644-ish on success'
#[test]
fn write_new_refuses_a_relative_dest_and_oversize_and_writes_the_umask_mode() {
    let dir = Scratch::new("writenew");
    assert_eq!(
        write_new(Some(Path::new("out.pdf")), BYTES, None, &none()),
        Err(SaveFail::Unreadable)
    );
    assert_eq!(
        write_new(
            Some(&dir.join("x.pdf")),
            &vec![0u8; MAX_SAVE_BYTES + 1],
            None,
            &none()
        ),
        Err(SaveFail::TooLarge)
    );
    let dest = dir.join("new.pdf");
    let written = write_new(Some(&dest), BYTES, None, &none()).unwrap();
    assert_eq!(written.name, "new.pdf");
    assert_eq!(fs::read(&dest).unwrap(), BYTES);
    #[cfg(unix)]
    assert_eq!(mode(&dest), new_file_mode(dir.path()));
    let inject = Inject {
        write_all: Some(Box::new(|_, _| Err(os_error(ENOSPC)))),
        ..Inject::default()
    };
    assert_eq!(
        write_new(Some(&dir.join("fail.pdf")), BYTES, None, &inject),
        Err(SaveFail::DiskFull)
    );
    assert!(!dir.join("fail.pdf").exists());
    assert_eq!(temps(dir.path()), Vec::<String>::new());
}

// ------------------------------------------------------------------ p1-save: the journal

/// `^\.[^/]+\.[A-Za-z0-9_-]+\.tmp$`
fn temp_name_shape(name: &str) -> bool {
    let Some(stem) = name.strip_prefix('.').and_then(|r| r.strip_suffix(".tmp")) else {
        return false;
    };
    let Some((head, tail)) = stem.rsplit_once('.') else {
        return false;
    };
    !head.is_empty()
        && !head.contains('/')
        && !tail.is_empty()
        && tail
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

// was tests/p1-save.test.mjs: 'journal entry is present before rename; success leaves empty 0600 journal'
// Read inside the read-back hook (after the write, before the rename; `Inject` has no rename hook).
#[test]
fn the_journal_names_the_temp_before_the_rename_and_is_empty_after() {
    let s = scratch("journal", BYTES);
    let journal_path = s.dir.join("pending-saves.json");
    let during: Arc<Mutex<Option<Vec<Value>>>> = Arc::default();
    let (seen, read_from) = (during.clone(), journal_path.clone());
    let inject = Inject {
        reopen: Some(Box::new(move |_| {
            *seen.lock().unwrap() = Some(journal(&read_from));
            Err(os_error(EIO))
        })),
        ..Inject::default()
    };
    assert_eq!(
        save_copy(
            Some(&s.dest),
            BYTES,
            &s.source,
            Some(&journal_path),
            false,
            &inject,
            &yes
        ),
        Err(SaveFail::WriteFailed)
    );
    let during = during.lock().unwrap().take().expect("the hook ran");
    assert_eq!(during.len(), 1);
    let temp = PathBuf::from(during[0]["temp"].as_str().unwrap());
    let dest = PathBuf::from(during[0]["dest"].as_str().unwrap());
    assert_eq!(dest.file_name().unwrap(), "out.pdf");
    assert_eq!(temp.parent(), dest.parent());
    assert!(
        temp_name_shape(&temp.file_name().unwrap().to_string_lossy()),
        "{temp:?}"
    );
    assert!(during[0]["started"].is_number());
    assert_eq!(temps(s.dir.path()), Vec::<String>::new());
    assert_eq!(journal(&journal_path), Vec::<Value>::new());

    save_copy(
        Some(&s.dest),
        BYTES,
        &s.source,
        Some(&journal_path),
        false,
        &none(),
        &yes,
    )
    .unwrap();
    assert_eq!(journal(&journal_path), Vec::<Value>::new());
    #[cfg(unix)]
    assert_eq!(mode(&journal_path), 0o600);
    assert_eq!(temps(s.dir.path()), Vec::<String>::new());
}

// was tests/p1-save.test.mjs: 'cleanupOrphans unlinks a matching temp, ignores the rest, treats corrupt as empty'
#[test]
fn cleanup_orphans_removes_a_matching_temp_keeps_the_rest_and_reads_corrupt_as_empty() {
    let dir = Scratch::new("orphan");
    let outside_dir = Scratch::new("orphan-out");
    let dest = dir.join("out.pdf");
    let journal_path = dir.join("pending-saves.json");
    let orphan = dir.join(".out.pdf.abc123.tmp");
    fs::write(&orphan, b"leftover").unwrap();
    let target = dir.join("real.pdf");
    fs::write(&target, b"keep-link-target").unwrap();
    let linked = dir.join(".out.pdf.symlink.tmp");
    #[cfg(unix)]
    symlink(&target, &linked);
    let nested = dir.join(".out.pdf.dir.tmp");
    fs::create_dir(&nested).unwrap();
    let other = dir.join("not-a-temp.pdf");
    fs::write(&other, b"keep-other").unwrap();
    let outside = outside_dir.join(".out.pdf.abc456.tmp");
    fs::write(&outside, b"keep-outside").unwrap();
    let missing = dir.join(".gone.pdf.abc.tmp");
    let entry = |temp: &Path, started: u32| serde_json::json!({"temp": temp.to_string_lossy(), "dest": dest.to_string_lossy(), "started": started});
    let mut entries = vec![entry(&orphan, 1)];
    if cfg!(unix) {
        entries.push(entry(&linked, 2));
    }
    entries.extend([
        entry(&nested, 3),
        entry(&other, 4),
        entry(&outside, 5),
        entry(&missing, 6),
    ]);
    fs::write(&journal_path, Value::Array(entries).to_string()).unwrap();

    cleanup_orphans(&journal_path);

    assert!(!orphan.exists());
    assert_eq!(fs::read(&target).unwrap(), b"keep-link-target");
    #[cfg(unix)]
    assert!(
        fs::symlink_metadata(&linked)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(nested.is_dir());
    assert_eq!(fs::read(&other).unwrap(), b"keep-other");
    assert_eq!(fs::read(&outside).unwrap(), b"keep-outside");
    let mut remain: Vec<String> = journal(&journal_path)
        .iter()
        .map(|e| e["temp"].as_str().unwrap().to_string())
        .collect();
    remain.sort();
    let mut expected: Vec<String> = [&nested, &other, &outside]
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    if cfg!(unix) {
        expected.push(linked.to_string_lossy().into_owned());
    }
    expected.sort();
    assert_eq!(remain, expected);

    fs::write(&journal_path, b"{not json").unwrap();
    cleanup_orphans(&journal_path);
    assert_eq!(journal(&journal_path), Vec::<Value>::new());
}

// was tests/p1-save.test.mjs: 'saveInPlace source-changed refuses before rename'
#[test]
fn save_in_place_refuses_a_changed_source_and_keeps_it() {
    let s = scratch("inplace-changed", BYTES);
    let changed = [BYTES, b"x"].concat();
    fs::write(&s.source_path, &changed).unwrap();
    assert_eq!(
        save_in_place(BYTES, &s.source, None, &none(), &yes),
        Err(SaveFail::SourceChanged)
    );
    assert_eq!(fs::read(&s.source_path).unwrap(), changed);
    assert_eq!(temps(s.dir.path()), Vec::<String>::new());
}

// was tests/p1-save.test.mjs: 'debt1 row 25: the journal is written to a .tmp sibling and
// renamed; a failed write keeps the old bytes'. `Inject` has no journal-write hook, so the failed
// journal write is a real one: the journal's folder is read-only (the reason is then permission,
// where the JS injected ENOSPC).
#[test]
fn the_journal_is_replaced_by_rename_and_a_failed_journal_write_keeps_it() {
    let s = scratch("journal-swap", BYTES);
    let jdir = Scratch::new("journal-swap-j");
    let journal_path = jdir.join("pending-saves.json");
    let old_temp = jdir.join(".old.aaaa.tmp").to_string_lossy().into_owned();
    let old_dest = jdir.join("old.pdf").to_string_lossy().into_owned();
    let previous = serde_json::json!([{ "temp": old_temp, "dest": old_dest, "started": 1 }]);
    fs::write(&journal_path, previous.to_string()).unwrap();
    fsx::set_mode(&journal_path, 0o600).unwrap();
    let same_as_previous = |entries: Vec<Value>| {
        entries.len() == 1
            && entries[0]["temp"] == old_temp.as_str()
            && entries[0]["dest"] == old_dest.as_str()
            && entries[0]["started"].as_f64() == Some(1.0)
    };

    save_copy(
        Some(&s.dest),
        BYTES,
        &s.source,
        Some(&journal_path),
        false,
        &none(),
        &yes,
    )
    .unwrap();
    assert!(
        same_as_previous(journal(&journal_path)),
        "journal add + drop leave the previous entry"
    );
    assert!(!jdir.join("pending-saves.json.tmp").exists());
    #[cfg(unix)]
    assert_eq!(mode(&journal_path), 0o600);

    #[cfg(unix)]
    {
        need_perms!();
        fs::remove_file(&s.dest).unwrap();
        fsx::set_mode(jdir.path(), 0o500).unwrap();
        let failed = save_copy(
            Some(&s.dest),
            BYTES,
            &s.source,
            Some(&journal_path),
            false,
            &none(),
            &yes,
        );
        fsx::set_mode(jdir.path(), 0o700).unwrap();
        assert_eq!(failed, Err(SaveFail::Permission));
        assert!(
            same_as_previous(journal(&journal_path)),
            "a journal write that fails leaves the previous journal intact"
        );
        assert!(!jdir.join("pending-saves.json.tmp").exists());
        assert!(
            !s.dest.exists(),
            "nothing is written without its journal entry"
        );
        assert_eq!(temps(s.dir.path()), Vec::<String>::new());
    }
}

// ------------------------------------------------------------------ m10: the reason texts

// was tests/m10-save-failure.test.mjs: 'every SaveFail reason has plain, non-empty text, and the
// three SaveResult copies agree'. Rust has one SaveFail enum (no copies to agree); the session
// shows `messages::save::reason(fail.as_str())`.
#[test]
fn every_save_failure_has_one_plain_sentence() {
    let all = all_fails();
    assert!(all.len() >= 10);
    let mut keys: Vec<&str> = all.iter().map(|f| f.as_str()).collect();
    keys.sort();
    keys.dedup();
    assert_eq!(keys.len(), all.len(), "the reason names are distinct");
    for fail in all {
        let reason = fail.as_str();
        let text = shown(fail);
        assert!(
            text.starts_with(|c: char| c.is_ascii_uppercase()) && text.ends_with('.'),
            "{reason}: one plain sentence or more: {text}"
        );
        assert!(
            !text.contains(['<', '>', '{', '}', '`', '$']),
            "{reason}: no markup or template left: {text}"
        );
        for word in ["undefined", "NaN", "null"] {
            assert!(!has_word(&text, word), "{reason}: {text}");
        }
        assert!(!has_error_code(&text), "{reason}: no error codes: {text}");
    }
    // In-place Save and Save As share the texts: no write failure may speak of "the copy" only.
    for fail in [
        SaveFail::DiskFull,
        SaveFail::Permission,
        SaveFail::WriteFailed,
        SaveFail::Unreadable,
    ] {
        assert!(!has_word(&shown(fail), "copy"), "{}", fail.as_str());
    }
    assert!(shown(SaveFail::Permission).contains("read-only"));
    assert!(shown(SaveFail::BrokenLink).contains("link"));
    // The checker itself.
    assert!(has_error_code("failed: ENOSPC.") && has_error_code("ERR_FS-X"));
    assert!(!has_error_code("Tandem PDF can save at most 50 MB."));
}

// ------------------------------------------------------------------ m10: read-only targets

// was tests/m10-save-failure.test.mjs: 'in-place Save over a read-only (0444) file is refused and leaves it intact'
#[cfg(unix)]
#[test]
fn in_place_save_over_a_read_only_file_is_refused() {
    need_perms!();
    let s = scratch("m10-ro", ORIGINAL);
    let jdir = Scratch::new("m10-ro-j");
    let journal_path = jdir.join("pending-saves.json");
    fsx::set_mode(&s.source_path, 0o444).unwrap();
    let source = FileId::of(&s.source_path).unwrap();
    assert_eq!(
        save_in_place(EDITED, &source, Some(&journal_path), &none(), &yes),
        Err(SaveFail::Permission)
    );
    assert_eq!(fs::read(&s.source_path).unwrap(), ORIGINAL);
    assert_eq!(mode(&s.source_path), 0o444);
    assert_eq!(temps(s.dir.path()), Vec::<String>::new());
    assert_eq!(journal(&journal_path), Vec::<Value>::new());
}

// was tests/m10-save-failure.test.mjs: 'Save As over an existing read-only file is refused and leaves both files intact'
#[cfg(unix)]
#[test]
fn save_as_over_a_read_only_file_is_refused() {
    need_perms!();
    let s = scratch("m10-ro-dest", ORIGINAL);
    let jdir = Scratch::new("m10-ro-dest-j");
    let journal_path = jdir.join("pending-saves.json");
    let dest = s.dir.join("other.pdf");
    fs::write(&dest, b"%PDF-someone-else").unwrap();
    fsx::set_mode(&dest, 0o444).unwrap();
    assert_eq!(
        save_copy(
            Some(&dest),
            EDITED,
            &s.source,
            Some(&journal_path),
            false,
            &none(),
            &yes
        ),
        Err(SaveFail::Permission)
    );
    assert_eq!(fs::read(&dest).unwrap(), b"%PDF-someone-else");
    assert_eq!(fs::read(&s.source_path).unwrap(), ORIGINAL);
    assert_eq!(temps(s.dir.path()), Vec::<String>::new());
    assert_eq!(journal(&journal_path), Vec::<Value>::new());
}

// was tests/m10-save-failure.test.mjs: 'Save As over an existing file keeps its permission bits;
// a new file gets 0666 minus the umask'
#[cfg(unix)]
#[test]
fn save_as_keeps_an_existing_files_mode_and_a_new_file_gets_the_umask_mode() {
    let s = scratch("m10-mode", ORIGINAL);
    let jdir = Scratch::new("m10-mode-j");
    let journal_path = jdir.join("pending-saves.json");
    let dest = s.dir.join("private.pdf");
    fs::write(&dest, b"%PDF-private").unwrap();
    fsx::set_mode(&dest, 0o600).unwrap();
    save_copy(
        Some(&dest),
        EDITED,
        &s.source,
        Some(&journal_path),
        false,
        &none(),
        &yes,
    )
    .unwrap();
    assert_eq!(fs::read(&dest).unwrap(), EDITED);
    assert_eq!(mode(&dest), 0o600);
    let fresh = s.dir.join("fresh.pdf");
    save_copy(
        Some(&fresh),
        EDITED,
        &s.source,
        Some(&journal_path),
        false,
        &none(),
        &yes,
    )
    .unwrap();
    assert_eq!(mode(&fresh), new_file_mode(s.dir.path()));
}

// was tests/m10-save-failure.test.mjs: 'a writable file in a read-only folder: in-place Save and
// Save As into it are refused'
#[cfg(unix)]
#[test]
fn a_read_only_folder_refuses_in_place_save_and_save_as_into_it() {
    need_perms!();
    let s = scratch("m10-ro-dir", ORIGINAL);
    let other = scratch("m10-ro-dir-other", ORIGINAL);
    let jdir = Scratch::new("m10-ro-dir-j");
    let journal_path = jdir.join("pending-saves.json");
    fsx::set_mode(s.dir.path(), 0o555).unwrap();
    let in_place = save_in_place(EDITED, &s.source, Some(&journal_path), &none(), &yes);
    let copy = save_copy(
        Some(&s.dir.join("new.pdf")),
        EDITED,
        &other.source,
        Some(&journal_path),
        false,
        &none(),
        &yes,
    );
    let names = names_in(s.dir.path());
    fsx::set_mode(s.dir.path(), 0o755).unwrap();
    assert_eq!(in_place, Err(SaveFail::Permission));
    assert_eq!(copy, Err(SaveFail::Permission));
    assert_eq!(fs::read(&s.source_path).unwrap(), ORIGINAL);
    assert_eq!(names, vec!["src.pdf"]);
    assert_eq!(journal(&journal_path), Vec::<Value>::new());
}

// was tests/m10-save-failure.test.mjs: 'the target made read-only while the bytes are written is still refused'
#[cfg(unix)]
#[test]
fn a_target_made_read_only_during_the_write_is_refused() {
    need_perms!();
    let s = scratch("m10-ro-mid", ORIGINAL);
    let dest = s.dir.join("other.pdf");
    fs::write(&dest, b"%PDF-someone-else").unwrap();
    let lock = dest.clone();
    let inject = Inject {
        write_all: Some(Box::new(move |file, data| {
            fsx::set_mode(&lock, 0o444)?;
            file.write_all(data)
        })),
        ..Inject::default()
    };
    assert_eq!(
        save_copy(Some(&dest), EDITED, &s.source, None, false, &inject, &yes),
        Err(SaveFail::Permission)
    );
    assert_eq!(fs::read(&dest).unwrap(), b"%PDF-someone-else");
    assert_eq!(fs::read(&s.source_path).unwrap(), ORIGINAL);
    assert_eq!(temps(s.dir.path()), Vec::<String>::new());
}

// ------------------------------------------------------------------ m10: the target changes during the write

// was tests/m10-save-failure.test.mjs: 'in-place Save: the file deleted during the write is refused and not recreated'
#[test]
fn in_place_save_of_a_file_deleted_during_the_write_is_refused_and_not_recreated() {
    let s = scratch("m10-deleted", ORIGINAL);
    let jdir = Scratch::new("m10-deleted-j");
    let journal_path = jdir.join("pending-saves.json");
    let gone = s.source_path.clone();
    let inject = Inject {
        write_all: Some(Box::new(move |file, data| {
            fs::remove_file(&gone)?;
            file.write_all(data)
        })),
        ..Inject::default()
    };
    let result = save_in_place(EDITED, &s.source, Some(&journal_path), &inject, &yes);
    assert_eq!(result, Err(SaveFail::SourceChanged));
    shown(SaveFail::SourceChanged);
    assert!(!s.source_path.exists());
    assert_eq!(temps(s.dir.path()), Vec::<String>::new());
    assert_eq!(journal(&journal_path), Vec::<Value>::new());
}

// was tests/m10-save-failure.test.mjs: 'in-place Save: the file replaced by another during the
// write is refused; the other file is kept'
#[test]
fn in_place_save_of_a_file_replaced_during_the_write_keeps_the_other_file() {
    let s = scratch("m10-replaced", ORIGINAL);
    let other: &[u8] = b"%PDF-replacement-from-another-app";
    let (incoming, target) = (s.dir.join("incoming"), s.source_path.clone());
    let inject = Inject {
        write_all: Some(Box::new(move |file, data| {
            fs::write(&incoming, other)?;
            fs::rename(&incoming, &target)?;
            file.write_all(data)
        })),
        ..Inject::default()
    };
    assert_eq!(
        save_in_place(EDITED, &s.source, None, &inject, &yes),
        Err(SaveFail::SourceChanged)
    );
    assert_eq!(fs::read(&s.source_path).unwrap(), other);
    assert_eq!(temps(s.dir.path()), Vec::<String>::new());
}

// was tests/m10-save-failure.test.mjs: 'Save As: the target replaced by a hard link to the open
// file during the write is refused'
#[test]
fn save_as_target_replaced_by_a_hard_link_to_the_open_file_during_the_write_is_refused() {
    let s = scratch("m10-hardlink", ORIGINAL);
    let dest = s.dir.join("copy.pdf");
    let (from, to) = (s.source_path.clone(), dest.clone());
    let inject = Inject {
        write_all: Some(Box::new(move |file, data| {
            fs::hard_link(&from, &to)?;
            file.write_all(data)
        })),
        ..Inject::default()
    };
    let result = save_copy(Some(&dest), EDITED, &s.source, None, false, &inject, &yes);
    assert_eq!(result, Err(SaveFail::SameFile));
    shown(SaveFail::SameFile);
    assert_eq!(fs::read(&s.source_path).unwrap(), ORIGINAL);
    assert_eq!(temps(s.dir.path()), Vec::<String>::new());
}

// was tests/m10-save-failure.test.mjs: 'Save As: the old target deleted during the write still
// gets the new copy; the open file is untouched'
#[test]
fn save_as_old_target_deleted_during_the_write_still_gets_the_copy() {
    let s = scratch("m10-olddest", ORIGINAL);
    let dest = s.dir.join("copy.pdf");
    fs::write(&dest, b"%PDF-old-copy").unwrap();
    let gone = dest.clone();
    let inject = Inject {
        write_all: Some(Box::new(move |file, data| {
            fs::remove_file(&gone)?;
            file.write_all(data)
        })),
        ..Inject::default()
    };
    save_copy(Some(&dest), EDITED, &s.source, None, false, &inject, &yes).unwrap();
    assert_eq!(fs::read(&dest).unwrap(), EDITED);
    assert_eq!(fs::read(&s.source_path).unwrap(), ORIGINAL);
    assert_eq!(temps(s.dir.path()), Vec::<String>::new());
}

// ------------------------------------------------------------------ m10: symbolic links

// was tests/m10-save-failure.test.mjs: 'in-place Save through a symlink writes the real file and keeps the link'
#[cfg(unix)]
#[test]
fn in_place_save_through_a_link_writes_the_real_file_and_keeps_the_link() {
    let s = scratch("m10-link", ORIGINAL);
    let link_dir = Scratch::new("m10-link-l");
    let jdir = Scratch::new("m10-link-j");
    let journal_path = jdir.join("pending-saves.json");
    let alias = link_dir.join("alias.pdf");
    symlink(&s.source_path, &alias);
    let source = FileId::of(&alias).unwrap();
    let saved = save_in_place(EDITED, &source, Some(&journal_path), &none(), &yes).unwrap();
    assert_eq!(saved.name, "alias.pdf");
    let file = saved.file.expect("written identity");
    assert_eq!(file.path, alias);
    assert_eq!((file.dev, file.ino), key(&s.source_path));
    assert!(
        fs::symlink_metadata(&alias)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read_link(&alias).unwrap(), s.source_path);
    assert_eq!(fs::read(&s.source_path).unwrap(), EDITED);
    assert_eq!(temps(s.dir.path()), Vec::<String>::new());
    assert_eq!(temps(link_dir.path()), Vec::<String>::new());
    assert_eq!(journal(&journal_path), Vec::<Value>::new());
    // A second Save through the link sees the file it wrote as unchanged.
    save_in_place(ORIGINAL, &file, Some(&journal_path), &none(), &yes).unwrap();
    assert_eq!(fs::read(&s.source_path).unwrap(), ORIGINAL);
    assert!(
        fs::symlink_metadata(&alias)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

// was tests/m10-save-failure.test.mjs: 'in-place Save through a symlink to a read-only file is refused'
#[cfg(unix)]
#[test]
fn in_place_save_through_a_link_to_a_read_only_file_is_refused() {
    need_perms!();
    let s = scratch("m10-link-ro", ORIGINAL);
    let link_dir = Scratch::new("m10-link-ro-l");
    let alias = link_dir.join("alias.pdf");
    symlink(&s.source_path, &alias);
    fsx::set_mode(&s.source_path, 0o444).unwrap();
    let source = FileId::of(&alias).unwrap();
    assert_eq!(
        save_in_place(EDITED, &source, None, &none(), &yes),
        Err(SaveFail::Permission)
    );
    assert_eq!(fs::read(&s.source_path).unwrap(), ORIGINAL);
    assert!(
        fs::symlink_metadata(&alias)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(temps(s.dir.path()), Vec::<String>::new());
}

// was tests/m10-save-failure.test.mjs: 'Save As onto a symlink writes through to its file; a link
// to the open file or to nothing is refused'
#[cfg(unix)]
#[test]
fn save_as_onto_a_link_writes_through_and_a_link_to_the_open_file_or_nothing_is_refused() {
    let s = scratch("m10-saveas-link", ORIGINAL);
    let other_dir = Scratch::new("m10-saveas-link-o");
    let jdir = Scratch::new("m10-saveas-link-j");
    let journal_path = jdir.join("pending-saves.json");
    let other = other_dir.join("other.pdf");
    fs::write(&other, b"%PDF-someone-else").unwrap();
    let alias = s.dir.join("alias.pdf");
    symlink(&other, &alias);
    let saved = save_copy(
        Some(&alias),
        EDITED,
        &s.source,
        Some(&journal_path),
        false,
        &none(),
        &yes,
    )
    .unwrap();
    assert_eq!(saved.name, "alias.pdf");
    assert!(
        fs::symlink_metadata(&alias)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read(&other).unwrap(),
        EDITED,
        "the file it names gets the copy"
    );
    assert_eq!(temps(s.dir.path()), Vec::<String>::new());
    assert_eq!(temps(other_dir.path()), Vec::<String>::new());

    let to_open = s.dir.join("to-open.pdf");
    symlink(&s.source_path, &to_open);
    assert_eq!(
        save_copy(
            Some(&to_open),
            EDITED,
            &s.source,
            Some(&journal_path),
            false,
            &none(),
            &yes
        ),
        Err(SaveFail::SameFile)
    );

    let dangling = s.dir.join("dangling.pdf");
    let gone = s.dir.join("gone.pdf");
    symlink(&gone, &dangling);
    let broken = save_copy(
        Some(&dangling),
        EDITED,
        &s.source,
        Some(&journal_path),
        false,
        &none(),
        &yes,
    );
    assert_eq!(broken, Err(SaveFail::BrokenLink));
    assert!(shown(SaveFail::BrokenLink).contains("link"));
    assert!(
        fs::symlink_metadata(&dangling)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(!gone.exists());
    assert_eq!(fs::read(&s.source_path).unwrap(), ORIGINAL);
    assert_eq!(temps(s.dir.path()), Vec::<String>::new());
    assert_eq!(journal(&journal_path), Vec::<Value>::new());
}

/// A link retargeted while the save runs: the save writes the file the link named when it began,
/// reports that file's identity, and the next Save refuses the file the link names now.
#[cfg(unix)]
fn link_retargeted_during_the_save(late: bool) {
    let dir = Scratch::new(if late {
        "m10-retarget-late"
    } else {
        "m10-retarget"
    });
    let jdir = Scratch::new("m10-retarget-j");
    let journal_path = jdir.join("pending-saves.json");
    let first = dir.join("t1.pdf");
    let second = dir.join("t2.pdf");
    fs::write(&first, ORIGINAL).unwrap();
    fs::write(&second, PRECIOUS).unwrap();
    let alias = dir.join("alias.pdf");
    symlink(&first, &alias);
    let (link, to) = (alias.clone(), second.clone());
    let inject = if late {
        // After the write and its fsync, just before the identity checks and the rename.
        Inject {
            reopen: Some(Box::new(move |temp| {
                retarget(&link, &to);
                fs::read(temp)
            })),
            ..Inject::default()
        }
    } else {
        Inject {
            write_all: Some(Box::new(move |file, data| {
                retarget(&link, &to);
                file.write_all(data)
            })),
            ..Inject::default()
        }
    };
    let source = FileId::of(&alias).unwrap();
    let saved = save_in_place(EDITED, &source, Some(&journal_path), &inject, &yes).unwrap();
    assert_eq!(fs::read(&first).unwrap(), EDITED);
    assert_eq!(fs::read(&second).unwrap(), PRECIOUS);
    let file = saved.file.expect("written identity");
    assert_eq!(file.path, alias);
    assert_eq!(
        (file.dev, file.ino),
        key(&first),
        "the identity of the file written"
    );
    assert_eq!(
        save_in_place(b"%PDF-next-save", &file, Some(&journal_path), &none(), &yes),
        Err(SaveFail::SourceChanged)
    );
    assert_eq!(fs::read(&second).unwrap(), PRECIOUS);
    assert_eq!(temps(dir.path()), Vec::<String>::new());
}

// was tests/m10-save-failure.test.mjs: 'a link retargeted inside the writeAll hook: the save
// writes the first file, the next Save refuses the second'
#[cfg(unix)]
#[test]
fn a_link_retargeted_during_the_write_saves_the_first_file_and_the_next_save_refuses() {
    link_retargeted_during_the_save(false);
}

// was tests/m10-save-failure.test.mjs: 'a link retargeted inside the rename hook: ...'. `Inject`
// has no rename hook; this retargets in the read-back hook, the last seam before the rename.
#[cfg(unix)]
#[test]
fn a_link_retargeted_just_before_the_rename_saves_the_first_file_and_the_next_save_refuses() {
    link_retargeted_during_the_save(true);
}

// ------------------------------------------------------------------ m10: a crash between write and rename

const CRASH_SOURCE: &str = "TANDEM_SW_TEST_CRASH_SOURCE";
const CRASH_JOURNAL: &str = "TANDEM_SW_TEST_CRASH_JOURNAL";

/// Not a test of its own: run as a child process by `crash_between_write_and_rename_*`, it starts
/// an in-place Save and SIGKILLs itself after the temp is written and fsynced, before the rename.
#[test]
fn crash_child_process() {
    let (Some(source), Some(journal_path)) = (
        std::env::var_os(CRASH_SOURCE),
        std::env::var_os(CRASH_JOURNAL),
    ) else {
        return;
    };
    let source = FileId::of(Path::new(&source)).expect("source");
    let inject = Inject {
        reopen: Some(Box::new(|_| {
            let pid = std::process::id().to_string();
            let _ = std::process::Command::new("kill")
                .args(["-KILL", &pid])
                .status();
            std::thread::sleep(std::time::Duration::from_secs(10));
            std::process::exit(3);
        })),
        ..Inject::default()
    };
    let _ = save_in_place(
        EDITED,
        &source,
        Some(Path::new(&journal_path)),
        &inject,
        &yes,
    );
    std::process::exit(3);
}

/// Runs [`crash_child_process`] in a child copy of this test binary; true if it was SIGKILLed.
#[cfg(unix)]
fn crash_in_rename(source: &Path, journal_path: &Path) -> bool {
    use std::os::unix::process::ExitStatusExt;
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "crash_child_process",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CRASH_SOURCE, source)
        .env(CRASH_JOURNAL, journal_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .unwrap();
    assert_eq!(
        out.status.signal(),
        Some(9),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    true
}

// was tests/m10-save-failure.test.mjs: 'a crash between write and rename leaves the original
// intact; cleanupOrphans removes the temp'. Also the Rust side of 'the app removes a crashed
// save's temp on its next start': `Host::new` runs `cleanup_orphans` on its journal.
#[cfg(unix)]
#[test]
fn crash_between_write_and_rename_leaves_the_original_and_the_next_start_removes_the_temp() {
    let s = scratch("m10-crash", ORIGINAL);
    let jdir = Scratch::new("m10-crash-j");
    let journal_path = jdir.join("pending-saves.json");
    assert!(crash_in_rename(&s.source_path, &journal_path));
    assert_eq!(fs::read(&s.source_path).unwrap(), ORIGINAL);
    let left = temps(s.dir.path());
    assert_eq!(left.len(), 1, "the fsynced temp is left behind");
    assert_eq!(fs::read(s.dir.join(&left[0])).unwrap(), EDITED);
    let pending: Vec<String> = journal(&journal_path)
        .iter()
        .map(|e| {
            Path::new(e["temp"].as_str().unwrap())
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(pending, left, "the journal names the temp");
    cleanup_orphans(&journal_path);
    assert_eq!(temps(s.dir.path()), Vec::<String>::new());
    assert_eq!(journal(&journal_path), Vec::<Value>::new());
    assert_eq!(fs::read(&s.source_path).unwrap(), ORIGINAL);

    // The next start: a host on the user-data folder that holds the journal removes the temp.
    let t = scratch("m10-crash-host", ORIGINAL);
    let udata = Scratch::new("m10-crash-udata");
    let journal_path = udata.join("pending-saves.json");
    assert!(crash_in_rename(&t.source_path, &journal_path));
    assert_eq!(temps(t.dir.path()).len(), 1);
    let host = Host::new(
        HostConfig {
            user_data: udata.path().to_path_buf(),
            smoke: true,
            echo_stdout: false,
        },
        Box::new(Scripted::default()),
    );
    assert_eq!(host.journal(), journal_path);
    assert_eq!(temps(t.dir.path()), Vec::<String>::new());
    assert_eq!(journal(&journal_path), Vec::<Value>::new());
    assert_eq!(fs::read(&t.source_path).unwrap(), ORIGINAL);
}

// ------------------------------------------------------------------ save-as-rebind

// was tests/save-as-rebind.test.mjs: 'Save As of an Untitled document onto a link retargeted
// inside the rename/dirSync hook binds to the file written'. `Inject` has no rename or dir-sync
// hook, so the link is retargeted right after the save returns (the race window itself is not
// reachable): the identity is the written file's, so the next Save refuses the other file.
#[cfg(unix)]
#[test]
fn save_as_onto_a_link_reports_the_file_written_and_the_next_save_refuses_a_retargeted_link() {
    let dir = Scratch::new("rebind");
    let jdir = Scratch::new("rebind-j");
    let journal_path = jdir.join("pending-saves.json");
    let fresh = dir.join("pdf4all-new-x");
    fs::create_dir(&fresh).unwrap();
    let untitled = fresh.join("Untitled.pdf");
    let blank = host::blank::blank_pdf(612.0, 792.0);
    fs::write(&untitled, &blank).unwrap();
    let source = FileId::of(&untitled).unwrap();
    let first = dir.join("t1.pdf");
    let second = dir.join("t2.pdf");
    fs::write(&first, b"%PDF-old-first").unwrap();
    fs::write(&second, PRECIOUS).unwrap();
    let alias = dir.join("alias.pdf");
    symlink(&first, &alias);
    let saved = save_copy(
        Some(&alias),
        EDITED,
        &source,
        Some(&journal_path),
        true,
        &none(),
        &yes,
    )
    .unwrap();
    retarget(&alias, &second);
    assert_eq!(fs::read(&first).unwrap(), EDITED);
    assert_eq!(fs::read(&second).unwrap(), PRECIOUS);
    assert_eq!(
        fs::read(&untitled).unwrap(),
        blank,
        "the Untitled file is untouched"
    );
    let file = saved.file.expect("written identity");
    assert_eq!(file.path, alias);
    assert_eq!(
        (file.dev, file.ino),
        key(&first),
        "the identity of the file written"
    );
    assert_eq!(file.size, EDITED.len() as u64);
    assert_eq!(
        save_in_place(b"%PDF-next-save", &file, Some(&journal_path), &none(), &yes),
        Err(SaveFail::SourceChanged)
    );
    assert_eq!(fs::read(&second).unwrap(), PRECIOUS);
    assert_eq!(temps(dir.path()), Vec::<String>::new());
}

// was tests/save-as-rebind.test.mjs: 'saveCopy and writeNew report the written file for a plain
// path; main.ts binds Save As to it' (the safe-write half)
#[test]
fn save_copy_and_write_new_report_the_written_file() {
    let s = scratch("rebind-plain", ORIGINAL);
    let dest = s.dir.join("made.pdf");
    let copied = save_copy(Some(&dest), EDITED, &s.source, None, false, &none(), &yes).unwrap();
    let file = copied.file.expect("written identity");
    let now = FileId::of(&dest).unwrap();
    assert_eq!(
        (&file.path, file.dev, file.ino, file.size),
        (&dest, now.dev, now.ino, now.size)
    );
    let made = s.dir.join("new.pdf");
    let written = write_new(Some(&made), EDITED, None, &none()).unwrap();
    let file = written.file.expect("written identity");
    assert_eq!((file.dev, file.ino), key(&made));
}

// was tests/save-as-rebind.test.mjs: 'saveCopy and writeNew report ...; main.ts binds Save As to
// it' (the main.ts half, which read its source text): the Rust host binds the window to the
// identity the write returned, so a link retargeted after Save As cannot redirect the next Save.
#[cfg(unix)]
#[test]
fn host_save_as_binds_the_window_to_the_file_written() {
    let dir = Scratch::new("rebind-host");
    let opened = dir.join("opened.pdf");
    fs::write(&opened, ORIGINAL).unwrap();
    let first = dir.join("t1.pdf");
    let second = dir.join("t2.pdf");
    fs::write(&first, b"%PDF-old-first").unwrap();
    fs::write(&second, PRECIOUS).unwrap();
    let alias = dir.join("alias.pdf");
    symlink(&first, &alias);

    let mut host = smoke_host(&dir);
    let doc = open(&mut host, &opened);
    save_to(&mut host, Some(alias.clone()));
    let rebind = SaveOptions {
        rebind: true,
        ..SaveOptions::default()
    };
    let ok = host.save_file(EDITED, &doc.url, &rebind).unwrap();
    assert_eq!(ok.name, "alias.pdf");
    assert!(!ok.same_file);
    assert_eq!(fs::read(&first).unwrap(), EDITED);
    assert_eq!(fs::read(&opened).unwrap(), ORIGINAL);
    let bound = host.open_record().expect("bound").file.clone();
    assert_eq!(bound.path, alias);
    assert_eq!((bound.dev, bound.ino), key(&first));
    assert_eq!(bound.size, EDITED.len() as u64);

    retarget(&alias, &second);
    let in_place = SaveOptions {
        in_place: true,
        ..SaveOptions::default()
    };
    assert_eq!(
        host.save_file(b"%PDF-next-save", &doc.url, &in_place),
        Err(SaveFail::SourceChanged)
    );
    assert_eq!(fs::read(&second).unwrap(), PRECIOUS);
    assert_eq!(fs::read(&first).unwrap(), EDITED);
    assert_eq!(temps(dir.path()), Vec::<String>::new());

    // Too large is refused before any sheet, whatever the target.
    let huge = vec![0u8; MAX_SAVE_BYTES + 1];
    assert_eq!(
        host.save_file(&huge, &doc.url, &in_place),
        Err(SaveFail::TooLarge)
    );
}

// ------------------------------------------------------------------ p1-open: the one-shot open

// was tests/p1-open.test.mjs: 'open stream caps at issued size and rejects truncated or non-file
// sources'. `read_capped` caps; a file shorter than issued is refused by `Host::read_open`.
#[test]
fn open_reads_cap_at_the_issued_size_and_refuse_changed_or_non_file_sources() {
    let dir = Scratch::new("open-cap");
    let file = dir.join("doc.pdf");
    let payload = vec![7u8; 100];
    fs::write(&file, &payload).unwrap();
    assert_eq!(read_capped(&file, 40).unwrap(), &payload[..40]);
    assert_eq!(read_capped(&file, 100).unwrap(), payload);
    // A cap past the end reads what is there; `read_open` then refuses the short read.
    assert_eq!(read_capped(&file, 150).unwrap().len(), 100);
    let nested = dir.join("subdir");
    fs::create_dir(&nested).unwrap();
    assert!(read_capped(&nested, 10).is_err());
    assert_eq!(FileId::of(&nested), None);
    assert_eq!(FileId::of(&dir.join("missing.pdf")), None);
    assert_eq!(FileId::of(&file).unwrap().size, 100);

    let mut host = smoke_host(&dir);
    // Truncated after the open was issued: refused.
    let doc = open(&mut host, &file);
    assert_eq!(doc.size, 100);
    fs::write(&file, &payload[..60]).unwrap();
    assert_eq!(host.read_open(&doc.url), None);
    // Grown after the open was issued: refused (never more than the issued size).
    let doc = open(&mut host, &file);
    assert_eq!(doc.size, 60);
    fs::write(&file, &payload).unwrap();
    assert_eq!(host.read_open(&doc.url), None);
    // Unchanged: the issued bytes, once.
    let doc = open(&mut host, &file);
    assert_eq!(host.read_open(&doc.url), Some(payload));
    assert_eq!(host.read_open(&doc.url), None);
}

// was tests/p1-open.test.mjs: 'openToken accepts a UUID path and rejects the rest'. The Rust host
// keeps the issued tokens: only an exact issued URL reads, once.
#[test]
fn only_an_exact_issued_open_url_reads() {
    let dir = Scratch::new("open-token");
    let file = dir.join("doc.pdf");
    fs::write(&file, ORIGINAL).unwrap();
    let mut host = smoke_host(&dir);
    let doc = open(&mut host, &file);
    let token = doc
        .url
        .strip_prefix("app://pdf4all/open/")
        .expect("an open URL")
        .to_string();
    for url in [
        "app://pdf4all/open/".to_string(),
        "app://pdf4all/open/../index.html".to_string(),
        format!("app://pdf4all/open/{}", token.to_uppercase()),
        format!("app://pdf4all/open/{}", &token[..token.len() - 1]),
        format!("app://pdf4all/open/{token}/"),
        format!("app://other/open/{token}"),
        format!("/open/{token}"),
        token.clone(),
        String::new(),
    ] {
        if url == doc.url {
            continue;
        }
        assert_eq!(host.read_open(&url), None, "{url}");
    }
    // The mistakes above did not spend it.
    assert_eq!(host.read_open(&doc.url), Some(ORIGINAL.to_vec()));
    assert_eq!(host.read_open(&doc.url), None);
}

// was tests/p0.test.mjs: 'protocol permits only the exact packaged assets' (the isFixture part):
// only the three packaged samples resolve, and never a path.
#[test]
fn only_the_packaged_samples_resolve() {
    assert_eq!(host::FIXTURES, ["hello.pdf", "form.pdf", "large.pdf"]);
    assert!(host::is_fixture("large.pdf"));
    for name in [
        "cjk.pdf",
        "rtl.pdf",
        "rotated.pdf",
        "../hello.pdf",
        "fixtures/hello.pdf",
        "/etc/passwd",
        "",
    ] {
        assert!(!host::is_fixture(name), "{name}");
        assert_eq!(host::fixture_path(name), None, "{name}");
    }
}
