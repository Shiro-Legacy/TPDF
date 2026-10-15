// SPDX-License-Identifier: AGPL-3.0-or-later
//! Content editing (engine.ts, content-edit.ts): edit-targets, edit-text,
//! edit-paragraph, text-add, edit-replace-preview and edit-replace-apply.
//!
//! `scan` discovers targets: one record per text show and per image or
//! Form invocation, each naming byte spans of a decoded content stream.
//! A target id is never trusted: every edit re-scans (through the session's
//! one-entry cache, keyed on generation, revision, page and content state)
//! and finds the id again or refuses. `plan` checks a write and builds its
//! bytes before any operation opens; `write` privatises shared content and
//! splices it inside the journal operation, so undo restores it exactly.
//!
//! The step names (`edit text`, `move text`, `edit paragraph`, `add text`,
//! `replace text`) are part of the wire contract.
pub mod font;
pub mod geometry;
pub mod glyph;
pub mod lex;
pub mod paragraph;
pub mod plan;
pub mod scan;
mod walk;
pub mod write;

use std::sync::Arc;

use serde_json::{Map, Value, json};

use self::geometry::round2;
use self::plan::{Geometry, Measure, TextPlan};
use self::scan::{Kind, PageInput, PageScan, scan_page};
use crate::app::request::{self, js_number, js_string};
use crate::app::{AppError, AppSession, info, reply};
use crate::document::Document;
use crate::object::ObjectId;
use crate::stream::{StreamErrorKind, StreamLimits};
use crate::stream_object::StreamObjectErrorKind;
use crate::view::ViewDocument;

/// The scan's cost model (content-edit.ts): decoded bytes (streams,
/// ToUnicode CMaps and copy-on-write copies), streams, Form depth, nesting,
/// tokens, raw and final targets, text per target and per reply, operands.
pub const MAX_EDIT_SCAN_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_EDIT_TRAVERSALS: usize = 128;
pub const MAX_EDIT_FORM_DEPTH: usize = 16;
pub const MAX_EDIT_NESTING: usize = 32;
pub const MAX_EDIT_TOKENS: usize = 200_000;
pub const MAX_EDIT_RAW_TARGETS: usize = 4096;
pub const MAX_EDIT_TARGETS: usize = 512;
pub const MAX_EDIT_TEXT_CHARS: usize = 4096;
pub const MAX_EDIT_REPLY_TEXT_CHARS: usize = 65_536;
pub const MAX_EDIT_OPERANDS: usize = 256;
/// engine.ts `MAX_REPLACE_FIND_CHARS`.
pub const MAX_REPLACE_FIND_CHARS: usize = 256;
/// limits.ts `MAX_TEXT_ADD_CHARS` and `MAX_TEXT_ADD_LINES`.
pub const MAX_TEXT_ADD_CHARS: usize = 4096;
pub const MAX_TEXT_ADD_LINES: usize = 200;
/// engine-core.ts `BASE14_FONTS`: the twelve WinAnsi standard fonts.
pub const BASE14_FONTS: [&str; 12] = [
    "Helvetica",
    "Helvetica-Bold",
    "Helvetica-Oblique",
    "Helvetica-BoldOblique",
    "Times-Roman",
    "Times-Bold",
    "Times-Italic",
    "Times-BoldItalic",
    "Courier",
    "Courier-Bold",
    "Courier-Oblique",
    "Courier-BoldOblique",
];

/// One scan's spending.
#[derive(Clone, Copy, Debug, Default)]
pub struct Budget {
    pub bytes: usize,
    pub traversals: usize,
    pub tokens: usize,
    pub reply_text: usize,
}

/// engine-core.ts `failReason`.
pub fn scan_error(reason: &str, message: impl Into<String>) -> AppError {
    AppError::reason(reason, message)
}

/// engine-core.ts `failEditText` / `failEditObject` / `failEditReplace`.
pub fn edit_error(detail: &str, message: impl Into<String>) -> AppError {
    AppError::detail("invalid", detail, message)
}

/// A JavaScript string's `length`: UTF-16 code units.
pub fn js_len(text: &str) -> usize {
    text.encode_utf16().count()
}

pub enum DecodeError {
    TooLarge,
    Invalid(String),
}

/// A stream's decoded bytes, at most `room` of them.
pub fn decode(doc: &Document<'_>, id: ObjectId, room: usize) -> Result<Vec<u8>, DecodeError> {
    let limit = room.min(StreamLimits::default().max_decoded_bytes);
    let limits = StreamLimits {
        max_decoded_bytes: limit,
        max_total_decoded_bytes: limit.saturating_mul(4).max(limit),
        ..StreamLimits::default()
    };
    doc.decoded_stream(id, limits).map_err(|error| {
        let over = matches!(
            &error.kind,
            StreamObjectErrorKind::Decode(e)
                if matches!(e.kind, StreamErrorKind::DecodedOutputLimit { .. }
                    | StreamErrorKind::TotalOutputLimit { .. })
        );
        if over && room < StreamLimits::default().max_decoded_bytes {
            DecodeError::TooLarge
        } else {
            DecodeError::Invalid(error.to_string())
        }
    })
}

/// What a scan was taken of: generation, revision, page, content state.
type ScanKey = (u64, u64, usize, u64);

/// engine.ts `editScanCache`: the last scan and what it was taken of.
#[derive(Clone, Debug, Default)]
pub struct ScanCache {
    entry: Option<(ScanKey, Arc<PageScan>)>,
}

/// Scans `page` of the session's document with its view.
fn fresh_scan(session: &mut AppSession, page: usize) -> Result<Arc<PageScan>, AppError> {
    let doc = session
        .doc
        .as_mut()
        .ok_or_else(|| AppError::reason("no-document", "edit-targets: no document open"))?;
    let (view, document) = doc
        .view_with_document()
        .map_err(|error| scan_error("invalid", format!("edit-targets: {error}")))?;
    let transform = view
        .page_transform(page)
        .map_err(|error| scan_error("invalid", format!("edit-targets: {error}")))?;
    let size = view
        .page_size(page)
        .map_err(|error| scan_error("invalid", format!("edit-targets: {error}")))?;
    let pages = info::pages(document);
    let dict = pages
        .get(page)
        .ok_or_else(|| scan_error("invalid", "edit-targets: no such page"))?
        .dict;
    let input = PageInput {
        index: page,
        dict,
        transform,
        size,
    };
    scan_page(document, Some(view), &input).map(Arc::new)
}

fn cache_key(session: &AppSession, page: usize) -> ScanKey {
    let state = session.doc.as_ref().map_or(0, |d| d.content_state());
    (session.generation, session.revision, page, state)
}

/// engine.ts `editScanFor`: the cached scan when it still matches.
pub fn scan_for(session: &mut AppSession, page: usize) -> Result<Arc<PageScan>, AppError> {
    let key = cache_key(session, page);
    let cached = session
        .doc
        .as_mut()
        .and_then(|doc| match &doc.edit_scan().entry {
            Some((cached, scan)) if *cached == key => Some(scan.clone()),
            _ => None,
        });
    if let Some(scan) = cached {
        return Ok(scan);
    }
    let scan = fresh_scan(session, page)?;
    if let Some(doc) = session.doc.as_mut() {
        doc.edit_scan().entry = Some((key, scan.clone()));
    }
    Ok(scan)
}

/// Drops the cached scan, so the next one is fresh (`edit-targets`).
fn forget_scan(session: &mut AppSession) {
    if let Some(doc) = session.doc.as_mut() {
        doc.edit_scan().entry = None;
    }
}

/// The view and page bounds plans measure with.
fn with_measure<T>(
    session: &mut AppSession,
    page: usize,
    work: impl FnOnce(&Measure<'_>, Option<&ViewDocument>) -> Result<T, AppError>,
) -> Result<T, AppError> {
    let doc = session
        .doc
        .as_mut()
        .ok_or_else(|| AppError::reason("no-document", "no document open"))?;
    let view: Option<&ViewDocument> = doc.view().ok();
    let bounds = view
        .and_then(|v| v.page_size(page).ok())
        .map_or([0.0, 0.0, 612.0, 792.0], |(w, h)| [0.0, 0.0, w, h]);
    let measurer = view.and_then(|v| v.measurer(page).ok());
    work(&Measure { measurer, bounds }, view)
}

/// engine.ts `resolveEditTarget`: the record a request names, re-found in
/// a fresh (or cached) scan of this exact revision.
fn resolve_target(
    session: &mut AppSession,
    request: &Value,
    page: usize,
) -> Result<(Arc<PageScan>, usize), AppError> {
    if session.doc.is_none() {
        return Err(AppError::reason(
            "no-document",
            "resolveEditTarget: no document open",
        ));
    }
    if session.locked {
        return Err(AppError::reason(
            "read-only",
            "resolveEditTarget: this document is read-only until it is unlocked",
        ));
    }
    let same = |key: &str, value: u64| {
        request
            .get(key)
            .and_then(request::as_js_number)
            .is_some_and(|n| n == value as f64)
    };
    if !same("generation", session.generation) {
        return Err(AppError::reason(
            "invalid",
            "resolveEditTarget: stale document generation",
        ));
    }
    if !same("revision", session.revision) {
        return Err(AppError::reason(
            "invalid",
            "resolveEditTarget: stale revision, the document changed",
        ));
    }
    session.check_page(request.get("page"), "resolveEditTarget")?;
    let id = request
        .get("targetId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| {
            AppError::reason(
                "invalid",
                "resolveEditTarget: targetId must be a non-empty string",
            )
        })?;
    let scan = scan_for(session, page)?;
    let index = scan.index(id).ok_or_else(|| {
        AppError::reason(
            "invalid",
            format!("resolveEditTarget: no target {id} on page {page}"),
        )
    })?;
    Ok((scan, index))
}

/// The open, unlocked document, refused with engine.ts's plain messages.
fn readable(session: &AppSession, op: &str) -> Result<(), AppError> {
    if session.doc.is_none() {
        return Err(AppError::plain(format!("{op}: no document open")));
    }
    if session.locked {
        return Err(AppError::plain(format!(
            "{op}: document is encrypted, a password is required"
        )));
    }
    Ok(())
}

fn copy_permitted(session: &AppSession, op: &str) -> Result<(), AppError> {
    let allowed = session
        .doc
        .as_ref()
        .is_some_and(|doc| info::permissions(doc.document()).copy);
    if allowed {
        Ok(())
    } else {
        Err(AppError::reason(
            "read-only",
            format!("{op}: this document does not permit copying"),
        ))
    }
}

/// engine.ts `edit-targets`: read-only; the copy bit decides.
pub fn targets(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    readable(session, "edit-targets")?;
    let page = session.check_page(request.get("page"), "edit-targets")?;
    copy_permitted(session, "edit-targets")?;
    forget_scan(session);
    let scan = scan_for(session, page)?;
    let paragraphs: Vec<Value> = scan.paragraphs.iter().map(|p| p.item()).collect();
    Ok(reply(
        request,
        json!({
            "page": page,
            "generation": session.generation,
            "revision": session.revision,
            "items": scan.items(),
            "paragraphs": paragraphs,
            "transform": scan.transform,
        }),
    ))
}

fn edit_state_reply(session: &AppSession, request: &Value, fields: Value) -> Value {
    let mut out = Map::new();
    if let Value::Object(fields) = fields {
        out.extend(fields);
    }
    out.extend(session.edit_state());
    reply(request, Value::Object(out))
}

/// A remapped `resolveEditTarget` refusal: `too-large` stays, all else is
/// `stale-target`.
fn stale(op: &str, error: AppError) -> AppError {
    let detail = if error.reason.as_deref() == Some("too-large") {
        "too-large"
    } else {
        "stale-target"
    };
    edit_error(detail, format!("{op}: {}", error.message))
}

/// engine.ts `edit-text`: replace, move or resize one run.
pub fn text(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    session.require_mutable("edit-text")?;
    let page = session.check_page(request.get("page"), "edit-text")?;
    let (scan, index) =
        resolve_target(session, request, page).map_err(|error| stale("edit-text", error))?;
    let record = &scan.records[index];
    if record.kind != Kind::Text || !record.candidate {
        let reason = record.reason.map(|r| format!(" ({r})")).unwrap_or_default();
        let mut error = edit_error(
            "not-candidate",
            format!("edit-text: that target cannot be edited{reason}"),
        );
        if let Some(reason) = record.reason {
            error = error.with_extra(json!({"reason": reason}));
        }
        return Err(error);
    }
    let geometry = Geometry {
        moved: request.get("move"),
        size: request.get("size"),
    };
    let moved = geometry.moved.is_some() || geometry.size.is_some();
    let next = match request.get("text") {
        None if moved => record.text.clone(),
        value => value.and_then(Value::as_str).map(str::to_string),
    };
    let Some(next) = next else {
        return Err(edit_error(
            "unsupported-encoding",
            "edit-text: text must be a string",
        ));
    };
    let plan = with_measure(session, page, |measure, _| {
        plan::plan_text(measure, index, record, &next, geometry)
    })?;
    let same = if record.glyph_line.is_some() {
        &plan.next
    } else {
        &next
    };
    if Some(same.as_str()) == record.text.as_deref()
        && plan.prefix.is_empty()
        && plan.suffix.is_empty()
        && !plan.rewrite
    {
        return Ok(edit_state_reply(
            session,
            request,
            json!({"page": page, "shift": 0}),
        ));
    }
    let step = if moved { "move text" } else { "edit text" };
    session.mutate(step, |session| {
        let mut tx = session.tx()?;
        let page_id = write::page_object(tx.document(), page, "edit-text")?;
        write::write_plans(
            &mut tx,
            &scan,
            page_id,
            std::slice::from_ref(&plan),
            |error, _| write::writer_error(error),
        )
    })?;
    let mut fields = json!({"page": page, "shift": plan.shift});
    if record.kerned {
        fields["notice"] = "kerning in this run was dropped".into();
    }
    Ok(edit_state_reply(session, request, fields))
}

/// engine.ts `edit-paragraph`: rewrap one paragraph.
pub fn paragraph(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    session.require_mutable("edit-paragraph")?;
    let page = session.check_page(request.get("page"), "edit-paragraph")?;
    let same = |key: &str, value: u64| {
        request
            .get(key)
            .and_then(request::as_js_number)
            .is_some_and(|n| n == value as f64)
    };
    if !same("generation", session.generation) || !same("revision", session.revision) {
        return Err(edit_error(
            "stale-target",
            "edit-paragraph: the document changed since the scan",
        ));
    }
    let scan = scan_for(session, page)?;
    let id = request.get("paragraphId");
    let Some(paragraph) = id.and_then(Value::as_str).and_then(|id| scan.paragraph(id)) else {
        return Err(edit_error(
            "stale-target",
            format!(
                "edit-paragraph: no paragraph {} on page {page}",
                js_string(id)
            ),
        ));
    };
    let plan = with_measure(session, page, |measure, _| {
        paragraph::plan(measure, &scan.records, paragraph, request)
    })?;
    let lines = plan.lines.len();
    if plan.noop {
        return Ok(edit_state_reply(
            session,
            request,
            json!({"page": page, "lines": lines}),
        ));
    }
    session.mutate("edit paragraph", |session| {
        let mut tx = session.tx()?;
        let page_id = write::page_object(tx.document(), page, "edit-paragraph")?;
        let span = write::SpanEdit {
            record: plan.record,
            splice: write::Splice {
                start: plan.start,
                end: plan.end,
                replacement: plan.replacement.clone(),
            },
        };
        let mut privatised = Default::default();
        let mut copied = 0;
        write::write_element(
            &mut tx,
            &scan,
            page_id,
            plan.element,
            &[],
            &[span],
            &mut privatised,
            &mut copied,
        )
        .map_err(write::writer_error)
    })?;
    let mut fields = json!({"page": page, "lines": lines});
    if paragraph.kerned {
        fields["notice"] = "kerning in this paragraph was dropped".into();
    }
    Ok(edit_state_reply(session, request, fields))
}

/// engine.ts `text-add`: a new run in a base-14 font, in its own content
/// stream appended to the page.
pub fn text_add(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    session.require_mutable("text-add")?;
    let page = session.check_page(request.get("page"), "text-add")?;
    let invalid = || {
        AppError::reason(
            "invalid",
            "text-add: invalid font, size, point, colour, or text",
        )
    };
    let font = request
        .get("font")
        .and_then(Value::as_str)
        .filter(|f| BASE14_FONTS.contains(f))
        .ok_or_else(invalid)?
        .to_string();
    let size = request
        .get("size")
        .and_then(request::as_number)
        .filter(|s| (4.0..=400.0).contains(s))
        .ok_or_else(invalid)?;
    let text = request
        .get("text")
        .and_then(Value::as_str)
        .filter(|t| (1..=MAX_TEXT_ADD_CHARS).contains(&js_len(t)))
        .ok_or_else(invalid)?
        .to_string();
    let point = request::numbers(request, "point")
        .filter(|p| p.len() == 2)
        .ok_or_else(invalid)?;
    let color = request::numbers(request, "color")
        .filter(|c| c.len() == 3 && c.iter().all(|v| (0.0..=1.0).contains(v)))
        .ok_or_else(invalid)?;
    let lines: Vec<&str> = text.split('\n').collect();
    if lines.len() > MAX_TEXT_ADD_LINES
        || text.contains('\r')
        || lines.iter().any(|line| line.chars().any(|c| c < ' '))
    {
        return Err(AppError::reason(
            "invalid",
            format!(
                "text-add: text must have at most {MAX_TEXT_ADD_LINES} lines and no control characters"
            ),
        ));
    }
    let mut missing: Vec<String> = Vec::new();
    for c in text.chars().filter(|&c| c != '\n') {
        if font::win_ansi_byte(c).is_none() && !missing.contains(&c.to_string()) {
            missing.push(c.to_string());
        }
    }
    if !missing.is_empty() {
        missing.truncate(20);
        return Err(edit_error(
            "unsupported-encoding",
            "text-add: unsupported WinAnsi characters",
        )
        .with_extra(json!({"missing": missing})));
    }
    let (transform, bounds) = with_measure(session, page, |measure, view| {
        let transform = view
            .and_then(|v| v.page_transform(page).ok())
            .unwrap_or([1.0, 0.0, 0.0, -1.0, 0.0, 792.0]);
        Ok((transform, measure.bounds))
    })?;
    let [x, y] = [point[0], point[1]];
    if x < bounds[0] || x > bounds[2] || y < bounds[1] || y > bounds[3] {
        return Err(AppError::reason(
            "invalid",
            "text-add: point is outside the page box",
        ));
    }
    {
        let doc = session.doc.as_ref().ok_or_else(invalid)?;
        let (_, dict) = doc.page(page).ok_or_else(invalid)?;
        let document = doc.document();
        let rotate = crate::app::pagetree::inherited(document, dict, b"Rotate")
            .map(|v| document.resolve(v))
            .and_then(|v| font::number(Some(v)));
        let unit = font::at(document, Some(dict), b"UserUnit").and_then(|v| font::number(Some(v)));
        let rotated = rotate.is_some_and(|r| r.trunc() % 360.0 != 0.0);
        if rotated || unit.is_some_and(|u| u != 1.0) {
            return Err(AppError::reason(
                "unsupported-transform",
                "text-add: rotated or scaled pages are not supported",
            ));
        }
    }
    let det = transform[0] * transform[3] - transform[1] * transform[2];
    let (px, py) = (x - transform[4], y - transform[5]);
    let point_pdf = [
        (transform[3] * px - transform[2] * py) / det,
        (-transform[1] * px + transform[0] * py) / det,
    ];
    let scan = scan_for(session, page)?;
    if !scan.balanced {
        return Err(AppError::reason(
            "unbalanced-content",
            "text-add: existing page content has unbalanced graphics state",
        ));
    }
    let font_name = {
        let doc = session.doc.as_ref().ok_or_else(invalid)?;
        let (_, dict) = doc.page(page).ok_or_else(invalid)?;
        let document = doc.document();
        let resources = match dict
            .get(b"Resources".as_slice())
            .map(|v| document.resolve(v))
        {
            Some(value) if !value.is_null() => value.as_dict(),
            _ => crate::app::pagetree::inherited(document, dict, b"Resources")
                .and_then(|v| document.resolve(v).as_dict()),
        };
        let fonts = font::dict_at(document, resources, b"Font");
        let mut index = 1u64;
        loop {
            let name = format!("PDF4AllF{index}");
            let taken = fonts.is_some_and(|f| !document.dict_get(f, name.as_bytes()).is_null());
            if !taken {
                break name;
            }
            index += 1;
        }
    };
    let mut stream = format!(
        "q BT /{font_name} {} Tf 0 Tc 0 Tw 100 Tz 0 TL 0 Ts 0 Tr 1 0 0 1 {} {} Tm {} rg ",
        js_number(size),
        js_number(round2(point_pdf[0])),
        js_number(round2(point_pdf[1])),
        color
            .iter()
            .map(|c| js_number(round2(*c)))
            .collect::<Vec<_>>()
            .join(" ")
    )
    .into_bytes();
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            stream
                .extend_from_slice(format!("0 -{} Td ", js_number(round2(size * 1.2))).as_bytes());
        }
        stream.extend_from_slice(&plan::literal(&plan::win_ansi_bytes(line)));
        stream.extend_from_slice(b" Tj ");
    }
    stream.extend_from_slice(b"ET Q");
    session.mutate("add text", |session| {
        let mut tx = session.tx()?;
        let page_id = write::page_object(tx.document(), page, "text-add")?;
        write::add_text(&mut tx, page_id, font_name.as_bytes(), &font, stream)
    })?;
    // The add has landed; a failed lookup scan must not turn it into a refusal.
    let added = scan_for(session, page).ok().and_then(|scan| {
        scan.records
            .iter()
            .rev()
            .find(|r| r.is_text() && r.text.as_deref() == lines.first().copied())
            .map(|r| (r.target_id.clone(), r.bounds))
    });
    let (target_id, bounds) = added.unwrap_or_default();
    Ok(edit_state_reply(
        session,
        request,
        json!({"page": page, "targetId": target_id, "bounds": bounds}),
    ))
}

/// engine.ts `checkReplaceArgs`.
fn replace_args(request: &Value) -> Result<(String, String), AppError> {
    let find = request
        .get("find")
        .and_then(Value::as_str)
        .filter(|f| (1..=MAX_REPLACE_FIND_CHARS).contains(&js_len(f)))
        .ok_or_else(|| {
            edit_error(
                "invalid",
                format!("edit-replace: find must be 1..{MAX_REPLACE_FIND_CHARS} characters"),
            )
        })?;
    let replace = request
        .get("replace")
        .and_then(Value::as_str)
        .filter(|r| js_len(r) <= MAX_EDIT_TEXT_CHARS)
        .ok_or_else(|| {
            edit_error(
                "invalid",
                format!("edit-replace: replace must be at most {MAX_EDIT_TEXT_CHARS} characters"),
            )
        })?;
    Ok((find.to_string(), replace.to_string()))
}

/// An optional `targetId`: absent, or a non-empty string.
fn scope_id<'r>(request: &'r Value, op: &str) -> Result<Option<&'r str>, AppError> {
    match request.get("targetId") {
        None => Ok(None),
        Some(value) => value
            .as_str()
            .filter(|id| !id.is_empty())
            .map(Some)
            .ok_or_else(|| {
                edit_error(
                    "invalid",
                    format!("{op}: targetId must be a non-empty string"),
                )
            }),
    }
}

struct Preview {
    matches: Vec<Value>,
    rejected: Vec<Value>,
    plans: Vec<(String, TextPlan)>,
    token: String,
}

/// engine.ts `buildReplacePreview`: every run on the page (or one run)
/// whose text contains `find`, judged by the same plan a single edit uses.
fn build_preview(
    session: &mut AppSession,
    page: usize,
    find: &str,
    replace: &str,
    target: Option<&str>,
) -> Result<Preview, AppError> {
    let scan = scan_for(session, page)?;
    let considered: Vec<usize> = match target {
        None => scan
            .map_order
            .iter()
            .copied()
            .filter(|&i| scan.records.get(i).is_some_and(|r| r.is_text()))
            .collect(),
        Some(id) => scan.index(id).into_iter().collect(),
    };
    if let (Some(id), true) = (target, considered.is_empty()) {
        return Err(edit_error(
            "stale-target",
            format!("edit-replace: no target {id} on page {page}"),
        ));
    }
    let too_large = || {
        edit_error(
            "too-large",
            format!(
                "edit-replace: the reply text is over the {MAX_EDIT_REPLY_TEXT_CHARS} character ceiling"
            ),
        )
    };
    let (generation, revision) = (session.generation, session.revision);
    let mut preview = with_measure(session, page, |measure, _| {
        let mut preview = Preview {
            matches: Vec::new(),
            rejected: Vec::new(),
            plans: Vec::new(),
            token: String::new(),
        };
        let mut reply_text = 0usize;
        for &index in &considered {
            let record = &scan.records[index];
            let Some(text) = record.text.as_deref() else {
                continue;
            };
            if !text.contains(find) {
                continue;
            }
            let count = text.matches(find).count();
            let id = record.target_id.clone();
            if !record.candidate {
                reply_text += js_len(text);
                let mut row = json!({"targetId": id, "text": text, "count": count,
                    "detail": record.reason.unwrap_or("not-candidate")});
                if let Some(reason) = record.reason {
                    row["reason"] = reason.into();
                }
                preview.rejected.push(row);
                continue;
            }
            if record.cow.as_ref().is_some_and(|c| c.form.is_some()) {
                reply_text += js_len(text);
                preview
                    .rejected
                    .push(json!({"targetId": id, "text": text, "count": count,
                    "detail": "shared-placement"}));
                continue;
            }
            let projected =
                js_len(text) as i64 + count as i64 * (js_len(replace) as i64 - js_len(find) as i64);
            if projected > MAX_EDIT_TEXT_CHARS as i64 {
                reply_text += js_len(text);
                preview
                    .rejected
                    .push(json!({"targetId": id, "text": text, "count": count,
                    "detail": "too-large"}));
                continue;
            }
            let next = text.replace(find, replace);
            reply_text += js_len(text) + js_len(&next);
            if reply_text > MAX_EDIT_REPLY_TEXT_CHARS {
                return Err(too_large());
            }
            match plan::plan_text(measure, index, record, &next, Geometry::default()) {
                Ok(plan) => {
                    preview.matches.push(json!({
                        "targetId": id, "text": text, "next": next, "count": count,
                        "oldWidth": plan.old_width, "newWidth": plan.new_width,
                        "tailFollows": record.tail_follows,
                    }));
                    preview.plans.push((id, plan));
                }
                Err(error) => {
                    let mut row = Map::new();
                    row.insert("targetId".into(), id.into());
                    row.insert("text".into(), text.into());
                    row.insert("count".into(), count.into());
                    row.insert(
                        "detail".into(),
                        error.detail.unwrap_or_else(|| "invalid".into()).into(),
                    );
                    row.extend(error.extra);
                    preview.rejected.push(Value::Object(row));
                }
            }
        }
        if reply_text > MAX_EDIT_REPLY_TEXT_CHARS {
            return Err(too_large());
        }
        Ok(preview)
    })?;
    let ids: Vec<&str> = preview.plans.iter().map(|(id, _)| id.as_str()).collect();
    let scope = target.unwrap_or("page");
    preview.token = scan::edit_hash_text(&format!(
        "{generation}|{revision}|{page}|{scope}|{find}|{replace}|{}",
        ids.join(",")
    ));
    Ok(preview)
}

/// engine.ts `edit-replace-preview`: read-only; no operation opens.
pub fn replace_preview(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    readable(session, "edit-replace-preview")?;
    let page = session.check_page(request.get("page"), "edit-replace-preview")?;
    copy_permitted(session, "edit-replace-preview")?;
    let (find, replace) = replace_args(request)?;
    let target = scope_id(request, "edit-replace-preview")?;
    // engine.ts ignored these; a preview of another revision than the
    // caller's scan would describe targets the caller does not show.
    for (key, value) in [
        ("generation", session.generation),
        ("revision", session.revision),
    ] {
        let given = request.get(key).filter(|v| !v.is_null());
        if given.is_some_and(|v| request::as_js_number(v) != Some(value as f64)) {
            return Err(edit_error(
                "stale-target",
                "edit-replace-preview: the document changed since the scan",
            ));
        }
    }
    let preview = build_preview(session, page, &find, &replace, target)?;
    Ok(reply(
        request,
        json!({
            "page": page,
            "generation": session.generation,
            "revision": session.revision,
            "find": find,
            "replace": replace,
            "matches": preview.matches,
            "rejected": preview.rejected,
            "previewToken": preview.token,
        }),
    ))
}

/// engine.ts `edit-replace-apply`: the previewed runs, recomputed and
/// checked against the token, written in one operation.
pub fn replace_apply(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    session.require_mutable("edit-replace-apply")?;
    let page = session.check_page(request.get("page"), "edit-replace-apply")?;
    let (find, replace) = replace_args(request)?;
    let token = request
        .get("previewToken")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| {
            edit_error(
                "invalid",
                "edit-replace-apply: previewToken must be a non-empty string",
            )
        })?
        .to_string();
    let ids: Vec<String> = request
        .get("targetIds")
        .and_then(Value::as_array)
        .filter(|ids| (1..=MAX_EDIT_TARGETS).contains(&ids.len()))
        .and_then(|ids| {
            ids.iter()
                .map(|id| id.as_str().filter(|s| !s.is_empty()).map(str::to_string))
                .collect::<Option<Vec<_>>>()
        })
        .ok_or_else(|| {
            edit_error(
                "invalid",
                format!(
                    "edit-replace-apply: targetIds must be 1..{MAX_EDIT_TARGETS} non-empty strings"
                ),
            )
        })?;
    let target = scope_id(request, "edit-replace-apply")?.map(str::to_string);
    let preview = build_preview(session, page, &find, &replace, target.as_deref())?;
    if preview.token != token {
        return Err(edit_error(
            "stale-preview",
            "edit-replace-apply: the page changed since the preview",
        ));
    }
    let mut plans = Vec::with_capacity(ids.len());
    for id in &ids {
        let Some((_, plan)) = preview.plans.iter().find(|(planned, _)| planned == id) else {
            return Err(edit_error(
                "stale-target",
                format!("edit-replace-apply: {id} is not in this preview"),
            )
            .with_extra(json!({"targetId": id})));
        };
        plans.push(plan.clone());
    }
    let scan = scan_for(session, page)?;
    let count = plans.len();
    session.mutate("replace text", |session| {
        let mut tx = session.tx()?;
        let page_id = write::page_object(tx.document(), page, "edit-replace-apply")?;
        write::write_plans(&mut tx, &scan, page_id, &plans, |error, id| {
            edit_error(
                "write-failed",
                format!("edit-replace-apply: Error: {}", error.message),
            )
            .with_extra(json!({"targetId": id}))
        })
    })?;
    Ok(edit_state_reply(
        session,
        request,
        json!({"page": page, "replaced": count}),
    ))
}
