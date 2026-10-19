// SPDX-License-Identifier: MIT
//! Redaction marks: redact-add, redact-set, redact-search and
//! redact-preview (engine.ts with redact.ts), each write one journal step
//! (`add redaction mark`, `move redaction mark`, `mark redaction search
//! hits`). The marks are Redact annotations (`app::redact::marks`); what
//! they cover is `app::redact::region`.
use serde_json::{Map, Value, json};

use crate::app::annots::{self, PageSpace};
use crate::app::document::{limits, open_options};
use crate::app::edit::{Builder, Edit};
use crate::app::info::{self, PERMISSION_BITS, Permissions};
use crate::app::layers;
use crate::app::ops::annots::find_annotation;
use crate::app::redact::apply::{self as redaction, ApplyError};
use crate::app::redact::content::Limits;
use crate::app::redact::inspect;
use crate::app::redact::inventory;
use crate::app::redact::marks::{
    self, MAX_REDACT_MARKS, MAX_REDACT_TEXT, Mark, Plan, SearchError, redact_rect,
};
use crate::app::redact::private;
use crate::app::redact::region::{self, GlyphRule, Shared, SharedError};
use crate::app::redact::sanitize::{self, Sanitize};
use crate::app::redact::verify;
use crate::app::{
    AppError, AppSession, Buffers, MAX_SAVE_BYTES, SecuritySetting, overlay, password, reply,
};
use crate::document::{Document, DocumentErrorKind};
use crate::object::ObjectId;
use crate::serialize::SerializeError;
use crate::view::ViewDocument;
use crate::writer::{DocumentWriteError, Encryption, Entropy, WriteOptions, write_document};

/// limits.ts `MAX_REDACT_SOURCE_BYTES`: the rewrite redact-apply starts
/// from (the measured memory guard).
pub const MAX_REDACT_SOURCE_BYTES: usize = 12 * 1024 * 1024;

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

/// A request's `regions` (`[{page, rect}]`, rects in page space as
/// redact-add's, clamped the same way) as area marks held in memory
/// (R6). Refusals are tagged `invalid` with detail `regions`; past
/// `MAX_REDACT_MARKS` boxes, `too-many`.
fn read_regions(session: &AppSession, value: &Value, op: &str) -> Result<Vec<Mark>, AppError> {
    read_boxes(session, value, op, false)
}

/// `read_regions`; with `named`, a region may also name one of the
/// document's Redact marks (`annotId`, on its page: its own rect, quads and
/// rule; `rect` is not needed), or ask for a search mark's tight rule
/// (`search: true`: the rect is a find hit grown by two points).
fn read_boxes(
    session: &AppSession,
    value: &Value,
    op: &str,
    named: bool,
) -> Result<Vec<Mark>, AppError> {
    let invalid = |message: String| AppError::detail("invalid", "regions", message);
    let Some(items) = value.as_array() else {
        return Err(invalid(format!(
            "{op}: regions must be a list of {{page, rect}}"
        )));
    };
    if items.len() > MAX_REDACT_MARKS {
        return Err(too_many(op));
    }
    let mut out = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let at = |error: AppError| invalid(format!("{op}: regions[{index}]: {}", error.message));
        if !item.is_object() {
            return Err(invalid(format!(
                "{op}: regions[{index}] must be {{page, rect}}"
            )));
        }
        let page = session.check_page(item.get("page"), op).map_err(at)?;
        if let (true, Some(annot)) = (named, item.get("annotId").filter(|v| !v.is_null())) {
            let id = find_annotation(session, page, Some(annot), op).map_err(at)?;
            let found = session.doc.as_ref().and_then(|app| {
                let (_, dict) = app.page(page)?;
                marks::page_marks(app.document(), page, dict)
                    .into_iter()
                    .find(|m| m.id == Some(id))
            });
            let mark = found.ok_or_else(|| {
                invalid(format!(
                    "{op}: regions[{index}]: annotation is not a redaction mark"
                ))
            })?;
            out.push(mark);
            continue;
        }
        let rect =
            redact_rect(page_bounds(session, page, op)?, item.get("rect"), op).map_err(at)?;
        let search = named && item.get("search").and_then(Value::as_bool) == Some(true);
        let mut mark = Mark::area(page, rect);
        if search {
            mark.by_area = false;
            mark.search = Some(String::new());
            mark.quads = vec![[
                (rect[0], rect[1]),
                (rect[2], rect[1]),
                (rect[0], rect[3]),
                (rect[2], rect[3]),
            ]];
        }
        out.push(mark);
    }
    Ok(out)
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
    // Past the view's limits (its resource budget included) the search
    // refuses `scan-budget`, as a page too complex to search does.
    let failed = |error: crate::view::ViewError| match error {
        crate::view::ViewError::TooComplex(_) => {
            AppError::reason("scan-budget", format!("{OP}: {error}"))
        }
        _ => AppError::plain(format!("{OP}: {error}")),
    };
    let found = match app.view_unlayered().map_err(failed)? {
        Some(view) => marks::search_plans(app.document(), &view, &text, room),
        None => {
            let (view, doc) = app.view_with_document().map_err(failed)?;
            marks::search_plans(doc, view, &text, room)
        }
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
    let (shared, inline_alt) = shared_of(app, &all, OP)?;
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

/// Page-space numbers in a reply, each kept to a hundredth of a point
/// (never NaN or infinite: JSON would read them as null).
fn rounded(values: &[f64]) -> Vec<f64> {
    (values.iter())
        .map(|v| match v.is_finite() {
            true => (v * 100.0).round() / 100.0,
            false => 0.0,
        })
        .collect()
}

/// redact-inspect: for each box of `regions`, what redact-apply would
/// remove under it with the same rules (`redact::inspect`): the glyphs'
/// quads and what they spell, the pictures, shapes and shadings it touches
/// (the part under the box) and the annotations and form fields it touches
/// (whole). Read only: no journal state, allowed on a signed document.
pub fn inspect(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    const OP: &str = "redact-inspect";
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
    let boxes = read_boxes(
        session,
        request.get("regions").unwrap_or(&Value::Null),
        OP,
        true,
    )?;
    let Some(app) = session.doc.as_mut() else {
        return Err(AppError::reason(
            "no-document",
            format!("{OP}: no document open"),
        ));
    };
    let found = match app.view_with_document() {
        Ok((view, doc)) => inspect::inspect(doc, Some(view), &boxes),
        Err(_) => inspect::inspect(app.document(), None, &boxes),
    }
    .map_err(|e| apply_error(e, OP))?;
    let regions: Vec<Value> = (boxes.iter().zip(&found))
        .map(|(mark, found)| {
            json!({
                "page": mark.page,
                "rect": rounded(&mark.rect),
                "text": found.text,
                "glyphs": found.glyphs.iter().map(|q| rounded(q)).collect::<Vec<_>>(),
                "images": found.images.iter().map(|i| json!({"rect": rounded(&i.rect), "whole": i.whole})).collect::<Vec<_>>(),
                "shapes": found.shapes.iter().map(|s| json!({"rect": rounded(&s.rect), "kind": s.kind, "whole": s.whole})).collect::<Vec<_>>(),
                "annots": found.annots.iter().map(|a| json!({"kind": a.kind, "rect": rounded(&a.rect), "label": a.label})).collect::<Vec<_>>(),
                "needsFlatten": found.needs_flatten,
                "truncated": found.truncated,
            })
        })
        .collect();
    Ok(reply(request, json!({"regions": regions})))
}

/// redact.ts `redactPreview`'s `shared` and `inlineAlt` for `marks`.
fn shared_of(
    app: &mut crate::app::AppDocument,
    marks: &[Mark],
    op: &str,
) -> Result<(Vec<Shared>, usize), AppError> {
    let shared = if marks.is_empty() {
        Ok((Vec::new(), 0))
    } else {
        match app.view_with_document() {
            Ok((view, doc)) => region::shared(doc, Some(view), marks, Limits::DEFAULT),
            Err(_) => region::shared(app.document(), None, marks, Limits::DEFAULT),
        }
    };
    shared.map_err(|e| shared_error(e, op))
}

fn shared_error(error: SharedError, op: &str) -> AppError {
    match error {
        SharedError::TooComplex(_) => AppError::reason(
            "scan-budget",
            format!("{op}: page content is too complex to check for shared graphics"),
        ),
        SharedError::Unreadable(page, why) => AppError::reason(
            "invalid",
            format!(
                "{op}: the content of page {} cannot be read ({why})",
                page + 1
            ),
        ),
    }
}

fn verify_failed(message: String) -> AppError {
    AppError::reason("verify-failed", message)
}

/// The pages (1-based) listed in a refusal at most; past it, "and N more".
const LISTED_PAGES: usize = 10;

/// The marks of `failed` as a refusal names them: how many, and on which
/// pages ("2 marks on pages 1 and 3"). Never their rects or search texts:
/// the message reaches the window, and a search text is what the user is
/// removing.
fn marks_named(marks: &[Mark], failed: &std::collections::BTreeSet<usize>) -> String {
    let pages: std::collections::BTreeSet<usize> = (failed.iter())
        .filter_map(|&i| marks.get(i))
        .map(|mark| mark.page + 1)
        .collect();
    let count = failed.len();
    let mut listed: Vec<String> = pages
        .iter()
        .take(LISTED_PAGES)
        .map(usize::to_string)
        .collect();
    let more = pages.len().saturating_sub(LISTED_PAGES);
    let list = match (listed.len(), more) {
        (_, more) if more > 0 => format!("{} and {more} more", listed.join(", ")),
        (0 | 1, _) => listed.join(""),
        _ => {
            let last = listed.pop().unwrap_or_default();
            format!("{} and {last}", listed.join(", "))
        }
    };
    format!(
        "{count} mark{} on page{} {list}",
        if count == 1 { "" } else { "s" },
        if pages.len() == 1 { "" } else { "s" },
    )
}

/// The pixel check's refusals: a page it cannot check is refused
/// `scan-budget`, not failed.
fn pixel_refusal(refusal: verify::PixelRefusal, op: &str) -> AppError {
    let message = match refusal {
        verify::PixelRefusal::Undrawable(page) => format!(
            "{op}: page {} has no size that can be drawn at 36 dpi or more, so its marks cannot be verified",
            page + 1
        ),
        verify::PixelRefusal::TooComplex(page) => format!(
            "{op}: page {} is too complex to draw, so its marks cannot be verified",
            page + 1
        ),
        verify::PixelRefusal::TooThin(page) => format!(
            "{op}: page {} is too complex to draw at a resolution that verifies its thinnest marks",
            page + 1
        ),
        verify::PixelRefusal::TooMany(page) => format!(
            "{op}: the marks are too many to verify against the drawing of page {}",
            page + 1
        ),
    };
    AppError::reason("scan-budget", message)
}

/// R4's refusal: `pages` (0-based) hold content a box covers that cannot be
/// cut exactly; resent with them in `flatten`, they become pictures.
fn needs_flatten(pages: &[usize], op: &str) -> AppError {
    let listed: Vec<String> = pages.iter().map(|p| (p + 1).to_string()).collect();
    AppError::reason(
        "needs-flatten",
        format!(
            "{op}: page {} holds artwork a box covers that cannot be cut exactly",
            listed.join(", ")
        ),
    )
    .with_extra(json!({ "pages": pages }))
}

/// A request's `flatten`: the pages (0-based) that may become pictures.
/// Absent or null, none.
fn read_flatten(
    session: &AppSession,
    value: Option<&Value>,
    op: &str,
) -> Result<Vec<usize>, AppError> {
    let invalid = |message: String| AppError::detail("invalid", "flatten", message);
    let Some(value) = value.filter(|v| !v.is_null()) else {
        return Ok(Vec::new());
    };
    let Some(items) = value.as_array() else {
        return Err(invalid(format!("{op}: flatten must be a list of pages")));
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let page = session
            .check_page(Some(item), op)
            .map_err(|e| invalid(e.message))?;
        if !out.contains(&page) {
            out.push(page);
        }
    }
    out.sort_unstable();
    Ok(out)
}

fn apply_error(error: ApplyError, op: &str) -> AppError {
    match error {
        ApplyError::NeedsFlatten(page) => needs_flatten(&[page], op),
        ApplyError::TooComplex(page) => AppError::reason(
            "scan-budget",
            format!(
                "{op}: the content of page {} is too complex to redact",
                page + 1
            ),
        ),
        ApplyError::Unreadable(page, why) => AppError::reason(
            "invalid",
            format!(
                "{op}: the content of page {} cannot be read ({why})",
                page + 1
            ),
        ),
        ApplyError::TooLarge(why) => AppError::reason("too-large", format!("{op}: {why}")),
        ApplyError::Unsupported(why) => AppError::reason(
            "invalid",
            format!("{op}: {why}, so the marks cannot be applied exactly"),
        ),
        ApplyError::Write(why) => AppError::plain(format!("{op}: {why}")),
    }
}

/// How the copy is encrypted: the save's choice (`save_encryption`).
fn encryption(setting: Option<&SecuritySetting>) -> Encryption<'_> {
    match setting {
        Some(SecuritySetting::None) => Encryption::Plaintext,
        Some(SecuritySetting::Aes256 {
            user_password,
            owner_password,
            permissions,
        }) => Encryption::Aes256 {
            user_password: user_password.as_bytes(),
            owner_password: owner_password.as_bytes(),
            permissions: *permissions,
        },
        _ => Encryption::Keep,
    }
}

/// engine.ts `validateDocument(session, bytes, sanitized)`: the copy opens
/// strictly with its password, has the session's pages, is encrypted as
/// the save's setting says and grants its permissions. The declared
/// features a sanitize removes are not compared.
fn check_copy(
    doc: &crate::app::AppDocument,
    session_password: Option<&str>,
    setting: Option<&SecuritySetting>,
    locked: bool,
    bytes: &[u8],
) -> Result<(), AppError> {
    let repaired = || AppError::reason("invalid", "validate: the saved copy needed repair");
    let wrong = || {
        AppError::reason(
            "invalid",
            "validate: the saved copy did not open with its own password",
        )
    };
    // As the app reads (`open_options`): a stream naming an external file
    // is read from its own bytes.
    let check = match Document::open_with(bytes, limits(), b"", open_options()) {
        Ok(check) => check,
        Err(error) if matches!(error.kind, DocumentErrorKind::PasswordRequired) => {
            let (password, revision) = match setting {
                Some(SecuritySetting::Aes256 { user_password, .. }) => {
                    (Some(user_password.as_str()), Some(6))
                }
                _ => (
                    session_password,
                    doc.document().security().map(|security| security.revision),
                ),
            };
            let Some(password) = password.filter(|p| !p.is_empty()) else {
                return Err(wrong());
            };
            let mut opened = None;
            for candidate in password::candidates(password, revision) {
                match Document::open_with(bytes, limits(), &candidate, open_options()) {
                    Ok(check) => {
                        opened = Some(check);
                        break;
                    }
                    Err(error) if matches!(error.kind, DocumentErrorKind::PasswordRequired) => {}
                    Err(_) => return Err(repaired()),
                }
            }
            opened.ok_or_else(wrong)?
        }
        Err(_) => return Err(repaired()),
    };
    let count = info::pages(&check).len();
    let expected = doc.page_count();
    if count != expected {
        return Err(AppError::reason(
            "invalid",
            format!("validate: the saved copy has {count} pages, expected {expected}"),
        ));
    }
    let before = doc.info(locked);
    let after = info::detect(&check, false, false);
    let encrypted = match setting {
        Some(SecuritySetting::None) => false,
        Some(SecuritySetting::Aes256 { .. }) => true,
        _ => before.encrypted,
    };
    if after.encrypted != encrypted {
        return Err(AppError::reason(
            "invalid",
            "validate: the saved copy changed the document's declared features",
        ));
    }
    match setting {
        Some(SecuritySetting::Aes256 { permissions, .. }) => {
            let bits = info::permission_bits(&check);
            for (name, bit) in PERMISSION_BITS {
                let written = bits.is_some_and(|bits| bits & bit == bit);
                if written != (permissions & bit == bit) {
                    return Err(AppError::reason(
                        "invalid",
                        format!(
                            "validate: the saved copy's {name} permission does not match what was requested"
                        ),
                    ));
                }
            }
        }
        Some(SecuritySetting::None) => {
            if after.permissions != Permissions::ALL {
                return Err(AppError::reason(
                    "invalid",
                    "validate: the saved copy is still restricted after encryption was removed",
                ));
            }
        }
        _ => {
            if after.permissions != before.permissions {
                return Err(AppError::reason(
                    "invalid",
                    "validate: the saved copy has different permissions than the source",
                ));
            }
        }
    }
    Ok(())
}

/// A plain copy of the output the view reads as the session's view reads
/// the document: each stream naming an external file read from its own
/// bytes (hayro refuses a stream with /F; reference viewers and the app
/// read its bytes), and with `shown` every optional content shown, as
/// redact-search reads (`AppDocument::view_unlayered`). Marked pages' content
/// is rewritten, so none of it is encoded with an image filter.
fn readable_view(
    doc: &Document<'static>,
    external: &[ObjectId],
    shown: Option<&layers::ShowAll>,
    limits: crate::writer::DocumentWriteLimits,
) -> Result<ViewDocument, AppError> {
    let failed = || verify_failed("redact-apply: output could not be verified".into());
    let mut copy = Builder::from_document(doc.clone());
    for &id in external {
        copy.update(id, |value| {
            if let Some(dictionary) = value.as_dict_mut() {
                for key in [b"F".as_slice(), b"FFilter", b"FDecodeParms"] {
                    dictionary.remove(key);
                }
            }
        })
        .map_err(|_| failed())?;
    }
    if let Some(shown) = shown {
        shown.apply(&mut copy).map_err(|_| failed())?;
    }
    let plain = WriteOptions {
        encryption: Encryption::Plaintext,
        ..WriteOptions::APP
    };
    let bytes = write_document(copy.document(), limits, plain).map_err(|_| failed())?;
    ViewDocument::open(bytes, "").map_err(|_| failed())
}

/// The streams of `doc` naming an external file (/F).
fn external_streams(doc: &Document<'_>) -> Vec<ObjectId> {
    (doc.objects())
        .filter(|object| object.stream.is_some())
        .filter(|object| {
            (object.value.as_dict())
                .and_then(|dictionary| dictionary.get(b"F".as_slice()))
                .is_some_and(|f| !f.is_null())
        })
        .map(|object| object.id)
        .collect()
}

/// The written copy opened as the app opens documents (with `password`
/// when encrypted); None when it does not open.
fn reopen<'b>(
    bytes: &'b [u8],
    password: Option<&str>,
    revision: Option<i64>,
) -> Option<Document<'b>> {
    match Document::open_with(bytes, limits(), b"", open_options()) {
        Ok(doc) => Some(doc),
        Err(error) if matches!(error.kind, DocumentErrorKind::PasswordRequired) => password
            .and_then(|password| {
                password::candidates(password, revision)
                    .into_iter()
                    .find_map(|candidate| {
                        Document::open_with(bytes, limits(), &candidate, open_options()).ok()
                    })
            }),
        Err(_) => None,
    }
}

/// R5: the pages (0-based, ascending) of the box runs still in the copy
/// beyond what it shows outside the boxes. `content` counts each run in
/// the marked pages' content (`verify::content`).
#[allow(clippy::too_many_arguments)]
fn runs_left(
    output: &ViewDocument,
    bytes: &[u8],
    password: Option<&str>,
    revision: Option<i64>,
    marked: &[usize],
    runs: &[verify::BoxRun],
    content: &[usize],
    op: &str,
) -> Result<Vec<usize>, AppError> {
    let counter = verify::RunCounter::new(runs);
    let shown = verify::visible_runs(output, marked.iter().copied(), &counter);
    let copy = reopen(bytes, password, revision)
        .ok_or_else(|| verify_failed(format!("{op}: output could not be verified")))?;
    let hidden = verify::hidden_runs(&copy, &counter);
    let mut everywhere: Option<Vec<usize>> = None;
    let mut pages = std::collections::BTreeSet::new();
    for (i, run) in runs.iter().enumerate() {
        let raw = content.get(i).copied().unwrap_or(0);
        let seen = shown.get(i).copied().unwrap_or(0);
        let elsewhere = hidden.get(i).copied().unwrap_or(0);
        let mut left = raw > seen;
        if !left && elsewhere > 0 {
            let all = everywhere.get_or_insert_with(|| {
                verify::visible_runs(output, 0..output.page_count(), &counter)
            });
            left = raw.saturating_add(elsewhere) > all.get(i).copied().unwrap_or(0);
        }
        if left {
            pages.extend(run.pages.iter().copied());
        }
    }
    Ok(pages.into_iter().collect())
}

/// The copy reopens strictly (with `password` when encrypted, as the app
/// opens documents) as one revision.
fn one_revision(
    bytes: &[u8],
    password: Option<&str>,
    revision: Option<i64>,
    op: &str,
) -> Result<(), AppError> {
    let compact = || verify_failed(format!("{op}: output is not one compact revision"));
    // As the app reads (`open_options`): a stream naming an external file
    // is read from its own bytes, so a copy keeping one opens.
    let check = match Document::open_with(bytes, limits(), b"", open_options()) {
        Ok(check) => check,
        Err(error) if matches!(error.kind, DocumentErrorKind::PasswordRequired) => {
            let opened = password.and_then(|password| {
                password::candidates(password, revision)
                    .into_iter()
                    .find_map(|candidate| {
                        Document::open_with(bytes, limits(), &candidate, open_options()).ok()
                    })
            });
            opened.ok_or_else(|| verify_failed(format!("{op}: output could not be verified")))?
        }
        Err(_) => return Err(compact()),
    };
    if check.trailer_chain().count() != 1 {
        return Err(compact());
    }
    Ok(())
}

/// engine.ts `redact-apply` (redact.ts `applyRedactions`): the marks
/// applied to a copy of the document (`redact::apply`), the chosen
/// surfaces sanitized (`redact::sanitize`), the copy checked
/// (`redact::verify`) and written in full with the save's encryption; the
/// open document, its marks and its journal stay as they are. Refusals in
/// engine.ts's order.
#[allow(clippy::too_many_lines)]
pub fn apply(
    session: &mut AppSession,
    request: &Value,
    buffers: &mut Buffers,
) -> Result<Value, AppError> {
    const OP: &str = "redact-apply";
    // D5: a signed document is redacted only into an unsigned copy, with
    // the user's consent (`unsign: true`); otherwise it refuses `signed`.
    let unsign = request.get("unsign").and_then(Value::as_bool) == Some(true);
    let signed = !session.locked
        && (session.doc.as_ref()).is_some_and(|doc| doc.info(session.locked).signed());
    match signed {
        true if !unsign => {
            return Err(AppError::reason(
                "signed",
                format!("{OP}: redacting removes the document's signatures"),
            ));
        }
        true => {
            let info = (session.doc.as_ref()).map(|doc| doc.info(session.locked));
            let need = crate::app::op_permission(OP);
            if !info.is_some_and(|info| need.allowed(&info)) {
                return Err(AppError::reason(
                    "read-only",
                    format!("{OP}: this document does not permit changes"),
                ));
            }
        }
        false => {
            session.require_mutable(OP)?;
        }
    }
    if session.source_bytes > MAX_SAVE_BYTES {
        return Err(AppError::reason(
            "too-large",
            format!("{OP}: source exceeds {MAX_SAVE_BYTES} bytes"),
        ));
    }
    let setting = session.effective_security().cloned();
    let password = match &session.security {
        Some(SecuritySetting::Aes256 { user_password, .. }) => Some(user_password.clone()),
        _ => session.password.clone(),
    };
    let locked = session.locked;
    // R6: boxes held in memory, applied with the document's marks.
    let regions = match request.get("regions") {
        None | Some(Value::Null) => Vec::new(),
        Some(value) => read_regions(session, value, OP)?,
    };
    let flatten = read_flatten(session, request.get("flatten"), OP)?;
    let entropy = (buffers.input(request.get("entropy")))
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
        .map(Entropy);
    let Some(app) = session.doc.as_mut() else {
        return Err(AppError::reason(
            "no-document",
            format!("{OP}: no document open"),
        ));
    };
    // redact.ts `redactPreview`.
    let mut all = marks::marks(app.document());
    if all.len().saturating_add(regions.len()) > MAX_REDACT_MARKS && !regions.is_empty() {
        return Err(too_many(OP));
    }
    all.extend(regions);
    let (shared, _) = shared_of(app, &all, OP)?;
    inventory::inventory(app.document(), OP)?;
    if all.is_empty() {
        return Err(AppError::reason(
            "no-marks",
            format!("{OP}: there are no marks"),
        ));
    }
    // R3: graphics a box touches that are also drawn elsewhere are copied
    // for the boxed placements (below), not refused.
    let copy_shared = !shared.is_empty();
    let chosen = Sanitize::read(request.get("sanitize"), OP)?;
    // The memory guard on the rewrite apply starts from.
    let mut guard = overlay::limits();
    guard.syntax.max_output_bytes = MAX_REDACT_SOURCE_BYTES + 1;
    let plain = WriteOptions {
        encryption: Encryption::Plaintext,
        ..WriteOptions::APP
    };
    let too_big = || {
        AppError::reason(
            "too-large",
            "Redaction is limited to documents up to 12 MiB in this version",
        )
    };
    match write_document(app.document(), guard, plain) {
        Ok(bytes) if bytes.len() <= MAX_REDACT_SOURCE_BYTES => {}
        Ok(_) | Err(DocumentWriteError::Serialize(SerializeError::OutputLimit)) => {
            return Err(too_big());
        }
        Err(error) => return Err(AppError::reason("invalid", format!("save: {error}"))),
    }
    let scratch = app.document().clone();
    let revision = scratch.security().map(|security| security.revision);
    let view: Option<&ViewDocument> = app.view().ok();
    // R5: the text the view reads inside each area box, before apply.
    let runs = view.map_or_else(Vec::new, |view| verify::box_runs(view, &all));
    let mut builder = Builder::from_document(scratch);
    if signed {
        sanitize::remove_signatures(&mut builder, OP)?;
    }
    // R4: the pages the user agreed may become a picture, among the marked
    // ones; their marks are burnt into the picture (`flatten`), the others
    // are applied.
    let marked: std::collections::BTreeSet<usize> = all.iter().map(|m| m.page).collect();
    let flat_pages: Vec<usize> = flatten
        .iter()
        .copied()
        .filter(|p| marked.contains(p))
        .collect();
    let normal: Vec<Mark> = (all.iter())
        .filter(|m| !flat_pages.contains(&m.page))
        .cloned()
        .collect();
    let origins = match copy_shared {
        true => {
            let private =
                private::privatize(&mut builder, view, &normal).map_err(|e| apply_error(e, OP))?;
            // What is still shared after the copies (a placement the
            // copies could not reach) refuses as before.
            let left = region::shared(builder.document(), view, &normal, Limits::DEFAULT)
                .map(|(left, _)| left)
                .map_err(|e| shared_error(e, OP))?;
            if !left.is_empty() {
                return Err(AppError::reason(
                    "shared-content",
                    format!("{OP}: marked graphics also appear without a mark"),
                ));
            }
            private.origins
        }
        false => std::collections::BTreeMap::new(),
    };
    let mut applied = match redaction::apply(&mut builder, view, &normal, &origins) {
        Ok(applied) => applied,
        Err(ApplyError::NeedsFlatten(_)) => {
            let pages = redaction::pages_to_flatten(builder.document(), view, &normal)
                .map_err(|e| apply_error(e, OP))?;
            return Err(needs_flatten(&pages, OP));
        }
        Err(error) => return Err(apply_error(error, OP)),
    };
    if !flat_pages.is_empty() {
        redaction::flatten(&mut builder, view, &all, &flat_pages)
            .map_err(|e| apply_error(e, OP))?;
        applied.pages.extend(flat_pages.iter().copied());
        applied.pages.sort_unstable();
        applied.pages.dedup();
    }
    // redact.ts takes the inventory after the marks are applied.
    let found = inventory::inventory(builder.document(), OP)?;
    let sanitized = chosen.split(&found, true);
    let kept = chosen.split(&found, false);
    let builder = sanitize::sanitize(builder, &chosen, OP)?;
    let rule = GlyphRule::new(builder.document(), &all);
    let checked = verify::content(builder.document(), view, &all, &rule, &runs)
        .map_err(|e| apply_error(e, OP))?;
    let mut text_failed = checked.text;
    let mut output_limits = overlay::limits();
    output_limits.syntax.max_output_bytes = MAX_SAVE_BYTES;
    let options = WriteOptions {
        encryption: encryption(setting.as_ref()),
        entropy,
        ..WriteOptions::APP
    };
    let bytes = match write_document(builder.document(), output_limits, options) {
        Ok(bytes) => bytes,
        Err(DocumentWriteError::Serialize(SerializeError::OutputLimit)) => {
            return Err(AppError::reason(
                "too-large",
                format!("{OP}: output exceeds {MAX_SAVE_BYTES} bytes"),
            ));
        }
        Err(error) => return Err(AppError::reason("invalid", format!("{OP}: {error}"))),
    };
    // The copy's view is bounded as the session's is (`view_cost`): past
    // the budget hayro parses every page's resources without bound.
    if crate::app::view_cost::over_budget(builder.document(), crate::app::view_cost::VIEW_LIMITS) {
        return Err(AppError::reason(
            "too-large",
            format!(
                "{OP}: the pages' resources are larger than the view's limit, so the copy cannot be checked"
            ),
        ));
    }
    // The copy as written; one keeping a stream that names an external
    // file (hayro refuses /F) is read as the session's view reads it.
    let external = external_streams(builder.document());
    let output = match external.is_empty() {
        true => ViewDocument::open(bytes.clone(), password.as_deref().unwrap_or(""))
            .map_err(|_| verify_failed(format!("{OP}: output could not be verified")))?,
        false => readable_view(builder.document(), &external, None, output_limits)?,
    };
    // redact-search reads every optional content: so does its check.
    let shown = layers::show_all(builder.document());
    let layered = !shown.is_empty();
    let (failed, found) = verify::view_text(&output, &all, &rule, !layered);
    if rule.spent() {
        return Err(AppError::reason(
            "scan-budget",
            format!("{OP}: the marks are too many to check against the page's text"),
        ));
    }
    text_failed.extend(failed);
    if !text_failed.is_empty() {
        return Err(verify_failed(format!(
            "{OP}: text check failed: text is still under {}",
            marks_named(&all, &text_failed)
        )));
    }
    let pixel_failed = verify::pixels(&output, &all).map_err(|r| pixel_refusal(r, OP))?;
    if !pixel_failed.is_empty() {
        return Err(verify_failed(format!(
            "{OP}: pixels check failed: something is still drawn over {}",
            marks_named(&all, &pixel_failed)
        )));
    }
    let left = match (checked.left, found) {
        (Some(search), _) | (None, Some(search)) => Some(search),
        // A plain copy with every optional content shown (`readable_view`).
        (None, None) if layered && !verify::searches(&all).is_empty() => {
            let view = readable_view(builder.document(), &external, Some(&shown), output_limits)?;
            verify::search_left(&view, &all)
        }
        (None, None) => None,
    };
    // Which search text is left stays out of the message (it reaches the
    // window); the count of searches looked for is in the reply on success.
    if left.is_some() {
        return Err(verify_failed(format!(
            "{OP}: raw check failed: a searched text is still in the marked pages' content"
        )));
    }
    // R5: no box run is left in the copy beyond what it still shows
    // outside the boxes: in the marked pages' content, against what those
    // pages show; with what the copy holds outside page content (alternate
    // texts, annotation and field texts, titles, metadata), against what
    // every page shows.
    if !runs.is_empty() {
        let raw_failed = runs_left(
            &output,
            &bytes,
            password.as_deref(),
            revision,
            &applied.pages,
            &runs,
            &checked.runs,
            OP,
        )?;
        if !raw_failed.is_empty() {
            return Err(verify_failed(format!(
                "{OP}: raw check failed: text read inside a box is still in the copy"
            ))
            .with_extra(json!({"detail": "raw", "pages": raw_failed})));
        }
    }
    if verify::marks_left(builder.document(), &applied.pages) {
        return Err(verify_failed(format!(
            "{OP}: raw check failed because a Redact mark remains"
        )));
    }
    if verify::derived_left(builder.document(), &applied.pages) {
        return Err(verify_failed(format!(
            "{OP}: raw check failed because a marked page keeps a thumbnail or application data"
        )));
    }
    one_revision(&bytes, password.as_deref(), revision, OP)?;
    if bytes.len() > MAX_SAVE_BYTES {
        return Err(AppError::reason(
            "too-large",
            format!("{OP}: output exceeds {MAX_SAVE_BYTES} bytes"),
        ));
    }
    let Some(app) = session.doc.as_ref() else {
        return Err(AppError::reason(
            "no-document",
            format!("{OP}: no document open"),
        ));
    };
    check_copy(
        app,
        session.password.as_deref(),
        setting.as_ref(),
        locked,
        &bytes,
    )?;
    let count = all.len();
    Ok(reply(
        request,
        json!({
            "bytes": buffers.put(bytes),
            "marks": count,
            "pages": applied.pages,
            "flattened": flat_pages,
            "unsigned": signed,
            "sanitized": sanitized.json(),
            "kept": kept.json(),
            "verified": {"text": count, "pixels": count, "raw": checked.searches},
        }),
    ))
}
