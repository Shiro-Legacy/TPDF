//! The engine, called natively (no WASM, no worker): the port of
//! `src/renderer/engine-rust.ts` and the worker's `request()`.
//!
//! A request is JSON with binary fields as `{"$buffer": n}` placeholders into
//! the inputs passed beside it, exactly the wire format of
//! engine/APP-ENGINE-DESIGN.md; the reply's `{"$buffer": n}` / `{"$bytes": n}`
//! markers name its output buffers. Data keys that begin with `$` carry one
//! more `$` on the wire: build such keys with [`esc_key`], and replies come
//! back unescaped.
//!
//! A panic inside the engine is caught: the engine then counts as stopped
//! (as the WASM app did after a trap) until [`EngineHost::restart`], which is
//! what the recovery journal (M9) replays into.

use serde_json::{Map, Value, json};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tandem_pdf_engine::app::{AppEngine, AppSession};

/// One reply, `ok: true`, with its output buffers.
#[derive(Debug, Default, Clone)]
pub struct Reply {
    pub value: Value,
    pub outputs: Vec<Vec<u8>>,
}

impl Reply {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.value.get(key)
    }
    /// The bytes a `{"$buffer": n}` / `{"$bytes": n}` marker at `value` names.
    pub fn bytes_at(&self, value: Option<&Value>) -> Option<&[u8]> {
        let marker = value?.as_object()?;
        let index = marker
            .get("$buffer")
            .or_else(|| marker.get("$bytes"))?
            .as_u64()? as usize;
        self.outputs.get(index).map(Vec::as_slice)
    }
    /// The bytes of the marker under `key`.
    pub fn bytes(&self, key: &str) -> Option<&[u8]> {
        self.bytes_at(self.value.get(key))
    }
    /// Takes the bytes of the marker under `key` out of the reply.
    pub fn take_bytes(&mut self, key: &str) -> Option<Vec<u8>> {
        let marker = self.value.get(key)?.as_object()?;
        let index = marker
            .get("$buffer")
            .or_else(|| marker.get("$bytes"))?
            .as_u64()? as usize;
        self.outputs.get_mut(index).map(std::mem::take)
    }
}

/// An `ok: false` reply: what the renderer's `request()` rejected with.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Refusal {
    pub message: String,
    pub reason: Option<String>,
    pub detail: Option<String>,
    /// The whole reply (journal state, extra fields such as `limit`), boxed so the
    /// `Err` side of every `EngineResult` stays small.
    pub reply: Box<Value>,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl Refusal {
    /// A refusal made on this side (the renderer's own `throw`).
    pub fn plain(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            ..Self::default()
        }
    }
    pub fn reason(reason: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            reason: Some(reason.into()),
            ..Self::default()
        }
    }
    /// `refusalOf(err).reason` in the renderer: the token, or "".
    pub fn reason_str(&self) -> &str {
        self.reason.as_deref().unwrap_or("")
    }
    pub fn detail_str(&self) -> &str {
        self.detail.as_deref().unwrap_or("")
    }
}

pub type EngineResult = Result<Reply, Refusal>;

/// A data key as the wire carries it (`$x` → `$$x`).
pub fn esc_key(key: &str) -> String {
    if key.starts_with('$') {
        format!("${key}")
    } else {
        key.to_string()
    }
}

/// Reply fields engine.ts timed; native Rust could time them itself, but the
/// adapter's rule (the whole dispatch, font install included) is kept.
fn timed(op: &str) -> &'static [&'static str] {
    match op {
        "open" => &["openMs"],
        "save" => &["saveMs"],
        "render" | "compare-render" => &["paintMs", "renderMs"],
        "search" => &["searchMs"],
        _ => &[],
    }
}

/// engine/src/app/ops/scan.rs `MAX_SCAN_ITEMS`, checked before dispatch.
const MAX_SCAN_ITEMS: usize = 40_000;

/// The engine plus the one document session, owned by the core thread.
pub struct EngineHost {
    engine: AppEngine,
    session: AppSession,
    /// Set by a panic; every later call answers `reason: "worker"`.
    stopped: Option<String>,
    cjk: Option<CjkFont>,
    next_id: u64,
}

/// Whether this process read the CJK font file (the drive report lists it as the Electron
/// app's `protocolRequests` did, `fonts/DroidSansFallback.ttf`).
static CJK_FONT_READ: AtomicBool = AtomicBool::new(false);

/// Whether any engine host in this process has read the CJK font file.
pub fn cjk_font_read() -> bool {
    CJK_FONT_READ.load(Ordering::Relaxed)
}

struct CjkFont {
    path: PathBuf,
    settled: bool,
}

impl Default for EngineHost {
    fn default() -> Self {
        Self::new()
    }
}

impl EngineHost {
    pub fn new() -> Self {
        Self {
            engine: AppEngine::new(),
            session: AppSession::default(),
            stopped: None,
            cjk: None,
            next_id: 1,
        }
    }

    /// Whether a panic stopped the engine.
    pub fn stopped(&self) -> Option<&str> {
        self.stopped.as_deref()
    }

    /// A fresh engine and session (the worker restart of M9).
    pub fn restart(&mut self) {
        self.engine = AppEngine::new();
        self.session = AppSession::default();
        self.stopped = None;
    }

    /// The CJK fallback font file, read the first time a render wants it.
    pub fn use_cjk_font(&mut self, path: PathBuf) {
        self.cjk = Some(CjkFont {
            path,
            settled: false,
        });
    }

    /// `{revision, dirty, canUndo, canRedo, generation, savedPosition, locked, auth, name, sourceBytes}`.
    pub fn session_state(&self) -> Value {
        self.session.state_json()
    }

    /// How many undo steps the open document's journal holds below its position: what an
    /// action really added to Undo (the revision also moves for a `security-set` or a
    /// mutation that changed nothing, which leave no step).
    pub fn journal_position(&self) -> Option<i64> {
        self.session.doc.as_ref().map(|doc| doc.position())
    }

    /// The session's current document bytes (tests and recovery).
    pub fn snapshot_bytes(&self) -> Option<Vec<u8>> {
        self.session.snapshot_bytes()
    }

    pub fn compare_bytes(&self) -> Option<Vec<u8>> {
        Some(self.session.compare.as_ref()?.bytes().to_vec())
    }

    pub fn debug_json(&self) -> Value {
        self.session.debug_json()
    }

    fn install_cjk(&mut self, preload: bool) -> bool {
        let Some(cjk) = self.cjk.as_mut() else {
            return false;
        };
        if cjk.settled {
            return false;
        }
        let wanted = if preload {
            !tandem_pdf_engine::view::has_cjk_font()
        } else {
            tandem_pdf_engine::view::cjk_font_wanted()
        };
        if !wanted {
            return false;
        }
        cjk.settled = true;
        match std::fs::read(&cjk.path) {
            Ok(bytes) => {
                CJK_FONT_READ.store(true, Ordering::Relaxed);
                tandem_pdf_engine::view::set_cjk_font(bytes).is_ok()
            }
            Err(_) => false,
        }
    }

    fn dispatch_once(
        &mut self,
        request: &Value,
        inputs: &[Vec<u8>],
    ) -> Result<(Value, Vec<Vec<u8>>), String> {
        let engine = &mut self.engine;
        let session = &mut self.session;
        let request = request.clone();
        let inputs = inputs.to_vec();
        catch_unwind(AssertUnwindSafe(move || {
            engine.dispatch(session, request, inputs)
        }))
        .map_err(|panic| {
            panic
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "internal error".to_string())
        })
    }

    /// The journal fields every reply carries.
    fn journal_state(&self) -> Map<String, Value> {
        let state = self.session.state_json();
        let mut out = Map::new();
        for key in ["revision", "dirty", "canUndo", "canRedo"] {
            if let Some(v) = state.get(key) {
                out.insert(key.into(), v.clone());
            }
        }
        out
    }

    /// One request; the raw reply (ok or not) and its outputs. Never panics.
    pub fn dispatch(
        &mut self,
        mut request: Value,
        mut inputs: Vec<Vec<u8>>,
    ) -> (Value, Vec<Vec<u8>>) {
        let op = request
            .get("op")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let id = self.next_id;
        self.next_id += 1;
        if let Value::Object(map) = &mut request {
            map.entry("id").or_insert(json!(id));
        }
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        if let Some(reason) = &self.stopped {
            let mut reply = Map::new();
            reply.insert("id".into(), id);
            reply.insert("ok".into(), false.into());
            reply.insert(
                "error".into(),
                format!("the PDF engine stopped after an internal error ({reason})").into(),
            );
            reply.insert("reason".into(), "worker".into());
            reply.extend(self.journal_state());
            return (Value::Object(reply), Vec::new());
        }
        if op == "scan-compose" && scan_items(&request) > MAX_SCAN_ITEMS {
            let mut reply = Map::new();
            reply.insert("id".into(), id);
            reply.insert("ok".into(), false.into());
            reply.insert(
                "error".into(),
                format!("scan-compose: a scan has at most {MAX_SCAN_ITEMS} runs and boxes in all")
                    .into(),
            );
            reply.insert("reason".into(), "too-large".into());
            reply.insert("detail".into(), "runs".into());
            reply.extend(self.journal_state());
            return (Value::Object(reply), Vec::new());
        }
        wire(&mut request, &op);
        // Ops whose output may be encrypted get 32 fresh random bytes, unless
        // the caller (the recovery log) brought its own.
        if matches!(op.as_str(), "save" | "redact-apply" | "unsigned-copy") {
            let has = request
                .get("entropy")
                .and_then(|v| v.get("$buffer"))
                .and_then(Value::as_u64)
                .is_some_and(|i| inputs.get(i as usize).is_some_and(|b| b.len() == 32));
            if !has {
                inputs.push(entropy(32));
                request["entropy"] = json!({ "$buffer": inputs.len() - 1 });
            }
        }
        let started = Instant::now();
        if op == "redact-apply" {
            self.install_cjk(true);
        }
        let mut result = self.dispatch_once(&request, &inputs);
        let ok = matches!(&result, Ok((reply, _)) if reply.get("ok") == Some(&Value::Bool(true)));
        let paint = matches!(op.as_str(), "render" | "thumbnail" | "compare-render");
        let font = matches!(op.as_str(), "scan-compose" | "scan-measure");
        if result.is_ok() && self.install_cjk(false) && ((paint && ok) || font) {
            result = self.dispatch_once(&request, &inputs);
        }
        let elapsed = started.elapsed().as_millis() as u64;
        match result {
            Ok((mut reply, outputs)) => {
                if reply.get("ok") == Some(&Value::Bool(true)) {
                    for key in timed(&op) {
                        if reply.get(*key).is_some_and(Value::is_number) {
                            reply[*key] = json!(elapsed);
                        }
                    }
                }
                let mut outputs = outputs;
                let mut echoed = Vec::new();
                resolve_inputs(&mut reply, &mut inputs, &mut outputs, &mut echoed);
                (unescape(reply), outputs)
            }
            Err(panic) => {
                self.stopped = Some(panic.clone());
                let mut reply = Map::new();
                reply.insert("id".into(), id);
                reply.insert("ok".into(), false.into());
                reply.insert(
                    "error".into(),
                    format!("the PDF engine stopped after an internal error ({panic})").into(),
                );
                reply.insert("reason".into(), "worker".into());
                reply.extend(self.journal_state());
                (Value::Object(reply), Vec::new())
            }
        }
    }

    /// The renderer's `request()`: `ok: true` → `Reply`, anything else → `Refusal`.
    pub fn request(&mut self, request: Value, inputs: Vec<Vec<u8>>) -> EngineResult {
        let (reply, outputs) = self.dispatch(request, inputs);
        if reply.get("ok") == Some(&Value::Bool(true)) {
            Ok(Reply {
                value: reply,
                outputs,
            })
        } else {
            Err(Refusal {
                message: reply
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                reason: reply
                    .get("reason")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                detail: reply
                    .get("detail")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                reply: Box::new(reply),
            })
        }
    }
}

/// `{"$input": n}` in a reply hands the request's own buffer `n` back (`validate`); the JS
/// adapter answered with the caller's `ArrayBuffer`. Here the buffer moves to the outputs and
/// the marker becomes a `{"$buffer": i}` like any other (one output per echoed input).
fn resolve_inputs(
    value: &mut Value,
    inputs: &mut [Vec<u8>],
    outputs: &mut Vec<Vec<u8>>,
    echoed: &mut Vec<(usize, usize)>,
) {
    match value {
        Value::Object(map) => {
            if map.len() == 1
                && let Some(index) = map.get("$input").and_then(Value::as_u64)
            {
                let index = index as usize;
                let at = match echoed.iter().find(|(i, _)| *i == index) {
                    Some((_, at)) => *at,
                    None => {
                        let bytes = inputs
                            .get_mut(index)
                            .map(std::mem::take)
                            .unwrap_or_default();
                        outputs.push(bytes);
                        echoed.push((index, outputs.len() - 1));
                        outputs.len() - 1
                    }
                };
                *value = json!({ "$buffer": at });
                return;
            }
            for item in map.values_mut() {
                resolve_inputs(item, inputs, outputs, echoed);
            }
        }
        Value::Array(items) => {
            for item in items {
                resolve_inputs(item, inputs, outputs, echoed);
            }
        }
        _ => {}
    }
}

fn scan_items(request: &Value) -> usize {
    let Some(pages) = request.get("pages").and_then(Value::as_array) else {
        return 0;
    };
    pages
        .iter()
        .map(|page| {
            ["runs", "boxes"]
                .iter()
                .map(|k| page.get(*k).and_then(Value::as_array).map_or(0, Vec::len))
                .sum::<usize>()
        })
        .sum()
}

/// The adapter's `wire`: entropy for `save`/`redact-apply`/`unsigned-copy`, `today` for
/// `annot-add`, the local UTC offset for `sign-prepare`.
fn wire(request: &mut Value, op: &str) {
    let Value::Object(map) = request else {
        return;
    };
    match op {
        "sign-prepare" => {
            if let Some(when) = map.get("when").and_then(Value::as_f64) {
                map.insert(
                    "timezoneOffset".into(),
                    json!(timezone_offset_minutes(when)),
                );
            }
        }
        "annot-add" => {
            let ok = map
                .get("today")
                .and_then(Value::as_str)
                .is_some_and(is_date);
            if !ok {
                map.insert("today".into(), json!(local_date()));
            }
        }
        _ => {}
    }
}

fn is_date(text: &str) -> bool {
    let b = text.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

/// `YYYY-MM-DD` in local time.
pub fn local_date() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// JavaScript's `Date#getTimezoneOffset()` at `when` (ms since the epoch):
/// minutes to add to local time to get UTC.
pub fn timezone_offset_minutes(when: f64) -> i64 {
    use chrono::{Offset, TimeZone};
    let secs = (when / 1000.0).floor() as i64;
    match chrono::Local.timestamp_opt(secs, 0) {
        chrono::LocalResult::Single(t) | chrono::LocalResult::Ambiguous(t, _) => {
            -(t.offset().fix().local_minus_utc() as i64) / 60
        }
        chrono::LocalResult::None => 0,
    }
}

/// `n` random bytes from the system.
pub fn entropy(n: usize) -> Vec<u8> {
    use std::io::Read;
    let mut out = vec![0u8; n];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        let _ = f.read_exact(&mut out);
    }
    out
}

/// Milliseconds since the epoch, as `Date.now()`.
pub fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0)
}

/// Removes the `$` escape from reply data keys (markers stay as they are).
fn unescape(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let single = map.len() == 1
                && map
                    .keys()
                    .next()
                    .is_some_and(|k| k.starts_with('$') && !k.starts_with("$$"));
            if single {
                return Value::Object(map);
            }
            Value::Object(
                map.into_iter()
                    .map(|(k, v)| {
                        let k = if let Some(rest) = k.strip_prefix('$') {
                            rest.to_string()
                        } else {
                            k
                        };
                        (k, unescape(v))
                    })
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.into_iter().map(unescape).collect()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_and_renders_hello() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/hello.pdf");
        let bytes = std::fs::read(path).unwrap();
        let mut host = EngineHost::new();
        let reply = host
            .request(
                json!({"op": "open", "name": "hello.pdf", "pdf": {"$buffer": 0}}),
                vec![bytes],
            )
            .unwrap();
        assert_eq!(reply.get("pageCount"), Some(&json!(1)));
        let mut render = host
            .request(json!({"op": "render", "page": 0, "scale": 1}), vec![])
            .unwrap();
        let w = render.get("width").and_then(Value::as_f64).unwrap() as u64;
        let h = render.get("height").and_then(Value::as_f64).unwrap() as u64;
        assert_eq!((w, h), (612, 792));
        let pixels = render.take_bytes("pixels").expect("pixels");
        assert_eq!(pixels.len() as u64, w * h * 4);
    }

    #[test]
    fn refusal_carries_reason() {
        let mut host = EngineHost::new();
        let err = host
            .request(json!({"op": "render", "page": 0, "scale": 1}), vec![])
            .unwrap_err();
        assert!(!err.message.is_empty());
    }

    #[test]
    fn escape_round_trip() {
        assert_eq!(esc_key("$x"), "$$x");
        assert_eq!(
            unescape(json!({"$$x": 1, "y": {"$buffer": 0}})),
            json!({"$x": 1, "y": {"$buffer": 0}})
        );
        assert!(is_date("2026-10-19"));
        assert!(!is_date("2026-1-19"));
    }
}
