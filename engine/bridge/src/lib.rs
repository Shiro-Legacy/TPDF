// SPDX-License-Identifier: AGPL-3.0-or-later
#![forbid(unsafe_code)]
use serde_json::Value;
use std::collections::BTreeMap;
use tandem_pdf_engine::app::{AppEngine, AppError, AppSession, error_reply, request};
use wasm_bindgen::prelude::*;

/// Installs the font every session's view draws CJK text in when the PDF
/// does not embed one (MuPDF's Droid Sans Fallback), for the whole module;
/// false for bytes that are not a usable TrueType or OpenType font.
#[wasm_bindgen]
pub fn set_cjk_font(bytes: Vec<u8>) -> bool {
    tandem_pdf_engine::view::set_cjk_font(bytes).is_ok()
}

/// Whether a render since the last install attempt drew nothing for CJK
/// text for want of that font: the cue to install it and, when
/// `set_cjk_font` returns true, render again. After a refused font renders
/// no longer raise it, so the host cannot loop.
#[wasm_bindgen]
pub fn cjk_font_wanted() -> bool {
    tandem_pdf_engine::view::cjk_font_wanted()
}

/// Whether a CJK font is installed.
#[wasm_bindgen]
pub fn has_cjk_font() -> bool {
    tandem_pdf_engine::view::has_cjk_font()
}

/// The application engine behind the worker's `SessionRequest` protocol.
/// Requests and replies are JSON; binary fields are `{"$buffer": n}`
/// placeholders into the inputs pushed before a dispatch and the outputs it
/// leaves behind. A panic poisons the object: wasm-bindgen refuses every
/// later call, so the adapter treats any throw as fatal.
#[wasm_bindgen]
#[derive(Default)]
pub struct RustApp {
    engine: AppEngine,
    sessions: BTreeMap<u32, AppSession>,
    next_session: u32,
    inputs: Vec<Vec<u8>>,
    outputs: Vec<Vec<u8>>,
}
#[wasm_bindgen]
impl RustApp {
    #[wasm_bindgen(constructor)]
    pub fn new() -> RustApp {
        Self::default()
    }
    pub fn create_session(&mut self) -> u32 {
        self.next_session = self.next_session.wrapping_add(1);
        self.sessions
            .insert(self.next_session, AppSession::default());
        self.next_session
    }
    pub fn drop_session(&mut self, session: u32) {
        self.sessions.remove(&session);
    }
    /// Queues an input buffer; the next dispatch takes every queued input.
    pub fn push_input(&mut self, bytes: Vec<u8>) {
        self.inputs.push(bytes);
    }
    /// Drops queued inputs, so a request abandoned before its dispatch leaves
    /// nothing for the next one's placeholders to bind.
    pub fn clear_inputs(&mut self) {
        self.inputs.clear();
    }
    /// One request in, one reply out. Errors are `ok: false` replies, never throws.
    pub fn dispatch(&mut self, session: u32, request: &str) -> String {
        let inputs = std::mem::take(&mut self.inputs);
        self.outputs.clear();
        let request = match serde_json::from_str::<Value>(request) {
            Ok(request) => request,
            Err(error) => {
                let error = AppError::plain(format!("invalid request: {error}"));
                let fallback = AppSession::default();
                let session = self.sessions.get(&session).unwrap_or(&fallback);
                return error_reply(&Value::Null, error, session).to_string();
            }
        };
        let Some(state) = self.sessions.get_mut(&session) else {
            let error = AppError::plain(format!("no session {session}"));
            return error_reply(&request, error, &AppSession::default()).to_string();
        };
        let (reply, outputs) = self.engine.dispatch(state, request, inputs);
        self.outputs = outputs;
        reply.to_string()
    }
    pub fn output_count(&self) -> u32 {
        self.outputs.len() as u32
    }
    /// Moves an output out; a second take of the same index is empty.
    pub fn take_output(&mut self, index: u32) -> Vec<u8> {
        self.outputs
            .get_mut(index as usize)
            .map(std::mem::take)
            .unwrap_or_default()
    }
    /// `AppSession::state_json`, or `null` for an unknown session.
    pub fn session_state(&self, session: u32) -> String {
        self.sessions
            .get(&session)
            .map_or(Value::Null, AppSession::state_json)
            .to_string()
    }
    /// The session's current document bytes, for test inspection.
    pub fn snapshot_bytes(&self, session: u32) -> Option<Vec<u8>> {
        self.sessions.get(&session)?.snapshot_bytes()
    }
    /// The bytes of the session's compare document, for test inspection.
    pub fn compare_bytes(&self, session: u32) -> Option<Vec<u8>> {
        let compare = self.sessions.get(&session)?.compare.as_ref()?;
        Some(compare.bytes().to_vec())
    }
    /// Test-only: `AppSession::debug_json`, or `null` for an unknown session.
    pub fn session_debug(&self, session: u32) -> String {
        self.sessions
            .get(&session)
            .map_or(Value::Null, AppSession::debug_json)
            .to_string()
    }
    /// Test-only: the fields tests assign directly on an engine.ts session,
    /// `{"sourceBytes": n}`, `{"clipboardFrom": session | null}` (a copy of
    /// another session's clipboard) and `{"signedSource": null}` (drops the
    /// retained signed source). Anything else changes nothing and is false.
    pub fn session_set(&mut self, session: u32, fields: &str) -> bool {
        let Ok(Value::Object(fields)) = serde_json::from_str::<Value>(fields) else {
            return false;
        };
        let mut source_bytes = None;
        let mut clipboard = None;
        let mut drop_signed = false;
        for (key, value) in &fields {
            match key.as_str() {
                "sourceBytes" => {
                    let bytes = request::as_int(value).and_then(|n| usize::try_from(n).ok());
                    let Some(bytes) = bytes else { return false };
                    source_bytes = Some(bytes);
                }
                "clipboardFrom" if value.is_null() => clipboard = Some(None),
                "clipboardFrom" => {
                    let from = request::as_int(value).and_then(|n| u32::try_from(n).ok());
                    let Some(from) = from.and_then(|from| self.sessions.get(&from)) else {
                        return false;
                    };
                    clipboard = Some(from.clipboard.clone());
                }
                "signedSource" if value.is_null() => drop_signed = true,
                _ => return false,
            }
        }
        let Some(state) = self.sessions.get_mut(&session) else {
            return false;
        };
        if let Some(bytes) = source_bytes {
            state.source_bytes = bytes;
        }
        if let Some(clipboard) = clipboard {
            state.clipboard = clipboard;
        }
        if drop_signed {
            state.signed_source = None;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(app: &mut RustApp, session: u32, request: &str) -> Value {
        serde_json::from_str(&app.dispatch(session, request)).unwrap()
    }

    /// One page drawing 日 at 100 pt in a Song font without a program
    /// (MuPDF's `pdf_add_cjk_font` shape).
    fn cjk_page() -> Vec<u8> {
        let content = "BT /C0 100 Tf 0 10 Td <65E5> Tj ET";
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] \
             /Resources << /Font << /C0 5 0 R >> >> /Contents 4 0 R >>"
                .into(),
            format!(
                "<< /Length {} >>\nstream\n{content}\nendstream",
                content.len()
            ),
            "<< /Type /Font /Subtype /Type0 /BaseFont /Song /Encoding /UniGB-UTF16-H \
             /DescendantFonts [6 0 R] >>"
                .into(),
            "<< /Type /Font /Subtype /CIDFontType0 /BaseFont /Song /CIDSystemInfo \
             << /Registry (Adobe) /Ordering (GB1) /Supplement 5 >> /FontDescriptor 7 0 R >>"
                .into(),
            "<< /Type /FontDescriptor /FontName (Song) /FontBBox [-200 -200 1200 1200] \
             /Flags 6 /ItalicAngle 0 /Ascent 1000 /Descent -200 /StemV 80 >>"
                .into(),
        ];
        let mut bytes = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (index, body) in objects.iter().enumerate() {
            offsets.push(bytes.len());
            bytes.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", index + 1).as_bytes());
        }
        let xref = bytes.len();
        bytes.extend_from_slice(b"xref\n0 8\n0000000000 65535 f \n");
        for offset in offsets {
            bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        let trailer = format!("trailer\n<< /Size 8 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n");
        bytes.extend_from_slice(trailer.as_bytes());
        bytes
    }

    /// A TrueType font of two glyphs: .notdef (empty) and a square from 100 to
    /// 900 units, mapped from U+65E5.
    fn square_font() -> Vec<u8> {
        let be16 = |v: u16| v.to_be_bytes();
        let mut head = vec![0u8; 54];
        head[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
        head[18..20].copy_from_slice(&be16(1000));
        head[50..52].copy_from_slice(&be16(1));
        let mut hhea = vec![0u8; 36];
        hhea[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        hhea[34..36].copy_from_slice(&be16(2));
        let mut maxp = vec![0u8; 32];
        maxp[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        maxp[4..6].copy_from_slice(&be16(2));
        let hmtx = [0u8, 0, 0, 0, 0x03, 0xE8, 0, 0];
        let mut glyph = Vec::new();
        for v in [1i16, 100, 100, 900, 900, 3, 0] {
            glyph.extend_from_slice(&v.to_be_bytes());
        }
        glyph.extend_from_slice(&[1, 1, 1, 1]);
        // x deltas, then y deltas, of (100 100) (900 100) (900 900) (100 900).
        for v in [100i16, 800, 0, -800, 100, 0, 800, 0] {
            glyph.extend_from_slice(&v.to_be_bytes());
        }
        let loca: Vec<u8> = [0u32, 0, glyph.len() as u32]
            .iter()
            .flat_map(|v| v.to_be_bytes())
            .collect();
        // One format 4 subtable: U+65E5 to glyph 1, then the 0xFFFF segment.
        let mut cmap = Vec::new();
        for v in [0u16, 1, 3, 1] {
            cmap.extend_from_slice(&be16(v));
        }
        cmap.extend_from_slice(&12u32.to_be_bytes());
        let delta = 1u16.wrapping_sub(0x65E5);
        for v in [
            4u16, 32, 0, 4, 4, 1, 0, 0x65E5, 0xFFFF, 0, 0x65E5, 0xFFFF, delta, 1, 0, 0,
        ] {
            cmap.extend_from_slice(&be16(v));
        }
        let tables: [(&[u8; 4], &[u8]); 7] = [
            (b"cmap", &cmap),
            (b"glyf", &glyph),
            (b"head", &head),
            (b"hhea", &hhea),
            (b"hmtx", &hmtx),
            (b"loca", &loca),
            (b"maxp", &maxp),
        ];
        let mut out = 0x0001_0000u32.to_be_bytes().to_vec();
        for v in [7u16, 64, 2, 48] {
            out.extend_from_slice(&be16(v));
        }
        let mut offset = 12 + 16 * tables.len();
        for (tag, data) in &tables {
            out.extend_from_slice(*tag);
            out.extend_from_slice(&[0; 4]);
            out.extend_from_slice(&(offset as u32).to_be_bytes());
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            offset += data.len().next_multiple_of(4);
        }
        for (_, data) in &tables {
            out.extend_from_slice(data);
            out.resize(out.len().next_multiple_of(4), 0);
        }
        out
    }

    #[test]
    fn a_cjk_font_installed_on_the_cue_draws_the_text_and_a_refused_one_stops_it() {
        let mut app = RustApp::new();
        let session = app.create_session();
        app.push_input(cjk_page());
        let open = r#"{"id":1,"op":"open","name":"a.pdf","pdf":{"$buffer":0}}"#;
        assert_eq!(reply(&mut app, session, open)["ok"], true);
        let ink = |app: &mut RustApp| {
            let render = r#"{"id":2,"op":"render","page":0,"scale":1}"#;
            assert_eq!(reply(app, session, render)["ok"], true);
            let pixels = app.take_output(0);
            assert_eq!(pixels.len(), 120 * 120 * 4);
            pixels
                .chunks(4)
                .filter(|p| p[..3] != [255, 255, 255])
                .count()
        };
        assert_eq!(ink(&mut app), 0, "nothing drawn without the font");
        assert!(cjk_font_wanted() && !has_cjk_font());
        // A host rendering again on the cue stops after a refused font.
        assert!(!set_cjk_font(b"not a font".to_vec()));
        assert!(!cjk_font_wanted());
        assert_eq!(ink(&mut app), 0);
        assert!(!cjk_font_wanted(), "a refused font latches the cue off");
        assert!(set_cjk_font(square_font()));
        assert!(has_cjk_font() && !cjk_font_wanted());
        assert!(ink(&mut app) > 5000, "the square glyph drawn");
    }

    #[test]
    fn cleared_inputs_never_reach_the_next_request() {
        let mut app = RustApp::new();
        let session = app.create_session();
        app.push_input(vec![0xaa]);
        app.clear_inputs();
        app.push_input(vec![0xbb]);
        assert_eq!(app.inputs, [vec![0xbb]]);
        let close = reply(&mut app, session, r#"{"id":1,"op":"close"}"#);
        assert_eq!(close, serde_json::json!({"id": 1, "ok": true}));
        assert!(app.inputs.is_empty(), "a dispatch takes every queued input");
    }

    #[test]
    fn unparsable_requests_and_unknown_sessions_are_plain_errors() {
        let mut app = RustApp::new();
        let session = app.create_session();
        let invalid = reply(&mut app, session, "{");
        assert_eq!(invalid["ok"], false);
        assert!(invalid.get("id").is_none() && invalid.get("reason").is_none());
        let unknown = reply(&mut app, 99, r#"{"id":2,"op":"close"}"#);
        assert_eq!(
            (&unknown["id"], &unknown["error"]),
            (&2.into(), &"no session 99".into())
        );
        assert_eq!(app.session_state(99), "null");
        assert_eq!(app.session_debug(99), "null");
    }

    #[test]
    fn test_setters_change_only_what_they_name() {
        let mut app = RustApp::new();
        let a = app.create_session();
        let b = app.create_session();
        assert!(app.session_set(a, r#"{"sourceBytes":52428801}"#));
        let state: Value = serde_json::from_str(&app.session_state(a)).unwrap();
        assert_eq!(state["sourceBytes"], 52_428_801);
        app.sessions.get_mut(&a).unwrap().clipboard = Some(Default::default());
        assert!(app.session_set(b, &format!(r#"{{"clipboardFrom":{a}}}"#)));
        assert!(app.sessions[&b].clipboard.is_some());
        assert!(app.session_set(b, r#"{"clipboardFrom":null}"#));
        assert!(app.sessions[&b].clipboard.is_none());
        app.sessions.get_mut(&a).unwrap().signed_source = Some(vec![1]);
        assert!(app.session_set(a, r#"{"signedSource":null}"#));
        assert!(app.sessions[&a].signed_source.is_none());
        for refused in [
            r#"{"sourceBytes":-1}"#,
            r#"{"sourceBytes":1.5}"#,
            r#"{"clipboardFrom":99}"#,
            r#"{"name":"x"}"#,
            r#"{"sourceBytes":1,"other":2}"#,
            r#"{"signedSource":[1]}"#,
            "[]",
            "{",
        ] {
            assert!(!app.session_set(a, refused), "{refused}");
        }
        assert_eq!(app.sessions[&a].source_bytes, 52_428_801);
        assert!(!app.session_set(99, r#"{"sourceBytes":1}"#));
    }
}
