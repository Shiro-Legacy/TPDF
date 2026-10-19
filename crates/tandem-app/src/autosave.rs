//! D8: unsaved edits survive a crash or a power cut.
//!
//! While a document has unsaved changes, its recovery log (the opened bytes plus every
//! edit since, `session::recovery_log`) is written to `<user data>/Recovery/<id>/` every
//! few seconds — never next to the person's file. Saving, or closing without saving,
//! removes it. On the next launch the start screen offers "Restore unsaved changes to
//! x.pdf?": the edits are replayed in a separate engine and the result opens as a new,
//! unsaved copy named after the original, so the original file is never changed behind
//! anyone's back. Undo history from before the crash is not restored.

use crate::engine::EngineHost;
use crate::session::Session;
use serde_json::{Map, Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How often unsaved changes are written at most.
const INTERVAL: Duration = Duration::from_secs(3);

#[derive(Debug, Default)]
pub struct Autosave {
    /// Off in headless runs and tests.
    pub enabled: bool,
    dir: Option<PathBuf>,
    last_write: Option<Instant>,
    /// (document name, base length, entry count) last written.
    last_key: Option<(String, usize, usize)>,
}

/// A document left behind by an earlier run.
#[derive(Debug, Clone, PartialEq)]
pub struct Recovered {
    pub id: String,
    pub name: String,
    pub edits: usize,
    pub saved_at: f64,
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16
            | (*chunk.get(1).unwrap_or(&0) as u32) << 8
            | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn unb64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut buf = 0u32;
    let mut bits = 0;
    for c in text.bytes() {
        if c == b'=' {
            break;
        }
        let v = B64.iter().position(|b| *b == c)? as u32;
        buf = buf << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits & 0xff) as u8);
        }
    }
    Some(out)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(tmp, path)
}

fn recovery_root(user_data: &Path) -> PathBuf {
    user_data.join("Recovery")
}

/// Documents a previous run left unsaved, newest first.
pub fn pending(user_data: &Path) -> Vec<Recovered> {
    let mut out = Vec::new();
    let Ok(dirs) = fs::read_dir(recovery_root(user_data)) else {
        return out;
    };
    for dir in dirs.flatten() {
        let manifest = dir.path().join("manifest.json");
        let Ok(text) = fs::read_to_string(&manifest) else {
            continue;
        };
        let Ok(m) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        out.push(Recovered {
            id: dir.file_name().to_string_lossy().into_owned(),
            name: m
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("Document.pdf")
                .to_string(),
            edits: m.get("edits").and_then(Value::as_u64).unwrap_or(0) as usize,
            saved_at: m.get("savedAt").and_then(Value::as_f64).unwrap_or(0.0),
        });
    }
    out.sort_by(|a, b| b.saved_at.total_cmp(&a.saved_at));
    out
}

/// Rebuilds a recovered document's bytes by replaying its edits in a fresh engine.
pub fn rebuild(user_data: &Path, id: &str) -> Result<(String, Vec<u8>), String> {
    let dir = recovery_root(user_data).join(id);
    let manifest: Value = serde_json::from_str(
        &fs::read_to_string(dir.join("manifest.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let name = manifest
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("Document.pdf")
        .to_string();
    let base = fs::read(dir.join("base.pdf")).map_err(|e| e.to_string())?;
    let log: Vec<Value> =
        serde_json::from_str(&fs::read_to_string(dir.join("log.json")).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let mut engine = EngineHost::new();
    engine
        .request(
            json!({"op": "open", "name": name, "pdf": {"$buffer": 0}}),
            vec![base],
        )
        .map_err(|e| e.message)?;
    for entry in log {
        let message = entry.get("message").cloned().unwrap_or(Value::Null);
        let inputs: Vec<Vec<u8>> = entry
            .get("inputs")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().and_then(unb64))
                    .collect()
            })
            .unwrap_or_default();
        // A step that fails (it should not) stops the rebuild where it is.
        if engine.request(message, inputs).is_err() {
            break;
        }
    }
    let mut saved = engine
        .request(json!({"op": "save"}), vec![])
        .map_err(|e| e.message)?;
    let bytes = saved
        .take_bytes("bytes")
        .ok_or("the engine returned no bytes")?;
    Ok((name, bytes))
}

pub fn discard(user_data: &Path, id: &str) {
    if !id.is_empty() && !id.contains('/') {
        let _ = fs::remove_dir_all(recovery_root(user_data).join(id));
    }
}

/// "Report.pdf" → "Report (recovered).pdf".
pub fn recovered_name(name: &str) -> String {
    match name.to_lowercase().strip_suffix(".pdf") {
        Some(_) => format!("{} (recovered).pdf", &name[..name.len() - 4]),
        None => format!("{name} (recovered).pdf"),
    }
}

impl Session {
    /// Called after every window event: writes or removes this document's recovery files.
    pub fn autosave_tick(&mut self) {
        if !self.autosave.enabled {
            return;
        }
        let user_data = self.host.user_data().to_path_buf();
        let dir = self
            .autosave
            .dir
            .get_or_insert_with(|| {
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0);
                recovery_root(&user_data).join(format!("{}-{stamp}", std::process::id()))
            })
            .clone();
        let log = &self.recovery;
        let worth_keeping =
            self.doc.is_some() && self.dirty && log.available() && !log.entries.is_empty();
        if !worth_keeping {
            if self.autosave.last_key.take().is_some() || dir.exists() {
                let _ = fs::remove_dir_all(&dir);
            }
            return;
        }
        let key = (
            log.name.clone(),
            log.base.as_ref().map_or(0, Vec::len),
            log.entries.len(),
        );
        if self.autosave.last_key.as_ref() == Some(&key) {
            return;
        }
        if self
            .autosave
            .last_write
            .is_some_and(|t| t.elapsed() < INTERVAL)
            && self.autosave.last_key.is_some()
        {
            return;
        }
        if self.write_recovery(&dir).is_ok() {
            self.autosave.last_key = Some(key);
            self.autosave.last_write = Some(Instant::now());
        }
    }

    fn write_recovery(&self, dir: &Path) -> std::io::Result<()> {
        fs::create_dir_all(dir)?;
        let log = &self.recovery;
        if let Some(base) = log.base.as_ref() {
            write_atomic(&dir.join("base.pdf"), base)?;
        }
        let entries: Vec<Value> = log
            .entries
            .iter()
            .filter(|e| crate::session::recovery_log::replayed(&Value::String(e.op.clone())))
            .map(|e| {
                let mut message: Map<String, Value> = e.message.clone();
                message.remove("id");
                json!({ "op": e.op, "message": Value::Object(message), "inputs": e.inputs.iter().map(|b| b64(b)).collect::<Vec<_>>() })
            })
            .collect();
        write_atomic(
            &dir.join("log.json"),
            serde_json::to_string(&entries)?.as_bytes(),
        )?;
        let manifest = json!({
            "name": self.doc.as_ref().map(|d| d.name.clone()).unwrap_or_default(),
            "edits": entries.len(),
            "savedAt": crate::engine::now_ms(),
        });
        write_atomic(&dir.join("manifest.json"), manifest.to_string().as_bytes())
    }

    /// The window is closing for good (the person chose not to save): nothing to recover.
    pub fn autosave_forget(&mut self) {
        if let Some(dir) = self.autosave.dir.take() {
            let _ = fs::remove_dir_all(dir);
        }
        self.autosave.last_key = None;
    }

    /// Restores a document an earlier run left unsaved.
    pub fn autosave_restore(&mut self, id: &str) {
        let user_data = self.host.user_data().to_path_buf();
        match rebuild(&user_data, id) {
            Ok((name, bytes)) => {
                self.host.open_untitled(&recovered_name(&name), &bytes);
                discard(&user_data, id);
            }
            Err(_) => {
                self.show_error(&format!(
                    "Your unsaved changes to {} could not be restored. The original file is unchanged; open it to continue.",
                    pending(&user_data).iter().find(|r| r.id == id).map(|r| r.name.clone()).unwrap_or_else(|| "the document".into())
                ));
                discard(&user_data, id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips() {
        for len in 0..10 {
            let data: Vec<u8> = (0..len).map(|i| (i * 37 + 5) as u8).collect();
            assert_eq!(unb64(&b64(&data)).unwrap(), data);
        }
        assert_eq!(b64(b"Man"), "TWFu");
        assert_eq!(b64(b"Ma"), "TWE=");
    }

    #[test]
    fn names() {
        assert_eq!(recovered_name("Report.pdf"), "Report (recovered).pdf");
        assert_eq!(recovered_name("x"), "x (recovered).pdf");
    }
}
