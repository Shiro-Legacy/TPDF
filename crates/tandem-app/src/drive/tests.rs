//! The harness against a stub session: argument parsing, `${OUT}`,
//! `matches`, step records, bad lines, close, secrets, timeouts, pre-answers,
//! the smoke flows and the report shape.

use super::*;
use crate::host::util;
use crate::platform::Scripted;
use std::fs;

/// A session that does just enough: opens record the name, edits record the
/// op (`note` and `undo` answer `P2_EDIT`), saves go through the host.
struct Stub {
    host: Host,
    lines: Vec<(String, Value)>,
    doc: Option<Document>,
    bytes: Vec<u8>,
    ops: Vec<String>,
    last_contents: Option<String>,
    error: String,
    notes: u32,
}

impl Stub {
    fn new(tag: &str) -> (Stub, PathBuf) {
        let dir = util::mkdtemp(&util::temp_dir().join(format!("tandem-drive-{tag}-"))).unwrap();
        let dir = fs::canonicalize(dir).unwrap();
        let host = Host::new(
            HostConfig {
                user_data: dir.join("udata"),
                smoke: true,
                echo_stdout: false,
            },
            Box::new(Scripted::default()),
        );
        let stub = Stub {
            host,
            lines: vec![],
            doc: None,
            bytes: vec![],
            ops: vec![],
            last_contents: None,
            error: String::new(),
            notes: 0,
        };
        (stub, dir)
    }
}

impl Driven for Stub {
    fn start(&mut self, fixture: Option<&str>) {
        let Some(name) = fixture else { return };
        self.lines.push((
            "P0_RESULT".into(),
            json!({"width": 612, "height": 792, "pixelWidth": 612, "pixelHeight": 792, "text": "PDF4All hello", "nonWhite": 900, "openMs": 3, "firstPageRenderMs": 4}),
        ));
        self.lines
            .push(("P1_RESULT".into(), json!({"name": name, "pageCount": 1})));
    }
    fn deliver_document(&mut self, doc: Document) {
        self.bytes = self.host.read_open(&doc.url).unwrap_or_default();
        self.lines.push((
            "P1_RESULT".into(),
            json!({"name": doc.name, "pageCount": 1}),
        ));
        self.error.clear();
        self.doc = Some(doc);
    }
    fn open_error(&mut self, error: OpenError) {
        self.error = if error.reason == "cancelled" {
            String::new()
        } else {
            format!("open: {}", error.reason)
        };
    }
    fn save_request(&mut self, request: SaveRequest) {
        let Some(doc) = self.doc.clone() else { return };
        let options = host::SaveOptions {
            in_place: request == SaveRequest::InPlace,
            rebind: request == SaveRequest::As,
            ..Default::default()
        };
        match self.host.save_file(&self.bytes, &doc.url, &options) {
            Ok(ok) => self.lines.push((
                "P1_SAVE".into(),
                json!({"name": ok.name, "bytes": self.bytes.len(), "validated": true}),
            )),
            Err(e) => {
                self.error = format!("save: {}", e.as_str());
                self.lines
                    .push(("P1_SAVE_ERROR".into(), json!({"reason": e.as_str()})));
            }
        }
    }
    fn edit_request(&mut self, op: &str, contents: Option<String>) {
        self.ops.push(op.to_string());
        self.last_contents = contents.clone();
        match op {
            "note" => {
                self.notes += 1;
                self.lines
                    .push(("P2_EDIT".into(), json!({"notes": self.notes})));
            }
            "undo" => {
                self.notes = self.notes.saturating_sub(1);
                self.lines
                    .push(("P2_EDIT".into(), json!({"notes": self.notes})));
            }
            "link" => {
                if let Some(uri) = contents {
                    self.host.open_link(&uri);
                }
            }
            "insert" => {
                if let Err(reason) = self.host.insert_source() {
                    self.error = reason.into();
                }
            }
            _ => {}
        }
    }
    fn print_request(&mut self, range: Option<String>) {
        self.ops
            .push(format!("print:{}", range.unwrap_or_default()));
    }
    fn state(&mut self) -> Value {
        json!({
            "doc": self.doc.as_ref().map(|d| json!({"name": d.name, "untitled": d.untitled})),
            "ops": self.ops, "contents": self.last_contents, "error": self.error, "notes": self.notes,
            "pages": {"count": 1, "rotate": [0]},
        })
    }
    fn take_lines(&mut self) -> Vec<(String, Value)> {
        std::mem::take(&mut self.lines)
    }
    fn host(&mut self) -> &mut Host {
        &mut self.host
    }
}

fn argv(items: &[&str]) -> Vec<String> {
    std::iter::once("tandem-headless")
        .chain(items.iter().copied())
        .map(str::to_string)
        .collect()
}

fn file_drive(stub: &mut Stub, out: &str, text: &str) -> Finish {
    let lines = parse_script(&with_out(text, out)).unwrap();
    run_drive(
        stub,
        "hello.pdf",
        out,
        Script::Lines(lines),
        std::io::empty(),
        &mut |_| {},
    )
}

fn live_drive(stub: &mut Stub, out: &str, text: &str) -> (Finish, Vec<Value>) {
    let mut emitted = Vec::new();
    let done = run_drive(
        stub,
        "hello.pdf",
        out,
        Script::Live,
        text.as_bytes(),
        &mut |s| emitted.push(s.clone()),
    );
    (done, emitted)
}

fn steps(done: &Finish) -> Vec<Value> {
    done.result
        .get("steps")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn hello() -> String {
    util::normalize(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/hello.pdf"))
        .to_string_lossy()
        .into_owned()
}

#[test]
fn parses_the_command_line() {
    let a = Args::parse(&argv(&[
        "--user-data-dir=/u",
        ".",
        "--drive",
        "s.jsonl",
        "--drive-out=/o",
        "--report=/r.json",
        "--gui",
        "--remote-allow-origins=*",
        "--password",
        "pw",
        "--new-passwords=a:b",
        "--save-to=x.pdf",
        "--images=a.png,b.png",
        "--expect=P1_THUMBS",
    ]));
    assert_eq!(a.drive.as_deref(), Some("s.jsonl"));
    assert!(a.smoke && a.gui && a.expect_thumbs);
    assert_eq!(a.drive_out.as_deref(), Some("/o"));
    assert_eq!(a.report.as_deref(), Some("/r.json"));
    assert_eq!(a.user_data_dir.as_deref(), Some("/u"));
    assert_eq!(a.fixture, "hello.pdf");
    assert_eq!(a.password.as_deref(), Some("pw"));
    assert_eq!(a.smoke_passwords(), vec!["pw", "a", "b"]);
    assert_eq!(a.answers().images.as_deref(), Some("a.png,b.png"));
    // `--name value` needs a value that does not start with `-`.
    assert_eq!(Args::parse(&argv(&["--drive", "--gui"])).drive, None);
    assert_eq!(
        Args::parse(&argv(&["--drive=-"])).drive.as_deref(),
        Some("-")
    );
    // `=`-only flags ignore the space form.
    assert_eq!(Args::parse(&argv(&["--report", "/r"])).report, None);
    assert!(!Args::parse(&argv(&["."])).smoke);
}

#[test]
fn startup_validation_order() {
    let err = |items: &[&str]| Args::parse(&argv(items)).startup_error();
    assert_eq!(err(&["--smoke"]), None);
    assert_eq!(
        err(&["--smoke", "--fixture=nope.pdf", "--save-in-place"]),
        Some("unknown fixture")
    );
    assert_eq!(
        err(&[
            "--smoke",
            "--print-to=/p",
            "--save-in-place",
            "--save-to=/s"
        ]),
        Some("print and save together")
    );
    assert_eq!(
        err(&["--smoke", "--save-to=/s", "--save-in-place"]),
        Some("save-to and save-in-place together")
    );
    assert_eq!(
        err(&["--smoke", "--save-in-place"]),
        Some("save-in-place needs --open")
    );
    assert_eq!(err(&["--smoke", "--edit=bogus"]), Some("unknown edit"));
    assert_eq!(err(&["--smoke", "--edit=note"]), Some("edit needs --open"));
    assert_eq!(
        err(&["--smoke", "--open=/a.pdf", "--edit=assemble"]),
        Some("assemble needs --insert-from")
    );
    assert_eq!(
        err(&["--smoke", "--open=/a.pdf", "--edit=shapes"]),
        Some("shapes needs --images")
    );
    assert_eq!(
        err(&["--smoke", "--open=/a.pdf", "--edit=extract"]),
        Some("extract needs --save-to")
    );
    assert_eq!(
        err(&["--smoke", "--open=/a.pdf", "--edit=export-png"]),
        Some("export needs --save-to")
    );
    assert_eq!(
        err(&["--smoke", "--open=/a.pdf", "--edit=encrypt", "--save-to=/s"]),
        Some("encrypt needs --new-passwords and --save-to")
    );
    assert_eq!(
        err(&[
            "--smoke",
            "--open=/a.pdf",
            "--edit=encrypt-keep",
            "--save-to=/s"
        ]),
        Some("encrypt-keep needs --password and --save-to")
    );
    assert_eq!(
        err(&["--smoke", "--new-from-images", "--images=a"]),
        Some("new-from-images needs --images and --save-to")
    );
    assert_eq!(
        err(&[
            "--smoke",
            "--new-from-images",
            "--images=a",
            "--save-to=/s",
            "--open=/a.pdf"
        ]),
        Some("new-from-images is its own smoke")
    );
}

#[test]
fn smoke_payloads() {
    let a = Args::parse(&argv(&[
        "--smoke",
        "--edit=encrypt",
        "--new-passwords=u:o:x",
    ]));
    let (op, contents) = a.smoke_edit_payload();
    assert_eq!(op, "encrypt");
    let body: Value = serde_json::from_str(&contents.unwrap()).unwrap();
    assert_eq!(
        body,
        json!({"user": "u", "owner": "o:x", "permissions": ["print", "annotate"]})
    );
    let a = Args::parse(&argv(&[
        "--smoke",
        "--edit=encrypt",
        "--new-passwords=same",
    ]));
    let body: Value = serde_json::from_str(&a.smoke_edit_payload().1.unwrap()).unwrap();
    assert_eq!(body["owner"], json!("same"));
    assert_eq!(
        Args::parse(&argv(&["--edit=pages"])).smoke_edit_payload(),
        ("pages".into(), None)
    );
    assert_eq!(
        Args::parse(&argv(&["--edit=content"])).smoke_edit_payload(),
        ("content".into(), None)
    );
    assert_eq!(
        Args::parse(&argv(&["--edit=note"])).smoke_edit_payload(),
        ("note".into(), Some("PDF4All P2 note".into()))
    );
}

#[test]
fn out_substitution() {
    assert_eq!(
        with_out(r#"{"path":"${OUT}/a.pdf"}"#, "/tmp/o"),
        r#"{"path":"/tmp/o/a.pdf"}"#
    );
    assert_eq!(
        with_out(r#"{"p":"${OUT}","q":"${OUT}/b"}"#, "/x"),
        r#"{"p":"/x","q":"/x/b"}"#
    );
    // JSON-escaped: a quote or backslash in the folder stays valid JSON.
    let text = with_out(r#"{"p":"${OUT}/c d"}"#, r#"/we"ird\dir"#);
    assert_eq!(
        serde_json::from_str::<Value>(&text).unwrap()["p"],
        json!(r#"/we"ird\dir/c d"#)
    );
    // The rest stops at a backslash (an escape in the line) and at a line end.
    assert_eq!(with_out("${OUT}/a\\nb", "/o"), "/o/a\\nb");
    assert_eq!(with_out("# ${OUT}/x\n{}", "/o"), "# /o/x\n{}");
    assert_eq!(with_out("no key", "/o"), "no key");
}

#[test]
fn expect_matching() {
    let got = json!({"a": 1, "b": [1, 2], "c": {"d": null, "e": "x"}, "f": 90.0});
    assert!(matches(Some(&got), &json!({"a": 1})));
    assert!(matches(Some(&got), &json!({"a": 1.0, "f": 90})));
    assert!(!matches(Some(&got), &json!({"a": "1"})));
    assert!(matches(Some(&got), &json!({"b": [1, 2]})));
    assert!(!matches(Some(&got), &json!({"b": [1]})));
    assert!(matches(Some(&got), &json!({"b": {"0": 1, "length": 2}})));
    assert!(matches(Some(&got), &json!({"c": {"d": null}})));
    assert!(!matches(Some(&got), &json!({"missing": null})));
    assert!(!matches(Some(&got), &json!({"c": {"e": {"f": 1}}})));
    assert!(matches(Some(&got), &json!({})));
    assert!(!matches(None, &json!({})));
    assert!(!matches(None, &json!(null)));
    assert!(matches(Some(&json!(null)), &json!(null)));
    assert!(!matches(Some(&json!([1])), &json!(null)));
}

#[test]
fn preload_caps() {
    assert_eq!(preload_edit("", None), None);
    assert_eq!(preload_edit("note", None), Some(None));
    assert_eq!(
        preload_edit("note", Some("x".repeat(4096))),
        Some(Some("x".repeat(4096)))
    );
    assert_eq!(preload_edit("note", Some("x".repeat(4097))), None);
    assert_eq!(
        preload_edit("sign-draw", Some("x".repeat(5000))).map(|c| c.map(|s| s.len())),
        Some(Some(5000))
    );
    assert_eq!(
        preload_edit("sign-draw", Some("x".repeat(MAX_SIGN_DRAW_CHARS + 1))),
        Some(Some(String::new()))
    );
}

#[test]
fn file_drive_steps_and_expects() {
    let (mut stub, dir) = Stub::new("file");
    let out = dir.join("out");
    fs::create_dir(&out).unwrap();
    fs::copy(hello(), out.join("in.pdf")).unwrap();
    let out_text = out.to_string_lossy().into_owned();
    let script = r#"
        # a comment with ${OUT}
        {"cmd":"open","path":"${OUT}/in.pdf","expect":{"doc":{"name":"in.pdf"}}}
        {"cmd":"note","contents":{"text":"hi"},"expect":{"notes":1}}
        {"expect":{"notes":2}}
        {"note":"no cmd, no expect"}
        5
        {"cmd":"save-to","path":"${OUT}/copy.pdf","expect":{"error":""}}
        {"cmd":"save-to","expect":{"error":"save: cancelled"}}
        {"cmd":"state","to":"${OUT}/x.pdf","expect":{"ops":["note","state"]}}
        {"cmd":"close"}
        {"cmd":"note","expect":{"notes":9}}
    "#;
    let done = file_drive(&mut stub, &out_text, script);
    let steps = steps(&done);
    assert!(!done.ok, "the bare expect fails");
    assert_eq!(done.result["out"], json!(out_text));
    assert!(done.result["memory"].is_array());
    assert_eq!(steps.len(), 8);
    assert_eq!(steps[0]["cmd"], json!("open"));
    assert_eq!(steps[0]["path"], json!(format!("{out_text}/in.pdf")));
    assert_eq!(steps[0]["ok"], json!(true));
    assert_eq!(steps[0]["state"]["doc"]["name"], json!("in.pdf"));
    // Object contents go to the session as JSON text; the record keeps the object.
    assert_eq!(steps[1]["contents"], json!({"text": "hi"}));
    assert_eq!(steps[1]["state"]["contents"], json!(r#"{"text":"hi"}"#));
    assert_eq!(steps[2], json!({"expect": {"notes": 2}, "ok": false}));
    assert_eq!(steps[3], json!({"note": "no cmd, no expect"}));
    assert_eq!(steps[4], json!({}));
    assert_eq!(steps[5]["ok"], json!(true));
    assert!(out.join("copy.pdf").is_file());
    // `to` answers one line only: the next save has no target.
    assert_eq!(steps[6]["ok"], json!(true));
    assert_eq!(steps[7]["ok"], json!(true));
    assert_eq!(stub.ops, vec!["note", "state"]);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn live_drive_bad_lines_and_eof() {
    let (mut stub, dir) = Stub::new("live");
    let text = "\n# skipped\nnot json\n[1,2]\n{\"cmd\":\"state\"}\n  {\"cmd\":\"note\",\"expect\":{\"notes\":1}}  \n";
    let (done, emitted) = live_drive(&mut stub, "/o", text);
    assert!(done.ok);
    assert_eq!(emitted.len(), 4);
    assert!(
        emitted[0]["error"]
            .as_str()
            .unwrap()
            .starts_with("bad line: ")
    );
    assert_eq!(emitted[1], json!({"error": "bad line: not a JSON object"}));
    assert_eq!(emitted[2]["cmd"], json!("state"));
    assert_eq!(emitted[3]["ok"], json!(true));
    assert_eq!(steps(&done), emitted);
    // close ends the session; nothing after it runs and close has no record.
    let (mut stub, dir2) = Stub::new("live-close");
    let (done, emitted) = live_drive(&mut stub, "/o", "{\"cmd\":\"close\"}\n{\"cmd\":\"note\"}\n");
    assert!(done.ok);
    assert!(emitted.is_empty());
    assert!(stub.ops.is_empty());
    fs::remove_dir_all(dir).unwrap();
    fs::remove_dir_all(dir2).unwrap();
}

#[test]
fn dropped_request_is_a_step_timeout() {
    let (mut stub, dir) = Stub::new("timeout");
    let long = "x".repeat(MAX_NOTE_CHARS + 1);
    let text = format!(
        "{{\"cmd\":\"state\"}}\n{{\"cmd\":\"note\",\"contents\":\"{long}\"}}\n{{\"cmd\":\"state\"}}\n"
    );
    let (done, emitted) = live_drive(&mut stub, "/o", &text);
    assert!(!done.ok);
    assert_eq!(done.result["error"], json!("step timeout"));
    let steps = steps(&done);
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[1]["error"], json!("step timeout"));
    assert!(steps[1].get("state").is_none());
    assert_eq!(emitted.len(), 2);
    // A non-string cmd never reaches the session either.
    let (mut stub, dir2) = Stub::new("timeout2");
    let done = file_drive(&mut stub, "/o", r#"{"cmd":5}"#);
    assert_eq!(done.result["error"], json!("step timeout"));
    fs::remove_dir_all(dir).unwrap();
    fs::remove_dir_all(dir2).unwrap();
}

#[test]
fn secrets_are_redacted_and_a_leak_fails() {
    let (mut stub, dir) = Stub::new("secrets");
    let text = r#"
        {"cmd":"password","contents":"hunter2"}
        {"cmd":"encrypt","contents":{"user":"u-secret","owner":"","permissions":[]}}
        {"cmd":"sign-draw","contents":{"strokes":[[[1,2],[3,4]]]}}
        {"cmd":"state"}
        {"cmd":"find","contents":{"query":"hunter2"}}
        {"cmd":"state"}
    "#;
    let lines = parse_script(text).unwrap();
    let done = run_drive(
        &mut stub,
        "hello.pdf",
        "/o",
        Script::Lines(lines),
        std::io::empty(),
        &mut |_| {},
    );
    // The password step's own STATE carries the contents in this stub, so it leaks at once.
    assert_eq!(done.result["error"], json!("password leaked in STATE"));
    assert!(!done.result.contains_key("memory"));
    assert!(
        !Value::Object(done.result.clone())
            .to_string()
            .contains("hunter2")
    );
    // A session that does not echo contents: the leak comes with `find`.
    struct Quiet(Stub);
    impl Driven for Quiet {
        fn start(&mut self, f: Option<&str>) {
            self.0.start(f)
        }
        fn deliver_document(&mut self, d: Document) {
            self.0.deliver_document(d)
        }
        fn open_error(&mut self, e: OpenError) {
            self.0.open_error(e)
        }
        fn save_request(&mut self, r: SaveRequest) {
            self.0.save_request(r)
        }
        fn edit_request(&mut self, op: &str, contents: Option<String>) {
            let shown = if op == "find" { contents.clone() } else { None };
            self.0.edit_request(op, contents);
            self.0.last_contents = shown;
        }
        fn print_request(&mut self, r: Option<String>) {
            self.0.print_request(r)
        }
        fn state(&mut self) -> Value {
            self.0.state()
        }
        fn take_lines(&mut self) -> Vec<(String, Value)> {
            self.0.take_lines()
        }
        fn host(&mut self) -> &mut Host {
            self.0.host()
        }
    }
    let (stub, dir2) = Stub::new("secrets2");
    let mut quiet = Quiet(stub);
    let lines = parse_script(text).unwrap();
    let done = run_drive(
        &mut quiet,
        "hello.pdf",
        "/o",
        Script::Lines(lines),
        std::io::empty(),
        &mut |_| {},
    );
    assert_eq!(done.result["error"], json!("password leaked in STATE"));
    let steps = steps(&done);
    assert_eq!(steps.len(), 4, "the leaking step is not recorded");
    assert_eq!(steps[0]["contents"], json!("[redacted]"));
    assert_eq!(steps[1]["contents"], json!("[redacted]"));
    assert_eq!(steps[2]["contents"], json!("[redacted]"));
    assert!(!Value::Array(steps.clone()).to_string().contains("u-secret"));
    assert_eq!(drive_secrets("password", Some(&json!(""))), (true, vec![]));
    assert_eq!(
        drive_secrets("sign", Some(&json!(r#"{"password":"p","user":"x"}"#))),
        (true, vec!["p".to_string()])
    );
    assert_eq!(
        drive_secrets("encrypt", Some(&json!("not json"))),
        (false, vec![])
    );
    assert_eq!(
        drive_secrets("note", Some(&json!({"password": "p"}))),
        (false, vec![])
    );
    fs::remove_dir_all(dir).unwrap();
    fs::remove_dir_all(dir2).unwrap();
}

#[test]
fn line_pre_answers() {
    let (mut stub, dir) = Stub::new("answers");
    let from = hello();
    // As a JSON string: a Windows path's backslashes must be escaped.
    let from_json = json!(from);
    let text = format!(
        r#"
        {{"cmd":"state","to":"rel/out.pdf","from":{from_json},"folder":"in","filled":"over","images":["a.png"," b.png "]}}
        {{"cmd":"state","images":"c.png, d.png"}}
        {{"cmd":"insert"}}
        {{"cmd":"new","contents":{{"size":"a4"}},"expect":{{"doc":{{"name":"Untitled.pdf","untitled":true}}}}}}
        {{"cmd":"new","contents":"[oops","expect":{{"error":"open: unreadable"}}}}
        {{"cmd":"open","expect":{{"error":""}}}}
        {{"cmd":"print-to","path":"p.pdf","range":"1-2"}}
        {{"cmd":"link","contents":"https://example.com"}}
        {{"cmd":"wait","ms":"1"}}
    "#
    );
    let lines = parse_script(&text).unwrap();
    let done = run_drive(
        &mut stub,
        "hello.pdf",
        "/o",
        Script::Lines(lines),
        std::io::empty(),
        &mut |_| {},
    );
    assert!(done.ok, "{:?}", done.result);
    let cwd = std::env::current_dir().unwrap();
    let answers = stub.host.platform.answers().unwrap().clone();
    assert_eq!(answers.save_to, None, "`to` is per line");
    assert_eq!(
        answers.insert_from.as_deref(),
        Some(Path::new(&from)),
        "`from` is sticky"
    );
    assert_eq!(answers.batch_dir, Some(cwd.join("in")));
    assert!(!answers.filled_over, "`filled` is per line");
    assert_eq!(
        answers.images,
        Some(format!(
            "{},{}",
            cwd.join("c.png").display(),
            cwd.join("d.png").display()
        ))
    );
    assert_eq!(answers.print_to, Some(cwd.join("p.pdf")));
    assert_eq!(
        stub.ops[stub.ops.len() - 3..],
        ["print:1-2".to_string(), "link".into(), "state".into()]
    );
    assert_eq!(
        stub.host.last_p2_link(),
        Some(("https://example.com", true))
    );
    assert_eq!(stub.error, "");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn smoke_open_flows() {
    let (mut stub, dir) = Stub::new("smoke-open");
    let src = dir.join("in.pdf");
    fs::copy(hello(), &src).unwrap();
    let src_text = src.to_string_lossy().into_owned();
    // Open only.
    let args = Args::parse(&argv(&["--smoke", &format!("--open={src_text}")]));
    let done = smoke_open(&mut stub, &args);
    assert!(done.ok, "{:?}", done.result);
    assert_eq!(done.result["open"]["name"], json!("in.pdf"));
    // An edit, then the undo check.
    let (mut stub, dir2) = Stub::new("smoke-edit");
    let args = Args::parse(&argv(&[
        "--smoke",
        &format!("--open={src_text}"),
        "--edit=note",
    ]));
    let done = smoke_open(&mut stub, &args);
    assert!(done.ok, "{:?}", done.result);
    assert_eq!(done.result["undo"], json!({"notes": 0}));
    assert_eq!(stub.ops, vec!["note", "undo"]);
    // A copy.
    let (mut stub, dir3) = Stub::new("smoke-save");
    let target = dir.join("copy.pdf");
    let args = Args::parse(&argv(&[
        "--smoke",
        &format!("--open={src_text}"),
        &format!("--save-to={}", target.display()),
    ]));
    stub.host.platform.answers().unwrap().save_to = Some(target.clone());
    let done = smoke_open(&mut stub, &args);
    assert!(done.ok, "{:?}", done.result);
    assert_eq!(done.result["sourceUnchanged"], json!(true));
    assert_eq!(done.result["save"]["name"], json!("copy.pdf"));
    // Nothing answers a print: the run ends as a smoke timeout.
    let (mut stub, dir4) = Stub::new("smoke-print");
    let args = Args::parse(&argv(&[
        "--smoke",
        &format!("--open={src_text}"),
        "--print-to=/nowhere.pdf",
    ]));
    let done = smoke_open(&mut stub, &args);
    assert_eq!(done.result["error"], json!("smoke timeout"));
    for d in [dir, dir2, dir3, dir4] {
        fs::remove_dir_all(d).unwrap();
    }
}

#[test]
fn plain_smoke_and_report_shape() {
    let (mut stub, dir) = Stub::new("plain");
    let args = Args::parse(&argv(&["--smoke"]));
    let done = smoke_plain(&mut stub, &args);
    assert!(done.ok, "{:?}", done.result);
    assert_eq!(
        done.result["roundtrip"]["readback"]["fieldValue"],
        json!("PDF4All P0")
    );
    assert_eq!(done.result["networkProbeDenied"], json!(true));
    let startup = &done.result["startup"];
    assert!(
        startup["firstPageAtMs"].as_u64().unwrap() > startup["mainStartedAtMs"].as_u64().unwrap()
    );
    let r = report(&done, Some(("mailto:x", true)));
    for key in [
        "ok",
        "packaged",
        "platform",
        "arch",
        "versions",
        "elapsedMs",
        "localRequests",
        "protocolRequests",
        "blockedRequests",
        "renderer",
        "fixture",
    ] {
        assert!(r.get(key).is_some(), "{key}");
    }
    assert_eq!(r["p2Link"], json!({"uri": "mailto:x", "allowed": true}));
    assert!(report(&Finish::error("x"), None).get("p2Link").is_none());
    assert_eq!(report(&Finish::error("x"), None)["ok"], json!(false));
    // The unwired stand-in fails cleanly.
    let mut unwired = Unwired::new(Host::new(
        HostConfig {
            user_data: dir.join("u2"),
            smoke: true,
            echo_stdout: false,
        },
        Box::new(Scripted::default()),
    ));
    let done = smoke_plain(&mut unwired, &args);
    assert!(
        done.result["error"]
            .as_str()
            .unwrap()
            .starts_with("P0_ERROR ")
    );
    let done = run_drive(
        &mut unwired,
        "hello.pdf",
        "/o",
        Script::Lines(vec![]),
        std::io::empty(),
        &mut |_| {},
    );
    assert!(!done.ok);
    fs::remove_dir_all(dir).unwrap();
}
