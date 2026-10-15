// SPDX-License-Identifier: AGPL-3.0-or-later
//! Op routing: one module per op group.
pub mod core;
pub mod fields;
pub mod pages;
pub mod view;

use super::{AppEngine, AppError, AppSession, Buffers, request::js_string};
use serde_json::Value;

/// The engine.ts dispatch switch.
pub(crate) fn route(
    engine: &mut AppEngine,
    session: &mut AppSession,
    request: &Value,
    buffers: &mut Buffers,
) -> Result<Value, AppError> {
    let op = request.get("op");
    match op.and_then(Value::as_str) {
        Some("open") => core::open(engine, session, request, buffers),
        Some("authenticate") => core::authenticate(session, request),
        Some("close") => Ok(core::close(engine, session, request)),
        Some(op @ ("undo" | "redo")) => core::undo_redo(session, request, op),
        Some("saved") => core::saved(session, request),
        Some("security-set") => core::security_set(session, request),
        Some("save") => core::save(session, request, buffers),
        Some("validate") => core::validate(session, request, buffers),
        Some("page-ids") => Ok(core::page_ids(session, request)),
        Some("render") => view::render(session, request, buffers),
        Some("thumbnail") => view::thumbnail(session, request, buffers),
        Some("search") => view::search(session, request),
        Some("text-all") => view::text_all(session, request),
        Some("layers") => view::layers(session, request),
        Some("layer-set") => view::layer_set(session, request),
        Some("page-rotate") => pages::rotate(session, request),
        Some("page-delete") => pages::delete(session, request),
        Some("page-move") => pages::move_page(session, request),
        Some("insert-pages") => pages::insert(session, request, buffers),
        Some("extract-pages") => pages::extract(session, request, buffers),
        Some("pdf-from-images") => pages::from_images(request, buffers),
        Some("labels") => pages::list_labels(session, request),
        Some("labels-set") => pages::labels_set(session, request),
        Some("labels-clear") => pages::labels_clear(session, request),
        Some("fields") => fields::list(session, request),
        Some("field-set") => fields::set(session, request),
        Some("field-add") => fields::add(session, request),
        Some("field-props") => fields::props(session, request),
        Some("field-delete") => fields::delete(session, request),
        _ => Err(AppError::plain(format!("unknown op: {}", js_string(op)))),
    }
}
