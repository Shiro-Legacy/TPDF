// SPDX-License-Identifier: MIT
//! Print-to-PDF glyph lines (content-edit.ts `editCollapseGlyphLines`):
//! Quartz writes one `q … cm BT … Tf (x) Tj ET Q Q` block per glyph; the
//! consecutive blocks of one line become one editable text target whose
//! operand span is the whole chain.
use std::collections::BTreeMap;
use std::sync::Arc;

use super::font::{Font, at, latin1, number};
use super::scan::{CidWrite, Kind, Record, target_id};
use super::walk::Scanner;
use super::{MAX_EDIT_TEXT_CHARS, scan_error};
use crate::app::AppError;
use crate::object::{Dictionary, Object, ObjectId};

/// One Quartz glyph block, keyed by its string operand's start.
#[derive(Clone, Debug, PartialEq)]
pub struct GlyphBlock {
    pub start: usize,
    pub end: usize,
    /// The colour operator group as written, "" when none.
    pub colour: String,
    pub scale: f64,
    pub d: f64,
    pub x: f64,
    pub y: f64,
    /// The font name as written (no `#` decoding).
    pub font: Vec<u8>,
}

/// A glyph line's members: code, x in user space, advance.
#[derive(Clone, Debug, PartialEq)]
pub struct GlyphLine {
    pub glyphs: Vec<(u32, f64, f64)>,
    pub y: f64,
    pub scale: f64,
    pub colour: String,
    /// The word gap in em.
    pub space: f64,
}

fn is_ws(b: u8) -> bool {
    matches!(b, 0 | 9 | 10 | 12 | 13 | 32)
}

fn is_delim_before(b: u8) -> bool {
    is_ws(b) || matches!(b, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}')
}

/// A cursor implementing the block pattern's pieces.
struct Cursor<'b> {
    bytes: &'b [u8],
    at: usize,
}

impl Cursor<'_> {
    fn ws(&mut self) -> Option<()> {
        let start = self.at;
        while self.bytes.get(self.at).copied().is_some_and(is_ws) {
            self.at += 1;
        }
        (self.at > start).then_some(())
    }
    /// `[+-]?(?:\d+(?:\.\d*)?|\.\d+)`.
    fn num(&mut self) -> Option<f64> {
        let start = self.at;
        if matches!(self.bytes.get(self.at), Some(b'+' | b'-')) {
            self.at += 1;
        }
        let digits = |c: &mut Self| {
            let s = c.at;
            while c.bytes.get(c.at).is_some_and(u8::is_ascii_digit) {
                c.at += 1;
            }
            c.at - s
        };
        if digits(self) > 0 {
            if self.bytes.get(self.at) == Some(&b'.') {
                self.at += 1;
                digits(self);
            }
        } else if self.bytes.get(self.at) == Some(&b'.') {
            self.at += 1;
            if digits(self) == 0 {
                self.at = start;
                return None;
            }
        } else {
            self.at = start;
            return None;
        }
        let text = std::str::from_utf8(&self.bytes[start..self.at]).ok()?;
        let text = text.strip_prefix('+').unwrap_or(text);
        text.parse().ok().or(Some(f64::NAN))
    }
    /// `/[^\0\t\n\f\r ()<>\[\]{}/%]+`, without the slash.
    fn name(&mut self) -> Option<&[u8]> {
        if self.bytes.get(self.at) != Some(&b'/') {
            return None;
        }
        let start = self.at + 1;
        let mut end = start;
        while self.bytes.get(end).copied().is_some_and(|b| {
            !is_ws(b)
                && !matches!(
                    b,
                    b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
                )
        }) {
            end += 1;
        }
        if end == start {
            return None;
        }
        self.at = end;
        Some(&self.bytes[start..end])
    }
    fn lit(&mut self, word: &[u8]) -> Option<()> {
        if self.bytes.get(self.at..self.at + word.len()) == Some(word) {
            self.at += word.len();
            Some(())
        } else {
            None
        }
    }
    fn nums<const N: usize>(&mut self) -> Option<[f64; N]> {
        let mut out = [0.0; N];
        for (i, slot) in out.iter_mut().enumerate() {
            if i > 0 {
                self.ws()?;
            }
            *slot = self.num()?;
        }
        Some(out)
    }
    /// `\((?:\\[\s\S]|[^\\()])*\)|<[0-9a-fA-F\0\t\n\f\r ]*>`.
    fn operand(&mut self) -> Option<()> {
        match self.bytes.get(self.at)? {
            b'(' => {
                let mut i = self.at + 1;
                loop {
                    match self.bytes.get(i)? {
                        b'\\' => {
                            self.bytes.get(i + 1)?;
                            i += 2;
                        }
                        b'(' => return None,
                        b')' => {
                            self.at = i + 1;
                            return Some(());
                        }
                        _ => i += 1,
                    }
                }
            }
            b'<' => {
                let mut i = self.at + 1;
                while self
                    .bytes
                    .get(i)
                    .copied()
                    .is_some_and(|b| b.is_ascii_hexdigit() || is_ws(b))
                {
                    i += 1;
                }
                if self.bytes.get(i) != Some(&b'>') {
                    return None;
                }
                self.at = i + 1;
                Some(())
            }
            _ => None,
        }
    }
}

/// The colour group: `(?:NAME W cs W)?(?:N W){1,4}(?:scn|sc|g|rg|k)`.
fn colour(c: &mut Cursor<'_>) -> Option<()> {
    let start = c.at;
    let mut with_space = Cursor {
        bytes: c.bytes,
        at: c.at,
    };
    let named = with_space.name().is_some()
        && with_space.ws().is_some()
        && with_space.lit(b"cs").is_some()
        && with_space.ws().is_some();
    let attempt = |from: usize| -> Option<usize> {
        let mut c = Cursor {
            bytes: c.bytes,
            at: from,
        };
        let mut count = 0;
        while count < 4 {
            let save = c.at;
            if c.num().is_some() && c.ws().is_some() {
                count += 1;
            } else {
                c.at = save;
                break;
            }
        }
        if count == 0 {
            return None;
        }
        for op in [&b"scn"[..], b"sc", b"g", b"rg", b"k"] {
            if c.bytes.get(c.at..c.at + op.len()) == Some(op) {
                return Some(c.at + op.len());
            }
        }
        None
    };
    let end = if named {
        attempt(with_space.at).or_else(|| attempt(start))
    } else {
        attempt(start)
    }?;
    c.at = end;
    Some(())
}

/// A block's `cm` and `Tm` numbers, font name and size.
type Head = ([f64; 6], [f64; 6], Vec<u8>, f64);

/// The pattern's match at `q`: the block (None when its numbers are not a
/// glyph block's), the match end and the string operand's start.
fn block_at(bytes: &[u8], start: usize) -> Option<(Option<GlyphBlock>, usize, usize)> {
    let head_tail = |c: &mut Cursor<'_>| -> Option<Head> {
        c.ws()?;
        c.lit(b"q")?;
        c.ws()?;
        let m = c.nums::<6>()?;
        c.ws()?;
        c.lit(b"cm")?;
        c.ws()?;
        c.lit(b"BT")?;
        c.ws()?;
        let t = c.nums::<6>()?;
        c.ws()?;
        c.lit(b"Tm")?;
        c.ws()?;
        let font = c.name()?.to_vec();
        c.ws()?;
        let size = c.num()?;
        c.ws()?;
        c.lit(b"Tf")?;
        c.ws()?;
        Some((m, t, font, size))
    };
    // `q`, an optional clip, an optional colour, then the rest; each
    // optional part is tried present first, as the regex backtracks.
    let mut options = Vec::new();
    for clip in [true, false] {
        for colour_part in [true, false] {
            options.push((clip, colour_part));
        }
    }
    for (clip, colour_part) in options {
        let mut c = Cursor { bytes, at: start };
        c.lit(b"q")?;
        if clip {
            let ok = (|| {
                c.ws()?;
                c.nums::<4>()?;
                c.ws()?;
                c.lit(b"re")?;
                c.ws()?;
                c.lit(b"W")?;
                c.ws()?;
                c.lit(b"n")
            })();
            if ok.is_none() {
                continue;
            }
        }
        let mut colour_text = String::new();
        if colour_part {
            if c.ws().is_none() {
                continue;
            }
            let from = c.at;
            if colour(&mut c).is_none() {
                continue;
            }
            colour_text = latin1(&bytes[from..c.at]);
        }
        let Some((m, t, font, size)) = head_tail(&mut c) else {
            continue;
        };
        let operand_start = c.at;
        if c.operand().is_none() {
            continue;
        }
        while c.bytes.get(c.at).copied().is_some_and(is_ws) {
            c.at += 1;
        }
        let tail = (|| {
            c.lit(b"Tj")?;
            c.ws()?;
            c.lit(b"ET")?;
            c.ws()?;
            c.lit(b"Q")?;
            c.ws()?;
            c.lit(b"Q")
        })();
        if tail.is_none() {
            continue;
        }
        if let Some(&next) = bytes.get(c.at) {
            let regular = !is_ws(next)
                && !matches!(
                    next,
                    b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
                );
            if regular {
                continue;
            }
        }
        let [a, b, cc, d, e, f] = m;
        let scale = a;
        if scale.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater)
            || (d - scale).abs() > 1e-3 * scale
            || b != 0.0
            || cc != 0.0
            || size != 1.0
            || t != [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]
        {
            return Some((None, c.at, operand_start));
        }
        let block = GlyphBlock {
            start,
            end: c.at,
            colour: colour_text,
            scale,
            d,
            x: e,
            y: f,
            font,
        };
        return Some((Some(block), c.at, operand_start));
    }
    None
}

/// content-edit.ts `editGlyphBlocks`: every block of one stream, by its
/// string operand's start.
pub fn blocks(bytes: &[u8]) -> BTreeMap<usize, GlyphBlock> {
    let mut out = BTreeMap::new();
    let mut at = 0;
    while at < bytes.len() {
        let starts = bytes[at] == b'q' && (at == 0 || is_delim_before(bytes[at - 1]));
        if starts {
            if let Some((block, end, operand)) = block_at(bytes, at) {
                at = end.max(at + 1);
                if let Some(block) = block {
                    out.insert(operand, block);
                }
                continue;
            }
        }
        at += 1;
    }
    out
}

struct Member {
    index: usize,
    block: GlyphBlock,
    code: u32,
    adv: f64,
    colour: String,
    simple: Option<(ObjectId, Arc<BTreeMap<u32, f64>>)>,
    space: bool,
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() <= 0.01
}

/// content-edit.ts `simpleWidths`: a simple WinAnsi or MacRoman font's
/// /Widths by code.
fn simple_widths(
    scanner: &Scanner<'_, '_>,
    font: &Font,
    resources: Option<&Dictionary>,
    tf_name: &[u8],
) -> Option<BTreeMap<u32, f64>> {
    if !font.simple || font.encoding_unknown || font.object.is_none() {
        return None;
    }
    let doc = scanner.doc;
    let dict = super::font::dict_at(doc, resources, b"Font")
        .and_then(|fonts| fonts.get(tf_name))
        .and_then(|value| doc.resolve(value).as_dict())?;
    let encoding = at(doc, Some(dict), b"Encoding").and_then(Object::as_name)?;
    if encoding != b"WinAnsiEncoding" && encoding != b"MacRomanEncoding" {
        return None;
    }
    let first = number(at(doc, Some(dict), b"FirstChar"))?;
    let list = at(doc, Some(dict), b"Widths")?.as_array()?;
    let mut widths = BTreeMap::new();
    for (index, item) in list.iter().enumerate() {
        if let Some(width) = number(Some(doc.resolve(item))) {
            let code = first + index as f64;
            if (0.0..=u32::MAX as f64).contains(&code) && code.fract() == 0.0 {
                widths.insert(code as u32, width);
            }
        }
    }
    Some(widths)
}

fn glyph_of(
    scanner: &mut Scanner<'_, '_>,
    index: usize,
    resources: Option<&Dictionary>,
    width_cache: &mut BTreeMap<ObjectId, Option<Arc<BTreeMap<u32, f64>>>>,
) -> Option<Member> {
    let record = scanner.records.get(index)?;
    if !record.is_text() || record.depth != 0 || record.element.is_none() {
        return None;
    }
    let ambiguous = !record.candidate && record.reason == Some("ambiguous-text-association");
    if !record.candidate && !ambiguous {
        return None;
    }
    let code = u32::from(*record.payload.as_ref().filter(|p| p.len() == 1)?.first()?);
    let text = record.text.clone()?;
    let path = record.path.clone();
    let operand = record.operand.start;
    let tf_name = record.tf_name.clone()?;
    let block = scanner.glyph_blocks_of(&path).get(&operand)?.clone();
    let record = scanner.records.get(index)?;
    if block.font != tf_name {
        return None;
    }
    let mut chars = text.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        return None;
    };
    if c.is_whitespace() && c != ' ' {
        return None;
    }
    if record.render_mode != 0.0 || record.oc_depth != 0 || record.mcid.is_some() {
        return None;
    }
    if record.actual_text.is_some() || record.struct_actual_text.is_some() {
        return None;
    }
    if record.character_spacing != 0.0
        || record.word_spacing != 0.0
        || record.horizontal_scale != 1.0
    {
        return None;
    }
    let (linear, origin) = (record.text_linear?, record.origin?);
    let m = scanner.page.transform;
    if !near(linear[0], block.scale)
        || !near(linear[1], 0.0)
        || !near(linear[2], 0.0)
        || !near(linear[3], -block.d)
    {
        return None;
    }
    if !near(origin[0], m[0] * block.x + m[2] * block.y + m[4])
        || !near(origin[1], m[1] * block.x + m[3] * block.y + m[5])
    {
        return None;
    }
    let doc = scanner.doc;
    let font = scanner
        .fonts
        .resolve(doc, resources, Some(&tf_name), &mut scanner.budget)?;
    if font.is_subtype(b"Type0") {
        return None;
    }
    let (adv, simple) = match &font.cid {
        Some(cid) => {
            if cid.bytes_per_code != 1 || c == ' ' {
                return None;
            }
            (cid.width(code) / 1000.0 * block.scale, None)
        }
        None => {
            let object = font.object?;
            let widths = match width_cache.get(&object) {
                Some(cached) => cached.clone(),
                None => {
                    let widths = simple_widths(scanner, &font, resources, &tf_name).map(Arc::new);
                    width_cache.insert(object, widths.clone());
                    widths
                }
            }?;
            let width = *widths.get(&code)?;
            if !(0x20..=0x7e).contains(&code) || u32::from(c) != code {
                return None;
            }
            (width / 1000.0 * block.scale, Some((object, widths)))
        }
    };
    let colour = normalise_space(&block.colour);
    Some(Member {
        index,
        code,
        adv,
        colour,
        simple,
        space: code == 0x20,
        block,
    })
}

fn normalise_space(text: &str) -> String {
    let mut out = String::new();
    let mut in_space = false;
    for c in text.chars() {
        if matches!(c, '\0' | '\t' | '\n' | '\u{c}' | '\r' | ' ') {
            if !in_space {
                out.push(' ');
            }
            in_space = true;
        } else {
            out.push(c);
            in_space = false;
        }
    }
    out
}

/// What the collapse produces for one output position.
enum Out {
    Kept(usize),
    Line(Box<Record>),
}

/// content-edit.ts `editCollapseGlyphLines`. `map_order` becomes the record
/// map order (originals in order, then the new lines).
pub(super) fn collapse(
    scanner: &mut Scanner<'_, '_>,
    resources: Option<&Dictionary>,
    map_order: &mut Vec<usize>,
) -> Result<(), AppError> {
    if scanner.geometry_blocked {
        return Ok(());
    }
    let mut width_cache = BTreeMap::new();
    let count = scanner.records.len();
    let mut glyphs: Vec<Option<Member>> = Vec::with_capacity(count);
    for index in 0..count {
        glyphs.push(glyph_of(scanner, index, resources, &mut width_cache));
    }
    let mut drawn: BTreeMap<ObjectId, Vec<(char, u32)>> = BTreeMap::new();
    for glyph in glyphs.iter().flatten() {
        let Some((object, _)) = &glyph.simple else {
            continue;
        };
        if glyph.space {
            continue;
        }
        let Some(c) = char::from_u32(glyph.code) else {
            continue;
        };
        let entry = drawn.entry(*object).or_default();
        match entry.iter_mut().find(|(d, _)| *d == c) {
            Some(slot) => slot.1 = glyph.code,
            None => entry.push((c, glyph.code)),
        }
    }
    let mut out: Vec<Out> = Vec::new();
    let mut chain: Vec<Member> = Vec::new();
    let mut gaps: Vec<f64> = Vec::new();
    let mut lines = 0usize;
    let mut glyphs = glyphs.into_iter();
    for index in 0..count {
        let glyph = glyphs.next().flatten();
        let Some(glyph) = glyph else {
            flush(scanner, &mut chain, &mut gaps, &drawn, &mut out, &mut lines)?;
            out.push(Out::Kept(index));
            continue;
        };
        let mut gap = 0.0;
        if let Some(prev) = chain.last() {
            let path_prev = &scanner.records[prev.index].path;
            let path_this = &scanner.records[glyph.index].path;
            let mut between = path_prev == path_this && prev.block.end <= glyph.block.start;
            if between {
                if let Some(bytes) = scanner.streams.get(path_prev.as_str()) {
                    between = bytes
                        .get(prev.block.end..glyph.block.start)
                        .is_some_and(|gap| gap.iter().all(|b| is_ws(*b)));
                }
            }
            gap = (glyph.block.x - (prev.block.x + prev.adv)) / glyph.block.scale;
            let head = &chain[0];
            let same_font =
                scanner.records[glyph.index].tf_name == scanner.records[prev.index].tf_name;
            let joins = between
                && same_font
                && (glyph.block.scale - head.block.scale).abs() <= 1e-3 * head.block.scale
                && glyph.colour == prev.colour
                && (glyph.block.y - head.block.y).abs() < 0.005
                && gap > -0.2
                && gap <= 1.0;
            if !joins {
                flush(scanner, &mut chain, &mut gaps, &drawn, &mut out, &mut lines)?;
                gap = 0.0;
            }
        }
        if glyph.space && chain.is_empty() {
            out.push(Out::Kept(index));
            continue;
        }
        chain.push(glyph);
        gaps.push(gap);
    }
    flush(scanner, &mut chain, &mut gaps, &drawn, &mut out, &mut lines)?;
    if lines == 0 {
        return Ok(());
    }
    let mut old: Vec<Option<Record>> = std::mem::take(&mut scanner.records)
        .into_iter()
        .map(Some)
        .collect();
    let mut records = Vec::with_capacity(out.len());
    let mut originals: Vec<(usize, usize)> = Vec::new();
    let mut new_lines: Vec<usize> = Vec::new();
    for entry in out {
        match entry {
            Out::Kept(index) => {
                if let Some(record) = old.get_mut(index).and_then(Option::take) {
                    originals.push((index, records.len()));
                    records.push(record);
                }
            }
            Out::Line(record) => {
                new_lines.push(records.len());
                records.push(*record);
            }
        }
    }
    originals.sort_by_key(|(index, _)| *index);
    *map_order = originals
        .into_iter()
        .map(|(_, position)| position)
        .collect();
    map_order.extend(new_lines);
    scanner.records = records;
    Ok(())
}

fn flush(
    scanner: &Scanner<'_, '_>,
    chain: &mut Vec<Member>,
    gaps: &mut Vec<f64>,
    drawn: &BTreeMap<ObjectId, Vec<(char, u32)>>,
    out: &mut Vec<Out>,
    lines: &mut usize,
) -> Result<(), AppError> {
    let mut members = std::mem::take(chain);
    let mut flags = std::mem::take(gaps);
    let mut trailing = Vec::new();
    while members.last().is_some_and(|m| m.space) {
        if let Some(member) = members.pop() {
            trailing.push(member.index);
        }
        flags.pop();
    }
    trailing.reverse();
    collapse_members(scanner, members, flags, drawn, out, lines)?;
    out.extend(trailing.into_iter().map(Out::Kept));
    Ok(())
}

fn collapse_members(
    scanner: &Scanner<'_, '_>,
    members: Vec<Member>,
    flags: Vec<f64>,
    drawn: &BTreeMap<ObjectId, Vec<(char, u32)>>,
    out: &mut Vec<Out>,
    lines: &mut usize,
) -> Result<(), AppError> {
    if members.is_empty() {
        return Ok(());
    }
    let records = &scanner.records;
    let lender = members
        .iter()
        .find(|m| !m.space && records[m.index].candidate);
    let bounded: Vec<&Member> = members
        .iter()
        .filter(|m| records[m.index].bounds.is_some())
        .collect();
    let drawn_chars = lender
        .and_then(|l| l.simple.as_ref())
        .and_then(|(object, _)| drawn.get(object));
    let write = match lender {
        Some(Member {
            simple: Some((_, widths)),
            ..
        }) => Some(Arc::new(CidWrite {
            chars: drawn_chars.cloned().unwrap_or_default(),
            widths: widths.clone(),
            dw: 0.0,
            bytes_per_code: 1,
        })),
        Some(l) => records[l.index].cid_write.clone(),
        None => None,
    };
    let (Some(lender), Some(write), false) = (lender, write, bounded.is_empty()) else {
        out.extend(members.iter().map(|m| Out::Kept(m.index)));
        return Ok(());
    };
    let first = &members[0];
    let last = &members[members.len() - 1];
    let top = bounded
        .iter()
        .filter_map(|m| records[m.index].bounds.map(|b| b[1]))
        .fold(f64::INFINITY, f64::min);
    let bottom = bounded
        .iter()
        .filter_map(|m| records[m.index].bounds.map(|b| b[3]))
        .fold(f64::NEG_INFINITY, f64::max);
    let boxes: Vec<[f64; 4]> = members
        .iter()
        .map(|m| {
            let record = &records[m.index];
            record.bounds.unwrap_or_else(|| {
                let x = record.origin.map_or(0.0, |o| o[0]);
                [x, top, x + m.adv, bottom]
            })
        })
        .collect();
    let bounds = [
        boxes.iter().map(|b| b[0]).fold(f64::INFINITY, f64::min),
        boxes.iter().map(|b| b[1]).fold(f64::INFINITY, f64::min),
        boxes.iter().map(|b| b[2]).fold(f64::NEG_INFINITY, f64::max),
        boxes.iter().map(|b| b[3]).fold(f64::NEG_INFINITY, f64::max),
    ];
    let word_gap = |index: usize| {
        index > 0
            && flags.get(index).is_some_and(|g| *g > 0.12)
            && !members[index].space
            && !members[index - 1].space
    };
    let mut words: Vec<f64> = (0..members.len())
        .filter(|&i| word_gap(i))
        .filter_map(|i| flags.get(i).copied())
        .collect();
    words.sort_by(f64::total_cmp);
    let space_width = if members.iter().any(|m| m.space) {
        lender
            .simple
            .as_ref()
            .and_then(|(_, w)| w.get(&0x20).copied())
    } else {
        None
    };
    let space = match space_width {
        Some(width) => width / 1000.0,
        None if !words.is_empty() => {
            let n = words.len();
            if n % 2 == 1 {
                words[(n - 1) / 2]
            } else {
                (words[n / 2 - 1] + words[n / 2]) / 2.0
            }
        }
        None => 0.28,
    };
    let repeat = |index: usize| index > 0 && members[index].space && members[index - 1].space;
    let mut text = String::new();
    let mut key = Vec::new();
    for (index, member) in members.iter().enumerate() {
        if repeat(index) {
            continue;
        }
        if word_gap(index) {
            text.push(' ');
        }
        text.push_str(records[member.index].text.as_deref().unwrap_or(""));
        key.push(format!(
            "{}{:x}",
            if word_gap(index) { "+" } else { "" },
            member.code
        ));
    }
    if super::js_len(&text) > MAX_EDIT_TEXT_CHARS {
        return Err(scan_error(
            "too-large",
            format!(
                "edit-targets: a text target is over the {MAX_EDIT_TEXT_CHARS} character ceiling"
            ),
        ));
    }
    let lend = &records[lender.index];
    let own: Option<String> = match drawn_chars {
        Some(chars) => {
            let mut sorted = chars.clone();
            sorted.sort_by_key(|(_, code)| *code);
            Some(sorted.into_iter().map(|(c, _)| c).collect())
        }
        None => lend.repertoire.clone(),
    };
    let repertoire = match own {
        Some(own) if own.contains(' ') => own,
        own => format!(" {}", own.unwrap_or_default()),
    };
    let first_record = &records[first.index];
    let mut hashed = b"GL|".to_vec();
    hashed.extend_from_slice(key.join(".").as_bytes());
    let id = target_id(
        scanner.page.index,
        &first_record.path,
        first.block.start,
        &hashed,
    );
    let mut record = Record {
        op: last.block.end..last.block.end,
        operand: first.block.start..last.block.end,
        candidate: true,
        text: Some(text),
        font: lend.font.clone(),
        bounds: Some(bounds),
        codec: lend.codec,
        text_linear: lend.text_linear,
        first_on_line: true,
        origin: first_record.origin,
        tf_name: lend.tf_name.clone(),
        tf_size: lend.tf_size,
        leading: lend.leading,
        font_size: lend.font_size,
        cid_key: lender
            .simple
            .as_ref()
            .map(|(object, _)| *object)
            .or(lend.cid_key),
        repertoire: Some(repertoire),
        cid_write: Some(write),
        cow: lend.cow.clone(),
        glyph_line: Some(Arc::new(GlyphLine {
            glyphs: members
                .iter()
                .filter(|m| !m.space)
                .map(|m| (m.code, m.block.x, m.adv))
                .collect(),
            y: first.block.y,
            scale: first.block.scale,
            colour: first.block.colour.clone(),
            space,
        })),
        glyphs: Some(members.iter().filter(|m| !m.space).count()),
        font_path: lend.font_path.clone(),
        ..first_record.clone()
    };
    record.target_id = id;
    record.kind = Kind::Text;
    record.reason = None;
    record.tail_follows = false;
    record.tail_target = None;
    record.actual_text = None;
    record.mcid = None;
    record.oc_depth = 0;
    record.horizontal_scale = 1.0;
    record.character_spacing = 0.0;
    record.word_spacing = 0.0;
    record.render_mode = 0.0;
    record.tj_adjustment = 0.0;
    record.tj_lead = 0.0;
    record.kerned = false;
    record.prev_show = None;
    record.between = None;
    record.payload = None;
    record.struct_actual_text = None;
    record.font_type0 = false;
    *lines += 1;
    out.push(Out::Line(Box::new(record)));
    Ok(())
}
