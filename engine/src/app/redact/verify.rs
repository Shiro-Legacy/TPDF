// SPDX-License-Identifier: MIT
//! redact-apply's checks before the copy is handed back (redact.ts
//! `verifyText`, `verifyPixels`, its raw check and its marks check), each
//! at least as strict as removal:
//!
//! - text: the rewritten pages walked again hold no glyph `GlyphRule`
//!   removes and no unmeasured run reaching a mark; the copy's view reads
//!   no character `GlyphRule::under` finds (clipped and hidden ones too);
//! - pixels: the copy drawn at 72 dpi is one colour inside each of a
//!   mark's rects, two pixels in (the black box, nothing over it); at 36
//!   dpi when the view refuses the drawing's cost, in the pixels wholly
//!   inside each rect;
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
use crate::object::{Object, ObjectId};
use crate::stream::StreamLimits;
use crate::view::{
    MAX_RENDER_EDGE, MAX_RENDER_PIXELS, RenderBudgets, RenderOptions, StructuredText, ViewDocument,
    ViewError,
};

use super::apply::ApplyError;
use super::content::{self, Detail, DrawKind, Limits, WalkError};
use super::marks::Mark;
use super::region::{self, GlyphRule};

/// XObject entries the raw check visits through resources, in all.
const MAX_RAW_ENTRIES: usize = 1_000_000;
/// Decoded bytes the raw check reads in all.
const MAX_RAW_BYTES: usize = 512 << 20;
/// Rows of distinct mark rects the pixel check reads in all (each a step
/// once its row is read); past it the check refuses (`PixelRefusal::TooMany`).
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
                    // Past the query budget the caller refuses.
                    if rule.spent() {
                        return (failed, left);
                    }
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

/// Why the pixel check could not check a marked page: refused, neither
/// passed nor failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelRefusal {
    /// The page (index) has no size the check can draw: not finite, not
    /// positive, or so large that its scale within the view's pixel bound
    /// is below [`MIN_SCALE`].
    Undrawable(usize),
    /// The page (index) cannot be drawn: the view refuses its content, or
    /// refuses its drawing at each of [`SCALES`].
    TooComplex(usize),
    /// The page (index) draws only at a reduced scale, where a mark rect
    /// the check reads at 72 dpi keeps no pixel wholly inside it.
    TooThin(usize),
    /// Past [`MAX_PIXEL_ROWS`] rect rows in all, at the page (index).
    TooMany(usize),
}

/// The scales the pixel check draws a page at, as fractions of its full
/// scale (72 dpi, or less for a page past the view's pixel bound), in
/// order: a fixed list, never a computed loop.
pub const SCALES: [f64; 2] = [1.0, 0.5];
/// The lowest scale the check draws at (36 dpi): a pixel is at most 2 pt,
/// so the band along a rect's edges it does not read stays within about
/// 2 pt, as at 72 dpi.
pub const MIN_SCALE: f64 = 0.5;
/// How far (pixels) a pixel read at a reduced scale must lie inside a
/// rect's edge, for rounding where the black box was drawn.
const INSIDE_PX: f64 = 0.01;

/// A rect's pixels the check reads at `scale` (`[y0, y1, x0, x1]`, clamped
/// to `w` x `h`), or `None` when none is left: at 72 dpi those two pixels
/// in from its edges (rounded out), as before; at a reduced scale those
/// wholly inside it, so the band not read is under a pixel (2 pt at 36
/// dpi) and every pixel read is one the box covers whole.
fn interior(r: &[f64; 4], scale: f64, w: usize, h: usize) -> Option<[usize; 4]> {
    let r = r.map(|v| v * scale);
    let clamp = |v: f64, max: usize| (v.max(0.0) as usize).min(max);
    let [x0, y0, x1, y1] = if scale >= 1.0 {
        [
            r[0].floor() + 2.0,
            r[1].floor() + 2.0,
            r[2].ceil() - 2.0,
            r[3].ceil() - 2.0,
        ]
    } else {
        [
            (r[0] + INSIDE_PX).ceil(),
            (r[1] + INSIDE_PX).ceil(),
            (r[2] - INSIDE_PX).floor(),
            (r[3] - INSIDE_PX).floor(),
        ]
    };
    if ![x0, y0, x1, y1].iter().all(|v| v.is_finite()) {
        return None;
    }
    let (x0, x1, y0, y1) = (clamp(x0, w), clamp(x1, w), clamp(y0, h), clamp(y1, h));
    (x0 < x1 && y0 < y1).then_some([y0, y1, x0, x1])
}

/// The image size the view draws a `w` x `h` point page at `scale`
/// (`render::pixel_size`).
fn image_size(w: f64, h: f64, scale: f64) -> (usize, usize) {
    let edge = |v: f64| (v * scale - 0.001).ceil().max(0.0) as usize;
    (edge(w), edge(h))
}

/// The scale a `w` x `h` point page is checked at in full: 72 dpi, or the
/// largest within the view's pixel bound; `None` unless the size is finite
/// and positive and that scale is at least [`MIN_SCALE`].
fn full_scale(size: Option<(f64, f64)>) -> Option<f64> {
    let (w, h) = size?;
    if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
        return None;
    }
    let area = (MAX_RENDER_PIXELS as f64 / (w * h)).sqrt() * 0.99;
    let edge = f64::from(MAX_RENDER_EDGE - 1) / w.max(h);
    let full = area.min(edge).min(1.0);
    (full.is_finite() && full >= MIN_SCALE).then_some(full)
}

/// The marks whose rects, drawn at 72 dpi (less for a page past the
/// view's pixel bound), are not one colour two pixels in from their edges.
///
/// A page the view refuses to draw for its scale's cost (the render's
/// fill, clip and pixmap budgets) is drawn again at half the scale (each
/// of [`SCALES`] once), when that is at least [`MIN_SCALE`]. There the
/// check reads the pixels wholly inside each rect, so the band along its
/// edges it does not read is under 2 pt, as at 72 dpi; a rect the check
/// reads at 72 dpi with no such pixel refuses the page. Any ink left in a
/// pixel read shows as a different (antialiased) colour, which the exact
/// comparison catches. A page that cannot be drawn, or has no size the
/// check can draw (not finite or positive, or too large for [`MIN_SCALE`]
/// within the pixel bound), is refused, not failed.
pub fn pixels(output: &ViewDocument, marks: &[Mark]) -> Result<BTreeSet<usize>, PixelRefusal> {
    pixels_with(output, marks, RenderBudgets::default())
}

/// [`pixels`] with the pages drawn within `budgets` instead of the app's.
/// For tests of the reduced scales.
#[doc(hidden)]
pub fn pixels_with(
    output: &ViewDocument,
    marks: &[Mark],
    budgets: RenderBudgets,
) -> Result<BTreeSet<usize>, PixelRefusal> {
    let mut pages: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (index, mark) in marks.iter().enumerate() {
        pages.entry(mark.page).or_default().push(index);
    }
    let mut failed = BTreeSet::new();
    let mut budget = MAX_PIXEL_ROWS;
    for (page, on_page) in pages {
        let size = output.page_size(page).ok();
        let Some(full) = full_scale(size) else {
            return Err(PixelRefusal::Undrawable(page));
        };
        let (pw, ph) = size.unwrap_or_default();
        let mut drawn = None;
        let mut refused = false;
        for factor in SCALES {
            let scale = full * factor;
            if scale < MIN_SCALE {
                break;
            }
            match output.render_with_budgets(page, scale, RenderOptions::default(), budgets) {
                Ok(image) => {
                    drawn = Some((image, scale));
                    break;
                }
                Err(e) if e.scale_bound() => refused = true,
                Err(ViewError::TooComplex(_)) => return Err(PixelRefusal::TooComplex(page)),
                Err(_) => break,
            }
        }
        let Some((image, scale)) = drawn else {
            if refused {
                return Err(PixelRefusal::TooComplex(page));
            }
            failed.extend(on_page);
            continue;
        };
        let (w, h) = (image.width as usize, image.height as usize);
        // What the check reads at 72 dpi, the reference a reduced scale
        // must still read.
        let (rw, rh) = image_size(pw, ph, 1.0);
        // Each distinct pixel rect once, with the marks it belongs to
        // (identical quads, which a file can bring by the hundred
        // thousand, are checked once).
        let mut rects: BTreeMap<[usize; 4], Vec<usize>> = BTreeMap::new();
        for &index in &on_page {
            let Some(mark) = marks.get(index) else {
                continue;
            };
            for r in mark.rects() {
                let here = interior(&r, scale, w, h);
                if scale < 1.0 && here.is_none() && interior(&r, 1.0, rw, rh).is_some() {
                    return Err(PixelRefusal::TooThin(page));
                }
                let Some(key) = here else {
                    continue;
                };
                let owners = rects.entry(key).or_default();
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
            return Err(PixelRefusal::TooMany(page));
        }
        budget -= rows;
        let bad = uniform_rows(&image.pixels, w, rects.keys());
        for (at, owners) in rects.values().enumerate() {
            if bad.get(at).copied().unwrap_or(true) {
                failed.extend(owners.iter().copied());
            }
        }
    }
    Ok(failed)
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

/// Which text of content `texts` hands its visitor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    /// One string of a show operator.
    Piece,
    /// All the strings of one show operator, read together.
    Shown,
    /// An alternate text of a property list.
    Alt,
    /// A comment.
    Comment,
}

/// Calls `visit` with the text of content `bytes` a reader could take for
/// the page's: the strings each show operator shows (one by one, and read
/// together, as a `TJ` array's), the alternate texts of property lists
/// (/ActualText, /Alt, /E) and comments; `skip` spans (inline image data)
/// are not read, and without them an inline image's data is skipped to the
/// `EI` after it. Stops at the first `visit` that answers true.
fn texts(bytes: &[u8], skip: &[Range<usize>], mut visit: impl FnMut(Part, &[u8]) -> bool) -> bool {
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
                if visit(Part::Comment, bytes.get(start..i).unwrap_or(&[])) {
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
                        if shown.iter().any(|s| visit(Part::Piece, s))
                            || visit(Part::Shown, &together)
                        {
                            return true;
                        }
                    }
                    if alt.iter().any(|s| visit(Part::Alt, s)) {
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
    alt.iter().any(|s| visit(Part::Alt, s))
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
    /// R5: how often each box run (`box_runs`) is still in the content the
    /// marked pages reach (shown strings read per operator, alternate
    /// texts, comments).
    pub runs: Vec<usize>,
    /// The same in alternate texts and comments only (counted apart: a
    /// tagged page's /ActualText repeats what it shows).
    pub runs_alt: Vec<usize>,
}

/// The text check (a walk of each rewritten marked page) and the raw check
/// (each mark's search text in the content the marked pages reach: what
/// they draw, and the Forms their resources reach).
pub fn content(
    doc: &Document<'static>,
    view: Option<&ViewDocument>,
    marks: &[Mark],
    rule: &GlyphRule,
    runs: &[BoxRun],
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
            if rule.spent() {
                return Err(ApplyError::TooComplex(page));
            }
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
                if rule.spent() {
                    return Err(ApplyError::TooComplex(page));
                }
                if rule.removes_glyph(page, &glyph.bounds, &glyph.quad) {
                    out.text.extend(marks_at(marks, page, &glyph.bounds));
                }
            }
        }
        if searches.is_empty() && runs.is_empty() {
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
    out.runs = vec![0; runs.len()];
    out.runs_alt = vec![0; runs.len()];
    if !runs.is_empty() {
        let counter = RunCounter::new(runs);
        for (bytes, skip) in &streams {
            count_texts(&counter, bytes, skip, &mut out.runs, &mut out.runs_alt);
        }
    }
    for search in searches {
        let needles = needles(search);
        let found = streams.iter().any(|(bytes, skip)| {
            texts(bytes, skip, |_, text| {
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

/// Characters a box run (R5) has at least, white space aside.
pub const MIN_RUN_CHARS: usize = 4;
/// Characters a word of a box run has at least to be looked for on its own.
const MIN_WORD_CHARS: usize = 6;
/// Box runs one apply checks at most (the first found, page by page).
const MAX_RUNS: usize = 4096;
/// Characters of a box run kept (a longer run is checked by its start).
const MAX_RUN_CHARS: usize = 256;

/// Text the view read inside area boxes before apply (R5): each maximal
/// stretch of one line whose characters' centres lie in a box, white space
/// collapsed, with at least `MIN_RUN_CHARS` characters besides white space.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoxRun {
    pub text: String,
    /// The pages it was read on.
    pub pages: BTreeSet<usize>,
}

/// `text` without white space: how runs and raw strings are compared (a
/// `TJ` array places words without a space character).
fn squeezed(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// The texts a raw string may hold, squeezed: as a PDF text string (byte
/// order marks, else PDFDocEncoding), and as UTF-16BE without its mark
/// when its high bytes are mostly zero.
fn decodings(bytes: &[u8]) -> Vec<String> {
    let mut out = vec![squeezed(&crate::object::decode_text_string(bytes))];
    let pairs = bytes.len() / 2;
    let zeros = bytes
        .chunks_exact(2)
        .filter(|p| p.first() == Some(&0))
        .count();
    if pairs > 0 && bytes.len() % 2 == 0 && zeros * 2 >= pairs {
        let units: Vec<u16> = (bytes.chunks_exact(2))
            .map(|p| u16::from_be_bytes([p[0], p[1]]))
            .collect();
        out.push(squeezed(&String::from_utf16_lossy(&units)));
    }
    out
}

/// Counts box runs (squeezed) in texts in one pass per text: runs are
/// found by their first `MIN_RUN_CHARS` characters, then compared whole.
/// Occurrences of one run do not overlap.
pub struct RunCounter {
    wanted: Vec<Vec<char>>,
    by_prefix: std::collections::HashMap<[char; MIN_RUN_CHARS], Vec<usize>>,
}

impl RunCounter {
    pub fn new(runs: &[BoxRun]) -> Self {
        let wanted: Vec<Vec<char>> = (runs.iter())
            .map(|r| squeezed(&r.text).chars().collect())
            .collect();
        let mut by_prefix: std::collections::HashMap<[char; MIN_RUN_CHARS], Vec<usize>> =
            std::collections::HashMap::new();
        for (i, chars) in wanted.iter().enumerate() {
            if let Ok(prefix) =
                <[char; MIN_RUN_CHARS]>::try_from(chars.get(..MIN_RUN_CHARS).unwrap_or(&[]))
            {
                by_prefix.entry(prefix).or_default().push(i);
            }
        }
        Self { wanted, by_prefix }
    }

    pub fn len(&self) -> usize {
        self.wanted.len()
    }

    pub fn is_empty(&self) -> bool {
        self.wanted.is_empty()
    }

    /// Adds each run's occurrences in `hay` (squeezed characters) to `out`.
    fn count_chars(&self, hay: &[char], out: &mut [usize]) {
        let mut next_free = vec![0usize; self.wanted.len()];
        for start in 0..hay.len() {
            let Some(prefix) = hay.get(start..start + MIN_RUN_CHARS) else {
                break;
            };
            let Ok(prefix) = <[char; MIN_RUN_CHARS]>::try_from(prefix) else {
                break;
            };
            let Some(candidates) = self.by_prefix.get(&prefix) else {
                continue;
            };
            for &i in candidates {
                let (Some(want), Some(free)) = (self.wanted.get(i), next_free.get_mut(i)) else {
                    continue;
                };
                if start >= *free && hay.get(start..start + want.len()) == Some(want.as_slice()) {
                    *free = start + want.len();
                    if let Some(count) = out.get_mut(i) {
                        *count += 1;
                    }
                }
            }
        }
    }

    /// Adds each run's occurrences in the likeliest decoding of `forms`.
    fn count_forms(&self, forms: &[String], out: &mut [usize]) {
        let mut best = vec![0usize; self.wanted.len()];
        for form in forms {
            let chars: Vec<char> = form.chars().collect();
            let mut here = vec![0usize; self.wanted.len()];
            self.count_chars(&chars, &mut here);
            for (b, h) in best.iter_mut().zip(here) {
                *b = (*b).max(h);
            }
        }
        for (o, b) in out.iter_mut().zip(best) {
            *o = o.saturating_add(b);
        }
    }

    /// Adds each run's occurrences in a raw string.
    fn count_raw(&self, bytes: &[u8], out: &mut [usize]) {
        self.count_forms(&decodings(bytes), out);
    }

    /// Whether `bytes` (a string, read every way `count_raw` reads it)
    /// hold any run.
    pub fn quotes(&self, bytes: &[u8]) -> bool {
        let mut out = vec![0usize; self.len()];
        self.count_raw(bytes, &mut out);
        out.iter().any(|&n| n > 0)
    }
}

/// R5: the text `view` (the document before apply) reads inside the area
/// marks of `marks`, as runs; a page whose text cannot be read gives none
/// (the walk check still holds for it).
pub fn box_runs(view: &ViewDocument, marks: &[Mark]) -> Vec<BoxRun> {
    let mut by_page: BTreeMap<usize, Vec<[f64; 4]>> = BTreeMap::new();
    for mark in marks.iter().filter(|m| m.by_area) {
        by_page.entry(mark.page).or_default().extend(mark.rects());
    }
    let pages: Vec<usize> = by_page.keys().copied().collect();
    let boxes = region::Regions::from_rects(by_page);
    let reader = view.text_reader();
    let mut out: Vec<BoxRun> = Vec::new();
    for page in pages {
        let Ok(text) = reader.structured_text(page) else {
            continue;
        };
        let inside = |q: &[f64; 8]| {
            let x = (q[0] + q[2] + q[4] + q[6]) / 4.0;
            let y = (q[1] + q[3] + q[5] + q[7]) / 4.0;
            x.is_finite() && y.is_finite() && boxes.meets(page, &[x, y, x, y])
        };
        for block in text.unclipped_blocks() {
            for line in &block.lines {
                let taken: Vec<bool> = line.chars.iter().map(|c| inside(&c.quad)).collect();
                let mut runs: Vec<String> = vec![String::new()];
                for (i, c) in line.chars.iter().enumerate() {
                    let between = c.synthetic
                        && i > 0
                        && taken.get(i - 1) == Some(&true)
                        && taken.get(i + 1) == Some(&true);
                    match (taken.get(i) == Some(&true) || between, runs.last_mut()) {
                        (true, Some(run)) => run.push(c.c),
                        (false, Some(run)) if !run.is_empty() => runs.push(String::new()),
                        _ => {}
                    }
                }
                for run in runs {
                    let words: Vec<&str> = run.split_whitespace().collect();
                    let whole: String = words.join(" ").chars().take(MAX_RUN_CHARS).collect();
                    // The stretch itself, and each of its long words (a
                    // secret inside a line is looked for on its own too).
                    let long = (words.iter())
                        .filter(|w| w.chars().count() >= MIN_WORD_CHARS && words.len() > 1)
                        .map(|w| w.chars().take(MAX_RUN_CHARS).collect::<String>());
                    for text in std::iter::once(whole).chain(long) {
                        if squeezed(&text).chars().count() < MIN_RUN_CHARS {
                            continue;
                        }
                        if let Some(found) = out.iter_mut().find(|r| r.text == text) {
                            found.pages.insert(page);
                        } else if out.len() < MAX_RUNS {
                            out.push(BoxRun {
                                text,
                                pages: BTreeSet::from([page]),
                            });
                        }
                    }
                }
            }
        }
    }
    out
}

/// R5: what the annotations a box touches show or say, as runs added to
/// `runs` (each string whole and each word of `MIN_WORD_CHARS` or more):
/// their /Contents, /RC and string /V, and the strings their appearance
/// streams show. They go with the box; a copy of them elsewhere must not
/// stay.
pub fn annotation_runs(doc: &Document<'_>, marks: &[Mark], runs: &mut Vec<BoxRun>) {
    let regions = region::Regions::of(marks);
    let add = |text: &str, page: usize, runs: &mut Vec<BoxRun>| {
        let words: Vec<&str> = text.split_whitespace().collect();
        let whole: String = words.join(" ").chars().take(MAX_RUN_CHARS).collect();
        let long = (words.iter())
            .filter(|w| w.chars().count() >= MIN_WORD_CHARS && words.len() > 1)
            .map(|w| w.chars().take(MAX_RUN_CHARS).collect::<String>());
        for text in std::iter::once(whole).chain(long) {
            if squeezed(&text).chars().count() < MIN_RUN_CHARS {
                continue;
            }
            if let Some(found) = runs.iter_mut().find(|r| r.text == text) {
                found.pages.insert(page);
            } else if runs.len() < MAX_RUNS {
                runs.push(BoxRun {
                    text,
                    pages: BTreeSet::from([page]),
                });
            }
        }
    };
    for (index, page) in info::pages(doc).iter().enumerate() {
        if regions.on(index).is_empty() {
            continue;
        }
        for (annot, dict) in region::annotation_entries(doc, index, page.dict) {
            if !regions.touches(index, &annot.rect) {
                continue;
            }
            for key in [b"Contents".as_slice(), b"RC", b"V"] {
                if let Some(text) = doc.dict_get(dict, key).as_string() {
                    add(&crate::object::decode_text_string(text), index, runs);
                }
            }
            for id in region::appearance_streams(doc, dict) {
                let Ok(bytes) = doc.decoded_stream(id, StreamLimits::default()) else {
                    continue;
                };
                texts(&bytes, &[], |part, text| {
                    if part == Part::Shown {
                        add(&crate::object::decode_text_string(text), index, runs);
                    }
                    false
                });
            }
        }
    }
}

/// R5: how often the copy's view shows each run on `pages`, read as the
/// runs were (its characters line by line, white space aside), only what
/// the drawing shows (not what a clip hides). A page whose text cannot be read shows none.
pub fn visible_runs(
    output: &ViewDocument,
    pages: impl IntoIterator<Item = usize>,
    counter: &RunCounter,
) -> Vec<usize> {
    let mut out = vec![0usize; counter.len()];
    let reader = output.text_reader();
    for page in pages {
        let Ok(text) = reader.structured_text(page) else {
            continue;
        };
        for block in text.blocks() {
            for line in &block.lines {
                let chars: Vec<char> = (line.chars.iter())
                    .map(|c| c.c)
                    .filter(|c| !c.is_whitespace())
                    .collect();
                counter.count_chars(&chars, &mut out);
            }
        }
    }
    out
}

/// Adds each run's occurrences in content `bytes`: in what its show
/// operators show (read per operator) to `shown`, in its alternate texts
/// and comments to `alt`.
fn count_texts(
    counter: &RunCounter,
    bytes: &[u8],
    skip: &[Range<usize>],
    shown: &mut [usize],
    alt: &mut [usize],
) {
    texts(bytes, skip, |part, text| {
        match part {
            Part::Piece => {}
            Part::Shown => counter.count_raw(text, shown),
            Part::Alt | Part::Comment => counter.count_raw(text, alt),
        }
        false
    });
}

/// Content streams the backstop reads at most; past it apply fails closed.
const MAX_BACKSTOP_UNITS: usize = 20_000;

/// Why the backstop could not read the copy: both fail closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backstop {
    /// Past its bounds (streams, entries visited, decoded bytes).
    TooComplex,
    /// A stream does not decode, its text cannot be read, or a stream that
    /// shows text has no resources to read it with.
    Unreadable,
}

/// One unit the backstop reads: content streams and the resources to read
/// them with (each candidate counted, the largest count kept).
struct Unit {
    ids: Vec<ObjectId>,
    candidates: Vec<crate::object::Dictionary>,
}

/// R5's backstop (third and fifth reviews): each run's occurrences in every
/// content stream the written copy holds, read by the view as text —
/// through each font's encoding and /ToUnicode, CID fonts included, and
/// across the separate shows of a line — with every layer shown: each
/// page's own content and every Form or annotation appearance,
/// tiling-pattern cell and Type 3 glyph procedure, each on a page of its own
/// with no XObjects (so a Form drawn twice counts once). A Form or glyph
/// procedure without its own resources is read with those of every
/// resources dictionary that names it (each, the largest count kept), else
/// the form's /DR, else none when it shows no text; otherwise the backstop
/// fails. Returns the counts in what they show and in their alternate texts
/// and comments (read raw). `plain` is the copy written without encryption.
/// Nothing is skipped.
#[allow(clippy::too_many_lines)]
pub fn stream_runs(
    plain: Vec<u8>,
    counter: &RunCounter,
) -> Result<(Vec<usize>, Vec<usize>), Backstop> {
    use crate::app::edit::{Builder, Edit};
    use crate::object::Dictionary;
    let mut shown = vec![0usize; counter.len()];
    let mut alt = vec![0usize; counter.len()];
    if counter.is_empty() {
        return Ok((shown, alt));
    }
    // As the app reads (`limits`, `open_options`): a stream naming an
    // external file is read from its own bytes.
    let doc = Document::open_shared_with(
        std::sync::Arc::new(plain),
        crate::app::document::limits(),
        b"",
        crate::app::document::open_options(),
    )
    .map_err(|_| Backstop::Unreadable)?;
    let resolved = |value: Option<&Object>| value.and_then(|v| doc.resolve(v).as_dict()).cloned();
    // Every resources dictionary, once, and what its /XObject and /Font
    // name (one pass, one budget).
    let mut dictionaries: Vec<Dictionary> = Vec::new();
    let mut by_holder: BTreeMap<ObjectId, usize> = BTreeMap::new();
    let mut named_in: BTreeMap<ObjectId, BTreeSet<usize>> = BTreeMap::new();
    let mut visits = 0usize;
    for object in doc.objects() {
        let dict = match object.value.as_dict() {
            Some(d) => d,
            None => continue,
        };
        let Some(value) = dict.get(b"Resources".as_slice()) else {
            continue;
        };
        let Some(resources) = doc.resolve(value).as_dict() else {
            continue;
        };
        // Indirect resources once; a direct one belongs to its owner.
        let key = value.as_reference().unwrap_or(object.id);
        let index = match by_holder.get(&key) {
            Some(&i) => i,
            None => {
                dictionaries.push(resources.clone());
                by_holder.insert(key, dictionaries.len() - 1);
                dictionaries.len() - 1
            }
        };
        for category in [b"XObject".as_slice(), b"Font"] {
            let Some(entries) = doc.dict_get(resources, category).as_dict() else {
                continue;
            };
            for entry in entries.values() {
                visits += 1;
                if visits > MAX_RAW_ENTRIES {
                    return Err(Backstop::TooComplex);
                }
                if let Some(id) = entry.as_reference() {
                    named_in.entry(id).or_default().insert(index);
                }
            }
        }
    }
    let default = (doc.resolve_id(doc.root()).as_dict())
        .and_then(|c| doc.dict_get(c, b"AcroForm").as_dict())
        .and_then(|a| resolved(a.get(b"DR".as_slice())));
    let shows_text = |id: ObjectId| -> bool {
        doc.decoded_stream(id, StreamLimits::default())
            .map_or(true, |bytes| {
                bytes
                    .windows(2)
                    .any(|w| w == b"Tj" || w == b"TJ" || w == b"Tf")
                    || bytes.contains(&b'\'')
                    || bytes.contains(&b'"')
            })
    };
    // The candidates for content without resources of its own, `holder`
    // being what resources name (the Form, or a Type 3 font).
    let fallback = |holder: ObjectId, ids: &[ObjectId]| -> Result<Vec<Dictionary>, Backstop> {
        let named: Vec<Dictionary> = (named_in.get(&holder).into_iter().flatten())
            .filter_map(|&i| dictionaries.get(i).cloned())
            .collect();
        if !named.is_empty() {
            return Ok(named);
        }
        if let Some(default) = &default {
            return Ok(vec![default.clone()]);
        }
        match ids.iter().any(|id| shows_text(*id)) {
            true => Err(Backstop::Unreadable),
            false => Ok(vec![Dictionary::new()]),
        }
    };
    let mut units: Vec<Unit> = Vec::new();
    for page in info::pages(&doc) {
        // A page without resources has none to fall back on.
        let resources =
            resolved(pagetree::inherited(&doc, page.dict, b"Resources")).unwrap_or_default();
        units.push(Unit {
            ids: content::content_streams(&doc, page.dict),
            candidates: vec![resources],
        });
    }
    for object in doc.objects() {
        let Some(dict) = object.value.as_dict() else {
            continue;
        };
        if object.stream.is_some() {
            let image = doc.dict_get(dict, b"Subtype").as_name() == Some(b"Image");
            let form = doc.dict_get(dict, b"Subtype").as_name() == Some(b"Form")
                || (dict.get(b"BBox".as_slice()).is_some() && !image)
                || doc.dict_get(dict, b"PatternType").as_i64() == Some(1);
            if form {
                let candidates = match resolved(dict.get(b"Resources".as_slice())) {
                    Some(own) => vec![own],
                    None => fallback(object.id, &[object.id])?,
                };
                units.push(Unit {
                    ids: vec![object.id],
                    candidates,
                });
            }
        }
        if let Some(procs) = doc.dict_get(dict, b"CharProcs").as_dict() {
            let procs: Vec<ObjectId> = procs.values().filter_map(Object::as_reference).collect();
            let candidates = match resolved(dict.get(b"Resources".as_slice())) {
                Some(own) => vec![own],
                None => fallback(object.id, &procs)?,
            };
            for id in procs {
                units.push(Unit {
                    ids: vec![id],
                    candidates: candidates.clone(),
                });
            }
        }
        if units.len() > MAX_BACKSTOP_UNITS {
            return Err(Backstop::TooComplex);
        }
    }
    let pages: usize = units.iter().map(|u| u.candidates.len()).sum();
    if pages > MAX_BACKSTOP_UNITS {
        return Err(Backstop::TooComplex);
    }
    // Alternate texts and comments, read raw; every stream must decode.
    let mut read = 0usize;
    let mut seen = BTreeSet::new();
    for unit in &units {
        for id in &unit.ids {
            if !seen.insert(*id) {
                continue;
            }
            let room = MAX_RAW_BYTES.saturating_sub(read);
            if room == 0 {
                return Err(Backstop::TooComplex);
            }
            let limits = StreamLimits {
                max_decoded_bytes: room,
                max_total_decoded_bytes: room,
                ..StreamLimits::default()
            };
            let bytes = doc
                .decoded_stream(*id, limits)
                .map_err(|_| Backstop::Unreadable)?;
            read = read.saturating_add(bytes.len());
            let mut ignored = vec![0usize; counter.len()];
            count_texts(counter, &bytes, &[], &mut ignored, &mut alt);
        }
    }
    // One page per unit and candidate, in a fresh page tree, every layer
    // shown; a stream naming an external file is given as its own bytes
    // (the view refuses /F).
    let external: Vec<ObjectId> = (doc.objects())
        .filter(|o| o.stream.is_some())
        .filter(|o| {
            (o.value.as_dict())
                .and_then(|d| d.get(b"F".as_slice()))
                .is_some_and(|f| !f.is_null())
        })
        .map(|o| o.id)
        .collect();
    let mut builder = Builder::from_document(doc);
    for id in external {
        builder
            .update(id, |value| {
                if let Some(dict) = value.as_dict_mut() {
                    for key in [b"F".as_slice(), b"FFilter", b"FDecodeParms"] {
                        dict.remove(key);
                    }
                }
            })
            .map_err(|_| Backstop::Unreadable)?;
    }
    let shown_all = crate::app::layers::show_all(builder.document());
    shown_all
        .apply(&mut builder)
        .map_err(|_| Backstop::Unreadable)?;
    let pages_node = builder
        .allocate(Object::Null)
        .map_err(|_| Backstop::Unreadable)?;
    let mut kids = Vec::with_capacity(pages);
    let mut owner: Vec<usize> = Vec::with_capacity(pages);
    for (u, unit) in units.iter().enumerate() {
        for candidate in &unit.candidates {
            let mut resources = candidate.clone();
            resources.remove(b"XObject".as_slice());
            let mut page = Dictionary::new();
            page.insert(b"Type".to_vec(), Object::name("Page"));
            page.insert(b"Parent".to_vec(), Object::Reference(pages_node));
            page.insert(
                b"MediaBox".to_vec(),
                Object::Array([-14400, -14400, 14400, 14400].map(Object::Integer).to_vec()),
            );
            page.insert(b"Resources".to_vec(), Object::Dictionary(resources));
            page.insert(
                b"Contents".to_vec(),
                Object::Array(unit.ids.iter().copied().map(Object::Reference).collect()),
            );
            let id = builder
                .allocate(Object::Dictionary(page))
                .map_err(|_| Backstop::Unreadable)?;
            kids.push(Object::Reference(id));
            owner.push(u);
        }
    }
    let count = kids.len();
    let mut node = Dictionary::new();
    node.insert(b"Type".to_vec(), Object::name("Pages"));
    node.insert(b"Kids".to_vec(), Object::Array(kids));
    node.insert(b"Count".to_vec(), Object::Integer(count as i64));
    builder
        .put_value(pages_node, Object::Dictionary(node))
        .map_err(|_| Backstop::Unreadable)?;
    let root = builder.document().root();
    builder
        .update(root, |value| {
            if let Some(dict) = value.as_dict_mut() {
                dict.insert(b"Pages".to_vec(), Object::Reference(pages_node));
            }
        })
        .map_err(|_| Backstop::Unreadable)?;
    let plain = crate::writer::WriteOptions {
        encryption: crate::writer::Encryption::Plaintext,
        ..crate::writer::WriteOptions::APP
    };
    let bytes =
        crate::writer::write_document(builder.document(), crate::app::overlay::limits(), plain)
            .map_err(|_| Backstop::TooComplex)?;
    let view = ViewDocument::open(bytes, "").map_err(|_| Backstop::Unreadable)?;
    let reader = view.text_reader();
    // Per unit, the largest count over its candidates.
    let mut best: Vec<Vec<usize>> = vec![vec![0usize; counter.len()]; units.len()];
    for (page, &u) in owner.iter().enumerate() {
        let text = reader
            .structured_text(page)
            .map_err(|_| Backstop::Unreadable)?;
        let mut here = vec![0usize; counter.len()];
        for block in text.unclipped_blocks() {
            for line in &block.lines {
                let chars: Vec<char> = (line.chars.iter())
                    .map(|c| c.c)
                    .filter(|c| !c.is_whitespace())
                    .collect();
                counter.count_chars(&chars, &mut here);
            }
        }
        if let Some(slot) = best.get_mut(u) {
            for (b, h) in slot.iter_mut().zip(here) {
                *b = (*b).max(h);
            }
        }
    }
    for unit in best {
        for (s, c) in shown.iter_mut().zip(unit) {
            *s = s.saturating_add(c);
        }
    }
    Ok((shown, alt))
}

/// Keys whose string values hold text a reader can take outside page
/// content: annotation and field texts, alternate texts and titles of the
/// structure tree, outline and document titles, document information;
/// sixth review: an annotation's subject, layer and configuration names
/// and creators, page-label prefixes, named destinations a link or action
/// goes to, languages and a file's names and description.
const HIDDEN_KEYS: [&[u8]; 25] = [
    // JavaScript (an action's script; fifth review: fail closed).
    b"JS",
    b"Contents",
    // A button's caption (/MK /CA). A field's name (/T) is left out: a
    // form's label is often its name, and boxing the label would fail.
    // An annotation's author (/T of one that is not a widget) counts.
    b"CA",
    b"RC",
    b"V",
    b"DV",
    b"TU",
    b"ActualText",
    b"Alt",
    b"E",
    b"Title",
    b"Subject",
    b"Subj",
    b"Author",
    b"Keywords",
    // Optional-content group and configuration names, their creators.
    b"Name",
    b"Creator",
    // A page label's prefix (only page labels hold a text /P).
    b"P",
    // Named destinations (a link's, a go-to action's).
    b"Dest",
    b"D",
    b"Lang",
    // File specifications: names and description.
    b"F",
    b"UF",
    b"Desc",
    b"DOS",
];

/// Values `hidden_runs` visits at most (strings, arrays, dictionaries).
const MAX_HIDDEN_VALUES: usize = 8_000_000;

/// R5: how often each run is in what the copy holds outside page content:
/// the strings of `HIDDEN_KEYS` in every object (annotations, fields,
/// structure elements, outlines, property lists, layers), layer-list
/// labels, name-tree keys and destination names, every string of the
/// document information dictionary, the decoded XMP metadata, XFA and
/// script streams.
/// `doc` is the copy as written (reopened), so nothing unwritten counts.
pub fn hidden_runs(doc: &Document<'_>, counter: &RunCounter) -> Vec<usize> {
    let mut out = vec![0usize; counter.len()];
    if counter.is_empty() {
        return out;
    }
    let mut visits = 0usize;
    let info = doc
        .trailer()
        .get(b"Info".as_slice())
        .map(|i| doc.resolve(i));
    if let Some(info) = info.and_then(|i| i.as_dict()) {
        for value in info.values() {
            if let Some(text) = doc.resolve(value).as_string() {
                counter.count_raw(text, &mut out);
            }
        }
    }
    // The form's XFA (one stream, or [name stream ...]) and JavaScript held
    // in streams: their text, markup aside.
    let mut streams: Vec<ObjectId> = Vec::new();
    let acro =
        (doc.resolve_id(doc.root()).as_dict()).and_then(|c| doc.dict_get(c, b"AcroForm").as_dict());
    if let Some(xfa) = acro.and_then(|a| a.get(b"XFA".as_slice())) {
        match doc.resolve(xfa) {
            Object::Array(items) => streams.extend(items.iter().filter_map(Object::as_reference)),
            _ => streams.extend(xfa.as_reference()),
        }
    }
    // Every object's values, direct parts at any depth: the strings of
    // `HIDDEN_KEYS`; the labels of a layer list (/Order); the keys of name
    // trees (/Names, /Limits); names a destination or go-to action is
    // given by; the script streams /JS names (in any action, direct or
    // not).
    for object in doc.objects() {
        // (value, every string in it counts)
        let mut stack: Vec<(&Object, bool)> = vec![(&object.value, false)];
        while let Some((value, every)) = stack.pop() {
            visits += 1;
            if visits > MAX_HIDDEN_VALUES {
                break;
            }
            match value {
                Object::String(text) if every => counter.count_raw(text, &mut out),
                Object::Array(items) => stack.extend(items.iter().map(|i| (i, every))),
                Object::Dictionary(dict) => {
                    let author = dict
                        .get(b"Subtype".as_slice())
                        .and_then(Object::as_name)
                        .is_some_and(|s| s != b"Widget")
                        && dict.get(b"Rect".as_slice()).is_some();
                    for (key, item) in dict.iter() {
                        let key = key.as_slice();
                        match item {
                            Object::String(text)
                                if HIDDEN_KEYS.contains(&key) || (author && key == b"T") =>
                            {
                                counter.count_raw(text, &mut out);
                            }
                            Object::Name(name) if matches!(key, b"Dest" | b"D") => {
                                counter.count_raw(name, &mut out);
                            }
                            Object::Reference(id) if key == b"JS" => streams.push(*id),
                            Object::Array(items) if key == b"Names" => {
                                // A name tree's keys and values.
                                for (at, entry) in items.iter().enumerate() {
                                    match entry {
                                        Object::String(text) if at % 2 == 0 => {
                                            counter.count_raw(text, &mut out);
                                        }
                                        _ => stack.push((entry, every)),
                                    }
                                }
                            }
                            Object::Array(_) if matches!(key, b"Order" | b"Limits") => {
                                stack.push((item, true));
                            }
                            Object::Array(_) | Object::Dictionary(_) => stack.push((item, every)),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        let metadata = (object.value.as_dict())
            .is_some_and(|d| doc.dict_get(d, b"Type").as_name() == Some(b"Metadata"));
        if metadata && object.stream.is_some() {
            if let Ok(bytes) = doc.decoded_stream(object.id, StreamLimits::default()) {
                // XMP's text, its markup aside (tag names are not text).
                let text = squeezed(&without_markup(&String::from_utf8_lossy(&bytes)));
                counter.count_forms(&[text], &mut out);
            }
        }
    }
    streams.sort_unstable();
    streams.dedup();
    for id in streams {
        if let Ok(bytes) = doc.decoded_stream(id, StreamLimits::default()) {
            let text = squeezed(&without_markup(&String::from_utf8_lossy(&bytes)));
            counter.count_forms(&[text], &mut out);
        }
    }
    // The names of the catalog's destinations (PDF 1.1: a dictionary).
    let dests =
        (doc.resolve_id(doc.root()).as_dict()).and_then(|c| doc.dict_get(c, b"Dests").as_dict());
    for key in dests.into_iter().flat_map(|d| d.keys()) {
        counter.count_raw(key, &mut out);
    }
    out
}

/// `xml` without its tags: each `<...>` becomes a space.
fn without_markup(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut inside = false;
    for c in xml.chars() {
        match (c, inside) {
            ('<', _) => {
                inside = true;
                out.push(' ');
            }
            ('>', true) => inside = false,
            (_, false) => out.push(c),
            _ => {}
        }
    }
    out
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
        texts(bytes, &[], |_, t| needles.iter().any(|n| contains(t, n)))
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
