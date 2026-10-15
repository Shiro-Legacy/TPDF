// SPDX-License-Identifier: AGPL-3.0-or-later
//! Redaction marks: redact-add, redact-set, redact-search and
//! redact-preview (engine.ts with redact.ts), each write one journal step
//! (`add redaction mark`, `move redaction mark`, `mark redaction search
//! hits`). The marks are Redact annotations (`app::redact::marks`); what
//! they cover is `app::redact::region`.
use serde_json::{Map, Value, json};

use crate::app::annots::{self, PageSpace};
use crate::app::ops::annots::find_annotation;
use crate::app::redact::content::Limits;
use crate::app::redact::inventory;
use crate::app::redact::marks::{
    self, MAX_REDACT_MARKS, MAX_REDACT_TEXT, Plan, SearchError, redact_rect,
};
use crate::app::redact::region::{self, SharedError};
use crate::app::{AppError, AppSession, reply};

fn page_bounds(session: &AppSession, page: usize, op: &str) -> Result<[f64; 4], AppError> {
    let app = session
        .doc
        .as_ref()
        .ok_or_else(|| AppError::reason("no-document", format!("{op}: no document open")))?;
    let (_, dict) = app
        .page(page)
        .ok_or_else(|| AppError::plain(format!("{op}: no such page")))?;
    Ok(PageSpace::of(app.document(), dict).bounds())
}

fn with_journal(session: &AppSession, fields: Value) -> Value {
    let mut out = Map::new();
    if let Value::Object(fields) = fields {
        out.extend(fields);
    }
    out.extend(session.journal_state().json());
    Value::Object(out)
}

fn too_many(op: &str) -> AppError {
    AppError::reason(
        "too-many",
        format!("{op}: at most {MAX_REDACT_MARKS} marks"),
    )
}

/// engine.ts `redact-add`: an area mark, its rect clamped to the page.
pub fn add(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    const OP: &str = "redact-add";
    let app = session.require_mutable(OP)?;
    let page = session.check_page(request.get("page"), OP)?;
    let rect = redact_rect(page_bounds(session, page, OP)?, request.get("rect"), OP)?;
    if marks::count(app.document()) >= MAX_REDACT_MARKS {
        return Err(too_many(OP));
    }
    let plan = Plan {
        page,
        rect,
        quads: Vec::new(),
        search: None,
    };
    session.mutate("add redaction mark", |session| {
        let mut tx = session.tx()?;
        marks::write_plans(&mut tx, &[plan], OP).map(|_| ())
    })?;
    Ok(reply(
        request,
        with_journal(session, json!({"added": 1, "pages": [page]})),
    ))
}

/// engine.ts `redact-set`: a mark's new rect, refusals tagged with what
/// was wrong; the same rect (within 0.01) is a no-op. A moved search mark
/// becomes an area mark.
pub fn set(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    const OP: &str = "redact-set";
    let app = session.require_mutable(OP)?;
    let page = session.check_page(request.get("page"), OP)?;
    let rect = redact_rect(page_bounds(session, page, OP)?, request.get("rect"), OP)
        .map_err(|error| AppError::detail("invalid", "rect", error.message))?;
    let id = find_annotation(session, page, request.get("annotId"), OP)
        .map_err(|error| AppError::detail("invalid", "no-such-annot", error.message))?;
    let doc = app.document();
    let (_, page_dict) = app
        .page(page)
        .ok_or_else(|| AppError::plain(format!("{OP}: no such page")))?;
    let annot = doc.resolve_id(id).as_dict();
    let subtype = annot.and_then(|a| doc.dict_get(a, b"Subtype").as_name());
    let Some(annot) = annot.filter(|_| subtype == Some(b"Redact")) else {
        return Err(AppError::detail(
            "invalid",
            "not-redact",
            format!("{OP}: only a redaction mark can be moved"),
        ));
    };
    let space = PageSpace::of(doc, page_dict);
    let current = annots::page_rect(doc, &space, annot);
    if current.is_some_and(|c| c.iter().zip(rect).all(|(a, b)| (b - a).abs() <= 0.01)) {
        return Ok(reply(request, with_journal(session, json!({"noop": true}))));
    }
    session.mutate("move redaction mark", |session| {
        let mut tx = session.tx()?;
        marks::set_rect(&mut tx, page, id, rect)
    })?;
    Ok(reply(request, with_journal(session, json!({}))))
}

/// engine.ts `redact-search`: a mark over every hit of `text` on every
/// page, its quads inflated by two points, tagged with the text.
pub fn search(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    const OP: &str = "redact-search";
    let app = session.require_mutable(OP)?;
    let text = request
        .get("text")
        .and_then(Value::as_str)
        .filter(|text| (1..=MAX_REDACT_TEXT).contains(&text.encode_utf16().count()))
        .filter(|text| !text.chars().any(|c| c <= '\u{1f}' || c == '\u{7f}'))
        .ok_or_else(|| {
            AppError::reason(
                "invalid",
                format!("{OP}: text must be 1..{MAX_REDACT_TEXT} characters with no controls"),
            )
        })?
        .to_string();
    let existing = marks::count(app.document());
    let room = MAX_REDACT_MARKS.saturating_sub(existing);
    let Some(app) = session.doc.as_mut() else {
        return Err(AppError::reason(
            "no-document",
            format!("{OP}: no document open"),
        ));
    };
    // Without optional content the cached view is the same drawing.
    let doc = app.document();
    let layered = (doc.resolve_id(doc.root()).as_dict())
        .is_some_and(|catalog| catalog.contains_key(b"OCProperties".as_slice()));
    let failed = |error: crate::view::ViewError| AppError::plain(format!("{OP}: {error}"));
    let found = if layered {
        let view = app.view_unlayered().map_err(failed)?;
        marks::search_plans(app.document(), &view, &text, room)
    } else {
        let (view, doc) = app.view_with_document().map_err(failed)?;
        marks::search_plans(doc, view, &text, room)
    };
    let plans = match found {
        Ok(plans) => plans,
        Err(SearchError::TooMany) => return Err(too_many(OP)),
        Err(SearchError::Unsearchable(page)) => {
            return Err(AppError::reason(
                "scan-budget",
                format!(
                    "{OP}: the text of page {} is too complex to search; mark it by area",
                    page + 1
                ),
            ));
        }
    };
    if plans.is_empty() {
        return Err(AppError::reason("no-match", format!("{OP}: no match")));
    }
    if existing + plans.len() > MAX_REDACT_MARKS {
        return Err(too_many(OP));
    }
    let mut pages: Vec<usize> = Vec::new();
    for plan in &plans {
        if !pages.contains(&plan.page) {
            pages.push(plan.page);
        }
    }
    let added = plans.len();
    session.mutate("mark redaction search hits", |session| {
        let mut tx = session.tx()?;
        marks::write_plans(&mut tx, &plans, OP).map(|_| ())
    })?;
    Ok(reply(
        request,
        with_journal(session, json!({"added": added, "pages": pages})),
    ))
}

/// engine.ts `redact-preview`: the marks per page, marked graphics drawn
/// elsewhere without a mark (`shared`), the sanitize inventory and the
/// inline alternate texts on marked pages. No journal state.
pub fn preview(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    const OP: &str = "redact-preview";
    if session.doc.is_none() {
        return Err(AppError::reason(
            "no-document",
            format!("{OP}: no document open"),
        ));
    }
    if session.locked {
        return Err(AppError::reason(
            "read-only",
            format!("{OP}: this document is locked"),
        ));
    }
    let Some(app) = session.doc.as_mut() else {
        return Err(AppError::reason(
            "no-document",
            format!("{OP}: no document open"),
        ));
    };
    let all = marks::marks(app.document());
    let mut counts: Vec<(usize, usize)> = Vec::new();
    for mark in &all {
        match counts.iter_mut().find(|(page, _)| *page == mark.page) {
            Some((_, n)) => *n += 1,
            None => counts.push((mark.page, 1)),
        }
    }
    counts.sort_unstable();
    let shared = if all.is_empty() {
        Ok((Vec::new(), 0))
    } else {
        match app.view_with_document() {
            Ok((view, doc)) => region::shared(doc, Some(view), &all, Limits::DEFAULT),
            Err(_) => region::shared(app.document(), None, &all, Limits::DEFAULT),
        }
    };
    let (shared, inline_alt) = match shared {
        Ok(found) => found,
        Err(SharedError::TooComplex(_)) => {
            return Err(AppError::reason(
                "scan-budget",
                format!("{OP}: page content is too complex to check for shared graphics"),
            ));
        }
        Err(SharedError::Unreadable(page, why)) => {
            return Err(AppError::reason(
                "invalid",
                format!(
                    "{OP}: the content of page {} cannot be read ({why})",
                    page + 1
                ),
            ));
        }
    };
    let sanitize = inventory::inventory(app.document(), OP)?;
    let shared: Vec<Value> = (shared.iter())
        .map(|s| json!({"page": s.page, "kind": s.kind, "otherPages": [s.other]}))
        .collect();
    let page_counts: Vec<Value> = (counts.iter())
        .map(|(page, marks)| json!({"page": page, "marks": marks}))
        .collect();
    let pages: Vec<usize> = counts.iter().map(|(page, _)| *page).collect();
    Ok(reply(
        request,
        json!({
            "marks": all.len(),
            "pages": pages,
            "pageCounts": page_counts,
            "shared": shared,
            "sanitize": sanitize.json(),
            "inlineAlt": inline_alt,
        }),
    ))
}
