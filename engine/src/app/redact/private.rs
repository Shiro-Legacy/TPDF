// SPDX-License-Identifier: MIT
//! R3 of the redaction redesign, as the fourth review decided: what apply
//! changes is changed IN PLACE and every use of it, anywhere, sees the
//! redacted object; there are no private copies. A picture or Form a box
//! touches that something else also shows is therefore redacted there too
//! (over-redaction), and the reply says so (`overRedacted`), decided by
//! reachability (`used_elsewhere`), never by proving where a copy would
//! be safe.
//!
//! - `touched`: which draws of a walk a box touches, or hold something a box
//!   touches (apply changes their objects);
//! - `changed_objects`: those objects on the marked pages, with the soft
//!   masks and masks of the images among them (blanked in place too);
//! - `used_elsewhere`: whether anything other than the boxed placements
//!   themselves reaches one of the changed or emptied objects, or a box
//!   touches one at more than one placement (each then loses what the
//!   other boxes cover): another page,
//!   an unused resource name, a draw a box does not touch, a pattern, soft
//!   mask, Type 3 font or annotation appearance, the catalog (outside the
//!   page tree).
use std::collections::{BTreeMap, BTreeSet};

use crate::app::info;
use crate::app::pagetree;
use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId};
use crate::view::ViewDocument;

use super::apply::ApplyError;
use super::content::{Content, Detail, Draw, DrawKind, Limits, WalkError};
use super::marks::Mark;
use super::region::{self, GlyphRule};

/// Objects `used_elsewhere` visits at most; past it, everything counts as
/// used elsewhere.
const MAX_REACH: usize = 1 << 21;

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

/// The soft masks and masks (streams) of the images among `objects`: apply
/// blanks them in place with their images.
pub fn masks_of(doc: &Document<'_>, objects: &BTreeSet<ObjectId>) -> BTreeSet<ObjectId> {
    let mut out = BTreeSet::new();
    for &id in objects {
        let Ok(dict) = doc.stream_dict(id) else {
            continue;
        };
        if doc.dict_get(dict, b"Subtype").as_name() != Some(b"Image") {
            continue;
        }
        for key in [b"SMask".as_slice(), b"Mask"] {
            if let Some(mask) = dict.get(key).and_then(Object::as_reference) {
                if doc.stream_dict(mask).is_ok() {
                    out.insert(mask);
                }
            }
        }
    }
    out
}

/// A page's annotations apply keeps: not marks, and not touched by a box
/// (those go, with their appearances emptied in place).
fn kept_annotations(
    doc: &Document<'_>,
    regions: &region::Regions,
    index: usize,
    page: &Dictionary,
) -> Vec<Object> {
    let space = crate::app::annots::PageSpace::of(doc, page);
    let items = doc.dict_get(page, b"Annots").as_array().unwrap_or(&[]);
    (items.iter())
        .filter(|item| {
            let Some(annot) = doc.resolve(item).as_dict() else {
                return true;
            };
            if doc.dict_get(annot, b"Subtype").as_name() == Some(b"Redact") {
                return false;
            }
            let rect = annot
                .get(b"Rect".as_slice())
                .and_then(|r| crate::app::annots::rect(doc, r))
                .map(|r| space.rect_to_page(r));
            !rect.is_some_and(|r| regions.touches(index, &r))
        })
        .cloned()
        .collect()
}

/// Whether a page not in `marked` reaches one of `ids` (page dictionaries
/// not followed): property lists apply cleaned of alternate texts.
pub fn reached_from_unmarked(
    doc: &Document<'_>,
    marked: &BTreeSet<usize>,
    ids: &BTreeSet<ObjectId>,
) -> bool {
    if ids.is_empty() {
        return false;
    }
    let mut stack: Vec<Object> = Vec::new();
    for (index, page) in info::pages(doc).iter().enumerate() {
        if marked.contains(&index) {
            continue;
        }
        stack.extend(
            (page.dict.iter())
                .filter(|(k, _)| k.as_slice() != b"Parent")
                .map(|(_, v)| v.clone()),
        );
        if let Some(resources) = pagetree::inherited(doc, page.dict, b"Resources") {
            stack.push(resources.clone());
        }
    }
    let mut seen = BTreeSet::new();
    let mut queue = Vec::new();
    loop {
        while let Some(value) = stack.pop() {
            match value {
                Object::Reference(id) => queue.push(id),
                Object::Array(items) => stack.extend(items),
                Object::Dictionary(dict) => stack.extend(dict.into_values()),
                _ => {}
            }
        }
        let Some(id) = queue.pop() else {
            return false;
        };
        if !seen.insert(id) {
            continue;
        }
        if ids.contains(&id) || seen.len() > MAX_REACH {
            return true;
        }
        let value = doc.resolve_id(id);
        if value
            .as_dict()
            .is_some_and(|d| matches!(doc.dict_get(d, b"Type").as_name(), Some(b"Page" | b"Pages")))
        {
            continue;
        }
        stack.push(value.clone());
        if let Ok(dict) = doc.stream_dict(id) {
            stack.extend(dict.values().cloned());
        }
    }
}

/// The appearance streams (and widget icons) of the annotations a box
/// touches on the marked pages: apply removes the annotations, and empties
/// these in place (another annotation may share one).
pub fn removed_appearances(doc: &Document<'_>, marks: &[Mark]) -> BTreeSet<ObjectId> {
    let regions = region::Regions::of(marks);
    let mut out = BTreeSet::new();
    for (index, page) in info::pages(doc).iter().enumerate() {
        if regions.on(index).is_empty() {
            continue;
        }
        for (annot, dict) in region::annotation_entries(doc, index, page.dict) {
            if !regions.touches(index, &annot.rect) {
                continue;
            }
            out.extend(region::appearance_streams(doc, dict));
            if let Some(mk) = doc.dict_get(dict, b"MK").as_dict() {
                for key in [b"I".as_slice(), b"RI", b"IX"] {
                    if let Some(id) = mk.get(key).and_then(Object::as_reference) {
                        out.insert(id);
                    }
                }
            }
        }
    }
    out
}

/// The text of content a draw's operation spans (its operands), read in
/// the page's joined /Contents or its Form's stream.
fn operands<'c>(content: &'c Content, joined: &'c [u8], base: &[usize], draw: &Draw) -> &'c [u8] {
    match draw.parent {
        None => {
            let start = match draw.from {
                Some((e, o)) => base.get(e).map(|b| b + o),
                None => base.get(draw.element).map(|b| b + draw.start),
            };
            let end = base.get(draw.element).map(|b| b + draw.op.start);
            (start.zip(end))
                .and_then(|(s, e)| joined.get(s..e))
                .unwrap_or(&[])
        }
        Some(_) => (content.streams.get(&draw.stream))
            .and_then(|b| b.get(draw.start..draw.op.start))
            .unwrap_or(&[]),
    }
}

/// Whether anything other than the boxed placements themselves reaches an
/// object of `changed` (what apply changes or empties): the catalog outside
/// the page tree; every page not marked (all of its dictionary); on a
/// marked page, its annotations, thumbnail and other entries, its resources
/// other than the image and Form names only touched draws use, and every
/// draw a box does not touch (at any depth); in a touched Form, its own
/// resources other than what its touched draws use. Page dictionaries are
/// not followed (a destination names a page; the page tree is gone through
/// page by page). `flat` are pages being flattened: only their entries
/// other than /Contents and /Resources count. A page the walk cannot read
/// is an error (apply refuses it too).
#[allow(clippy::too_many_lines)]
pub fn used_elsewhere(
    doc: &Document<'static>,
    view: Option<&ViewDocument>,
    marks: &[Mark],
    flat: &[usize],
    changed: &BTreeSet<ObjectId>,
) -> Result<bool, ApplyError> {
    if changed.is_empty() {
        return Ok(false);
    }
    let marked: BTreeSet<usize> = marks.iter().map(|m| m.page).collect();
    let rule = GlyphRule::new(doc, marks);
    let regions = region::Regions::of(marks);
    // The annotations apply removes (from every page listing them): not
    // followed, so the form's field list reaching a removed widget's icon
    // is no use elsewhere (sixth review).
    let boxed: BTreeSet<(usize, ObjectId)> = (region::annotations(doc, &regions).into_iter())
        .filter_map(|a| a.id.map(|id| (a.page, id)))
        .collect();
    let removed: BTreeSet<ObjectId> = boxed.iter().map(|(_, id)| *id).collect();
    // One a page lists where no box touches it is lost there.
    for (index, page) in info::pages(doc).iter().enumerate() {
        let items = doc.dict_get(page.dict, b"Annots").as_array().unwrap_or(&[]);
        if (items.iter().filter_map(Object::as_reference))
            .any(|id| removed.contains(&id) && !boxed.contains(&(index, id)))
        {
            return Ok(true);
        }
    }
    // Images a box touches, by the soft mask or mask they share.
    let mut mask_users: BTreeMap<ObjectId, BTreeSet<ObjectId>> = BTreeMap::new();
    let mut roots: Vec<Object> = Vec::new();
    let mut reached: Vec<ObjectId> = Vec::new();
    if let Some(catalog) = doc.resolve_id(doc.root()).as_dict() {
        for (key, value) in catalog.iter() {
            if key.as_slice() != b"Pages" {
                roots.push(value.clone());
            }
        }
    }
    let pages = info::pages(doc);
    // An object a box touches at more than one placement takes the edits
    // of all of them at each: each loses what the others' boxes cover.
    let mut placements: BTreeMap<ObjectId, usize> = BTreeMap::new();
    let mut seen_pages: BTreeMap<ObjectId, usize> = BTreeMap::new();
    for page in pages.iter() {
        if let Some(id) = page.id {
            *seen_pages.entry(id).or_default() += 1;
        }
    }
    for (index, page) in pages.iter().enumerate() {
        // A page object the tree lists twice is shown twice.
        let listed_again = page
            .id
            .is_some_and(|id| seen_pages.get(&id).copied().unwrap_or(0) > 1);
        let entries = page.dict.iter().filter(|(k, _)| k.as_slice() != b"Parent");
        if flat.contains(&index) && !listed_again {
            roots.extend(
                entries
                    .filter(|(k, _)| {
                        !matches!(k.as_slice(), b"Contents" | b"Resources" | b"Annots")
                    })
                    .map(|(_, v)| v.clone()),
            );
            roots.extend(kept_annotations(doc, &regions, index, page.dict));
            continue;
        }
        if !marked.contains(&index) || listed_again {
            roots.extend(entries.map(|(_, v)| v.clone()));
            if let Some(resources) = pagetree::inherited(doc, page.dict, b"Resources") {
                roots.push(resources.clone());
            }
            continue;
        }
        roots.extend(
            entries
                .filter(|(k, _)| !matches!(k.as_slice(), b"Contents" | b"Resources" | b"Annots"))
                .map(|(_, v)| v.clone()),
        );
        roots.extend(kept_annotations(doc, &regions, index, page.dict));
        let content = region::page_content(doc, view, index, Detail::Full, Limits::DEFAULT)
            .map_err(|e| walk_error(index, e))?;
        let needs = touched(&content, Some(&rule), index);
        if rule.spent() {
            return Err(ApplyError::TooComplex(index));
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
        // Names only touched draws use, by the content that names them
        // (None: the page's /Contents), and names untouched draws use.
        let mut only_touched: BTreeMap<Option<usize>, BTreeSet<Vec<u8>>> = BTreeMap::new();
        let mut untouched_names: BTreeSet<(Option<usize>, Vec<u8>)> = BTreeSet::new();
        for (i, draw) in content.draws.iter().enumerate() {
            let Some((object, _)) = draw.kind.xobject() else {
                continue;
            };
            let name = operand_name(operands(&content, &joined, &base, draw));
            if needs.get(i).copied().unwrap_or(false) {
                if let Ok(dict) = doc.stream_dict(object) {
                    for key in [b"SMask".as_slice(), b"Mask"] {
                        if let Some(mask) = dict.get(key).and_then(Object::as_reference) {
                            let users = mask_users.entry(mask).or_default();
                            users.insert(object);
                            // Two images a box touches share the mask: each
                            // loses what the other's box covers.
                            if users.len() > 1 {
                                return Ok(true);
                            }
                        }
                    }
                }
                let count = placements.entry(object).or_default();
                *count += 1;
                if *count > 1 && changed.contains(&object) {
                    return Ok(true);
                }
                only_touched.entry(draw.parent).or_default().insert(name);
            } else {
                untouched_names.insert((draw.parent, name));
                reached.push(object);
            }
        }
        // The page's resources: every category but images and Forms, and
        // the image and Form names not used by touched draws alone.
        let resources = pagetree::inherited(doc, page.dict, b"Resources")
            .and_then(|r| doc.resolve(r).as_dict());
        let keep_resources =
            |roots: &mut Vec<Object>, resources: &Dictionary, container: Option<usize>| {
                for (key, value) in resources.iter() {
                    if key.as_slice() != b"XObject" {
                        roots.push(value.clone());
                        continue;
                    }
                    let Some(xobjects) = doc.resolve(value).as_dict() else {
                        roots.push(value.clone());
                        continue;
                    };
                    for (name, entry) in xobjects.iter() {
                        let touched_only = only_touched
                            .get(&container)
                            .is_some_and(|names| names.contains(name))
                            && !untouched_names.contains(&(container, name.clone()));
                        if !touched_only {
                            roots.push(entry.clone());
                        }
                    }
                }
            };
        if let Some(resources) = resources {
            keep_resources(&mut roots, resources, None);
        }
        // Each touched Form's own resources, likewise (a Form without its
        // own reads the page's, done above).
        for (i, draw) in content.draws.iter().enumerate() {
            if !needs.get(i).copied().unwrap_or(false) {
                continue;
            }
            let DrawKind::Form { object, .. } = draw.kind else {
                continue;
            };
            let own = (doc.stream_dict(object).ok())
                .and_then(|d| d.get(b"Resources".as_slice()))
                .and_then(|r| doc.resolve(r).as_dict());
            if let Some(own) = own {
                keep_resources(&mut roots, own, Some(i));
            }
            // A Form's other entries (its /Group, /OC, ...).
            if let Ok(dict) = doc.stream_dict(object) {
                roots.extend(
                    (dict.iter())
                        .filter(|(k, _)| !matches!(k.as_slice(), b"Resources" | b"Length"))
                        .map(|(_, v)| v.clone()),
                );
            }
        }
    }
    // The closure, page dictionaries not followed.
    let mut seen: BTreeSet<ObjectId> = BTreeSet::new();
    let mut stack: Vec<Object> = roots;
    let mut queue: Vec<ObjectId> = reached;
    loop {
        while let Some(value) = stack.pop() {
            match value {
                Object::Reference(id) => queue.push(id),
                Object::Array(items) => stack.extend(items),
                Object::Dictionary(dict) => stack.extend(dict.into_values()),
                _ => {}
            }
        }
        let Some(id) = queue.pop() else {
            break;
        };
        if !seen.insert(id) || removed.contains(&id) {
            continue;
        }
        if changed.contains(&id) || seen.len() > MAX_REACH {
            return Ok(true);
        }
        let value = doc.resolve_id(id);
        let page_node = value.as_dict().is_some_and(|d| {
            matches!(doc.dict_get(d, b"Type").as_name(), Some(b"Page" | b"Pages"))
        });
        if page_node {
            continue;
        }
        stack.push(value.clone());
        if let Ok(dict) = doc.stream_dict(id) {
            stack.extend(dict.values().cloned());
        }
    }
    Ok(false)
}
