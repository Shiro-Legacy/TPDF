//! The host's file rules: copy names, the filled-form and pending-boxes
//! questions, Untitled, recent files, one-shot opens, batch, links, print,
//! the trust list, certificates and the single-instance lock.

use super::*;
use crate::platform::{Answers, Scripted};
use serde_json::json;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture(rel: &str) -> PathBuf {
    util::normalize(&root().join(rel))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = util::mkdtemp(&util::temp_dir().join(format!("tandem-host-{tag}-"))).unwrap();
    fs::canonicalize(dir).unwrap()
}

/// A platform for GUI-mode tests: queued message-box answers, recorded sheets.
#[derive(Default)]
struct Recorder {
    replies: Vec<usize>,
    asked: Vec<Question>,
    save_defaults: Vec<PathBuf>,
    save_answer: Option<PathBuf>,
    open_answer: Option<PathBuf>,
    links: Vec<String>,
    recent_docs: Vec<PathBuf>,
}

/// Lets a test read the recorder after handing it to a host.
struct Shared(std::sync::Arc<std::sync::Mutex<Recorder>>);

impl Platform for Shared {
    fn pick_file(&mut self, _pick: Pick, _filters: &[Filter]) -> Option<PathBuf> {
        self.0.lock().unwrap().open_answer.clone()
    }
    fn pick_images(&mut self) -> Vec<PathBuf> {
        Vec::new()
    }
    fn pick_folder(&mut self) -> Option<PathBuf> {
        None
    }
    fn pick_save(
        &mut self,
        _purpose: SavePurpose,
        default_path: &Path,
        _filters: &[Filter],
    ) -> Option<PathBuf> {
        let mut r = self.0.lock().unwrap();
        r.save_defaults.push(default_path.to_path_buf());
        r.save_answer.clone()
    }
    fn ask(&mut self, question: &MessageBox) -> usize {
        let mut r = self.0.lock().unwrap();
        r.asked.push(question.question);
        if r.replies.is_empty() {
            question.cancel
        } else {
            r.replies.remove(0)
        }
    }
    fn open_external(&mut self, uri: &str) {
        self.0.lock().unwrap().links.push(uri.into());
    }
    fn print(&mut self, _pdf: &[u8], _job_name: &str) -> PrintOutcome {
        PrintOutcome::NoPrinter
    }
    fn add_recent_document(&mut self, path: &Path) {
        self.0.lock().unwrap().recent_docs.push(path.into());
    }
    fn clear_recent_documents(&mut self) {}
    fn set_represented_file(&mut self, _path: Option<&Path>, _title: &str) {}
    fn set_document_state(&mut self, _edited: bool, _save: bool, _save_as: bool) {}
    fn answers(&mut self) -> Option<&mut Answers> {
        None
    }
}

fn gui_host(tag: &str) -> (Host, std::sync::Arc<std::sync::Mutex<Recorder>>, PathBuf) {
    let dir = scratch(tag);
    let rec = std::sync::Arc::new(std::sync::Mutex::new(Recorder::default()));
    let host = Host::new(
        HostConfig {
            user_data: dir.join("udata"),
            smoke: false,
            echo_stdout: false,
        },
        Box::new(Shared(rec.clone())),
    );
    (host, rec, dir)
}

fn smoke_host(tag: &str) -> (Host, PathBuf) {
    let dir = scratch(tag);
    let host = Host::new(
        HostConfig {
            user_data: dir.join("udata"),
            smoke: true,
            echo_stdout: false,
        },
        Box::new(Scripted::default()),
    );
    (host, dir)
}

fn answers(host: &mut Host) -> &mut Answers {
    host.platform.answers().unwrap()
}

/// Opens `path` and returns the delivered document.
fn open(host: &mut Host, path: &Path) -> Document {
    host.deliver_open(Some(path), false);
    match host.take_events().as_slice() {
        [Event::Document(doc)] => doc.clone(),
        other => panic!("expected a document, got {other:?}"),
    }
}

fn copy_fixture(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    fs::copy(fixture(&format!("fixtures/{name}")), &path).unwrap();
    path
}

#[test]
fn names() {
    assert_eq!(
        suggested_kind_name(Some("report"), SaveKind::Pdf).as_deref(),
        Some("report.pdf")
    );
    assert_eq!(
        suggested_kind_name(Some("A.PDF"), SaveKind::Pdf).as_deref(),
        Some("A.PDF")
    );
    assert_eq!(
        suggested_kind_name(Some("p.jpeg"), SaveKind::Jpeg).as_deref(),
        Some("p.jpeg")
    );
    assert_eq!(
        suggested_kind_name(Some("p"), SaveKind::Jpeg).as_deref(),
        Some("p.jpg")
    );
    assert_eq!(suggested_kind_name(Some("a/b"), SaveKind::Pdf), None);
    assert_eq!(suggested_kind_name(Some(""), SaveKind::Txt), None);
    assert_eq!(
        suggested_kind_name(Some(&"x".repeat(252)), SaveKind::Pdf),
        None
    );
    assert_eq!(
        suggested_kind_name(Some(&"x".repeat(251)), SaveKind::Pdf).map(|s| s.len()),
        Some(255)
    );
    assert_eq!(batch_out_name("a.pdf", &["a.pdf".into()]), Some("a.pdf"));
    assert_eq!(batch_out_name("report.txt", &[]), Some("report.txt"));
    assert_eq!(batch_out_name("../a.pdf", &["../a.pdf".into()]), None);
    assert_eq!(batch_out_name("b.pdf", &["a.pdf".into()]), None);
    assert!(link_allowed("https://example.com/x"));
    assert!(link_allowed(" HTTP://a"));
    assert!(link_allowed("mailto:"));
    assert!(link_allowed("http:example.com"));
    assert!(!link_allowed("http://"));
    assert!(!link_allowed("javascript:alert(1)"));
    assert!(!link_allowed("file:///etc/passwd"));
    assert!(!link_allowed("no scheme"));
    assert_eq!(strip_pdf_ext("a.PDF"), "a");
    assert_eq!(strip_pdf_ext("a.txt"), "a.txt");
}

#[test]
fn open_and_read_once() {
    let (mut host, dir) = smoke_host("open");
    let path = copy_fixture(&dir, "hello.pdf");
    let doc = open(&mut host, &path);
    assert_eq!(doc.name, "hello.pdf");
    assert!(!doc.untitled);
    assert_eq!(doc.size, fs::metadata(&path).unwrap().len());
    let bytes = host.read_open(&doc.url).unwrap();
    assert!(bytes.starts_with(b"%PDF"));
    // One shot.
    assert_eq!(host.read_open(&doc.url), None);
    assert_eq!(host.recent_list()[0].name, "hello.pdf");
    // A missing file is unreadable and leaves the recent list.
    fs::remove_file(&path).unwrap();
    host.deliver_open(Some(&path), false);
    assert_eq!(
        host.take_events(),
        vec![Event::OpenError(OpenError::unreadable(Some(
            "hello.pdf".into()
        )))]
    );
    assert!(host.recent_list().is_empty());
    // No path in a headless run: cancelled (no sheet).
    host.deliver_open(None, false);
    assert_eq!(
        host.take_events(),
        vec![Event::OpenError(OpenError::cancelled())]
    );
    host.open_path("rel.pdf");
    assert_eq!(
        host.take_events(),
        vec![Event::OpenError(OpenError::unreadable(Some(
            "rel.pdf".into()
        )))]
    );
    host.recent_open("stale");
    assert_eq!(
        host.take_events(),
        vec![Event::OpenError(OpenError::unreadable(None))]
    );
    assert_eq!(
        Host::inspect_open("rel.pdf"),
        Some(OpenError::unreadable(Some("rel.pdf".into())))
    );
    assert_eq!(
        Host::inspect_open(&path.to_string_lossy()),
        Some(OpenError::unreadable(Some("hello.pdf".into())))
    );
    assert_eq!(
        OpenError::too_large("a.pdf".into()).to_json(),
        json!({"reason": "too-large", "name": "a.pdf", "limit": 125829120})
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn save_a_copy_and_save_in_place() {
    let (mut host, dir) = smoke_host("save");
    let path = copy_fixture(&dir, "hello.pdf");
    let doc = open(&mut host, &path);
    let bytes = fs::read(&path).unwrap();
    // Save a copy with no target: cancelled (a headless run shows no sheet).
    assert_eq!(
        host.save_file(&bytes, &doc.url, &SaveOptions::default()),
        Err(SaveFail::Cancelled)
    );
    assert_eq!(
        host.save_file(&bytes, "app://pdf4all/open/other", &SaveOptions::default()),
        Err(SaveFail::DocumentChanged)
    );
    assert_eq!(
        host.save_file(&bytes, "", &SaveOptions::default()),
        Err(SaveFail::DocumentChanged)
    );
    assert_eq!(
        host.save_file(b"%PDF", &doc.url, &SaveOptions::default()),
        Err(SaveFail::Unreadable)
    );
    let copy = dir.join("copy.pdf");
    answers(&mut host).save_to = Some(copy.clone());
    let ok = host
        .save_file(&bytes, &doc.url, &SaveOptions::default())
        .unwrap();
    assert_eq!(
        ok,
        SaveOk {
            name: "copy.pdf".into(),
            warning: None,
            same_file: false
        }
    );
    assert_eq!(fs::read(&copy).unwrap(), bytes);
    // A copy leaves the window on the original.
    assert_eq!(host.open_record().unwrap().file.path, path);
    // Never over the open file.
    answers(&mut host).save_to = Some(path.clone());
    assert_eq!(
        host.save_file(&bytes, &doc.url, &SaveOptions::default()),
        Err(SaveFail::SameFile)
    );
    // Save in place.
    let mut edited = bytes.clone();
    edited.extend_from_slice(b"\n% edited\n");
    let ok = host
        .save_file(
            &edited,
            &doc.url,
            &SaveOptions {
                in_place: true,
                ..SaveOptions::default()
            },
        )
        .unwrap();
    assert_eq!(ok.name, "hello.pdf");
    assert_eq!(fs::read(&path).unwrap(), edited);
    // Save As onto the open file itself is a plain Save.
    let ok = host
        .save_file(
            &bytes,
            &doc.url,
            &SaveOptions {
                rebind: true,
                ..SaveOptions::default()
            },
        )
        .unwrap();
    assert!(ok.same_file);
    // Save As elsewhere rebinds the window.
    let other = dir.join("other.pdf");
    answers(&mut host).save_to = Some(other.clone());
    let ok = host
        .save_file(
            &bytes,
            &doc.url,
            &SaveOptions {
                rebind: true,
                ..SaveOptions::default()
            },
        )
        .unwrap();
    assert_eq!(ok.name, "other.pdf");
    assert_eq!(host.open_record().unwrap().file.path, other);
    assert_eq!(host.open_record().unwrap().url, doc.url);
    assert_eq!(host.recent_list()[0].name, "other.pdf");
    // Save As with no target: cancelled.
    answers(&mut host).save_to = None;
    assert_eq!(
        host.save_file(
            &bytes,
            &doc.url,
            &SaveOptions {
                rebind: true,
                ..SaveOptions::default()
            }
        ),
        Err(SaveFail::Cancelled)
    );
    // The journal holds nothing once the saves are done.
    let journal = fs::read_to_string(host.journal()).unwrap_or_default();
    assert!(
        !journal.contains(".tmp") || journal.trim() == "[]",
        "{journal}"
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn copy_name_and_filled_question_in_a_window() {
    let (mut host, rec, dir) = gui_host("filled");
    let path = copy_fixture(&dir, "form.pdf");
    let doc = open(&mut host, &path);
    let bytes = fs::read(&path).unwrap();
    // Save a Copy suggests "<name>-copy.pdf"; a suggested name wins.
    let _ = host.save_file(&bytes, &doc.url, &SaveOptions::default());
    let _ = host.save_file(
        &bytes,
        &doc.url,
        &SaveOptions {
            suggested_name: Some("mine".into()),
            ..SaveOptions::default()
        },
    );
    // Save As suggests the current path.
    let _ = host.save_file(
        &bytes,
        &doc.url,
        &SaveOptions {
            rebind: true,
            ..SaveOptions::default()
        },
    );
    // A filled form's Save asks; Escape (Cancel) cancels.
    let filled = SaveOptions {
        in_place: true,
        filled: true,
        ..SaveOptions::default()
    };
    assert_eq!(
        host.save_file(&bytes, &doc.url, &filled),
        Err(SaveFail::Cancelled)
    );
    // "Save as New File…" suggests "<dir>/<name> (filled).pdf" and rebinds.
    let out = dir.join("form (filled).pdf");
    {
        let mut r = rec.lock().unwrap();
        r.replies = vec![1];
        r.save_answer = Some(out.clone());
    }
    assert_eq!(
        host.save_file(&bytes, &doc.url, &filled).unwrap().name,
        "form (filled).pdf"
    );
    assert_eq!(host.open_record().unwrap().file.path, out);
    // "Save Over" saves in place.
    rec.lock().unwrap().replies = vec![0];
    let ok = host.save_file(&bytes, &doc.url, &filled).unwrap();
    assert_eq!(ok.name, "form (filled).pdf");
    let r = rec.lock().unwrap();
    assert_eq!(
        r.save_defaults,
        vec![
            PathBuf::from("form-copy.pdf"),
            PathBuf::from("mine.pdf"),
            path.clone(),
            dir.join("form (filled).pdf")
        ]
    );
    assert_eq!(
        r.asked,
        vec![Question::Filled, Question::Filled, Question::Filled]
    );
    assert_eq!(r.recent_docs, vec![path.clone(), out.clone()]);
    drop(r);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn filled_answer_in_a_drive() {
    let (mut host, dir) = smoke_host("filled-drive");
    let path = copy_fixture(&dir, "form.pdf");
    let doc = open(&mut host, &path);
    let bytes = fs::read(&path).unwrap();
    let filled = SaveOptions {
        in_place: true,
        filled: true,
        ..SaveOptions::default()
    };
    // No `to`: the new file has no target, so cancelled.
    assert_eq!(
        host.save_file(&bytes, &doc.url, &filled),
        Err(SaveFail::Cancelled)
    );
    let out = dir.join("f2.pdf");
    answers(&mut host).save_to = Some(out.clone());
    assert_eq!(
        host.save_file(&bytes, &doc.url, &filled).unwrap().name,
        "f2.pdf"
    );
    // `filled: "over"` saves over the bound file.
    answers(&mut host).filled_over = true;
    answers(&mut host).save_to = None;
    assert_eq!(
        host.save_file(&bytes, &doc.url, &filled).unwrap().name,
        "f2.pdf"
    );
    assert_eq!(
        fs::read(&path).unwrap(),
        fs::read(fixture("fixtures/form.pdf")).unwrap()
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn untitled_documents() {
    let (mut host, dir) = smoke_host("untitled");
    host.new_blank(Some(&json!([100, 200])));
    let events = host.take_events();
    let [Event::Document(doc)] = events.as_slice() else {
        panic!("{events:?}")
    };
    assert!(doc.untitled);
    assert_eq!(doc.name, "Untitled.pdf");
    let bytes = host.read_open(&doc.url).unwrap();
    assert_eq!(bytes, blank::blank_pdf(100.0, 200.0));
    // Untitled is no recent file.
    assert!(host.recent_list().is_empty());
    // Save on Untitled is a Save As: no target → cancelled; with one, the window rebinds.
    let save = SaveOptions {
        in_place: true,
        ..SaveOptions::default()
    };
    assert_eq!(
        host.save_file(&bytes, &doc.url, &save),
        Err(SaveFail::Cancelled)
    );
    let out = dir.join("kept.pdf");
    answers(&mut host).save_to = Some(out.clone());
    assert_eq!(
        host.save_file(&bytes, &doc.url, &save).unwrap().name,
        "kept.pdf"
    );
    assert_eq!(host.open_record().unwrap().file.path, out);
    // Now a plain in-place save.
    answers(&mut host).save_to = None;
    assert_eq!(
        host.save_file(&bytes, &doc.url, &save).unwrap().name,
        "kept.pdf"
    );
    assert_eq!(host.recent_list()[0].name, "kept.pdf");
    // A bad size is an unreadable Untitled.pdf.
    host.new_blank(Some(&json!(null)));
    assert_eq!(
        host.take_events(),
        vec![Event::OpenError(OpenError::unreadable(Some(
            "Untitled.pdf".into()
        )))]
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn pending_boxes_and_discard_in_a_window() {
    let (mut host, rec, dir) = gui_host("boxes");
    let path = copy_fixture(&dir, "hello.pdf");
    open(&mut host, &path);
    host.set_redact_pending(2, false);
    host.set_dirty(true);
    // "Redact Now…": the open is cancelled and the redact dialog requested.
    rec.lock().unwrap().replies = vec![0];
    host.deliver_open(Some(&path), false);
    assert_eq!(
        host.take_events(),
        vec![
            Event::EditRequest {
                op: "redact-apply-open".into(),
                contents: None
            },
            Event::OpenError(OpenError::cancelled())
        ]
    );
    // "Discard the Boxes": opens with no discard question.
    rec.lock().unwrap().replies = vec![1];
    open(&mut host, &path);
    assert_eq!(
        rec.lock().unwrap().asked,
        vec![Question::PendingBoxes, Question::PendingBoxes]
    );
    // Dirty with no boxes: the discard question; Cancel keeps the document.
    host.set_dirty(true);
    host.deliver_open(Some(&path), false);
    assert_eq!(
        host.take_events(),
        vec![Event::OpenError(OpenError::cancelled())]
    );
    rec.lock().unwrap().replies = vec![0];
    open(&mut host, &path);
    // The signed copy's reopen skips both questions for marks.
    host.set_dirty(true);
    host.set_redact_pending(1, false);
    host.deliver_open(Some(&path), true);
    assert!(matches!(
        host.take_events().as_slice(),
        [Event::Document(_)]
    ));
    // Closing: memory boxes ask, then nothing else.
    host.set_redact_pending(1, true);
    host.set_dirty(true);
    rec.lock().unwrap().replies = vec![2];
    assert!(!host.close_requested());
    rec.lock().unwrap().replies = vec![1];
    assert!(host.close_requested());
    let q = pending_question(1, true, true);
    assert_eq!(q.message, "1 area is marked but not redacted yet.");
    assert_eq!(q.buttons[1], "Close Without Redacting");
    assert!(
        q.detail
            .unwrap()
            .starts_with("These boxes are kept only while the file is open. Closing now")
    );
    let q = pending_question(3, false, false);
    assert_eq!(q.message, "3 areas are marked but not redacted yet.");
    assert_eq!(q.buttons[1], "Discard the Boxes");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn smoke_skips_questions() {
    let (mut host, dir) = smoke_host("smoke-q");
    let path = copy_fixture(&dir, "hello.pdf");
    open(&mut host, &path);
    host.set_redact_pending(2, false);
    host.set_dirty(true);
    open(&mut host, &path);
    assert!(host.close_requested());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn redacted_copy_and_original() {
    let (mut host, dir) = smoke_host("redacted");
    let path = copy_fixture(&dir, "hello.pdf");
    let doc = open(&mut host, &path);
    let bytes = fs::read(&path).unwrap();
    answers(&mut host).save_to = Some(path.clone());
    assert_eq!(
        host.save_new(&bytes, Some("hello-redacted"), SaveKind::Pdf, false, true),
        Err(SaveFail::SameFile)
    );
    let out = dir.join("hello-redacted.pdf");
    answers(&mut host).save_to = Some(out.clone());
    let saved = host
        .save_new(&bytes, Some("hello-redacted"), SaveKind::Pdf, false, true)
        .unwrap();
    assert_eq!(saved.original, Some(true));
    let url = saved.url.unwrap();
    assert!(!host.bind_open(&doc.url));
    assert!(host.read_open(&url).is_some());
    assert!(host.bind_open(&url));
    assert_eq!(host.open_record().unwrap().file.path, out);
    let back = host.open_original().unwrap();
    assert_eq!(back.name, "hello.pdf");
    assert!(host.bind_open(&back.url));
    assert_eq!(host.open_original(), Err("unreadable"));
    // Exports never open and keep their own extension.
    answers(&mut host).save_to = Some(dir.join("p.txt"));
    assert_eq!(
        host.save_new(b"PDF4All text", None, SaveKind::Txt, true, true)
            .unwrap()
            .url,
        None
    );
    assert!(host.take_events().is_empty());
    assert_eq!(
        host.save_new(b"", None, SaveKind::Txt, false, false),
        Err(SaveFail::Unreadable)
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn sources_images_and_batch() {
    let (mut host, dir) = smoke_host("sources");
    assert_eq!(host.insert_source(), Err("cancelled"));
    answers(&mut host).insert_from = Some(fixture("fixtures/form.pdf"));
    let picked = host.insert_source().unwrap();
    assert_eq!(picked.name, "form.pdf");
    answers(&mut host).insert_from = Some(PathBuf::from("fixtures/form.pdf"));
    assert_eq!(host.insert_source(), Err("unreadable"));
    assert_eq!(host.pick_images(), Err("cancelled"));
    let png = dir.join("a.png");
    fs::write(&png, [0x89, b'P', b'N', b'G']).unwrap();
    answers(&mut host).images = Some(format!("{}, ,{}", png.display(), png.display()));
    assert_eq!(host.pick_images().unwrap().len(), 2);
    answers(&mut host).images = Some(vec![png.to_string_lossy().into_owned(); 51].join(","));
    assert_eq!(host.pick_images(), Err("too-many"));
    // Batch.
    let folder = dir.join("in");
    fs::create_dir(&folder).unwrap();
    for name in ["b10.pdf", "B2.PDF", "a.pdf", "notes.txt"] {
        fs::write(folder.join(name), b"%PDF-1.7 x").unwrap();
    }
    fs::create_dir(folder.join("dir.pdf")).unwrap();
    assert_eq!(host.pick_folder(), Err("cancelled"));
    assert_eq!(host.batch_read("a.pdf"), Err("not-listed"));
    answers(&mut host).batch_dir = Some(folder.clone());
    let listing = host.pick_folder().unwrap();
    assert_eq!(listing.out, folder.join(BATCH_OUT));
    let names: Vec<&str> = listing.files.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, vec!["a.pdf", "B2.PDF", "b10.pdf"]);
    assert_eq!(host.batch_read("a.pdf").unwrap(), b"%PDF-1.7 x");
    assert_eq!(host.batch_read("notes.txt"), Err("not-listed"));
    assert_eq!(host.batch_exists("a.pdf"), Ok(false));
    assert_eq!(host.batch_write("a.pdf", b"%PDF out"), Ok("a.pdf".into()));
    assert_eq!(host.batch_exists("a.pdf"), Ok(true));
    assert_eq!(host.batch_write("a.pdf", b"%PDF out"), Err("exists"));
    assert_eq!(
        host.batch_write("report.txt", b"done"),
        Ok("report.txt".into())
    );
    assert_eq!(host.batch_write("../x.pdf", b"x"), Err("bad-name"));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn links_and_print() {
    let (mut host, dir) = smoke_host("links");
    host.open_link("https://example.com");
    host.open_link("javascript:x");
    host.open_link(&"h".repeat(2049));
    assert_eq!(
        host.stdout_lines,
        vec![
            r#"P2_LINK {"allowed":true,"uri":"https://example.com"}"#.to_string(),
            r#"P2_LINK {"allowed":false,"uri":"javascript:x"}"#.to_string()
        ]
    );
    assert_eq!(host.last_p2_link(), Some(("javascript:x", false)));
    // Print without a target: no sheet in a headless run.
    assert_eq!(host.print(b"%PDF-1.4 print"), Err("cancelled"));
    answers(&mut host).print_to = Some(PathBuf::from("relative.pdf"));
    assert_eq!(host.print(b"%PDF-1.4 print"), Err("failed"));
    let out = dir.join("print.pdf");
    answers(&mut host).print_to = Some(out.clone());
    assert_eq!(host.print(b"%PDF-1.4 print"), Ok(()));
    assert_eq!(host.take_print_result(), Some(Ok(())));
    assert_eq!(fs::read(out).unwrap(), b"%PDF-1.4 print");
    fs::remove_dir_all(dir).unwrap();

    // In a window: asked first; no printer falls back to a save sheet.
    let (mut host, rec, dir) = gui_host("links-gui");
    host.open_link("javascript:x");
    host.open_link("https://a.example");
    rec.lock().unwrap().replies = vec![0];
    host.open_link("https://b.example");
    assert_eq!(
        rec.lock().unwrap().links,
        vec!["https://b.example".to_string()]
    );
    assert_eq!(host.print(b"%PDF-1.4 print"), Err("cancelled"));
    assert_eq!(
        rec.lock().unwrap().save_defaults,
        vec![PathBuf::from("document-print.pdf")]
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn stores_are_files_outside_a_smoke() {
    let (mut host, _rec, dir) = gui_host("stores");
    let path = copy_fixture(&dir, "hello.pdf");
    open(&mut host, &path);
    assert!(dir.join("udata/recent-files.json").is_file());
    assert!(host.signature_set(Some(
        &json!({"v": 1, "width": 20, "height": 10, "strokes": [[[0, 0], [5, 5]]]})
    )));
    assert!(dir.join("udata/drawn-signature.json").is_file());
    assert!(!host.signature_set(Some(&json!({"v": 3}))));
    assert!(host.signature_get().is_some());
    let (mut smoke, sdir) = smoke_host("stores-smoke");
    let p = copy_fixture(&sdir, "hello.pdf");
    open(&mut smoke, &p);
    assert!(smoke.signature_set(Some(
        &json!({"v": 1, "width": 20, "height": 10, "strokes": [[[0, 0], [5, 5]]]})
    )));
    assert!(!sdir.join("udata/recent-files.json").exists());
    assert!(!sdir.join("udata/drawn-signature.json").exists());
    fs::remove_dir_all(dir).unwrap();
    fs::remove_dir_all(sdir).unwrap();
}

#[test]
fn trust_list_through_the_host() {
    let (mut host, dir) = smoke_host("trust");
    assert_eq!(host.trust_add(None), Err("cancelled"));
    let ca = fixture("fixtures/p0b/.ca/ca.pem");
    assert_eq!(host.trust_add(Some(&ca.to_string_lossy())), Ok((1, 1)));
    assert_eq!(host.trust_add(Some(&ca.to_string_lossy())), Ok((0, 1)));
    let verified = host
        .verify_signatures(
            b"%PDF-1.7",
            &[json!({"field": "S", "page": 0, "empty": true})],
        )
        .unwrap();
    assert_eq!(verified["trusted"].as_array().unwrap().len(), 1);
    assert_eq!(verified["verdicts"].as_array().unwrap().len(), 1);
    assert!(verified["verdicts"][0]["plain"].is_string());
    let many = vec![json!({}); 201];
    assert_eq!(host.verify_signatures(b"%PDF", &many), Err("too-many"));
    assert!(host.trust_clear());
    let verified = host.verify_signatures(b"%PDF-1.7", &[json!(null)]).unwrap();
    assert_eq!(verified["trusted"], json!([]));
    assert_eq!(verified["verdicts"][0]["status"], json!("unsupported"));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn certificates_sign_once() {
    let (mut host, dir) = smoke_host("sign");
    assert_eq!(
        host.certificate_info("pdf4all"),
        Err(sign::SignFail::of("no-certificate"))
    );
    assert_eq!(host.pick_certificate(None), Err("cancelled"));
    let p12 = fixture("fixtures/p5/leaf.p12");
    assert_eq!(
        host.pick_certificate(Some(&p12.to_string_lossy())),
        Ok("leaf.p12".into())
    );
    assert_eq!(
        host.certificate_info("wrong").unwrap_err().reason,
        "bad-password"
    );
    assert!(!host.holding_certificate());
    let info = host.certificate_info("pdf4all").unwrap();
    assert!(!info.cn.is_empty());
    assert!(info.certs >= 1);
    assert!(host.holding_certificate());
    host.certificate_clear();
    assert!(!host.holding_certificate());
    host.certificate_info("pdf4all").unwrap();

    // Prepare hello.pdf for signing with the engine, then sign it.
    let now = crate::engine::now_ms();
    let mut engine = crate::engine::EngineHost::new();
    let pdf = fs::read(fixture("fixtures/hello.pdf")).unwrap();
    engine
        .request(
            json!({"op": "open", "name": "hello.pdf", "pdf": {"$buffer": 0}}),
            vec![pdf],
        )
        .unwrap();
    let mut prepared = engine
        .request(
            json!({"op": "sign-prepare", "name": "Tester", "reason": "unit", "page": 0, "rect": null, "certify": false, "when": now}),
            vec![],
        )
        .unwrap();
    let offsets = sign::SignOffsets {
        contents_at: prepared
            .get("contentsAt")
            .and_then(serde_json::Value::as_u64)
            .unwrap(),
        contents_len: prepared
            .get("contentsLen")
            .and_then(serde_json::Value::as_u64)
            .unwrap(),
        byte_range_at: prepared
            .get("byteRangeAt")
            .and_then(serde_json::Value::as_u64)
            .unwrap(),
    };
    let bytes = prepared.take_bytes("bytes").unwrap();
    // A signing time far from now is refused (and the key is spent).
    assert_eq!(
        host.sign_file(bytes.clone(), offsets, now - 3_600_000.0, None)
            .unwrap_err()
            .reason,
        "invalid"
    );
    assert_eq!(
        host.sign_file(bytes.clone(), offsets, now, None)
            .unwrap_err()
            .reason,
        "no-certificate"
    );
    host.certificate_info("pdf4all").unwrap();
    // No target: cancelled, and the key is gone.
    assert_eq!(
        host.sign_file(bytes.clone(), offsets, now, Some("hello-signed"))
            .unwrap_err()
            .reason,
        "cancelled"
    );
    host.certificate_info("pdf4all").unwrap();
    let out = dir.join("hello-signed.pdf");
    answers(&mut host).save_to = Some(out.clone());
    let signed = host
        .sign_file(bytes, offsets, now, Some("hello-signed"))
        .unwrap();
    assert_eq!(signed.name, "hello-signed.pdf");
    assert!(!signed.opened);
    assert!(!host.holding_certificate());
    let written = fs::read(&out).unwrap();
    // The signed copy verifies.
    engine
        .request(
            json!({"op": "open", "name": "hello-signed.pdf", "pdf": {"$buffer": 0}}),
            vec![written.clone()],
        )
        .unwrap();
    let list = engine.request(json!({"op": "sign-list"}), vec![]).unwrap();
    let mut entries: Vec<serde_json::Value> = list
        .get("signatures")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    for entry in &mut entries {
        if let Some(bytes) = list.bytes_at(entry.get("contents")) {
            entry["contents"] = json!(bytes.iter().map(|b| format!("{b:02x}")).collect::<String>());
        }
    }
    assert_eq!(entries.len(), 1, "{}", list.value);
    let verified = host.verify_signatures(&written, &entries).unwrap();
    assert_eq!(
        verified["verdicts"][0]["status"],
        json!("valid"),
        "{verified}"
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn single_instance_lock() {
    let dir = scratch("lock");
    let udata = dir.join("udata");
    let mut first = SingletonLock::acquire(&udata)
        .unwrap()
        .expect("first instance takes the lock");
    assert!(udata.join("SingletonLock").exists());
    assert!(
        SingletonLock::acquire(&udata).unwrap().is_none(),
        "a second instance must not"
    );
    first.release();
    assert!(!udata.join("SingletonLock").exists());
    let again = SingletonLock::acquire(&udata).unwrap();
    assert!(again.is_some());
    drop(again);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn fixtures_resolve() {
    assert!(is_fixture("hello.pdf") && !is_fixture("../x.pdf"));
    assert!(fixture_path("hello.pdf").is_some());
    assert!(fixture_path("nope.pdf").is_none());
    let (host, dir) = smoke_host("fixtures");
    assert!(host.fixture_bytes("form.pdf").unwrap().starts_with(b"%PDF"));
    fs::remove_dir_all(dir).unwrap();
}
