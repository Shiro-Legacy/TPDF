// SPDX-License-Identifier: MIT
//! What the content walk needs of a font to place each glyph a show
//! operator draws: how its bytes split into codes, each code's advance and
//! the glyph box's height. Advances come from the font dictionary (/Widths,
//! /W, Type 3 /Widths through /FontMatrix); a simple font without /Widths
//! is measured by the view (hayro draws it with the same advances). A font
//! that cannot be measured is `measured() == false`: its runs get one
//! conservative box, never per-glyph positions.
use std::collections::BTreeMap;
use std::sync::Arc;

use hayro_interpret::hayro_cmap::{self, CMap, CMapName, WritingMode};

use crate::app::annots;
use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId};

/// /W entries read from one descendant font at most.
const MAX_W_ENTRIES: usize = 65_536;
/// A CMap stream larger than this is not parsed (the run is unmeasured).
const MAX_CMAP_BYTES: usize = 4 << 20;

#[derive(Clone, Debug)]
pub enum Codes {
    /// One byte per code: simple and Type 3 fonts.
    Single,
    /// A Type 0 font's CMap: codes of one to four bytes.
    Cmap(Arc<CMap>),
    /// A Type 0 font whose CMap could not be read: code lengths unknown.
    Unknown,
}

/// The facts the walk places glyphs with.
#[derive(Clone, Debug)]
pub struct FontMetrics {
    pub object: Option<ObjectId>,
    pub codes: Codes,
    /// Simple and Type 3 fonts: advance in text space per unit size, by
    /// code; None where the font has no advance for it.
    simple: Option<Vec<Option<f64>>>,
    /// The simple font has no /Widths: the view measures it.
    pub needs_measure: bool,
    /// Type 0: advances by CID (1/1000 units), /DW, vertical metrics.
    cid_widths: Arc<BTreeMap<u32, f64>>,
    dw: f64,
    pub vertical: bool,
    /// /DW2's vertical advance (1/1000 units, negative downwards).
    dw2: f64,
    /// /W2 is present: vertical advances vary by CID (not read).
    w2: bool,
    /// Type 3: glyph space to text space.
    pub matrix: [f64; 6],
    pub type3: bool,
    /// The glyph box's top and bottom in text space per unit size.
    pub ascent: f64,
    pub descent: f64,
    /// An advance no code exceeds, per unit size (conservative boxes).
    pub max_advance: f64,
}

impl FontMetrics {
    /// The advance of `code` (`len` bytes long) per unit size, None when
    /// it is not known.
    pub fn advance(&self, code: u32, len: usize) -> Option<f64> {
        if let Some(widths) = &self.simple {
            return widths.get(code as usize).copied().flatten();
        }
        let Codes::Cmap(cmap) = &self.codes else {
            return None;
        };
        // A code the CMap does not map draws CID 0 (.notdef).
        let len = u8::try_from(len).ok()?;
        let cid = cmap.lookup_cid_code(code, len).unwrap_or(0);
        Some(self.cid_widths.get(&cid).copied().unwrap_or(self.dw) / 1000.0)
    }

    /// The vertical advance of a code per unit size (negative: downwards).
    pub fn vertical_advance(&self) -> Option<f64> {
        (!self.w2).then_some(self.dw2 / 1000.0)
    }

    /// Splits a string operand into codes, each with its byte range; None
    /// when the code lengths are not known.
    pub fn split(&self, bytes: &[u8]) -> Option<Vec<(u32, std::ops::Range<usize>)>> {
        match &self.codes {
            Codes::Single => Some(
                (bytes.iter().enumerate())
                    .map(|(i, &b)| (u32::from(b), i..i + 1))
                    .collect(),
            ),
            Codes::Unknown => None,
            Codes::Cmap(cmap) => {
                let mut out = Vec::new();
                let mut at = 0;
                while at < bytes.len() {
                    let (code, len) = read_code(cmap, bytes, at);
                    out.push((code, at..at + len));
                    at += len;
                }
                Some(out)
            }
        }
    }

    /// Glyph positions can be computed for this font's runs.
    pub fn measured(&self) -> bool {
        match &self.codes {
            Codes::Unknown => false,
            Codes::Single => self.simple.is_some() || self.needs_measure,
            Codes::Cmap(_) => !self.vertical || !self.w2,
        }
    }

    /// A font the walk could not read: every run gets a conservative box.
    pub fn unknown() -> Self {
        Self {
            object: None,
            codes: Codes::Unknown,
            simple: None,
            needs_measure: false,
            cid_widths: Arc::default(),
            dw: 1000.0,
            vertical: false,
            dw2: -1000.0,
            w2: false,
            matrix: [0.001, 0.0, 0.0, 0.001, 0.0, 0.0],
            type3: false,
            ascent: 1.0,
            descent: -0.5,
            max_advance: 1.0,
        }
    }
}

/// hayro's `read_code`: the shortest prefix the CMap maps, else one byte.
fn read_code(cmap: &CMap, bytes: &[u8], offset: usize) -> (u32, usize) {
    let mut code = 0u32;
    let remaining = bytes.len().saturating_sub(offset);
    for n in 0..remaining.min(4) {
        code = (code << 8) | u32::from(bytes.get(offset + n).copied().unwrap_or(0));
        if cmap.lookup_cid_code(code, (n + 1) as u8).is_some() {
            return (code, n + 1);
        }
    }
    (0, 1)
}

fn num(doc: &Document<'_>, value: Option<&Object>) -> Option<f64> {
    annots::number(doc, value?)
}

fn dict<'d>(doc: &'d Document<'_>, owner: &'d Dictionary, key: &[u8]) -> Option<&'d Dictionary> {
    doc.dict_get(owner, key).as_dict()
}

/// The font box's top and bottom per unit size: the descriptor's /Ascent
/// and /Descent and /FontBBox, each where it is plausible, never inside
/// the defaults (0.8 and -0.2), so the box holds what structured text
/// reports for the glyph.
fn extents(doc: &Document<'_>, descriptor: Option<&Dictionary>, scale: f64) -> (f64, f64) {
    let (mut ascent, mut descent) = (0.8f64, -0.2f64);
    let Some(descriptor) = descriptor else {
        return (ascent, descent);
    };
    let plausible_top = |v: f64| v > 0.0 && v <= 2.5;
    let plausible_bottom = |v: f64| (-1.5..0.0).contains(&v);
    if let Some(a) = num(doc, descriptor.get(b"Ascent".as_slice())).map(|v| v * scale) {
        if plausible_top(a) {
            ascent = ascent.max(a);
        }
    }
    if let Some(d) = num(doc, descriptor.get(b"Descent".as_slice())).map(|v| v * scale) {
        if plausible_bottom(d) {
            descent = descent.min(d);
        }
    }
    if let Some(bbox) = descriptor
        .get(b"FontBBox".as_slice())
        .and_then(|b| annots::rect(doc, b))
    {
        if plausible_top(bbox[3] * scale) {
            ascent = ascent.max(bbox[3] * scale);
        }
        if plausible_bottom(bbox[1] * scale) {
            descent = descent.min(bbox[1] * scale);
        }
    }
    (ascent, descent)
}

/// The font dictionary's metrics (with a cache of its own).
pub fn read(doc: &Document<'_>, font: &Dictionary, object: Option<ObjectId>) -> FontMetrics {
    FontCache::default().read(doc, font, object)
}

/// A CMap's identity: a predefined name or a stream.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum CmapKey {
    Name(Vec<u8>),
    Stream(ObjectId),
}

/// What fonts read in one walk share: CMaps by name or stream and /W
/// tables by descendant font, each read once, and the work reading took.
#[derive(Debug, Default)]
pub struct FontCache {
    cmaps: BTreeMap<CmapKey, Option<Arc<CMap>>>,
    /// /W by descendant font, with the largest width in it.
    widths: BTreeMap<ObjectId, (Arc<BTreeMap<u32, f64>>, f64)>,
    /// A unit per font read, plus its /Widths or /W entries and a unit per
    /// 8 CMap bytes parsed (each about a content token's work).
    pub work: usize,
}

impl FontCache {
    /// The font dictionary's metrics.
    pub fn read(
        &mut self,
        doc: &Document<'_>,
        font: &Dictionary,
        object: Option<ObjectId>,
    ) -> FontMetrics {
        let subtype = doc.dict_get(font, b"Subtype").as_name().unwrap_or(b"");
        self.work = self.work.saturating_add(1);
        if subtype != b"Type0" {
            let widths = doc
                .dict_get(font, b"Widths")
                .as_array()
                .map_or(0, <[_]>::len);
            self.work = self.work.saturating_add(widths.min(256));
        }
        match subtype {
            b"Type0" => self.type0(doc, font, object),
            b"Type3" => type3(doc, font, object),
            _ => simple(doc, font, object),
        }
    }

    fn cmap(&mut self, doc: &Document<'_>, encoding: &Object) -> Option<Arc<CMap>> {
        let (holder, value) = doc.resolve_with_holder(encoding);
        let key = match (value.as_name(), holder) {
            (Some(name), _) => CmapKey::Name(name.to_vec()),
            (None, Some(id)) => CmapKey::Stream(id),
            (None, None) => return None,
        };
        if let Some(cached) = self.cmaps.get(&key) {
            return cached.clone();
        }
        let cmap = read_cmap(doc, value, holder, &mut self.work).map(Arc::new);
        self.cmaps.insert(key, cmap.clone());
        cmap
    }

    fn type0(
        &mut self,
        doc: &Document<'_>,
        font: &Dictionary,
        object: Option<ObjectId>,
    ) -> FontMetrics {
        let (descendant_id, descendant) = (doc.dict_get(font, b"DescendantFonts").as_array())
            .and_then(|items| items.first())
            .map(|first| doc.resolve_with_holder(first))
            .map_or((None, None), |(id, value)| (id, value.as_dict()));
        let cmap = font
            .get(b"Encoding".as_slice())
            .and_then(|e| self.cmap(doc, e));
        let vertical = cmap
            .as_ref()
            .is_some_and(|c| c.metadata().writing_mode == Some(WritingMode::Vertical));
        let descriptor = descendant.and_then(|d| dict(doc, d, b"FontDescriptor"));
        let (ascent, descent) = extents(doc, descriptor, 0.001);
        let dw = descendant
            .and_then(|d| num(doc, d.get(b"DW".as_slice())))
            .unwrap_or(1000.0);
        let cached = descendant_id.and_then(|id| self.widths.get(&id).cloned());
        let (widths, widest) = cached.unwrap_or_else(|| {
            let read = Arc::new(cid_widths(
                doc,
                descendant.and_then(|d| doc.dict_get(d, b"W").as_array()),
            ));
            self.work = self.work.saturating_add(read.len());
            let widest = max_of(read.values().copied(), 0.0);
            if let Some(id) = descendant_id {
                self.widths.insert(id, (read.clone(), widest));
            }
            (read, widest)
        });
        let dw2 = descendant
            .and_then(|d| doc.dict_get(d, b"DW2").as_array())
            .and_then(|a| a.get(1))
            .and_then(|v| annots::number(doc, v))
            .unwrap_or(-1000.0);
        let w2 = descendant.is_some_and(|d| d.contains_key(b"W2".as_slice()));
        let max_advance = max_of([widest, dw].into_iter(), 0.0) / 1000.0;
        FontMetrics {
            object,
            codes: cmap.map_or(Codes::Unknown, Codes::Cmap),
            simple: None,
            needs_measure: false,
            cid_widths: widths,
            dw,
            vertical,
            dw2,
            w2,
            matrix: [0.001, 0.0, 0.0, 0.001, 0.0, 0.0],
            type3: false,
            ascent,
            descent,
            max_advance: max_advance.max(if vertical { dw2.abs() / 1000.0 } else { 0.0 }),
        }
    }
}

fn widths_array(doc: &Document<'_>, font: &Dictionary, scale: f64) -> Option<Vec<Option<f64>>> {
    let widths = doc.dict_get(font, b"Widths").as_array()?;
    let first = num(doc, font.get(b"FirstChar".as_slice())).unwrap_or(0.0);
    if !(0.0..=255.0).contains(&first) {
        return None;
    }
    let first = first as usize;
    let missing = dict(doc, font, b"FontDescriptor")
        .and_then(|d| num(doc, d.get(b"MissingWidth".as_slice())))
        .unwrap_or(0.0);
    let mut out = vec![Some(missing * scale); 256];
    for (i, width) in widths.iter().enumerate().take(256) {
        if let Some(slot) = out.get_mut(first + i) {
            *slot = Some(num(doc, Some(width)).unwrap_or(0.0) * scale);
        }
    }
    Some(out)
}

fn max_of(values: impl Iterator<Item = f64>, floor: f64) -> f64 {
    values
        .filter(|v| v.is_finite())
        .fold(floor, |m, v| m.max(v.abs()))
}

fn simple(doc: &Document<'_>, font: &Dictionary, object: Option<ObjectId>) -> FontMetrics {
    let descriptor = dict(doc, font, b"FontDescriptor");
    let (ascent, descent) = extents(doc, descriptor, 0.001);
    let simple = widths_array(doc, font, 0.001);
    let bbox_width = descriptor
        .and_then(|d| d.get(b"FontBBox".as_slice()))
        .and_then(|b| annots::rect(doc, b))
        .map_or(0.0, |r| (r[2] - r[0]) * 0.001);
    let max_advance = match &simple {
        Some(widths) => max_of(widths.iter().flatten().copied(), 0.0),
        None => max_of([bbox_width].into_iter(), 1.0).min(4.0),
    };
    FontMetrics {
        object,
        codes: Codes::Single,
        needs_measure: simple.is_none(),
        simple,
        cid_widths: Arc::default(),
        dw: 1000.0,
        vertical: false,
        dw2: -1000.0,
        w2: false,
        matrix: [0.001, 0.0, 0.0, 0.001, 0.0, 0.0],
        type3: false,
        ascent,
        descent,
        max_advance,
    }
}

fn type3(doc: &Document<'_>, font: &Dictionary, object: Option<ObjectId>) -> FontMetrics {
    let matrix = (doc.dict_get(font, b"FontMatrix").as_array())
        .filter(|m| m.len() == 6)
        .map(|m| {
            let n = |i: usize| m.get(i).and_then(|v| annots::number(doc, v)).unwrap_or(0.0);
            [n(0), n(1), n(2), n(3), n(4), n(5)]
        })
        .unwrap_or([0.001, 0.0, 0.0, 0.001, 0.0, 0.0]);
    // Widths are in glyph space; the advance is their image's x.
    let simple = widths_array(doc, font, 1.0).map(|w| {
        w.into_iter()
            .map(|v| v.map(|v| v * matrix[0]))
            .collect::<Vec<_>>()
    });
    let (mut ascent, mut descent) = (0.8, -0.2);
    if let Some(bbox) = font
        .get(b"FontBBox".as_slice())
        .and_then(|b| annots::rect(doc, b))
    {
        let r = annots::transform_rect(&matrix, bbox);
        if r[3] > r[1] && r[3] - r[1] <= 4.0 {
            ascent = r[3].max(0.0);
            descent = r[1].min(0.0);
        }
    }
    let max_advance = simple
        .as_ref()
        .map_or(1.0, |w| max_of(w.iter().flatten().copied(), 0.0));
    FontMetrics {
        object,
        codes: Codes::Single,
        needs_measure: false,
        simple,
        cid_widths: Arc::default(),
        dw: 1000.0,
        vertical: false,
        dw2: -1000.0,
        w2: false,
        matrix,
        type3: true,
        ascent,
        descent,
        max_advance,
    }
}

fn embedded(name: CMapName<'_>) -> Option<&'static [u8]> {
    hayro_cmap::load_embedded(name)
}

fn read_cmap(
    doc: &Document<'_>,
    value: &Object,
    holder: Option<ObjectId>,
    work: &mut usize,
) -> Option<CMap> {
    let charge = |work: &mut usize, bytes: usize| *work = work.saturating_add(bytes / 8);
    if let Some(name) = value.as_name() {
        return match CMapName::from_bytes(name) {
            CMapName::IdentityH => Some(CMap::identity_h()),
            CMapName::IdentityV => Some(CMap::identity_v()),
            other => {
                let data = embedded(other)?;
                charge(work, data.len());
                CMap::parse(data, embedded)
            }
        };
    }
    let limits = crate::stream::StreamLimits {
        max_decoded_bytes: MAX_CMAP_BYTES,
        max_total_decoded_bytes: MAX_CMAP_BYTES,
        ..crate::stream::StreamLimits::default()
    };
    let bytes = doc.decoded_stream(holder?, limits).ok()?;
    charge(work, bytes.len());
    CMap::parse(&bytes, embedded)
}

/// /W: `c [w ...]` and `c_first c_last w` entries, in 1/1000 units.
fn cid_widths(doc: &Document<'_>, w: Option<&[Object]>) -> BTreeMap<u32, f64> {
    let mut out = BTreeMap::new();
    let Some(items) = w else {
        return out;
    };
    let mut i = 0;
    let mut entries = 0usize;
    let int = |v: Option<&Object>| {
        num(doc, v)
            .filter(|v| (0.0..=f64::from(u32::MAX)).contains(v))
            .map(|v| v as u32)
    };
    while i < items.len() && entries < MAX_W_ENTRIES {
        let Some(first) = int(items.get(i)) else {
            break;
        };
        match items.get(i + 1).map(|v| doc.resolve(v)) {
            Some(Object::Array(list)) => {
                for (k, width) in list.iter().enumerate() {
                    if entries >= MAX_W_ENTRIES {
                        break;
                    }
                    let cid = first.saturating_add(k as u32);
                    out.insert(cid, num(doc, Some(width)).unwrap_or(0.0));
                    entries += 1;
                }
                i += 2;
            }
            Some(_) => {
                let (Some(last), Some(width)) = (int(items.get(i + 1)), num(doc, items.get(i + 2)))
                else {
                    break;
                };
                // A range stores one entry per CID up to the bound; the
                // rest of a huge range is left at /DW.
                let mut cid = first;
                while cid <= last && entries < MAX_W_ENTRIES {
                    out.insert(cid, width);
                    entries += 1;
                    match cid.checked_add(1) {
                        Some(next) => cid = next,
                        None => break,
                    }
                }
                i += 3;
            }
            None => break,
        }
    }
    out
}
