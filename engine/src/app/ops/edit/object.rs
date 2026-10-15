// SPDX-License-Identifier: AGPL-3.0-or-later
//! Image and Form placements (engine.ts `edit-object` and `edit-objects`,
//! content-edit.ts `editPlanPlacement` and its helpers).
//!
//! A simple placement is one `q [x y w h re W n] cm /Name Do Q` block of a
//! page element (`Record::placement`). Move, resize, transform and crop are
//! one page-space affine applied to its `cm` and clip; delete removes the
//! block; duplicate inserts a moved copy after its `Q`; replace points the
//! `Do` at a new image. Every check runs before the operation opens; the
//! write is `write::write_element`'s one privatise and splice per element.
use std::collections::BTreeSet;

use serde_json::{Value, json};

use super::geometry::{affine, edit_number, map_rect, orient_of, writable};
use super::scan::{Kind, Orient, PageScan, Placement, Record};
use super::write::{self, SpanEdit, Splice};
use super::{MAX_EDIT_TARGETS, edit_error, edit_state_reply, resolve_target, stale};
use crate::app::edit::Place;
use crate::app::ops::pages::images::{self, ImageError, Placed};
use crate::app::request::{self, js_string};
use crate::app::{AppError, AppSession, Buffers, MAX_SAVE_BYTES, pagetree};
use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId};

/// engine-core.ts `MAX_IMAGE_PAGE_POINTS`: the longest side of a box.
pub const MAX_IMAGE_PAGE_POINTS: f64 = 14_400.0;
/// engine.ts `MAX_STAMP_IMAGE_BYTES`: a replacement image's file.
pub const MAX_STAMP_IMAGE_BYTES: usize = 4 * 1024 * 1024;

/// The actions `edit-objects` takes (no crop, no replace).
const COMPOUND_ACTIONS: [&str; 5] = ["move", "delete", "duplicate", "resize", "transform"];

/// content-edit.ts `editRectOf`: four finite numbers, normalized.
fn rect_of(doc: &Document<'_>, value: Option<&Object>) -> Option<[f64; 4]> {
    let items = doc.resolve(value?).as_array()?;
    if items.len() != 4 {
        return None;
    }
    let mut n = [0.0; 4];
    for (slot, item) in n.iter_mut().zip(items) {
        *slot = match doc.resolve(item) {
            Object::Integer(v) => *v as f64,
            Object::Real(v) => *v,
            _ => return None,
        };
    }
    n.iter().all(|v| v.is_finite()).then(|| {
        [
            n[0].min(n[2]),
            n[1].min(n[3]),
            n[0].max(n[2]),
            n[1].max(n[3]),
        ]
    })
}

/// content-edit.ts `editPageClipBox`: MediaBox within CropBox, in user
/// space; Letter without a MediaBox. Both boxes are inherited here
/// (engine.ts read the page's own keys only).
pub fn page_clip_box(doc: &Document<'_>, page: &Dictionary) -> [f64; 4] {
    let media = rect_of(doc, pagetree::inherited(doc, page, b"MediaBox"))
        .unwrap_or([0.0, 0.0, 612.0, 792.0]);
    let Some(crop) = rect_of(doc, pagetree::inherited(doc, page, b"CropBox")) else {
        return media;
    };
    let clipped = [
        media[0].max(crop[0]),
        media[1].max(crop[1]),
        media[2].min(crop[2]),
        media[3].min(crop[3]),
    ];
    if clipped[2] > clipped[0] && clipped[3] > clipped[1] {
        clipped
    } else {
        media
    }
}

/// The page clip box of `page` in the session's document.
pub fn session_clip_box(session: &AppSession, page: usize) -> [f64; 4] {
    session
        .doc
        .as_ref()
        .and_then(|doc| {
            let (_, dict) = doc.page(page)?;
            Some(page_clip_box(doc.document(), dict))
        })
        .unwrap_or([0.0, 0.0, 612.0, 792.0])
}

/// content-edit.ts `editObjectBox` for a request value.
pub fn object_box(value: Option<&Value>, clip: &[f64; 4]) -> Result<[f64; 4], AppError> {
    let numbers = value
        .and_then(Value::as_array)
        .filter(|items| items.len() == 4)
        .and_then(|items| {
            items
                .iter()
                .map(request::as_number)
                .collect::<Option<Vec<f64>>>()
        });
    let Some([x0, y0, x1, y1]) = numbers.and_then(|n| <[f64; 4]>::try_from(n).ok()) else {
        return Err(edit_error(
            "out-of-bounds",
            "edit-object: box must be four finite numbers [x0, y0, x1, y1]",
        ));
    };
    check_box(&[x0, y0, x1, y1], clip)
}

/// `value >= min`, false for NaN.
fn at_least(value: f64, min: f64) -> bool {
    value >= min
}

/// content-edit.ts `editObjectBox` for a computed box.
pub fn check_box(rect: &[f64; 4], clip: &[f64; 4]) -> Result<[f64; 4], AppError> {
    let [x0, y0, x1, y1] = *rect;
    if !at_least(x1 - x0, 1.0) || !at_least(y1 - y0, 1.0) {
        return Err(edit_error(
            "out-of-bounds",
            "edit-object: each side of the box must be at least 1 pt",
        ));
    }
    if x1 - x0 > MAX_IMAGE_PAGE_POINTS || y1 - y0 > MAX_IMAGE_PAGE_POINTS {
        return Err(edit_error(
            "too-large",
            "edit-object: a side over the 14400 pt ceiling",
        ));
    }
    if x0 < clip[0] || y0 < clip[1] || x1 > clip[2] || y1 > clip[3] {
        return Err(edit_error(
            "out-of-bounds",
            "edit-object: the box must stay inside the page",
        ));
    }
    Ok(*rect)
}

/// content-edit.ts `editPlacementMatrix`: the `cm` mapping `rect` onto
/// `full` in orientation `orient`.
pub fn placement_numbers(full: &[f64; 4], rect: &[f64; 4], orient: Orient) -> [f64; 6] {
    if !orient.swap {
        let a = orient.sx * (full[2] - full[0]) / (rect[2] - rect[0]);
        let d = orient.sy * (full[3] - full[1]) / (rect[3] - rect[1]);
        let e = full[0] - (a * rect[0]).min(a * rect[2]);
        let f = full[1] - (d * rect[1]).min(d * rect[3]);
        return [a, 0.0, 0.0, d, e, f];
    }
    let c = orient.sx * (full[2] - full[0]) / (rect[3] - rect[1]);
    let b = orient.sy * (full[3] - full[1]) / (rect[2] - rect[0]);
    let e = full[0] - (c * rect[1]).min(c * rect[3]);
    let f = full[1] - (b * rect[0]).min(b * rect[2]);
    [0.0, b, c, 0.0, e, f]
}

/// `placement_numbers` as `cm` operands.
pub fn placement_matrix(full: &[f64; 4], rect: &[f64; 4], orient: Orient) -> String {
    matrix_text(&placement_numbers(full, rect, orient))
}

/// Six numbers as operands.
pub fn matrix_text(m: &[f64; 6]) -> String {
    m.iter()
        .map(|&v| edit_number(v))
        .collect::<Vec<_>>()
        .join(" ")
}

/// content-edit.ts `editRectAffine`: the axis-aligned map of `from` onto `to`.
pub fn rect_affine(from: &[f64; 4], to: &[f64; 4]) -> [f64; 6] {
    let sx = (to[2] - to[0]) / (from[2] - from[0]);
    let sy = (to[3] - to[1]) / (from[3] - from[1]);
    [sx, 0.0, 0.0, sy, to[0] - sx * from[0], to[1] - sy * from[1]]
}

/// content-edit.ts `editTransformAbout`: a quarter turn or a flip about
/// `rect`'s centre.
pub fn transform_about(token: &str, rect: &[f64; 4]) -> Option<[f64; 6]> {
    let linear = match token {
        "rotate-cw" => [0.0, -1.0, 1.0, 0.0],
        "rotate-ccw" => [0.0, 1.0, -1.0, 0.0],
        "flip-h" => [-1.0, 0.0, 0.0, 1.0],
        "flip-v" => [1.0, 0.0, 0.0, -1.0],
        _ => return None,
    };
    let cx = (rect[0] + rect[2]) / 2.0;
    let cy = (rect[1] + rect[3]) / 2.0;
    let [a, b, c, d] = linear;
    Some(affine(
        &affine(&[1.0, 0.0, 0.0, 1.0, -cx, -cy], &[a, b, c, d, 0.0, 0.0]),
        &[1.0, 0.0, 0.0, 1.0, cx, cy],
    ))
}

/// Whether `clip_operands` writes `rect` as plain numbers.
fn clip_writable(rect: &[f64; 4]) -> bool {
    [rect[0], rect[1], rect[2] - rect[0], rect[3] - rect[1]]
        .into_iter()
        .all(writable)
}

/// content-edit.ts `editClipOperands`: `x y w h`.
pub fn clip_operands(rect: &[f64; 4]) -> String {
    format!(
        "{} {} {} {}",
        edit_number(rect[0]),
        edit_number(rect[1]),
        edit_number(rect[2] - rect[0]),
        edit_number(rect[3] - rect[1])
    )
}

/// What a placement becomes under a page-space affine.
#[derive(Clone, Debug, PartialEq)]
pub struct After {
    pub full: [f64; 4],
    pub orient: Orient,
    pub clip: Option<[f64; 4]>,
    pub visible: [f64; 4],
    pub cm: String,
    pub clip_text: Option<String>,
    /// Every number of `cm` and `clip_text` is written plainly.
    pub writable: bool,
}

/// content-edit.ts `editPlacementAfter`: `g` applied to the `cm` and the
/// clip; `target` is what `g` maps the visible rect onto, taken verbatim.
pub fn placement_after(placement: &Placement, g: &[f64; 6], target: Option<[f64; 4]>) -> After {
    let orient = orient_of(&affine(&placement.matrix, g)).unwrap_or(placement.orient);
    let clip = placement.clip.as_ref().map(|c| map_rect(g, &c.rect));
    let full = match (target, clip) {
        (Some(target), None) => target,
        _ => map_rect(g, &placement.full),
    };
    let visible = target.unwrap_or_else(|| map_rect(g, &placement.visible));
    let cm = placement_numbers(&full, &placement.rect, orient);
    After {
        full,
        orient,
        clip,
        visible,
        cm: matrix_text(&cm),
        clip_text: clip.as_ref().map(clip_operands),
        writable: cm.into_iter().all(writable) && clip.as_ref().is_none_or(clip_writable),
    }
}

/// content-edit.ts `editCheckFull`: the uncropped box stays finite and
/// within the side ceiling (it may leave the page under a clip).
pub fn check_full(full: &[f64; 4]) -> Result<(), AppError> {
    if full.iter().any(|v| !v.is_finite())
        || full[2] - full[0] > MAX_IMAGE_PAGE_POINTS
        || full[3] - full[1] > MAX_IMAGE_PAGE_POINTS
    {
        return Err(edit_error(
            "too-large",
            "edit-object: the uncropped box would be over the 14400 pt ceiling",
        ));
    }
    Ok(())
}

/// `check_full`, then the written numbers: a placement whose `cm` would
/// take a number past `MAX_WRITTEN` (a near-zero /BBox drawn large) cannot
/// be written; engine.ts wrote exponents or `Infinity`.
pub fn check_after(after: &After) -> Result<(), AppError> {
    check_full(&after.full)?;
    if !after.writable {
        return Err(out_of_range());
    }
    Ok(())
}

fn out_of_range() -> AppError {
    edit_error(
        "unsupported-transform",
        "edit-object: the placement's numbers would be out of range",
    )
}

/// A placement whose rect or box has no area cannot be mapped; engine.ts
/// wrote `Infinity` into its `cm`.
pub fn check_area(placement: &Placement) -> Result<(), AppError> {
    let positive =
        |r: &[f64; 4]| r.iter().all(|v| v.is_finite()) && r[2] - r[0] > 0.0 && r[3] - r[1] > 0.0;
    if positive(&placement.rect) && positive(&placement.full) && positive(&placement.visible) {
        Ok(())
    } else {
        Err(edit_error(
            "unsupported-transform",
            "edit-object: the placement has no area",
        ))
    }
}

/// One planned placement write: the new `cm` (None leaves it) and clip
/// operands (None means no clip).
#[derive(Clone, Debug, PartialEq)]
pub struct PlacementWrite {
    pub cm: Option<String>,
    pub clip_text: Option<String>,
    pub noop: bool,
}

/// content-edit.ts `editPlanPlacement`: a move, resize, transform or crop
/// checked and planned. A transform is never a no-op.
pub fn plan_placement(
    placement: &Placement,
    action: &str,
    raw_box: Option<&Value>,
    transform: Option<&Value>,
    page_clip: &[f64; 4],
) -> Result<PlacementWrite, AppError> {
    check_area(placement)?;
    let current_cm = placement_matrix(&placement.full, &placement.rect, placement.orient);
    let current_clip = placement.clip.as_ref().map(|c| clip_operands(&c.rect));
    if action == "crop" {
        let target = object_box(raw_box, page_clip)?;
        let full = placement.full;
        if target[0] < full[0] - 0.01
            || target[1] < full[1] - 0.01
            || target[2] > full[2] + 0.01
            || target[3] > full[3] + 0.01
        {
            return Err(edit_error(
                "out-of-bounds",
                "edit-object: a crop must stay inside the picture",
            ));
        }
        let whole = target.iter().zip(full).all(|(a, b)| (a - b).abs() <= 0.01);
        if !whole && !clip_writable(&target) {
            return Err(out_of_range());
        }
        let clip_text = (!whole).then(|| clip_operands(&target));
        let noop = clip_text == current_clip;
        return Ok(PlacementWrite {
            cm: None,
            clip_text,
            noop,
        });
    }
    let (g, target) = if action == "transform" {
        let t = transform
            .and_then(Value::as_str)
            .and_then(|token| transform_about(token, &placement.visible))
            .ok_or_else(|| {
                edit_error(
                    "invalid-transform",
                    "edit-object: transform must be rotate-cw, rotate-ccw, flip-h or flip-v",
                )
            })?;
        let turned = map_rect(&t, &placement.visible);
        let target = match raw_box {
            None => check_box(&turned, page_clip)?,
            Some(_) => object_box(raw_box, page_clip)?,
        };
        (affine(&t, &rect_affine(&turned, &target)), target)
    } else {
        let target = object_box(raw_box, page_clip)?;
        (rect_affine(&placement.visible, &target), target)
    };
    let after = placement_after(placement, &g, Some(target));
    check_after(&after)?;
    let noop = action != "transform" && after.cm == current_cm && after.clip_text == current_clip;
    Ok(PlacementWrite {
        cm: Some(after.cm),
        clip_text: after.clip_text,
        noop,
    })
}

/// content-edit.ts `editPlacementSpans`: the `cm` and, when the clip
/// changes, its operands, its removal or a new one after the `q`.
pub fn placement_spans(
    placement: &Placement,
    cm: Option<&str>,
    clip_text: Option<&str>,
) -> Vec<Splice> {
    let mut spans = Vec::new();
    if let Some(cm) = cm {
        spans.push(Splice {
            start: placement.cm.start,
            end: placement.cm.end,
            replacement: cm.as_bytes().to_vec(),
        });
    }
    match (&placement.clip, clip_text) {
        (Some(clip), Some(text)) => spans.push(Splice {
            start: clip.ops.start,
            end: clip.ops.end,
            replacement: text.as_bytes().to_vec(),
        }),
        (Some(clip), None) => spans.push(Splice {
            start: clip.start,
            end: clip.end,
            replacement: Vec::new(),
        }),
        (None, Some(text)) => {
            let at = placement.q_start.saturating_add(1);
            spans.push(Splice {
                start: at,
                end: at,
                replacement: format!("\n{text} re W n").into_bytes(),
            });
        }
        (None, None) => {}
    }
    spans
}

/// content-edit.ts `editDuplicateEdit`: a copy of the block at `target`,
/// inserted after its `Q`, its clip and `cm` following the same affine.
pub fn duplicate_edit(
    bytes: &[u8],
    placement: &Placement,
    target: &[f64; 4],
) -> Result<Splice, AppError> {
    let after = placement_after(
        placement,
        &rect_affine(&placement.visible, target),
        Some(*target),
    );
    let gone = || {
        AppError::reason(
            "invalid",
            "edit-object: the recorded spans no longer fit the stream",
        )
    };
    let slice = |start: usize, end: usize| bytes.get(start..end).ok_or_else(gone);
    let mut replacement = vec![b'\n'];
    match (&placement.clip, &after.clip_text) {
        (Some(clip), Some(text)) => {
            replacement.extend_from_slice(slice(placement.q_start, clip.ops.start)?);
            replacement.extend_from_slice(text.as_bytes());
            replacement.extend_from_slice(slice(clip.ops.end, placement.cm.start)?);
        }
        _ => replacement.extend_from_slice(slice(placement.q_start, placement.cm.start)?),
    }
    replacement.extend_from_slice(after.cm.as_bytes());
    replacement.extend_from_slice(slice(placement.cm.end, placement.q_end)?);
    Ok(Splice {
        start: placement.q_end,
        end: placement.q_end,
        replacement,
    })
}

/// The `not-candidate` refusal of a text or unsupported target.
fn not_candidate(op: &str, subject: &str, record: &Record) -> AppError {
    let reason = record.reason.map(|r| format!(" ({r})")).unwrap_or_default();
    let error = edit_error(
        "not-candidate",
        format!("{op}: {subject} cannot be edited{reason}"),
    );
    match record.reason {
        Some(reason) => error.with_extra(json!({"reason": reason})),
        None => error,
    }
}

/// An action the placement does not offer: its own reason is the token.
fn unavailable(op: &str, action: &str, subject: &str, record: &Record) -> AppError {
    let reason = record.reason.map(|r| format!(" ({r})")).unwrap_or_default();
    let error = edit_error(
        record.reason.unwrap_or("not-candidate"),
        format!("{op}: {action} is not available for {subject}{reason}"),
    );
    match record.reason {
        Some(reason) => error.with_extra(json!({"reason": reason})),
        None => error,
    }
}

fn offers(record: &Record, action: &str) -> bool {
    record
        .object_ops
        .as_ref()
        .is_some_and(|ops| ops.contains(&action))
}

/// A replacement image read up front, so a bad file is a refusal.
struct Replacement {
    placed: Placed,
}

/// engine.ts `decodeStampImage` for edit-object: a name of 1..255
/// characters and 1..4 MiB of PNG or JPEG bytes.
fn read_replacement(value: Option<&Value>, buffers: &Buffers) -> Result<Replacement, AppError> {
    let op = "edit-object";
    let name = value
        .and_then(|v| v.get("name"))
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty() && super::js_len(name) <= 255);
    let Some(name) = name else {
        return Err(edit_error(
            "invalid-image",
            format!("{op}: a stamp image needs a name of at most 255 characters"),
        ));
    };
    let bytes = buffers
        .input(value.and_then(|v| v.get("bytes")))
        .filter(|bytes| !bytes.is_empty());
    let Some(bytes) = bytes else {
        return Err(edit_error(
            "invalid-image",
            format!("{op}: stamp image {name} has no bytes"),
        ));
    };
    if bytes.len() > MAX_STAMP_IMAGE_BYTES {
        return Err(edit_error(
            "too-large",
            format!("{op}: {name} is over the {MAX_STAMP_IMAGE_BYTES} byte image ceiling"),
        ));
    }
    match images::read(bytes, MAX_SAVE_BYTES) {
        Ok(placed) if placed.width > 0 && placed.height > 0 => Ok(Replacement { placed }),
        Ok(_) => Err(edit_error(
            "invalid-image",
            format!("{op}: {name} has no pixels"),
        )),
        Err(ImageError::TooLarge) => Err(edit_error(
            "too-large",
            format!("{op}: {name} is over the {MAX_SAVE_BYTES} byte image ceiling"),
        )),
        Err(ImageError::Unreadable(why)) => Err(edit_error(
            "invalid-image",
            format!("{op}: {name} is not a PNG or JPEG the app can read ({why})"),
        )),
    }
}

/// The unit square in EXIF orientation `o` (1 upright), as a `cm` prefix.
fn exif_unit(orientation: u8) -> [f64; 6] {
    match orientation {
        2 => [-1.0, 0.0, 0.0, 1.0, 1.0, 0.0],
        3 => [-1.0, 0.0, 0.0, -1.0, 1.0, 1.0],
        4 => [1.0, 0.0, 0.0, -1.0, 0.0, 1.0],
        5 => [0.0, -1.0, -1.0, 0.0, 1.0, 1.0],
        6 => [0.0, -1.0, 1.0, 0.0, 0.0, 1.0],
        7 => [0.0, 1.0, 1.0, 0.0, 0.0, 0.0],
        8 => [0.0, 1.0, -1.0, 0.0, 1.0, 0.0],
        _ => [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
    }
}

/// The orientation whose turn undoes `orientation`'s.
fn exif_inverse(orientation: u8) -> u8 {
    match orientation {
        6 => 8,
        8 => 6,
        other => other,
    }
}

/// The orientation a placed image's `cm` carries (`images::TURNED_KEY`, as
/// a replace here or an image page writes it), 1 for none.
fn turned_by(doc: &Document<'_>, image: Option<&Object>) -> u8 {
    let dict = image.and_then(|image| doc.resolve(image).as_dict());
    match dict.map(|dict| doc.dict_get(dict, images::TURNED_KEY)) {
        Some(Object::Integer(o @ 2..=8)) => u8::try_from(*o).unwrap_or(1),
        _ => 1,
    }
}

/// The `cm` a replaced simple placement gets, None to keep it. The turn an
/// earlier replace gave it (`turned`) is taken back out first, so the
/// placement is the upright picture's: an upright unclipped one is
/// refitted to the new aspect (anchored at its bottom-left) when it differs
/// by more than 1%, as engine.ts does; the new picture's EXIF orientation
/// then turns it within the box (MuPDF placed the stored pixels).
fn replaced_cm(
    placement: &Placement,
    placed: &Placed,
    turned: u8,
) -> Result<Option<String>, AppError> {
    let (base, orient) = match turned {
        2..=8 => {
            let base = affine(&exif_unit(exif_inverse(turned)), &placement.matrix);
            (base, orient_of(&base))
        }
        _ => (placement.matrix, Some(placement.orient)),
    };
    let (wide, high) = match placed.orientation {
        5..=8 => (placed.height, placed.width),
        _ => (placed.width, placed.height),
    };
    let upright = placement.clip.is_none() && orient == Some(Orient::IDENTITY);
    let [x0, y0, x1, y1] = placement.full;
    let (old_width, old_height) = (x1 - x0, y1 - y0);
    let old_aspect = old_width / old_height;
    let aspect = f64::from(wide) / f64::from(high.max(1));
    let refit = upright && wide > 0 && (aspect - old_aspect).abs() > old_aspect * 0.01;
    let base = match (refit, aspect > old_aspect) {
        (false, _) => base,
        (true, true) => [old_width, 0.0, 0.0, old_width / aspect, x0, y0],
        (true, false) => [old_height * aspect, 0.0, 0.0, old_height, x0, y0],
    };
    let cm = affine(&exif_unit(placed.orientation), &base);
    if cm == placement.matrix {
        return Ok(None);
    }
    if !cm.into_iter().all(writable) {
        return Err(out_of_range());
    }
    Ok(Some(matrix_text(&cm)))
}

/// The writes of one edit-object action on one placement.
enum ObjectWrite {
    Spans(Vec<Splice>),
    Replace {
        rename: Splice,
        cm: Option<Splice>,
        name: Vec<u8>,
        placed: Box<Placed>,
        /// The orientation the new `cm` carries (`images::TURNED_KEY`).
        turn: Option<u8>,
    },
}

/// The page's own /Resources, else the inherited ones (inherited ones are
/// privatised before a write, so a name free here is free there).
pub fn page_resources<'d>(doc: &'d Document<'_>, page: &'d Dictionary) -> Option<&'d Dictionary> {
    let own = page
        .get(b"Resources".as_slice())
        .map(|v| doc.resolve(v))
        .filter(|v| !v.is_null());
    match own {
        Some(value) => value.as_dict(),
        None => pagetree::inherited(doc, page, b"Resources").and_then(|v| doc.resolve(v).as_dict()),
    }
}

/// The first free `<name>r<n>`.
fn replacement_name(xobjects: &Dictionary, name: &[u8]) -> Option<Vec<u8>> {
    (1..=u32::MAX).find_map(|n| {
        let mut candidate = name.to_vec();
        candidate.extend_from_slice(format!("r{n}").as_bytes());
        (!xobjects.contains_key(candidate.as_slice())).then_some(candidate)
    })
}

/// Writes one element's spans (privatising it first when the scan says so).
fn write_spans(
    session: &mut AppSession,
    scan: &PageScan,
    page: usize,
    groups: &[(usize, Vec<SpanEdit>)],
    op: &str,
) -> Result<ObjectId, AppError> {
    let mut tx = session.tx()?;
    let page_id = write::page_object(tx.document(), page, op)?;
    let mut privatised = BTreeSet::new();
    let mut copied = 0;
    for (element, spans) in groups {
        write::write_element(
            &mut tx,
            scan,
            page_id,
            *element,
            &[],
            spans,
            &mut privatised,
            &mut copied,
        )
        .map_err(write::writer_error)?;
    }
    Ok(page_id)
}

/// engine.ts `edit-object`: one action on one placement.
pub fn edit_object(
    session: &mut AppSession,
    request: &Value,
    buffers: &Buffers,
) -> Result<Value, AppError> {
    let op = "edit-object";
    session.require_mutable(op)?;
    let page = session.check_page(request.get("page"), op)?;
    let (scan, index) = resolve_target(session, request, page).map_err(|error| stale(op, error))?;
    let Some(record) = scan.records.get(index) else {
        return Err(edit_error(
            "stale-target",
            "edit-object: the target is gone",
        ));
    };
    if record.kind == Kind::Text || !record.candidate {
        return Err(not_candidate(op, "that target", record));
    }
    let action_value = request.get("action");
    let action = action_value
        .and_then(Value::as_str)
        .filter(|a| offers(record, a))
        .ok_or_else(|| unavailable(op, &js_string(action_value), "that placement", record))?;
    let mut duplicate_box = None;
    let mut planned = None;
    if matches!(
        action,
        "move" | "resize" | "duplicate" | "transform" | "crop"
    ) {
        let clip = session_clip_box(session, page);
        let Some(current) = &record.placement else {
            return Err(edit_error(
                "not-candidate",
                "edit-object: that placement cannot be edited",
            ));
        };
        if action == "duplicate" {
            check_area(current)?;
            let target = object_box(request.get("box"), &clip)?;
            check_after(&placement_after(
                current,
                &rect_affine(&current.visible, &target),
                Some(target),
            ))?;
            duplicate_box = Some(target);
        } else {
            let write = plan_placement(
                current,
                action,
                request.get("box"),
                request.get("transform"),
                &clip,
            )?;
            if write.noop {
                return Ok(edit_state_reply(
                    session,
                    request,
                    json!({"page": page, "noop": true}),
                ));
            }
            planned = Some(write);
        }
    }
    if action == "duplicate" && scan.records.len() + 1 > MAX_EDIT_TARGETS {
        return Err(edit_error(
            "too-large",
            format!(
                "edit-object: a duplicate would put more than {MAX_EDIT_TARGETS} targets on this page"
            ),
        ));
    }
    let replacement = match action {
        "replace" => Some(read_replacement(request.get("image"), buffers)?),
        _ => None,
    };
    let Some(element) = record.element else {
        return Err(edit_error(
            "not-candidate",
            "edit-object: only top-level page content is editable",
        ));
    };
    let placement = record.placement.as_ref();
    let missing = |what: &str| {
        edit_error(
            "not-candidate",
            format!("edit-object: that placement cannot be {what}"),
        )
    };
    let write = match action {
        "delete" => {
            let p = placement.ok_or_else(|| missing("deleted"))?;
            ObjectWrite::Spans(vec![Splice {
                start: p.q_start,
                end: p.q_end,
                replacement: Vec::new(),
            }])
        }
        "duplicate" => {
            let (p, target) = placement
                .zip(duplicate_box)
                .ok_or_else(|| missing("duplicated"))?;
            let bytes = scan
                .streams
                .get(&record.stream)
                .ok_or_else(|| missing("duplicated"))?;
            ObjectWrite::Spans(vec![duplicate_edit(bytes, p, &target)?])
        }
        "replace" => {
            let Some(Replacement { placed }) = replacement else {
                return Err(missing("replaced"));
            };
            let name = record
                .object_name
                .clone()
                .ok_or_else(|| missing("replaced"))?;
            let doc = session.doc.as_ref().ok_or_else(|| missing("replaced"))?;
            let (_, dict) = doc.page(page).ok_or_else(|| missing("replaced"))?;
            let resources = page_resources(doc.document(), dict);
            let xobjects = super::font::dict_at(doc.document(), resources, b"XObject");
            let xobjects = xobjects.ok_or_else(|| {
                AppError::reason("invalid", "edit-object: the page has no XObject resources")
            })?;
            let fresh = replacement_name(xobjects, &name).ok_or_else(|| missing("replaced"))?;
            let rename = Splice {
                start: record.operand.start,
                end: record.operand.end,
                replacement: super::plan::name_source(&fresh).into_bytes(),
            };
            let turned = turned_by(doc.document(), xobjects.get(name.as_slice()));
            let cm = match placement {
                Some(p) => replaced_cm(p, &placed, turned)?.map(|text| Splice {
                    start: p.cm.start,
                    end: p.cm.end,
                    replacement: text.into_bytes(),
                }),
                None => None,
            };
            // A replace-only placement keeps its `cm`, so its picture is
            // drawn as stored and carries no turn.
            let turn = placement
                .is_some()
                .then_some(placed.orientation)
                .filter(|o| (2..=8).contains(o));
            ObjectWrite::Replace {
                rename,
                cm,
                name: fresh,
                placed: Box::new(placed),
                turn,
            }
        }
        _ => {
            let (p, write) = placement.zip(planned).ok_or_else(|| missing("moved"))?;
            ObjectWrite::Spans(placement_spans(
                p,
                write.cm.as_deref(),
                write.clip_text.as_deref(),
            ))
        }
    };
    session.mutate("edit object", |session| match write {
        ObjectWrite::Spans(spans) => {
            let spans = spans
                .into_iter()
                .map(|splice| SpanEdit {
                    record: index,
                    splice,
                })
                .collect();
            write_spans(session, &scan, page, &[(element, spans)], op).map(|_| ())
        }
        ObjectWrite::Replace {
            rename,
            cm,
            name,
            placed,
            turn,
        } => {
            let spans = std::iter::once(rename)
                .chain(cm)
                .map(|splice| SpanEdit {
                    record: index,
                    splice,
                })
                .collect();
            let page_id = write_spans(session, &scan, page, &[(element, spans)], op)?;
            let mut tx = session.tx()?;
            let doc = tx.document();
            let place = Place::object(page_id)
                .child(doc, b"Resources")
                .child(doc, b"XObject");
            // Privatised resources hold the names the plan saw.
            let free = place
                .read(doc)
                .as_dict()
                .is_some_and(|x| !x.contains_key(name.as_slice()));
            if !free {
                return Err(AppError::reason(
                    "invalid",
                    "edit-object: the page has no XObject resources",
                ));
            }
            let image = images::store(&mut tx, *placed)?;
            if let Some(turn) = turn {
                tx.update(image, |value| {
                    if let Some(dict) = value.as_dict_mut() {
                        dict.insert(
                            images::TURNED_KEY.to_vec(),
                            Object::Integer(i64::from(turn)),
                        );
                    }
                })?;
            }
            place.update(&mut tx, |value| {
                if let Some(dict) = value.as_dict_mut() {
                    dict.insert(name, Object::Reference(image));
                }
            })?;
            Ok(())
        }
    })?;
    Ok(edit_state_reply(session, request, json!({"page": page})))
}

/// One planned item of edit-objects.
struct Planned {
    element: usize,
    record: usize,
    spans: Vec<Splice>,
}

/// engine.ts `edit-objects`: 2..512 placement edits on one page, every
/// check first, one operation.
pub fn edit_objects(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    let op = "edit-objects";
    session.require_mutable(op)?;
    let page = session.check_page(request.get("page"), op)?;
    let items = request.get("items");
    let list = items.and_then(Value::as_array);
    if list.is_some_and(|l| l.len() > MAX_EDIT_TARGETS) {
        return Err(edit_error(
            "too-large",
            format!("edit-objects: more than {MAX_EDIT_TARGETS} items"),
        ));
    }
    let shaped = |item: &Value| {
        item.get("targetId")
            .and_then(Value::as_str)
            .is_some_and(|id| !id.is_empty())
            && item
                .get("action")
                .and_then(Value::as_str)
                .is_some_and(|a| COMPOUND_ACTIONS.contains(&a))
    };
    let Some(list) = list.filter(|l| l.iter().all(shaped)) else {
        return Err(edit_error(
            "invalid-selection",
            "edit-objects: items must be { targetId, action: move|resize|transform|delete|duplicate, box?, transform? }",
        ));
    };
    if list.len() < 2 {
        return Err(edit_error(
            "invalid-selection",
            "edit-objects: at least two items (one item: use edit-object)",
        ));
    }
    let mut seen = BTreeSet::new();
    for item in list {
        let id = id_of(item);
        if !seen.insert(id) {
            return Err(edit_error(
                "invalid-selection",
                format!("edit-objects: {id} is listed twice"),
            )
            .with_extra(json!({"targetId": id})));
        }
    }
    let clip = session_clip_box(session, page);
    let mut planned: Vec<Planned> = Vec::new();
    let mut duplicates = 0usize;
    let mut last_scan = None;
    for item in list {
        let id = id_of(item);
        let action = item.get("action").and_then(Value::as_str).unwrap_or("");
        let tagged = |error: AppError| error.with_extra(json!({"targetId": id}));
        let single = json!({
            "generation": request.get("generation"),
            "revision": request.get("revision"),
            "page": page,
            "targetId": id,
        });
        let (scan, index) =
            resolve_target(session, &single, page).map_err(|error| tagged(stale(op, error)))?;
        let Some(record) = scan.records.get(index) else {
            return Err(tagged(edit_error(
                "stale-target",
                "edit-objects: the target is gone",
            )));
        };
        let subject = id.to_string();
        if record.kind == Kind::Text || !record.candidate {
            return Err(tagged(not_candidate(op, &subject, record)));
        }
        if !offers(record, action) {
            return Err(tagged(unavailable(op, action, &subject, record)));
        }
        let (Some(placement), Some(element)) = (&record.placement, record.element) else {
            return Err(tagged(edit_error(
                "not-candidate",
                format!("edit-objects: only top-level simple placements are editable ({id})"),
            )));
        };
        let spans = match action {
            "delete" => vec![Splice {
                start: placement.q_start,
                end: placement.q_end,
                replacement: Vec::new(),
            }],
            "duplicate" => {
                let target = check_area(placement)
                    .and_then(|()| object_box(item.get("box"), &clip))
                    .and_then(|target| {
                        let g = rect_affine(&placement.visible, &target);
                        check_after(&placement_after(placement, &g, Some(target)))?;
                        Ok(target)
                    })
                    .map_err(|error| item_error(error, id))?;
                let bytes = scan.streams.get(&record.stream).ok_or_else(|| {
                    tagged(edit_error(
                        "stale-target",
                        "edit-objects: the target is gone",
                    ))
                })?;
                duplicates += 1;
                vec![duplicate_edit(bytes, placement, &target).map_err(tagged)?]
            }
            _ => {
                let write = plan_placement(
                    placement,
                    action,
                    item.get("box"),
                    item.get("transform"),
                    &clip,
                )
                .map_err(|error| item_error(error, id))?;
                if write.noop {
                    continue;
                }
                placement_spans(placement, write.cm.as_deref(), write.clip_text.as_deref())
            }
        };
        planned.push(Planned {
            element,
            record: index,
            spans,
        });
        last_scan = Some(scan);
    }
    let Some(scan) = last_scan else {
        return Ok(edit_state_reply(
            session,
            request,
            json!({"page": page, "noop": true}),
        ));
    };
    if duplicates > 0 && scan.records.len() + duplicates > MAX_EDIT_TARGETS {
        return Err(edit_error(
            "too-large",
            format!(
                "edit-objects: the duplicates would put more than {MAX_EDIT_TARGETS} targets on this page"
            ),
        ));
    }
    let count = planned.len();
    let groups = group_spans(planned.into_iter().map(|p| (p.element, p.record, p.spans)));
    session.mutate("edit objects", |session| {
        write_spans(session, &scan, page, &groups, op).map(|_| ())
    })?;
    Ok(edit_state_reply(
        session,
        request,
        json!({"page": page, "count": count}),
    ))
}

fn id_of(item: &Value) -> &str {
    item.get("targetId").and_then(Value::as_str).unwrap_or("")
}

/// A planning refusal of one item: its own detail (else `out-of-bounds`)
/// and its id.
fn item_error(error: AppError, id: &str) -> AppError {
    let detail = error.detail.unwrap_or_else(|| "out-of-bounds".into());
    edit_error(&detail, format!("edit-objects: {} ({id})", error.message))
        .with_extra(json!({"targetId": id}))
}

/// Spans grouped by element, in first-seen order.
pub fn group_spans(
    items: impl IntoIterator<Item = (usize, usize, Vec<Splice>)>,
) -> Vec<(usize, Vec<SpanEdit>)> {
    let mut groups: Vec<(usize, Vec<SpanEdit>)> = Vec::new();
    for (element, record, spans) in items {
        let edits = spans.into_iter().map(|splice| SpanEdit { record, splice });
        match groups.iter_mut().find(|(e, _)| *e == element) {
            Some((_, group)) => group.extend(edits),
            None => groups.push((element, edits.collect())),
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ops::edit::scan::Clip;

    fn placement(matrix: [f64; 6], clip: Option<[f64; 4]>) -> Placement {
        let full = map_rect(&matrix, &[0.0, 0.0, 1.0, 1.0]);
        let visible = clip.map_or(full, |c| {
            [
                full[0].max(c[0]),
                full[1].max(c[1]),
                full[2].min(c[2]),
                full[3].min(c[3]),
            ]
        });
        Placement {
            cm: 0..0,
            q_start: 0,
            q_end: 0,
            full,
            rect: [0.0, 0.0, 1.0, 1.0],
            view: visible,
            matrix,
            orient: orient_of(&matrix).unwrap(),
            clip: clip.map(|rect| Clip {
                rect,
                start: 0,
                end: 0,
                ops: 0..0,
            }),
            visible,
        }
    }

    #[test]
    fn matrices_are_written_as_engine_ts_writes_them() {
        let p = placement([100.0, 0.0, 0.0, 50.0, 72.0, 600.0], None);
        let cw = plan_placement(
            &p,
            "transform",
            None,
            Some(&json!("rotate-cw")),
            &[0.0, 0.0, 612.0, 792.0],
        )
        .unwrap();
        assert_eq!(cw.cm.as_deref(), Some("0 -100 50 0 97 675"));
        let flip = plan_placement(
            &p,
            "transform",
            None,
            Some(&json!("flip-h")),
            &[0.0, 0.0, 612.0, 792.0],
        )
        .unwrap();
        assert_eq!(flip.cm.as_deref(), Some("-100 0 0 50 172 600"));
        let same = plan_placement(
            &p,
            "move",
            Some(&json!([72, 600, 172, 650])),
            None,
            &[0.0, 0.0, 612.0, 792.0],
        )
        .unwrap();
        assert!(same.noop);
        let crop = plan_placement(
            &p,
            "crop",
            Some(&json!([92, 610, 152, 640])),
            None,
            &[0.0, 0.0, 612.0, 792.0],
        )
        .unwrap();
        assert_eq!(crop.clip_text.as_deref(), Some("92 610 60 30"));
        assert_eq!(crop.cm, None);
    }

    #[test]
    fn hostile_numbers_never_panic() {
        let p = placement([1e-300, 0.0, 0.0, 1e-300, 0.0, 0.0], None);
        for raw in [
            json!([0, 0, 1e308, 1e308]),
            json!([f64::MAX, 0, 1, 1]),
            json!("x"),
            json!([1, 2]),
        ] {
            let _ = plan_placement(&p, "move", Some(&raw), None, &[0.0, 0.0, 612.0, 792.0]);
            let _ = plan_placement(&p, "crop", Some(&raw), None, &[0.0, 0.0, 612.0, 792.0]);
        }
        assert!(
            duplicate_edit(
                b"q",
                &placement([1.0, 0.0, 0.0, 1.0, 0.0, 0.0], None),
                &[0.0, 0.0, 5.0, 5.0]
            )
            .is_ok()
        );
        let mut far = placement([1.0, 0.0, 0.0, 1.0, 0.0, 0.0], None);
        far.q_end = 99;
        assert!(duplicate_edit(b"q", &far, &[0.0, 0.0, 5.0, 5.0]).is_err());
    }

    #[test]
    fn exif_inverse_undoes_each_turn() {
        for o in 1..=8 {
            let back = affine(&exif_unit(o), &exif_unit(exif_inverse(o)));
            assert_eq!(back, crate::app::ops::edit::geometry::IDENTITY, "{o}");
        }
    }

    #[test]
    fn exif_orientation_turns_the_unit_square() {
        for o in 1..=8 {
            let m = exif_unit(o);
            assert_eq!(
                map_rect(&m, &[0.0, 0.0, 1.0, 1.0]),
                [0.0, 0.0, 1.0, 1.0],
                "{o}"
            );
        }
    }
}
