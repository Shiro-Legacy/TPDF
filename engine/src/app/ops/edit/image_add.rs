// SPDX-License-Identifier: MIT
//! `image-add`: a PNG or JPEG drawn into a page's content as a simple
//! placement (`q <cm> cm /Name Do Q`, the block paste writes), so the
//! picture is page content that edit-object can then move, resize, turn,
//! crop, replace or delete like any other.
//!
//! Without a `box` the picture gets its natural size (its declared
//! resolution, else 72 dpi), shrunk to half the page each way, centred on
//! `at` (default user space) or the page and kept on the page. The
//! picture is drawn upright as the page is shown: its EXIF orientation and
//! the page's /Rotate are taken into the `cm`. A page with a /UserUnit
//! other than 1 is refused (its pictures are not editable).
use serde_json::{Value, json};

use super::geometry::{affine, round2, writable};
use super::object::{
    check_box, exif_unit, matrix_text, page_quarters, page_resources, read_image, session_clip_box,
};
use super::{MAX_EDIT_TARGETS, edit_error, edit_state_reply, scan_for};
use crate::app::edit::Place;
use crate::app::ops::pages::images::{self, Placed};
use crate::app::request;
use crate::app::{AppError, AppSession, Buffers};
use crate::object::{Dictionary, Object};

const OP: &str = "image-add";
/// The share of the page (each way) a picture without a box may cover.
const FIT_SHARE: f64 = 0.5;

/// Whether the page has a /UserUnit other than 1 (the scan's
/// `objects_blocked`).
fn scaled(session: &AppSession, page: usize) -> bool {
    let Some(doc) = session.doc.as_ref() else {
        return false;
    };
    let Some((_, dict)) = doc.page(page) else {
        return false;
    };
    super::font::number(super::font::at(doc.document(), Some(dict), b"UserUnit"))
        .is_some_and(|u| u != 1.0)
}

/// The unit square turned so it reads upright on a page shown turned
/// clockwise `quarters` times (the picture is turned back the other way).
fn turn_unit(quarters: u8) -> [f64; 6] {
    match quarters {
        1 => exif_unit(8),
        2 => exif_unit(3),
        3 => exif_unit(6),
        _ => [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
    }
}

/// The picture's upright size in pixels (EXIF 5..8 swap the sides).
fn upright_pixels(placed: &Placed) -> (f64, f64) {
    match placed.orientation {
        5..=8 => (f64::from(placed.height), f64::from(placed.width)),
        _ => (f64::from(placed.width), f64::from(placed.height)),
    }
}

/// The default box: natural size within half the page, centred on `at`
/// (or the page), kept inside `clip`. All in default user space.
fn default_box(placed: &Placed, clip: &[f64; 4], quarters: u8, at: Option<[f64; 2]>) -> [f64; 4] {
    // engine.ts: a declared resolution other than 96, else 72.
    let dpi = if placed.dpi > 0 && placed.dpi != 96 {
        f64::from(placed.dpi)
    } else {
        72.0
    };
    let (across, down) = upright_pixels(placed);
    let (across, down) = (across * 72.0 / dpi, down * 72.0 / dpi);
    // The page as shown: a quarter turn swaps its sides.
    let (page_w, page_h) = (clip[2] - clip[0], clip[3] - clip[1]);
    let (shown_w, shown_h) = match quarters % 2 {
        1 => (page_h, page_w),
        _ => (page_w, page_h),
    };
    let shrink = 1f64
        .min(shown_w * FIT_SHARE / across)
        .min(shown_h * FIT_SHARE / down);
    let (w, h) = (
        round2((across * shrink).max(1.0)),
        round2((down * shrink).max(1.0)),
    );
    // Back to user space.
    let (w, h) = match quarters % 2 {
        1 => (h, w),
        _ => (w, h),
    };
    let [cx, cy] = at.unwrap_or([(clip[0] + clip[2]) / 2.0, (clip[1] + clip[3]) / 2.0]);
    let x0 = (cx - w / 2.0).min(clip[2] - w).max(clip[0]);
    let y0 = (cy - h / 2.0).min(clip[3] - h).max(clip[1]);
    [
        round2(x0),
        round2(y0),
        round2(x0 + w).min(clip[2]),
        round2(y0 + h).min(clip[3]),
    ]
}

/// The first `Im<n>` the page's XObject resources do not hold.
fn free_name(xobjects: Option<&Dictionary>) -> String {
    (1u64..)
        .map(|n| format!("Im{n}"))
        .find(|name| !xobjects.is_some_and(|x| x.contains_key(name.as_bytes())))
        .unwrap_or_else(|| "Im0".into())
}

/// `image-add {page, image: {name, bytes}, box?, at?}`.
pub fn image_add(
    session: &mut AppSession,
    request: &Value,
    buffers: &Buffers,
) -> Result<Value, AppError> {
    session.require_mutable(OP)?;
    let page = session.check_page(request.get("page"), OP)?;
    let placed = read_image(OP, request.get("image"), buffers)?;
    if scaled(session, page) {
        return Err(edit_error(
            "unsupported-transform",
            format!("{OP}: pages with a /UserUnit take no added pictures"),
        ));
    }
    let clip = session_clip_box(session, page);
    let quarters = page_quarters(session, page);
    let at = match request.get("at") {
        None | Some(Value::Null) => None,
        Some(_) => Some(
            request::numbers(request, "at")
                .filter(|p| p.len() == 2 && p.iter().all(|v| v.is_finite()))
                .map(|p| [p[0], p[1]])
                .ok_or_else(|| {
                    edit_error("invalid", format!("{OP}: at must be two finite numbers"))
                })?,
        ),
    };
    let target = match request.get("box") {
        None | Some(Value::Null) => check_box(&default_box(&placed, &clip, quarters, at), &clip),
        Some(_) => super::object::object_box(request.get("box"), &clip),
    }
    .map_err(|error| AppError {
        message: error.message.replace("edit-object", OP),
        ..error
    })?;
    let scan = scan_for(session, page)?;
    if !scan.balanced {
        return Err(edit_error(
            "unbalanced-content",
            format!("{OP}: existing page content has unbalanced graphics state"),
        ));
    }
    if scan.records.len() + 1 > MAX_EDIT_TARGETS {
        return Err(edit_error(
            "too-large",
            format!("{OP}: a picture would put more than {MAX_EDIT_TARGETS} targets on this page"),
        ));
    }
    // The unit square: stored pixels upright (EXIF), turned against the
    // page's /Rotate, then stretched onto the box.
    let [x0, y0, x1, y1] = target;
    let cm = affine(
        &affine(&exif_unit(placed.orientation), &turn_unit(quarters)),
        &[x1 - x0, 0.0, 0.0, y1 - y0, x0, y0],
    );
    if !cm.into_iter().all(writable) {
        return Err(edit_error(
            "unsupported-transform",
            format!("{OP}: the placement's numbers would be out of range"),
        ));
    }
    let name = {
        let doc = session
            .doc
            .as_ref()
            .ok_or_else(|| AppError::plain("image-add: no document open"))?;
        let (_, dict) = doc
            .page(page)
            .ok_or_else(|| AppError::reason("invalid", "image-add: no such page"))?;
        let resources = page_resources(doc.document(), dict);
        free_name(super::font::dict_at(doc.document(), resources, b"XObject"))
    };
    let orientation = placed.orientation;
    let block = format!("\nq\n{} cm\n/{name} Do\nQ\n", matrix_text(&cm));
    session.mutate("add image", |session| {
        let mut tx = session.tx()?;
        let page_id = super::write::page_object(tx.document(), page, OP)?;
        let (mut resources, mut xobjects, contents) = {
            let doc = tx.document();
            let dict = doc.resolve_id(page_id).as_dict().ok_or_else(|| {
                AppError::reason("invalid", "image-add: the page is not a dictionary")
            })?;
            let resources = page_resources(doc, dict).cloned().unwrap_or_default();
            let xobjects = super::font::dict_at(doc, Some(&resources), b"XObject")
                .cloned()
                .unwrap_or_default();
            let contents: Vec<Object> = match dict.get(b"Contents".as_slice()) {
                None => Vec::new(),
                Some(value) => match doc.resolve(value) {
                    Object::Array(items) => items.clone(),
                    Object::Null => Vec::new(),
                    _ => vec![value.clone()],
                },
            };
            (resources, xobjects, contents)
        };
        let image = images::store(&mut tx, placed)?;
        if (2..=8).contains(&orientation) {
            // A later replace takes the EXIF turn back out (edit-object).
            tx.update(image, |value| {
                if let Some(dict) = value.as_dict_mut() {
                    dict.insert(
                        images::TURNED_KEY.to_vec(),
                        Object::Integer(i64::from(orientation)),
                    );
                }
            })?;
        }
        xobjects.insert(name.as_bytes().to_vec(), Object::Reference(image));
        resources.insert(b"XObject".to_vec(), Object::Dictionary(xobjects));
        let stream = tx.allocate(
            Object::Dictionary(Dictionary::new()),
            Some(block.into_bytes()),
        )?;
        let mut contents = contents;
        contents.push(Object::Reference(stream));
        Place::object(page_id).update(&mut tx, |value| {
            if let Some(dict) = value.as_dict_mut() {
                dict.insert(b"Resources".to_vec(), Object::Dictionary(resources));
                dict.insert(b"Contents".to_vec(), Object::Array(contents));
            }
        })?;
        Ok(())
    })?;
    // The add has landed; a failed lookup scan must not turn it into a refusal.
    let target_id = scan_for(session, page).ok().and_then(|scan| {
        scan.records
            .iter()
            .rev()
            .find(|r| r.object_name.as_deref() == Some(name.as_bytes()) && r.candidate)
            .map(|r| r.target_id.clone())
    });
    Ok(edit_state_reply(
        session,
        request,
        json!({"page": page, "name": name, "box": target, "targetId": target_id}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where the unit square's corners go under `m`.
    fn corners(m: [f64; 6]) -> [[f64; 2]; 3] {
        let at = |u: f64, v: f64| [m[0] * u + m[2] * v + m[4], m[1] * u + m[3] * v + m[5]];
        [at(0.0, 0.0), at(1.0, 0.0), at(0.0, 1.0)]
    }

    #[test]
    fn turned_units_read_upright() {
        // Shown a quarter turn clockwise: the picture's right edge runs up
        // the page (shown as right), its top to the page's left (shown up).
        assert_eq!(corners(turn_unit(1)), [[1.0, 0.0], [1.0, 1.0], [0.0, 0.0]]);
        assert_eq!(corners(turn_unit(2)), [[1.0, 1.0], [0.0, 1.0], [1.0, 0.0]]);
        assert_eq!(corners(turn_unit(3)), [[0.0, 1.0], [0.0, 0.0], [1.0, 1.0]]);
    }
}
