//! Exports, unit 7 (renderer.ts 9389–9532): the export dialog, a page as PNG or JPEG, the
//! whole text as .txt, "New from images…", and the one place a new file leaves the app.
#![allow(dead_code)]

use super::Session;
use super::consts::PRINT_MAX_BYTES;
use super::dom_ids as ids;
use super::field_designer::js_to_number;
use super::types::*;
use crate::host::SaveKind;
use crate::messages;
use serde_json::{Value, json};
use tandem_protocol::limits::MAX_SCALE;

/// R:9392: file stem for a suggested name: `nav.pdf` to `nav` (`/\.[A-Za-z0-9]+$/`).
pub fn file_stem(name: &str) -> String {
    let ext = name
        .bytes()
        .rev()
        .take_while(u8::is_ascii_alphanumeric)
        .count();
    let cut = name.len() - ext;
    let stem = if ext > 0 && name[..cut].ends_with('.') {
        &name[..cut - 1]
    } else {
        name
    };
    if stem.is_empty() {
        "document".into()
    } else {
        stem.into()
    }
}

impl Session {
    /// R:9400: main owns the dialog, the extension and the write; a cancel is not an error
    /// and reads as None. `kind` is "pdf" | "png" | "jpeg" | "txt".
    pub fn save_new_file(
        &mut self,
        bytes: &[u8],
        suggested_name: &str,
        kind: &str,
        open: bool,
    ) -> Option<String> {
        let save_kind = SaveKind::parse(kind)?;
        match self
            .host
            .save_new(bytes, Some(suggested_name), save_kind, open, false)
        {
            Ok(saved) => Some(saved.name),
            Err(fail) => {
                let reason = fail.as_str();
                if reason == "cancelled" {
                    return None;
                }
                let what = if kind == "txt" {
                    "text".to_string()
                } else {
                    kind.to_uppercase()
                };
                let text = match messages::save::new_file(reason) {
                    Some(text) => format!("The {what} was not saved. {text}"),
                    None => format!("The {what} was not saved ({reason})."),
                };
                self.show_error(&text);
                None
            }
        }
    }

    /// R:9415: "Export…": the dialog picks a kind and a resolution.
    pub fn request_export(&mut self) {
        if self.doc.as_ref().is_none_or(|doc| doc.info.needs_password) {
            return;
        }
        self.dom.set_open(ids::EXPORT_DIALOG, true);
    }

    /// R:9421.
    pub fn sync_export_dpi(&mut self) {
        let txt = self.dom.value(ids::EXPORT_KIND_SELECT) == "txt";
        self.dom.set_disabled(ids::EXPORT_DPI_SELECT, txt);
    }

    /// UI entry: `#export-kind` changed (R:10991).
    pub fn on_export_kind(&mut self, kind: &str) {
        self.dom.set_value(ids::EXPORT_KIND_SELECT, kind);
        self.sync_export_dpi();
    }

    /// R:9425.
    pub fn save_export_dialog(&mut self) {
        let kind = match self.dom.value(ids::EXPORT_KIND_SELECT).as_str() {
            "jpeg" => "jpeg",
            "txt" => "txt",
            _ => "png",
        };
        let dpi = js_to_number(&self.dom.value(ids::EXPORT_DPI_SELECT));
        let dpi = if dpi == 0.0 || dpi.is_nan() {
            150.0
        } else {
            dpi
        };
        self.dom.set_open(ids::EXPORT_DIALOG, false);
        self.export_current(kind, dpi);
    }

    /// R:9436: one export: a page rendered at `dpi` and encoded, or the whole text. Nothing
    /// here touches the document's journal or dirty state.
    pub fn export_current(&mut self, kind: &str, dpi: f64) -> bool {
        let Some(doc) = self.doc.clone() else {
            return false;
        };
        self.export_summary = None;
        // Redaction §2.3 step 7: an exported page or its text still holds what unredacted boxes cover.
        if !self.confirm_pending_save(PendingAction::Export) {
            return false;
        }
        if kind == "txt" {
            let text = match self.request(json!({"op": "text-all"}), vec![]) {
                Ok(reply) => reply
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                Err(err) => {
                    let message = self.message_of(&err);
                    self.show_error(&message);
                    return false;
                }
            };
            let bytes = text.into_bytes();
            let Some(_) = self.save_new_file(
                &bytes,
                &format!("{}.txt", file_stem(&doc.name)),
                "txt",
                false,
            ) else {
                return false;
            };
            self.export_summary = Some(ExportSummary {
                kind: "txt".into(),
                bytes: bytes.len() as i64,
                pages: Some(doc.page_count),
                ..ExportSummary::default()
            });
            let plural = if doc.page_count == 1 { "" } else { "s" };
            self.announce_page(&format!("Exported text ({} page{plural})", doc.page_count));
            return true;
        }

        if !self.print_available() {
            self.show_error(
                "This document does not permit printing, so a page cannot be exported.",
            );
            return false;
        }
        let page = self.wanted_page;
        // The engine caps a render at MAX_SCALE (4) and a 4096 px edge; the status line
        // reports the pixels actually made.
        let scale = MAX_SCALE.min(dpi / 72.0);
        let mut reply = match self.request(
            json!({"op": "render", "page": page, "scale": scale}),
            vec![],
        ) {
            Ok(reply) => reply,
            Err(err) => {
                let message = self.message_of(&err);
                self.show_error(&message);
                return false;
            }
        };
        let pixels = reply.take_bytes("pixels").unwrap_or_default();
        let width = reply.get("pixelWidth").and_then(Value::as_i64).unwrap_or(0);
        let height = reply
            .get("pixelHeight")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        if pixels.len() > PRINT_MAX_BYTES {
            self.show_error(
                "That page is too many pixels to export at this resolution; try a lower dpi.",
            );
            return false;
        }
        // The TS encoded on an OffscreenCanvas: PNG lossless, JPEG at quality 0.9.
        let (w, h) = (
            u32::try_from(width).unwrap_or(0),
            u32::try_from(height).unwrap_or(0),
        );
        let encoded = if kind == "png" {
            crate::imaging::encode_png(&pixels, w, h)
        } else {
            crate::imaging::encode_jpeg(&pixels, w, h)
        };
        let bytes = match encoded {
            Ok(bytes) => bytes,
            Err(message) => {
                self.show_error(&message);
                return false;
            }
        };
        let extension = if kind == "png" { "png" } else { "jpg" };
        let suggested = format!("{} page {}.{extension}", file_stem(&doc.name), page + 1);
        if self
            .save_new_file(&bytes, &suggested, kind, false)
            .is_none()
        {
            return false;
        }
        self.export_summary = Some(ExportSummary {
            kind: kind.into(),
            bytes: bytes.len() as i64,
            width: Some(width as f64),
            height: Some(height as f64),
            pages: None,
        });
        self.announce_page(&format!(
            "Exported page {} as {} ({width}×{height})",
            page + 1,
            kind.to_uppercase()
        ));
        true
    }

    /// R:9490: "New from images…": pick images, build one PDF, save it and let main open it.
    /// The engine decodes the images (`pdf-from-images`), as it did for the TS.
    pub fn request_new_from_images(&mut self) {
        let images = match self.host.pick_images() {
            Ok(images) => images,
            Err(reason) => {
                if reason != "cancelled" {
                    self.show_error(match reason {
                        "too-many" => "Too many images; pick at most 50.",
                        "too-large" => "Those images are larger than this app will read at once.",
                        _ => "Those images could not be read.",
                    });
                }
                return;
            }
        };
        let Some(first) = images.first() else {
            return;
        };
        let suggested = format!("{}.pdf", file_stem(&first.name));
        let list: Vec<Value> = images
            .iter()
            .enumerate()
            .map(|(index, image)| json!({"name": image.name, "bytes": {"$buffer": index}}))
            .collect();
        let inputs = images.into_iter().map(|image| image.bytes).collect();
        let mut reply = match self
            .engine
            .request(json!({"op": "pdf-from-images", "images": list}), inputs)
        {
            Ok(reply) => reply,
            Err(err) => {
                let message = self.message_of(&err);
                self.show_error(&message);
                return;
            }
        };
        let pages = reply.get("pageCount").and_then(Value::as_i64).unwrap_or(0);
        let bytes = reply.take_bytes("bytes").unwrap_or_default();
        let Some(name) = self.save_new_file(&bytes, &suggested, "pdf", true) else {
            return;
        };
        let plural = if pages == 1 { "" } else { "s" };
        self.announce_page(&format!("Created {name} ({pages} page{plural})"));
        self.console_line(
            "P2_NEW",
            serde_json::json!({"pages": pages, "bytes": bytes.len()}),
        );
    }

    /// R:9524: `--edit=export-png|export-jpeg|export-txt`: page 1 at 150 dpi (or all text), then log.
    pub fn smoke_export(&mut self, kind: &str) {
        if self.doc.is_none() {
            self.log_edit();
            return;
        }
        self.export_current(kind, 150.0);
        self.log_edit();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stems() {
        assert_eq!(file_stem("nav.pdf"), "nav");
        assert_eq!(file_stem("scan.final.JPG"), "scan.final");
        assert_eq!(file_stem("noext"), "noext");
        assert_eq!(file_stem(".pdf"), "document");
        assert_eq!(file_stem("a.b-c"), "a.b-c");
    }
}
