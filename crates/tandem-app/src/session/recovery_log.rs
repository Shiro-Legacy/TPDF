//! recovery.ts: the M9 recovery log (engine/design/app-m9-recovery.md), ported onto the
//! in-process engine. The engine owns the open document, so a caught engine panic (the WASM
//! trap of the Electron app) loses every unsaved edit with it. The session keeps, in its own
//! memory only: the bytes the document was opened from (the replay base), every request since
//! that open that changed the engine's session (`OPS` kind `mutate`, `unlock` or `clipboard`),
//! in order, with a fingerprint of the answer, and the bytes of the last acknowledged save in
//! place (the fallback). After `EngineHost::restart`, [`Session::replay`] reopens the base in
//! the fresh engine and sends the log again; every answer must match its fingerprint, so ids,
//! revisions and undo history are those the UI holds.
#![allow(dead_code)]

use super::Session;
use super::consts::{MAX_REPLAY_CLOSES, OPS, RECOVERY_BUDGET_BYTES, REPLAY_BASE_MS};
use super::consts::{MOBILE_RECOVERY_BUDGET_BYTES, SMALL_RECOVERY_BUDGET_BYTES};
use super::types::{Capture, Kept, OpKind, RecoveryEntry, RecoveryLog, ReplayResult, Unavailable};
use serde_json::{Map, Value, json};
use std::time::{Duration, Instant};

/// V:96 `opInfo`: the op's entry `(kind, label, long)`, or None for a name that is not an op.
pub fn op_info(op: &Value) -> Option<(OpKind, &'static str, bool)> {
    op_info_str(op.as_str()?)
}

pub fn op_info_str(op: &str) -> Option<(OpKind, &'static str, bool)> {
    OPS.iter()
        .find(|(name, ..)| *name == op)
        .map(|(_, kind, label, long)| (*kind, *label, *long))
}

/// V:101 `replayed`: kinds a replay sends again.
pub fn replayed(op: &Value) -> bool {
    matches!(
        op_info(op).map(|(kind, ..)| kind),
        Some(OpKind::Mutate | OpKind::Unlock | OpKind::Clipboard)
    )
}

/// V:122 `recoveryBudgetFor`: `mobile` takes the mobile default; a known `deviceMemoryGiB`
/// (Chromium's `navigator.deviceMemory`, capped at 8) of 4 or less takes the small one.
pub fn recovery_budget_for(mobile: bool, device_memory_gib: Option<f64>) -> usize {
    if mobile {
        return MOBILE_RECOVERY_BUDGET_BYTES;
    }
    match device_memory_gib {
        Some(memory) if memory.is_finite() && memory > 0.0 && memory <= 4.0 => {
            SMALL_RECOVERY_BUDGET_BYTES
        }
        _ => RECOVERY_BUDGET_BYTES,
    }
}

/// `Number(text)` for the integer test of a ticket's generation: None unless an integer.
fn js_number_of(text: &str) -> Option<f64> {
    let trimmed = crate::js::js_trim(text);
    if trimmed.is_empty() {
        return Some(0.0);
    }
    let lower = trimmed.to_ascii_lowercase();
    let parsed = if let Some(hex) = lower.strip_prefix("0x") {
        u64::from_str_radix(hex, 16).ok().map(|v| v as f64)
    } else if let Some(bin) = lower.strip_prefix("0b") {
        u64::from_str_radix(bin, 2).ok().map(|v| v as f64)
    } else if let Some(oct) = lower.strip_prefix("0o") {
        u64::from_str_radix(oct, 8).ok().map(|v| v as f64)
    } else if trimmed
        .bytes()
        .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-'))
    {
        trimmed.parse::<f64>().ok()
    } else {
        None
    };
    parsed.filter(|v| v.is_finite() && *v == v.trunc())
}

/// V:177 `ticketGeneration`: the generation a save ticket names (its first field), or None.
fn ticket_generation(ticket: &Value) -> Option<f64> {
    let ticket = ticket.as_str()?;
    js_number_of(ticket.split(':').next().unwrap_or(""))
}

/// V:184 `messageGeneration`: an edit handle's generation, or a `saved` ticket's.
fn message_generation(message: &Map<String, Value>) -> Option<f64> {
    if let Some(generation) = message.get("generation").and_then(Value::as_f64) {
        return Some(generation);
    }
    if message.get("op").and_then(Value::as_str) == Some("saved") {
        return message.get("ticket").and_then(ticket_generation);
    }
    None
}

/// The index a `{"$buffer": n}` / `{"$bytes": n}` marker names.
fn marker(value: &Value) -> Option<usize> {
    let map = value.as_object()?;
    if map.len() != 1 {
        return None;
    }
    let index = map.get("$buffer").or_else(|| map.get("$bytes"))?;
    index.as_u64().map(|i| i as usize)
}

/// V:194 `sizeOf`: approximate bytes a request holds: buffers in full, strings at two bytes a
/// unit (UTF-16), every object and array 16, every other value 8.
pub fn size_of(message: &Value, inputs: &[Vec<u8>]) -> usize {
    let mut total = 0;
    let mut stack = vec![message];
    while let Some(item) = stack.pop() {
        match item {
            Value::String(s) => total += 16 + s.encode_utf16().count() * 2,
            Value::Object(map) => {
                if let Some(index) = marker(item) {
                    total += 16 + inputs.get(index).map_or(0, Vec::len);
                    continue;
                }
                total += 16;
                stack.extend(map.values());
            }
            Value::Array(items) => {
                total += 16;
                stack.extend(items.iter());
            }
            _ => total += 8,
        }
    }
    total
}

/// V:213 `volatile`: keys whose value may differ between two runs of the same request.
fn volatile(key: &str) -> bool {
    key == "id" || key == "compareClosed" || key.ends_with("Ms")
}

/// V:218 cyrb53 over UTF-16 code units: a 53-bit string hash, base 36.
fn hash(text: &str) -> String {
    let mut h1: u32 = 0xdead_beef;
    let mut h2: u32 = 0x41c6_ce57;
    for code in text.encode_utf16() {
        h1 = (h1 ^ code as u32).wrapping_mul(2_654_435_761);
        h2 = (h2 ^ code as u32).wrapping_mul(1_597_334_677);
    }
    h1 = (h1 ^ (h1 >> 16)).wrapping_mul(2_246_822_507)
        ^ (h2 ^ (h2 >> 13)).wrapping_mul(3_266_489_909);
    h2 = (h2 ^ (h2 >> 16)).wrapping_mul(2_246_822_507)
        ^ (h1 ^ (h1 >> 13)).wrapping_mul(3_266_489_909);
    let mut value = 4_294_967_296u64 * (2_097_151 & h2) as u64 + h1 as u64;
    if value == 0 {
        return "0".into();
    }
    let digits = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = Vec::new();
    while value > 0 {
        out.push(digits[(value % 36) as usize]);
        value /= 36;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// The reply reduced as `fingerprint`'s replacer reduces it: volatile keys dropped, buffers
/// by length.
fn reduce(value: &Value, outputs: &[Vec<u8>]) -> Value {
    match value {
        Value::Object(map) => {
            if let Some(index) = marker(value) {
                return json!({ "$bytes": outputs.get(index).map_or(0, Vec::len) });
            }
            Value::Object(
                map.iter()
                    .filter(|(key, _)| !volatile(key))
                    .map(|(key, v)| (key.clone(), reduce(v, outputs)))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(|v| reduce(v, outputs)).collect()),
        other => other.clone(),
    }
}

/// V:234 `fingerprint`: an answer reduced to what a replay must reproduce: every field but
/// timings and the request id, buffers by length.
pub fn fingerprint(reply: &Value, outputs: &[Vec<u8>]) -> String {
    let text = serde_json::to_string(&reduce(reply, outputs)).unwrap_or_default();
    format!("{}:{}", text.encode_utf16().count(), hash(&text))
}

/// V:245 `copyMessage`: the request without its id.
fn copy_message(message: &Map<String, Value>) -> Map<String, Value> {
    let mut copy = message.clone();
    copy.remove("id");
    copy
}

impl RecoveryLog {
    /// V:282: bytes held: base, saved copy and log.
    pub fn bytes(&self) -> usize {
        self.base.as_ref().map_or(0, Vec::len)
            + self.saved.as_ref().map_or(0, Vec::len)
            + self.entry_bytes
    }

    /// V:287: a document is tracked and its log can be replayed.
    pub fn available(&self) -> bool {
        self.base.is_some()
    }

    /// V:292: forgets the document.
    pub fn reset(&mut self) {
        self.base = None;
        self.name.clear();
        self.opened.clear();
        self.opened_ms = 0.0;
        self.saved = None;
        self.saved_lost = false;
        self.entries.clear();
        self.entry_bytes = 0;
        self.unavailable = None;
        self.clipboard_in_log = false;
        self.captures.clear();
    }

    /// V:307: turns recovery off for this document and frees what it held.
    fn off(&mut self, reason: Unavailable) {
        self.unavailable = Some(reason);
        self.base = None;
        self.entries.clear();
        self.entry_bytes = 0;
        self.saved = None;
    }

    /// V:320 `keep`: before a request is sent, a copy of what the log will need if it
    /// succeeds, or None when the request is not tracked.
    pub fn keep(&self, message: &Map<String, Value>, inputs: &[Vec<u8>]) -> Option<Kept> {
        let op = message.get("op").cloned().unwrap_or(Value::Null);
        let name = op.as_str().unwrap_or("").to_string();
        let kind = op_info(&op).map(|(kind, ..)| kind);
        if kind == Some(OpKind::Base) {
            let pdf = (name == "open")
                .then(|| message.get("pdf").and_then(marker))
                .flatten()
                .and_then(|index| inputs.get(index));
            let Some(pdf) = pdf else {
                return Some(Kept {
                    op: name,
                    bytes: 0,
                    message: None,
                    inputs: Vec::new(),
                });
            };
            let bytes = pdf.len();
            if bytes > self.budget {
                return Some(Kept {
                    op: name,
                    bytes,
                    message: None,
                    inputs: Vec::new(),
                });
            }
            let mut copy = copy_message(message);
            copy.insert("pdf".into(), json!({ "$buffer": 0 }));
            return Some(Kept {
                op: name,
                bytes,
                message: Some(copy),
                inputs: vec![pdf.clone()],
            });
        }
        if !replayed(&op) {
            return None;
        }
        // An untracked copy still has to be seen: it replaces the clipboard a later paste reads.
        if !self.available() {
            return (name == "clip-copy").then(|| Kept {
                op: name,
                bytes: 0,
                message: None,
                inputs: Vec::new(),
            });
        }
        let bytes = size_of(&Value::Object(message.clone()), inputs);
        if self.bytes() + bytes <= self.budget {
            Some(Kept {
                op: name,
                bytes,
                message: Some(copy_message(message)),
                inputs: inputs.to_vec(),
            })
        } else {
            Some(Kept {
                op: name,
                bytes,
                message: None,
                inputs: Vec::new(),
            })
        }
    }

    /// V:336 `record`: after the engine answered `kept`'s request successfully.
    pub fn record(&mut self, kept: Kept, reply: &Value, outputs: &[Vec<u8>], ms: f64) {
        let op = kept.op.as_str();
        if op == "open" {
            self.reset();
            match kept.message {
                Some(message) if kept.inputs.len() == 1 => {
                    self.name = message
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    self.base = kept.inputs.into_iter().next();
                    self.opened = fingerprint(reply, outputs);
                    self.opened_ms = ms;
                }
                _ => self.unavailable = Some(Unavailable::Budget),
            }
            return;
        }
        if op == "close" {
            self.reset();
            return;
        }
        if op == "authenticate" && reply.get("authenticated") != Some(&Value::Bool(true)) {
            return;
        }
        if op == "clip-copy" && !self.available() {
            self.clipboard_in_log = false;
            return;
        }
        if !self.available() {
            return;
        }
        let Some(message) = kept.message else {
            return self.off(Unavailable::Budget);
        };
        if op == "clip-paste" && !self.clipboard_in_log {
            return self.off(Unavailable::Clipboard);
        }
        if let Some(generation) = message_generation(&message) {
            let known = self.generation();
            if known.is_some_and(|known| known != generation)
                || generation < 1.0
                || generation - 1.0 > MAX_REPLAY_CLOSES as f64
            {
                return self.off(Unavailable::Generation);
            }
        }
        let mut capture = None;
        if op == "saved"
            && let Some(ticket) = message.get("ticket").and_then(Value::as_str)
        {
            capture = self.captures.get(ticket).map(|c| c.index);
            // A ticket the log never saw issued cannot be mapped onto a replay.
            if capture.is_none() {
                return self.off(Unavailable::Generation);
            }
        }
        let clip_copy = op == "clip-copy";
        self.entry_bytes += kept.bytes;
        self.entries.push(RecoveryEntry {
            op: kept.op.clone(),
            message,
            inputs: kept.inputs,
            bytes: kept.bytes,
            reply: fingerprint(reply, outputs),
            ms: ms.max(0.0),
            capture,
        });
        if clip_copy {
            self.clipboard_in_log = true;
        }
    }

    /// V:377 `savedCopy`: after an acknowledged save in place, `bytes` (a copy of what was
    /// written, or None when it could not be kept) becomes the fallback a failed replay reopens.
    pub fn saved_copy(&mut self, bytes: Option<Vec<u8>>) {
        if !self.available() {
            return;
        }
        self.saved = None;
        match bytes {
            Some(bytes) if self.bytes() + bytes.len() <= self.budget => {
                self.saved = Some(bytes);
                self.saved_lost = false;
            }
            _ => self.saved_lost = true,
        }
    }

    /// V:388 `fitsSaved`: whether a saved copy of `size` bytes fits (the previous one is replaced).
    pub fn fits_saved(&self, size: usize) -> bool {
        self.available()
            && self.bytes() - self.saved.as_ref().map_or(0, Vec::len) + size <= self.budget
    }

    /// V:393 `replayBudgetMs`.
    pub fn replay_budget_ms(&self) -> f64 {
        REPLAY_BASE_MS + 2.0 * (self.opened_ms + self.entries.iter().map(|e| e.ms).sum::<f64>())
    }

    /// V:398 `generation`: the document generation the logged edit handles carry.
    pub fn generation(&self) -> Option<f64> {
        if let Some(generation) = self
            .entries
            .iter()
            .find_map(|e| message_generation(&e.message))
        {
            return Some(generation);
        }
        // A save whose `saved` is still to come names it too.
        self.captures
            .keys()
            .find_map(|ticket| ticket_generation(&Value::String(ticket.clone())))
    }

    /// V:415 `noteSave`: a `save` answered with `ticket` names the log's current length.
    pub fn note_save(&mut self, ticket: Option<&Value>) {
        let Some(ticket) = ticket else { return };
        if !self.available() || ticket_generation(ticket).is_none() {
            return;
        }
        if let Some(text) = ticket.as_str() {
            self.captures.insert(
                text.to_string(),
                Capture {
                    index: self.entries.len(),
                    epoch: self.epoch,
                },
            );
        }
    }

    /// V:427 `replayTicket`: the ticket a replayed session accepts for a state captured after
    /// `capture` entries: only the security part (the count of `security-set`s) differs.
    pub fn replay_ticket(&self, ticket: &str, capture: usize) -> String {
        let mut parts: Vec<String> = ticket.split(':').map(str::to_string).collect();
        if parts.len() < 3 {
            return ticket.to_string();
        }
        let count = self
            .entries
            .iter()
            .take(capture)
            .filter(|e| e.op == "security-set")
            .count();
        parts[2] = count.to_string();
        parts.join(":")
    }

    /// V:435 `restored`: tickets issued before a successful replay are rebuilt from now on.
    pub fn restored(&mut self) {
        self.epoch += 1;
    }

    /// V:440 `ticketNow`: one issued before the last restore, rebuilt for the restored session.
    pub fn ticket_now(&mut self, ticket: &str) -> String {
        let Some(capture) = self.captures.get(ticket).copied() else {
            return ticket.to_string();
        };
        if capture.epoch == self.epoch {
            return ticket.to_string();
        }
        let rebuilt = self.replay_ticket(ticket, capture.index);
        self.captures.insert(
            rebuilt.clone(),
            Capture {
                index: capture.index,
                epoch: self.epoch,
            },
        );
        rebuilt
    }

    /// V:449 `fallback`: the last saved copy, else the base; None when neither is the saved state.
    pub fn fallback(&self) -> Option<Vec<u8>> {
        if let Some(saved) = &self.saved {
            return Some(saved.clone());
        }
        if self.saved_lost {
            None
        } else {
            self.base.clone()
        }
    }
}

/// Why a replay stopped (V:456 `ReplayMismatch`, V:463 `ReplayOverBudget`, or a refusal).
#[derive(Clone, Debug, PartialEq)]
pub enum ReplayError {
    Mismatch { op: String, index: usize },
    OverBudget { done: usize, total: usize },
    Other(String),
}

impl std::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReplayError::Mismatch { op, index } => {
                write!(
                    f,
                    "replay: {op} (step {}) did not answer as it did before",
                    index + 1
                )
            }
            ReplayError::OverBudget { done, total } => write!(
                f,
                "replay: still running after its time budget ({done} of {total} changes replayed)"
            ),
            ReplayError::Other(message) => f.write_str(message),
        }
    }
}

impl Session {
    /// V:493 `replay`: reopens the log's base in the restarted engine and sends its log again,
    /// in order, through `request(…, replaying)`. Stops on the first answer that differs from
    /// the original, on a refusal and past the time budget (checked between steps).
    /// `on_step(done, total)` runs after the open (done 0) and after each replayed change.
    pub(crate) fn replay(
        &mut self,
        mut on_step: impl FnMut(&mut Session, usize, usize),
    ) -> Result<ReplayResult, ReplayError> {
        let budget = self.recovery.replay_budget_ms().max(0.0);
        let deadline = Instant::now() + Duration::from_secs_f64(budget / 1000.0);
        let total = self.recovery.entries.len();
        let Some(base) = self.recovery.base.clone() else {
            return Err(ReplayError::Other(
                "replay: recovery is not available for this document".into(),
            ));
        };
        let generation = self.recovery.generation();
        // A fresh engine numbers its first open 1; each close before it adds one.
        let closes = generation.map_or(0, |g| (g - 1.0).max(0.0) as i64);
        for _ in 0..closes {
            self.request_replaying(json!({"op": "close"}), Vec::new())
                .map_err(|err| ReplayError::Other(err.message))?;
        }
        let name = self.recovery.name.clone();
        let open = self
            .request_replaying(
                json!({"op": "open", "pdf": {"$buffer": 0}, "name": name}),
                vec![base],
            )
            .map_err(|err| ReplayError::Other(err.message))?;
        if fingerprint(&open.value, &open.outputs) != self.recovery.opened {
            return Err(ReplayError::Other(
                "replay: the document did not open as it did before".into(),
            ));
        }
        let open_map = open.value.as_object().cloned().unwrap_or_default();
        let mut journal = open_map.clone();
        let mut pages = open_map.clone();
        on_step(self, 0, total);
        for index in 0..total {
            if Instant::now() > deadline {
                return Err(ReplayError::OverBudget { done: index, total });
            }
            let Some(entry) = self.recovery.entries.get(index).cloned() else {
                break;
            };
            let mut message = entry.message.clone();
            if let (Some(capture), Some(ticket)) = (
                entry.capture,
                entry.message.get("ticket").and_then(Value::as_str),
            ) {
                message.insert(
                    "ticket".into(),
                    Value::String(self.recovery.replay_ticket(ticket, capture)),
                );
            }
            let reply = self
                .request_replaying(Value::Object(message), entry.inputs.clone())
                .map_err(|err| ReplayError::Other(err.message))?;
            if fingerprint(&reply.value, &reply.outputs) != entry.reply {
                return Err(ReplayError::Mismatch {
                    op: entry.op.clone(),
                    index,
                });
            }
            if reply.get("revision").is_some_and(Value::is_number) {
                journal = reply.value.as_object().cloned().unwrap_or_default();
            }
            if reply.get("pages").is_some_and(Value::is_array)
                && reply.get("pageCount").is_some_and(Value::is_number)
            {
                pages = reply.value.as_object().cloned().unwrap_or_default();
            }
            on_step(self, index + 1, total);
        }
        Ok(ReplayResult {
            open: open_map,
            journal,
            pages,
            steps: total,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cyrb53_matches_js() {
        // cyrb53("a") and cyrb53("") with seed 0, from the JS reference implementation.
        assert_eq!(hash("a"), to36(7_929_297_801_672_961).to_string());
        assert_eq!(hash(""), to36(3_338_908_027_751_811).to_string());
        // UTF-16 code units, as `charCodeAt` reads them (checked with node).
        assert_eq!(hash("{\"ok\":true,\"x\":\"é😀\"}"), "kb517wjssa");
    }

    fn to36(mut value: u64) -> String {
        let digits = b"0123456789abcdefghijklmnopqrstuvwxyz";
        let mut out = Vec::new();
        while value > 0 {
            out.push(digits[(value % 36) as usize]);
            value /= 36;
        }
        out.reverse();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn fingerprint_drops_volatile_keys_and_sizes_buffers() {
        let a = json!({"id": 1, "ok": true, "openMs": 5, "bytes": {"$buffer": 0}, "revision": 2});
        let b = json!({"id": 9, "ok": true, "openMs": 7, "bytes": {"$buffer": 0}, "revision": 2});
        assert_eq!(
            fingerprint(&a, &[vec![1, 2]]),
            fingerprint(&b, &[vec![3, 4]])
        );
        assert_ne!(fingerprint(&a, &[vec![1, 2]]), fingerprint(&b, &[vec![3]]));
    }

    #[test]
    fn sizes_and_tickets() {
        let message = json!({"op": "annot-set", "contents": "ab", "pdf": {"$buffer": 0}});
        // object 16 + "annot-set" (16+18) + "ab" (16+4) + buffer (16+3)
        assert_eq!(size_of(&message, &[vec![0; 3]]), 16 + 34 + 20 + 19);
        assert_eq!(ticket_generation(&json!("3:7:0")), Some(3.0));
        assert_eq!(ticket_generation(&json!("x:7")), None);
        let mut log = RecoveryLog::new(1000);
        log.base = Some(vec![0; 10]);
        log.note_save(Some(&json!("1:4:0")));
        assert_eq!(log.generation(), Some(1.0));
        assert_eq!(log.ticket_now("1:4:0"), "1:4:0");
        log.restored();
        assert_eq!(log.ticket_now("1:4:9"), "1:4:9");
        assert_eq!(log.ticket_now("1:4:0"), "1:4:0");
        assert_eq!(
            recovery_budget_for(false, Some(4.0)),
            SMALL_RECOVERY_BUDGET_BYTES
        );
        assert_eq!(recovery_budget_for(false, Some(8.0)), RECOVERY_BUDGET_BYTES);
    }

    #[test]
    fn keep_and_record() {
        let mut log = RecoveryLog::new(1 << 20);
        let open = json!({"op": "open", "pdf": {"$buffer": 0}, "name": "a.pdf", "id": 3});
        let kept = log
            .keep(open.as_object().unwrap(), &[vec![7; 100]])
            .unwrap();
        assert!(kept.message.as_ref().is_some_and(|m| !m.contains_key("id")));
        log.record(kept, &json!({"ok": true, "pageCount": 1}), &[], 4.0);
        assert!(log.available() && log.name == "a.pdf" && log.bytes() == 100);
        let paste = json!({"op": "clip-paste", "page": 0});
        let kept = log.keep(paste.as_object().unwrap(), &[]).unwrap();
        log.record(kept, &json!({"ok": true}), &[], 1.0);
        assert_eq!(log.unavailable, Some(Unavailable::Clipboard));
        assert!(!log.available());
        assert!(
            log.keep(json!({"op": "render"}).as_object().unwrap(), &[])
                .is_none()
        );
    }
}
