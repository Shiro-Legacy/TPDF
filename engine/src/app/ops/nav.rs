// SPDX-License-Identifier: AGPL-3.0-or-later
//! Bookmarks and links: outline, outline-add, outline-set, outline-delete,
//! links, link-add, link-delete and link-set (engine.ts).
//!
//! The step names (`add bookmark`, `rename bookmark`, `delete bookmark`,
//! `add link`, `delete link`, `edit link`) are what undo and redo report.
pub mod links;
pub mod outline;
pub mod target;

use serde_json::{Map, Value};

use crate::app::{AppDocument, AppError, AppSession, info, reply, request};
use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId};

use links::{Link, Listing, NewTarget};
use outline::Outline;
use target::{CLIPPED, Resolver, Target};

/// engine.ts `MAX_BOOKMARK_CHARS`, in UTF-16 code units.
pub const MAX_BOOKMARK_CHARS: usize = 4096;
/// engine.ts `MAX_URI_CHARS`, in UTF-16 code units.
pub const MAX_URI_CHARS: usize = 2048;
/// The `outline` and `links` replies list at most this many entries: a few
/// MB of object streams can hold two million items, whose JSON reply would
/// exhaust the WASM heap. Outline ids past it still name items for the
/// edit ops.
pub const MAX_LISTED_ITEMS: usize = 100_000;
/// The `outline` and `links` replies also stop once their titles and URIs
/// pass this many bytes of JSON: one string can be shared by every entry.
pub const MAX_LISTED_BYTES: usize = 16 * 1024 * 1024;
/// engine.ts `LINK_SCHEMES`: the only schemes the app creates or follows.
pub const LINK_SCHEMES: [&str; 3] = ["http:", "https:", "mailto:"];

fn plain(message: impl Into<String>) -> AppError {
    AppError::plain(message)
}

/// engine-core.ts `failLink`: `invalid` with a detail token.
fn fail_link(detail: &str, message: impl Into<String>) -> AppError {
    AppError::detail("invalid", detail, message)
}

/// The bytes `text` takes as a JSON string's contents.
fn json_len(text: &str) -> usize {
    text.chars()
        .map(|c| match c {
            '"' | '\\' | '\u{8}' | '\u{C}' | '\n' | '\r' | '\t' => 2,
            c if c < ' ' => 6,
            c => c.len_utf8(),
        })
        .fold(0, usize::saturating_add)
}

/// A listing reply: `fields`, with `truncated` when the list stopped early.
fn listing_reply(request: &Value, mut fields: Map<String, Value>, truncated: bool) -> Value {
    if truncated {
        fields.insert("truncated".into(), true.into());
    }
    reply(request, Value::Object(fields))
}

/// JavaScript `string.length`.
fn js_length(text: &str) -> usize {
    text.encode_utf16().count()
}

/// JavaScript `String.prototype.trim` whitespace.
fn js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{B}' | '\u{C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

/// engine.ts `schemeOf`: the scheme with its colon, lowercased; "" when
/// the URI has none.
pub fn scheme_of(uri: &str) -> String {
    let scheme = uri.split_once(':').map_or("", |(scheme, _)| scheme);
    let mut chars = scheme.chars();
    let valid = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    if valid {
        format!("{}:", scheme.to_ascii_lowercase())
    } else {
        String::new()
    }
}

fn edit_reply(session: &AppSession, request: &Value) -> Value {
    reply(request, Value::Object(session.edit_state()))
}

/// The open document for a read op, with engine.ts's plain messages.
fn readable<'s>(session: &'s AppSession, op: &str) -> Result<&'s AppDocument, AppError> {
    let Some(doc) = &session.doc else {
        return Err(plain(format!("{op}: no document open")));
    };
    if session.locked {
        return Err(plain(format!(
            "{op}: document is encrypted, a password is required"
        )));
    }
    Ok(doc)
}

/// engine.ts `checkTitle`: a string of at most 4096 characters, not blank;
/// stored as given.
fn check_title(title: Option<&Value>, op: &str) -> Result<String, AppError> {
    let Some(title) = title
        .and_then(Value::as_str)
        .filter(|title| js_length(title) <= MAX_BOOKMARK_CHARS)
    else {
        return Err(plain(format!(
            "{op}: title must be a string of at most {MAX_BOOKMARK_CHARS} characters"
        )));
    };
    if title.trim_matches(js_space).is_empty() {
        return Err(plain(format!("{op}: a bookmark needs a title")));
    }
    Ok(title.to_string())
}

/// engine.ts `outlineCursor`: the preorder id of an item that exists.
fn check_outline_id(
    session: &AppSession,
    value: Option<&Value>,
    op: &str,
) -> Result<usize, AppError> {
    let Some(id) = value.and_then(request::as_int).filter(|&id| id >= 0) else {
        return Err(plain(format!("{op}: id must be a non-negative integer")));
    };
    let count = (session.doc.as_ref()).map_or(0, |doc| Outline::read(doc.document()).len());
    usize::try_from(id)
        .ok()
        .filter(|&id| id < count)
        .ok_or_else(|| plain(format!("{op}: no bookmark {}", request::js_string(value))))
}

/// An outline item's title, `""` when it has none; a clipped one ends with
/// `CLIPPED`.
fn title_of(doc: &Document<'_>, item: &Dictionary) -> String {
    match doc.dict_get(item, b"Title").as_string() {
        Some(bytes) => match target::decode_text(bytes) {
            (title, true) => title + &CLIPPED.to_string(),
            (title, false) => title,
        },
        None => String::new(),
    }
}

/// engine.ts `outline`: every bookmark in preorder, `{id, title, depth,
/// page}`, with `uri` when the destination is not a page here; at most
/// `MAX_LISTED_ITEMS` of them and `MAX_LISTED_BYTES` of titles and URIs,
/// with `truncated` when there were more.
pub fn list_outline(session: &AppSession, request: &Value) -> Result<Value, AppError> {
    let doc = readable(session, "outline")?.document();
    let outline = Outline::read_at_most(doc, MAX_LISTED_ITEMS);
    let resolver = Resolver::new(doc);
    let mut truncated = outline.truncated;
    let mut bytes = 0usize;
    let mut items = Vec::with_capacity(outline.len());
    for (id, item) in outline.items.iter().enumerate() {
        let dict = item.place.read(doc).as_dict();
        let title = dict.map(|dict| title_of(doc, dict)).unwrap_or_default();
        let target = dict.and_then(|dict| resolver.outline(dict));
        let uri = target.as_ref().and_then(Target::uri);
        bytes = (bytes.saturating_add(json_len(&title))).saturating_add(uri.map_or(0, json_len));
        if bytes > MAX_LISTED_BYTES {
            truncated = true;
            break;
        }
        let mut entry = Map::new();
        entry.insert("id".into(), id.into());
        entry.insert("title".into(), title.into());
        entry.insert("depth".into(), item.depth.into());
        match (&target, uri) {
            (Some(Target::Page(page)), _) => entry.insert("page".into(), (*page).into()),
            (_, Some(uri)) => {
                entry.insert("page".into(), Value::Null);
                entry.insert("uri".into(), uri.into())
            }
            _ => entry.insert("page".into(), Value::Null),
        };
        items.push(Value::Object(entry));
    }
    let mut fields = Map::new();
    fields.insert("items".into(), items.into());
    Ok(listing_reply(request, fields, truncated))
}

/// `[page /Fit]`.
fn fit(page: ObjectId) -> Object {
    Object::Array(vec![Object::Reference(page), Object::name("Fit")])
}

/// engine.ts `outline-add`: a top-level bookmark after the last, going to
/// the whole page.
pub fn outline_add(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    session.require_mutable("outline-add")?;
    let page = session.check_page(request.get("page"), "outline-add")?;
    let title = check_title(request.get("title"), "outline-add")?;
    session.mutate("add bookmark", |session| {
        let mut tx = session.tx()?;
        let page = links::page_object(&mut tx, page)?;
        outline::append(&mut tx, &title, fit(page))?;
        Ok(())
    })?;
    Ok(edit_reply(session, request))
}

/// engine.ts `outline-set`: renames a bookmark. Only /Title is written;
/// MuPDF's iterator update also rewrote the destination and dropped the
/// style (/C, /F) and any action that was not a link.
pub fn outline_set(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    session.require_mutable("outline-set")?;
    let title = check_title(request.get("title"), "outline-set")?;
    let index = check_outline_id(session, request.get("outlineId"), "outline-set")?;
    session.mutate("rename bookmark", |session| {
        let mut tx = session.tx()?;
        outline::rename(&mut tx, index, &title)?;
        Ok(())
    })?;
    Ok(edit_reply(session, request))
}

/// engine.ts `outline-delete`: the bookmark and its subtree.
pub fn outline_delete(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    session.require_mutable("outline-delete")?;
    let index = check_outline_id(session, request.get("outlineId"), "outline-delete")?;
    session.mutate("delete bookmark", |session| {
        let mut tx = session.tx()?;
        outline::delete(&mut tx, index)?;
        Ok(())
    })?;
    Ok(edit_reply(session, request))
}

/// A listed link as the panel reads it: `{id, annotId, rect, kind,
/// page|uri}`, a URI `uri` only on an allowed scheme, `blocked` otherwise
/// (and when clipped).
pub fn link_json(id: usize, link: &Link) -> Value {
    // `+ 0.0` turns -0 into 0, which strict equality in the app tells apart.
    let rect: Vec<Value> = (link.rect.iter())
        .map(|&value| request::js_number_value(value + 0.0))
        .collect();
    let mut entry = Map::new();
    entry.insert("id".into(), id.into());
    entry.insert("annotId".into(), link.annot_id.into());
    entry.insert("rect".into(), rect.into());
    match &link.target {
        Target::Page(page) => {
            entry.insert("kind".into(), "page".into());
            entry.insert("page".into(), (*page).into());
        }
        Target::Uri(uri) => {
            let allowed = LINK_SCHEMES.contains(&scheme_of(uri).as_str());
            let kind = if allowed { "uri" } else { "blocked" };
            entry.insert("kind".into(), kind.into());
            entry.insert("uri".into(), uri.clone().into());
        }
        Target::Clipped(uri) => {
            entry.insert("kind".into(), "blocked".into());
            entry.insert("uri".into(), uri.clone().into());
        }
    }
    Value::Object(entry)
}

/// The page's links as the `links` reply lists them.
pub fn page_links(session: &AppSession, page: usize) -> Listing {
    match &session.doc {
        Some(doc) => {
            let doc = doc.document();
            links::list(doc, &Resolver::new(doc), page)
        }
        None => Listing::default(),
    }
}

/// engine.ts `links`: the page's links in /Annots order, with `truncated`
/// when `links::list` stopped early.
pub fn list_links(session: &AppSession, request: &Value) -> Result<Value, AppError> {
    readable(session, "links")?;
    let page = session.check_page(request.get("page"), "links")?;
    let listing = page_links(session, page);
    let items: Vec<Value> = (listing.links.iter().enumerate())
        .map(|(id, link)| link_json(id, link))
        .collect();
    let mut fields = Map::new();
    fields.insert("page".into(), page.into());
    fields.insert("items".into(), items.into());
    Ok(listing_reply(request, fields, listing.truncated))
}

/// engine.ts `pageBox`: the page box links are checked against, MuPDF's
/// page bounds [0, 0, width, height]; zero when there is no such page.
pub fn page_box(session: &AppSession, page: usize) -> (f64, f64) {
    (session.doc.as_ref())
        .and_then(|doc| {
            let (_, dict) = doc.page(page)?;
            Some(info::page_size(doc.document(), dict))
        })
        .unwrap_or((0.0, 0.0))
}

/// engine.ts `checkLinkRect`: four finite numbers, normalized, with a
/// positive area, both corners on the page box `size` (`page_box`), edges
/// included. clip-paste checks pasted link rects with it as `link-add`.
pub fn check_link_rect(
    size: (f64, f64),
    rect: Option<&Value>,
    op: &str,
) -> Result<[f64; 4], AppError> {
    let numbers: Option<Vec<f64>> = (rect.and_then(Value::as_array))
        .filter(|items| items.len() == 4)
        .and_then(|items| items.iter().map(request::as_number).collect());
    let Some([x0, y0, x1, y1]) = numbers.and_then(|numbers| <[f64; 4]>::try_from(numbers).ok())
    else {
        return Err(plain(format!("{op}: rect must be 4 finite numbers")));
    };
    let (left, right) = (x0.min(x1), x0.max(x1));
    let (top, bottom) = (y0.min(y1), y0.max(y1));
    if right - left <= 0.0 || bottom - top <= 0.0 {
        return Err(plain(format!("{op}: rect must have a positive area")));
    }
    let (width, height) = size;
    let on_page = |x: f64, y: f64| x >= 0.0 && x <= width && y >= 0.0 && y <= height;
    if !on_page(left, top) || !on_page(right, bottom) {
        let n = request::js_number;
        return Err(plain(format!(
            "{op}: rect [{}, {}, {}, {}] is off the page",
            n(left),
            n(top),
            n(right),
            n(bottom)
        )));
    }
    Ok([left, top, right, bottom])
}

/// Why a link target is refused, before the op words it.
pub enum TargetError {
    /// `target.page` failed `checkPage`.
    Page(AppError),
    /// A scheme off the allowlist, or a URI over the length bound.
    Scheme(String),
    /// Neither `{page}` nor `{uri}`.
    Shape,
}

/// A link target: `{page}` (any `page` key, null included) or `{uri}` with
/// an allowed scheme and at most 2048 characters.
pub fn link_target(
    session: &AppSession,
    target: Option<&Value>,
    op: &str,
) -> Result<NewTarget, TargetError> {
    let target = target.filter(|target| target.is_object());
    if let Some(page) = target.and_then(|target| target.get("page")) {
        let page = session
            .check_page(Some(page), op)
            .map_err(TargetError::Page)?;
        return Ok(NewTarget::Page(page));
    }
    let Some(uri) = target
        .and_then(|target| target.get("uri"))
        .and_then(Value::as_str)
    else {
        return Err(TargetError::Shape);
    };
    let scheme = scheme_of(uri);
    if !LINK_SCHEMES.contains(&scheme.as_str()) {
        let scheme = if scheme.is_empty() {
            "a URI without a scheme"
        } else {
            &scheme
        };
        return Err(TargetError::Scheme(format!(
            "{op}: {scheme} is not a link this app opens"
        )));
    }
    if js_length(uri) > MAX_URI_CHARS {
        return Err(TargetError::Scheme(format!(
            "{op}: a link URI holds at most {MAX_URI_CHARS} characters"
        )));
    }
    Ok(NewTarget::Uri(uri.to_string()))
}

/// engine.ts `link-add`: a new link with a GoTo (whole page) or URI action.
pub fn link_add(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    session.require_mutable("link-add")?;
    let page = session.check_page(request.get("page"), "link-add")?;
    let rect = check_link_rect(page_box(session, page), request.get("rect"), "link-add")?;
    let to =
        link_target(session, request.get("target"), "link-add").map_err(|error| match error {
            TargetError::Page(error) => error,
            TargetError::Scheme(message) => plain(message),
            TargetError::Shape => plain("link-add: target must be {page} or {uri}"),
        })?;
    session.mutate("add link", |session| {
        let mut tx = session.tx()?;
        links::add(&mut tx, page, rect, &to)?;
        Ok(())
    })?;
    Ok(edit_reply(session, request))
}

/// engine.ts `link-delete`: the link the `links` reply listed as `linkId`.
pub fn link_delete(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    session.require_mutable("link-delete")?;
    let page = session.check_page(request.get("page"), "link-delete")?;
    let links = page_links(session, page).links;
    let value = request.get("linkId");
    let Some(link) = value
        .and_then(request::as_int)
        .and_then(|index| usize::try_from(index).ok())
        .and_then(|index| links.get(index))
    else {
        return Err(plain(format!(
            "link-delete: no link {} on this page",
            request::js_string(value)
        )));
    };
    let position = link.position;
    session.mutate("delete link", |session| {
        let mut tx = session.tx()?;
        links::remove(&mut tx, page, position)?;
        Ok(())
    })?;
    Ok(edit_reply(session, request))
}

/// engine.ts `link-set`: moves, resizes or retargets one link, named by its
/// /Annots object number. Refusals are `invalid` with one detail token, in
/// engine.ts's order; a request that changes nothing writes nothing and
/// answers `noop`.
pub fn link_set(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    session.require_mutable("link-set")?;
    let page = session.check_page(request.get("page"), "link-set")?;
    let rect_value = request.get("rect");
    let target_value = request.get("target");
    if rect_value.is_none() && target_value.is_none() {
        return Err(fail_link("target", "link-set: rect or target is required"));
    }
    let rect = match rect_value {
        Some(_) => Some(
            check_link_rect(page_box(session, page), rect_value, "link-set")
                .map_err(|error| fail_link("rect", error.message))?,
        ),
        None => None,
    };
    let wanted = request.get("annotId");
    let missing = || {
        fail_link(
            "no-such-annot",
            format!(
                "link-set: page {page} has no link {}",
                request::js_string(wanted)
            ),
        )
    };
    let number = wanted
        .and_then(request::as_int)
        .filter(|&number| number > 0)
        .ok_or_else(missing)?;
    let link = {
        let Some(doc) = &session.doc else {
            return Err(missing());
        };
        let (_, page_dict) = doc.page(page).ok_or_else(missing)?;
        let doc = doc.document();
        let entry = (doc
            .dict_get(page_dict, b"Annots")
            .as_array()
            .unwrap_or(&[])
            .iter())
        .find(|entry| {
            entry
                .as_reference()
                .is_some_and(|id| i64::from(id.number()) == number)
        })
        .ok_or_else(missing)?;
        if crate::app::edit::get(doc, entry, b"Subtype").as_name() != Some(b"Link") {
            return Err(fail_link(
                "not-link",
                format!("link-set: annotation {number} is not a link"),
            ));
        }
        (page_links(session, page).links.into_iter())
            .find(|link| i64::from(link.annot_id) == number)
            .ok_or_else(missing)?
    };
    let to = match target_value {
        Some(value) => Some(
            link_target(session, Some(value), "link-set").map_err(|error| match error {
                TargetError::Page(error) => fail_link("target", error.message),
                TargetError::Scheme(message) => fail_link("scheme", message),
                TargetError::Shape => {
                    fail_link("target", "link-set: target must be {page} or {uri}")
                }
            })?,
        ),
        None => None,
    };
    let rect_changed = rect.is_some_and(|rect| {
        !(rect
            .iter()
            .zip(link.rect)
            .all(|(new, old)| (new - old).abs() <= 0.01))
    });
    let target_changed = match &to {
        Some(NewTarget::Page(page)) => link.target != Target::Page(*page),
        Some(NewTarget::Uri(uri)) => {
            !matches!(&link.target, Target::Uri(current) if current == uri)
        }
        None => false,
    };
    if !rect_changed && !target_changed {
        let mut fields = session.journal_state().json();
        fields.insert("noop".into(), true.into());
        return Ok(reply(request, Value::Object(fields)));
    }
    let Some(holder) = link.holder else {
        return Err(missing());
    };
    let rect = rect.filter(|_| rect_changed);
    let to = to.filter(|_| target_changed);
    session.mutate("edit link", |session| {
        let mut tx = session.tx()?;
        links::set(&mut tx, page, holder, rect, to.as_ref())?;
        Ok(())
    })?;
    Ok(edit_reply(session, request))
}
