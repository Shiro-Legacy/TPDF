// SPDX-License-Identifier: AGPL-3.0-or-later
//! Edit-target records (content-edit.ts `EditTargetRecord`): one per text
//! show and per image or Form invocation, naming byte spans of the decoded
//! streams. A target id is the page, the stream path, the operator's offset
//! and a hash of its payload, so a later edit re-scans and finds the same
//! operator or refuses. Also the structured-text association, and the
//! passes after the walk (`walk`): growth limits and move rules.
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::sync::Arc;

use serde_json::{Map, Value, json};

use super::font::{Codec, FontSummary, at, dict_at, latin1, number};
use super::geometry::{invert, round2};
use super::glyph::GlyphLine;
use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId};
use crate::view::ViewDocument;

pub use super::walk::{PageInput, scan_page};

/// content-edit.ts `MAX_EDIT_EQUAL_MATCHES`.
pub const MAX_EDIT_EQUAL_MATCHES: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    Image,
    Form,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Image => "image",
            Self::Form => "form",
        }
    }
}

/// content-edit.ts `EditCow`: the privatisations an edit makes first.
#[derive(Clone, Debug, PartialEq)]
pub struct Cow {
    pub inherited_resources: bool,
    /// The /Contents element to copy because another page lists it.
    pub shared_contents: Option<usize>,
    pub form: Option<FormCow>,
}

/// A depth-1 Form invoked from a page element: copied, registered under a
/// fresh name, and this one `Do` renamed.
#[derive(Clone, Debug, PartialEq)]
pub struct FormCow {
    pub name: Vec<u8>,
    pub reference: ObjectId,
    pub element: usize,
    pub operand: Range<usize>,
}

/// One of the eight axis-aligned orientations of a placement `cm`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Orient {
    pub swap: bool,
    pub sx: f64,
    pub sy: f64,
}

impl Orient {
    pub const IDENTITY: Self = Self {
        swap: false,
        sx: 1.0,
        sy: 1.0,
    };
    pub fn json(self) -> Value {
        json!({"swap": self.swap, "sx": self.sx as i64, "sy": self.sy as i64})
    }
}

/// A placement clip: `start..end` removes it, `ops` are the four `re` operands.
#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    pub rect: [f64; 4],
    pub start: usize,
    pub end: usize,
    pub ops: Range<usize>,
}

/// content-edit.ts `EditPlacement`: a `q [clip] cm Do Q` block.
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    pub cm: Range<usize>,
    pub q_start: usize,
    pub q_end: usize,
    /// The full box in page points.
    pub full: [f64; 4],
    /// What the block's `cm` maps to `full`.
    pub rect: [f64; 4],
    /// The visible rect in view space (obstacles).
    pub view: [f64; 4],
    pub matrix: [f64; 6],
    pub orient: Orient,
    pub clip: Option<Clip>,
    /// `full` within the clip.
    pub visible: [f64; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImageFacts {
    pub width: i64,
    pub height: i64,
    pub smask: bool,
    pub mask: bool,
    pub decode: bool,
    pub interpolate: bool,
    pub oc: bool,
}

/// An `/ActualText` string operand: its span in the stream at `path`.
#[derive(Clone, Debug, PartialEq)]
pub struct ActualText {
    pub path: String,
    pub span: Range<usize>,
    pub bytes: Vec<u8>,
}

/// The write side of a proven code map: character to code, the advances.
#[derive(Clone, Debug, PartialEq)]
pub struct CidWrite {
    /// In code order (the repertoire's order).
    pub chars: Vec<(char, u32)>,
    pub widths: BTreeMap<u32, f64>,
    pub dw: f64,
    pub bytes_per_code: usize,
}

impl CidWrite {
    pub fn code(&self, c: char) -> Option<u32> {
        self.chars
            .iter()
            .find(|(d, _)| *d == c)
            .map(|(_, code)| *code)
    }
    pub fn width(&self, code: u32) -> f64 {
        self.widths.get(&code).copied().unwrap_or(self.dw)
    }
}

/// Where a run's font lives: the page's resources, or a Form's named from
/// the page by XObject name.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct FontPath {
    pub forms: Vec<Vec<u8>>,
}

/// content-edit.ts `EditTargetRecord`. Never built from a request.
#[derive(Clone, Debug)]
pub struct Record {
    pub target_id: String,
    pub kind: Kind,
    pub path: String,
    /// The /Contents element index when `path` is `c<i>`.
    pub element: Option<usize>,
    /// The stream holding the operator.
    pub stream: ObjectId,
    pub op: Range<usize>,
    pub operand: Range<usize>,
    pub depth: usize,
    pub candidate: bool,
    pub reason: Option<&'static str>,
    pub text: Option<String>,
    pub font: Option<FontSummary>,
    pub bounds: Option<[f64; 4]>,
    pub tail_follows: bool,
    pub tail_target: Option<String>,
    pub horizontal_scale: f64,
    pub character_spacing: f64,
    pub word_spacing: f64,
    pub tj_adjustment: f64,
    /// A TJ's numbers before its first non-empty string: where the first
    /// glyph starts past the origin, in thousandths of the size.
    pub tj_lead: f64,
    pub render_mode: f64,
    pub growth_limit: Option<f64>,
    pub obstacle_limit: Option<f64>,
    pub tail_right: Option<f64>,
    pub left_to_right_tail: Option<bool>,
    pub kerned: bool,
    /// The run's simple-font codec (None without a resolved font).
    pub codec: Option<Codec>,
    pub font_type0: bool,
    pub actual_text: Option<ActualText>,
    pub mcid: Option<i64>,
    pub oc_depth: usize,
    pub payload: Option<Vec<u8>>,
    pub struct_actual_text: Option<String>,
    pub cow: Option<Cow>,
    /// Linear part of Tm x CTM x page transform at the show.
    pub text_linear: Option<[f64; 4]>,
    pub first_on_line: bool,
    pub origin: Option<[f64; 2]>,
    pub tf_name: Option<Vec<u8>>,
    pub tf_size: Option<f64>,
    pub leading: Option<f64>,
    pub prev_show: Option<String>,
    pub between: Option<[f64; 2]>,
    pub font_size: Option<f64>,
    pub cid_key: Option<ObjectId>,
    pub repertoire: Option<String>,
    pub cid_write: Option<Arc<CidWrite>>,
    pub placement: Option<Placement>,
    pub object_ops: Option<Vec<&'static str>>,
    pub object_name: Option<Vec<u8>>,
    pub image: Option<ImageFacts>,
    pub glyph_line: Option<Arc<GlyphLine>>,
    /// The object an image or Form invocation draws.
    pub object: Option<ObjectId>,
    pub font_path: FontPath,
    // Reply-only facts filled after the walk.
    pub moveable: Option<bool>,
    pub move_reason: Option<&'static str>,
    pub reply_font_size: Option<f64>,
    pub paragraph_id: Option<String>,
    pub glyphs: Option<usize>,
}

impl Record {
    pub(super) fn new(
        target_id: String,
        kind: Kind,
        path: &str,
        stream: ObjectId,
        depth: usize,
    ) -> Self {
        Self {
            target_id,
            kind,
            element: element_index(path),
            path: path.to_string(),
            stream,
            op: 0..0,
            operand: 0..0,
            depth,
            candidate: false,
            reason: None,
            text: None,
            font: None,
            bounds: None,
            tail_follows: false,
            tail_target: None,
            horizontal_scale: 1.0,
            character_spacing: 0.0,
            word_spacing: 0.0,
            tj_adjustment: 0.0,
            tj_lead: 0.0,
            render_mode: 0.0,
            growth_limit: None,
            obstacle_limit: None,
            tail_right: None,
            left_to_right_tail: None,
            kerned: false,
            codec: None,
            font_type0: false,
            actual_text: None,
            mcid: None,
            oc_depth: 0,
            payload: None,
            struct_actual_text: None,
            cow: None,
            text_linear: None,
            first_on_line: false,
            origin: None,
            tf_name: None,
            tf_size: None,
            leading: None,
            prev_show: None,
            between: None,
            font_size: None,
            cid_key: None,
            repertoire: None,
            cid_write: None,
            placement: None,
            object_ops: None,
            object_name: None,
            image: None,
            glyph_line: None,
            object: None,
            font_path: FontPath::default(),
            moveable: None,
            move_reason: None,
            reply_font_size: None,
            paragraph_id: None,
            glyphs: None,
        }
    }

    pub fn is_text(&self) -> bool {
        self.kind == Kind::Text
    }

    /// The reply's `EditTarget`.
    pub fn item(&self) -> Value {
        let mut item = Map::new();
        item.insert("targetId".into(), self.target_id.clone().into());
        item.insert("kind".into(), self.kind.as_str().into());
        item.insert(
            "bounds".into(),
            self.bounds.map_or(Value::Null, |b| json!(b)),
        );
        let support = if self.candidate {
            "candidate"
        } else {
            "unsupported"
        };
        item.insert("support".into(), support.into());
        if self.is_text() && self.glyph_line.is_none() {
            item.insert("tailFollows".into(), self.tail_follows.into());
            if self.kerned {
                item.insert("kerned".into(), true.into());
            }
        } else if self.glyph_line.is_some() {
            item.insert("tailFollows".into(), false.into());
        }
        if self.cow.is_some() {
            item.insert("cowable".into(), true.into());
        }
        if let Some(reason) = self.reason {
            item.insert("reason".into(), reason.into());
        }
        if let Some(text) = &self.text {
            item.insert("text".into(), text.clone().into());
        }
        if let Some(font) = &self.font {
            item.insert("font".into(), font.json());
        }
        if let Some(placement) = &self.placement {
            let mut entry = Map::new();
            entry.insert("box".into(), json!(placement.visible));
            entry.insert("orient".into(), placement.orient.json());
            if placement.clip.is_some() {
                entry.insert("full".into(), json!(placement.full));
            }
            item.insert("placement".into(), Value::Object(entry));
        }
        if let Some(ops) = &self.object_ops {
            item.insert("objectOps".into(), json!(ops));
        }
        if let Some(name) = &self.object_name {
            item.insert("objectName".into(), latin1(name).into());
        }
        if let Some(image) = &self.image {
            item.insert(
                "image".into(),
                json!({"width": image.width, "height": image.height, "smask": image.smask,
                    "mask": image.mask, "decode": image.decode, "interpolate": image.interpolate,
                    "oc": image.oc}),
            );
        }
        if let Some(repertoire) = &self.repertoire {
            item.insert("repertoire".into(), repertoire.clone().into());
        }
        if let Some(glyphs) = self.glyphs {
            item.insert("glyphs".into(), glyphs.into());
        }
        if let Some(size) = self.reply_font_size {
            item.insert("fontSize".into(), size.into());
        }
        if let Some(moveable) = self.moveable {
            item.insert("moveable".into(), moveable.into());
        }
        if let Some(reason) = self.move_reason {
            item.insert("moveReason".into(), reason.into());
        }
        if let Some(id) = &self.paragraph_id {
            item.insert("paragraphId".into(), id.clone().into());
        }
        Value::Object(item)
    }
}

/// `^c(\d+)$`.
pub fn element_index(path: &str) -> Option<usize> {
    let digits = path.strip_prefix('c')?;
    (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
        .then(|| digits.parse().ok())
        .flatten()
}

/// content-edit.ts `editHash`: FNV-1a over bytes, 8 hex digits.
pub fn edit_hash(bytes: impl IntoIterator<Item = u8>) -> String {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in bytes {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    format!("{hash:08x}")
}

/// `editHash` of a JavaScript string: each UTF-16 unit's low byte.
pub fn edit_hash_text(text: &str) -> String {
    edit_hash(text.encode_utf16().map(|unit| (unit & 0xff) as u8))
}

/// content-edit.ts `editTargetId`; `payload` is the hashed bytes.
pub fn target_id(page: usize, path: &str, op_start: usize, payload: &[u8]) -> String {
    format!(
        "p{page}.{path}.{op_start}.{}",
        edit_hash(payload.iter().copied())
    )
}

/// One structured-text line, for association and bounds.
#[derive(Clone, Debug)]
pub struct Run {
    pub chars: Vec<RunChar>,
    pub font_name: Option<String>,
    /// The line's text, each character's byte offset in it, and the text
    /// trimmed as JavaScript trims.
    text: String,
    offsets: Vec<usize>,
    trimmed: String,
    /// Holds a character outside the BMP (two UTF-16 units, one quad).
    astral: bool,
}

#[derive(Clone, Debug)]
pub struct RunChar {
    pub c: char,
    pub size: f64,
    pub quad: [f64; 8],
    pub origin: [f64; 2],
}

/// Single precision, as MuPDF's structured text reports coordinates.
fn f32_of(value: f64) -> f64 {
    f64::from(value as f32)
}

impl Run {
    pub fn new(chars: Vec<RunChar>, font_name: Option<String>) -> Self {
        let text: String = chars.iter().map(|c| c.c).collect();
        let offsets = text.char_indices().map(|(i, _)| i).collect();
        let trimmed = js_trim(&text).to_string();
        let astral = chars.iter().any(|c| u32::from(c.c) > 0xffff);
        Self {
            chars,
            font_name,
            text,
            offsets,
            trimmed,
            astral,
        }
    }

    /// `indexOf(needle, from)` in characters.
    fn index_of(&self, needle: &str, from: usize) -> Option<usize> {
        let start = *self.offsets.get(from)?;
        let found = start + self.text.get(start..)?.find(needle)?;
        self.offsets.binary_search(&found).ok()
    }
}

/// The preserve-spans lines of the page as the view extracts them; a page
/// whose text cannot be read has none.
pub fn runs_of(view: &ViewDocument, page: usize) -> Vec<Run> {
    let Ok(text) = view.structured_text(page) else {
        return Vec::new();
    };
    let fonts = text.fonts();
    let mut runs = Vec::new();
    for block in text.span_blocks() {
        for line in &block.lines {
            if line.chars.is_empty() {
                continue;
            }
            let font_name = line
                .chars
                .first()
                .and_then(|c| fonts.get(c.font))
                .map(|f| f.name.clone());
            let chars = line
                .chars
                .iter()
                .map(|c| RunChar {
                    c: c.c,
                    size: if c.size.is_finite() { c.size } else { 0.0 },
                    quad: c.quad.map(f32_of),
                    origin: c.origin.map(f32_of),
                })
                .collect();
            runs.push(Run::new(chars, font_name));
        }
    }
    runs
}

/// content-edit.ts `editSameFont`: names compared without the subset
/// prefix, an axis suffix or a font-file extension, ignoring case.
pub fn same_font(left: &str, right: &str) -> bool {
    fn normalise(name: &str) -> String {
        let bytes = name.as_bytes();
        let name = if super::font::has_subset_prefix(bytes) {
            &name[7..]
        } else {
            name
        };
        let name = name.split('[').next().unwrap_or("");
        let lower = name.to_ascii_lowercase();
        for ext in [".ttf", ".otf", ".ttc", ".pfb", ".pfa"] {
            if let Some(stripped) = lower.strip_suffix(ext) {
                return stripped.to_lowercase();
            }
        }
        name.to_lowercase()
    }
    normalise(left) == normalise(right)
}

/// content-edit.ts `editUnionQuads`.
pub fn union_quads<'q>(quads: impl IntoIterator<Item = &'q [f64; 8]>) -> Option<[f64; 4]> {
    let mut out = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for quad in quads {
        for pair in quad.chunks_exact(2) {
            let (x, y) = (pair[0], pair[1]);
            if !x.is_finite() || !y.is_finite() {
                continue;
            }
            out = [out[0].min(x), out[1].min(y), out[2].max(x), out[3].max(y)];
        }
    }
    (out[0].is_finite() && out[1].is_finite()).then_some(out)
}

/// content-edit.ts `editMatchRuns`: the one occurrence of `text` in the
/// page's lines drawn in `font` (the one at `origin` when given), its
/// bounds and characters; None for none or an ambiguous several.
pub fn match_runs<'r>(
    runs: &'r [Run],
    text: &str,
    font: Option<&str>,
    any_of_equal: bool,
    kerned: bool,
    origin: Option<[f64; 2]>,
    depth: usize,
) -> Option<(Option<[f64; 4]>, Vec<&'r RunChar>)> {
    let needle: Vec<char> = text.chars().collect();
    let needle_trim: String = js_trim(text).to_string();
    let limit = if any_of_equal {
        MAX_EDIT_EQUAL_MATCHES + 1
    } else {
        2
    };
    let mut matches: Vec<(Vec<&RunChar>, Option<&str>)> = Vec::new();
    let mut near: Vec<Vec<&RunChar>> = Vec::new();
    let mut whole_lines = 0;
    let mut near_whole = false;
    let mut overflow = false;
    for run in runs {
        // One quad per UTF-16 unit only holds without astral characters.
        if run.astral {
            continue;
        }
        if let (Some(base), Some(name)) = (font, run.font_name.as_deref()) {
            if !same_font(name, base) {
                continue;
            }
        }
        let whole = run.trimmed == needle_trim;
        let mut add =
            |chars: Vec<&'r RunChar>, matches: &mut Vec<(Vec<&'r RunChar>, Option<&'r str>)>| {
                if whole {
                    whole_lines += 1;
                }
                if let (Some(origin), Some(first)) = (origin, chars.first()) {
                    let tolerance = (first.size / 2.0).max(1.0);
                    let distance = (first.origin[0] - origin[0]).hypot(first.origin[1] - origin[1]);
                    if distance <= tolerance {
                        near.push(chars.clone());
                        near_whole = whole;
                    }
                }
                if matches.len() < limit {
                    matches.push((chars, run.font_name.as_deref()));
                }
            };
        let hay = |i: usize| run.chars.get(i).map(|c| c.c);
        let mut from = 0;
        loop {
            let found = if needle.is_empty() {
                (from <= run.chars.len()).then_some(from)
            } else {
                run.index_of(text, from)
            };
            if let Some(found) = found {
                if run.chars.len() >= found + needle.len() {
                    add(
                        run.chars[found..found + needle.len()].iter().collect(),
                        &mut matches,
                    );
                    from = found + 1;
                    if matches.len() >= limit && origin.is_none() {
                        break;
                    }
                    continue;
                }
            }
            if !kerned || !matches.is_empty() {
                break;
            }
            let mut chars: Vec<&RunChar> = Vec::new();
            let mut cursor = 0;
            for &character in &needle {
                if character != ' ' {
                    while hay(cursor) == Some(' ') {
                        cursor += 1;
                    }
                }
                match (hay(cursor), run.chars.get(cursor)) {
                    (Some(c), Some(ch)) if c == character => {
                        chars.push(ch);
                        cursor += 1;
                    }
                    _ => {
                        chars.clear();
                        break;
                    }
                }
            }
            if chars.len() == needle.len() && !chars.is_empty() {
                add(chars, &mut matches);
            }
            break;
        }
        if matches.len() >= limit {
            overflow = true;
            origin?;
        }
    }
    if near.len() == 1 && (depth == 0 || whole_lines == usize::from(near_whole)) {
        let chars = near.swap_remove(0);
        return Some((union_quads(chars.iter().map(|c| &c.quad)), chars));
    }
    if overflow || matches.is_empty() {
        return None;
    }
    if matches.len() == 1 {
        let (chars, _) = matches.swap_remove(0);
        return Some((union_quads(chars.iter().map(|c| &c.quad)), chars));
    }
    if !any_of_equal {
        return None;
    }
    let (first_chars, first_font) = &matches[0];
    let size = first_chars.first().map(|c| c.size);
    if !matches
        .iter()
        .all(|(chars, name)| name == first_font && chars.first().map(|c| c.size) == size)
    {
        return None;
    }
    Some((None, first_chars.clone()))
}

/// JavaScript `String.prototype.trim`.
pub fn js_trim(text: &str) -> &str {
    text.trim_matches(|c: char| {
        matches!(c, '\u{9}'..='\u{D}' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'..='\u{200A}'
            | '\u{2028}' | '\u{2029}' | '\u{202F}' | '\u{205F}' | '\u{3000}' | '\u{FEFF}')
    })
}

/// content-edit.ts `editIsRtlOrShaping`.
pub fn is_rtl_or_shaping(text: &str) -> bool {
    text.chars().any(|c| {
        matches!(u32::from(c), 0x0300..=0x036f | 0x0590..=0x08ff | 0x1ab0..=0x1aff
            | 0x1dc0..=0x1dff | 0x20d0..=0x20ff | 0xfb1d..=0xfdff | 0xfe20..=0xfe2f
            | 0xfe70..=0xfeff)
    })
}

/// Number-tree nodes one lookup may visit.
const MAX_NUMBER_TREE_NODES: usize = 4096;

/// The page's ParentTree entry (its MCID array), through /StructParents.
pub fn parent_entry<'d>(doc: &'d Document<'_>, page: &'d Dictionary) -> Option<&'d [Object]> {
    let key = number(at(doc, Some(page), b"StructParents"))?.trunc() as i64;
    let catalog = doc.resolve_id(doc.root()).as_dict();
    let tree = dict_at(doc, catalog, b"StructTreeRoot")?;
    let node = dict_at(doc, Some(tree), b"ParentTree")?;
    let value = number_tree(doc, node, key)?;
    doc.resolve(value).as_array()
}

/// The StructElem `mcid` names in a ParentTree entry, and its holder.
pub fn struct_element_in<'d>(
    doc: &'d Document<'_>,
    entry: &'d [Object],
    mcid: i64,
) -> Option<(Option<ObjectId>, &'d Dictionary)> {
    let entry = entry.get(usize::try_from(mcid).ok()?)?;
    let (holder, value) = doc.resolve_with_holder(entry);
    Some((holder, value.as_dict()?))
}

/// The StructElem a page MCID maps to through /StructParents and the
/// ParentTree number tree (/Nums, then /Kids within /Limits).
pub fn struct_element<'d>(
    doc: &'d Document<'_>,
    page: &'d Dictionary,
    mcid: i64,
) -> Option<(Option<ObjectId>, &'d Dictionary)> {
    struct_element_in(doc, parent_entry(doc, page)?, mcid)
}

/// Depth first in /Kids order, each node once, at most
/// `MAX_NUMBER_TREE_NODES` of them; a kid whose /Limits exclude `key` is
/// not entered.
fn number_tree<'d>(doc: &'d Document<'_>, root: &'d Dictionary, key: i64) -> Option<&'d Object> {
    let mut seen = BTreeSet::new();
    let mut stack = vec![(root, 0usize)];
    let mut queued = 1usize;
    while let Some((node, depth)) = stack.pop() {
        if let Some(nums) = at(doc, Some(node), b"Nums").and_then(Object::as_array) {
            let found = nums.chunks_exact(2).find_map(|pair| {
                let k = number(Some(doc.resolve(pair.first()?)))?;
                (k.trunc() as i64 == key).then_some(pair.get(1)?)
            });
            if found.is_some() {
                return found;
            }
            continue;
        }
        if depth > 32 {
            continue;
        }
        let Some(kids) = at(doc, Some(node), b"Kids").and_then(Object::as_array) else {
            continue;
        };
        let mut next = Vec::new();
        for kid in kids {
            if let Some(id) = kid.as_reference() {
                if !seen.insert(id) {
                    continue;
                }
            }
            let Some(kid) = doc.resolve(kid).as_dict() else {
                continue;
            };
            if let Some(limits) = at(doc, Some(kid), b"Limits").and_then(Object::as_array) {
                let low = limits.first().and_then(|v| number(Some(doc.resolve(v))));
                let high = limits.get(1).and_then(|v| number(Some(doc.resolve(v))));
                if let (Some(low), Some(high)) = (low, high) {
                    if (key as f64) < low || (key as f64) > high {
                        continue;
                    }
                }
            }
            queued += 1;
            if queued > MAX_NUMBER_TREE_NODES {
                return None;
            }
            next.push((kid, depth + 1));
        }
        stack.extend(next.into_iter().rev());
    }
    None
}

/// A finished scan of one page.
#[derive(Clone, Debug, Default)]
pub struct PageScan {
    pub page: usize,
    /// Records in item (reply) order.
    pub records: Vec<Record>,
    /// Record indices in engine.ts's record-map order (a collapsed glyph
    /// line after the others).
    pub map_order: Vec<usize>,
    pub paragraphs: Vec<super::paragraph::Paragraph>,
    pub balanced: bool,
    pub transform: [f64; 6],
    /// The scan's decoded bytes, which a copy-on-write copy adds to.
    pub budget_bytes: usize,
    /// The decoded bytes of every stream the scan walked, by object; the
    /// records' spans index these, and a writer splices them.
    pub streams: BTreeMap<ObjectId, Arc<Vec<u8>>>,
}

impl PageScan {
    pub fn record(&self, id: &str) -> Option<&Record> {
        self.records.iter().find(|r| r.target_id == id)
    }
    pub fn index(&self, id: &str) -> Option<usize> {
        self.records.iter().position(|r| r.target_id == id)
    }
    pub fn paragraph(&self, id: &str) -> Option<&super::paragraph::Paragraph> {
        self.paragraphs.iter().find(|p| p.id == id)
    }
    pub fn items(&self) -> Vec<Value> {
        self.records.iter().map(Record::item).collect()
    }
}

/// content-edit.ts `editAttachGrowthLimits`: how far a run may grow.
pub(super) fn attach_growth_limits(
    records: &mut [Record],
    geometry_blocked: bool,
    page_right: f64,
) {
    let text: Vec<usize> = (0..records.len())
        .filter(|&i| records[i].is_text() && records[i].bounds.is_some())
        .collect();
    let right = text
        .iter()
        .filter_map(|&i| records[i].bounds.map(|b| b[2]))
        .fold(0.0f64, f64::max);
    let box_right = if geometry_blocked { right } else { page_right };
    let overlaps = |a: &[f64; 4], b: &[f64; 4]| a[1] < b[3] && b[1] < a[3];
    let mut updates = Vec::new();
    for &i in &text {
        let record = &records[i];
        let Some(bounds) = record.bounds else {
            continue;
        };
        let mut tail_ids: BTreeSet<&str> = BTreeSet::new();
        let mut tail_right = bounds[2];
        let mut left_to_right = true;
        let mut cursor = record;
        while let Some(next_id) = cursor.tail_target.as_deref() {
            let Some(next) = records.iter().find(|r| r.target_id == next_id) else {
                break;
            };
            let Some(next_bounds) = next.bounds else {
                break;
            };
            if !tail_ids.insert(next.target_id.as_str()) {
                break;
            }
            if next_bounds[0] + 0.5 < bounds[2] {
                left_to_right = false;
            }
            tail_right = tail_right.max(next_bounds[2]);
            cursor = next;
        }
        let mut lefts = Vec::new();
        for (j, other) in records.iter().enumerate() {
            if j == i || tail_ids.contains(other.target_id.as_str()) {
                continue;
            }
            let other_box = if other.is_text() {
                other.bounds
            } else {
                other.placement.as_ref().map(|p| p.view)
            };
            let Some(other_box) = other_box else { continue };
            if !overlaps(&bounds, &other_box) || other_box[0] < tail_right - 0.5 {
                continue;
            }
            lefts.push(other_box[0]);
        }
        let obstacle = lefts.iter().copied().reduce(f64::min).map(|l| l - 1.0);
        let ours = record.tf_name.as_deref().is_some_and(is_pdf4all_font);
        let growth = obstacle.unwrap_or(if ours { box_right } else { right });
        updates.push((i, tail_right, left_to_right, growth, obstacle));
    }
    for (i, tail_right, left_to_right, growth, obstacle) in updates {
        let record = &mut records[i];
        record.tail_right = Some(tail_right);
        record.left_to_right_tail = Some(left_to_right);
        record.growth_limit = Some(growth);
        record.obstacle_limit = obstacle;
    }
}

/// `^PDF4AllF\d*$`: a font name text-add writes.
pub fn is_pdf4all_font(name: &[u8]) -> bool {
    name.strip_prefix(b"PDF4AllF")
        .is_some_and(|rest| rest.iter().all(u8::is_ascii_digit))
}

/// content-edit.ts `editTextScale`: the page-pt height of one Tf unit.
pub fn text_scale(record: &Record) -> Option<f64> {
    let linear = record.text_linear?;
    invert(&linear)?;
    record.tf_name.as_ref()?;
    let size = record.tf_size?;
    if size == 0.0 || !size.is_finite() {
        return None;
    }
    Some(linear[2].hypot(linear[3]))
}

/// content-edit.ts `editMoveRefusal`.
pub fn move_refusal(record: &Record) -> Option<(&'static str, &'static str)> {
    if !record.candidate {
        return Some(("not-candidate", record.reason.unwrap_or("not-candidate")));
    }
    if text_scale(record).is_none() {
        return Some(("not-candidate", "matrix"));
    }
    if !record.first_on_line {
        return Some(("not-candidate", "tail"));
    }
    if record.tail_follows {
        return Some(("tail-would-move", "tail-would-move"));
    }
    None
}

/// content-edit.ts `editAttachGeometry`.
pub(super) fn attach_geometry(records: &mut [Record]) {
    for record in records.iter_mut().filter(|r| r.is_text()) {
        if let (Some(scale), Some(size)) = (text_scale(record), record.tf_size) {
            record.reply_font_size = Some(round2(size.abs() * scale));
        }
        let refusal = move_refusal(record);
        record.moveable = Some(refusal.is_none());
        if let (Some((_, reason)), true) = (refusal, record.candidate) {
            record.move_reason = Some(reason);
        }
    }
}
