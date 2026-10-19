//! The `--smoke` / `--drive` protocol of the headless app (port of main.ts's
//! command line, smoke modes and `startDrive`, M:24-57, M:141-159,
//! M:611-672, M:1760-2257; spec-drive-state.md §1–2).
//!
//! - [`Args`]: the command line, parsed as main.ts parsed it.
//! - [`main`]: the process: single-instance lock (exit 3), the [`Host`],
//!   startup validation, one smoke mode or the drive, the report, the exit code.
//! - [`run_drive`]: the JSON-lines harness (`--drive=<file>` and live
//!   `--drive=-`), `${OUT}` substitution, dialog pre-answers per line, secrets,
//!   step records and [`matches`].
//! - [`Driven`]: what the document session implements so the harness can
//!   drive it: the main → renderer messages, STATE, and the console lines.
//!
//! The engine is synchronous, so a step is: apply the line's pre-answers, call
//! the session, read STATE. Electron's 60 s step wait collapses to "did this
//! request reach the session": a request the preload drops (over-long
//! `contents`) gives the `step timeout` record at once.

use crate::host::{self, Document, Event, Host, HostConfig, OpenError, SaveRequest, SingletonLock};
use crate::platform::{Answers, Scripted};
use crate::safe_write::{self, FileId, Inject};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Instant;

/// When this process began (`started = performance.now()`, M:58).
static STARTED: OnceLock<Instant> = OnceLock::new();

fn started() -> Instant {
    *STARTED.get_or_init(Instant::now)
}

/// `MAX_NOTE_CHARS` (preload.cts:24): the cap on an `edit-request`'s contents.
pub const MAX_NOTE_CHARS: usize = 4096;
/// `MAX_SIGN_DRAW_CHARS` (preload.cts:27).
pub const MAX_SIGN_DRAW_CHARS: usize = 4 * 1024 * 1024;

/// The `--edit` smoke operations (M:625).
pub const SMOKE_EDITS: [&str; 16] = [
    "note",
    "highlight",
    "freetext",
    "pages",
    "form",
    "nav",
    "assemble",
    "extract",
    "export-png",
    "export-jpeg",
    "export-txt",
    "decorate",
    "shapes",
    "encrypt",
    "encrypt-keep",
    "content",
];

// ------------------------------------------------------------------ the session seam

/// What the harness drives: the document session (the port of the Electron
/// renderer). Each method is one main → renderer message; the session handles
/// it to the end (the engine is synchronous) and then answers [`Driven::state`].
///
/// Lines the renderer printed with `console.log` (`P0_RESULT`, `P0_ERROR`,
/// `P1_RESULT`, `P1_THUMBS`, `P1_SAVE`, `P1_SAVE_ERROR`, `P1_PRINT`, `P2_EDIT`,
/// `P2_NEW`, `P3_EDIT`) are queued by the session and collected with
/// [`Driven::take_lines`] after each call. `STATE` lines are not queued: the
/// harness asks [`Driven::state`] after every request instead.
///
/// The session owns the [`Host`] (`session.host`) and calls it for every file,
/// dialog, print and link. When a host call queues main → renderer messages
/// ([`Host::take_events`]: the `document` of an open, an `edit-request` the
/// pending-boxes question sent) the session should handle them itself before
/// returning; the harness also drains any left over and delivers them here.
pub trait Driven {
    /// The startup sample (`openStartupFixture`, R:2380): `Some(name)` opens
    /// the packaged fixture (bytes from [`Host::fixture_bytes`]) and queues
    /// `P0_RESULT` then `P1_RESULT` (or `P0_ERROR {"message"}`); `None` shows the
    /// start screen. Not a driven request: no STATE follows.
    fn start(&mut self, fixture: Option<&str>);
    /// `document {name, size, url, untitled?}` (R:1660): open it (bytes from
    /// [`Host::read_open`]); queues `P1_RESULT` when it opened.
    fn deliver_document(&mut self, doc: Document);
    /// `open-error {reason, name?, limit?}` (R:1666).
    fn open_error(&mut self, error: OpenError);
    /// `save-request` (R:1681): Copy = `saveAs()`, InPlace = `saveInPlace()`,
    /// As = `saveInPlace("save-as")`; queues `P1_SAVE` / `P1_SAVE_ERROR`.
    fn save_request(&mut self, request: SaveRequest);
    /// `edit-request {op, contents?}` (R:1687), after the preload's checks:
    /// `op` is non-empty and `contents` within the caps. `None` = the menu path.
    /// The session switches the mode first (`OP_MODE`), then runs
    /// `runEditRequest(op, contents)`; smoke edits queue `P2_EDIT`/`P3_EDIT`/`P2_NEW`.
    fn edit_request(&mut self, op: &str, contents: Option<String>);
    /// `print-request {range?}` (R:1699): `None` opens the print dialog, else
    /// `startPrint(range)`, which hands the PDF to [`Host::print`] and queues `P1_PRINT`.
    fn print_request(&mut self, range: Option<String>);
    /// STATE after a driven request: `refreshPageIds()` then `stateSummary()`
    /// (R:1712-1717, R:9926). Called exactly once after every request.
    fn state(&mut self) -> Value;
    /// The console lines queued since the last call, in order: `(prefix, payload)`,
    /// e.g. `("P1_RESULT", {...})`.
    fn take_lines(&mut self) -> Vec<(String, Value)>;
    /// The host the session owns.
    fn host(&mut self) -> &mut Host;

    /// Delivers one main → renderer message to the matching method.
    fn dispatch(&mut self, event: Event) {
        match event {
            Event::Document(doc) => self.deliver_document(doc),
            Event::OpenError(error) => self.open_error(error),
            Event::SaveRequest(request) => self.save_request(request),
            Event::EditRequest { op, contents } => self.edit_request(&op, contents),
            Event::PrintRequest { range } => self.print_request(range),
        }
    }
}

/// The preload's `onEditRequest` checks (preload.cts:107-121): `None` when the
/// request is dropped (no STATE follows); an over-cap `sign-draw` arrives empty.
pub fn preload_edit(op: &str, contents: Option<String>) -> Option<Option<String>> {
    if op.is_empty() {
        return None;
    }
    let Some(text) = contents else {
        return Some(None);
    };
    let cap = if op == "sign-draw" {
        MAX_SIGN_DRAW_CHARS
    } else {
        MAX_NOTE_CHARS
    };
    if text.encode_utf16().count() <= cap {
        Some(Some(text))
    } else if op == "sign-draw" {
        Some(Some(String::new()))
    } else {
        None
    }
}

/// Hands one message to the session the way `contents.send` + the preload
/// did; `false` when the preload dropped it.
fn deliver<D: Driven + ?Sized>(s: &mut D, event: Event) -> bool {
    match event {
        Event::EditRequest { op, contents } => match preload_edit(&op, contents) {
            Some(contents) => {
                s.edit_request(&op, contents);
                true
            }
            None => false,
        },
        other => {
            s.dispatch(other);
            true
        }
    }
}

/// Delivers the messages the host queued (bounded: a message may queue more).
fn pump<D: Driven + ?Sized>(s: &mut D) -> usize {
    let mut delivered = 0;
    for _ in 0..16 {
        let events = s.host().take_events();
        if events.is_empty() {
            break;
        }
        for event in events {
            if deliver(s, event) {
                delivered += 1;
            }
        }
    }
    delivered
}

// ------------------------------------------------------------------ command line

/// The command line (M:24-57; spec §1.2). Unknown arguments (Chromium switches,
/// the `.` app entry, PDF paths) are ignored.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Args {
    pub drive: Option<String>,
    pub drive_out: Option<String>,
    pub gui: bool,
    /// `--smoke`, or any `--drive` (a drive is a smoke).
    pub smoke: bool,
    pub report: Option<String>,
    pub net_log: Option<String>,
    pub fixture: String,
    pub sample: bool,
    pub open: Option<String>,
    pub save_to: Option<String>,
    pub save_in_place: bool,
    pub edit: Option<String>,
    pub password: Option<String>,
    pub new_passwords: Option<String>,
    pub insert_from: Option<String>,
    pub print_to: Option<String>,
    pub print_range: Option<String>,
    pub expect_thumbs: bool,
    pub new_from_images: bool,
    pub images: Option<String>,
    pub batch_dir: Option<String>,
    pub user_data_dir: Option<String>,
}

impl Args {
    /// `argv` as `std::env::args()` gives it (the program first).
    pub fn parse(argv: &[String]) -> Args {
        // `argvValue` (M:26): `--name=value` (first match) or `--name value` when
        // the next item exists and does not start with `-`.
        let value = |name: &str| -> Option<String> {
            let prefix = format!("{name}=");
            if let Some(v) = argv.iter().find_map(|a| a.strip_prefix(&prefix)) {
                return Some(v.to_string());
            }
            let i = argv.iter().position(|a| a == name)?;
            argv.get(i + 1)
                .filter(|next| !next.is_empty() && !next.starts_with('-'))
                .cloned()
        };
        // Every other valued flag: `--name=value` only (first match).
        let eq = |name: &str| -> Option<String> {
            let prefix = format!("{name}=");
            argv.iter()
                .find_map(|a| a.strip_prefix(&prefix))
                .map(str::to_string)
        };
        let flag = |name: &str| argv.iter().any(|a| a == name);
        let drive = value("--drive");
        Args {
            smoke: flag("--smoke") || drive.is_some(),
            drive,
            drive_out: value("--drive-out"),
            gui: flag("--gui"),
            report: eq("--report"),
            net_log: eq("--net-log"),
            fixture: eq("--fixture").unwrap_or_else(|| "hello.pdf".into()),
            sample: flag("--sample"),
            open: eq("--open"),
            save_to: eq("--save-to"),
            save_in_place: flag("--save-in-place"),
            edit: eq("--edit"),
            password: value("--password"),
            new_passwords: value("--new-passwords"),
            insert_from: eq("--insert-from"),
            print_to: eq("--print-to"),
            print_range: eq("--print-range"),
            expect_thumbs: flag("--expect=P1_THUMBS"),
            new_from_images: flag("--new-from-images"),
            images: eq("--images"),
            batch_dir: eq("--batch-dir"),
            user_data_dir: eq("--user-data-dir"),
        }
    }

    /// The headless platform's pre-answers from the command line (raw, as main kept them).
    pub fn answers(&self) -> Answers {
        Answers {
            save_to: self.save_to.as_ref().map(PathBuf::from),
            insert_from: self.insert_from.as_ref().map(PathBuf::from),
            images: self.images.clone(),
            batch_dir: self.batch_dir.as_ref().map(PathBuf::from),
            filled_over: false,
            print_to: self.print_to.as_ref().map(PathBuf::from),
        }
    }

    /// The startup validation (M:611-657; spec §1.3, items 1-15): the error a
    /// smoke ends with before anything opens.
    pub fn startup_error(&self) -> Option<&'static str> {
        let edit = self.edit.as_deref();
        let checks: [(bool, &'static str); 15] = [
            (!host::is_fixture(&self.fixture), "unknown fixture"),
            (
                self.print_to.is_some() && (self.save_to.is_some() || self.save_in_place),
                "print and save together",
            ),
            (
                self.save_to.is_some() && self.save_in_place,
                "save-to and save-in-place together",
            ),
            (
                self.save_in_place && self.open.is_none(),
                "save-in-place needs --open",
            ),
            (
                edit.is_some_and(|e| !SMOKE_EDITS.contains(&e)),
                "unknown edit",
            ),
            (edit.is_some() && self.open.is_none(), "edit needs --open"),
            (
                edit == Some("assemble") && self.insert_from.is_none(),
                "assemble needs --insert-from",
            ),
            (
                edit == Some("shapes") && self.images.is_none(),
                "shapes needs --images",
            ),
            (
                edit == Some("extract") && self.save_to.is_none(),
                "extract needs --save-to",
            ),
            (
                edit == Some("extract") && self.save_in_place,
                "extract is a copy",
            ),
            (
                edit.is_some_and(|e| e.starts_with("export-")) && self.save_to.is_none(),
                "export needs --save-to",
            ),
            (
                edit == Some("encrypt") && (self.new_passwords.is_none() || self.save_to.is_none()),
                "encrypt needs --new-passwords and --save-to",
            ),
            (
                edit == Some("encrypt-keep") && (self.password.is_none() || self.save_to.is_none()),
                "encrypt-keep needs --password and --save-to",
            ),
            (
                self.new_from_images && (self.save_to.is_none() || self.images.is_none()),
                "new-from-images needs --images and --save-to",
            ),
            (
                self.new_from_images && (self.open.is_some() || self.edit.is_some()),
                "new-from-images is its own smoke",
            ),
        ];
        checks.iter().find(|(bad, _)| *bad).map(|(_, error)| *error)
    }

    /// `smokeEditPayload` (M:229): the `--edit` request.
    pub fn smoke_edit_payload(&self) -> (String, Option<String>) {
        let edit = self.edit.clone().unwrap_or_default();
        match edit.as_str() {
            "pages" => (edit, None),
            "encrypt" => {
                let (user, owner) = split_passwords(self.new_passwords.as_deref().unwrap_or(""));
                (
                    edit,
                    Some(
                        json!({"user": user, "owner": owner, "permissions": ["print", "annotate"]})
                            .to_string(),
                    ),
                )
            }
            // The P2 encrypt-keep smoke: the user password rides in the payload.
            "encrypt-keep" => {
                let password = self.password.as_ref().map_or(Value::Null, |p| json!(p));
                let mut body = Map::new();
                // JSON.stringify drops an undefined password.
                if !password.is_null() {
                    body.insert("password".into(), password);
                }
                body.insert("note".into(), json!("PDF4All P2 encrypt-keep"));
                (edit, Some(Value::Object(body).to_string()))
            }
            // P3 content smoke on a locked file: the user password rides in the payload.
            "content" => (
                edit,
                self.password
                    .as_ref()
                    .filter(|p| !p.is_empty())
                    .map(|p| json!({"password": p}).to_string()),
            ),
            _ => {
                let contents = format!("PDF4All P2 {edit}");
                (edit, Some(contents))
            }
        }
    }

    /// `smokePasswords` (M:245): the strings a P2_EDIT must not contain.
    pub fn smoke_passwords(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(p) = &self.password {
            out.push(p.clone());
        }
        if let Some(np) = &self.new_passwords {
            match np.split_once(':') {
                Some((user, owner)) => {
                    out.push(user.to_string());
                    out.push(owner.to_string());
                }
                None => out.push(np.clone()),
            }
        }
        out.retain(|p| !p.is_empty());
        out
    }
}

/// `--new-passwords=<user>:<owner>`: split at the first colon; none = owner is user.
fn split_passwords(text: &str) -> (String, String) {
    match text.split_once(':') {
        Some((u, o)) => (u.to_string(), o.to_string()),
        None => (text.to_string(), text.to_string()),
    }
}

// ------------------------------------------------------------------ the report

/// A run's end: the result object merged into the report, and whether it passed.
#[derive(Clone, Debug, PartialEq)]
pub struct Finish {
    pub result: Map<String, Value>,
    pub ok: bool,
}

impl Finish {
    pub fn new(result: Value, ok: bool) -> Finish {
        Finish {
            result: result.as_object().cloned().unwrap_or_default(),
            ok,
        }
    }
    pub fn error(error: impl Into<String>) -> Finish {
        Finish::new(json!({"error": error.into()}), false)
    }
}

/// Node's `process.platform` / `process.arch` names.
fn node_platform() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    }
}

fn node_arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        "x86" => "ia32",
        other => other,
    }
}

/// Whether this binary runs from an app bundle (`app.isPackaged`).
fn packaged() -> bool {
    std::env::current_exe().is_ok_and(|p| p.to_string_lossy().contains(".app/Contents/MacOS/"))
}

/// Whether the process runs under the macOS App Sandbox: the sandbox gives
/// every sandboxed process `APP_SANDBOX_CONTAINER_ID`. Elsewhere `false`.
pub fn sandboxed() -> bool {
    std::env::var_os("APP_SANDBOX_CONTAINER_ID").is_some()
}

/// `metrics()` (M:328): one process here, with its memory in KB.
pub fn metrics() -> Value {
    let (rss, peak) = memory_kb();
    let mut m = json!({"type": "Browser", "sandboxed": sandboxed()});
    if let Some(rss) = rss {
        m["workingSetSize"] = json!(rss);
    }
    if let Some(peak) = peak.or(rss) {
        m["peakWorkingSetSize"] = json!(peak);
    }
    json!([m])
}

/// Resident and peak memory in KB: `/proc/self/status` on Linux, `ps` on macOS.
fn memory_kb() -> (Option<u64>, Option<u64>) {
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        let field = |name: &str| {
            status
                .lines()
                .find_map(|l| l.strip_prefix(name))
                .and_then(|rest| rest.split_whitespace().next())
                .and_then(|n| n.parse::<u64>().ok())
        };
        return (field("VmRSS:"), field("VmHWM:"));
    }
    let rss = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .trim()
                .parse::<u64>()
                .ok()
        });
    (rss, None)
}

/// The report object (M:144): `ok, packaged, platform, arch, versions,
/// elapsedMs, localRequests, protocolRequests, blockedRequests, p2Link?, ...result`.
/// The request lists are empty: the native app serves no assets over a protocol
/// and blocks nothing because it makes no requests at all.
pub fn report(finish: &Finish, p2_link: Option<(&str, bool)>) -> Value {
    let mut r = Map::new();
    r.insert("ok".into(), json!(finish.ok));
    r.insert("packaged".into(), json!(packaged()));
    r.insert("platform".into(), json!(node_platform()));
    r.insert("arch".into(), json!(node_arch()));
    r.insert(
        "versions".into(),
        json!({"tandem": env!("CARGO_PKG_VERSION"), "engine": "rust", "shell": "headless"}),
    );
    r.insert(
        "elapsedMs".into(),
        json!(started().elapsed().as_millis() as u64),
    );
    r.insert("localRequests".into(), json!([]));
    r.insert("protocolRequests".into(), json!([]));
    r.insert("blockedRequests".into(), json!([]));
    if let Some((uri, allowed)) = p2_link {
        r.insert("p2Link".into(), json!({"uri": uri, "allowed": allowed}));
    }
    for (k, v) in &finish.result {
        r.insert(k.clone(), v.clone());
    }
    Value::Object(r)
}

/// `finish(result, ok)` (M:141): one `P0_SMOKE` line, the `--report` file, the
/// exit code (0 iff ok; a report that cannot be written makes it 1).
fn finish(
    finish: Finish,
    args: &Args,
    p2_link: Option<(&str, bool)>,
    lock: Option<&mut SingletonLock>,
) -> ! {
    let report = report(&finish, p2_link);
    let mut ok = finish.ok;
    {
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "P0_SMOKE {report}");
        let _ = out.flush();
    }
    let mut failed = false;
    if let Some(path) = &args.report {
        let text = serde_json::to_string_pretty(&report).unwrap_or_default() + "\n";
        failed |= write_file(path, text.as_bytes()).is_err();
    }
    // `--net-log`: Electron wrote Chromium's network log; the native app makes no network
    // requests, so the log has no events (nothing reads it).
    if let Some(path) = &args.net_log {
        failed |= write_file(path, b"{\"constants\":{},\"events\":[]}\n").is_err();
    }
    if failed {
        eprintln!("Unable to write smoke report");
        ok = false;
    }
    if let Some(lock) = lock {
        lock.release();
    }
    std::process::exit(if ok { 0 } else { 1 })
}

fn write_file(path: &str, bytes: &[u8]) -> Result<(), safe_write::SaveFail> {
    let path = host::util::resolve(path);
    safe_write::write_new(Some(&path), bytes, None, &Inject::default()).map(|_| ())
}

// ------------------------------------------------------------------ the process

/// The headless process: `tandem-headless --smoke …` or `--drive=<file|->`.
/// `make` builds the document session around the host.
pub fn main<D: Driven>(make: impl FnOnce(Host) -> D) -> ! {
    started();
    let argv: Vec<String> = std::env::args().collect();
    let args = Args::parse(&argv);
    if !args.smoke {
        eprintln!(
            "tandem-headless runs only --smoke or --drive=<script.jsonl|->; the app is the Tauri shell"
        );
        std::process::exit(2);
    }
    let user_data = args
        .user_data_dir
        .as_ref()
        .map_or_else(host::default_user_data, |d| host::util::resolve(d));
    // M:584: one instance per user-data folder; a second exits 3 touching nothing there.
    let mut lock = match SingletonLock::acquire(&user_data) {
        Ok(Some(lock)) => lock,
        Ok(None) => std::process::exit(3),
        Err(_) => finish(Finish::error("startup failed"), &args, None, None),
    };
    let host = Host::new(
        HostConfig {
            user_data,
            smoke: true,
            echo_stdout: true,
        },
        Box::new(Scripted::new(args.answers())),
    );
    if let Some(error) = args.startup_error() {
        finish(Finish::error(error), &args, None, Some(&mut lock));
    }
    if let Some(open) = &args.open
        && let Some(e) = Host::inspect_open(open)
    {
        finish(
            Finish::new(json!({"openError": e.to_json()}), false),
            &args,
            None,
            Some(&mut lock),
        );
    }
    if let Some(from) = &args.insert_from
        && let Some(e) = Host::inspect_open(from)
    {
        finish(
            Finish::new(json!({"insertError": e.to_json()}), false),
            &args,
            None,
            Some(&mut lock),
        );
    }
    // The drive script is read before the window loads (M:2149-2156).
    let script = match args.drive.as_deref() {
        Some("-") => Some(Script::Live),
        Some(path) => match read_script(path, &drive_out(&args)) {
            Ok(lines) => Some(Script::Lines(lines)),
            Err(error) => finish(
                Finish::error(format!("drive script: {error}")),
                &args,
                None,
                Some(&mut lock),
            ),
        },
        None => None,
    };
    let mut session = make(host);
    let done = match script {
        Some(script) => {
            let live = matches!(script, Script::Live);
            let stdin = std::io::stdin();
            let stdout = std::io::stdout();
            let mut emit = |step: &Value| {
                if live {
                    let mut out = stdout.lock();
                    let _ = writeln!(out, "STEP {step}");
                    let _ = out.flush();
                }
            };
            run_drive(
                &mut session,
                &args.fixture,
                &drive_out(&args),
                script,
                stdin.lock(),
                &mut emit,
            )
        }
        None if args.open.is_some() => smoke_open(&mut session, &args),
        None if args.new_from_images => smoke_new_from_images(&mut session, &args),
        None => smoke_plain(&mut session, &args),
    };
    let link = session
        .host()
        .last_p2_link()
        .map(|(u, a)| (u.to_string(), a));
    finish(
        done,
        &args,
        link.as_ref().map(|(u, a)| (u.as_str(), *a)),
        Some(&mut lock),
    )
}

/// `--drive-out` or `os.tmpdir()`.
fn drive_out(args: &Args) -> String {
    args.drive_out
        .clone()
        .unwrap_or_else(|| host::util::temp_dir().to_string_lossy().into_owned())
}

// ------------------------------------------------------------------ the drive

/// Where the drive's lines come from.
#[derive(Debug)]
pub enum Script {
    /// A script file, already substituted and parsed.
    Lines(Vec<Value>),
    /// `--drive=-`: stdin, one line at a time.
    Live,
}

/// `${OUT}` substitution (M:2146): `${OUT}` plus everything after it up to the
/// next `"`, `\`, CR or LF becomes `out` + that rest, JSON-string-escaped.
pub fn with_out(text: &str, out: &str) -> String {
    const KEY: &str = "${OUT}";
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find(KEY) {
        result.push_str(&rest[..i]);
        let after = &rest[i + KEY.len()..];
        let end = after.find(['"', '\\', '\r', '\n']).unwrap_or(after.len());
        let quoted = serde_json::to_string(&format!("{out}{}", &after[..end])).unwrap_or_default();
        result.push_str(&quoted[1..quoted.len() - 1]);
        rest = &after[end..];
    }
    result.push_str(rest);
    result
}

/// File mode (M:2150-2155): substitute over the whole text, split on `\n`,
/// trim, drop empty and `#` lines, parse each.
pub fn read_script(path: &str, out: &str) -> Result<Vec<Value>, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let text = String::from_utf8(bytes).map_err(|e| e.to_string())?;
    parse_script(&with_out(&text, out))
}

/// The parse step of [`read_script`].
pub fn parse_script(text: &str) -> Result<Vec<Value>, String> {
    text.split('\n')
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| serde_json::from_str(l).map_err(|e| e.to_string()))
        .collect()
}

/// `matches(got, want)` (M:2116): a subset match. `want` null or a primitive:
/// strict equality (numbers as JS numbers); an array: same length, each item
/// matching; an object: every key of `want` matches the same key of `got`. An
/// absent `got` (JS `undefined`) matches nothing.
pub fn matches(got: Option<&Value>, want: &Value) -> bool {
    match want {
        Value::Array(w) => match got {
            Some(Value::Array(g)) => {
                g.len() == w.len() && w.iter().zip(g).all(|(w, g)| matches(Some(g), w))
            }
            _ => false,
        },
        Value::Object(w) => match got {
            Some(Value::Object(g)) => w.iter().all(|(k, v)| matches(g.get(k), v)),
            // An array is a JS object too: its index keys and `length`.
            Some(Value::Array(g)) => w.iter().all(|(k, v)| {
                let item = if k == "length" {
                    Some(json!(g.len()))
                } else {
                    k.parse::<usize>()
                        .ok()
                        .filter(|i| i.to_string() == *k)
                        .and_then(|i| g.get(i).cloned())
                };
                matches(item.as_ref(), v)
            }),
            _ => false,
        },
        _ => match (got, want) {
            (Some(Value::Number(a)), Value::Number(b)) => a.as_f64() == b.as_f64(),
            (Some(g), w) => g == w,
            (None, _) => false,
        },
    }
}

/// `String(value)` for a JSON value.
fn js_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n
            .as_f64()
            .map_or_else(|| n.to_string(), |f| host::util::js_number(f).to_string()),
        Value::Array(items) => items
            .iter()
            .map(|i| {
                if i.is_null() {
                    String::new()
                } else {
                    js_string(i)
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".into(),
    }
}

/// `Number(value) || 0` for a wait's `ms`.
fn js_ms(v: Option<&Value>) -> f64 {
    let n = match v {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(Value::String(s)) => {
            let t = s.trim();
            if t.is_empty() {
                0.0
            } else {
                t.parse::<f64>().unwrap_or(f64::NAN)
            }
        }
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        _ => 0.0,
    };
    if n.is_finite() && n > 0.0 { n } else { 0.0 }
}

/// `pathOf` (M:2177): a non-empty string, resolved against the working folder.
fn path_of(v: Option<&Value>) -> Option<PathBuf> {
    v.and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(host::util::resolve)
}

/// `driveSecrets(line)` (M:2123): a `password` step's contents; an `encrypt`,
/// `encrypt-keep` or `sign` step's `user`/`owner`/`password` (sign: `password`).
/// Unlike Electron, an empty password is not a secret (every text contains "",
/// so it would fail any run); its contents are still redacted.
pub fn drive_secrets(cmd: &str, contents: Option<&Value>) -> (bool, Vec<String>) {
    if cmd == "password" {
        if let Some(Value::String(s)) = contents {
            return (
                true,
                if s.is_empty() {
                    vec![]
                } else {
                    vec![s.clone()]
                },
            );
        }
        return (false, vec![]);
    }
    let keys: &[&str] = match cmd {
        "encrypt" | "encrypt-keep" => &["user", "owner", "password"],
        "sign" => &["password"],
        _ => return (false, vec![]),
    };
    let body = match contents {
        Some(Value::String(s)) => serde_json::from_str(s).unwrap_or(json!({})),
        Some(v) => v.clone(),
        None => Value::Null,
    };
    let found: Vec<String> = keys
        .iter()
        .filter_map(|k| body.get(*k).and_then(Value::as_str))
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    (!found.is_empty(), found)
}

/// Runs a drive (M:2142-2257) on the startup sample `fixture`: `out` is the
/// `${OUT}` folder, `script` the lines (or `input`, read line by line), `emit`
/// writes a live `STEP` line. Returns the run's end.
pub fn run_drive<D: Driven + ?Sized, R: BufRead>(
    s: &mut D,
    fixture: &str,
    out: &str,
    script: Script,
    mut input: R,
    emit: &mut dyn FnMut(&Value),
) -> Finish {
    let mut steps: Vec<Value> = Vec::new();
    let end = |error: String, steps: &[Value]| {
        Finish::new(json!({"error": error, "out": out, "steps": steps}), false)
    };
    // Wait for the startup sample (the first P1_RESULT) before any step.
    s.start(Some(fixture));
    let mut opened = false;
    for (kind, payload) in s.take_lines() {
        if kind == "P0_ERROR" {
            return end(format!("P0_ERROR {payload}"), &steps);
        }
        opened |= kind == "P1_RESULT";
    }
    if !opened {
        return end("startup failed".into(), &steps);
    }
    let live = matches!(script, Script::Live);
    let mut lines = match script {
        Script::Lines(lines) => lines.into_iter(),
        Script::Live => Vec::new().into_iter(),
    };
    let mut secrets: Vec<String> = Vec::new();
    let mut failed = false;
    let mut last: Option<Value> = None;
    loop {
        let line: Value = if live {
            let mut raw = Vec::new();
            match input.read_until(b'\n', &mut raw) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let text = String::from_utf8_lossy(&raw);
            let text = text.trim();
            if text.is_empty() || text.starts_with('#') {
                continue;
            }
            match serde_json::from_str::<Value>(&with_out(text, out)) {
                Ok(v) if v.is_object() => v,
                Ok(_) => {
                    let bad = json!({"error": "bad line: not a JSON object"});
                    emit(&bad);
                    steps.push(bad);
                    continue;
                }
                Err(e) => {
                    let bad = json!({"error": format!("bad line: {e}")});
                    emit(&bad);
                    steps.push(bad);
                    continue;
                }
            }
        } else {
            match lines.next() {
                Some(v) => v,
                None => break,
            }
        };
        // A file line that is not an object has no cmd or expect: an empty record.
        let mut args = match line {
            Value::Object(map) => map,
            Value::Array(items) => items
                .into_iter()
                .enumerate()
                .map(|(i, v)| (i.to_string(), v))
                .collect(),
            _ => Map::new(),
        };
        if args.get("cmd").and_then(Value::as_str) == Some("close") {
            break;
        }
        let cmd = args.remove("cmd");
        let expect = args.remove("expect");
        let mut step = Map::new();
        if let Some(cmd) = &cmd {
            step.insert("cmd".into(), cmd.clone());
        }
        step.extend(args.clone());
        if let Some(cmd) = &cmd {
            let name = cmd.as_str().unwrap_or("");
            let (redact, found) = drive_secrets(name, args.get("contents"));
            secrets.extend(found);
            // draw-signature review: the stroke points never reach the report or a STEP line.
            if redact || (name == "sign-draw" && args.contains_key("contents")) {
                step.insert("contents".into(), json!("[redacted]"));
            }
            apply_pre_answers(s.host(), &args);
            let reached = run_step(s, cmd, &args);
            for (kind, payload) in s.take_lines() {
                if kind == "P0_ERROR" {
                    return end(format!("P0_ERROR {payload}"), &steps);
                }
            }
            if !reached {
                step.insert("error".into(), json!("step timeout"));
                let timed_out = Value::Object(step);
                emit(&timed_out);
                steps.push(timed_out);
                return end("step timeout".into(), &steps);
            }
            let state = s.state();
            let text = state.to_string();
            if secrets.iter().any(|secret| text.contains(secret.as_str())) {
                return end("password leaked in STATE".into(), &steps);
            }
            step.insert("state".into(), state.clone());
            last = Some(state);
        }
        if let Some(expect) = expect {
            let ok = matches(last.as_ref(), &expect);
            step.insert("expect".into(), expect);
            step.insert("ok".into(), json!(ok));
            failed |= !ok;
        }
        let record = Value::Object(step);
        emit(&record);
        steps.push(record);
    }
    Finish::new(
        json!({"out": out, "steps": steps, "memory": metrics()}),
        !failed,
    )
}

/// The line-level dialog pre-answers (M:2214-2220).
fn apply_pre_answers(host: &mut Host, args: &Map<String, Value>) {
    let Some(answers) = host.platform.answers() else {
        return;
    };
    // A save target answers this line's dialog only (re-review fix 6).
    answers.save_to = path_of(args.get("to"));
    if let Some(from) = args.get("from") {
        answers.insert_from = path_of(Some(from));
    }
    if let Some(folder) = args.get("folder") {
        answers.batch_dir = path_of(Some(folder));
    }
    // UX item 8: the filled-form answer holds for this line only.
    answers.filled_over = args.get("filled").and_then(Value::as_str) == Some("over");
    if let Some(images) = args.get("images") {
        let items: Vec<String> = match images {
            Value::Array(items) => items.iter().map(js_string).collect(),
            other => js_string(other).split(',').map(str::to_string).collect(),
        };
        let resolved: Vec<String> = items
            .iter()
            .map(|i| host::util::resolve(i.trim()).to_string_lossy().into_owned())
            .collect();
        answers.images = Some(resolved.join(","));
    }
}

/// One step's request (M:2222-2239); `false` when it never reached the session.
fn run_step<D: Driven + ?Sized>(s: &mut D, cmd: &Value, args: &Map<String, Value>) -> bool {
    let set_save_to = |s: &mut D, path: Option<PathBuf>| {
        if let Some(a) = s.host().platform.answers() {
            a.save_to = path;
        }
    };
    match cmd.as_str() {
        Some("open") => {
            // An empty or missing path would show the open sheet: a headless run answers cancelled.
            let path = path_of(args.get("path"));
            s.host().deliver_open(path.as_deref(), false);
            pump(s) > 0
        }
        Some("new") => {
            // Contents are optional; present ones must be an object (or its JSON) whose `size` is valid.
            let body = match args.get("contents") {
                None => None,
                Some(Value::String(text)) => {
                    Some(serde_json::from_str(text).unwrap_or(Value::Null))
                }
                Some(v) => Some(v.clone()),
            };
            let size = match &body {
                None => None,
                Some(Value::Object(map)) => map.get("size").cloned(),
                Some(_) => Some(Value::Null),
            };
            s.host().new_blank(size.as_ref());
            pump(s) > 0
        }
        Some("save-to") => {
            set_save_to(s, path_of(args.get("path")));
            s.save_request(SaveRequest::Copy);
            pump(s);
            true
        }
        Some("save-in-place") => {
            set_save_to(s, path_of(args.get("path")));
            s.save_request(SaveRequest::InPlace);
            pump(s);
            true
        }
        // UX item 7: Save As writes `path` and the window then edits it.
        Some("save-as") => {
            set_save_to(s, path_of(args.get("path")));
            s.save_request(SaveRequest::As);
            pump(s);
            true
        }
        Some("print-to") => {
            let target = path_of(args.get("path"));
            if let Some(a) = s.host().platform.answers() {
                a.print_to = target;
            }
            let range = args
                .get("range")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            s.print_request(Some(range));
            pump(s);
            true
        }
        Some("wait") => {
            let ms = js_ms(args.get("ms")).min(600_000.0);
            std::thread::sleep(std::time::Duration::from_millis(ms as u64));
            deliver(
                s,
                Event::EditRequest {
                    op: "state".into(),
                    contents: None,
                },
            )
        }
        Some(op) => {
            // A drive `batch` without contents is refused by the session, never the menu's dialog.
            let body = match args.get("contents") {
                Some(Value::String(text)) => Some(text.clone()),
                Some(other) => Some(other.to_string()),
                None if op == "batch" => Some(String::new()),
                None if op == "content" => Some("{}".into()),
                None => None,
            };
            let reached = deliver(
                s,
                Event::EditRequest {
                    op: op.to_string(),
                    contents: body,
                },
            );
            pump(s);
            reached
        }
        // A non-string cmd is an edit-request the preload drops.
        None => false,
    }
}

// ------------------------------------------------------------------ smoke modes

fn file_baseline(path: &str) -> Option<(FileId, String, u32)> {
    use std::os::unix::fs::PermissionsExt;
    let meta = FileId::of(Path::new(path))?;
    let hash = sha256_file(path)?;
    let mode = std::fs::metadata(path).ok()?.permissions().mode();
    Some((meta, hash, mode))
}

fn sha256_file(path: &str) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    Some(
        Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    )
}

fn source_unchanged(open: &str, baseline: &Option<(FileId, String, u32)>) -> bool {
    let Some((meta, hash, _)) = baseline else {
        return false;
    };
    FileId::of(Path::new(open)).is_some_and(|now| now.mtime_ns == meta.mtime_ns)
        && sha256_file(open).as_ref() == Some(hash)
}

fn file_size(path: &str) -> Option<u64> {
    FileId::of(Path::new(path)).map(|m| m.size)
}

/// A request the smoke sends (`contents.send`), and the lines it produced.
fn send<D: Driven + ?Sized>(s: &mut D, queue: &mut VecDeque<(String, Value)>, event: Event) {
    deliver(s, event);
    pump(s);
    queue.extend(s.take_lines());
}

fn at<'a>(v: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(v, |v, k| v.get(*k))
}

fn num_at(v: &Value, path: &[&str]) -> Option<f64> {
    at(v, path).and_then(Value::as_f64)
}

/// `navEditOk` (M:185).
fn nav_edit_ok(state: &Value) -> bool {
    let Some(outline) = at(state, &["nav", "outline"])
        .and_then(Value::as_array)
        .filter(|o| !o.is_empty())
    else {
        return false;
    };
    let last = &outline[outline.len() - 1];
    if last.get("title") != Some(&json!("PDF4All P2 nav")) || num_at(last, &["page"]) != Some(1.0) {
        return false;
    }
    let by_title = |t: &str| {
        outline
            .iter()
            .rev()
            .find(|i| i.get("title").and_then(Value::as_str) == Some(t))
    };
    // `Map(outline.map(...))` keeps the last item of a title.
    if by_title("Two").and_then(|i| i.get("page")) != Some(&Value::Null)
        || by_title("Two-A").and_then(|i| i.get("page")) != Some(&Value::Null)
    {
        return false;
    }
    if by_title("Three").and_then(|i| num_at(i, &["page"])) != Some(1.0)
        || num_at(state, &["pages", "count"]) != Some(2.0)
    {
        return false;
    }
    let (Some(p0), Some(p1)) = (
        at(state, &["nav", "links", "0"]).and_then(Value::as_array),
        at(state, &["nav", "links", "1"]).and_then(Value::as_array),
    ) else {
        return false;
    };
    if p0
        .iter()
        .filter(|i| i.get("kind") == Some(&json!("page")) && num_at(i, &["page"]) == Some(1.0))
        .count()
        != 1
    {
        return false;
    }
    if !p0.iter().any(|i| i.get("kind") == Some(&json!("uri"))) {
        return false;
    }
    p1.len() == 1 && p1[0].get("kind") == Some(&json!("blocked"))
}

/// `navUndone` (M:202).
fn nav_undone(state: &Value) -> bool {
    num_at(state, &["pages", "count"]) == Some(3.0)
        && at(state, &["nav", "outline"])
            .and_then(Value::as_array)
            .and_then(|o| o.iter().find(|i| i.get("title") == Some(&json!("Two"))))
            .and_then(|i| num_at(i, &["page"]))
            == Some(1.0)
}

/// `assembleOk` (M:217).
fn assemble_ok(state: &Value, opened: f64) -> bool {
    let Some(inserted) = num_at(state, &["assemble", "inserted"]).filter(|n| *n >= 1.0) else {
        return false;
    };
    if num_at(state, &["pages", "count"]) != Some(opened + inserted) {
        return false;
    }
    match at(state, &["assemble", "fieldNames"]).and_then(Value::as_array) {
        Some(names) => {
            let strings: Option<Vec<&str>> = names.iter().map(Value::as_str).collect();
            strings.is_some_and(|s| {
                s.iter().collect::<std::collections::HashSet<_>>().len() == s.len()
            })
        }
        None => false,
    }
}

/// `extractPagesOk` (M:224).
fn extract_pages_ok(state: &Value) -> bool {
    at(state, &["extract", "pages"])
        .and_then(Value::as_array)
        .is_some_and(|p| p.len() == 1 && p[0].as_f64() == Some(1.0))
}

/// `decorateOk` (M:280).
fn decorate_ok(state: &Value, pages: f64, want_layers: bool) -> bool {
    let Some(d) = state.get("decorate") else {
        return false;
    };
    if num_at(d, &["stamps", "watermark"]) != Some(pages)
        || num_at(d, &["stamps", "header"]) != Some(pages)
    {
        return false;
    }
    if num_at(state, &["pages", "count"]) != Some(pages) {
        return false;
    }
    if at(d, &["labels"])
        .and_then(Value::as_array)
        .and_then(|l| l.first())
        != Some(&json!("i"))
    {
        return false;
    }
    if want_layers {
        let Some(layers) = d
            .get("layers")
            .and_then(Value::as_array)
            .filter(|l| l.len() >= 2)
        else {
            return false;
        };
        if layers[1].get("visible") != Some(&json!(false)) {
            return false;
        }
    }
    true
}

/// `decorateUndone` (M:293).
fn decorate_undone(state: &Value) -> bool {
    if at(state, &["decorate", "layersSaved"]).and_then(Value::as_bool) == Some(true) {
        return at(state, &["decorate", "layers"])
            .and_then(Value::as_array)
            .and_then(|l| l.get(1))
            .and_then(|l| l.get("visible"))
            == Some(&json!(true));
    }
    at(state, &["decorate", "labels"])
        .and_then(Value::as_array)
        .and_then(|l| l.first())
        == Some(&json!("1"))
}

/// `shapesOk` (M:298).
fn shapes_ok(state: &Value) -> bool {
    let a = |k: &str| num_at(state, &["annots", k]);
    a("Square") == Some(1.0)
        && a("Circle") == Some(1.0)
        && a("Line") == Some(1.0)
        && a("Ink") == Some(1.0)
        && a("Stamp") == Some(2.0)
        && at(state, &["sign", "subj"]) == Some(&json!("Signature appearance"))
}

/// `exportMagicOk` (M:308).
fn export_magic_ok(kind: &str, bytes: &[u8]) -> bool {
    match kind.trim_start_matches("export-") {
        "png" => bytes.len() >= 8 && bytes.starts_with(&[0x89, 0x50, 0x4e, 0x47]),
        "jpeg" => bytes.len() >= 3 && bytes.starts_with(&[0xff, 0xd8, 0xff]),
        "txt" => String::from_utf8_lossy(bytes).contains("PDF4All"),
        _ => false,
    }
}

/// `--smoke --open=<pdf> [--edit=…] [--save-to=… | --save-in-place | --print-to=…] [--expect=P1_THUMBS]`
/// (M:1764-2008; spec §1.5.2), driven by the lines the session queues.
pub fn smoke_open<D: Driven + ?Sized>(s: &mut D, args: &Args) -> Finish {
    let open_arg = args.open.clone().unwrap_or_default();
    let edit_arg = args.edit.clone();
    let save_to = args.save_to.clone();
    let in_place = args.save_in_place;
    let print_to = args.print_to.clone();
    let baseline = if save_to.is_some() || in_place {
        file_baseline(&open_arg)
    } else {
        None
    };
    let open_name = host::util::basename(Path::new(&open_arg));
    let (mut pushed, mut saving, mut printing, mut editing, mut undoing) =
        (false, false, false, false, false);
    let mut opened: Option<Value> = None;
    let mut edit: Option<Value> = None;
    let mem = metrics;
    let with = |pairs: &[(&str, Option<Value>)]| -> Value {
        let mut m = Map::new();
        for (k, v) in pairs {
            if let Some(v) = v {
                m.insert((*k).to_string(), v.clone());
            }
        }
        Value::Object(m)
    };
    s.start(Some(&args.fixture));
    let mut queue: VecDeque<(String, Value)> = s.take_lines().into();
    while let Some((kind, payload)) = queue.pop_front() {
        let whole = format!("{kind} {payload}");
        match kind.as_str() {
            "P0_ERROR" => return Finish::error(whole),
            "P1_SAVE_ERROR" if save_to.is_some() || in_place => return Finish::error(whole),
            "P3_EDIT" => {
                edit = Some(payload.get("content").cloned().unwrap_or(payload));
                if save_to.is_some() {
                    if !saving {
                        saving = true;
                        send(s, &mut queue, Event::SaveRequest(SaveRequest::Copy));
                    }
                    continue;
                }
                return Finish::new(
                    with(&[
                        ("open", opened.clone()),
                        ("content", edit.clone()),
                        ("memory", Some(mem())),
                    ]),
                    true,
                );
            }
            "P2_EDIT" => {
                let state = payload;
                let opened_count = opened.as_ref().and_then(|o| num_at(o, &["pageCount"]));
                let edit_name = edit_arg.as_deref().unwrap_or("");
                if undoing {
                    let undone = match edit_name {
                        "pages" | "assemble" => {
                            opened_count.is_some()
                                && num_at(&state, &["pages", "count"]) == opened_count
                        }
                        "highlight" => num_at(&state, &["annots", "Highlight"]) == Some(0.0),
                        "freetext" => num_at(&state, &["annots", "FreeText"]) == Some(0.0),
                        "form" => at(&state, &["fields", "values", "size"]) == Some(&json!("")),
                        "nav" => nav_undone(&state),
                        "decorate" => decorate_undone(&state),
                        "shapes" => num_at(&state, &["annots", "Stamp"]) == Some(1.0),
                        _ => num_at(&state, &["notes"]) == Some(0.0),
                    };
                    return Finish::new(
                        with(&[
                            ("open", opened.clone()),
                            ("edit", edit.clone()),
                            ("undo", Some(state)),
                            ("memory", Some(mem())),
                        ]),
                        undone,
                    );
                }
                if args
                    .smoke_passwords()
                    .iter()
                    .any(|pw| whole.contains(pw.as_str()))
                {
                    return Finish::new(
                        json!({"error": "password leaked in P2_EDIT", "edit": state}),
                        false,
                    );
                }
                edit = Some(state.clone());
                let fail = || {
                    Finish::new(
                        with(&[
                            ("open", opened.clone()),
                            ("edit", Some(state.clone())),
                            ("memory", Some(mem())),
                        ]),
                        false,
                    )
                };
                let perms_ok = at(&state, &["security", "permissions"])
                    .and_then(Value::as_array)
                    .is_some_and(|p| p.contains(&json!("print")) && p.contains(&json!("annotate")));
                let filled = at(&state, &["fields", "values"]);
                let bad = match edit_name {
                    "encrypt" => {
                        !(at(&state, &["security", "mode"]) == Some(&json!("aes-256")) && perms_ok)
                    }
                    "encrypt-keep" => at(&state, &["security", "mode"]) != Some(&json!("keep")),
                    "highlight" => num_at(&state, &["annots", "Highlight"]) != Some(1.0),
                    "pages" => {
                        !(opened_count
                            .is_some_and(|n| num_at(&state, &["pages", "count"]) == Some(n - 1.0))
                            && at(&state, &["pages", "rotate", "1"])
                                .or_else(|| at(&state, &["pages", "rotate"]).and_then(|r| r.get(1)))
                                .and_then(Value::as_f64)
                                == Some(90.0))
                    }
                    "form" => {
                        !(filled.is_some_and(|f| {
                            f.get("name") == Some(&json!("PDF4All P2 form"))
                                && f.get("agree") == Some(&json!(true))
                                && f.get("choice") == Some(&json!("two"))
                                && f.get("size") == Some(&json!("M"))
                        }))
                    }
                    "nav" => !nav_edit_ok(&state),
                    "assemble" => !opened_count.is_some_and(|n| assemble_ok(&state, n)),
                    "decorate" => !opened_count
                        .is_some_and(|n| decorate_ok(&state, n, open_name == "layers.pdf")),
                    "shapes" => !shapes_ok(&state),
                    _ => false,
                };
                if bad {
                    return fail();
                }
                if edit_name == "extract" {
                    if !extract_pages_ok(&state) {
                        return fail();
                    }
                    if let Some(target) = &save_to {
                        if file_size(target).is_some_and(|n| n >= 5) {
                            let ok = source_unchanged(&open_arg, &baseline);
                            return Finish::new(
                                with(&[
                                    ("open", opened.clone()),
                                    ("edit", edit.clone()),
                                    ("sourceUnchanged", Some(json!(ok))),
                                    ("memory", Some(mem())),
                                ]),
                                ok,
                            );
                        }
                        continue;
                    }
                    return Finish::new(
                        with(&[
                            ("open", opened.clone()),
                            ("edit", edit.clone()),
                            ("memory", Some(mem())),
                        ]),
                        true,
                    );
                }
                if let Some(want) = edit_name.strip_prefix("export-") {
                    if at(&state, &["export", "kind"]) != Some(&json!(want)) {
                        return fail();
                    }
                    let ok = save_to
                        .as_ref()
                        .and_then(|t| std::fs::read(t).ok())
                        .is_some_and(|b| export_magic_ok(edit_name, &b));
                    return Finish::new(
                        with(&[
                            ("open", opened.clone()),
                            ("edit", edit.clone()),
                            ("memory", Some(mem())),
                        ]),
                        ok,
                    );
                }
                // The renderer's encrypt smokes save by themselves.
                if edit_name == "encrypt" || edit_name == "encrypt-keep" {
                    continue;
                }
                if save_to.is_some() {
                    if !saving {
                        saving = true;
                        send(s, &mut queue, Event::SaveRequest(SaveRequest::Copy));
                    }
                    continue;
                }
                if in_place {
                    if !saving {
                        saving = true;
                        send(s, &mut queue, Event::SaveRequest(SaveRequest::InPlace));
                    }
                    continue;
                }
                undoing = true;
                send(
                    s,
                    &mut queue,
                    Event::EditRequest {
                        op: "undo".into(),
                        contents: None,
                    },
                );
            }
            "P1_SAVE" if save_to.is_some() => {
                let save = payload;
                let target = save_to.clone().unwrap_or_default();
                let source_ok = source_unchanged(&open_arg, &baseline);
                let size = file_size(&target);
                if edit_arg.as_deref() == Some("extract") {
                    let ok = extract_pages_ok(edit.as_ref().unwrap_or(&json!({})))
                        && size.is_some_and(|n| n >= 5)
                        && source_ok;
                    return Finish::new(
                        with(&[
                            ("open", opened.clone()),
                            ("edit", edit.clone()),
                            ("save", Some(save)),
                            ("sourceUnchanged", Some(json!(source_ok))),
                            ("memory", Some(mem())),
                        ]),
                        ok,
                    );
                }
                let name_ok =
                    save.get("name") == Some(&json!(host::util::basename(Path::new(&target))));
                let size_ok = size.is_some_and(|n| Some(n as f64) == num_at(&save, &["bytes"]));
                let ok =
                    name_ok && size_ok && source_ok && save.get("validated") == Some(&json!(true));
                let content = if edit_arg.as_deref() == Some("content") {
                    edit.clone()
                } else {
                    None
                };
                return Finish::new(
                    with(&[
                        ("open", opened.clone()),
                        ("edit", edit.clone()),
                        ("content", content),
                        ("save", Some(save)),
                        ("sourceUnchanged", Some(json!(source_ok))),
                        ("memory", Some(mem())),
                    ]),
                    ok,
                );
            }
            "P1_SAVE" if in_place => {
                use std::os::unix::fs::PermissionsExt;
                let save = payload;
                let changed = baseline
                    .as_ref()
                    .is_some_and(|(_, hash, _)| sha256_file(&open_arg).as_ref() != Some(hash));
                let mode_ok = baseline.as_ref().is_some_and(|(_, _, mode)| {
                    std::fs::metadata(&open_arg)
                        .is_ok_and(|m| m.permissions().mode() & 0o777 == mode & 0o777)
                });
                let ok = changed && mode_ok && save.get("validated") == Some(&json!(true));
                return Finish::new(
                    with(&[
                        ("open", opened.clone()),
                        ("edit", edit.clone()),
                        ("save", Some(save)),
                        ("memory", Some(mem())),
                    ]),
                    ok,
                );
            }
            "P1_PRINT" if print_to.is_some() => {
                if opened.is_none() {
                    continue;
                }
                let print = payload;
                let target = print_to.clone().unwrap_or_default();
                // `await printed`: the host's print has settled by now (the engine is synchronous).
                let outcome = s.host().take_print_result().unwrap_or(Err("failed"));
                let size = file_size(&target);
                let magic = std::fs::read(&target).is_ok_and(|b| b.starts_with(b"%PDF-"));
                let pages_ok = num_at(&print, &["pages"]).is_some_and(|n| n > 0.0);
                let ok = outcome.is_ok() && size.is_some_and(|n| n >= 1024) && magic && pages_ok;
                return Finish::new(
                    with(&[
                        ("open", opened.clone()),
                        ("print", Some(print)),
                        ("printedBytes", Some(json!(size.unwrap_or(0)))),
                        ("memory", Some(mem())),
                    ]),
                    ok,
                );
            }
            "P1_THUMBS" if args.expect_thumbs => {
                let thumbs = payload;
                if opened.is_none() || save_to.is_some() {
                    continue;
                }
                let ok = opened
                    .as_ref()
                    .and_then(|o| num_at(o, &["pageCount"]))
                    .is_some_and(|n| n >= 1.0);
                return Finish::new(
                    with(&[
                        ("open", opened.clone()),
                        ("thumbs", Some(thumbs)),
                        ("memory", Some(mem())),
                    ]),
                    ok,
                );
            }
            "P1_RESULT" => {
                let open = payload;
                if !pushed {
                    pushed = true;
                    s.host().deliver_open(Some(Path::new(&open_arg)), false);
                    pump(s);
                    queue.extend(s.take_lines());
                    continue;
                }
                if open.get("name") != Some(&json!(open_name)) {
                    continue;
                }
                opened = Some(open.clone());
                if edit_arg.is_some() {
                    if !editing {
                        editing = true;
                        let (op, contents) = args.smoke_edit_payload();
                        send(s, &mut queue, Event::EditRequest { op, contents });
                    }
                    continue;
                }
                if in_place {
                    if !saving {
                        saving = true;
                        send(s, &mut queue, Event::SaveRequest(SaveRequest::InPlace));
                    }
                    continue;
                }
                if save_to.is_some() {
                    if !saving {
                        saving = true;
                        send(s, &mut queue, Event::SaveRequest(SaveRequest::Copy));
                    }
                    continue;
                }
                if print_to.is_some() {
                    if !printing {
                        printing = true;
                        let range = args.print_range.clone().unwrap_or_default();
                        send(s, &mut queue, Event::PrintRequest { range: Some(range) });
                    }
                    continue;
                }
                if args.expect_thumbs {
                    continue;
                }
                let ok = num_at(&open, &["pageCount"]).is_some_and(|n| n >= 1.0);
                return Finish::new(json!({"open": open, "memory": mem()}), ok);
            }
            _ => {}
        }
    }
    // Electron waited 120 s here; nothing more can arrive in a synchronous session.
    Finish::error("smoke timeout")
}

/// `--smoke --new-from-images --images=… --save-to=…` (M:2009-2028).
pub fn smoke_new_from_images<D: Driven + ?Sized>(s: &mut D, args: &Args) -> Finish {
    s.start(Some(&args.fixture));
    let mut queue: VecDeque<(String, Value)> = s.take_lines().into();
    let mut sent = false;
    while let Some((kind, payload)) = queue.pop_front() {
        match kind.as_str() {
            "P0_ERROR" => return Finish::error(format!("{kind} {payload}")),
            "P2_NEW" => {
                let target = args.save_to.as_deref().and_then(file_size);
                let ok = num_at(&payload, &["pages"]).is_some_and(|n| n >= 1.0)
                    && payload.get("bytes").is_some_and(Value::is_number)
                    && target.is_some_and(|n| n >= 5);
                return Finish::new(json!({"new": payload, "memory": metrics()}), ok);
            }
            "P0_RESULT" if !sent => {
                sent = true;
                send(
                    s,
                    &mut queue,
                    Event::EditRequest {
                        op: "new-from-images".into(),
                        contents: None,
                    },
                );
            }
            _ => {}
        }
    }
    Finish::error("smoke timeout")
}

/// The P0 round trip (worker.ts `roundtrip`), natively on two fresh engines:
/// a Square annotation and the `name` field's value on form.pdf, saved and
/// read back from a fresh open of the saved bytes. Returns the report part and
/// the saved bytes.
pub fn roundtrip() -> (Value, Vec<u8>) {
    use crate::engine::EngineHost;
    let fail = |error: String| {
        (
            json!({"engine": "rust", "ok": false, "error": error}),
            Vec::new(),
        )
    };
    let Some(pdf) = host::fixture_path("form.pdf").and_then(|p| std::fs::read(p).ok()) else {
        return fail("form HTTP 404".into());
    };
    if pdf.len() < 5 || pdf.len() > 1_000_000 {
        return fail("PDF exceeds size bound".into());
    }
    let mut edited = EngineHost::new();
    let mut again = EngineHost::new();
    let call = |engine: &mut EngineHost, request: Value, inputs: Vec<Vec<u8>>| {
        let op = request
            .get("op")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        engine
            .request(request, inputs)
            .map_err(|e| format!("{op}: {}", e.message))
    };
    let run = (|| -> Result<(Value, Vec<u8>), String> {
        call(
            &mut edited,
            json!({"op": "open", "pdf": {"$buffer": 0}, "name": "form.pdf"}),
            vec![pdf],
        )?;
        let fields = call(&mut edited, json!({"op": "fields", "page": 0}), vec![])?;
        let widget = fields
            .get("items")
            .and_then(Value::as_array)
            .and_then(|items| items.iter().find(|i| i.get("name") == Some(&json!("name"))))
            .and_then(|i| i.get("id").cloned())
            .ok_or("missing name field")?;
        call(
            &mut edited,
            json!({"op": "annot-add", "page": 0, "type": "Square", "rect": [100, 100, 300, 200], "contents": "PDF4All P0 note"}),
            vec![],
        )?;
        call(
            &mut edited,
            json!({"op": "field-set", "page": 0, "widgetId": widget, "value": "PDF4All P0"}),
            vec![],
        )?;
        let saved = call(&mut edited, json!({"op": "save"}), vec![])?
            .take_bytes("bytes")
            .ok_or("save: no bytes")?;
        call(
            &mut again,
            json!({"op": "open", "pdf": {"$buffer": 0}, "name": "roundtrip.pdf"}),
            vec![saved.clone()],
        )?;
        let annots = call(&mut again, json!({"op": "annots", "page": 0}), vec![])?;
        let squares: Vec<Value> = annots
            .get("items")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter(|a| a.get("type") == Some(&json!("Square")))
                    .map(|a| json!({"type": a["type"], "rect": a["rect"], "contents": a["contents"]}))
                    .collect()
            })
            .unwrap_or_default();
        let fields = call(&mut again, json!({"op": "fields", "page": 0}), vec![])?;
        let value = fields
            .get("items")
            .and_then(Value::as_array)
            .and_then(|items| items.iter().find(|i| i.get("name") == Some(&json!("name"))))
            .and_then(|i| i.get("value").cloned())
            .unwrap_or(json!(""));
        let report = json!({"engine": "rust", "ok": true, "savedSize": saved.len(), "readback": {"annots": squares, "fieldValue": value}});
        Ok((report, saved))
    })();
    let _ = edited.request(json!({"op": "close"}), vec![]);
    let _ = again.request(json!({"op": "close"}), vec![]);
    run.unwrap_or_else(fail)
}

/// Plain `--smoke` (M:2029-2102): the sample's first page, then the native
/// equivalents of Electron's probes. What each check is here:
/// - `renderer`: the `P0_RESULT` payload; `helloOk` as in Electron (612 × 792
///   points, text containing `PDF4All`).
/// - `isolation`: `nodeAbsent` is true (no JavaScript runtime at all);
///   `width`/`height`/`nonwhitePixels` are the first page's pixels as the
///   session rendered them (no canvas or `canvasCount`).
/// - `sandbox`: whether the process runs under the macOS App Sandbox. It is
///   reported, not required: the headless core is not a sandboxed renderer.
/// - `networkProbeDenied` / `noUnexpectedRequests`: true, because the core
///   contains no network code (nothing to probe); the `workerProbe` and CSP
///   checks have no native counterpart and are omitted.
/// - `roundtrip`: worker.ts's round trip on `form.pdf`, on the native engine.
///
/// `ok` = the first page is right and non-blank, and the round trip read back
/// what it wrote. `--report` also writes `<report>.roundtrip.pdf` (there is no
/// window to capture as `<report>.png`).
pub fn smoke_plain<D: Driven + ?Sized>(s: &mut D, args: &Args) -> Finish {
    s.start(Some(&args.fixture));
    for (kind, payload) in s.take_lines() {
        if kind == "P0_ERROR" {
            return Finish::error(format!("{kind} {payload}"));
        }
        if kind != "P0_RESULT" {
            continue;
        }
        // M10 startup: when the first page was drawn, since this process began and as wall-clock time.
        let elapsed = started().elapsed().as_secs_f64() * 1000.0;
        let startup = json!({"firstPageAtMs": elapsed.ceil().max(1.0) as u64, "firstPageEpochMs": crate::engine::now_ms() as u64, "mainStartedAtMs": 0});
        let renderer = payload;
        let (width, height) = (
            num_at(&renderer, &["pixelWidth"]),
            num_at(&renderer, &["pixelHeight"]),
        );
        let non_white = num_at(&renderer, &["nonWhite"]);
        let isolation = json!({"nodeAbsent": true, "width": width, "height": height, "nonwhitePixels": non_white});
        let nonblank = match (width, height, non_white) {
            (Some(w), Some(h), Some(n)) => n > 100.0 && n < w * h / 2.0,
            _ => false,
        };
        let hello_ok = num_at(&renderer, &["width"]) == Some(612.0)
            && num_at(&renderer, &["height"]) == Some(792.0)
            && renderer.to_string().contains("PDF4All");
        let (roundtrip, saved) = roundtrip();
        let square = at(&roundtrip, &["readback", "annots"])
            .and_then(Value::as_array)
            .is_some_and(|a| {
                a.iter().any(|x| {
                    x.get("type") == Some(&json!("Square"))
                        && x.get("contents") == Some(&json!("PDF4All P0 note"))
                })
            });
        let roundtrip_ok = roundtrip.get("ok") == Some(&json!(true))
            && at(&roundtrip, &["readback", "fieldValue"]) == Some(&json!("PDF4All P0"))
            && square;
        if let Some(report) = &args.report
            && !saved.is_empty()
        {
            let _ = write_file(&format!("{report}.roundtrip.pdf"), &saved);
        }
        let ok = hello_ok && nonblank && roundtrip_ok;
        let result = json!({
            "engine": "rust", "renderer": renderer, "startup": startup, "isolation": isolation, "sandbox": sandboxed(),
            "noUnexpectedRequests": true, "networkProbeDenied": true, "roundtrip": roundtrip, "fixture": args.fixture,
            "openMs": renderer.get("openMs").cloned().unwrap_or(Value::Null),
            "firstPageRenderMs": renderer.get("firstPageRenderMs").cloned().unwrap_or(Value::Null),
            "memory": metrics(),
        });
        return Finish::new(result, ok);
    }
    Finish::error("smoke timeout")
}

// ------------------------------------------------------------------ not wired yet

/// A stand-in session for the binary until `session::Session` implements
/// [`Driven`]: the startup fails with `P0_ERROR`, so every run ends cleanly.
pub struct Unwired {
    host: Host,
    lines: Vec<(String, Value)>,
}

impl Unwired {
    pub fn new(host: Host) -> Self {
        Unwired {
            host,
            lines: Vec::new(),
        }
    }
}

impl Driven for Unwired {
    fn start(&mut self, _fixture: Option<&str>) {
        self.lines.push((
            "P0_ERROR".into(),
            json!({"message": "the document session is not wired into tandem-headless yet"}),
        ));
    }
    fn deliver_document(&mut self, _doc: Document) {}
    fn open_error(&mut self, _error: OpenError) {}
    fn save_request(&mut self, _request: SaveRequest) {}
    fn edit_request(&mut self, _op: &str, _contents: Option<String>) {}
    fn print_request(&mut self, _range: Option<String>) {}
    fn state(&mut self) -> Value {
        json!({})
    }
    fn take_lines(&mut self) -> Vec<(String, Value)> {
        std::mem::take(&mut self.lines)
    }
    fn host(&mut self) -> &mut Host {
        &mut self.host
    }
}

#[cfg(test)]
mod tests;
