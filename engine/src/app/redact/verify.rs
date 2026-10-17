// SPDX-License-Identifier: AGPL-3.0-or-later
//! redact-apply's checks before the copy is handed back (redact.ts
//! `verifyText`, `verifyPixels`, its raw check and its marks check), each
//! at least as strict as removal:
//!
//! - text: the rewritten pages walked again hold no glyph `GlyphRule`
//!   removes and no unmeasured run reaching a mark; the copy's view reads
//!   no character `GlyphRule::under` finds (clipped and hidden ones too);
//! - pixels: the copy drawn at 72 dpi is one colour inside each of a
//!   mark's rects, two pixels in (the black box, nothing over it);
//! - raw: no search text of a mark is left in the strings, names or
//!   comments of the content the marked pages reach (their /Contents and
//!   every Form their resources reach), as UTF-8, UTF-16BE (with or
//!   without its mark) or Latin-1, a `TJ` array's strings read together;
//!   and redact-search's own search of the copy's marked pages finds none
//!   (text in any encoding, shows split apart);
//! - no mark is left, and no marked page keeps a derived copy of itself
//!   (/Thumb, /PieceInfo, /Metadata).
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::sync::Arc;

use crate::app::annots;
use crate::app::info;
use crate::app::pagetree;
use crate::document::Document;
use crate::object::ObjectId;
use crate::stream::StreamLimits;
use crate::view::{MAX_RENDER_PIXELS, RenderOptions, StructuredText, ViewDocument};

use super::apply::ApplyError;
use super::content::{self, Detail, DrawKind, Limits, WalkError};
use super::marks::Mark;
use super::region::{self, GlyphRule};

/// XObject entries the raw check visits through resources, in all.
const MAX_RAW_ENTRIES: usize = 1_000_000;
/// Decoded bytes the raw check reads in all.
const MAX_RAW_BYTES: usize = 512 << 20;
/// Rows of distinct mark rects the pixel check reads in all (each a step
/// once its row is read); past it the marks of the page left fail.
const MAX_PIXEL_ROWS: usize = 1 << 27;

fn walk_error(page: usize, error: WalkError) -> ApplyError {
    match error {
        WalkError::TooComplex(_) => ApplyError::TooComplex(page),
        WalkError::Unreadable(why) => ApplyError::Unreadable(page, why),
    }
}

/// The marks (indices) on `page` whose rects meet `bounds`; every mark of
/// the page when none does (a box without position).
fn marks_at(marks: &[Mark], page: usize, bounds: &[f64; 4]) -> Vec<usize> {
    let meets = |r: &[f64; 4]| {
        bounds[0] <= r[2] && bounds[2] >= r[0] && bounds[1] <= r[3] && bounds[3] >= r[1]
    };
    let found: Vec<usize> = (marks.iter().enumerate())
        .filter(|(_, m)| m.page == page && m.rects().iter().any(meets))
        .map(|(i, _)| i)
        .collect();
    if found.is_empty() {
        (marks.iter().enumerate())
            .filter(|(_, m)| m.page == page)
            .map(|(i, _)| i)
            .collect()
    } else {
        found
    }
}

/// The distinct search texts of `marks`, in first-mark order.
pub fn searches(marks: &[Mark]) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    for mark in marks {
        if let Some(search) = mark.search.as_deref() {
            if !out.contains(&search) {
                out.push(search);
            }
        }
    }
    out
}

/// The first of `searches` redact-search still finds in `text`.
fn found(text: &StructuredText, searches: &[&str]) -> Option<String> {
    (searches.iter())
        .find(|search| !text.search_hits(search, 1).hits.is_empty())
        .map(|search| (*search).to_string())
}

/// The first search text redact-search still finds on a marked page of
/// `output` (a view with every layer shown); a page whose text cannot be
/// read finds every one.
pub fn search_left(output: &ViewDocument, marks: &[Mark]) -> Option<String> {
    let searches = searches(marks);
    let pages: BTreeSet<usize> = marks.iter().map(|m| m.page).collect();
    let reader = output.text_reader();
    for page in pages {
        if searches.is_empty() {
            break;
        }
        let left = match reader.structured_text(page) {
            Ok(text) => found(&text, &searches),
            Err(_) => searches.first().map(|s| (*s).to_string()),
        };
        if left.is_some() {
            return left;
        }
    }
    None
}

/// Whether a marked page keeps a thumbnail, application data or XMP.
pub fn derived_left(doc: &Document<'_>, pages: &[usize]) -> bool {
    let refs = info::pages(doc);
    pages.iter().any(|&page| {
        refs.get(page).is_some_and(|p| {
            [b"Thumb".as_slice(), b"PieceInfo", b"Metadata"]
                .iter()
                .any(|key| p.dict.get(*key).is_some())
        })
    })
}

/// The marks a character the copy's view reads is still under, and, when
/// `search` (the view shows every layer), the first search text
/// redact-search still finds on a marked page (`search_left`).
pub fn view_text(
    output: &ViewDocument,
    marks: &[Mark],
    rule: &GlyphRule,
    search: bool,
) -> (BTreeSet<usize>, Option<String>) {
    let pages: BTreeSet<usize> = marks.iter().map(|m| m.page).collect();
    let searches = match search {
        true => searches(marks),
        false => Vec::new(),
    };
    let reader = output.text_reader();
    let mut failed = BTreeSet::new();
    let mut left = None;
    for page in pages {
        let Ok(text) = reader.structured_text(page) else {
            failed.extend(marks_at(marks, page, &[f64::NAN; 4]));
            left = left.or_else(|| searches.first().map(|s| (*s).to_string()));
            continue;
        };
        if left.is_none() {
            left = found(&text, &searches);
        }
        for block in text.unclipped_blocks() {
            for line in &block.lines {
                for c in line.chars.iter().filter(|c| !c.synthetic) {
                    let q = c.quad;
                    let xs = [q[0], q[2], q[4], q[6]];
                    let ys = [q[1], q[3], q[5], q[7]];
                    let bounds = [
                        xs.iter().copied().fold(f64::INFINITY, f64::min),
                        ys.iter().copied().fold(f64::INFINITY, f64::min),
                        xs.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                        ys.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                    ];
                    if rule.under(page, &bounds) {
                        failed.extend(marks_at(marks, page, &bounds));
                    }
                }
            }
        }
    }
    (failed, left)
}

/// The marks whose rects, drawn at 72 dpi (less for a page past the
/// view's pixel bound), are not one colour two pixels in from their edges.
pub fn pixels(output: &ViewDocument, marks: &[Mark]) -> BTreeSet<usize> {
    let mut pages: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (index, mark) in marks.iter().enumerate() {
        pages.entry(mark.page).or_default().push(index);
    }
    let mut failed = BTreeSet::new();
    let mut budget = MAX_PIXEL_ROWS;
    for (page, on_page) in pages {
        let scale = match output.page_size(page) {
            Ok((w, h)) if w > 0.0 && h > 0.0 => {
                let area = (MAX_RENDER_PIXELS as f64 / (w * h)).sqrt() * 0.99;
                let edge = f64::from(u16::MAX - 1) / w.max(h);
                area.min(edge).min(1.0)
            }
            _ => 1.0,
        };
        let Ok(image) = output.render(page, scale, RenderOptions::default()) else {
            failed.extend(on_page);
            continue;
        };
        let (w, h) = (image.width as usize, image.height as usize);
        // Each distinct pixel rect once, with the marks it belongs to
        // (identical quads, which a file can bring by the hundred
        // thousand, are checked once).
        let mut rects: BTreeMap<[usize; 4], Vec<usize>> = BTreeMap::new();
        for &index in &on_page {
            let Some(mark) = marks.get(index) else {
                continue;
            };
            for r in mark.rects() {
                let r = r.map(|v| v * scale);
                let clamp = |v: f64, max: usize| (v.max(0.0) as usize).min(max);
                let x0 = clamp(r[0].floor() + 2.0, w);
                let x1 = clamp(r[2].ceil() - 2.0, w);
                let y0 = clamp(r[1].floor() + 2.0, h);
                let y1 = clamp(r[3].ceil() - 2.0, h);
                if x0 >= x1 || y0 >= y1 {
                    continue;
                }
                let owners = rects.entry([y0, y1, x0, x1]).or_default();
                if owners.last() != Some(&index) {
                    owners.push(index);
                }
            }
        }
        let rows = (rects.keys())
            .map(|r| r[1] - r[0])
            .fold(0usize, usize::saturating_add);
        if rows > budget {
            // Too many to check: refused, not passed.
            failed.extend(on_page);
            budget = 0;
            continue;
        }
        budget -= rows;
        let bad = uniform_rows(&image.pixels, w, rects.keys());
        for (at, owners) in rects.values().enumerate() {
            if bad.get(at).copied().unwrap_or(true) {
                failed.extend(owners.iter().copied());
            }
        }
    }
    failed
}

/// For each rect (`[y0, y1, x0, x1]` in pixels, by `y0`), whether it is
/// not one colour (RGB) in `pixels` (RGBA rows of `w`). Row by row: a
/// rect's part of a row is one colour when the row's run from `x0`
/// reaches `x1`, and the rect is when each row's colour is its first
/// row's; so each rect costs a step per row, not per pixel, and each row
/// is read once (acceptance round 2: 262,144 page-size quads took 290 s).
fn uniform_rows<'r>(
    pixels: &[u8],
    w: usize,
    rects: impl Iterator<Item = &'r [usize; 4]>,
) -> Vec<bool> {
    let rects: Vec<[usize; 4]> = rects.copied().collect();
    let mut bad = vec![false; rects.len()];
    let mut first: Vec<Option<[u8; 3]>> = vec![None; rects.len()];
    let rgb = |x: usize, y: usize| -> Option<[u8; 3]> {
        let at = y.checked_mul(w)?.checked_add(x)?.checked_mul(4)?;
        let p = pixels.get(at..at.checked_add(3)?)?;
        Some([*p.first()?, *p.get(1)?, *p.get(2)?])
    };
    // run[x]: one past the last pixel from x on the row of x's colour.
    let mut run = vec![0usize; w];
    let mut active: Vec<usize> = Vec::new();
    let mut next = 0;
    let mut y = rects.first().map_or(0, |r| r[0]);
    while next < rects.len() || !active.is_empty() {
        if active.is_empty() {
            if let Some(r) = rects.get(next) {
                y = y.max(r[0]);
            }
        }
        while let Some(r) = rects.get(next) {
            if r[0] > y {
                break;
            }
            active.push(next);
            next += 1;
        }
        let mut row_ok = true;
        let mut end = w;
        let mut right = None;
        for x in (0..w).rev() {
            let here = rgb(x, y);
            if here.is_none() {
                row_ok = false;
                break;
            }
            if right.is_some() && here != right {
                end = x + 1;
            }
            if let Some(slot) = run.get_mut(x) {
                *slot = end;
            }
            right = here;
        }
        for &i in &active {
            let (Some(r), Some(seen)) = (rects.get(i), first.get_mut(i)) else {
                continue;
            };
            let colour = rgb(r[2], y);
            let one = row_ok && run.get(r[2]).is_some_and(|&e| e >= r[3]);
            let same = match seen {
                None => {
                    *seen = colour;
                    true
                }
                Some(c) => colour == Some(*c),
            };
            if !one || !same || colour.is_none() {
                if let Some(b) = bad.get_mut(i) {
                    *b = true;
                }
            }
        }
        y += 1;
        active.retain(|&i| {
            rects.get(i).is_some_and(|r| r[1] > y) && !bad.get(i).copied().unwrap_or(true)
        });
    }
    bad
}

/// The byte forms a search text could be stored in: UTF-8, UTF-16BE with
/// and without its byte order mark, and Latin-1 when it has one.
fn needles(text: &str) -> Vec<Vec<u8>> {
    let utf8 = text.as_bytes().to_vec();
    let wide: Vec<u8> = text.encode_utf16().flat_map(u16::to_be_bytes).collect();
    let mut marked = vec![0xfe, 0xff];
    marked.extend_from_slice(&wide);
    let mut out = vec![utf8, wide, marked];
    let latin: Option<Vec<u8>> = text
        .chars()
        .map(|c| u8::try_from(u32::from(c)).ok())
        .collect();
    if let Some(latin) = latin {
        if !out.contains(&latin) {
            out.push(latin);
        }
    }
    out.retain(|n| !n.is_empty());
    out
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    let Some(&first) = needle.first() else {
        return false;
    };
    if hay.len() < needle.len() {
        return false;
    }
    (0..=hay.len() - needle.len())
        .any(|i| hay.get(i) == Some(&first) && hay.get(i..i + needle.len()) == Some(needle))
}

/// A literal string's bytes from `bytes[*i]` (its `(`); `*i` ends past it.
fn literal(bytes: &[u8], i: &mut usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut nesting = 0usize;
    while let Some(&c) = bytes.get(*i) {
        *i += 1;
        match c {
            b'\\' => {
                let Some(&e) = bytes.get(*i) else { break };
                *i += 1;
                match e {
                    b'n' => out.push(10),
                    b'r' => out.push(13),
                    b't' => out.push(9),
                    b'b' => out.push(8),
                    b'f' => out.push(12),
                    b'0'..=b'7' => {
                        let mut v = u32::from(e - b'0');
                        for _ in 0..2 {
                            match bytes.get(*i) {
                                Some(d @ b'0'..=b'7') => {
                                    v = v * 8 + u32::from(d - b'0');
                                    *i += 1;
                                }
                                _ => break,
                            }
                        }
                        out.push((v & 0xff) as u8);
                    }
                    b'\n' => {}
                    b'\r' => {
                        if bytes.get(*i) == Some(&b'\n') {
                            *i += 1;
                        }
                    }
                    other => out.push(other),
                }
            }
            b'(' => {
                nesting += 1;
                if nesting > 1 {
                    out.push(c);
                }
            }
            b')' => {
                nesting = nesting.saturating_sub(1);
                if nesting == 0 {
                    break;
                }
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

/// A hex string's bytes from `bytes[*i]` (its `<`); `*i` ends past it.
fn hex(bytes: &[u8], i: &mut usize) -> Vec<u8> {
    *i += 1;
    let mut out = Vec::new();
    let mut high: Option<u8> = None;
    while let Some(&c) = bytes.get(*i) {
        *i += 1;
        let d = match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            b'>' => break,
            _ => continue,
        };
        match high.take() {
            None => high = Some(d),
            Some(h) => out.push((h << 4) | d),
        }
    }
    if let Some(h) = high {
        out.push(h << 4);
    }
    out
}

/// Calls `visit` with the text of content `bytes` a reader could take for
/// the page's: the strings each show operator shows (one by one, and read
/// together, as a `TJ` array's), the alternate texts of property lists
/// (/ActualText, /Alt, /E) and comments; `skip` spans (inline image data)
/// are not read, and without them an inline image's data is skipped to the
/// `EI` after it. Stops at the first `visit` that answers true.
fn texts(bytes: &[u8], skip: &[Range<usize>], mut visit: impl FnMut(&[u8]) -> bool) -> bool {
    const ALT: [&[u8]; 3] = [b"ActualText", b"Alt", b"E"];
    let white = |b: u8| matches!(b, 0 | 9 | 10 | 12 | 13 | 32);
    let regular = |b: u8| {
        !white(b)
            && !matches!(
                b,
                b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
            )
    };
    let mut skip: Vec<Range<usize>> = skip.to_vec();
    skip.sort_by_key(|r| r.start);
    let mut k = 0;
    let mut i = 0;
    // Strings since the last operator, and alternate texts.
    let mut shown: Vec<Vec<u8>> = Vec::new();
    let mut alt: Vec<Vec<u8>> = Vec::new();
    let mut key: Option<Vec<u8>> = None;
    let take = |text: Vec<u8>,
                key: &Option<Vec<u8>>,
                shown: &mut Vec<Vec<u8>>,
                alt: &mut Vec<Vec<u8>>| match key.as_deref() {
        Some(k) if ALT.contains(&k) => alt.push(text),
        _ => shown.push(text),
    };
    while i < bytes.len() {
        while skip.get(k).is_some_and(|r| r.end <= i) {
            k += 1;
        }
        if let Some(r) = skip.get(k).filter(|r| r.start <= i) {
            i = r.end;
            continue;
        }
        let Some(&b) = bytes.get(i) else {
            break;
        };
        match b {
            b'%' => {
                let start = i;
                while bytes.get(i).is_some_and(|b| !matches!(b, b'\n' | b'\r')) {
                    i += 1;
                }
                if visit(bytes.get(start..i).unwrap_or(&[])) {
                    return true;
                }
            }
            b'(' => {
                let text = literal(bytes, &mut i);
                take(text, &key, &mut shown, &mut alt);
                key = None;
            }
            b'<' if bytes.get(i + 1) == Some(&b'<') => i += 2,
            b'>' if bytes.get(i + 1) == Some(&b'>') => i += 2,
            b'<' => {
                let text = hex(bytes, &mut i);
                take(text, &key, &mut shown, &mut alt);
                key = None;
            }
            b'/' => {
                let start = i + 1;
                i += 1;
                while bytes.get(i).is_some_and(|c| regular(*c)) {
                    i += 1;
                }
                key = bytes.get(start..i).map(<[u8]>::to_vec);
            }
            c if regular(c) => {
                let start = i;
                while bytes.get(i).is_some_and(|c| regular(*c)) {
                    i += 1;
                }
                let word = bytes.get(start..i).unwrap_or(&[]);
                if !word
                    .first()
                    .is_some_and(|c| c.is_ascii_digit() || matches!(c, b'-' | b'+' | b'.'))
                {
                    // An operator: what it shows, and alternate texts.
                    if matches!(word, b"Tj" | b"TJ" | b"'" | b"\"") {
                        let together: Vec<u8> = shown.concat();
                        if shown.iter().any(|s| visit(s)) || visit(&together) {
                            return true;
                        }
                    }
                    if alt.iter().any(|s| visit(s)) {
                        return true;
                    }
                    shown.clear();
                    alt.clear();
                    key = None;
                }
                // Inline image data the walk did not delimit: to the next
                // `EI` between white space.
                if word == b"ID" {
                    let mut j = i + 1;
                    while j + 2 <= bytes.len() {
                        let around = |p: Option<usize>| {
                            p.and_then(|p| bytes.get(p)).is_none_or(|b| white(*b))
                        };
                        if bytes.get(j..j + 2) == Some(b"EI")
                            && around(j.checked_sub(1))
                            && around(Some(j + 2))
                        {
                            break;
                        }
                        j += 1;
                    }
                    i = j + 2;
                }
            }
            _ => i += 1,
        }
    }
    alt.iter().any(|s| visit(s))
}

/// A decoded stream the raw check reads, with the inline image data spans
/// it skips.
type Read = (Arc<Vec<u8>>, Vec<Range<usize>>);

/// What the rewritten pages' content holds under the marks: the marks some
/// glyph is still under, how many distinct search texts were looked for,
/// and the first one still in the content.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContentCheck {
    pub text: BTreeSet<usize>,
    pub searches: usize,
    pub left: Option<String>,
}

/// The text check (a walk of each rewritten marked page) and the raw check
/// (each mark's search text in the content the marked pages reach: what
/// they draw, and the Forms their resources reach).
pub fn content(
    doc: &Document<'static>,
    view: Option<&ViewDocument>,
    marks: &[Mark],
    rule: &GlyphRule,
) -> Result<ContentCheck, ApplyError> {
    let searches = searches(marks);
    let pages: BTreeSet<usize> = marks.iter().map(|m| m.page).collect();
    let page_refs = info::pages(doc);
    let mut out = ContentCheck {
        searches: searches.len(),
        ..ContentCheck::default()
    };
    // Streams read, with the inline image spans the walk found in them.
    let mut streams: Vec<Read> = Vec::new();
    let mut seen: BTreeSet<ObjectId> = BTreeSet::new();
    let mut budget = MAX_RAW_ENTRIES;
    let mut read = 0usize;
    for page in pages {
        let content = region::page_content(doc, view, page, Detail::Full, Limits::DEFAULT)
            .map_err(|e| walk_error(page, e))?;
        for draw in &content.draws {
            let DrawKind::Text(text) = &draw.kind else {
                continue;
            };
            if !text.measured {
                if rule.reaches(page, &draw.bounds) {
                    out.text.extend(marks_at(marks, page, &draw.bounds));
                }
                continue;
            }
            for glyph in &text.glyphs {
                if rule.removes_glyph(page, &glyph.bounds, &glyph.quad) {
                    out.text.extend(marks_at(marks, page, &glyph.bounds));
                }
            }
        }
        if searches.is_empty() {
            continue;
        }
        // Inline image data, by stream.
        let mut inline: BTreeMap<ObjectId, Vec<Range<usize>>> = BTreeMap::new();
        for draw in &content.draws {
            if matches!(draw.kind, DrawKind::Image { object: None, .. }) {
                inline
                    .entry(draw.stream)
                    .or_default()
                    .push(draw.start..draw.op.end);
            }
        }
        for (id, bytes) in &content.streams {
            if !seen.insert(*id) {
                continue;
            }
            let skip = inline.remove(id).unwrap_or_default();
            read = read.saturating_add(bytes.len());
            streams.push((bytes.clone(), skip));
        }
        // Forms the page's resources reach but does not draw.
        let Some(dict) = page_refs.get(page).map(|p| p.dict) else {
            continue;
        };
        let resources =
            pagetree::inherited(doc, dict, b"Resources").and_then(|r| doc.resolve(r).as_dict());
        let reachable =
            content::reachable_xobjects(doc, resources, &mut budget).ok_or_else(|| {
                ApplyError::TooLarge("the marked pages reach too many objects".into())
            })?;
        for id in reachable {
            let form = doc
                .stream_dict(id)
                .is_ok_and(|d| doc.dict_get(d, b"Subtype").as_name() == Some(b"Form"));
            if !form || !seen.insert(id) {
                continue;
            }
            let room = MAX_RAW_BYTES.saturating_sub(read);
            let limits = StreamLimits {
                max_decoded_bytes: room,
                max_total_decoded_bytes: room,
                ..StreamLimits::default()
            };
            let bytes = doc
                .decoded_stream(id, limits)
                .ok()
                .or_else(|| view.and_then(|v| v.decoded_stream(id.number(), id.generation())))
                .unwrap_or_default();
            read = read.saturating_add(bytes.len());
            if read > MAX_RAW_BYTES {
                return Err(ApplyError::TooLarge(
                    "the marked pages reach too much content to check".into(),
                ));
            }
            streams.push((Arc::new(bytes), Vec::new()));
        }
    }
    for search in searches {
        let needles = needles(search);
        let found = streams.iter().any(|(bytes, skip)| {
            texts(bytes, skip, |text| {
                needles.iter().any(|n| contains(text, n))
            })
        });
        if found {
            out.left = Some(search.to_string());
            break;
        }
    }
    Ok(out)
}

/// Whether a mark is left on any of `pages`.
pub fn marks_left(doc: &Document<'_>, pages: &[usize]) -> bool {
    let refs = info::pages(doc);
    pages.iter().any(|&page| {
        refs.get(page).is_some_and(|p| {
            annots::list(doc, p.dict)
                .iter()
                .any(|a| a.subtype == b"Redact")
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(bytes: &[u8], needle: &str) -> bool {
        let needles = needles(needle);
        texts(bytes, &[], |t| needles.iter().any(|n| contains(t, n)))
    }

    #[test]
    fn search_texts_are_found_in_shown_strings_not_operators() {
        assert!(found(b"BT (a SECRET b) Tj ET", "SECRET"));
        assert!(found(b"[(SEC) -20 (RET)] TJ", "SECRET"));
        assert!(found(b"<534543524554> Tj", "SECRET"));
        assert!(found(b"<feff0053004500430052004500540020> Tj", "SECRET"));
        assert!(found(b"% SECRET\n", "SECRET"));
        assert!(found(b"(caf\\351) Tj", "café"));
        assert!(found(
            b"/Span <</Lang (en) /ActualText (SECRET)>> BDC",
            "SECRET"
        ));
        assert!(!found(b"/Span <</Lang (SECRET)>> BDC", "SECRET"));
        assert!(!found(b"100 0 0 100 0 0 cm /Im Do", "100"));
        assert!(!found(b"/SECRET Do", "SECRET"));
        assert!(!found(b"BI /W 1 /H 1 ID SECRET EI Q", "SECRET"));
    }
}
