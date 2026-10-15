// SPDX-License-Identifier: AGPL-3.0-or-later
//! Annotations: annots, annot-add, annot-set, annot-move and annot-delete
//! (engine.ts), each change one journal step with the appearance stream
//! MuPDF's `update()` writes (`app::annots::appearance`).
//!
//! The step names (`add note`, `add text box`, `add <type>`, `add
//! signature`, `add drawn signature`, `edit annotation`, `move text box`,
//! `move signature`, `resize signature`, `delete annotation`) are what undo
//! and redo report.
use serde_json::{Map, Value, json};

use crate::app::annots::appearance::{self, Form, Mark, Style, number, numbers};
use crate::app::annots::fonts::{self, Base14};
use crate::app::annots::{self, Color, PageSpace, Point, Quad};
use crate::app::ops::pages::images;
use crate::app::request::{self, js_number, js_string};
use crate::app::{AppError, AppSession, Buffers, MAX_SAVE_BYTES, Need, Transaction, reply};
use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId};

/// engine.ts `NOTE_SIZE`.
pub const NOTE_SIZE: f64 = 20.0;
/// engine.ts `FREE_TEXT_SIZE`.
pub const FREE_TEXT_SIZE: [f64; 2] = [160.0, 40.0];
/// engine.ts `MAX_QUADS`.
pub const MAX_QUADS: usize = 64;
/// engine.ts `EDITABLE_TYPES`.
pub const EDITABLE_TYPES: [&str; 10] = [
    "Text",
    "Highlight",
    "Underline",
    "StrikeOut",
    "FreeText",
    "Square",
    "Circle",
    "Line",
    "Ink",
    "Stamp",
];
/// engine.ts `STAMP_ICONS`.
pub const STAMP_ICONS: [&str; 6] = [
    "Approved",
    "Draft",
    "Confidential",
    "Final",
    "NotApproved",
    "Void",
];
/// engine.ts `MAX_INK_STROKES`, `MIN_INK_POINTS`, `MAX_INK_POINTS`.
pub const MAX_INK_STROKES: usize = 64;
pub const MIN_INK_POINTS: usize = 2;
pub const MAX_INK_POINTS: usize = 2048;
/// engine.ts `MAX_STAMP_IMAGE_BYTES`.
pub const MAX_STAMP_IMAGE_BYTES: usize = 4 * 1024 * 1024;
/// engine.ts `SIGNATURE_SUBJECT` and `DRAWN_SIGNATURE_SUBJECT`.
pub const SIGNATURE_SUBJECT: &str = "Signature appearance";
pub const DRAWN_SIGNATURE_SUBJECT: &str = "Drawn signature";
/// engine.ts `MAX_SIGN_NAME_CHARS`, in UTF-16 code units.
pub const MAX_SIGN_NAME_CHARS: usize = 200;
/// engine.ts `MAX_NOTE_CHARS`, in UTF-16 code units.
pub const MAX_NOTE_CHARS: usize = 4096;
/// Where a note's popup window opens: beside the note, this size.
const POPUP_SIZE: [f64; 2] = [200.0, 100.0];
/// The FreeText font the app writes and `fit` measures.
const FIT_SIZE: f64 = 12.0;

fn plain(message: impl Into<String>) -> AppError {
    AppError::plain(message)
}

/// engine-core.ts `failEditText`.
fn edit_text(detail: &str, message: impl Into<String>, extra: Value) -> AppError {
    AppError::detail("invalid", detail, message).with_extra(extra)
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// A reply number: -0 becomes 0.
fn out(value: f64) -> Value {
    json!(value + 0.0)
}

fn rect_value(rect: Option<[f64; 4]>) -> Value {
    Value::Array(rect.map_or_else(Vec::new, |r| r.iter().map(|&v| out(v)).collect()))
}

/// engine.ts `annots`: the page's annotations as the renderer lists them.
pub fn list(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    if session.doc.is_none() {
        return Err(plain("annots: no document open"));
    }
    if session.locked {
        return Err(plain(
            "annots: document is encrypted, a password is required",
        ));
    }
    let page = session.check_page(request.get("page"), "annots")?;
    let Some(app) = &session.doc else {
        return Err(plain("annots: no document open"));
    };
    let info = app.info(session.locked);
    let annotated = !info.signed() && Need::Annotate.allowed(&info);
    let doc = app.document();
    let (_, dict) = app
        .page(page)
        .ok_or_else(|| plain("annots: no such page"))?;
    let space = PageSpace::of(doc, dict);
    let items: Vec<Value> = annots::list(doc, dict)
        .iter()
        .map(|annot| {
            let kind = String::from_utf8_lossy(annot.subtype).into_owned();
            let editable = annotated && annot.id.is_some() && EDITABLE_TYPES.contains(&&*kind);
            let mut item = Map::new();
            item.insert("id".into(), annot.number().into());
            item.insert("type".into(), kind.clone().into());
            let rect = annots::page_rect(doc, &space, annot.dict);
            item.insert("rect".into(), rect_value(rect));
            item.insert(
                "contents".into(),
                annots::text(doc, annot.dict, b"Contents").into(),
            );
            item.insert(
                "subject".into(),
                annots::text(doc, annot.dict, b"Subj").into(),
            );
            item.insert("editable".into(), editable.into());
            if kind == "Redact" && searched(doc, annot.dict) {
                item.insert("search".into(), true.into());
            }
            Value::Object(item)
        })
        .collect();
    Ok(reply(request, json!({"page": page, "items": items})))
}

/// A Redact mark still carrying search quads or the search tag.
fn searched(doc: &Document<'_>, annot: &Dictionary) -> bool {
    !annots::quads(doc, annot).is_empty() || annot.contains_key(b"PDF4AllSearch".as_slice())
}

/// engine.ts `checkContents`.
fn check_contents<'a>(request: &'a Value, op: &str) -> Result<&'a str, AppError> {
    request
        .get("contents")
        .and_then(Value::as_str)
        .filter(|text| utf16_len(text) <= MAX_NOTE_CHARS)
        .ok_or_else(|| {
            plain(format!(
                "{op}: contents must be a string of at most {MAX_NOTE_CHARS} characters"
            ))
        })
}

/// engine.ts `finiteNumbers`.
fn finite_numbers(
    value: Option<&Value>,
    count: usize,
    label: &str,
    op: &str,
) -> Result<Vec<f64>, AppError> {
    let refuse = || plain(format!("{op}: {label} must be {count} finite numbers"));
    let items = value
        .and_then(Value::as_array)
        .filter(|items| items.len() == count)
        .ok_or_else(refuse)?;
    items
        .iter()
        .map(|item| request::as_number(item).ok_or_else(refuse))
        .collect()
}

fn on_page(page: &[f64; 4], x: f64, y: f64) -> bool {
    x >= page[0] && x <= page[2] && y >= page[1] && y <= page[3]
}

/// engine.ts `checkPoint`: on the page, refused and never clamped.
fn check_point(page: &[f64; 4], value: Option<&Value>, op: &str) -> Result<Point, AppError> {
    let point = finite_numbers(value, 2, "point", op)?;
    let (x, y) = (point[0], point[1]);
    if !on_page(page, x, y) {
        return Err(plain(format!(
            "{op}: point [{}, {}] is off the page",
            js_number(x),
            js_number(y)
        )));
    }
    Ok((x, y))
}

/// engine.ts `checkQuads`: 1..64 quads of on-page corners.
fn check_quads(page: &[f64; 4], value: Option<&Value>, op: &str) -> Result<Vec<Quad>, AppError> {
    let quads = value
        .and_then(Value::as_array)
        .filter(|quads| (1..=MAX_QUADS).contains(&quads.len()))
        .ok_or_else(|| plain(format!("{op}: quads must be 1..{MAX_QUADS} quads")))?;
    quads
        .iter()
        .enumerate()
        .map(|(index, quad)| {
            let c = finite_numbers(Some(quad), 8, &format!("quad {index}"), op)?;
            let quad = [(c[0], c[1]), (c[2], c[3]), (c[4], c[5]), (c[6], c[7])];
            if quad.iter().any(|&(x, y)| !on_page(page, x, y)) {
                return Err(plain(format!("{op}: quad {index} is off the page")));
            }
            Ok(quad)
        })
        .collect()
}

/// engine.ts `checkRect`: positive area, both corners on the page.
fn check_rect(page: &[f64; 4], value: Option<&Value>, op: &str) -> Result<[f64; 4], AppError> {
    let r = finite_numbers(value, 4, "rect", op)?;
    let [x0, y0, x1, y1] = [r[0], r[1], r[2], r[3]];
    // Finite numbers: no NaN to compare.
    if x1 <= x0 || y1 <= y0 {
        return Err(plain(format!("{op}: rect must have a positive area")));
    }
    if !on_page(page, x0, y0) || !on_page(page, x1, y1) {
        let text: Vec<String> = [x0, y0, x1, y1].iter().map(|&v| js_number(v)).collect();
        return Err(plain(format!(
            "{op}: rect [{}] is off the page",
            text.join(", ")
        )));
    }
    Ok([x0, y0, x1, y1])
}

/// engine.ts `checkLine`: two distinct on-page points.
fn check_line(page: &[f64; 4], value: Option<&Value>, op: &str) -> Result<[Point; 2], AppError> {
    let points = value
        .and_then(Value::as_array)
        .filter(|points| points.len() == 2)
        .ok_or_else(|| plain(format!("{op}: a Line takes exactly two points")))?;
    let start = check_point(page, points.first(), op)?;
    let end = check_point(page, points.get(1), op)?;
    if start == end {
        return Err(plain(format!("{op}: a Line needs two distinct points")));
    }
    Ok([start, end])
}

/// engine.ts `checkStrokes`: 1..64 strokes of 2..2048 on-page points.
fn check_strokes(
    page: &[f64; 4],
    value: Option<&Value>,
    op: &str,
) -> Result<Vec<Vec<Point>>, AppError> {
    let strokes = value
        .and_then(Value::as_array)
        .filter(|strokes| (1..=MAX_INK_STROKES).contains(&strokes.len()))
        .ok_or_else(|| {
            plain(format!(
                "{op}: strokes must be 1..{MAX_INK_STROKES} strokes"
            ))
        })?;
    strokes
        .iter()
        .enumerate()
        .map(|(index, stroke)| {
            let points = stroke
                .as_array()
                .filter(|points| (MIN_INK_POINTS..=MAX_INK_POINTS).contains(&points.len()))
                .ok_or_else(|| {
                    plain(format!(
                        "{op}: stroke {index} must be {MIN_INK_POINTS}..{MAX_INK_POINTS} points"
                    ))
                })?;
            points
                .iter()
                .enumerate()
                .map(|(position, point)| {
                    check_point(
                        page,
                        Some(point),
                        &format!("{op}: stroke {index} point {position}"),
                    )
                })
                .collect()
        })
        .collect()
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

/// engine.ts `checkSignName`.
fn check_sign_name(value: Option<&Value>, op: &str) -> Result<String, AppError> {
    value
        .and_then(Value::as_str)
        .filter(|name| !name.trim_matches(js_space).is_empty())
        .filter(|name| utf16_len(name) <= MAX_SIGN_NAME_CHARS)
        .map(str::to_string)
        .ok_or_else(|| {
            plain(format!(
                "{op}: a signature name must be 1..{MAX_SIGN_NAME_CHARS} characters"
            ))
        })
}

/// A decoded stamp image (engine.ts `decodeStampImage`).
struct StampImage {
    placed: images::Placed,
}

fn decode_stamp_image(
    value: Option<&Value>,
    buffers: &Buffers,
    op: &str,
) -> Result<StampImage, AppError> {
    let name = value
        .and_then(|image| image.get("name"))
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty() && utf16_len(name) <= 255)
        .ok_or_else(|| {
            plain(format!(
                "{op}: a stamp image needs a name of at most 255 characters"
            ))
        })?;
    let bytes = buffers
        .input(value.and_then(|image| image.get("bytes")))
        .filter(|bytes| !bytes.is_empty())
        .ok_or_else(|| plain(format!("{op}: stamp image {name} has no bytes")))?;
    if bytes.len() > MAX_STAMP_IMAGE_BYTES {
        return Err(plain(format!(
            "{op}: {name} is over the {MAX_STAMP_IMAGE_BYTES} byte image ceiling"
        )));
    }
    let placed = match images::read(bytes, MAX_SAVE_BYTES) {
        Ok(placed) => placed,
        Err(images::ImageError::Unreadable(reason)) => {
            return Err(plain(format!(
                "{op}: {name} is not a PNG or JPEG the app can read ({reason})"
            )));
        }
        Err(images::ImageError::TooLarge) => {
            return Err(plain(format!("{op}: {name} is too large to store")));
        }
    };
    if placed.width == 0 || placed.height == 0 {
        return Err(plain(format!("{op}: {name} has no pixels")));
    }
    Ok(StampImage { placed })
}

/// engine.ts `signatureContents`: `Signed by <name> on <YYYY-MM-DD>`, the
/// date in the worker's local time, which the adapter sends as `today`
/// (UTC from the clock when it does not).
fn signature_contents(name: &str, today: Option<&str>) -> String {
    let date = today
        .filter(|date| date.len() == 10 && date.bytes().all(|b| b.is_ascii_digit() || b == b'-'))
        .map_or_else(utc_today, str::to_string);
    format!("Signed by {name} on {date}")
}

#[cfg(not(target_arch = "wasm32"))]
fn utc_today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    civil_date(i64::try_from(secs / 86_400).unwrap_or(0))
}

#[cfg(target_arch = "wasm32")]
fn utc_today() -> String {
    civil_date(0)
}

/// The proleptic Gregorian date `days` after 1970-01-01.
fn civil_date(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// engine.ts `freeTextBox`: the fixed 160 x 40 box, its top left moved
/// inside the page.
fn free_text_box(page: &[f64; 4], point: Point) -> [f64; 4] {
    let [width, height] = FREE_TEXT_SIZE;
    let x = page[0].max(point.0.min(page[2] - width));
    let y = page[1].max(point.1.min(page[3] - height));
    [x, y, x + width, y + height]
}

/// The Helvetica 12 advance of a line, as engine.ts `editAdvance` measures it.
fn fit_advance(line: &str) -> f64 {
    line.chars()
        .filter_map(fonts::win_ansi)
        .map(|byte| Base14::Helvetica.advance(byte) / 1000.0 * FIT_SIZE)
        .sum()
}

/// engine.ts `freeTextFit`: a box that shows every line unwrapped; `point`
/// is the left edge and the first line's vertical centre.
fn free_text_fit(page: &[f64; 4], point: Point, contents: &str) -> Result<[f64; 4], AppError> {
    let lh = appearance::line_height(FIT_SIZE);
    let lines: Vec<&str> = contents.split('\n').collect();
    let width = lines
        .iter()
        .map(|line| fit_advance(line))
        .fold(1.0, f64::max)
        + 0.5;
    let height = lines.len() as f64 * lh;
    if width > page[2] - page[0] || height > page[3] - page[1] {
        return Err(edit_text(
            "overflow",
            "The text does not fit on the page; use shorter lines",
            Value::Null,
        ));
    }
    let x = page[0].max(point.0.min(page[2] - width));
    let y = page[1].max((point.1 - lh / 2.0).min(page[3] - height));
    Ok([x, y, x + width, y + height])
}

/// engine.ts `checkFreeTextEncoding`: every character but a line feed must
/// be WinAnsi; the first 20 that are not are listed.
fn check_free_text_encoding(contents: &str, op: &str) -> Result<(), AppError> {
    let mut missing: Vec<String> = Vec::new();
    for c in contents.chars() {
        if c == '\n' || fonts::win_ansi(c).is_some() {
            continue;
        }
        let text = c.to_string();
        if !missing.contains(&text) {
            missing.push(text);
        }
    }
    if missing.is_empty() {
        return Ok(());
    }
    missing.truncate(20);
    Err(edit_text(
        "unsupported-encoding",
        format!("{op}: unsupported WinAnsi characters"),
        json!({ "missing": missing }),
    ))
}

/// engine.ts `signatureLineWidth`.
fn signature_line_width(width: f64) -> f64 {
    (width * 0.0075).clamp(0.6, 4.0)
}

/// A /DA's font name, size and colour (MuPDF's
/// `pdf_parse_default_appearance`; Helv 12 black when it says nothing).
#[derive(Clone, Debug, PartialEq)]
pub struct DefaultAppearance {
    pub font: Vec<u8>,
    pub size: f64,
    pub color: Option<Color>,
}

pub fn default_appearance(doc: &Document<'_>, annot: &Dictionary) -> DefaultAppearance {
    let mut da = DefaultAppearance {
        font: b"Helv".to_vec(),
        size: 12.0,
        color: Some(Color::Gray(0.0)),
    };
    let Some(text) = doc.dict_get(annot, b"DA").as_string() else {
        return da;
    };
    let mut stack: Vec<&[u8]> = Vec::new();
    for token in text
        .split(|b| b.is_ascii_whitespace())
        .filter(|t| !t.is_empty())
        .take(10_000)
    {
        let numbers = |stack: &[&[u8]], n: usize| -> Option<Vec<f64>> {
            let start = stack.len().checked_sub(n)?;
            stack[start..]
                .iter()
                .map(|t| std::str::from_utf8(t).ok()?.parse::<f64>().ok())
                .collect()
        };
        match token {
            b"Tf" => {
                let size = numbers(&stack, 1).and_then(|v| v.first().copied());
                let name = stack.len().checked_sub(2).and_then(|i| stack.get(i));
                if let (Some(size), Some(name)) = (size, name.and_then(|n| n.strip_prefix(b"/"))) {
                    da.font = name.to_vec();
                    da.size = size;
                }
                stack.clear();
            }
            b"g" => {
                da.color = numbers(&stack, 1).map(|v| Color::Gray(v[0]));
                stack.clear();
            }
            b"rg" => {
                da.color = numbers(&stack, 3).map(|v| Color::Rgb([v[0], v[1], v[2]]));
                stack.clear();
            }
            b"k" => {
                da.color = numbers(&stack, 4).map(|v| Color::Cmyk([v[0], v[1], v[2], v[3]]));
                stack.clear();
            }
            _ if token.starts_with(b"/")
                || token[0].is_ascii_digit()
                || b"-+.".contains(&token[0]) =>
            {
                stack.push(token);
            }
            _ => stack.clear(),
        }
    }
    da
}

/// A FreeText's appearance from its dictionary for the /Rect `rect` (user
/// space): laid out in `rect` less /RD, its /BBox `rect`.
pub fn free_text_form(doc: &Document<'_>, annot: &Dictionary, rect: [f64; 4], text: &str) -> Form {
    let da = default_appearance(doc, annot);
    let rotate = annot
        .get(b"Rotate".as_slice())
        .and_then(|v| annots::number(doc, v))
        .map_or(0, |r| ((r as i64).rem_euclid(360) / 90 * 90) as i32);
    let quadding = doc.dict_get(annot, b"Q").as_i64().unwrap_or(0);
    let layout = appearance::FreeText {
        text,
        font_name: &da.font,
        font: Base14::from_resource_name(&da.font),
        size: if da.size > 0.0 { da.size } else { 12.0 },
        color: da.color,
        border: annots::border_width(doc, annot),
        background: annot.get(b"C".as_slice()).and_then(|c| Color::read(doc, c)),
        quadding,
        rotate,
        opacity: annots::opacity(doc, annot),
    };
    appearance::free_text(rect, annots::design_rect(doc, annot, rect), &layout)
}

fn border_style(width: f64) -> Object {
    let mut bs = Dictionary::new();
    bs.insert(b"Type".to_vec(), Object::name("Border"));
    bs.insert(b"W".to_vec(), number(width));
    Object::Dictionary(bs)
}

fn rgb(color: [f64; 3]) -> Color {
    Color::Rgb(color)
}

/// Stores `form` as a new stream and points the annotation's /AP /N at it.
fn put_appearance(
    tx: &mut Transaction<'_>,
    annot: &mut Dictionary,
    form: &Form,
) -> Result<(), AppError> {
    annots::put_appearance(tx, annot, form)?;
    Ok(())
}

/// The geometry an `annot-add` request carries, validated.
enum Shape {
    Note(Point),
    Markup(Vec<Quad>),
    FreeText([f64; 4]),
    Rectangle([f64; 4]),
    Line([Point; 2]),
    Ink(Vec<Vec<Point>>),
    Stamp([f64; 4]),
}

/// engine.ts `annot-add`.
pub fn add(
    session: &mut AppSession,
    request: &Value,
    buffers: &mut Buffers,
) -> Result<Value, AppError> {
    const OP: &str = "annot-add";
    let doc = session.require_mutable(OP)?;
    let page = session.check_page(request.get("page"), OP)?;
    let kind = request
        .get("type")
        .and_then(Value::as_str)
        .filter(|kind| EDITABLE_TYPES.contains(kind))
        .ok_or_else(|| {
            plain(format!(
                "{OP}: type must be one of {}",
                EDITABLE_TYPES.join(", ")
            ))
        })?
        .to_string();
    let contents = check_contents(request, OP)?.to_string();
    let (_, page_dict) = doc
        .page(page)
        .ok_or_else(|| plain(format!("{OP}: no such page")))?;
    let space = PageSpace::of(doc.document(), page_dict);
    let bounds = space.bounds();
    let has = |key: &str| request.get(key).is_some();
    let markup = matches!(kind.as_str(), "Highlight" | "Underline" | "StrikeOut");
    let point = match kind.as_str() {
        "Text" | "FreeText" => Some(check_point(&bounds, request.get("point"), OP)?),
        _ => None,
    };
    let quads = if markup {
        Some(check_quads(&bounds, request.get("quads"), OP)?)
    } else {
        None
    };
    let rect = match kind.as_str() {
        "Square" | "Circle" | "Stamp" => Some(check_rect(&bounds, request.get("rect"), OP)?),
        _ => None,
    };
    let line = match kind.as_str() {
        "Line" => Some(check_line(&bounds, request.get("points"), OP)?),
        _ => None,
    };
    let strokes = match kind.as_str() {
        "Ink" => Some(check_strokes(&bounds, request.get("strokes"), OP)?),
        _ => None,
    };
    let stamp = kind == "Stamp";
    let signature = request.get("signature");
    if stamp
        && signature.is_some()
        && (signature != Some(&Value::Bool(true)) || !has("image") || has("icon") || has("sign"))
    {
        return Err(plain(format!(
            "{OP}: a Stamp signature takes an image and no icon or sign name"
        )));
    }
    let icon = match (stamp, request.get("icon")) {
        (true, Some(icon)) => Some(
            icon.as_str()
                .filter(|icon| STAMP_ICONS.contains(icon))
                .ok_or_else(|| {
                    plain(format!(
                        "{OP}: icon must be one of {}",
                        STAMP_ICONS.join(", ")
                    ))
                })?
                .to_string(),
        ),
        _ => None,
    };
    if stamp && icon.is_none() != has("image") {
        return Err(plain(format!(
            "{OP}: a Stamp takes one of {} or an image",
            STAMP_ICONS.join(", ")
        )));
    }
    let sign_name = match (stamp, request.get("sign")) {
        (true, Some(sign)) => Some(check_sign_name(sign.get("name"), OP)?),
        _ => None,
    };
    if sign_name.is_some() && !has("image") {
        return Err(plain(format!(
            "{OP}: a signature appearance needs an image"
        )));
    }
    let image = match request.get("image") {
        Some(image) => Some(decode_stamp_image(Some(image), buffers, OP)?),
        None => None,
    };
    let fit = kind == "FreeText" && request.get("fit") == Some(&Value::Bool(true));
    if fit {
        check_free_text_encoding(&contents, OP)?;
    }
    let fit_rect = match (fit, point) {
        (true, Some(point)) => Some(free_text_fit(&bounds, point, &contents)?),
        _ => None,
    };
    if signature.is_some() && !stamp && (kind != "Ink" || !signature.is_some_and(Value::is_boolean))
    {
        return Err(plain(format!(
            "{OP}: signature applies to Ink only and must be true or false"
        )));
    }
    let signed = signature == Some(&Value::Bool(true));
    let drawn = strokes.is_some() && signed;
    let image_signature = stamp && signed;
    let step = if drawn {
        "add drawn signature".to_string()
    } else if sign_name.is_some() || image_signature {
        "add signature".to_string()
    } else if kind == "Text" {
        "add note".to_string()
    } else if kind == "FreeText" {
        "add text box".to_string()
    } else {
        format!("add {}", kind.to_lowercase())
    };
    let shape = match (point, quads, rect, line, strokes) {
        (Some(point), ..) if kind == "Text" => Shape::Note(point),
        (Some(point), ..) => {
            Shape::FreeText(fit_rect.unwrap_or_else(|| free_text_box(&bounds, point)))
        }
        (_, Some(quads), ..) => Shape::Markup(quads),
        (_, _, Some(rect), ..) if stamp => Shape::Stamp(rect),
        (_, _, Some(rect), ..) => Shape::Rectangle(rect),
        (.., Some(line), _) => Shape::Line(line),
        (.., Some(strokes)) => Shape::Ink(strokes),
        _ => return Err(plain(format!("{OP}: no geometry"))),
    };
    let today = request.get("today").and_then(Value::as_str);
    let new = NewAnnot {
        page,
        space,
        kind: &kind,
        contents: &contents,
        shape,
        icon,
        image: image.map(|image| image.placed),
        sign_name,
        drawn,
        image_signature,
        today,
    };
    session.mutate(&step, |session| {
        let mut tx = session.tx()?;
        new.write(&mut tx)
    })?;
    Ok(reply(request, Value::Object(session.edit_state())))
}

/// A validated `annot-add`, written in the step.
struct NewAnnot<'r> {
    page: usize,
    space: PageSpace,
    kind: &'r str,
    contents: &'r str,
    shape: Shape,
    icon: Option<String>,
    image: Option<images::Placed>,
    sign_name: Option<String>,
    drawn: bool,
    image_signature: bool,
    today: Option<&'r str>,
}

impl NewAnnot<'_> {
    fn write(self, tx: &mut Transaction<'_>) -> Result<(), AppError> {
        let page_ref = annots::page_object(tx.document(), self.page)
            .map_err(|error| plain(format!("annot-add: {error}")))?
            .map(Object::Reference);
        let space = self.space;
        let mut annot = Dictionary::new();
        annot.insert(b"Type".to_vec(), Object::name("Annot"));
        annot.insert(b"Subtype".to_vec(), Object::name(self.kind.as_bytes()));
        let put = |annot: &mut Dictionary, key: &[u8], value: Object| {
            annot.insert(key.to_vec(), value);
        };
        let place_p = |annot: &mut Dictionary, flags: i64| {
            if let Some(page) = &page_ref {
                annot.insert(b"P".to_vec(), page.clone());
            }
            annot.insert(b"F".to_vec(), Object::Integer(flags));
        };
        let contents = Object::text(self.contents);
        // The annotation's number comes first, as MuPDF's createAnnotation
        // takes it; its appearance and images follow.
        let id = tx.allocate(Object::Null, None)?;
        match self.shape {
            Shape::Note(point) => {
                let rect = space.rect_to_user([
                    point.0,
                    point.1,
                    point.0 + NOTE_SIZE,
                    point.1 + NOTE_SIZE,
                ]);
                let yellow = rgb([1.0, 1.0, 0.0]);
                put(&mut annot, b"Rect", numbers(&rect));
                put(&mut annot, b"C", yellow.object());
                let popup = tx.allocate(Object::Null, None)?;
                put(&mut annot, b"Popup", Object::Reference(popup));
                place_p(&mut annot, 28);
                put(&mut annot, b"Contents", contents);
                put_appearance(
                    tx,
                    &mut annot,
                    &appearance::note(b"Note", Some(&yellow), 1.0),
                )?;
                tx.put_value(id, Object::Dictionary(annot))?;
                let mut window = Dictionary::new();
                window.insert(b"Type".to_vec(), Object::name("Annot"));
                window.insert(b"Subtype".to_vec(), Object::name("Popup"));
                window.insert(b"Parent".to_vec(), Object::Reference(id));
                let at = popup_rect(&space, point);
                window.insert(b"Rect".to_vec(), numbers(&space.rect_to_user(at)));
                window.insert(b"Open".to_vec(), Object::Boolean(false));
                tx.put_value(popup, Object::Dictionary(window))?;
                append(tx, self.page, id)?;
                append(tx, self.page, popup)?;
                return Ok(());
            }
            Shape::Markup(quads) => {
                let color = rgb(match self.kind {
                    "Highlight" => [1.0, 1.0, 0.0],
                    _ => [1.0, 0.0, 0.0],
                });
                put(&mut annot, b"C", color.object());
                place_p(&mut annot, 4);
                let quads: Vec<Quad> = quads.into_iter().map(|q| space.quad_to_user(q)).collect();
                let flat: Vec<f64> = quads.iter().flatten().flat_map(|&(x, y)| [x, y]).collect();
                put(&mut annot, b"QuadPoints", numbers(&flat));
                put(&mut annot, b"Contents", contents);
                let (form, rect) = match self.kind {
                    "Highlight" => appearance::highlight(&quads, Some(&color), 1.0),
                    "Underline" => {
                        appearance::text_line(Mark::Underline, &quads, Some(&color), 1.0)
                    }
                    _ => appearance::text_line(Mark::StrikeOut, &quads, Some(&color), 1.0),
                };
                put(&mut annot, b"Rect", numbers(&rect));
                put_appearance(tx, &mut annot, &form)?;
            }
            Shape::FreeText(rect) => {
                if space.rotate != 0 {
                    put(
                        &mut annot,
                        b"Rotate",
                        Object::Integer(i64::from(space.rotate)),
                    );
                }
                let rect = space.rect_to_user(rect);
                put(&mut annot, b"Rect", numbers(&rect));
                put(&mut annot, b"BS", border_style(0.0));
                put(
                    &mut annot,
                    b"DA",
                    Object::string(b"/Helv 12 Tf 0 0 0 rg".to_vec()),
                );
                place_p(&mut annot, 4);
                put(&mut annot, b"Contents", contents);
                let form = free_text_form(tx.document(), &annot, rect, self.contents);
                put_appearance(tx, &mut annot, &form)?;
            }
            Shape::Rectangle(inner) => {
                let inner = space.rect_to_user(inner);
                let red = rgb([1.0, 0.0, 0.0]);
                let width = 1.0;
                let rd = appearance::border_allowance(width);
                let outer = appearance::grow(inner, rd);
                put(&mut annot, b"Rect", numbers(&outer));
                put(&mut annot, b"RD", numbers(&[rd; 4]));
                put(&mut annot, b"BS", border_style(width));
                put(&mut annot, b"C", red.object());
                place_p(&mut annot, 4);
                put(&mut annot, b"Contents", contents);
                let style = Style {
                    stroke: Some(red),
                    width,
                    ..Style::default()
                };
                let form = appearance::rectangle(inner, outer, self.kind == "Circle", &style);
                put_appearance(tx, &mut annot, &form)?;
            }
            Shape::Line([a, b]) => {
                let (a, b) = (space.to_user(a), space.to_user(b));
                let red = rgb([1.0, 0.0, 0.0]);
                put(&mut annot, b"L", numbers(&[a.0, a.1, b.0, b.1]));
                put(&mut annot, b"BS", border_style(1.0));
                put(&mut annot, b"C", red.object());
                place_p(&mut annot, 4);
                put(&mut annot, b"Contents", contents);
                let style = Style {
                    stroke: Some(red),
                    ..Style::default()
                };
                let (form, rect) = appearance::line(a, b, [b"", b""], &style);
                put(&mut annot, b"Rect", numbers(&rect));
                put_appearance(tx, &mut annot, &form)?;
            }
            Shape::Ink(strokes) => {
                let (color, width) = if self.drawn {
                    let xs = strokes.iter().flatten().map(|p| p.0);
                    let (lo, hi) = xs.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), x| {
                        (lo.min(x), hi.max(x))
                    });
                    (rgb([0.0, 0.0, 0.0]), signature_line_width(hi - lo))
                } else {
                    (rgb([0.0, 0.0, 1.0]), 1.5)
                };
                let strokes: Vec<Vec<Point>> = strokes
                    .into_iter()
                    .map(|stroke| stroke.into_iter().map(|p| space.to_user(p)).collect())
                    .collect();
                write_ink(&mut annot, &strokes, width, &color, page_ref.as_ref())?;
                if self.drawn {
                    put(&mut annot, b"Subj", Object::text(DRAWN_SIGNATURE_SUBJECT));
                } else {
                    put(&mut annot, b"Contents", contents);
                }
                let style = Style {
                    stroke: Some(color),
                    width,
                    ..Style::default()
                };
                let (form, rect) = appearance::ink(&strokes, &style);
                put(&mut annot, b"Rect", numbers(&rect));
                put_appearance(tx, &mut annot, &form)?;
            }
            Shape::Stamp(rect) => {
                let upright = space.upright();
                match (self.image, self.icon) {
                    (Some(placed), _) => {
                        let orientation = placed.orientation;
                        let image = images::store(tx, placed)?;
                        put(&mut annot, b"Rect", numbers(&space.rect_to_user(rect)));
                        place_p(&mut annot, 4);
                        let form = appearance::image_stamp(
                            image,
                            orientation_matrix(orientation),
                            upright,
                        );
                        put_appearance(tx, &mut annot, &form)?;
                        if let Some(name) = &self.sign_name {
                            put(&mut annot, b"Subj", Object::text(SIGNATURE_SUBJECT));
                            let text = signature_contents(name, self.today);
                            put(&mut annot, b"Contents", Object::text(&text));
                        } else if self.image_signature {
                            put(&mut annot, b"Subj", Object::text(SIGNATURE_SUBJECT));
                        } else {
                            put(&mut annot, b"Contents", contents);
                        }
                    }
                    (None, icon) => {
                        let icon = icon.unwrap_or_else(|| "Draft".to_string());
                        let red = rgb([1.0, 0.0, 0.0]);
                        put(
                            &mut annot,
                            b"Rect",
                            numbers(&space.rect_to_user(appearance::icon_fit(rect))),
                        );
                        put(&mut annot, b"C", red.object());
                        put(&mut annot, b"Name", Object::name(icon.as_bytes()));
                        place_p(&mut annot, 4);
                        put(&mut annot, b"Contents", contents);
                        let form = appearance::stamp(icon.as_bytes(), Some(&red), 1.0, upright);
                        put_appearance(tx, &mut annot, &form)?;
                    }
                }
            }
        }
        tx.put_value(id, Object::Dictionary(annot))?;
        append(tx, self.page, id)?;
        Ok(())
    }
}

/// The Ink entries in MuPDF's order: /BS, /RD, /C, /P, /F and /InkList.
fn write_ink(
    annot: &mut Dictionary,
    strokes: &[Vec<Point>],
    width: f64,
    color: &Color,
    page: Option<&Object>,
) -> Result<(), AppError> {
    annot.insert(b"BS".to_vec(), border_style(width));
    let rd = appearance::ink_allowance(width);
    annot.insert(b"RD".to_vec(), numbers(&[rd; 4]));
    annot.insert(b"C".to_vec(), color.object());
    if let Some(page) = page {
        annot.insert(b"P".to_vec(), page.clone());
    }
    annot.insert(b"F".to_vec(), Object::Integer(4));
    let list = strokes
        .iter()
        .map(|stroke| {
            let flat: Vec<f64> = stroke.iter().flat_map(|&(x, y)| [x, y]).collect();
            numbers(&flat)
        })
        .collect();
    annot.insert(b"InkList".to_vec(), Object::Array(list));
    Ok(())
}

/// The unit square drawn upright for an EXIF orientation.
fn orientation_matrix(orientation: u8) -> [f64; 6] {
    match orientation {
        2 => [-1.0, 0.0, 0.0, 1.0, 1.0, 0.0],
        3 => [-1.0, 0.0, 0.0, -1.0, 1.0, 1.0],
        4 => [1.0, 0.0, 0.0, -1.0, 0.0, 1.0],
        5 => [0.0, -1.0, -1.0, 0.0, 1.0, 1.0],
        6 => [0.0, -1.0, 1.0, 0.0, 0.0, 1.0],
        7 => [0.0, 1.0, 1.0, 0.0, 0.0, 0.0],
        8 => [0.0, 1.0, -1.0, 0.0, 1.0, 0.0],
        _ => appearance::IDENTITY,
    }
}

/// A note's popup window: beside the note's top right, inside the page.
fn popup_rect(space: &PageSpace, point: Point) -> [f64; 4] {
    let [w, h] = POPUP_SIZE;
    let x = (point.0 + NOTE_SIZE).min(space.width - w).max(0.0);
    let y = point.1.min(space.height - h).max(0.0);
    [
        x,
        y,
        (x + w).min(space.width.max(x)),
        (y + h).min(space.height.max(y)),
    ]
}

fn append(tx: &mut Transaction<'_>, page: usize, id: ObjectId) -> Result<(), AppError> {
    annots::append(tx, page, id).map_err(|error| plain(format!("annotation: {error}")))
}

/// engine.ts `findAnnotation`: an integer id naming one of the page's
/// listed annotations ("<op>: id must be an integer", "<op>: no annotation
/// <id> on this page").
pub(crate) fn find_annotation(
    session: &AppSession,
    page: usize,
    id: Option<&Value>,
    op: &str,
) -> Result<ObjectId, AppError> {
    let number = id
        .and_then(request::as_int)
        .ok_or_else(|| plain(format!("{op}: id must be an integer")))?;
    let missing = || {
        plain(format!(
            "{op}: no annotation {} on this page",
            js_string(id)
        ))
    };
    let app = session.doc.as_ref().ok_or_else(missing)?;
    let (_, dict) = app.page(page).ok_or_else(missing)?;
    annots::find(app.document(), dict, number)
        .and_then(|annot| annot.id)
        .ok_or_else(missing)
}

fn subtype<'d>(doc: &'d Document<'_>, id: ObjectId) -> &'d [u8] {
    let annot = doc.resolve_id(id);
    annot
        .as_dict()
        .and_then(|annot| doc.dict_get(annot, b"Subtype").as_name())
        .unwrap_or(b"")
}

/// The kinds `annot-set` writes a missing appearance for.
fn synthesizable(kind: &[u8]) -> bool {
    matches!(
        kind,
        b"Text"
            | b"FreeText"
            | b"Highlight"
            | b"Underline"
            | b"StrikeOut"
            | b"Square"
            | b"Circle"
            | b"Line"
            | b"Ink"
            | b"Stamp"
    )
}

/// engine.ts `annot-set`: new contents, and for a Helv 12 FreeText with
/// `fit` a box re-fitted around them from the same top left.
pub fn set(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    const OP: &str = "annot-set";
    let app = session.require_mutable(OP)?;
    let page = session.check_page(request.get("page"), OP)?;
    let contents = check_contents(request, OP)?.to_string();
    let id = find_annotation(session, page, request.get("annotId"), OP)?;
    let doc = app.document();
    let kind = subtype(doc, id).to_vec();
    if !EDITABLE_TYPES
        .iter()
        .any(|t| t.as_bytes() == kind.as_slice())
    {
        return Err(plain(format!(
            "{OP}: only {} annotations are editable",
            EDITABLE_TYPES.join(", ")
        )));
    }
    let annot = doc.resolve_id(id).as_dict().cloned().unwrap_or_default();
    let fit = request.get("fit") == Some(&Value::Bool(true)) && kind == b"FreeText" && {
        let da = default_appearance(doc, &annot);
        da.font == b"Helv" && da.size == 12.0
    };
    let (_, page_dict) = app
        .page(page)
        .ok_or_else(|| plain(format!("{OP}: no such page")))?;
    let space = PageSpace::of(doc, page_dict);
    let refit = if fit {
        check_free_text_encoding(&contents, OP)?;
        let current = annots::page_rect(doc, &space, &annot).unwrap_or_default();
        Some(free_text_fit(
            &space.bounds(),
            (
                current[0],
                current[1] + appearance::line_height(FIT_SIZE) / 2.0,
            ),
            &contents,
        )?)
    } else {
        None
    };
    session.mutate("edit annotation", |session| {
        let mut tx = session.tx()?;
        let doc = tx.document();
        let mut annot = annot;
        if let Some(rect) = refit {
            let outer = annots::grow_by_rd(doc, &annot, space.rect_to_user(rect));
            annot.insert(b"Rect".to_vec(), numbers(&outer));
        }
        annot.insert(b"Contents".to_vec(), Object::text(&contents));
        annot.remove(b"RC".as_slice());
        let has_ap = doc.dict_get(&annot, b"AP").as_dict().is_some();
        if kind == b"FreeText" || (!has_ap && synthesizable(&kind)) {
            if let Some((form, extra)) = regenerate(doc, &annot, &contents) {
                for (key, value) in extra {
                    annot.insert(key, value);
                }
                put_appearance(&mut tx, &mut annot, &form)?;
            }
        }
        tx.put_value(id, Object::Dictionary(annot))?;
        Ok(())
    })?;
    Ok(reply(request, Value::Object(session.edit_state())))
}

/// Dictionary entries an appearance comes with (/Rect, /RD).
type Entries = Vec<(Vec<u8>, Object)>;

/// A fresh appearance for an annotation from its dictionary, with the
/// entries it changes.
fn regenerate(doc: &Document<'_>, annot: &Dictionary, contents: &str) -> Option<(Form, Entries)> {
    let kind = doc.dict_get(annot, b"Subtype").as_name()?;
    if kind == b"FreeText" {
        let outer = annot
            .get(b"Rect".as_slice())
            .and_then(|r| annots::rect(doc, r))?;
        return Some((free_text_form(doc, annot, outer, contents), Vec::new()));
    }
    let synthesized = annots::view::synthesize(doc, annot)?;
    let mut extra = vec![(b"Rect".to_vec(), numbers(&synthesized.rect))];
    if let Some(rd) = synthesized.rd {
        extra.push((b"RD".to_vec(), numbers(&rd)));
    }
    Some((synthesized.form, extra))
}

/// engine.ts `annot-move`: a FreeText keeps its size; a drawn or image
/// signature can also be resized (`width`, aspect kept). The top left goes
/// to `point`, clamped inside the page; landing in place is a no-op.
pub fn move_annotation(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    const OP: &str = "annot-move";
    let app = session.require_mutable(OP)?;
    let page = session.check_page(request.get("page"), OP)?;
    let point = finite_numbers(request.get("point"), 2, "point", OP)?;
    let (px, py) = (point[0], point[1]);
    let doc = app.document();
    let (_, page_dict) = app
        .page(page)
        .ok_or_else(|| plain(format!("{OP}: no such page")))?;
    let space = PageSpace::of(doc, page_dict);
    let bounds = space.bounds();
    let id = find_annotation(session, page, request.get("annotId"), OP)?;
    let annot = doc.resolve_id(id).as_dict().cloned().unwrap_or_default();
    let kind = subtype(doc, id).to_vec();
    let subject = annots::text(doc, &annot, b"Subj");
    let stamp_signature = kind == b"Stamp" && subject == SIGNATURE_SUBJECT;
    let drawn = (kind == b"Ink" && subject == DRAWN_SIGNATURE_SUBJECT) || stamp_signature;
    if kind != b"FreeText" && !drawn {
        return Err(plain(format!(
            "{OP}: only FreeText annotations can be moved"
        )));
    }
    let width = request.get("width");
    if width.is_some() && !drawn {
        return Err(plain(format!(
            "{OP}: width applies to drawn signatures only"
        )));
    }
    let width = match width {
        Some(width) => Some(
            request::as_number(width)
                .ok_or_else(|| plain(format!("{OP}: width must be a finite number")))?,
        ),
        None => None,
    };
    // A drawn signature's box is its strokes' bounds, never its /Rect.
    let current = if kind == b"Ink" {
        let points = annots::ink_list(doc, &annot).into_iter().flatten();
        appearance::bounds(points.map(|p| space.to_page(p)))
    } else {
        annots::page_rect(doc, &space, &annot)
    }
    .ok_or_else(|| plain(format!("{OP}: this annotation has no geometry to move")))?;
    let (page_w, page_h) = (bounds[2] - bounds[0], bounds[3] - bounds[1]);
    let near = |rect: &[f64; 4]| {
        rect.iter()
            .zip(&current)
            .all(|(a, b)| (a - b).abs() <= 0.01)
    };
    if drawn {
        let [x0, y0, x1, y1] = current;
        let (w0, h0) = (x1 - x0, y1 - y0);
        if width.is_some() && w0 < 1.0 {
            return Err(plain(format!(
                "{OP}: this signature is too narrow to resize"
            )));
        }
        let fit_h = if h0 > 0.0 {
            page_h * w0 / h0
        } else {
            f64::INFINITY
        };
        let new_w = match width {
            None if stamp_signature => w0.min(page_w).min(fit_h),
            None => w0,
            Some(width) => width.max(24.0).min(page_w).min(fit_h),
        };
        let scale = if w0 > 0.0 { new_w / w0 } else { 1.0 };
        let new_h = h0 * scale;
        let x = bounds[0].max(px.min(bounds[2] - new_w));
        let y = bounds[1].max(py.min(bounds[3] - new_h));
        let rect = [x, y, x + new_w, y + new_h];
        if near(&rect) {
            return Ok(noop(session, request));
        }
        let step = if (new_w - w0).abs() > 0.01 {
            "resize signature"
        } else {
            "move signature"
        };
        let strokes: Vec<Vec<Point>> = if stamp_signature {
            Vec::new()
        } else {
            let clamp = |(sx, sy): Point| {
                let mx = (x + (sx - x0) * scale).min(bounds[2]).max(bounds[0]);
                let my = (y + (sy - y0) * scale).min(bounds[3]).max(bounds[1]);
                space.to_user((mx, my))
            };
            annots::ink_list(doc, &annot)
                .into_iter()
                .map(|stroke| {
                    stroke
                        .into_iter()
                        .map(|p| clamp(space.to_page(p)))
                        .collect()
                })
                .collect()
        };
        session.mutate(step, |session| {
            let mut tx = session.tx()?;
            let doc = tx.document();
            let mut annot = annot;
            if stamp_signature {
                let outer = annots::grow_by_rd(doc, &annot, space.rect_to_user(rect));
                annot.insert(b"Rect".to_vec(), numbers(&outer));
            } else {
                let line = signature_line_width(new_w);
                let color = (annot.get(b"C".as_slice()))
                    .and_then(|c| Color::read(doc, c))
                    .unwrap_or(Color::Rgb([0.0, 0.0, 0.0]));
                let page_ref = annot.get(b"P".as_slice()).cloned();
                let mut rebuilt = Dictionary::new();
                for (key, value) in annot.iter() {
                    if matches!(key.as_slice(), b"Type" | b"Subtype") {
                        rebuilt.insert(key.clone(), value.clone());
                    }
                }
                write_ink(&mut rebuilt, &strokes, line, &color, page_ref.as_ref())?;
                for (key, value) in annot.iter() {
                    if !rebuilt.contains_key(key.as_slice()) {
                        rebuilt.insert(key.clone(), value.clone());
                    }
                }
                let style = Style {
                    stroke: Some(color),
                    width: line,
                    opacity: annots::opacity(doc, &rebuilt),
                    ..Style::default()
                };
                let (form, outer) = appearance::ink(&strokes, &style);
                rebuilt.insert(b"Rect".to_vec(), numbers(&outer));
                put_appearance(&mut tx, &mut rebuilt, &form)?;
                annot = rebuilt;
            }
            tx.put_value(id, Object::Dictionary(annot))?;
            Ok(())
        })?;
        return Ok(reply(request, Value::Object(session.edit_state())));
    }
    let width = (current[2] - current[0]).min(page_w);
    let height = (current[3] - current[1]).min(page_h);
    let x = bounds[0].max(px.min(bounds[2] - width));
    let y = bounds[1].max(py.min(bounds[3] - height));
    let rect = [x, y, x + width, y + height];
    if near(&rect) {
        return Ok(noop(session, request));
    }
    session.mutate("move text box", |session| {
        let mut tx = session.tx()?;
        let doc = tx.document();
        let mut annot = annot;
        let outer = annots::grow_by_rd(doc, &annot, space.rect_to_user(rect));
        annot.insert(b"Rect".to_vec(), numbers(&outer));
        let has_ap = doc.dict_get(&annot, b"AP").as_dict().is_some();
        if !has_ap {
            let text = annots::text(doc, &annot, b"Contents");
            let form = free_text_form(doc, &annot, outer, &text);
            put_appearance(&mut tx, &mut annot, &form)?;
        }
        tx.put_value(id, Object::Dictionary(annot))?;
        Ok(())
    })?;
    Ok(reply(request, Value::Object(session.edit_state())))
}

fn noop(session: &AppSession, request: &Value) -> Value {
    let mut fields = Map::new();
    fields.insert("noop".into(), true.into());
    fields.extend(session.edit_state());
    reply(request, Value::Object(fields))
}

/// engine.ts `annot-delete`: an editable annotation or a Redact mark, its
/// popup with it.
pub fn delete(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    const OP: &str = "annot-delete";
    let app = session.require_mutable(OP)?;
    let page = session.check_page(request.get("page"), OP)?;
    let id = find_annotation(session, page, request.get("annotId"), OP)?;
    let kind = subtype(app.document(), id);
    let editable = EDITABLE_TYPES.iter().any(|t| t.as_bytes() == kind);
    if !editable && kind != b"Redact" {
        return Err(plain(format!(
            "{OP}: only editable annotations and Redact marks can be deleted"
        )));
    }
    session.mutate("delete annotation", |session| {
        let mut tx = session.tx()?;
        annots::remove(&mut tx, page, id).map_err(|error| plain(format!("{OP}: {error}")))
    })?;
    Ok(reply(request, Value::Object(session.edit_state())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates() {
        assert_eq!(civil_date(0), "1970-01-01");
        assert_eq!(civil_date(20_740), "2026-10-14");
        assert_eq!(civil_date(-1), "1969-12-31");
        assert_eq!(civil_date(11_016), "2000-02-29");
    }

    #[test]
    fn default_appearances_parse() {
        let doc = crate::app::edit::Builder::new("1.7")
            .unwrap()
            .into_document();
        let mut annot = Dictionary::new();
        annot.insert(
            b"DA".to_vec(),
            Object::string(b"/TiRo 20 Tf 1 0 0 rg".to_vec()),
        );
        let da = default_appearance(&doc, &annot);
        assert_eq!(da.font, b"TiRo");
        assert_eq!(da.size, 20.0);
        assert_eq!(da.color, Some(Color::Rgb([1.0, 0.0, 0.0])));
        annot.insert(b"DA".to_vec(), Object::string(b"0 g /Helv 0 Tf".to_vec()));
        let da = default_appearance(&doc, &annot);
        assert_eq!((da.font.as_slice(), da.size), (b"Helv".as_slice(), 0.0));
        annot.insert(b"DA".to_vec(), Object::string(b"Tf rg ) ( 1 2".to_vec()));
        assert_eq!(default_appearance(&doc, &annot).font, b"Helv");
    }
}
