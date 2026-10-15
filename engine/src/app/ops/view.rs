// SPDX-License-Identifier: AGPL-3.0-or-later
//! Read ops on the current view (engine.ts): render, thumbnail, search and
//! text-all, drawn and extracted by hayro, and the layers panel (layers,
//! layer-set), whose visibility the view honours. Errors are plain except
//! text-all's copy and size refusals.
//!
//! Timing fields (`paintMs`, `renderMs`, `searchMs`) are 0 here:
//! `std::time::Instant` panics on wasm32-unknown-unknown, so the TypeScript
//! adapter measures the dispatch and fills them in.
use serde_json::{Value, json};

use crate::app::info;
use crate::app::request::{self, js_number, js_number_value, js_string};
use crate::app::{AppDocument, AppError, AppSession, Buffers, MAX_SAVE_BYTES, reply};
use crate::view::{RenderOptions, ViewDocument, ViewError};

/// engine.ts `MIN_SCALE` and `MAX_SCALE`: a page render's scale range.
pub const MIN_SCALE: f64 = 0.25;
pub const MAX_SCALE: f64 = 4.0;
/// engine.ts `MAX_PIXEL_EDGE`.
pub const MAX_PIXEL_EDGE: u32 = 4096;
/// engine.ts `THUMB_MIN_EDGE` and `THUMB_MAX_EDGE`.
pub const THUMB_MIN_EDGE: i64 = 32;
pub const THUMB_MAX_EDGE: i64 = 256;
/// engine.ts `MAX_QUERY_CHARS`, in UTF-16 code units.
pub const MAX_QUERY_CHARS: usize = 256;
/// engine.ts `MAX_HITS_DEFAULT` and `MAX_HITS_CAP`.
pub const MAX_HITS_DEFAULT: f64 = 500.0;
pub const MAX_HITS_CAP: f64 = 2000.0;

fn plain(message: impl Into<String>) -> AppError {
    AppError::plain(message)
}

fn failed(op: &str) -> impl Fn(ViewError) -> AppError + '_ {
    move |error| plain(format!("{op}: {error}"))
}

/// The open document, refused as engine.ts does while it is locked.
fn unlocked<'s>(session: &'s mut AppSession, op: &str) -> Result<&'s mut AppDocument, AppError> {
    let locked = session.locked;
    let Some(doc) = session.doc.as_mut() else {
        return Err(plain(format!("{op}: no document open")));
    };
    if locked {
        return Err(plain(format!(
            "{op}: document is encrypted, a password is required"
        )));
    }
    Ok(doc)
}

/// `Number.isInteger(request.page)`, before anything else is checked.
fn page_number(request: &Value, op: &str) -> Result<i64, AppError> {
    (request.get("page").and_then(request::as_int))
        .ok_or_else(|| plain(format!("{op}: page must be an integer")))
}

/// The page index when it is below the view's page count.
fn in_range(request: &Value, page: i64, view: &ViewDocument, op: &str) -> Result<usize, AppError> {
    let count = view.page_count();
    usize::try_from(page)
        .ok()
        .filter(|&page| page < count)
        .ok_or_else(|| {
            let last = count
                .checked_sub(1)
                .map_or("-1".into(), |last| last.to_string());
            plain(format!(
                "{op}: page {} is out of range (0..{last})",
                js_string(request.get("page"))
            ))
        })
}

fn edge_limit(op: &str, width: f64, height: f64) -> AppError {
    plain(format!(
        "{op}: {}×{} px is over the {MAX_PIXEL_EDGE} px edge limit",
        js_number(width),
        js_number(height)
    ))
}

/// engine.ts `countNonWhite`.
fn non_white(rgba: &[u8]) -> usize {
    (rgba.chunks_exact(4))
        .filter(|px| px[0] != 255 || px[1] != 255 || px[2] != 255)
        .count()
}

/// engine.ts `render`: the page at `scale` with annotations (MuPDF
/// `toPixmap` with View usage), its preserve-spans text and lines.
pub fn render(
    session: &mut AppSession,
    request: &Value,
    buffers: &mut Buffers,
) -> Result<Value, AppError> {
    let revision = session.revision;
    let doc = unlocked(session, "render")?;
    let page = page_number(request, "render")?;
    let scale = request::number(request, "scale")
        .filter(|scale| (MIN_SCALE..=MAX_SCALE).contains(scale))
        .ok_or_else(|| {
            plain(format!(
                "render: scale must be between {} and {}",
                js_number(MIN_SCALE),
                js_number(MAX_SCALE)
            ))
        })?;
    let view = doc.view().map_err(failed("render"))?;
    let index = in_range(request, page, view, "render")?;
    let (width, height) = view.page_size(index).map_err(failed("render"))?;
    let wanted = ((width * scale).ceil(), (height * scale).ceil());
    let edge = f64::from(MAX_PIXEL_EDGE);
    if wanted.0 > edge || wanted.1 > edge {
        return Err(edge_limit("render", wanted.0, wanted.1));
    }
    let image = (view.render(index, scale, RenderOptions::default())).map_err(failed("render"))?;
    if image.width > MAX_PIXEL_EDGE || image.height > MAX_PIXEL_EDGE {
        return Err(edge_limit(
            "render",
            f64::from(image.width),
            f64::from(image.height),
        ));
    }
    let text = view.structured_text(index).map_err(failed("render"))?;
    let lines: Vec<Value> = (text.line_summaries().into_iter())
        .map(|line| {
            json!({"text": line.text, "x": line.x, "y": line.y, "w": line.w, "h": line.h,
                "size": line.size, "family": line.family, "weight": line.weight,
                "style": line.style})
        })
        .collect();
    let non_white = non_white(&image.pixels);
    let fields = json!({
        "page": request.get("page"),
        "revision": revision,
        "width": width,
        "height": height,
        "pixelWidth": image.width,
        "pixelHeight": image.height,
        "pixels": buffers.put(image.pixels),
        "text": text.spans_text(),
        "lines": lines,
        "nonWhite": non_white,
        "paintMs": 0,
        "renderMs": 0,
    });
    Ok(reply(request, fields))
}

/// engine.ts `thumbnail`: pixels only, the long edge pinned to `maxEdge`;
/// `MIN_SCALE` does not apply.
pub fn thumbnail(
    session: &mut AppSession,
    request: &Value,
    buffers: &mut Buffers,
) -> Result<Value, AppError> {
    let revision = session.revision;
    let doc = unlocked(session, "thumbnail")?;
    let page = page_number(request, "thumbnail")?;
    let max_edge = (request.get("maxEdge").and_then(request::as_int))
        .filter(|edge| (THUMB_MIN_EDGE..=THUMB_MAX_EDGE).contains(edge))
        .ok_or_else(|| {
            plain(format!(
                "thumbnail: maxEdge must be an integer between {THUMB_MIN_EDGE} and {THUMB_MAX_EDGE}"
            ))
        })?;
    let view = doc.view().map_err(failed("thumbnail"))?;
    let index = in_range(request, page, view, "thumbnail")?;
    let (width, height) = view.page_size(index).map_err(failed("thumbnail"))?;
    if width.is_nan() || height.is_nan() || width <= 0.0 || height <= 0.0 {
        return Err(plain("thumbnail: page has no area"));
    }
    let max_edge = max_edge as f64;
    let scale = (max_edge / width).min(max_edge / height).min(MAX_SCALE);
    let image =
        (view.render(index, scale, RenderOptions::default())).map_err(failed("thumbnail"))?;
    if image.width > MAX_PIXEL_EDGE || image.height > MAX_PIXEL_EDGE {
        return Err(edge_limit(
            "thumbnail",
            f64::from(image.width),
            f64::from(image.height),
        ));
    }
    let fields = json!({
        "page": request.get("page"),
        "revision": revision,
        "width": width,
        "height": height,
        "pixelWidth": image.width,
        "pixelHeight": image.height,
        "pixels": buffers.put(image.pixels),
    });
    Ok(reply(request, fields))
}

/// JavaScript `String.prototype.trim`: WhiteSpace and LineTerminator.
fn js_trim(text: &str) -> &str {
    text.trim_matches(|c: char| {
        matches!(c, '\u{9}'..='\u{D}' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'..='\u{200A}'
            | '\u{2028}' | '\u{2029}' | '\u{202F}' | '\u{205F}' | '\u{3000}' | '\u{FEFF}')
    })
}

/// engine.ts `hitBudget`: a finite `maxHits` truncated, else 500, within 1..2000.
fn hit_budget(max_hits: Option<&Value>) -> usize {
    let asked = max_hits
        .and_then(request::as_number)
        .map_or(MAX_HITS_DEFAULT, f64::trunc);
    asked.clamp(1.0, MAX_HITS_CAP) as usize
}

/// engine.ts `search`: case-sensitive hits in page order, at most the hit
/// budget; `truncated` when a further match exists. A page has no 500-quad
/// cap (MuPDF's silently under-counts), and a page whose content is refused
/// as too complex has no hits.
pub fn search(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    let locked = session.locked;
    let Some(doc) = session.doc.as_mut() else {
        return Err(plain("search: no document open"));
    };
    let query = js_trim(request::string(request, "query").unwrap_or(""));
    if !(1..=MAX_QUERY_CHARS).contains(&query.encode_utf16().count()) {
        return Err(plain(format!(
            "search: query must be 1..{MAX_QUERY_CHARS} characters"
        )));
    }
    if locked {
        return Err(plain(
            "search: document is encrypted, a password is required",
        ));
    }
    let budget = hit_budget(request.get("maxHits"));
    let view = doc.view().map_err(failed("search"))?;
    let reader = view.text_reader();
    let mut hits = Vec::new();
    let mut truncated = false;
    for index in 0..view.page_count() {
        let Ok(text) = reader.structured_text(index) else {
            continue;
        };
        let found = text.search_hits(query, budget.saturating_sub(hits.len()));
        for quads in found.hits {
            let quads: Vec<Vec<Value>> = (quads.iter())
                .map(|quad| quad.map(|v| js_number_value(f64::from(v as f32))).to_vec())
                .collect();
            hits.push(json!({"page": index, "quads": quads}));
        }
        if found.truncated {
            truncated = true;
            break;
        }
    }
    let total = hits.len();
    Ok(reply(
        request,
        json!({"hits": hits, "total": total, "truncated": truncated, "searchMs": 0}),
    ))
}

/// engine.ts `text-all`: every page's preserve-whitespace text, trimmed and
/// joined by form feeds, when the document permits copying; refused once the
/// text passes the save ceiling. A page refused as too complex adds "".
pub fn text_all(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    let doc = unlocked(session, "text-all")?;
    if !info::permissions(doc.document()).copy {
        return Err(AppError::reason(
            "read-only",
            "text-all: this document does not permit copying",
        ));
    }
    let view = doc.view().map_err(failed("text-all"))?;
    let reader = view.text_reader();
    let mut parts = Vec::new();
    let mut bytes = 0usize;
    for index in 0..view.page_count() {
        let text = (reader.structured_text(index))
            .map(|text| js_trim(&text.as_text()).to_string())
            .unwrap_or_default();
        bytes = bytes.saturating_add(text.len());
        parts.push(text);
        if bytes > MAX_SAVE_BYTES {
            return Err(AppError::reason(
                "too-large",
                format!("text-all: the text exceeds the {MAX_SAVE_BYTES} byte ceiling"),
            ));
        }
    }
    Ok(reply(request, json!({"text": parts.join("\u{c}")})))
}

/// engine.ts `listLayers`: a group without a name is "layer N".
fn layer_items(doc: &AppDocument) -> Value {
    let items = (doc.layers().into_iter().enumerate()).map(|(index, layer)| {
        let name = match layer.name {
            name if name.is_empty() => format!("layer {}", index + 1),
            name => name,
        };
        json!({"id": index, "name": name, "visible": layer.visible})
    });
    Value::Array(items.collect())
}

/// engine.ts `layers`: the optional-content groups and their visibility.
pub fn layers(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    let doc = unlocked(session, "layers")?;
    Ok(reply(request, json!({"items": layer_items(doc)})))
}

/// engine.ts `layer-set`: shows or hides a group in the view only, as
/// MuPDF `setLayerVisible`: no journal step, no revision, nothing saved, but
/// render, thumbnail, search and text-all honour it.
pub fn layer_set(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    let doc = unlocked(session, "layer-set")?;
    let layers = doc.layers();
    let layer = (request.get("layerId").and_then(request::as_int))
        .and_then(|layer| usize::try_from(layer).ok())
        .and_then(|layer| layers.get(layer))
        .ok_or_else(|| {
            plain(format!(
                "layer-set: layer must be an integer in 0..{}",
                layers.len().saturating_sub(1)
            ))
        })?;
    let Some(visible) = request::boolean(request, "visible") else {
        return Err(plain("layer-set: visible must be true or false"));
    };
    doc.set_layer_visible(layer.ocg, visible);
    Ok(reply(
        request,
        json!({"persisted": false, "items": layer_items(doc)}),
    ))
}
