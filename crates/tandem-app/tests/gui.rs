//! The window checks of the Electron GUI tests, at the session level.
//!
//! Electron's GUI tests drove a real window over the DevTools protocol: real key presses,
//! clicks on the start screen, computed styles. In the Rust app the window (crates/tandem-ui)
//! only draws `Session::ui_state()` and sends `UiAction`s; the session behind it is what the
//! Electron renderer was. So these tests send the same `UiAction`s the window sends
//! (`Session::ui_act`), mixed with drive lines for set-up (`DriveRun::line`, as the JS mixed
//! `--drive=-` lines with real input), and read the drive STATE and the view model back.
//!
//! The session is synchronous (one core thread, crates/tandem-desktop/src/core.rs): a Save
//! sent "while a ⌘Enter commit is in flight" runs after it, so the Electron races cannot happen;
//! the sequential tests below still pin the outcome (the saved file has the text).
//!
//! A few checks can only be source checks (computed styles, accessible names, menu labels):
//! those read crates/tandem-ui and crates/tandem-desktop sources with `include_str!` and say so.
// Some helpers serve only the Unix tests (modes, links).
#![cfg_attr(not(unix), allow(unused_imports, dead_code))]

use serde_json::{Value, json};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tandem_app::drive::{DriveRun, Driven, Outcome};
use tandem_app::host::{Host, HostConfig};
use tandem_app::platform::{
    Answers, Filter, MessageBox, Pick, Platform, PrintOutcome, SavePurpose, Scripted,
};
use tandem_app::session::Session;
use tandem_protocol::ui::UiAction;

// ------------------------------------------------------------------ helpers

/// A temp folder, removed when dropped.
struct Tmp(PathBuf);

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tmp(tag: &str) -> Tmp {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "tandem-t-gui-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    Tmp(std::fs::canonicalize(&dir).unwrap())
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name)
}

/// A copy of a fixture in `dir` (nothing here may write into fixtures/).
fn copy_fixture(dir: &Path, name: &str) -> PathBuf {
    let to = dir.join(Path::new(name).file_name().unwrap());
    std::fs::copy(fixture(name), &to).unwrap();
    to
}

/// What the native alerts were asked, shared with the test.
type Asked = Arc<Mutex<Vec<MessageBox>>>;

/// The window's platform for a test: the scripted pickers (pre-answers), and alerts answered
/// by button label from a queue. An alert with no queued answer gets its Cancel button, never
/// its default (an unanswered "Save" default once wrote over a fixture).
struct Answering {
    inner: Scripted,
    buttons: Arc<Mutex<VecDeque<String>>>,
    asked: Asked,
}

impl Platform for Answering {
    fn pick_file(&mut self, pick: Pick, filters: &[Filter]) -> Option<PathBuf> {
        self.inner.pick_file(pick, filters)
    }
    fn pick_images(&mut self) -> Vec<PathBuf> {
        self.inner.pick_images()
    }
    fn pick_folder(&mut self) -> Option<PathBuf> {
        self.inner.pick_folder()
    }
    fn pick_save(
        &mut self,
        purpose: SavePurpose,
        default_path: &Path,
        filters: &[Filter],
    ) -> Option<PathBuf> {
        self.inner.pick_save(purpose, default_path, filters)
    }
    fn ask(&mut self, question: &MessageBox) -> usize {
        self.asked.lock().unwrap().push(question.clone());
        let wanted = self.buttons.lock().unwrap().pop_front();
        wanted
            .and_then(|label| question.buttons.iter().position(|b| *b == label))
            .unwrap_or(question.cancel)
    }
    fn open_external(&mut self, uri: &str) {
        self.inner.open_external(uri);
    }
    fn print(&mut self, pdf: &[u8], job_name: &str) -> PrintOutcome {
        self.inner.print(pdf, job_name)
    }
    fn add_recent_document(&mut self, path: &Path) {
        self.inner.add_recent_document(path);
    }
    fn clear_recent_documents(&mut self) {
        self.inner.clear_recent_documents();
    }
    fn set_represented_file(&mut self, path: Option<&Path>, title: &str) {
        self.inner.set_represented_file(path, title);
    }
    fn set_document_state(&mut self, edited: bool, save: bool, save_as: bool) {
        self.inner.set_document_state(edited, save, save_as);
    }
    fn answers(&mut self) -> Option<&mut Answers> {
        self.inner.answers()
    }
}

/// A window on the startup sample (hello.pdf), like the JS `session()`: drive lines for
/// set-up, `UiAction`s for what a person does. `headless` is off once the sample is open,
/// so the session behaves as the window's (native questions, the dialog paths).
struct Gui {
    s: Session,
    run: DriveRun,
    dir: Tmp,
    buttons: Arc<Mutex<VecDeque<String>>>,
    asked: Asked,
}

impl Gui {
    fn new(tag: &str) -> Gui {
        let dir = tmp(tag);
        let buttons = Arc::new(Mutex::new(VecDeque::new()));
        let asked: Asked = Arc::default();
        let platform = Answering {
            inner: Scripted::new(Answers::default()),
            buttons: buttons.clone(),
            asked: asked.clone(),
        };
        let host = Host::new(
            HostConfig {
                user_data: dir.0.join("udata"),
                smoke: true,
                echo_stdout: false,
            },
            Box::new(platform),
        );
        let mut s = Session::for_drive(host);
        let run = DriveRun::start(&mut s, "hello.pdf", &dir.0.to_string_lossy()).unwrap();
        s.headless = false;
        s.ui_pump();
        Gui {
            s,
            run,
            dir,
            buttons,
            asked,
        }
    }

    /// One drive line; its STATE.
    fn step(&mut self, line: Value) -> Value {
        match self.run.line(&mut self.s, line) {
            Outcome::Record(record) => {
                // The window pumps after every action; a drive line is one too.
                self.s.ui_pump();
                assert!(record.get("error").is_none(), "drive step failed: {record}");
                record["state"].clone()
            }
            other => panic!("the drive ended: {other:?}"),
        }
    }

    /// The drive STATE now.
    fn state(&mut self) -> Value {
        Driven::state(&mut self.s)
    }

    fn act(&mut self, action: UiAction) {
        self.s.ui_act(action);
    }

    fn key(&mut self, key: &str) {
        self.act(UiAction::Key {
            key: key.into(),
            shift: false,
            alt: false,
            meta: false,
        });
    }

    fn tool(&mut self, id: &str) {
        self.act(UiAction::Tool {
            id: id.into(),
            variant: None,
        });
    }

    fn click(&mut self, id: &str) {
        self.act(UiAction::Click { id: id.into() });
    }

    fn input(&mut self, id: &str, value: &str) {
        self.act(UiAction::Input {
            id: id.into(),
            value: value.into(),
        });
    }

    fn pointer(&mut self, phase: &str, x: f64, y: f64) {
        self.act(UiAction::Pointer {
            phase: phase.into(),
            page: 0,
            x,
            y,
            shift: false,
            alt: false,
            meta: false,
        });
    }

    /// A click on page 1 (down and up at one point).
    fn press_at(&mut self, x: f64, y: f64) {
        self.pointer("down", x, y);
        self.pointer("up", x, y);
    }

    fn ui_tool(&mut self) -> String {
        self.s.ui_state().view.tool.id
    }

    fn dialog(&mut self) -> Option<String> {
        self.s.ui_state().view.dialog.map(|d| d.id)
    }

    fn answer(&self, button: &str) {
        self.buttons.lock().unwrap().push_back(button.into());
    }
}

/// A real window's session with no sample: the start screen over `user_data` on disk, as a
/// normal launch (crates/tandem-desktop/src/session_core.rs).
fn launch(user_data: &Path) -> Session {
    let platform = Answering {
        inner: Scripted::new(Answers::default()),
        buttons: Arc::default(),
        asked: Arc::default(),
    };
    let host = Host::new(
        HostConfig {
            user_data: user_data.to_path_buf(),
            smoke: false,
            echo_stdout: false,
        },
        Box::new(platform),
    );
    let mut s = Session::with_host(host);
    s.start(None);
    s.ui_pump();
    s
}

/// The open document's matches for `text` (the window's search).
fn found_in_doc(g: &mut Gui, text: &str) -> u64 {
    g.act(UiAction::Search { query: text.into() });
    g.state()["search"]["count"].as_u64().unwrap_or(0)
}

/// How many times `text` is found in the PDF at `path` (a fresh window opens it and searches).
fn found_in(path: &Path, text: &str) -> u64 {
    let mut g = Gui::new("found");
    let st = g.step(json!({"cmd": "open", "path": path}));
    assert_eq!(
        st["doc"]["name"],
        json!(path.file_name().unwrap().to_string_lossy())
    );
    found_in_doc(&mut g, text)
}

/// The engine tests' `classic` PDF (engine/tests/app_redact_boxes.rs), objects numbered from 1.
fn classic_pdf(objects: &[&str]) -> Vec<u8> {
    let mut text = String::from("%PDF-1.7\n");
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(text.len());
        text.push_str(&format!("{} 0 obj\n{body}\nendobj\n", i + 1));
    }
    let xref = text.len();
    text.push_str(&format!(
        "xref\n0 {}\n0000000000 65535 f \n",
        objects.len() + 1
    ));
    for offset in offsets {
        text.push_str(&format!("{offset:010} 00000 n \n"));
    }
    text.push_str(&format!(
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        objects.len() + 1
    ));
    text.into_bytes()
}

fn pdf_stream(dict: &str, content: &str) -> String {
    format!(
        "<< {dict} /Length {} >>\nstream\n{content}\nendstream",
        content.len()
    )
}

fn courier() -> String {
    format!(
        "<< /Type /Font /Subtype /Type1 /BaseFont /Courier /FirstChar 32 /LastChar 126 /Widths [{}] /Encoding /WinAnsiEncoding >>",
        vec!["600"; 95].join(" ")
    )
}

/// Every `.rs` file under `dir`, as (path, text).
fn sources(dir: &Path) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).unwrap();
                out.push((path, text));
            }
        }
    }
    out.sort();
    out
}

// ------------------------------------------------------------------ Escape and the editors

// was tests/p1-ux-gui.test.mjs: 'p1-ux GUI: chrome is not selectable but content is; Esc keeps a
// typed note; the inspector Sign opens the chooser' (review fix 10: the real Escape behaves as
// the drive's).
#[test]
fn one_escape_after_a_committed_add_text_stops_the_tool() {
    let mut g = Gui::new("addtext");
    g.step(json!({"cmd": "new"}));
    g.tool("text");
    assert_eq!(
        g.state()["tool"],
        "Adding text: click empty space on the page"
    );
    g.press_at(300.0, 400.0);
    assert_eq!(g.state()["direct"]["active"]["kind"], "add");
    g.act(UiAction::EditorInput {
        value: "Typed for real".into(),
    });
    // ⌘Enter in the window's editor is EditorCommit.
    g.act(UiAction::EditorCommit);
    let st = g.state();
    assert_eq!(st["revision"], 1);
    assert_eq!(st["tool"], "Adding text: click empty space on the page");
    g.key("Escape");
    assert_eq!(
        g.state()["tool"],
        Value::Null,
        "one real Escape stopped the tool"
    );
    assert_eq!(g.ui_tool(), "select", "and the strip is back on Select");
    assert_eq!(found_in_doc(&mut g, "Typed for real"), 1);
}

// was tests/p1-ux-gui.test.mjs: 'p1-ux GUI: chrome is not selectable … Esc keeps a typed note …'
// (review fix 8: a real Escape keeps a note that has text, and discards an empty one).
#[test]
fn escape_keeps_a_typed_note_and_discards_an_empty_one() {
    let mut g = Gui::new("note");
    g.step(json!({"cmd": "new"}));
    g.tool("note");
    g.press_at(300.0, 400.0);
    assert_eq!(g.state()["direct"]["active"]["kind"], "note");
    g.act(UiAction::EditorInput {
        value: "Typed, then Esc".into(),
    });
    g.key("Escape");
    let st = g.state();
    assert_eq!(st["annots"]["Text"], 1, "{}", st["direct"]);
    assert_eq!(st["comments"][0]["contents"], "Typed, then Esc");
    assert_eq!(st["direct"]["active"], Value::Null);
    // The note tool stays ready (§3.2: placing tools stay armed).
    assert_eq!(g.ui_tool(), "note");
    g.press_at(300.0, 500.0);
    assert_eq!(g.state()["direct"]["active"]["kind"], "note");
    g.key("Escape");
    let st = g.state();
    assert_eq!(st["annots"]["Text"], 1, "the empty note was discarded");
    assert_eq!(st["direct"]["active"], Value::Null);
}

// was tests/p1-ux-gui.test.mjs: 'p1-ux GUI: … the inspector Sign opens the chooser' (review fix 9)
// and tests/p1-ux.test.mjs: 'item 9: one Sign entry point opens a chooser; the certificate button
// is "Sign and save as…" and starts disabled' (never a silent no-op).
#[test]
fn every_sign_button_opens_the_one_chooser() {
    let mut g = Gui::new("sign");
    g.tool("fill-sign");
    // The tool options' "Sign…" and the side panel's "Sign…" both send this click
    // (crates/tandem-ui/src/editor.rs, panel.rs).
    g.click("fill-sign");
    let dialog = g.s.ui_state().view.dialog.expect("the chooser");
    assert_eq!(dialog.id, "sign-choice-dialog");
    for id in ["sign-choice-picture", "sign-choice-certificate"] {
        assert!(dialog.nodes.contains_key(id), "{id}: {:?}", dialog.nodes);
    }
    g.click("sign-choice-certificate");
    let st = g.state();
    assert_eq!(
        st["sign"]["certificateDialog"],
        json!({"ready": false, "label": "Sign and save as…"})
    );
    assert_eq!(g.dialog().as_deref(), Some("cert-sign-dialog"));
    // Pressed anyway (a disabled button cannot be, but the core does not rely on that): it
    // says what to do, and signs nothing.
    g.click("cert-sign-go");
    assert_eq!(
        g.s.dom.text("cert-sign-status"),
        "Choose a certificate first."
    );
    assert_eq!(g.state()["sign"]["last"], Value::Null);
    g.click("cert-sign-cancel");
    assert_eq!(g.dialog(), None);
    // The picture choice with nothing saved yet opens the pad.
    g.click("fill-sign");
    g.click("sign-choice-picture");
    assert_eq!(g.dialog().as_deref(), Some("sigpad-dialog"));
}

// ------------------------------------------------------------------ the start screen

// was tests/p1-ux-gui.test.mjs: 'p1-ux GUI: the start screen lists recent files, redraws after a
// missing one, and clears them'; tests/p1-ux.test.mjs: 'review fix 3: a recent entry is opened by
// its id, so a list drawn before an entry was dropped never opens another file' (the window opens
// an entry by its path, so a list drawn before an entry was dropped still opens its own file).
#[test]
fn the_start_screen_lists_recent_files_drops_a_missing_one_and_clears_them() {
    let dir = tmp("recent");
    let nav = copy_fixture(&dir.0, "nav.pdf");
    let form = copy_fixture(&dir.0, "form.pdf");
    let missing = dir.0.join("no-such-file.pdf");
    let udata = dir.0.join("udata");
    std::fs::create_dir_all(&udata).unwrap();
    std::fs::write(
        udata.join("recent-files.json"),
        serde_json::to_string(&[&nav, &missing, &form]).unwrap(),
    )
    .unwrap();
    let mut s = launch(&udata);
    let drawn = s.ui_state().view;
    assert_eq!(drawn.screen, "start");
    let names: Vec<String> = drawn.recent.iter().map(|r| r.name.clone()).collect();
    assert_eq!(names, ["nav.pdf", "no-such-file.pdf", "form.pdf"]);
    // The missing file says so, leaves the list, and the list is redrawn.
    s.ui_act(UiAction::Open {
        path: Some(drawn.recent[1].path.clone()),
    });
    let view = s.ui_state().view;
    assert_eq!(view.screen, "start");
    let names: Vec<String> = view.recent.iter().map(|r| r.name.clone()).collect();
    assert_eq!(names, ["nav.pdf", "form.pdf"]);
    let notice = view.notice.expect("the missing file is named");
    assert_eq!(notice.kind, "error");
    assert!(
        notice.text.starts_with("no-such-file.pdf can't be found.")
            && notice.text.contains("removed from Recent Files"),
        "{}",
        notice.text
    );
    // The row drawn before the drop still opens its own file (an index would name form.pdf).
    s.ui_act(UiAction::Open {
        path: Some(drawn.recent[2].path.clone()),
    });
    let view = s.ui_state().view;
    assert_eq!(view.screen, "editor");
    assert_eq!(view.doc.map(|d| d.name).as_deref(), Some("form.pdf"));
    // The list persists, newest first, without the missing file.
    let saved: Vec<PathBuf> =
        serde_json::from_str(&std::fs::read_to_string(udata.join("recent-files.json")).unwrap())
            .unwrap();
    assert_eq!(saved, [form.clone(), nav.clone()]);
    drop(s);

    // Review fix 15: Clear empties the list (the start screen then has none to show).
    let mut again = launch(&udata);
    assert_eq!(again.ui_state().view.recent.len(), 2);
    again.ui_act(UiAction::Click {
        id: "start-recent-clear".into(),
    });
    assert!(again.ui_state().view.recent.is_empty());
    assert_eq!(
        std::fs::read_to_string(udata.join("recent-files.json")).unwrap(),
        "[]"
    );
}

// was tests/p1-ux.test.mjs: 'X4: a normal launch shows the start screen; a smoke, a drive or
// --sample opens the sample' (and the start screen's own Open and New).
#[test]
fn a_normal_launch_shows_the_start_screen_and_a_drive_opens_the_sample() {
    let dir = tmp("launch");
    let mut s = launch(&dir.0.join("udata"));
    let view = s.ui_state().view;
    assert_eq!(view.screen, "start");
    assert!(view.doc.is_none());
    assert!(view.recent.is_empty());
    // Open… with the panel cancelled stays on the start screen and says nothing.
    s.ui_act(UiAction::Open { path: None });
    let view = s.ui_state().view;
    assert_eq!(view.screen, "start");
    assert!(view.notice.is_none(), "{:?}", view.notice);
    // New blank PDF opens an Untitled document.
    s.ui_act(UiAction::New { size: None });
    let view = s.ui_state().view;
    assert_eq!(view.screen, "editor");
    assert_eq!(view.doc.map(|d| d.name).as_deref(), Some("Untitled.pdf"));
    // A drive (and a smoke) starts on the sample.
    let mut g = Gui::new("launch-drive");
    let view = g.s.ui_state().view;
    assert_eq!(view.screen, "editor");
    assert_eq!(view.doc.map(|d| d.name).as_deref(), Some("hello.pdf"));
}

// ------------------------------------------------------------------ notices

// was tests/p1-ux-gui.test.mjs: "p1-ux GUI: a signed file's notice says it opens for reading".
#[test]
fn a_signed_files_notice_says_it_opens_for_reading() {
    let mut g = Gui::new("signed");
    let signed = copy_fixture(&g.dir.0, "p0b/signed-intact.pdf");
    let st = g.step(json!({"cmd": "open", "path": signed}));
    assert_eq!(st["doc"]["readOnly"], true);
    let banner = g.s.ui_state().view.banner.expect("the signed notice");
    assert_eq!(banner.kind, "warning");
    assert!(
        banner
            .text
            .starts_with("This file is signed, so it opens for reading"),
        "{}",
        banner.text
    );
    let ids: Vec<&str> = banner.actions.iter().map(|(id, _)| id.as_str()).collect();
    assert!(ids.contains(&"verify-open"), "{ids:?}");
}

// was tests/p2-shell.test.mjs: 'the document notice can be closed: it stays closed for that text
// and returns for a new document'.
#[test]
fn the_document_notice_can_be_closed_and_returns_for_a_new_document() {
    let mut g = Gui::new("dismiss");
    let signed = copy_fixture(&g.dir.0, "p0b/signed-intact.pdf");
    g.step(json!({"cmd": "open", "path": signed}));
    let text = g.s.ui_state().view.banner.expect("a notice").text;
    g.act(UiAction::Notice {
        action: "banner-dismiss".into(),
    });
    assert!(g.s.ui_state().view.banner.is_none(), "closed");
    // Moving around keeps it closed.
    g.act(UiAction::Zoom {
        to: "in".into(),
        scale: None,
    });
    assert!(g.s.ui_state().view.banner.is_none(), "stays closed");
    g.step(json!({"cmd": "open", "path": signed}));
    let again =
        g.s.ui_state()
            .view
            .banner
            .expect("back for the next document");
    assert_eq!(again.text, text);
}

// ------------------------------------------------------------------ Escape during a drag

/// The move grip of the first text run on page 1 (in page points) and its target id: just
/// outside the run's top-left corner (crates/tandem-ui/src/viewport.rs `handles`).
fn text_grip(g: &mut Gui) -> ((f64, f64), String) {
    let scale = g.s.ui_scale();
    let view = g.s.ui_state().view;
    let run = view.overlays[0]
        .targets
        .iter()
        .find(|t| t.kind == "text" && t.handles.iter().any(|h| h == "grip"))
        .expect("a movable text run")
        .clone();
    (
        (run.rect[0] - 8.0 / scale, run.rect[1] - 8.0 / scale),
        run.id,
    )
}

/// A blank page with "Drag me", the Text tool on, the run focused and its grip pressed and
/// dragged 60 x 30 points (not released). Returns the release point.
fn drag_a_run(tag: &str) -> (Gui, (f64, f64)) {
    let mut g = Gui::new(tag);
    g.step(json!({"cmd": "new"}));
    let st = g.step(json!({"cmd": "text-add", "contents": {"page": 0, "point": [72, 120], "text": "Drag me", "font": "Helvetica", "size": 18, "color": [0, 0, 0]}}));
    assert_eq!(st["revision"], 1);
    g.tool("text");
    assert_eq!(
        g.state()["tool"],
        "Adding text: click empty space on the page"
    );
    let (grip, id) = text_grip(&mut g);
    // The window sends Focus when Tab or VoiceOver reaches the run; its handles then take the
    // pointer (the window sends no hover moves, only moves during a press).
    g.act(UiAction::Focus { target: id });
    g.pointer("down", grip.0, grip.1);
    for i in 1..=6 {
        let i = f64::from(i);
        g.pointer("move", grip.0 + i * 10.0, grip.1 + i * 5.0);
    }
    assert!(g.s.escape_owned(), "the drag is live");
    (g, (grip.0 + 60.0, grip.1 + 30.0))
}

// was tests/p1-ux-gui.test.mjs: 'p1-ux GUI: Escape during a text drag cancels the drag, even with
// a tool armed (re-review fix 2)'.
#[test]
fn escape_during_a_text_drag_cancels_the_drag() {
    let (mut g, release) = drag_a_run("drag-esc");
    g.key("Escape");
    g.pointer("up", release.0, release.1);
    let st = g.state();
    assert_eq!(
        st["revision"], 1,
        "Escape cancelled the drag: nothing moved"
    );
    assert_eq!(
        st["tool"], "Adding text: click empty space on the page",
        "the drag took the Escape, not the tool"
    );
    assert_eq!(
        st["direct"]["active"],
        Value::Null,
        "releasing the cancelled drag is not a click: no add-text editor opened"
    );
    // A pressed run that stayed selected: Escape peels the selection first (as the Electron
    // test, only when there is one), then stops the tool.
    if st["direct"]["selection"]
        .as_array()
        .is_some_and(|s| !s.is_empty())
    {
        g.key("Escape");
        let st = g.state();
        assert_eq!(st["direct"]["selection"], json!([]));
        assert_eq!(st["tool"], "Adding text: click empty space on the page");
    }
    g.key("Escape");
    assert_eq!(
        g.state()["tool"],
        Value::Null,
        "the next Escape stops the tool"
    );
}

/// The same drag without Escape moves the run: the test above passes on the Escape, not on a
/// drag that never started.
#[test]
fn a_text_drag_without_escape_moves_the_run() {
    let (mut g, release) = drag_a_run("drag-moves");
    g.pointer("up", release.0, release.1);
    assert_eq!(g.state()["revision"], 2, "the run moved");
}

// ------------------------------------------------------------------ Save after ⌘Enter

// was tests/p1-ux-gui.test.mjs: 'p1-ux GUI: Save right after ⌘Enter waits for that commit
// (re-review fix 3)'. The Rust session is synchronous, so Save cannot overtake the commit; the
// outcome is still pinned: the saved file has the committed text.
#[test]
fn save_right_after_a_commit_keeps_the_committed_text() {
    let mut g = Gui::new("commit-save");
    g.step(json!({"cmd": "new"}));
    let path = g.dir.0.join("commit-then-save.pdf");
    g.step(json!({"cmd": "save-to", "path": path}));
    let st = g.step(json!({"cmd": "open", "path": path}));
    assert_eq!(st["doc"]["name"], "commit-then-save.pdf");
    g.tool("text");
    g.press_at(100.0, 200.0);
    assert_eq!(g.state()["direct"]["active"]["kind"], "add");
    g.act(UiAction::EditorInput {
        value: "Committed before the save".into(),
    });
    g.act(UiAction::EditorCommit);
    g.act(UiAction::Save {
        how: "in-place".into(),
    });
    let st = g.state();
    assert_eq!(st["save"]["ok"], true, "{}", st["save"]);
    assert_eq!(st["dirty"], false);
    assert_eq!(found_in(&path, "Committed before the save"), 1);
}

// was tests/p1-ux-gui.test.mjs: 'p1-ux GUI: the toolbar Save clicked during a ⌘Enter commit waits
// for it (final check 1)'. Synchronous here too; the sharper case is a Save with the editor still
// open (the click that blurs it): Save commits it first (session/save.rs commit_open_editor).
#[test]
fn save_with_the_editor_still_open_commits_it_first() {
    let mut g = Gui::new("button-save");
    g.step(json!({"cmd": "new"}));
    let path = g.dir.0.join("button-save.pdf");
    g.step(json!({"cmd": "save-to", "path": path}));
    g.step(json!({"cmd": "open", "path": path}));
    let st = g.step(json!({"cmd": "note", "contents": "already edited"}));
    assert_eq!(st["dirty"], true, "Save is enabled");
    g.tool("text");
    g.press_at(100.0, 300.0);
    assert_eq!(g.state()["direct"]["active"]["kind"], "add");
    g.act(UiAction::EditorInput {
        value: "Kept by the Save button".into(),
    });
    // The toolbar's Save (no ⌘Enter at all).
    g.act(UiAction::Save {
        how: "in-place".into(),
    });
    let st = g.state();
    assert_eq!(st["save"]["ok"], true, "{}", st["save"]);
    assert_eq!(st["dirty"], false);
    assert_eq!(
        st["direct"]["active"],
        Value::Null,
        "the editor was committed"
    );
    assert_eq!(found_in(&path, "Kept by the Save button"), 1);
}

// ------------------------------------------------------------------ redaction

// was tests/integration.test.mjs: 'GUI: a real Escape deselects a box, then stops the redaction
// tool; the redacted copy joins the recent files'.
#[test]
fn escape_peels_the_redaction_box_then_the_tool_and_the_copy_joins_recent_files() {
    let mut g = Gui::new("redact-esc");
    let file = copy_fixture(&g.dir.0, "redact.pdf");
    g.step(json!({"cmd": "open", "path": file}));
    g.tool("redact");
    // Drag a box, then click it.
    g.pointer("down", 60.0, 30.0);
    g.pointer("move", 200.0, 50.0);
    g.pointer("up", 360.0, 70.0);
    assert_eq!(g.state()["redact"]["marks"], 1);
    g.press_at(200.0, 50.0);
    let st = g.state();
    assert_eq!(
        st["direct"]["active"]["kind"], "redact",
        "the box is selected: {}",
        st["direct"]
    );
    g.key("Escape");
    let st = g.state();
    assert_eq!(
        st["direct"]["active"],
        Value::Null,
        "the first Escape deselected the box"
    );
    assert_eq!(st["redact"]["tool"], true, "and left the tool on");
    assert_eq!(g.ui_tool(), "redact");
    g.key("Escape");
    let st = g.state();
    assert_eq!(
        st["redact"]["tool"], false,
        "the second Escape stopped the tool"
    );
    assert_eq!(st["mode"], "read");
    assert_eq!(g.ui_tool(), "select");
    // An open dialog takes Escape first: the tool stays on while the redaction dialog closes.
    g.tool("redact");
    g.s.ui_menu("edit:redact-apply-open");
    assert_eq!(g.state()["redact"]["dialog"], true);
    // The dialog states the size limit (Help says it too: see the source check below).
    let dialog = g.s.ui_state().view.dialog.expect("the redaction dialog");
    assert_eq!(
        dialog.nodes["redact-limit"].text,
        "Redaction works on files up to 12.5 MB in this version."
    );
    g.key("Escape");
    let st = g.state();
    assert_eq!(st["redact"]["dialog"], false);
    assert_eq!(
        st["redact"]["tool"], true,
        "Escape closed the dialog, not the tool"
    );
    let out = g.dir.0.join("redact redacted.pdf");
    let st = g.step(json!({"cmd": "redact-confirm", "contents": {}, "to": out}));
    assert_eq!(st["doc"]["name"], "redact redacted.pdf");
    assert_eq!(st["redact"]["last"]["ok"], true, "{}", st["redact"]["last"]);
    let recent: Vec<String> =
        g.s.ui_state()
            .view
            .recent
            .into_iter()
            .map(|r| r.name)
            .collect();
    assert!(
        recent.iter().any(|n| n == "redact redacted.pdf"),
        "{recent:?}"
    );
}

// was tests/integration.test.mjs: 'GUI: a file with only Adobe Reader usage rights gets its own
// question and banner words'.
#[test]
fn reader_usage_rights_get_their_own_question_and_banner_words() {
    let mut g = Gui::new("ur3");
    // engine/tests/app_redact_boxes.rs p4_signature_corners "ur3": /Perms << /UR3 >> and its
    // signature dictionary only.
    let content = pdf_stream("", "BT /F1 20 Tf 100 300 Td (ACCT-4417-0093) Tj ET");
    let font = courier();
    let bytes = classic_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R /Perms << /UR3 6 0 R >> >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>",
        &content,
        &font,
        "<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /adbe.pkcs7.detached /ByteRange [0 0 0 0] /Contents <00> /Reference [<< /TransformMethod /UR3 >>] >>",
    ]);
    let file = g.dir.0.join("reader-extended.pdf");
    std::fs::write(&file, bytes).unwrap();
    let st = g.step(json!({"cmd": "open", "path": file}));
    assert_eq!(st["doc"]["name"], "reader-extended.pdf");
    g.tool("redact");
    g.step(json!({"cmd": "redact", "contents": {"page": 0, "rect": [90, 470, 400, 500]}}));
    // Declined (the alert's Cancel): STATE names which question it was.
    g.s.ui_menu("edit:redact-apply-open");
    let declined = g.dir.0.join("declined.pdf");
    let st = g.step(json!({"cmd": "redact-confirm", "contents": {}, "to": declined}));
    assert_eq!(
        json!({"reason": st["redact"]["last"]["reason"], "question": st["redact"]["last"]["question"]}),
        json!({"reason": "cancelled", "question": "usage-rights"})
    );
    {
        let asked = g.asked.lock().unwrap();
        let question = asked.last().expect("the window asked");
        assert_eq!(
            question.message,
            "This file has special permissions from Adobe Reader."
        );
        assert_eq!(
            question.detail.as_deref(),
            Some(
                "Redacting removes them. Make a redacted copy without them? The original keeps them."
            )
        );
        assert_eq!(question.buttons[0], "Make redacted copy");
    }
    assert!(!declined.exists());
    // Answered yes.
    g.s.ui_menu("edit:redact-apply-open");
    g.answer("Make redacted copy");
    let out = g.dir.0.join("reader-extended redacted.pdf");
    let st = g.step(json!({"cmd": "redact-confirm", "contents": {}, "to": out}));
    let last = &st["redact"]["last"];
    assert_eq!(last["ok"], true, "{last}");
    assert_eq!(last["unsigned"], true);
    let banner = g.s.dom.text("redact-banner-text");
    assert!(
        banner.contains("The copy no longer has Adobe Reader's special permissions."),
        "{banner}"
    );
    assert!(!banner.contains("no signatures"), "{banner}");
    let shown =
        g.s.ui_state()
            .view
            .banner
            .map(|b| b.text)
            .unwrap_or_default();
    assert!(
        shown.contains("no longer has Adobe Reader's special permissions"),
        "the window shows it: {shown}"
    );
}

// ------------------------------------------------------------------ inspector, keys, dialogs

// was tests/p1-ux.test.mjs: 'X5: the search arrows sit inside the search field, apart from the
// page arrows; X3: Organize keeps the inspector tab' (the X3 half; X5 is a source check below).
#[test]
fn organize_keeps_the_inspector_tab() {
    let mut g = Gui::new("organize");
    g.act(UiAction::Pane {
        which: "panel".into(),
        open: Some(true),
        tab: Some("links".into()),
    });
    assert_eq!(g.state()["tab"], "tab-links");
    g.tool("pages");
    let st = g.state();
    assert_eq!(st["mode"], "organize");
    assert_eq!(st["tab"], "tab-links", "Organize keeps the tab");
    let panel = g.s.ui_state().view.panel;
    assert!(panel.open);
    assert_eq!(panel.tab, "links");
}

// was tests/p2-shell.test.mjs: 'review S2: a drive key on a focused .sig-target reaches its own
// keydown (Enter signs, §9.17)'. In the window: Tab or VoiceOver focuses the signature field
// (UiAction::Focus), then Return.
#[test]
fn return_on_a_focused_signature_field_signs_it() {
    let mut g = Gui::new("sigfield");
    let file = copy_fixture(&g.dir.0, "p5/empty-sigfield.pdf");
    g.step(json!({"cmd": "open", "path": file}));
    g.tool("fill-sign");
    let view = g.s.ui_state().view;
    let mut fields: Vec<_> = view.overlays[0]
        .targets
        .iter()
        .filter(|t| t.kind == "field")
        .cloned()
        .collect();
    fields.sort_by(|a, b| a.rect[0].total_cmp(&b.rect[0]));
    let first = fields.first().expect("a signature field").clone();
    g.act(UiAction::Focus {
        target: first.id.clone(),
    });
    g.key("Enter");
    let st = g.state();
    assert_eq!(
        g.dialog().as_deref(),
        Some("cert-sign-dialog"),
        "{}",
        st["sign"]
    );
    assert_eq!(st["sign"]["dialog"]["page"], 0, "{}", st["sign"]);
    assert!(
        st["sign"]["dialog"]["field"].is_string(),
        "for that field: {}",
        st["sign"]
    );
}

// was tests/p2-shell.test.mjs: 'Bates stamp dialog: clearError first, refusal records
// decorate.last, one-page announcement'.
#[test]
fn the_bates_dialog_records_a_refusal_and_names_a_one_page_number_once() {
    let mut g = Gui::new("bates");
    g.click("stamp-header");
    assert_eq!(g.dialog().as_deref(), Some("stamp-dialog"));
    g.input("stamp-text", "No. {bates}");
    g.input("stamp-bates-start", "x1");
    g.click("stamp-save");
    let st = g.state();
    assert_eq!(
        st["error"],
        "Bates start must be a whole number and digits 1\u{2013}15"
    );
    assert_eq!(
        st["decorate"]["last"],
        json!({"action": "stamp", "kind": "header", "ok": false})
    );
    assert_eq!(g.dialog().as_deref(), Some("stamp-dialog"), "it stays open");
    g.input("stamp-bates-start", "7");
    g.input("stamp-bates-pad", "4");
    g.click("stamp-save");
    let st = g.state();
    assert_eq!(st["error"], "", "the stale refusal is cleared");
    assert_eq!(st["decorate"]["last"]["ok"], true);
    assert_eq!(st["decorate"]["last"]["bates"], json!({"from": 7, "to": 7}));
    assert_eq!(g.dialog(), None);
    assert_eq!(
        g.s.dom.text("page-status"),
        "Added header to 1 page (Bates 0007)"
    );
}

// The same announcement, where the window reads it.
#[test]
fn the_bates_announcement_reaches_the_window() {
    let mut g = Gui::new("bates-live");
    g.click("stamp-header");
    g.input("stamp-text", "No. {bates}");
    g.input("stamp-bates-start", "7");
    g.input("stamp-bates-pad", "4");
    g.click("stamp-save");
    let view = g.s.ui_state().view;
    assert_eq!(view.announce, "Added header to 1 page (Bates 0007)");
}

// A page announcement is said each time, also the same words twice (a second export of the
// same document), and it is what the window says even when the status line changed too.
#[test]
fn the_same_page_announcement_twice_is_said_twice() {
    let mut g = Gui::new("page-said");
    g.s.announce_page("Exported text (1 page)");
    g.s.ui_pump();
    let first = g.s.ui_state().view;
    assert_eq!(first.announce, "Exported text (1 page)");
    g.s.set_status("Page 1 of 1");
    g.s.announce_page("Exported text (1 page)");
    g.s.ui_pump();
    let second = g.s.ui_state().view;
    assert_eq!(second.announce, "Exported text (1 page)");
    assert!(second.announce_seq > first.announce_seq, "said again");
    // A later status change is said as before.
    g.s.set_status("Page 1 of 1 — opened");
    g.s.ui_pump();
    assert_eq!(g.s.ui_state().view.announce, "Page 1 of 1 — opened");
}

// was tests/p2-shell.test.mjs: 'Batch review R1-R6: counter prefill after a run, pad fallback,
// Close while running, Bates column, scroll box, list name' (R1, R3; R4 cannot happen: a batch
// runs within one action, so nothing can press Close while it runs).
#[test]
fn a_batch_run_prefills_the_next_number_and_lists_each_files_numbers() {
    let mut g = Gui::new("batch");
    let folder = g.dir.0.join("batch");
    std::fs::create_dir_all(&folder).unwrap();
    copy_fixture(&folder, "hello.pdf");
    copy_fixture(&folder, "nav.pdf");
    g.s.ui_menu("edit:batch");
    assert_eq!(g.dialog().as_deref(), Some("batch-dialog"));
    g.s.host.platform.answers().unwrap().batch_dir = Some(folder.clone());
    g.click("batch-pick");
    g.input("batch-text", "{bates}");
    g.input("batch-bates-start", "5");
    g.input("batch-bates-pad", "3");
    g.click("batch-start");
    let dialog = g.s.ui_state().view.dialog.expect("the batch dialog");
    assert_eq!(
        dialog.rows,
        vec![
            vec!["hello.pdf".to_string(), "Done, 1 page, number 005".into()],
            vec![
                "nav.pdf".to_string(),
                "Done, 3 pages, numbers 006\u{2013}008".into()
            ],
        ]
    );
    // R1: a second Start continues the counter.
    assert_eq!(dialog.nodes["batch-bates-start"].value, "9");
    assert_eq!(dialog.nodes["batch-bates-pad"].value, "3");
    assert!(!dialog.nodes["batch-close"].disabled);
    let summary = &dialog.nodes["batch-summary"].text;
    assert!(
        summary.contains("pdf4all-out"),
        "where the copies went: {summary}"
    );
    // The originals are untouched.
    assert_eq!(
        std::fs::read(folder.join("hello.pdf")).unwrap(),
        std::fs::read(fixture("hello.pdf")).unwrap()
    );
}

// was tests/p2-shell.test.mjs: 'Compare review fixes are wired (contract-compare.md §9.19)'
// (R1 overlay on B's grid, R7 captions, R4 an emptied or impossible page field).
#[test]
fn compare_overlays_bs_grid_and_names_both_files() {
    let mut g = Gui::new("compare");
    let content = pdf_stream("", "BT /F1 20 Tf 30 30 Td (B side) Tj ET");
    let font = courier();
    let small = classic_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>",
        &content,
        &font,
    ]);
    g.s.open_compare(small, "small.pdf");
    g.s.ui_pump();
    let (wa, ha, _) = g.s.ui_pixels("compare-a", 0, 1.0, None).unwrap();
    let (wb, hb, _) = g.s.ui_pixels("compare-b", 0, 1.0, None).unwrap();
    let (wo, ho, overlay) = g.s.ui_pixels("compare-overlay", 0, 1.0, None).unwrap();
    assert_eq!((wa, ha), (612, 792));
    assert_eq!((wb, hb), (300, 200));
    assert_eq!((wo, ho), (wb, hb), "the red pixels sit on B's pixels");
    assert_eq!(overlay.len(), (wb * hb * 4) as usize);
    let compare = g.s.ui_state().view.compare.expect("compare is shown");
    assert_eq!(
        (compare.name_a.as_str(), compare.name_b.as_str()),
        ("hello.pdf", "small.pdf")
    );
    // An emptied page field keeps the page; one past the end shows the last page and says so.
    g.input("compare-page-b", "");
    assert_eq!(g.s.ui_state().view.compare.unwrap().page_b, 0);
    g.input("compare-page-b", "9");
    let compare = g.s.ui_state().view.compare.unwrap();
    assert_eq!(compare.page_b, 0);
    assert!(
        compare
            .summary
            .starts_with("Page 9 doesn't exist: small.pdf has 1 page. Showing page 1."),
        "{}",
        compare.summary
    );
    g.key("Escape");
    assert!(g.s.ui_state().view.compare.is_none(), "Esc closes Compare");
}

// ------------------------------------------------------------------ source checks

const STYLES: &str = include_str!("../../tandem-ui/styles.css");
const UI_DIALOGS: &str = include_str!("../../tandem-ui/src/dialogs.rs");
const UI_EDITOR: &str = include_str!("../../tandem-ui/src/editor.rs");
const UI_SOURCES: &[(&str, &str)] = &[
    ("dialogs.rs", UI_DIALOGS),
    ("editor.rs", UI_EDITOR),
    ("lib.rs", include_str!("../../tandem-ui/src/lib.rs")),
    ("pages.rs", include_str!("../../tandem-ui/src/pages.rs")),
    ("panel.rs", include_str!("../../tandem-ui/src/panel.rs")),
    ("screens.rs", include_str!("../../tandem-ui/src/screens.rs")),
    ("start.rs", include_str!("../../tandem-ui/src/start.rs")),
    (
        "viewport.rs",
        include_str!("../../tandem-ui/src/viewport.rs"),
    ),
];
const MENU: &str = include_str!("../../tandem-desktop/src/menu.rs");

fn ui_source(file: &str) -> &'static str {
    UI_SOURCES
        .iter()
        .find(|(name, _)| *name == file)
        .unwrap_or_else(|| panic!("no {file}"))
        .1
}

// was tests/p1-ux-gui.test.mjs: 'p1-ux GUI: chrome is not selectable but content is …' (review
// fix 4, X12). Source check: computed styles exist only in the real window.
#[test]
fn chrome_is_not_selectable_but_content_is() {
    let line = |start: &str| {
        STYLES
            .lines()
            .find(|l| l.starts_with(start))
            .unwrap_or_else(|| panic!("no rule {start}"))
    };
    assert!(line("body {").contains("user-select: none"));
    let text = line(".selectable, input, textarea, [contenteditable] {");
    assert!(text.contains("user-select: text"), "{text}");
    assert!(line(".notice p {").contains("user-select: text"), "notices");
    assert!(line(".help {").contains("user-select: text"), "Help");
    // Dialog status lines, the verify summary, the compare summary and the panel status can be
    // selected and copied.
    for (file, needle) in [
        (
            "dialogs.rs",
            r#"<p role="alert" class="selectable dialog-error">"#,
        ),
        (
            "dialogs.rs",
            r#"<p class="selectable">{d.text("verify-summary")}</p>"#,
        ),
        ("screens.rs", r#"<p class="selectable compare-summary""#),
        (
            "panel.rs",
            r#"<p class="panel-status selectable" role="status">"#,
        ),
    ] {
        assert!(ui_source(file).contains(needle), "{file}: {needle}");
    }
}

// was tests/p2-shell.test.mjs: 'debt1 row 18: the Save As button reads the same as the File menu
// item'. Source check: crates/tandem-desktop/src/menu.rs and the window's own File menu
// (crates/tandem-ui/src/editor.rs).
#[test]
fn save_as_reads_the_same_in_the_menu_and_the_window() {
    let menu_label = |id: &str| {
        let key = format!("(\"{id}\", \"");
        let at = MENU
            .find(&key)
            .unwrap_or_else(|| panic!("no menu item {id}"));
        let rest = &MENU[at + key.len()..];
        rest[..rest.find('"').unwrap()].to_string()
    };
    let label = menu_label("save:as");
    assert_eq!(label, "Save As…");
    let window = format!("item(\"{label}\", UiAction::Save {{ how: \"as\".into() }})");
    assert!(UI_EDITOR.contains(&window), "the window says {label}");
}

// was tests/sign-merge.test.mjs: '(d) static: the old Sign… dialog and the "Signature
// Appearance…" menu item are gone; the pad has "Use an image…"'. Source check of the menu and
// the pad (the pad's behaviour: ui.rs the_menu_signature_is_ready_to_place_after_the_pad).
#[test]
fn one_add_signature_menu_item_and_a_pad_that_takes_a_picture() {
    // One item places a signature (the id also appears in the list of editor-only items).
    assert_eq!(MENU.matches("\"Add My Signature\"").count(), 1);
    let folded: String = MENU.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(folded.contains("( \"edit:sign-place\", \"Add My Signature\","));
    // The only other item with "Signature" in it is Verify Signatures….
    let labels: Vec<&str> = MENU
        .lines()
        .filter(|l| l.contains("Signature"))
        .map(str::trim)
        .collect();
    assert!(
        labels
            .iter()
            .all(|l| l.starts_with("\"Add My Signature\"") || l.contains("\"Verify Signatures…\"")),
        "{labels:?}"
    );
    assert!(!MENU.contains("Signature Appearance"));
    assert!(!MENU.contains("Place Signature"));
    // No old Sign… dialog among the window's dialogs.
    assert!(
        tandem_app::dialog_ids::DIALOGS
            .iter()
            .all(|(id, _)| *id != "sign-dialog")
    );
    assert!(
        tandem_app::dialog_ids::DIALOGS
            .iter()
            .any(|(id, _)| *id == "sigpad-dialog")
    );
    // The pad: "Your signature", with a picture option.
    assert!(UI_DIALOGS.contains(r#"{title("Your signature")}"#));
    assert!(UI_DIALOGS.contains(r#"id="sigpad-image" label="Use a picture…""#));
}

// was tests/p2-shell.test.mjs: 'shell HTML has required control IDs' (the stamp hint: the
// placeholders a person can type). Source check of the stamp and batch dialogs.
#[test]
fn the_stamp_and_batch_dialogs_name_every_placeholder() {
    for field in ["stamp-text", "batch-text"] {
        let at = UI_DIALOGS
            .find(&format!("id=\"{field}\""))
            .unwrap_or_else(|| panic!("no {field}"));
        let tag = &UI_DIALOGS[at..at + UI_DIALOGS[at..].find("/>").unwrap()];
        for placeholder in ["{page}", "{pages}", "{label}", "{bates}"] {
            assert!(tag.contains(placeholder), "{field} names {placeholder}");
        }
    }
}

// was tests/p2-shell.test.mjs: 'U7/P4/P5 help dialog: ten matrix rows, a ceilings list and every
// panel reason text' (what the plain-words Help keeps; X7 replaced the matrix). Help states the
// redaction size limit the dialog states (that one is checked in the redaction test above).
#[test]
fn help_states_the_redaction_size_limit() {
    let at = UI_DIALOGS.find("fn Help()").expect("the Help component");
    let help = &UI_DIALOGS[at..];
    assert!(help.contains("Files up to 12.5 MB can be redacted."));
    assert!(help.contains("scripts inside PDF files are never run"));
}

// was tests/p1-ux.test.mjs: 'X5: the search arrows sit inside the search field, apart from the
// page arrows …' (the X5 half). Source check of the toolbar's search form.
#[test]
fn the_search_arrows_sit_inside_the_search_field() {
    let start = UI_EDITOR
        .find(r#"<form class="search""#)
        .expect("search form");
    let form = &UI_EDITOR[start..start + UI_EDITOR[start..].find("</form>").unwrap()];
    assert!(form.contains(r#"id="search-input""#));
    assert!(form.contains(r#"aria-label="Previous match""#));
    assert!(form.contains(r#"aria-label="Next match""#));
    assert!(!form.contains("Goto"), "the page arrows are elsewhere");
}

// was tests/p2-shell.test.mjs: 'modes are radios; every control has id + accessible name' (the
// accessible-name half, for buttons). Source check: every <button> in the window has words (a
// text child) or an aria-label / title, so VoiceOver can name it.
#[test]
fn every_button_in_the_window_has_a_name() {
    let mut nameless = Vec::new();
    for (file, src) in UI_SOURCES {
        let mut from = 0;
        while let Some(at) = src[from..].find("<button") {
            let start = from + at;
            let end = start
                + src[start..]
                    .find("</button>")
                    .unwrap_or_else(|| panic!("{file}: unclosed button"));
            let button = &src[start..end];
            let named = button.contains("aria-label")
                || button.contains("title=")
                || button.contains(">\"")
                || button.contains(">{")
                || button.contains("<strong>");
            if !named {
                let shown: String = button.chars().take(120).collect();
                nameless.push(format!("{file}: {shown}"));
            }
            from = end;
        }
    }
    assert!(nameless.is_empty(), "{nameless:#?}");
}

// was tests/p2-shell.test.mjs: 'A1: every renderer request op is classified by the engine
// permission table'. Every op the session sends (`"op": "…"` in crates/tandem-app/src) is in
// the engine's OP_PERMISSION, or one of the read-only ops it never guards, or one of the
// mutations that take its Edit fallback on purpose.
#[test]
fn every_request_op_is_classified_by_the_engine_permission_table() {
    use tandem_pdf_engine::app::OP_PERMISSION;
    let mut ops = std::collections::BTreeSet::new();
    for (_, text) in sources(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src")) {
        for (at, _) in text.match_indices("\"op\": \"") {
            let rest = &text[at + 7..];
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_lowercase() || *c == '-')
                .collect();
            if !name.is_empty() && rest[name.len()..].starts_with('"') {
                ops.insert(name);
            }
        }
    }
    assert!(ops.len() >= 40, "the session's ops: {ops:?}");
    // Reads and plumbing the engine never routes through require_mutable.
    const READ_ONLY: &[&str] = &[
        "open",
        "render",
        "thumbnail",
        "search",
        "fields",
        "annots",
        "outline",
        "links",
        "edit-targets",
        "edit-replace-preview",
        "text-all",
        "labels",
        "layers",
        "redact-preview",
        "redact-inspect",
        "save",
        "validate",
        "saved",
        "close",
        "authenticate",
        "extract-pages",
        "pdf-from-images",
        "batch-file",
        "page-ids",
        "redact",
        "sign",
        "sign-prepare",
        "sign-list",
        "verify",
        "compare-open",
        "compare-close",
        "compare-render",
        "compare-text",
        // New in the Rust app: the editable copy of a signed file is a new document; the
        // open one is not changed.
        "unsigned-copy",
    ];
    // Mutations with no table entry: require_mutable reads them as Edit (engine/src/app/mod.rs).
    const DEFAULT_EDIT: &[&str] = &[
        "edit-text",
        "edit-paragraph",
        "edit-object",
        "edit-objects",
        "edit-replace-apply",
        "security-set",
        "layer-set",
    ];
    let in_table = |op: &str| OP_PERMISSION.iter().any(|(name, _)| *name == op);
    let unexplained: Vec<&String> = ops
        .iter()
        .filter(|op| {
            !in_table(op)
                && !READ_ONLY.contains(&op.as_str())
                && !DEFAULT_EDIT.contains(&op.as_str())
        })
        .collect();
    assert!(
        unexplained.is_empty(),
        "add the op to OP_PERMISSION, READ_ONLY or DEFAULT_EDIT: {unexplained:?}"
    );
    let doubled: Vec<&&str> = READ_ONLY
        .iter()
        .chain(DEFAULT_EDIT)
        .filter(|op| in_table(op))
        .collect();
    assert!(
        doubled.is_empty(),
        "a read-only or fallback op must not also claim a permission rule: {doubled:?}"
    );
}

// was tests/p2-shell.test.mjs: 'review R-U3-1: every button shipped disabled with its own const is
// enabled somewhere in the renderer'. The session's initial page (session/dom_ids.rs INITIAL)
// marks some controls disabled; each must be switched by a `set_disabled(` somewhere in
// crates/tandem-app/src, by its const or its id. Source check (the table is private).
#[test]
fn every_control_shipped_disabled_is_switched_somewhere() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let dom_ids = std::fs::read_to_string(root.join("session/dom_ids.rs")).unwrap();
    let initial = &dom_ids[dom_ids.find("const INITIAL").expect("the initial table")..];
    let mut disabled = Vec::new();
    for entry in initial.split("Initial {").skip(1) {
        let entry = &entry[..entry.find('}').unwrap_or(entry.len())];
        if !entry.contains("disabled: true") {
            continue;
        }
        let at = entry.find("id: \"").unwrap() + 5;
        disabled.push(entry[at..at + entry[at..].find('"').unwrap()].to_string());
    }
    assert!(disabled.len() >= 20, "{disabled:?}");
    // Most are enabled once an editable document is open and changed: hello.pdf with a note
    // and a search match (the markup buttons mark matches), then a three-page file on its
    // middle page (the page buttons). What stays disabled needs more (a selection, a box, a
    // folder, a certificate) and must at least be switched in the source.
    let mut g = Gui::new("disabled");
    g.step(json!({"cmd": "note", "contents": "an edit"}));
    g.act(UiAction::Search {
        query: "PDF4All".into(),
    });
    let mut enabled: Vec<String> = disabled
        .iter()
        .filter(|id| !g.s.dom.disabled(id))
        .cloned()
        .collect();
    let nav = copy_fixture(&g.dir.0, "nav.pdf");
    g.step(json!({"cmd": "open", "path": nav}));
    g.act(UiAction::Goto { page: 1 });
    enabled.extend(disabled.iter().filter(|id| !g.s.dom.disabled(id)).cloned());
    let still: Vec<&String> = disabled.iter().filter(|id| !enabled.contains(id)).collect();
    // All sources, whitespace folded, so a call split over lines reads as one.
    let all: String = sources(&root)
        .into_iter()
        .map(|(_, text)| text.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("\n");
    // `set_disabled(NAME, …)`, or NAME in the list of a `for … in [ …, NAME, … ]` just before.
    let switched = |name: &str| {
        all.match_indices("set_disabled(").any(|(at, _)| {
            let rest = &all[at + 13..];
            let arg = rest.split(',').next().unwrap_or("").trim();
            if arg == name || arg.ends_with(&format!("::{name}")) {
                return true;
            }
            let mut from = at.saturating_sub(600);
            while !all.is_char_boundary(from) {
                from += 1;
            }
            let before = &all[from..at];
            let Some(list) = before.rfind("for ").map(|i| &before[i..]) else {
                return false;
            };
            list.contains(" in [")
                && [",", "]", " "].iter().any(|end| {
                    list.contains(&format!("::{name}{end}"))
                        || list.contains(&format!("[{name}{end}"))
                        || list.contains(&format!(" {name}{end}"))
                })
        })
    };
    let mut never = Vec::new();
    for id in still {
        let literal = format!("\"{id}\"");
        let consts: Vec<String> = dom_ids
            .lines()
            .filter_map(|line| {
                let rest = line.trim().strip_prefix("pub const ")?;
                let (name, value) = rest.split_once(": &str = ")?;
                (value.trim_end_matches(';') == literal).then(|| name.to_string())
            })
            .collect();
        if !switched(&literal) && !consts.iter().any(|c| switched(c)) {
            never.push(id.clone());
        }
    }
    assert!(
        never.is_empty(),
        "shipped disabled and never enabled: {never:?}"
    );
}
