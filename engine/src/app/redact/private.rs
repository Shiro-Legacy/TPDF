// SPDX-License-Identifier: MIT
//! R3 of the redaction redesign: an image or Form a box touches that is
//! also drawn where no box covers it is copied for the boxed placements
//! instead of refused (copy-on-write). Before apply plans its edits, every
//! XObject placement on a marked page whose drawing a box touches (an
//! image a box touches; a Form with anything a box touches inside it, at any
//! depth) gets its own copy under a new resource name: the page's content
//! names the copy, and so does a copied Form's content for its own children.
//! The originals are left as they are for every other placement (other
//! pages, annotation appearances, patterns, masks); apply then rewrites the
//! copies in place. Copies keep the original's bytes and filters, so an
//! image the view decodes is decoded through its original's number
//! (`Private::origins`).
use std::collections::{BTreeMap, BTreeSet};

use crate::app::edit::{Builder, Edit};
use crate::app::info;
use crate::app::pagetree;
use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId};
use crate::view::ViewDocument;

use super::apply::ApplyError;
use super::content::{Detail, DrawKind, Limits, WalkError};
use super::marks::Mark;
use super::region::{self, GlyphRule};

/// Copies one apply makes at most.
const MAX_COPIES: usize = 1 << 16;
/// Stream bytes one apply copies at most.
const MAX_COPY_BYTES: usize = 256 << 20;

/// What `privatize` did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Private {
    /// Each copy and the object it was copied from.
    pub origins: BTreeMap<ObjectId, ObjectId>,
}

fn walk_error(page: usize, error: WalkError) -> ApplyError {
    match error {
        WalkError::TooComplex(_) => ApplyError::TooComplex(page),
        WalkError::Unreadable(why) => ApplyError::Unreadable(page, why),
    }
}

/// A resource name not in `taken`.
fn fresh(taken: &mut BTreeSet<Vec<u8>>, counter: &mut usize) -> Vec<u8> {
    loop {
        *counter += 1;
        let name = format!("RdCopy{counter}").into_bytes();
        if taken.insert(name.clone()) {
            return name;
        }
    }
}

/// A resources dictionary as a direct copy (resolved), with its /XObject
/// subdictionary resolved too.
fn own_resources(doc: &Document<'_>, value: Option<&Object>) -> Dictionary {
    let mut out = value
        .and_then(|v| doc.resolve(v).as_dict())
        .cloned()
        .unwrap_or_default();
    let xobjects = out
        .get(b"XObject".as_slice())
        .and_then(|v| doc.resolve(v).as_dict())
        .cloned()
        .unwrap_or_default();
    out.insert(b"XObject".to_vec(), Object::Dictionary(xobjects));
    out
}

fn add_xobject(resources: &mut Dictionary, name: Vec<u8>, id: ObjectId) {
    if let Some(Object::Dictionary(x)) = resources.get_mut(b"XObject".as_slice()) {
        x.insert(name, Object::Reference(id));
    }
}

fn names_of(resources: &Dictionary) -> BTreeSet<Vec<u8>> {
    match resources.get(b"XObject".as_slice()) {
        Some(Object::Dictionary(x)) => x.keys().cloned().collect(),
        _ => BTreeSet::new(),
    }
}

/// A copy being made: its original, the name edits to its content (decoded
/// offsets) and its own resources.
struct Copy {
    origin: ObjectId,
    /// The draw it is copied for.
    draw: usize,
    /// Names added to its resources.
    added: BTreeSet<Vec<u8>>,
    form: bool,
    splices: Vec<(usize, usize, Vec<u8>)>,
    resources: Option<Dictionary>,
}

/// Gives every placement a box touches on the marked pages its own copy of
/// the XObject it draws (see the module note). Pages are rewritten with one
/// new content stream and their own resources.
#[allow(clippy::too_many_lines)]
pub fn privatize(
    target: &mut Builder,
    view: Option<&ViewDocument>,
    marks: &[Mark],
) -> Result<Private, ApplyError> {
    let mut out = Private::default();
    let pages: BTreeSet<usize> = marks.iter().map(|m| m.page).collect();
    let mut counter = 0usize;
    let mut copied = 0usize;
    for page in pages {
        let Some(plan) = plan_page(target.document(), view, marks, page)? else {
            continue;
        };
        let PagePlan {
            page_id,
            joined,
            base,
            mut page_resources,
            draws,
            streams,
            uses,
        } = plan;
        let mut page_added: BTreeSet<Vec<u8>> = BTreeSet::new();
        let mut page_names = names_of(&page_resources);
        let mut page_splices: Vec<(usize, usize, Vec<u8>)> = Vec::new();
        // Draw index -> its copy (an index into `copies`).
        let mut copy_of: BTreeMap<usize, usize> = BTreeMap::new();
        let mut copies: Vec<(ObjectId, Copy)> = Vec::new();
        for planned in draws {
            copied = copied.saturating_add(planned.bytes);
            if copies.len() >= MAX_COPIES || copied > MAX_COPY_BYTES {
                return Err(ApplyError::TooLarge(
                    "the pictures and drawings under the boxes are too large to copy".into(),
                ));
            }
            let id = target.allocate(Object::Null)?;
            match planned.parent {
                None => {
                    let start = match planned.from {
                        Some((e, o)) => base.get(e).map(|b| b + o),
                        None => base.get(planned.element).map(|b| b + planned.start),
                    };
                    let end = base.get(planned.element).map(|b| b + planned.op_start);
                    let (Some(start), Some(end)) = (start, end) else {
                        continue;
                    };
                    let name = fresh(&mut page_names, &mut counter);
                    page_splices.push((start, end, operand(&name)));
                    page_added.insert(name.clone());
                    add_xobject(&mut page_resources, name, id);
                }
                Some(p) => {
                    let Some((_, parent)) = copy_of.get(&p).and_then(|&c| copies.get_mut(c)) else {
                        continue;
                    };
                    let Some(resources) = parent.resources.as_mut() else {
                        continue;
                    };
                    let mut taken = names_of(resources);
                    let name = fresh(&mut taken, &mut counter);
                    parent
                        .splices
                        .push((planned.start, planned.op_start, operand(&name)));
                    parent.added.insert(name.clone());
                    add_xobject(resources, name, id);
                }
            }
            copy_of.insert(planned.index, copies.len());
            copies.push((
                id,
                Copy {
                    origin: planned.object,
                    draw: planned.index,
                    added: BTreeSet::new(),
                    form: planned.form,
                    splices: Vec::new(),
                    resources: planned.resources,
                },
            ));
        }
        // Each resources dictionary keeps the names its content still draws
        // and the copies' names: an original the page or a copied Form no
        // longer draws is not reachable from it (the raw checks read what
        // the marked pages' resources reach).
        let kept = |parent: Option<usize>| -> BTreeSet<Vec<u8>> {
            (uses.iter())
                .filter(|(index, p, _)| *p == parent && !copy_of.contains_key(index))
                .map(|(_, _, name)| name.clone())
                .collect()
        };
        retain_xobjects(&mut page_resources, &kept(None), &page_added);
        for (_, copy) in &mut copies {
            let still = kept(Some(copy.draw));
            if let Some(resources) = copy.resources.as_mut() {
                retain_xobjects(resources, &still, &copy.added);
            }
        }
        // Write the copies.
        for (id, copy) in copies {
            let (mut dict, data) = {
                let doc = target.document();
                let dict = doc
                    .stream_dict(copy.origin)
                    .map_err(|e| ApplyError::Write(e.to_string()))?
                    .clone();
                let data = doc
                    .stream_data(copy.origin)
                    .map_err(|e| ApplyError::Write(e.to_string()))?
                    .unwrap_or(&[])
                    .to_vec();
                (dict, data)
            };
            dict.remove(b"Length".as_slice());
            let data = match (copy.form, copy.splices.is_empty()) {
                (true, false) => {
                    let decoded = streams.get(&copy.origin).ok_or_else(|| {
                        ApplyError::Unreadable(page, "a drawing could not be read back".into())
                    })?;
                    let body = splice(decoded, copy.splices)?;
                    dict.remove(b"DecodeParms".as_slice());
                    dict.insert(b"Filter".to_vec(), Object::name("FlateDecode"));
                    miniz_oxide::deflate::compress_to_vec_zlib(&body, 6)
                }
                _ => data,
            };
            if let (true, Some(resources)) = (copy.form, copy.resources) {
                dict.insert(b"Resources".to_vec(), Object::Dictionary(resources));
            }
            target.put_stream_raw(id, dict, data)?;
            out.origins.insert(id, copy.origin);
        }
        // The page: one content stream naming the copies, its own resources.
        let body = splice(&joined, page_splices)?;
        let mut dict = Dictionary::new();
        dict.insert(b"Filter".to_vec(), Object::name("FlateDecode"));
        let stream = target.allocate(Object::Null)?;
        target.put_stream_raw(
            stream,
            dict,
            miniz_oxide::deflate::compress_to_vec_zlib(&body, 6),
        )?;
        target.update(page_id, |value| {
            if let Some(dict) = value.as_dict_mut() {
                dict.insert(b"Contents".to_vec(), Object::Reference(stream));
                dict.insert(b"Resources".to_vec(), Object::Dictionary(page_resources));
            }
        })?;
    }
    Ok(out)
}

/// Whether a Form draw from `parent` up has no usable /BBox (missing, or
/// without width or height).
fn unbounded(draws: &[super::content::Draw], mut parent: Option<usize>) -> bool {
    let mut depth = 0;
    while let Some(p) = parent {
        let Some(draw) = draws.get(p) else {
            return false;
        };
        if let DrawKind::Form { bbox, .. } = &draw.kind {
            let usable = bbox.is_some_and(|b| b[2] - b[0] != 0.0 && b[3] - b[1] != 0.0);
            if !usable {
                return true;
            }
        }
        parent = draw.parent;
        depth += 1;
        if depth > 64 {
            return true;
        }
    }
    false
}

/// Keeps in `resources`' /XObject only the names in `kept` or `added`.
fn retain_xobjects(
    resources: &mut Dictionary,
    kept: &BTreeSet<Vec<u8>>,
    added: &BTreeSet<Vec<u8>>,
) {
    if let Some(Object::Dictionary(x)) = resources.get_mut(b"XObject".as_slice()) {
        x.retain(|name, _| kept.contains(name) || added.contains(name));
    }
}

/// A `Do` operand's name (`/Name`, `#xx` escapes read).
fn operand_name(bytes: &[u8]) -> Vec<u8> {
    let text = bytes.trim_ascii();
    let text = text.strip_prefix(b"/").unwrap_or(text);
    let mut out = Vec::with_capacity(text.len());
    let mut i = 0;
    while let Some(&c) = text.get(i) {
        let hex = |d: u8| (d as char).to_digit(16);
        if c == b'#' {
            if let (Some(a), Some(b)) = (
                text.get(i + 1).and_then(|d| hex(*d)),
                text.get(i + 2).and_then(|d| hex(*d)),
            ) {
                out.push((a * 16 + b) as u8);
                i += 3;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

/// `/name ` as a `Do` operand.
fn operand(name: &[u8]) -> Vec<u8> {
    let mut out = b"/".to_vec();
    out.extend_from_slice(name);
    out.push(b' ');
    out
}

/// A placement to copy, read before anything is written.
struct Planned {
    index: usize,
    object: ObjectId,
    form: bool,
    parent: Option<usize>,
    element: usize,
    from: Option<(usize, usize)>,
    start: usize,
    op_start: usize,
    /// A Form's own resources (direct, resolved; the page's when it has
    /// none), for its children's copies.
    resources: Option<Dictionary>,
    bytes: usize,
}

/// One marked page's placements to copy, and what rewriting it needs.
struct PagePlan {
    page_id: ObjectId,
    joined: Vec<u8>,
    base: Vec<usize>,
    page_resources: Dictionary,
    draws: Vec<Planned>,
    streams: BTreeMap<ObjectId, std::sync::Arc<Vec<u8>>>,
    /// Every XObject draw: its index, the Form draw it is inside and the
    /// name its `Do` uses.
    uses: Vec<(usize, Option<usize>, Vec<u8>)>,
}

/// Reads page `page`: which XObject placements a box touches (an image a
/// box touches; a Form with something a box touches inside it); None when
/// there are none.
fn plan_page(
    doc: &Document<'static>,
    view: Option<&ViewDocument>,
    marks: &[Mark],
    page: usize,
) -> Result<Option<PagePlan>, ApplyError> {
    let rule = GlyphRule::new(doc, marks);
    let regions = rule.drawn();
    let pages = info::pages(doc);
    let Some(page_ref) = pages.get(page) else {
        return Ok(None);
    };
    let Some(page_id) = page_ref.id else {
        return Ok(None);
    };
    let content = region::page_content(doc, view, page, Detail::Full, Limits::DEFAULT)
        .map_err(|e| walk_error(page, e))?;
    let mut needs = vec![false; content.draws.len()];
    for (i, draw) in content.draws.iter().enumerate() {
        let touched = match &draw.kind {
            DrawKind::Text(text) if text.measured => {
                (text.glyphs.iter()).any(|g| rule.removes_glyph(page, &g.bounds, &g.quad))
            }
            DrawKind::Text(_) => rule.reaches(page, &draw.bounds),
            DrawKind::Form { .. } => false,
            // A shading in a Form without a usable /BBox may paint anywhere
            // at another placement (as `region::shared` compares it).
            DrawKind::Shading => {
                regions.touches(page, &draw.bounds) || unbounded(&content.draws, draw.parent)
            }
            _ => regions.touches(page, &draw.bounds),
        };
        if rule.spent() {
            return Err(ApplyError::TooComplex(page));
        }
        if !touched {
            continue;
        }
        if matches!(
            draw.kind,
            DrawKind::Image {
                object: Some(_),
                ..
            }
        ) {
            needs[i] = true;
        }
        let mut parent = draw.parent;
        while let Some(p) = parent {
            match needs.get_mut(p) {
                Some(need) if !*need => *need = true,
                _ => break,
            }
            parent = content.draws.get(p).and_then(|d| d.parent);
        }
    }
    if !needs.iter().any(|n| *n) {
        return Ok(None);
    }
    let inherited = pagetree::inherited(doc, page_ref.dict, b"Resources");
    let mut draws = Vec::new();
    for (i, draw) in content.draws.iter().enumerate() {
        if !needs[i] {
            continue;
        }
        let (object, form) = match &draw.kind {
            DrawKind::Image {
                object: Some(id), ..
            } => (*id, false),
            DrawKind::Form {
                object,
                entered: true,
                ..
            } => (*object, true),
            _ => continue,
        };
        let resources = form.then(|| {
            let own = (doc.stream_dict(object).ok())
                .and_then(|d| d.get(b"Resources".as_slice()))
                .or(inherited);
            own_resources(doc, own)
        });
        draws.push(Planned {
            index: i,
            object,
            form,
            parent: draw.parent,
            element: draw.element,
            from: draw.from,
            start: draw.start,
            op_start: draw.op.start,
            resources,
            bytes: doc
                .stream_data(object)
                .ok()
                .flatten()
                .map_or(0, <[u8]>::len),
        });
    }
    let mut base = Vec::with_capacity(content.elements.len());
    let mut joined: Vec<u8> = Vec::new();
    for (i, stream) in content.elements.iter().enumerate() {
        if i > 0 {
            joined.push(b'\n');
        }
        base.push(joined.len());
        if let Some(bytes) = content.streams.get(stream) {
            joined.extend_from_slice(bytes);
        }
    }
    let mut uses = Vec::new();
    for (i, draw) in content.draws.iter().enumerate() {
        if draw.kind.xobject().is_none() {
            continue;
        }
        let bytes = match draw.parent {
            None => {
                let start = match draw.from {
                    Some((e, o)) => base.get(e).map(|b| b + o),
                    None => base.get(draw.element).map(|b| b + draw.start),
                };
                let end = base.get(draw.element).map(|b| b + draw.op.start);
                start.zip(end).and_then(|(s, e)| joined.get(s..e))
            }
            Some(_) => {
                (content.streams.get(&draw.stream)).and_then(|b| b.get(draw.start..draw.op.start))
            }
        };
        if let Some(bytes) = bytes {
            uses.push((i, draw.parent, operand_name(bytes)));
        }
    }
    Ok(Some(PagePlan {
        page_id,
        uses,
        joined,
        base,
        page_resources: own_resources(doc, inherited),
        draws,
        streams: content.streams,
    }))
}

/// `bytes` with each `(start, end, replacement)`; overlapping edits refuse.
fn splice(bytes: &[u8], mut edits: Vec<(usize, usize, Vec<u8>)>) -> Result<Vec<u8>, ApplyError> {
    edits.sort_by_key(|(start, _, _)| std::cmp::Reverse(*start));
    let mut previous = bytes.len();
    let mut out = bytes.to_vec();
    for (start, end, replacement) in edits {
        if end > previous || end < start {
            return Err(ApplyError::Unsupported(
                "a drawing's name could not be rewritten".into(),
            ));
        }
        previous = start;
        out.splice(start..end, replacement);
    }
    Ok(out)
}
