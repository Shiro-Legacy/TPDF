//! Insert ▸ Image… and the Text tool's "Add image…": a PNG or JPEG picked through the
//! system's picker and drawn into the shown page as page content (engine `image-add`). It
//! lands centred on the page at its natural size (at most half the page each way), upright
//! on a turned page, and comes up picked in the Text tool, so it can be dragged, resized,
//! turned, cropped, replaced or deleted at once like any picture already on the page.

use super::Session;
use super::consts::{CONTENT_DETAIL_TEXT, lookup};
use super::nav::{pdf_box, view_box};
use super::types::*;
use serde_json::{Map, Value, json};
use tandem_protocol::engine::EditTargetKind;

/// The status line once the picture is on the page.
pub const IMAGE_ADDED: &str =
    "Added the picture. Drag it to move it, or drag a corner to resize it.";

impl Session {
    /// `image-add` on the shown page. `contents` (the drive's) may carry `page`, and `at`
    /// (the centre) or `box` in view points; the picture itself comes from the picker.
    pub fn add_image(&mut self, contents: Option<&str>) -> bool {
        if self.doc.is_none() || !self.can_edit() {
            if self.doc.is_some() {
                self.show_error(lookup(&CONTENT_DETAIL_TEXT, "read-only").unwrap_or_default());
            }
            return false;
        }
        let given: Map<String, Value> = contents
            .and_then(|text| serde_json::from_str::<Value>(text).ok())
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        let page = given
            .get("page")
            .and_then(Value::as_i64)
            .unwrap_or(self.wanted_page);
        let Some((name, bytes)) = self.pick_one_image() else {
            return false;
        };
        self.clear_error();
        let mut message = Map::new();
        message.insert("op".into(), json!("image-add"));
        message.insert("page".into(), json!(page));
        message.insert(
            "image".into(),
            json!({"name": name, "bytes": {"$buffer": 0}}),
        );
        // The drive's view points through the page transform, as paste takes its point.
        let wants_view = given.contains_key("at") || given.contains_key("box");
        if wants_view {
            let Some(m) = self.page_transform_of(page) else {
                return false;
            };
            if let Some(at) = given.get("at").and_then(numbers).filter(|p| p.len() == 2) {
                match pdf_box(&[at[0], at[1], at[0], at[1]], &m) {
                    Ok(bx) => {
                        message.insert("at".into(), json!([bx[0], bx[1]]));
                    }
                    Err(_) => return false,
                }
            }
            if let Some(bx) = given.get("box").and_then(numbers).filter(|b| b.len() == 4) {
                match pdf_box(&bx, &m) {
                    Ok(bx) => {
                        message.insert("box".into(), json!(bx));
                    }
                    Err(_) => return false,
                }
            }
        }
        let reply = match self.rq_with(Value::Object(message), vec![bytes]) {
            Ok(reply) => reply,
            Err(err) => {
                let detail = err
                    .reply
                    .get("detail")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| err.reason.clone().unwrap_or_default());
                let text = match detail.as_str() {
                    "unsupported-transform" => {
                        "A picture can't be added to this page: its content is scaled.".to_string()
                    }
                    "unbalanced-content" => {
                        "A picture can't be added to this page: its content is drawn in a way the editor can't add to.".to_string()
                    }
                    "too-large" if err.message.contains("targets") => {
                        "This page already holds as many items as the editor can handle.".to_string()
                    }
                    "too-large" => "That picture is too large. Pick one under 4 MB.".to_string(),
                    _ => lookup(&CONTENT_DETAIL_TEXT, &detail)
                        .map(str::to_string)
                        .unwrap_or_else(|| err.message.clone()),
                };
                self.announce(&text);
                self.show_error(&text);
                return false;
            }
        };
        let added = reply
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        self.content_edited = Some(ContentEdited::Object {
            kind: "image".into(),
            action: "add".into(),
            name: added.clone(),
            count: None,
            skipped: None,
        });
        if self.ui_mode != UiMode::Edit {
            self.set_mode(UiMode::Edit);
        }
        self.apply_edit(&super::modes::journal_of(&reply.value), IMAGE_ADDED);
        // Picked, as a paste picks what it pasted (set after the refresh, §10.34).
        let pick = self
            .content_scan
            .as_ref()
            .filter(|s| s.page == page)
            .and_then(|scan| {
                let m = scan.transform.as_ref()?;
                let item = scan.items.iter().find(|v| {
                    v.kind != EditTargetKind::Text
                        && v.object_name.as_deref() == Some(added.as_str())
                        && v.placement.is_some()
                })?;
                Some(ObjectFocus {
                    kind: item.kind.as_str().into(),
                    name: added.clone(),
                    target_id: item.target_id.clone(),
                    rect: view_box(&item.placement.as_ref()?.r#box, m),
                    text: None,
                })
            });
        self.clear_direct_focus();
        if let Some(pick) = pick {
            self.set_selection(vec![pick.clone()]);
            self.restore_direct_focus(true);
            self.sync_anchor(&pick);
        }
        true
    }

    /// The scanned page transform of `page`, scanning it first when the scan is stale.
    fn page_transform_of(&mut self, page: i64) -> Option<Vec<f64>> {
        let current = |s: &Self| {
            s.content_scan.as_ref().is_some_and(|scan| {
                scan.page == page
                    && scan.transform.is_some()
                    && scan.doc_generation == Some(s.generation)
            })
        };
        if !current(self) {
            self.refresh_content_targets(true);
        }
        self.content_scan
            .as_ref()
            .filter(|s| s.page == page)
            .and_then(|s| s.transform.clone())
    }
}

/// A JSON array of finite numbers.
fn numbers(value: &Value) -> Option<Vec<f64>> {
    value
        .as_array()?
        .iter()
        .map(|v| v.as_f64().filter(|n| n.is_finite()))
        .collect()
}
