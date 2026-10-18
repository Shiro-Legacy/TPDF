// SPDX-License-Identifier: MIT
//! engine.ts `edit-batch`: 2..512 writes of mixed kinds on one page, all
//! or nothing, as one journal step.
//!
//! Run moves and placement moves or deletes are planned against the one
//! scan and written once per /Contents element. Every other item is its
//! own op dispatched through the router: first with the session in batch
//! `Plan` (every check, no write), then, inside the batch's step, in batch
//! `Run`, where its `mutate` joins the step. A refusal anywhere comes
//! before the step opens, so the journal and the redo branch are untouched.
use std::collections::BTreeSet;

use serde_json::{Value, json};

use super::object::{placement_spans, plan_placement, session_clip_box};
use super::plan::{self, Geometry, TextPlan};
use super::scan::{Kind, PageScan};
use super::write::{self, SpanEdit, Splice};
use super::{MAX_EDIT_TARGETS, edit_error, edit_state_reply, resolve_target, stale, with_measure};
use crate::app::request::{self, js_string};
use crate::app::{AppEngine, AppError, AppSession, Batch, Buffers};

/// engine.ts `ALLOW`: the keys each item may carry besides `op`.
const ALLOW: [(&str, &[&str]); 8] = [
    ("edit-text", &["targetId", "move"]),
    ("edit-object", &["targetId", "action", "box"]),
    ("field-props", &["widgetId", "rect"]),
    ("field-delete", &["widgetId"]),
    ("link-set", &["annotId", "rect"]),
    ("redact-set", &["annotId", "rect"]),
    ("link-delete", &["annotId"]),
    ("annot-delete", &["annotId"]),
];

fn allowed(op: &str) -> &'static [&'static str] {
    ALLOW
        .iter()
        .find(|(name, _)| *name == op)
        .map_or(&[], |(_, keys)| keys)
}

fn op_of(item: &Value) -> &str {
    item.get("op").and_then(Value::as_str).unwrap_or("")
}

/// engine.ts `keyOf`: the key naming an item's target.
fn key_of(op: &str) -> &'static str {
    match op {
        "edit-text" | "edit-object" => "targetId",
        "field-props" | "field-delete" => "widgetId",
        _ => "annotId",
    }
}

/// engine.ts `idOf`: `{index, <key>: <id>}`.
fn id_of(item: &Value, index: usize) -> Value {
    let key = key_of(op_of(item));
    let mut id = json!({"index": index});
    id[key] = item.get(key).cloned().unwrap_or(Value::Null);
    id
}

/// engine.ts `shaped`.
fn shaped(item: &Value) -> bool {
    let Some(fields) = item.as_object() else {
        return false;
    };
    let target = item
        .get("targetId")
        .and_then(Value::as_str)
        .is_some_and(|id| !id.is_empty());
    let int = |key: &str| item.get(key).and_then(request::as_int).is_some();
    match op_of(item) {
        "edit-text" => {
            target
                && !fields.contains_key("text")
                && !fields.contains_key("size")
                && item
                    .get("move")
                    .is_some_and(|m| m.is_object() || m.is_array())
        }
        "edit-object" => {
            target
                && matches!(
                    item.get("action").and_then(Value::as_str),
                    Some("move" | "delete")
                )
        }
        "field-props" => int("widgetId") && fields.contains_key("rect"),
        "field-delete" => int("widgetId"),
        "link-set" | "redact-set" => int("annotId") && fields.contains_key("rect"),
        "link-delete" | "annot-delete" => int("annotId"),
        _ => false,
    }
}

/// engine.ts `tagMember(idOf(item, index), run, "stale-target")`: an
/// untagged refusal gets a detail (its reason, else `stale-target`), and
/// every refusal the item's position.
fn member(error: AppError, item: &Value, index: usize) -> AppError {
    let mut error = error;
    if error.detail.is_none() {
        error.detail = Some(match error.reason.as_deref() {
            Some(reason) if reason != "invalid" => reason.to_string(),
            _ => "stale-target".into(),
        });
        if error.reason.is_none() {
            error.reason = Some("invalid".into());
        }
    }
    error.with_extra(id_of(item, index))
}

/// An item that runs as its own op.
fn dispatched(op: &str) -> bool {
    !matches!(op, "edit-text" | "edit-object")
}

/// One item as its single op (engine.ts `runAnnotation`): only its allowed
/// keys travel. The nested reply's extra fields are dropped.
fn run_item(
    engine: &mut AppEngine,
    session: &mut AppSession,
    request: &Value,
    page: usize,
    item: &Value,
) -> Result<Value, AppError> {
    let op = op_of(item);
    let mut single = json!({"id": request.get("id"), "page": page, "op": op});
    for key in allowed(op) {
        if let Some(value) = item.get(*key) {
            single[*key] = value.clone();
        }
    }
    crate::app::ops::route(engine, session, &single, &mut Buffers::default()).map_err(|error| {
        AppError {
            message: error.message,
            reason: error.reason,
            detail: error.detail,
            extra: Default::default(),
        }
    })
}

/// The page's links as the `links` listing answers them (empty when it
/// cannot list them).
fn links(engine: &mut AppEngine, session: &mut AppSession, page: usize) -> Vec<Value> {
    let listing = json!({"op": "links", "page": page});
    crate::app::ops::route(engine, session, &listing, &mut Buffers::default())
        .ok()
        .and_then(|reply| reply.get("items")?.as_array().cloned())
        .unwrap_or_default()
}

/// A batch `link-delete` names the link by its /Annots object number; the
/// `links` listing pairs it with the `linkId` that `link-delete` takes.
fn link_id(rows: &[Value], annot: Option<&Value>) -> Option<i64> {
    let annot = annot.and_then(request::as_int).filter(|&a| a > 0)?;
    let row = rows
        .iter()
        .find(|row| row.get("annotId").and_then(request::as_int) == Some(annot))?;
    row.get("id").and_then(request::as_int)
}

fn no_link(page: usize, annot: Option<&Value>) -> AppError {
    edit_error(
        "stale-target",
        format!("edit-batch: page {page} has no link {}", js_string(annot)),
    )
}

/// Runs `work` with the session's batch mode set to `mode`.
fn in_batch<T>(
    session: &mut AppSession,
    mode: Batch,
    work: impl FnOnce(&mut AppSession) -> T,
) -> T {
    let previous = std::mem::replace(&mut session.batch, mode);
    let out = work(session);
    session.batch = previous;
    out
}

/// One element's planned text moves and placement spans.
#[derive(Default)]
struct Group {
    plans: Vec<TextPlan>,
    spans: Vec<SpanEdit>,
}

/// engine.ts `edit-batch`.
pub fn edit_batch(
    engine: &mut AppEngine,
    session: &mut AppSession,
    request: &Value,
) -> Result<Value, AppError> {
    let op = "edit-batch";
    session.require_mutable(op)?;
    let page = session.check_page(request.get("page"), op)?;
    let raw = request.get("items").and_then(Value::as_array);
    if raw.is_some_and(|items| items.len() > MAX_EDIT_TARGETS) {
        return Err(edit_error(
            "too-large",
            format!("edit-batch: more than {MAX_EDIT_TARGETS} items"),
        ));
    }
    let Some(list) = raw.filter(|items| items.len() >= 2 && items.iter().all(shaped)) else {
        return Err(edit_error(
            "invalid-selection",
            "edit-batch: 2 or more items, each one edit-text {targetId, move}, edit-object {targetId, move|delete}, field-props, field-delete, link-set, link-delete, redact-set or annot-delete",
        ));
    };
    for (index, item) in list.iter().enumerate() {
        let op = op_of(item);
        let keys = allowed(op);
        let stray = item.as_object().and_then(|fields| {
            fields
                .keys()
                .find(|key| *key != "op" && !keys.contains(&key.as_str()))
        });
        if let Some(key) = stray {
            return Err(edit_error(
                "invalid-selection",
                format!("edit-batch: unexpected key {key} on {op}"),
            )
            .with_extra(id_of(item, index)));
        }
    }
    let mut seen = BTreeSet::new();
    for (index, item) in list.iter().enumerate() {
        let key = key_of(op_of(item));
        if !seen.insert(format!("{key}:{}", js_string(item.get(key)))) {
            return Err(edit_error(
                "invalid-selection",
                format!("edit-batch: item {index} repeats an id"),
            )
            .with_extra(id_of(item, index)));
        }
    }
    super::require_each(session, list.iter().map(op_of)).map_err(|(index, error)| {
        match list.get(index) {
            Some(item) => member(error, item, index),
            None => error,
        }
    })?;
    let clip = session_clip_box(session, page);
    let mut groups: Vec<(usize, Group)> = Vec::new();
    let mut scan: Option<std::sync::Arc<PageScan>> = None;
    for (index, item) in list.iter().enumerate() {
        let op = op_of(item);
        if dispatched(op) {
            continue;
        }
        let planned = plan_content(session, request, page, item, &clip)
            .map_err(|error| member(error, item, index))?;
        let (found, element, write) = planned;
        scan.get_or_insert(found);
        let at = match groups.iter().position(|(e, _)| *e == element) {
            Some(at) => at,
            None => {
                groups.push((element, Group::default()));
                groups.len().saturating_sub(1)
            }
        };
        let Some((_, group)) = groups.get_mut(at) else {
            continue;
        };
        match write {
            Content::Text(plan) => group.plans.push(*plan),
            Content::Spans(record, spans) => group
                .spans
                .extend(spans.into_iter().map(|splice| SpanEdit { record, splice })),
            Content::Nothing => {}
        }
    }
    let mut listed: Option<Vec<Value>> = None;
    for (index, item) in list.iter().enumerate() {
        if op_of(item) != "link-delete" {
            continue;
        }
        let rows = listed.get_or_insert_with(|| links(engine, session, page));
        if link_id(rows, item.get("annotId")).is_none() {
            return Err(member(no_link(page, item.get("annotId")), item, index));
        }
    }
    in_batch(session, Batch::Plan, |session| {
        for (index, item) in list.iter().enumerate() {
            let op = op_of(item);
            if dispatched(op) && op != "link-delete" {
                run_item(engine, session, request, page, item)
                    .map_err(|error| member(error, item, index))?;
            }
        }
        Ok::<(), AppError>(())
    })?;
    groups.retain(|(_, group)| !group.plans.is_empty() || !group.spans.is_empty());
    let annotations = list.iter().any(|item| dispatched(op_of(item)));
    if groups.is_empty() && !annotations {
        return Ok(edit_state_reply(
            session,
            request,
            json!({"page": page, "count": 0, "noop": true}),
        ));
    }
    groups.sort_by_key(|(element, _)| *element);
    session.mutate("edit batch", |session| {
        if let Some(scan) = &scan {
            let mut tx = session.tx()?;
            let page_id = write::page_object(tx.document(), page, op)?;
            let mut privatised = BTreeSet::new();
            let mut copied = 0;
            for (element, group) in &groups {
                let mut plans: Vec<&TextPlan> = group.plans.iter().collect();
                plans.sort_by_key(|p| {
                    std::cmp::Reverse(scan.records.get(p.record).map_or(0, |r| r.op.start))
                });
                write::write_element(
                    &mut tx,
                    scan,
                    page_id,
                    *element,
                    &plans,
                    &group.spans,
                    &mut privatised,
                    &mut copied,
                )
                .map_err(write::writer_error)?;
            }
        }
        in_batch(session, Batch::Run, |session| {
            for (index, item) in list.iter().enumerate() {
                let op = op_of(item);
                if !dispatched(op) {
                    continue;
                }
                if op != "link-delete" {
                    run_item(engine, session, request, page, item)
                        .map_err(|error| member(error, item, index))?;
                    continue;
                }
                // Each deletion renumbers the links after it: list again.
                let annot = item.get("annotId");
                let Some(link) = link_id(&links(engine, session, page), annot) else {
                    return Err(member(no_link(page, annot), item, index));
                };
                let single = json!({"id": request.get("id"), "op": "link-delete", "page": page,
                    "linkId": link});
                crate::app::ops::route(engine, session, &single, &mut Buffers::default())
                    .map_err(|error| member(error, item, index))?;
            }
            Ok(())
        })
    })?;
    Ok(edit_state_reply(
        session,
        request,
        json!({"page": page, "count": list.len()}),
    ))
}

/// What one content item writes.
enum Content {
    Text(Box<TextPlan>),
    Spans(usize, Vec<Splice>),
    Nothing,
}

/// An `edit-text` move or an `edit-object` move or delete, planned
/// against the scan the request names.
fn plan_content(
    session: &mut AppSession,
    request: &Value,
    page: usize,
    item: &Value,
    clip: &[f64; 4],
) -> Result<(std::sync::Arc<PageScan>, usize, Content), AppError> {
    let op = op_of(item);
    let id = item.get("targetId").and_then(Value::as_str).unwrap_or("");
    let single = json!({
        "generation": request.get("generation"),
        "revision": request.get("revision"),
        "page": page,
        "targetId": id,
    });
    let (scan, index) =
        resolve_target(session, &single, page).map_err(|error| stale("edit-batch", error))?;
    let Some(record) = scan.records.get(index) else {
        return Err(edit_error("stale-target", "edit-batch: the target is gone"));
    };
    let because = record.reason.map(|r| format!(" ({r})")).unwrap_or_default();
    let with_reason = |error: AppError| match record.reason {
        Some(reason) => error.with_extra(json!({"reason": reason})),
        None => error,
    };
    if op == "edit-text" {
        let text = record
            .text
            .clone()
            .filter(|_| record.kind == Kind::Text && record.candidate);
        let Some(text) = text else {
            return Err(with_reason(edit_error(
                "not-candidate",
                format!("edit-batch: {id} cannot be edited{because}"),
            )));
        };
        // A run inside a Form privatises its Form, which would split the
        // element's one splice; it moves on its own.
        if record.depth > 0 {
            return Err(edit_error(
                "not-candidate",
                format!("edit-batch: {id} is inside a Form; move it on its own"),
            ));
        }
        let geometry = Geometry {
            moved: item.get("move"),
            size: None,
        };
        let plan = with_measure(session, page, |measure, _| {
            plan::plan_text(measure, index, record, &text, geometry)
        })?;
        let element = plan.element;
        let write = if !plan.prefix.is_empty() || !plan.suffix.is_empty() || plan.rewrite {
            Content::Text(Box::new(plan))
        } else {
            Content::Nothing
        };
        return Ok((scan.clone(), element, write));
    }
    if record.kind == Kind::Text || !record.candidate {
        return Err(with_reason(edit_error(
            "not-candidate",
            format!("edit-batch: {id} cannot be edited{because}"),
        )));
    }
    let action = item.get("action").and_then(Value::as_str).unwrap_or("");
    let offered = record
        .object_ops
        .as_ref()
        .is_some_and(|ops| ops.contains(&action));
    if !offered {
        return Err(with_reason(edit_error(
            record.reason.unwrap_or("not-candidate"),
            // engine.ts names no reason here (edit-object does).
            format!("edit-batch: {action} is not available for {id}"),
        )));
    }
    let (Some(placement), Some(element)) = (&record.placement, record.element) else {
        return Err(edit_error(
            "not-candidate",
            format!("edit-batch: only top-level simple placements are editable ({id})"),
        ));
    };
    if action == "delete" {
        let span = Splice {
            start: placement.q_start,
            end: placement.q_end,
            replacement: Vec::new(),
        };
        return Ok((scan.clone(), element, Content::Spans(index, vec![span])));
    }
    let write = plan_placement(placement, "move", item.get("box"), None, clip)?;
    let content = match write.noop {
        true => Content::Nothing,
        false => Content::Spans(
            index,
            placement_spans(placement, write.cm.as_deref(), write.clip_text.as_deref()),
        ),
    };
    Ok((scan.clone(), element, content))
}
