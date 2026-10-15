// SPDX-License-Identifier: AGPL-3.0-or-later
//! Op routing: one module per op group.
pub mod annots;
pub mod compare;
pub mod core;
pub mod edit;
pub mod fields;
pub mod nav;
pub mod pages;
pub mod redact;
pub mod sign;
pub mod stamp;
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
        Some("outline") => nav::list_outline(session, request),
        Some("outline-add") => nav::outline_add(session, request),
        Some("outline-set") => nav::outline_set(session, request),
        Some("outline-delete") => nav::outline_delete(session, request),
        Some("links") => nav::list_links(session, request),
        Some("link-add") => nav::link_add(session, request),
        Some("link-delete") => nav::link_delete(session, request),
        Some("link-set") => nav::link_set(session, request),
        Some("stamp") => stamp::stamp(session, request),
        Some("batch-file") => stamp::batch_file(request, buffers),
        Some("compare-open") => compare::open(session, request, buffers),
        Some("compare-close") => Ok(compare::close(session, request)),
        Some("compare-render") => compare::render(session, request, buffers),
        Some("compare-text") => compare::text(session, request),
        Some("sign-list") => sign::list(session, request, buffers),
        Some("sign-prepare") => sign::prepare(session, request, buffers),
        Some("annots") => annots::list(session, request),
        Some("annot-add") => annots::add(session, request, buffers),
        Some("annot-set") => annots::set(session, request),
        Some("annot-move") => annots::move_annotation(session, request),
        Some("annot-delete") => annots::delete(session, request),
        Some("fields") => fields::list(session, request),
        Some("field-set") => fields::set(session, request),
        Some("field-add") => fields::add(session, request),
        Some("field-props") => fields::props(session, request),
        Some("field-delete") => fields::delete(session, request),
        Some("edit-targets") => edit::targets(session, request),
        Some("edit-text") => edit::text(session, request),
        Some("edit-paragraph") => edit::paragraph(session, request),
        Some("text-add") => edit::text_add(session, request),
        Some("edit-replace-preview") => edit::replace_preview(session, request),
        Some("edit-replace-apply") => edit::replace_apply(session, request),
        Some("edit-object") => edit::object::edit_object(session, request, buffers),
        Some("edit-objects") => edit::object::edit_objects(session, request),
        Some("clip-copy") => edit::clip::copy(engine, session, request),
        Some("clip-paste") => edit::clip::paste(engine, session, request),
        Some("edit-batch") => edit::batch::edit_batch(engine, session, request),
        Some("redact-add") => redact::add(session, request),
        Some("redact-set") => redact::set(session, request),
        Some("redact-search") => redact::search(session, request),
        Some("redact-preview") => redact::preview(session, request),
        _ => Err(AppError::plain(format!("unknown op: {}", js_string(op)))),
    }
}
