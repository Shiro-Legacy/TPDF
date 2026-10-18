// SPDX-License-Identifier: AGPL-3.0-or-later
//! Tiling patterns whose cell (/BBox) is larger than their step. Reference
//! viewers draw the cell at every step, clipped to /BBox, over its neighbours;
//! hayro renders one step-sized tile from the cell's corner and repeats it,
//! losing whatever lies past the first step. The view moves the pattern's
//! content into a form clipped to the cell and gives the pattern a
//! step-sized /BBox that draws the form at every offset where a cell
//! reaches into it, which hayro's repetition then draws as reference
//! viewers do:
//! nearer copies over farther ones, the copy at the step last. hayro
//! interprets a tile anew for every paint with the pattern, so the copies
//! times the content are bounded per pattern as a per-paint cost.
use crate::object::{Dictionary, Object, ObjectId};

use super::Fixes;

/// Copies of the cell one tile draws, and all tiles of a view together.
const MAX_COPIES: usize = 4096;
const MAX_VIEW_COPIES: usize = 65_536;
/// Copies times content bytes, per pattern: what one paint interprets.
const MAX_WORK: usize = 256 << 10;
const NAME: &[u8] = b"TPDFTile";

impl Fixes<'_, '_> {
    /// Redraws tiling pattern `id` (see the module) when a step is shorter
    /// than its cell, once.
    pub(super) fn fix_tiling(&mut self, id: ObjectId) {
        if !self.recolor.tiled.insert(id) {
            return;
        }
        let Some(dict) = self.stream_dict(id) else {
            return;
        };
        let doc = self.doc;
        let number = |value: &Object| doc.resolve(value).as_f64().filter(|v| v.is_finite());
        let step = |key: &[u8]| number(doc.dict_get(dict, key));
        let (Some(xs), Some(ys)) = (step(b"XStep"), step(b"YStep")) else {
            return;
        };
        let corners: Vec<f64> = (doc.dict_get(dict, b"BBox").as_array().unwrap_or_default())
            .iter()
            .filter_map(number)
            .collect();
        let [a, b, c, d] = corners[..] else {
            return;
        };
        let (x0, y0, x1, y1) = (a.min(c), b.min(d), a.max(c), b.max(d));
        let (width, height) = (x1 - x0, y1 - y0);
        // hayro's own reading of negative steps is left alone.
        if !(xs > 0.0 && ys > 0.0 && width > 0.0 && height > 0.0) || (xs >= width && ys >= height) {
            return;
        }
        let across =
            |extent: f64, step: f64| (extent / step).ceil().min(MAX_COPIES as f64) as usize;
        let (nx, ny) = (across(width, xs), across(height, ys));
        let copies = (nx + 1).saturating_mul(ny + 1);
        let total = self.recolor.tile_copies.saturating_add(copies);
        if copies > MAX_COPIES || total > MAX_VIEW_COPIES {
            return;
        }
        let mut pattern = dict.clone();
        let resources = pattern.remove(b"Resources".as_slice());
        let Some(content) = self.decoded(id) else {
            return;
        };
        if copies.saturating_mul(content.len().max(1)) > MAX_WORK {
            return;
        }
        let Some(form_id) = self.next() else {
            return;
        };
        self.recolor.tile_copies = total;
        let rect = |values: [f64; 4]| Object::Array(values.map(Object::Real).to_vec());
        let mut form = Dictionary::new();
        form.insert(b"Type".to_vec(), Object::name("XObject"));
        form.insert(b"Subtype".to_vec(), Object::name("Form"));
        form.insert(b"BBox".to_vec(), rect([x0, y0, x1, y1]));
        if let Some(resources) = resources {
            form.insert(b"Resources".to_vec(), resources);
        }
        for key in [b"Filter".as_slice(), b"DecodeParms", b"DL"] {
            pattern.remove(key);
        }
        pattern.insert(b"BBox".to_vec(), rect([x0, y0, x0 + xs, y0 + ys]));
        let xobjects = Object::dictionary([(NAME, Object::Reference(form_id))]);
        let tiles = Object::dictionary([("XObject", xobjects)]);
        pattern.insert(b"Resources".to_vec(), tiles);
        // Reference viewers paint the cells in increasing step order, rows
        // outer.
        let mut draw = Vec::new();
        for j in (0..=ny).rev() {
            for i in (0..=nx).rev() {
                let (dx, dy) = (-(i as f64) * xs, -(j as f64) * ys);
                draw.extend_from_slice(format!("q 1 0 0 1 {dx} {dy} cm /").as_bytes());
                draw.extend_from_slice(NAME);
                draw.extend_from_slice(b" Do Q\n");
            }
        }
        self.streams.insert(form_id, (form, content));
        self.streams.insert(id, (pattern, draw));
    }
}
