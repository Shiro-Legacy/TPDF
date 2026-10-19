//! Tandem PDF as an MCP server: `tandem-pdf --mcp [--out=<dir>]` (and `tandem-headless --mcp`).
//! The port of scripts/tandem-mcp.mjs (contract-agent-session.md D3): stdio, newline-delimited
//! JSON-RPC 2.0, the same six tools with the same schemas and replies. The drive session runs
//! in this process ([`DriveRun`] over [`Driven`]), not as a child app: it starts on the first
//! tool call that needs it, starts again after it ended (a step timeout, a leaked password),
//! and closes at stdin EOF. Register: `claude mcp add tandem-pdf -- <app binary> --mcp`.
//!
//! `--out=<dir>`: where relative `tandem_save` paths and `${OUT}` land (default a new temp
//! folder, kept after the server stops so saved files stay readable). The agent reference
//! (docs/agent.md, tool `tandem_help`) is compiled in.
//!
//! Nothing but JSON-RPC replies is written to stdout; the session's console lines are not echoed.

use crate::drive::{DriveRun, Driven, Outcome};
use crate::host::{self, Host, HostConfig};
use crate::platform::{Answers, Scripted};
use serde_json::{Map, Value, json};
use std::io::{BufRead, Write};
use std::path::{Component, Path, PathBuf};

/// The agent reference `tandem_help` serves.
pub const AGENT_DOC: &str = include_str!("../../../docs/agent.md");
/// The protocol version answered when `initialize` names none.
const PROTOCOL: &str = "2025-06-18";
/// The startup sample every session opens first (`--fixture` default).
const FIXTURE: &str = "hello.pdf";
/// The session's end when it stopped under a tool call (the Node server's child-exit text).
const ENDED: &str = "session ended (exit 1)";

const READ_BACK: &str = "The result is the drive STEP record {cmd, …, state}; state is the app's STATE after the step (doc.name, doc.pageCount, revision, dirty, error, announcement, content.edited/refused, fields.values/last, save).";

/// The six tools as `tools/list` gives them: `{name, description, inputSchema}`.
pub fn tools() -> Value {
    json!([
        {
            "name": "tandem_open",
            "description": format!("Open a PDF from disk (relative paths resolve against this server's working directory). Read state.doc {{name, pageCount}} and state.error (\"\" on success). {READ_BACK}"),
            "inputSchema": {"type": "object", "properties": {"path": {"type": "string", "description": "Path of the PDF to open"}}, "required": ["path"]},
        },
        {
            "name": "tandem_new",
            "description": format!("Create a blank one-page document named Untitled.pdf (letter 612x792 pt by default, \"a4\", or [width, height] in points, each 72-14400) and make it the open document. Add text with tandem_do text-add and fields with tandem_do fields, then tandem_save. Read state.doc.name (\"Untitled.pdf\") and state.error. {READ_BACK}"),
            "inputSchema": {"type": "object", "properties": {"size": {"oneOf": [{"type": "string", "enum": ["letter", "a4"]}, {"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2}]}}},
        },
        {
            "name": "tandem_do",
            "description": format!("Run one drive step on the open document: {{cmd, contents?, path?, to?, from?, range?, ms?}} exactly as a drive script line (e.g. {{\"cmd\":\"text-add\",\"contents\":{{\"page\":0,\"point\":[72,700],\"text\":\"Hi\",\"font\":\"Helvetica\",\"size\":12,\"color\":[0,0,0]}}}}, {{\"cmd\":\"fields\",\"contents\":{{\"action\":\"fill\",\"name\":\"x\",\"value\":\"y\"}}}}, {{\"cmd\":\"content\",\"contents\":{{\"target\":\"old text\",\"text\":\"new text\"}}}}, {{\"cmd\":\"undo\"}}). Call tandem_help with topic \"ops\" for the op list, refusal tokens and ceilings. Check state.error, state.revision and the op's own slice. {READ_BACK}"),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "cmd": {"type": "string"}, "contents": {"type": ["object", "string"]}, "path": {"type": "string"}, "to": {"type": "string"},
                    "from": {"type": "string"}, "range": {"type": "string"}, "ms": {"type": "number"},
                },
                "required": ["cmd"],
            },
        },
        {
            "name": "tandem_state",
            "description": format!("Read the current STATE without changing anything. {READ_BACK}"),
            "inputSchema": {"type": "object", "properties": {}},
        },
        {
            "name": "tandem_save",
            "description": format!("Save a copy of the open document to a path (a relative path lands in this server's output folder, --out, and missing folders under it are created; it must stay inside --out; an absolute path needs an existing folder). Read state.save {{ok, name}} (ok false carries a reason). {READ_BACK}"),
            "inputSchema": {"type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]},
        },
        {
            "name": "tandem_help",
            "description": "The agent reference (docs/agent.md): topic \"create\", \"modify\", \"fill\" or \"ops\"; the whole file without a topic. Read it before the first edit.",
            "inputSchema": {"type": "object", "properties": {"topic": {"type": "string", "enum": ["create", "modify", "fill", "ops"]}}},
        },
    ])
}

/// `help(topic)`: the section `## <topic>` (case-insensitive, to the next `## `), or the whole
/// reference for no topic, an unknown topic or a missing section.
pub fn help(topic: Option<&str>) -> &'static str {
    let Some(topic) = topic.filter(|t| ["create", "modify", "fill", "ops"].contains(t)) else {
        return AGENT_DOC;
    };
    let heading = |line: &str| {
        line.strip_prefix("## ").is_some_and(|rest| {
            rest.len() >= topic.len()
                && rest.is_char_boundary(topic.len())
                && rest[..topic.len()].eq_ignore_ascii_case(topic)
                // `\b`: the topic is not followed by another word character.
                && !rest[topic.len()..]
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
        })
    };
    let mut offset = 0;
    let mut at = None;
    for line in AGENT_DOC.split_inclusive('\n') {
        if at.is_none() && heading(line) {
            at = Some(offset);
        } else if at.is_some() && line.starts_with("## ") {
            return &AGENT_DOC[at.unwrap_or(0)..offset];
        }
        offset += line.len();
    }
    at.map_or(AGENT_DOC, |at| &AGENT_DOC[at..])
}

/// The live session: the document session, its drive and its own user-data folder.
struct Live<D> {
    session: D,
    run: DriveRun,
    dir: PathBuf,
}

impl<D> Drop for Live<D> {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The server: the `--out` folder (made on first use) and the live session.
pub struct Server<D, F> {
    make: F,
    out_arg: Option<String>,
    out: Option<PathBuf>,
    live: Option<Live<D>>,
}

impl<D: Driven, F: Fn(Host) -> D> Server<D, F> {
    /// `make` builds the document session around a host; `out` is `--out`.
    pub fn new(make: F, out: Option<String>) -> Self {
        Server {
            make,
            out_arg: out,
            out: None,
            live: None,
        }
    }

    /// The output folder: `--out` resolved, or a new `tandem-mcp-*` temp folder.
    fn out(&mut self) -> std::io::Result<PathBuf> {
        if let Some(out) = &self.out {
            return Ok(out.clone());
        }
        let out = match &self.out_arg {
            Some(dir) => host::util::resolve(dir),
            None => host::util::mkdtemp(&host::util::temp_dir().join("tandem-mcp-"))?,
        };
        self.out = Some(out.clone());
        Ok(out)
    }

    /// The live session, started when there is none.
    fn live(&mut self) -> Result<&mut Live<D>, String> {
        if self.live.is_none() {
            let out = self.out().map_err(|e| e.to_string())?;
            std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            let dir = host::util::mkdtemp(&host::util::temp_dir().join("tandem-mcp-session-"))
                .map_err(|e| e.to_string())?;
            let out = out.to_string_lossy().into_owned();
            let make = &self.make;
            let started = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let host = Host::new(
                    HostConfig {
                        user_data: dir.join("udata"),
                        smoke: true,
                        echo_stdout: false,
                    },
                    Box::new(Scripted::new(Answers::default())),
                );
                let mut session = make(host);
                DriveRun::start(&mut session, FIXTURE, &out).map(|run| (session, run))
            }));
            match started {
                Ok(Ok((session, run))) => self.live = Some(Live { session, run, dir }),
                _ => {
                    let _ = std::fs::remove_dir_all(&dir);
                    return Err(ENDED.into());
                }
            }
        }
        self.live.as_mut().ok_or_else(|| ENDED.to_string())
    }

    /// One drive line through the live session: its STEP record, or the session's end.
    fn step(&mut self, line: &Value) -> Result<Value, String> {
        let live = self.live()?;
        // As the line would have crossed the child's stdin: `${OUT}` substituted, then parsed.
        let text = line.to_string();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            live.run.live_text(&mut live.session, &text)
        }));
        match outcome {
            Ok(Some(Outcome::Record(record))) => Ok(record),
            // The session ends after the record it wrote; the next call starts a new one.
            Ok(Some(Outcome::Ended(Some(record), _))) => {
                self.live = None;
                Ok(record)
            }
            Ok(None) | Ok(Some(Outcome::Close)) => {
                self.live = None;
                Err(ENDED.into())
            }
            Ok(Some(Outcome::Ended(None, _))) | Err(_) => {
                self.live = None;
                Err(ENDED.into())
            }
        }
    }

    /// `tools/call` for one tool: the `{content, isError}` result.
    pub fn call_tool(&mut self, name: &str, args: &Map<String, Value>) -> Value {
        if let Some(answer) = answer_without_session(name, args) {
            return answer;
        }
        let save = args
            .get("path")
            .and_then(Value::as_str)
            .filter(|_| name == "tandem_save");
        // Bugfix 1 D4: a relative save lands inside --out (folded) and gets its missing folders.
        let relative_save = save.filter(|p| !is_absolute(p)).and_then(fold);
        let result = (|| {
            self.live()?;
            let line = match name {
                "tandem_open" => json!({"cmd": "open", "path": here(args.get("path"))}),
                "tandem_new" => match args.get("size") {
                    Some(size) => json!({"cmd": "new", "contents": {"size": size}}),
                    None => json!({"cmd": "new"}),
                },
                "tandem_state" => json!({"cmd": "state"}),
                "tandem_save" => {
                    let path = save.unwrap_or_default();
                    let target = if let Some(relative) = &relative_save {
                        let target = self.out().map_err(|e| e.to_string())?.join(relative);
                        if let Some(parent) = target.parent() {
                            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                        }
                        target.to_string_lossy().into_owned()
                    } else {
                        path.to_string()
                    };
                    json!({"cmd": "save-to", "path": target})
                }
                _ => {
                    let mut line = args.clone();
                    for key in ["path", "to", "from"] {
                        if let Some(v) = args.get(key) {
                            line.insert(key.into(), here(Some(v)));
                        }
                    }
                    Value::Object(line)
                }
            };
            self.step(&line)
        })();
        match result {
            Ok(step) => {
                let failed = truthy(step.get("error"))
                    || step
                        .get("state")
                        .and_then(|s| s.get("error"))
                        .and_then(Value::as_str)
                        .is_some_and(|e| !e.is_empty());
                text_json(&step, failed)
            }
            Err(error) => text_json(&json!({"error": error}), true),
        }
    }

    /// Ends the live session (stdin EOF).
    pub fn close(&mut self) {
        if let Some(mut live) = self.live.take() {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                live.run.line(&mut live.session, json!({"cmd": "close"}))
            }));
        }
    }
}

/// The answer to a tool call that needs no session (`tandem_help`, and the refusals), if it is one.
fn answer_without_session(name: &str, args: &Map<String, Value>) -> Option<Value> {
    if name == "tandem_help" {
        return Some(text(help(args.get("topic").and_then(Value::as_str)), false));
    }
    let required: &[&str] = match name {
        "tandem_open" | "tandem_save" => &["path"],
        "tandem_do" => &["cmd"],
        _ => &[],
    };
    for key in required {
        if args
            .get(*key)
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            return Some(text(&format!("{name}: {key} is required"), true));
        }
    }
    if name == "tandem_do" && args.get("cmd").and_then(Value::as_str) == Some("close") {
        return Some(text(
            "tandem_do: close is refused; the session belongs to the server and ends when it stops",
            true,
        ));
    }
    let save = args.get("path").and_then(Value::as_str);
    if name == "tandem_save" && save.is_some_and(|p| !is_absolute(p) && fold(p).is_none()) {
        return Some(text(
            "tandem_save: path must stay inside the output folder",
            true,
        ));
    }
    None
}

/// What one JSON-RPC message asks for.
#[derive(Debug, PartialEq)]
pub enum Handled {
    /// Answered at once: the reply to write, if any (notifications get none).
    Reply(Option<Value>),
    /// A tool call for the session: answered by the session thread, in arrival order.
    Call {
        id: Option<Value>,
        name: String,
        args: Map<String, Value>,
    },
}

/// `{jsonrpc, id, ...body}`, or nothing for a message without an id.
fn reply(id: Option<&Value>, body: Value) -> Option<Value> {
    let mut out = Map::new();
    out.insert("jsonrpc".into(), json!("2.0"));
    out.insert("id".into(), id?.clone());
    if let Value::Object(body) = body {
        out.extend(body);
    }
    Some(Value::Object(out))
}

/// One JSON-RPC message (the Node server's `handle`). Everything but a session tool call is
/// answered here, so `ping`, `tools/list` and `tandem_help` answer while a step runs.
pub fn handle(message: &Map<String, Value>) -> Handled {
    let id = message.get("id");
    let method = message.get("method");
    let params = message.get("params");
    let answer = |body: Value| Handled::Reply(reply(id, body));
    match method.and_then(Value::as_str) {
        Some("initialize") => {
            let version = params
                .and_then(|p| p.get("protocolVersion"))
                .and_then(Value::as_str)
                .unwrap_or(PROTOCOL);
            answer(json!({"result": {
                "protocolVersion": version,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "tandem-pdf", "version": env!("CARGO_PKG_VERSION")},
            }}))
        }
        Some(m) if m.starts_with("notifications/") => Handled::Reply(None),
        Some("ping") => answer(json!({"result": {}})),
        Some("tools/list") => answer(json!({"result": {"tools": tools()}})),
        Some("tools/call") => {
            let name = params.and_then(|p| p.get("name"));
            let known = name.and_then(Value::as_str).filter(|n| {
                tools()
                    .as_array()
                    .is_some_and(|t| t.iter().any(|t| t["name"] == *n))
            });
            let Some(name) = known else {
                return answer(
                    json!({"error": {"code": -32602, "message": format!("Unknown tool: {}", js_text(name))}}),
                );
            };
            let args = params
                .and_then(|p| p.get("arguments"))
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            match answer_without_session(name, &args) {
                Some(result) => answer(json!({"result": result})),
                None => Handled::Call {
                    id: id.cloned(),
                    name: name.to_string(),
                    args,
                },
            }
        }
        _ => answer(
            json!({"error": {"code": -32601, "message": format!("Method not found: {}", js_text(method))}}),
        ),
    }
}

/// Writes one JSON line on stdout (whole lines, from either thread).
fn write_line(value: &Value) {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let _ = writeln!(out, "{value}");
    let _ = out.flush();
}

/// Serves MCP on stdin/stdout until stdin ends; the process's exit code. Stdin is read here;
/// the session lives on a thread of its own with a roomy stack (as the window's core thread),
/// which takes the session tool calls one at a time, in order.
pub fn serve<D: Driven, F: Fn(Host) -> D + Send + 'static>(make: F, out: Option<String>) -> i32 {
    let (tx, rx) = std::sync::mpsc::channel::<(Option<Value>, String, Map<String, Value>)>();
    let worker = std::thread::Builder::new()
        .name("tandem-mcp-session".into())
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let mut server = Server::new(make, out);
            for (id, name, args) in rx {
                let result = server.call_tool(&name, &args);
                if let Some(line) = reply(id.as_ref(), json!({"result": result})) {
                    write_line(&line);
                }
            }
            server.close();
        });
    let Ok(worker) = worker else {
        return 1;
    };
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    loop {
        let mut raw = Vec::new();
        match input.read_until(b'\n', &mut raw) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let line = String::from_utf8_lossy(&raw);
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(&line) {
            Err(_) => write_line(
                &json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "Parse error"}}),
            ),
            Ok(Value::Object(message)) => match handle(&message) {
                Handled::Reply(Some(line)) => write_line(&line),
                Handled::Reply(None) => {}
                Handled::Call { id, name, args } => {
                    if tx.send((id, name, args)).is_err() {
                        break;
                    }
                }
            },
            Ok(_) => write_line(
                &json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32600, "message": "Invalid Request"}}),
            ),
        }
    }
    drop(tx);
    i32::from(worker.join().is_err())
}

/// `{content: [{type: "text", text}], isError}`.
fn text(value: &str, is_error: bool) -> Value {
    json!({"content": [{"type": "text", "text": value}], "isError": is_error})
}

/// [`text`] of `JSON.stringify(value, null, 1)`.
fn text_json(value: &Value, is_error: bool) -> Value {
    let mut buf = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b" ");
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);
    let shown = serde::Serialize::serialize(value, &mut ser)
        .ok()
        .and_then(|()| String::from_utf8(buf).ok())
        .unwrap_or_else(|| value.to_string());
    text(&shown, is_error)
}

/// JS truthiness of an optional value.
fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

/// `${value}` in a JS template: `undefined` for a missing value.
fn js_text(v: Option<&Value>) -> String {
    match v {
        None => "undefined".into(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

/// `here(path)`: a relative path is the agent's, resolved against this server's working
/// folder; `${OUT}/…` is left to the session, and a non-string is passed on as it is.
fn here(v: Option<&Value>) -> Value {
    match v {
        Some(Value::String(s)) if !s.starts_with("${OUT}") => {
            json!(host::util::resolve(s).to_string_lossy())
        }
        Some(other) => other.clone(),
        None => Value::Null,
    }
}

/// Node's `path.isAbsolute`: a root (`/x`, or `\x` on Windows) or a drive prefix.
fn is_absolute(path: &str) -> bool {
    let p = Path::new(path);
    p.is_absolute() || p.has_root()
}

/// A relative path folded lexically (`a/../b` is `b`), or `None` when it starts above its
/// folder (`normalize(p)` begins with `..`) or names a root or drive (`C:x` on Windows).
fn fold(path: &str) -> Option<PathBuf> {
    let mut parts: Vec<&std::ffi::OsStr> = Vec::new();
    for part in Path::new(path).components() {
        match part {
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::Normal(name) => parts.push(name),
            Component::CurDir => {}
            Component::Prefix(_) | Component::RootDir => return None,
        }
    }
    Some(parts.iter().collect())
}

/// Whether a relative save would leave its folder.
#[cfg(test)]
fn escapes(path: &str) -> bool {
    fold(path).is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_sections() {
        assert_eq!(help(None), AGENT_DOC);
        assert_eq!(help(Some("nosuch")), AGENT_DOC);
        for topic in ["create", "modify", "fill", "ops"] {
            let section = help(Some(topic));
            assert!(
                section.to_lowercase().starts_with(&format!("## {topic}")),
                "{topic}: {}",
                &section[..section.len().min(40)]
            );
            assert!(section.len() < AGENT_DOC.len());
            assert!(
                !section[3..].contains("\n## "),
                "{topic} stops at the next section"
            );
        }
    }

    #[test]
    fn save_paths_stay_inside() {
        assert!(!escapes("x.pdf"));
        assert!(!escapes("sub/deep/x.pdf"));
        assert!(!escapes("a/../x.pdf"));
        assert!(escapes("../escape.pdf"));
        assert!(escapes("a/../../x.pdf"));
        assert!(escapes(".."));
        assert_eq!(fold("sub/../x.pdf"), Some(PathBuf::from("x.pdf")));
        assert_eq!(fold("./a/./b.pdf"), Some(PathBuf::from("a/b.pdf")));
        assert!(is_absolute("/tmp/x.pdf"));
        assert!(!is_absolute("x.pdf"));
    }

    #[test]
    fn tool_list_matches_the_node_server() {
        let tools = tools();
        let names: Vec<&str> = tools
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "tandem_open",
                "tandem_new",
                "tandem_do",
                "tandem_state",
                "tandem_save",
                "tandem_help"
            ]
        );
        for tool in tools.as_array().unwrap() {
            assert_eq!(tool["inputSchema"]["type"], "object");
        }
    }

    #[test]
    fn pretty_json_uses_one_space() {
        let shown = text_json(&json!({"a": [1, {"b": "c"}]}), false);
        assert_eq!(
            shown["content"][0]["text"],
            "{\n \"a\": [\n  1,\n  {\n   \"b\": \"c\"\n  }\n ]\n}"
        );
        assert_eq!(shown["isError"], false);
    }

    #[test]
    fn json_rpc_without_a_session() {
        let mut server = Server::new(crate::session::Session::for_drive, None);
        let msg = |v: Value| v.as_object().cloned().unwrap();
        let reply_to = |m: &Map<String, Value>| match handle(m) {
            Handled::Reply(r) => r,
            Handled::Call { .. } => panic!("answered without a session"),
        };
        let init = reply_to(&msg(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26"}})))
            .unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(init["result"]["serverInfo"]["name"], "tandem-pdf");
        assert_eq!(init["result"]["capabilities"], json!({"tools": {}}));
        let keys: Vec<&String> = init.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["jsonrpc", "id", "result"]);
        assert!(
            reply_to(&msg(
                json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
            ))
            .is_none()
        );
        assert_eq!(
            reply_to(&msg(
                json!({"jsonrpc": "2.0", "id": 2, "method": "nosuch/method"})
            ))
            .unwrap()["error"]["code"],
            -32601
        );
        let unknown = reply_to(&msg(json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "nosuch"}})))
            .unwrap();
        assert_eq!(unknown["error"]["code"], -32602);
        assert_eq!(unknown["error"]["message"], "Unknown tool: nosuch");
        // A refusal is answered at once; a session tool goes to the session thread.
        let help = reply_to(&msg(json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "tandem_help", "arguments": {"topic": "fill"}}}))).unwrap();
        assert_eq!(help["result"]["isError"], false);
        assert!(matches!(
            handle(&msg(json!({"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {"name": "tandem_state"}}))),
            Handled::Call { ref name, .. } if name == "tandem_state"
        ));
        let refused = server.call_tool("tandem_do", &msg(json!({"cmd": "close"})));
        assert_eq!(refused["isError"], true);
        let missing = server.call_tool("tandem_open", &Map::new());
        assert_eq!(
            missing["content"][0]["text"],
            "tandem_open: path is required"
        );
        let escape = server.call_tool("tandem_save", &msg(json!({"path": "../x.pdf"})));
        assert_eq!(
            escape["content"][0]["text"],
            "tandem_save: path must stay inside the output folder"
        );
        assert!(server.live.is_none(), "refusals never start a session");
    }

    #[test]
    fn a_session_that_ends_is_started_again() {
        let out = host::util::session_scratch("mcp-out");
        let mut server = Server::new(
            crate::session::Session::for_drive,
            Some(out.to_string_lossy().into_owned()),
        );
        let msg = |v: Value| v.as_object().cloned().unwrap();
        let state = server.call_tool("tandem_state", &Map::new());
        assert_eq!(state["isError"], false, "{state}");
        let step: Value =
            serde_json::from_str(state["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(step["state"]["doc"]["name"], "hello.pdf");
        let created = server.call_tool("tandem_new", &msg(json!({"size": "a4"})));
        assert_eq!(created["isError"], false, "{created}");
        // A secret given in one step and asked for in the next leaks into STATE: the session ends.
        let secret = "mcp-unit-secret-4417";
        server.call_tool(
            "tandem_do",
            &msg(json!({"cmd": "password", "contents": secret})),
        );
        let leaked = server.call_tool(
            "tandem_do",
            &msg(json!({"cmd": "find", "contents": {"query": secret}})),
        );
        assert_eq!(leaked["isError"], true);
        let text = leaked["content"][0]["text"].as_str().unwrap();
        assert!(text.contains(ENDED) && !text.contains(secret), "{text}");
        assert!(server.live.is_none());
        // The next call starts a fresh session on the sample.
        let again = server.call_tool("tandem_state", &Map::new());
        let step: Value =
            serde_json::from_str(again["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(step["state"]["doc"]["name"], "hello.pdf");
        assert_eq!(
            server.call_tool("tandem_new", &Map::new())["isError"],
            false
        );
        let saved = server.call_tool("tandem_save", &msg(json!({"path": "sub/x.pdf"})));
        assert_eq!(saved["isError"], false, "{saved}");
        assert!(out.join("sub/x.pdf").is_file());
        let dir = server.live.as_ref().unwrap().dir.clone();
        server.close();
        assert!(!dir.exists(), "the session's own folder is removed");
        let _ = std::fs::remove_dir_all(&out);
    }

    #[test]
    fn click_lines_are_packed() {
        let packed = crate::drive::pack_line(json!({"cmd": "click", "page": 0, "x": 5, "y": 6}));
        assert_eq!(packed["contents"], json!({"page": 0, "x": 5, "y": 6}));
        assert_eq!(packed["x"], 5, "the original fields stay");
        let kept = json!({"cmd": "click", "x": 1, "contents": null});
        assert_eq!(crate::drive::pack_line(kept.clone()), kept);
        let other = json!({"cmd": "open", "path": "a.pdf"});
        assert_eq!(crate::drive::pack_line(other.clone()), other);
    }
}
