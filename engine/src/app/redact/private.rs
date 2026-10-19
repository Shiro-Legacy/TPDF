// SPDX-License-Identifier: MIT
//! R3 of the redaction redesign, in the form the third review asked for
//! (uncertainty fails closed): every image and Form apply changes is
//! redacted IN PLACE, its edits merged from every placement a box touches,
//! and what a flattened page drew is emptied in place, so every reference a
//! walk cannot prove (a resources dictionary other pages share or inherit,
//! an unused pattern, font or soft mask, an annotation appearance, a page
//! that cannot be read) sees the redacted object. Only a placement a walk
//! PROVES is drawn visibly and untouched gets a private, untouched copy:
//!
//! - it is drawn by a page's content (annotation appearances never count),
//!   at any depth, and neither it nor any Form it is inside is touched by a
//!   box (`touched`) or left unentered;
//! - its matrix is finite and not singular, and its box cut to its clip and
//!   the page box has area (so is each Form's it is inside);
//! - it is not inside optional content (a `BDC` tagged /OC, or /OC on it or
//!   a Form it is inside), and a Form is not drawn in text render mode 3 or
//!   7 (`Draw::hidden`).
//!
//! The placement and the Forms it is inside are copied under new resource
//! names; the page gets one new content stream naming them and its own
//! resources (a shared or inherited dictionary is left to the others).
//! Apply then rewrites the originals; it never reaches the copies, since
//! nothing a box touches draws them.
use std::collections::{BTreeMap, BTreeSet};

use crate::app::annots::PageSpace;
use crate::app::edit::{Builder, Edit};
use crate::app::info;
use crate::app::pagetree;
use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId};
use crate::view::ViewDocument;

use super::apply::ApplyError;
use super::content::{Content, Detail, Draw, DrawKind, Limits, WalkError};
use super::marks::Mark;
use super::region::{self, GlyphRule};

/// Copies one apply makes at most.
const MAX_COPIES: usize = 1 << 16;
/// Stream bytes one apply copies at most.
const MAX_COPY_BYTES: usize = 256 << 20;

/// What `protect` did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Protected {
    /// The untouched copies made, each with its original.
    pub copies: BTreeMap<ObjectId, ObjectId>,
    /// (original, page) for each placement proven and copied.
    pub proven: BTreeSet<(ObjectId, usize)>,
    /// Pages that could not be walked: their placements see the redacted
    /// originals.
    pub unread: BTreeSet<usize>,
}

/// A resource name not in `taken`.
fn fresh(taken: &mut BTreeSet<Vec<u8>>, counter: &mut usize) -> Vec<u8> {
    loop {
        *counter += 1;
        let name = format!("RdKeep{counter}").into_bytes();
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

fn walk_error(page: usize, error: WalkError) -> ApplyError {
    match error {
        WalkError::TooComplex(_) => ApplyError::TooComplex(page),
        WalkError::Unreadable(why) => ApplyError::Unreadable(page, why),
    }
}

/// Which draws of `content` a box touches (`rule`), or hold something a
/// box touches inside them: apply changes their objects.
pub fn touched(content: &Content, rule: Option<&GlyphRule>, page: usize) -> Vec<bool> {
    let mut needs = vec![false; content.draws.len()];
    let Some(rule) = rule else {
        return needs;
    };
    let regions = rule.drawn();
    for (i, draw) in content.draws.iter().enumerate() {
        // Past the rule's budget the caller refuses (`GlyphRule::spent`).
        if rule.spent() {
            break;
        }
        let hit = match &draw.kind {
            DrawKind::Text(text) if text.measured => (text.glyphs.iter())
                .any(|g| rule.spent() || rule.removes_glyph(page, &g.bounds, &g.quad)),
            DrawKind::Text(_) => rule.reaches(page, &draw.bounds),
            DrawKind::Form { .. } => false,
            _ => regions.touches(page, &draw.bounds),
        };
        if !hit {
            continue;
        }
        needs[i] = true;
        let mut parent = draw.parent;
        while let Some(p) = parent {
            match needs.get_mut(p) {
                Some(need) if !*need => *need = true,
                _ => break,
            }
            parent = content.draws.get(p).and_then(|d| d.parent);
        }
    }
    needs
}

/// The images and Forms apply changes on the marked pages of `marks`: each
/// draw of an image or Form whose drawing a box touches (`touched`).
pub fn changed_objects(
    doc: &Document<'static>,
    view: Option<&ViewDocument>,
    marks: &[Mark],
) -> Result<BTreeSet<ObjectId>, ApplyError> {
    let rule = GlyphRule::new(doc, marks);
    let pages: BTreeSet<usize> = marks.iter().map(|m| m.page).collect();
    let mut out = BTreeSet::new();
    for page in pages {
        let content = region::page_content(doc, view, page, Detail::Full, Limits::DEFAULT)
            .map_err(|e| walk_error(page, e))?;
        let needs = touched(&content, Some(&rule), page);
        if rule.spent() {
            return Err(ApplyError::TooComplex(page));
        }
        for (draw, need) in content.draws.iter().zip(&needs) {
            if *need {
                out.extend(draw.kind.xobject().map(|(id, _)| id));
            }
        }
    }
    Ok(out)
}

/// A draw a walk proves visible: finite, non-singular matrix; its box cut
/// to its clip and the page box has area; not hidden.
fn visible(draw: &Draw, area: [f64; 4]) -> bool {
    let matrix = match &draw.kind {
        DrawKind::Form { matrix, .. } | DrawKind::Image { matrix, .. } => *matrix,
        _ => draw.ctm,
    };
    let det = matrix[0] * matrix[3] - matrix[1] * matrix[2];
    if draw.hidden || !det.is_finite() || det == 0.0 || matrix.iter().any(|v| !v.is_finite()) {
        return false;
    }
    let b = draw.bounds;
    let c = draw.clip.unwrap_or(area);
    let x0 = b[0].max(c[0]).max(area[0]);
    let y0 = b[1].max(c[1]).max(area[1]);
    let x1 = b[2].min(c[2]).min(area[2]);
    let y1 = b[3].min(c[3]).min(area[3]);
    x0 < x1 && y0 < y1
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
    /// The placement is proven (not only a Form on the way to one).
    proven: bool,
    /// The name its `Do` uses, and whether another draw in the same
    /// content uses it for the original (then the copy takes a new name;
    /// else the name itself is pointed at the copy, which keeps the view's
    /// measuring of its text by name).
    name: Vec<u8>,
    rename: bool,
}

/// One page's placements to copy, and what rewriting it needs.
struct PagePlan {
    page_id: ObjectId,
    joined: Vec<u8>,
    base: Vec<usize>,
    page_resources: Dictionary,
    draws: Vec<Planned>,
    streams: BTreeMap<ObjectId, std::sync::Arc<Vec<u8>>>,
}

/// Reads page `page`: the untouched, visible draws of `targets` whose
/// Forms on the way are untouched, entered and visible, and those Forms.
/// None when there are none; an error when the page cannot be walked.
#[allow(clippy::too_many_lines)]
fn plan_page(
    doc: &Document<'static>,
    view: Option<&ViewDocument>,
    rule: Option<&GlyphRule>,
    targets: &BTreeSet<ObjectId>,
    page: usize,
) -> Result<Option<PagePlan>, ()> {
    let pages = info::pages(doc);
    let Some(page_ref) = pages.get(page) else {
        return Ok(None);
    };
    let area = PageSpace::of(doc, page_ref.dict).bounds();
    let content =
        region::page_content(doc, view, page, Detail::Full, Limits::DEFAULT).map_err(|_| ())?;
    let needs = touched(&content, rule, page);
    if rule.is_some_and(GlyphRule::spent) {
        return Err(());
    }
    let mut copy = vec![false; content.draws.len()];
    let mut proven = vec![false; content.draws.len()];
    for (i, draw) in content.draws.iter().enumerate() {
        let Some((object, _)) = draw.kind.xobject() else {
            continue;
        };
        if !targets.contains(&object)
            || needs.get(i).copied().unwrap_or(true)
            || !visible(draw, area)
            || matches!(draw.kind, DrawKind::Form { entered: false, .. })
        {
            continue;
        }
        // Every Form on the way: untouched, entered, visible.
        let mut chain = Vec::new();
        let mut parent = draw.parent;
        let mut fine = true;
        while let Some(p) = parent {
            let Some(up) = content.draws.get(p) else {
                fine = false;
                break;
            };
            if needs.get(p).copied().unwrap_or(true)
                || !matches!(up.kind, DrawKind::Form { entered: true, .. })
                || !visible(up, area)
            {
                fine = false;
                break;
            }
            chain.push(p);
            parent = up.parent;
        }
        if !fine {
            continue;
        }
        copy[i] = true;
        proven[i] = true;
        for p in chain {
            copy[p] = true;
        }
    }
    if !copy.iter().any(|c| *c) {
        return Ok(None);
    }
    // A page written into /Kids cannot be given its own content.
    let page_id = page_ref.id.ok_or(())?;
    let inherited = pagetree::inherited(doc, page_ref.dict, b"Resources");
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
    // Each XObject draw's name, and the names of the draws left as they are
    // by container (the page, or a Form draw).
    let name_of = |draw: &Draw| -> Option<Vec<u8>> {
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
        bytes.map(operand_name)
    };
    // The content whose resources a draw's name is read in: its Form's, or
    // the page's when the Form has none of its own (up the chain).
    let container = |draw: &Draw| -> Option<usize> {
        let mut parent = draw.parent;
        while let Some(p) = parent {
            let own = (content.draws.get(p))
                .and_then(|d| d.kind.xobject())
                .and_then(|(id, _)| doc.stream_dict(id).ok())
                .is_some_and(|d| d.get(b"Resources".as_slice()).is_some());
            if own {
                return Some(p);
            }
            parent = content.draws.get(p).and_then(|d| d.parent);
        }
        None
    };
    let mut left: BTreeSet<(Option<usize>, Vec<u8>)> = BTreeSet::new();
    for (i, draw) in content.draws.iter().enumerate() {
        if draw.kind.xobject().is_some() && !copy[i] {
            if let Some(name) = name_of(draw) {
                left.insert((container(draw), name));
            }
        }
    }
    let mut draws = Vec::new();
    for (i, draw) in content.draws.iter().enumerate() {
        if !copy[i] {
            continue;
        }
        let name = name_of(draw).unwrap_or_default();
        let rename = name.is_empty()
            || left.contains(&(container(draw), name.clone()))
            || left.contains(&(draw.parent, name.clone()));
        let (object, form) = match &draw.kind {
            DrawKind::Image {
                object: Some(id), ..
            } => (*id, false),
            DrawKind::Form { object, .. } => (*object, true),
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
            proven: proven[i],
            name,
            rename,
        });
    }
    Ok(Some(PagePlan {
        page_id,
        joined,
        base,
        page_resources: own_resources(doc, inherited),
        draws,
        streams: content.streams,
    }))
}

/// A copy being made: its original, the name edits to its content
/// (decoded offsets) and its own resources.
struct Copy {
    origin: ObjectId,
    form: bool,
    splices: Vec<(usize, usize, Vec<u8>)>,
    resources: Option<Dictionary>,
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

/// Gives each placement a walk proves visible and untouched of an object
/// in `targets` (what apply changes or empties) its own untouched copy, on
/// every page but `skip` (pages being flattened). `marks` decide what is
/// touched. A page that cannot be walked gets no copies (`unread`).
#[allow(clippy::too_many_lines)]
pub fn protect(
    target: &mut Builder,
    view: Option<&ViewDocument>,
    marks: &[Mark],
    targets: &BTreeSet<ObjectId>,
    skip: &[usize],
) -> Result<Protected, ApplyError> {
    let mut out = Protected::default();
    if targets.is_empty() {
        return Ok(out);
    }
    let count = info::pages(target.document()).len();
    let marked: BTreeSet<usize> = marks.iter().map(|m| m.page).collect();
    let mut counter = 0usize;
    let mut copied = 0usize;
    for page in (0..count).filter(|p| !skip.contains(p)) {
        let plan = {
            let doc = target.document();
            let rule = marked.contains(&page).then(|| GlyphRule::new(doc, marks));
            plan_page(doc, view, rule.as_ref(), targets, page)
        };
        let plan = match plan {
            Ok(Some(plan)) => plan,
            Ok(None) => continue,
            Err(()) => {
                out.unread.insert(page);
                continue;
            }
        };
        let PagePlan {
            page_id,
            joined,
            base,
            mut page_resources,
            draws,
            streams,
        } = plan;
        let mut page_names = names_of(&page_resources);
        let mut page_splices: Vec<(usize, usize, Vec<u8>)> = Vec::new();
        let mut copy_of: BTreeMap<usize, usize> = BTreeMap::new();
        let mut copies: Vec<(ObjectId, Copy)> = Vec::new();
        for planned in draws {
            copied = copied.saturating_add(planned.bytes);
            if copies.len() >= MAX_COPIES || copied > MAX_COPY_BYTES {
                return Err(ApplyError::TooLarge(
                    "the pictures and drawings shown elsewhere are too large to keep".into(),
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
                    if planned.rename {
                        let name = fresh(&mut page_names, &mut counter);
                        page_splices.push((start, end, operand(&name)));
                        add_xobject(&mut page_resources, name, id);
                    } else {
                        add_xobject(&mut page_resources, planned.name.clone(), id);
                    }
                }
                Some(p) => {
                    let Some((_, parent)) = copy_of.get(&p).and_then(|&c| copies.get_mut(c)) else {
                        continue;
                    };
                    let Some(resources) = parent.resources.as_mut() else {
                        continue;
                    };
                    if planned.rename {
                        let mut taken = names_of(resources);
                        let name = fresh(&mut taken, &mut counter);
                        parent
                            .splices
                            .push((planned.start, planned.op_start, operand(&name)));
                        add_xobject(resources, name, id);
                    } else {
                        add_xobject(resources, planned.name.clone(), id);
                    }
                }
            }
            if planned.proven {
                out.proven.insert((planned.object, page));
            }
            copy_of.insert(planned.index, copies.len());
            copies.push((
                id,
                Copy {
                    origin: planned.object,
                    form: planned.form,
                    splices: Vec::new(),
                    resources: planned.resources,
                },
            ));
        }
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
            out.copies.insert(id, copy.origin);
        }
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
