// SPDX-License-Identifier: AGPL-3.0-or-later
//! AcroForm fields as engine.ts reads them through MuPDF: the widgets of a
//! page in /Annots order, field attributes inherited along /Parent, the
//! page transform between /Rect and the y-down page space the app speaks,
//! and the facts the `fields` listing reports. Appearance synthesis is in
//! `appearance`, its writing in `write`, and the view's synthesized
//! appearances (NeedAppearances, a widget without /AP) in `view`.
pub mod appearance;
pub mod scripts;
pub mod text;
pub mod view;
pub mod write;

use std::collections::{BTreeMap, BTreeSet};

use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId, decode_text_string};

use super::info;
use super::pagetree;

/// Field flag bits (ISO 32000-2 Tables 227, 229, 231, 233).
pub const READ_ONLY: i64 = 1;
pub const REQUIRED: i64 = 1 << 1;
pub const MULTILINE: i64 = 1 << 12;
pub const PASSWORD: i64 = 1 << 13;
pub const RADIO: i64 = 1 << 15;
pub const PUSHBUTTON: i64 = 1 << 16;
pub const COMBO: i64 = 1 << 17;
pub const EDIT: i64 = 1 << 18;
pub const COMB: i64 = 1 << 24;

/// How many /Parent links the engine.ts walks follow (`MAX_FIELD_DEPTH`).
pub const MAX_FIELD_DEPTH: usize = 8;
/// A field name chain longer than this is read as if it ended there.
const MAX_NAME_DEPTH: usize = 64;
/// Characters of a full name or label read at most; a longer one is cut.
pub const MAX_NAME_CHARS: usize = 4096;

static NULL: Object = Object::Null;

/// MuPDF's widget types (`pdf_field_type`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WidgetType {
    PushButton,
    Checkbox,
    Radio,
    Text,
    Combo,
    List,
    Signature,
}

/// The kinds the `fields` listing reports (engine.ts `fieldKind`); push
/// buttons are not listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    Checkbox,
    Radio,
    Combo,
    List,
    Signature,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Checkbox => "checkbox",
            Self::Radio => "radio",
            Self::Combo => "combo",
            Self::List => "list",
            Self::Signature => "signature",
        }
    }

    pub fn of(widget_type: WidgetType) -> Option<Self> {
        match widget_type {
            WidgetType::PushButton => None,
            WidgetType::Checkbox => Some(Self::Checkbox),
            WidgetType::Radio => Some(Self::Radio),
            WidgetType::Text => Some(Self::Text),
            WidgetType::Combo => Some(Self::Combo),
            WidgetType::List => Some(Self::List),
            WidgetType::Signature => Some(Self::Signature),
        }
    }

    pub fn is_button(self) -> bool {
        matches!(self, Self::Checkbox | Self::Radio)
    }
}

/// One widget annotation of a page.
#[derive(Clone, Copy, Debug)]
pub struct Widget<'d> {
    /// The widget's own object; None for a dictionary written into /Annots.
    pub id: Option<ObjectId>,
    pub dict: &'d Dictionary,
}

impl Widget<'_> {
    /// engine.ts `annotId`: the object number, 0 for a direct dictionary.
    pub fn number(&self) -> u32 {
        self.id.map_or(0, |id| id.number())
    }
}

/// The page's widgets in /Annots order, each object once (MuPDF lists every
/// /Subtype /Widget dictionary of /Annots, whether or not /AcroForm lists
/// its field).
pub fn page_widgets<'d>(doc: &'d Document<'_>, page: &'d Dictionary) -> Vec<Widget<'d>> {
    let Some(annots) = doc.dict_get(page, b"Annots").as_array() else {
        return Vec::new();
    };
    let mut seen = BTreeSet::new();
    let mut widgets = Vec::new();
    for item in annots {
        let (holder, value) = doc.resolve_with_holder(item);
        let id = match item {
            Object::Reference(_) => match holder {
                Some(id) => Some(id),
                None => continue,
            },
            _ => None,
        };
        let Some(dict) = value.as_dict() else {
            continue;
        };
        if doc.dict_get(dict, b"Subtype").as_name() != Some(b"Widget") {
            continue;
        }
        if id.is_some_and(|id| !seen.insert(id)) {
            continue;
        }
        widgets.push(Widget { id, dict });
    }
    widgets
}

/// MuPDF's `pdf_dict_get_inheritable` on a field, resolved; null when absent.
pub fn inherited<'d>(doc: &'d Document<'_>, field: &'d Dictionary, key: &[u8]) -> &'d Object {
    pagetree::inherited(doc, field, key).map_or(&NULL, |value| doc.resolve(value))
}

/// MuPDF's `pdf_to_int`: an integer wraps to 32 bits, a real truncates.
pub fn to_int(value: &Object) -> i64 {
    match value {
        Object::Integer(value) => i64::from(*value as i32),
        Object::Real(value) if value.is_finite() => i64::from(*value as i32),
        _ => 0,
    }
}

/// The inherited /Ff (`pdf_field_flags`).
pub fn field_flags(doc: &Document<'_>, field: &Dictionary) -> i64 {
    to_int(inherited(doc, field, b"Ff"))
}

/// MuPDF's `pdf_field_type` from the inherited /FT and /Ff; anything
/// unknown is a push button.
pub fn widget_type(doc: &Document<'_>, field: &Dictionary) -> WidgetType {
    let flags = field_flags(doc, field);
    match inherited(doc, field, b"FT").as_name() {
        Some(b"Btn") if flags & PUSHBUTTON != 0 => WidgetType::PushButton,
        Some(b"Btn") if flags & RADIO != 0 => WidgetType::Radio,
        Some(b"Btn") => WidgetType::Checkbox,
        Some(b"Tx") => WidgetType::Text,
        Some(b"Ch") if flags & COMBO != 0 => WidgetType::Combo,
        Some(b"Ch") => WidgetType::List,
        Some(b"Sig") => WidgetType::Signature,
        _ => WidgetType::PushButton,
    }
}

/// engine.ts `fieldKind`.
pub fn kind(doc: &Document<'_>, field: &Dictionary) -> Option<Kind> {
    Kind::of(widget_type(doc, field))
}

/// A text string value as MuPDF's `pdf_to_text_string` reads it: a string,
/// decoded; anything else is empty.
pub fn text_of(value: &Object) -> String {
    value
        .as_string()
        .map(decode_text_string)
        .unwrap_or_default()
}

/// A name's bytes as the JavaScript string mupdf.js makes of them.
pub fn name_text(name: &[u8]) -> String {
    String::from_utf8_lossy(name).into_owned()
}

/// `text_of` cut at `max` characters, decoding only the bytes they can
/// take (4 at most each, after a byte order mark), so a long string costs
/// no more than a short one.
pub fn text_prefix(value: &Object, max: usize) -> String {
    let Some(bytes) = value.as_string() else {
        return String::new();
    };
    let limit = max.saturating_add(1).saturating_mul(4).saturating_add(3);
    cut_chars(decode_text_string(bytes.get(..limit).unwrap_or(bytes)), max)
}

/// `text` with at most `max` characters.
pub fn cut_chars(mut text: String, max: usize) -> String {
    if let Some((at, _)) = text.char_indices().nth(max) {
        text.truncate(at);
    }
    text
}

/// The field nodes from `field` up its /Parent chain, `limit` at most,
/// stopping at a repeat.
pub fn lineage<'d>(
    doc: &'d Document<'_>,
    id: Option<ObjectId>,
    field: &'d Dictionary,
    limit: usize,
) -> Vec<(Option<ObjectId>, &'d Dictionary)> {
    let mut chain = vec![(id, field)];
    let mut seen: BTreeSet<ObjectId> = id.into_iter().collect();
    let mut node = field;
    while chain.len() < limit {
        let Some(parent) = node.get(b"Parent".as_slice()) else {
            break;
        };
        let (holder, value) = doc.resolve_with_holder(parent);
        let Some(dict) = value.as_dict() else {
            break;
        };
        if holder.is_some_and(|holder| !seen.insert(holder)) {
            break;
        }
        chain.push((holder, dict));
        node = dict;
    }
    chain
}

/// MuPDF's `pdf_load_field_name` (`getName`): the /T of every node from the
/// top down, empty ones skipped, joined with dots; empty on a cycle. Cut at
/// `MAX_NAME_CHARS`.
pub fn full_name(doc: &Document<'_>, id: Option<ObjectId>, field: &Dictionary) -> String {
    let chain = lineage(doc, id, field, MAX_NAME_DEPTH + 1);
    let cyclic = chain.last().is_some_and(|(_, node)| {
        let parent = node.get(b"Parent".as_slice());
        parent.is_some_and(|parent| doc.resolve(parent).as_dict().is_some())
    });
    if cyclic {
        return String::new();
    }
    join_names(doc, &chain)
}

/// engine.ts `fullFieldName`: the same over eight nodes at most.
pub fn short_full_name(doc: &Document<'_>, id: Option<ObjectId>, field: &Dictionary) -> String {
    join_names(doc, &lineage(doc, id, field, MAX_FIELD_DEPTH))
}

/// The /T parts joined, cut at `MAX_NAME_CHARS`.
fn join_names(doc: &Document<'_>, chain: &[(Option<ObjectId>, &Dictionary)]) -> String {
    let parts: Vec<String> = (chain.iter().rev())
        .map(|(_, node)| text_prefix(doc.dict_get(node, b"T"), MAX_NAME_CHARS))
        .filter(|part| !part.is_empty())
        .collect();
    cut_chars(parts.join("."), MAX_NAME_CHARS)
}

/// MuPDF's `pdf_field_label`: the inherited /TU, else the inherited /T,
/// else "Unnamed"; a value that is not a string reads as empty. Cut at
/// `MAX_NAME_CHARS`.
pub fn label(doc: &Document<'_>, field: &Dictionary) -> String {
    let tu = inherited(doc, field, b"TU");
    let label = if tu.is_null() {
        inherited(doc, field, b"T")
    } else {
        tu
    };
    if label.is_null() {
        return "Unnamed".into();
    }
    text_prefix(label, MAX_NAME_CHARS)
}

/// MuPDF's `pdf_field_value` (`getValue`): the inherited /V as a name or a
/// text string; a stream (rich text) reads as its text.
pub fn value_text(doc: &Document<'_>, field: &Dictionary) -> String {
    let Some(value) = pagetree::inherited(doc, field, b"V") else {
        return String::new();
    };
    let stream = |id: ObjectId| doc.object(id).is_ok_and(|object| object.stream.is_some());
    match doc.resolve_with_holder(value) {
        (_, Object::Name(name)) => name_text(name),
        (Some(id), Object::Dictionary(_)) if stream(id) => stream_text(doc, id),
        (_, value) => text_of(value),
    }
}

/// `value_text` cut at `max` characters, reading no more than they take.
pub fn value_prefix(doc: &Document<'_>, field: &Dictionary, max: usize) -> String {
    let Some(value) = pagetree::inherited(doc, field, b"V") else {
        return String::new();
    };
    let stream = |id: ObjectId| doc.object(id).is_ok_and(|object| object.stream.is_some());
    match doc.resolve_with_holder(value) {
        (_, Object::Name(name)) => {
            let limit = max.saturating_mul(4);
            cut_chars(name_text(name.get(..limit).unwrap_or(name)), max)
        }
        (Some(id), Object::Dictionary(_)) if stream(id) => cut_chars(stream_text(doc, id), max),
        (_, value) => text_prefix(value, max),
    }
}

/// A rich-text /V stream as text, bounded.
fn stream_text(doc: &Document<'_>, id: ObjectId) -> String {
    let limits = crate::stream::StreamLimits {
        max_decoded_bytes: 1 << 20,
        max_total_decoded_bytes: 4 << 20,
        ..crate::stream::StreamLimits::default()
    };
    match doc.decoded_stream(id, limits) {
        Ok(bytes) => decode_text_string(&bytes),
        Err(_) => String::new(),
    }
}

/// The inherited /MaxLen (`getMaxLen`).
pub fn max_len(doc: &Document<'_>, field: &Dictionary) -> i64 {
    to_int(inherited(doc, field, b"MaxLen"))
}

/// One /Opt entry: its export value and display text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub export: String,
    pub display: String,
}

/// The inherited /Opt entries (MuPDF `pdf_choice_field_option`): a pair
/// gives its export and display strings, anything else is both.
pub fn choices(doc: &Document<'_>, field: &Dictionary) -> Vec<Choice> {
    let Some(options) = inherited(doc, field, b"Opt").as_array() else {
        return Vec::new();
    };
    (options.iter())
        .map(|option| match doc.resolve(option) {
            Object::Array(pair) if pair.len() == 2 => Choice {
                export: pair
                    .first()
                    .map_or_else(String::new, |v| text_of(doc.resolve(v))),
                display: pair
                    .get(1)
                    .map_or_else(String::new, |v| text_of(doc.resolve(v))),
            },
            value => {
                let text = text_of(value);
                Choice {
                    export: text.clone(),
                    display: text,
                }
            }
        })
        .collect()
}

/// `take` /Opt entries from the `skip`th, their strings cut at `max`
/// characters.
pub fn choices_range(
    doc: &Document<'_>,
    field: &Dictionary,
    skip: usize,
    take: usize,
    max: usize,
) -> Vec<Choice> {
    let Some(options) = inherited(doc, field, b"Opt").as_array() else {
        return Vec::new();
    };
    let text = |value: &Object| text_prefix(doc.resolve(value), max);
    (options.iter().skip(skip).take(take))
        .map(|option| match doc.resolve(option) {
            Object::Array(pair) if pair.len() == 2 => Choice {
                export: pair.first().map_or_else(String::new, text),
                display: pair.get(1).map_or_else(String::new, text),
            },
            value => {
                let text = text_prefix(value, max);
                Choice {
                    export: text.clone(),
                    display: text,
                }
            }
        })
        .collect()
}

/// engine.ts `fieldOptions`: the export values, else the display strings.
pub fn options(doc: &Document<'_>, field: &Dictionary) -> Vec<String> {
    choices(doc, field)
        .into_iter()
        .map(|choice| choice.export)
        .collect()
}

/// engine.ts `fieldOn`: the first key of the widget's own /AP /N that is
/// not Off, empty when none.
pub fn on_state(doc: &Document<'_>, widget: &Dictionary) -> Vec<u8> {
    let normal = doc.dict_get(
        doc.dict_get(widget, b"AP").as_dict().unwrap_or(&EMPTY),
        b"N",
    );
    normal
        .as_dict()
        .and_then(|states| states.keys().find(|key| key.as_slice() != b"Off"))
        .cloned()
        .unwrap_or_default()
}

static EMPTY: Dictionary = Dictionary::new();

/// engine.ts `fieldValueName`: the first /V that is a name, from the widget
/// up eight nodes.
pub fn value_name(doc: &Document<'_>, id: Option<ObjectId>, widget: &Dictionary) -> Vec<u8> {
    for (_, node) in lineage(doc, id, widget, MAX_FIELD_DEPTH) {
        if let Some(name) = doc.dict_get(node, b"V").as_name() {
            return name.to_vec();
        }
    }
    Vec::new()
}

/// engine.ts `buttonChecked`: the widget's own /AS when it is a name, else
/// the field's /V name.
pub fn button_checked(doc: &Document<'_>, id: Option<ObjectId>, widget: &Dictionary) -> bool {
    if let Some(state) = doc.dict_get(widget, b"AS").as_name() {
        return state != b"Off";
    }
    let value = value_name(doc, id, widget);
    !value.is_empty() && value != b"Off"
}

/// engine.ts `fieldOwnDict`: a radio kid's /Parent, otherwise the widget.
pub fn own_dict<'d>(
    doc: &'d Document<'_>,
    kind: Kind,
    id: Option<ObjectId>,
    widget: &'d Dictionary,
) -> (Option<ObjectId>, &'d Dictionary) {
    if kind == Kind::Radio
        && let Some(parent) = widget.get(b"Parent".as_slice())
    {
        let (holder, value) = doc.resolve_with_holder(parent);
        if let Some(parent) = value.as_dict() {
            return (holder, parent);
        }
    }
    (id, widget)
}

/// MuPDF's `find_head_of_field_group`: the nearest node from the widget up
/// that has /T (the field itself), or the widget when none has.
pub fn field_head<'d>(
    doc: &'d Document<'_>,
    id: Option<ObjectId>,
    widget: &'d Dictionary,
) -> (Option<ObjectId>, &'d Dictionary) {
    let chain = lineage(doc, id, widget, MAX_NAME_DEPTH);
    (chain.iter())
        .find(|(_, node)| {
            node.get(b"T".as_slice())
                .is_some_and(|t| !doc.resolve(t).is_null())
        })
        .copied()
        .unwrap_or((id, widget))
}

/// The widget annotations at and below a field node, each object once,
/// bounded by `limit` nodes visited.
pub fn field_widgets(doc: &Document<'_>, id: Option<ObjectId>, limit: usize) -> Vec<ObjectId> {
    let mut found = Vec::new();
    let Some(id) = id else {
        return found;
    };
    let mut seen = BTreeSet::from([id]);
    let mut stack = vec![(id, 0usize)];
    let mut visited = 0usize;
    while let Some((node, depth)) = stack.pop() {
        visited += 1;
        if visited > limit {
            break;
        }
        let Some(dict) = doc.resolve_id(node).as_dict() else {
            continue;
        };
        let kids = doc.dict_get(dict, b"Kids").as_array();
        if doc.dict_get(dict, b"Subtype").as_name() == Some(b"Widget") {
            found.push(node);
        }
        if depth >= MAX_NAME_DEPTH {
            continue;
        }
        for kid in kids.unwrap_or(&[]).iter().rev() {
            if let Object::Reference(kid) = kid
                && seen.insert(*kid)
            {
                stack.push((*kid, depth + 1));
            }
        }
    }
    found
}

/// The page transform: PDF user space to the app's page space (points, y
/// down, origin at the top left of the cropped, rotated page), as MuPDF's
/// `pdf_page_transform` computes it: in single precision, so `bounds` is
/// exactly the page size `info::page_size` reports and a listed rect sent
/// back unchanged is on the page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageSpace {
    ctm: [f32; 6],
    width: f32,
    height: f32,
}

impl PageSpace {
    pub fn of(doc: &Document<'_>, page: &Dictionary) -> Self {
        // As `info::page_size` reads the boxes, /UserUnit and /Rotate.
        let letter = [0.0, 0.0, 612.0, 792.0];
        let empty = |r: &[f64; 4]| !(r[0] < r[2] && r[1] < r[3]);
        let mut area = info::page_box(doc, page, b"MediaBox").unwrap_or([0.0; 4]);
        if let Some(crop) = info::page_box(doc, page, b"CropBox") {
            let x0 = area[0].max(crop[0]);
            let y0 = area[1].max(crop[1]);
            area = [
                x0,
                y0,
                area[2].min(crop[2]).max(x0),
                area[3].min(crop[3]).max(y0),
            ];
        }
        if empty(&area) {
            area = letter;
        } else if area[2] - area[0] < 1.0 || area[3] - area[1] < 1.0 {
            area = [0.0, 0.0, 1.0, 1.0];
        }
        let single = |value: &Object| match doc.resolve(value) {
            Object::Integer(v) => Some(*v as f32),
            Object::Real(v) => Some(*v as f32),
            _ => None,
        };
        let u = (page.get(b"UserUnit".as_slice()))
            .and_then(single)
            .filter(|u| u.is_finite() && *u > 0.0)
            .unwrap_or(1.0);
        let mut node = page;
        let mut rotate = None;
        for _ in 0..64 {
            rotate = node.get(b"Rotate".as_slice()).and_then(single);
            if rotate.is_some() {
                break;
            }
            match doc.dict_get(node, b"Parent").as_dict() {
                Some(parent) => node = parent,
                None => break,
            }
        }
        let mut rotate = rotate.map_or(0, |r| (r + 0.5).floor() as i32);
        if rotate < 0 {
            rotate = 360 - (rotate.unsigned_abs() % 360) as i32;
        }
        rotate = 90 * ((rotate % 360 + 45) / 90);
        if rotate >= 360 {
            rotate = 0;
        }
        let [x0, y0, x1, y1] = area.map(|v| v as f32);
        let (w, h) = (x1 * u - x0 * u, y1 * u - y0 * u);
        let (ctm, width, height) = match rotate {
            90 => ([0., u, u, 0., -(y0 * u), -(x0 * u)], h, w),
            180 => ([-u, 0., 0., u, x1 * u, -(y0 * u)], w, h),
            270 => ([0., -u, -u, 0., y1 * u, x1 * u], h, w),
            _ => ([u, 0., 0., -u, -(x0 * u), y1 * u], w, h),
        };
        Self { ctm, width, height }
    }

    /// The bound page `[0, 0, width, height]` (MuPDF `getBounds`).
    pub fn bounds(&self) -> [f64; 4] {
        [0.0, 0.0, f64::from(self.width), f64::from(self.height)]
    }

    /// A PDF rectangle in page space, normalized (MuPDF `pdf_annot_rect`).
    pub fn to_page(&self, rect: [f64; 4]) -> [f64; 4] {
        transform_rect(self.ctm, rect.map(|v| v as f32)).map(f64::from)
    }

    /// A page-space rectangle in PDF user space (MuPDF `pdf_set_annot_rect`
    /// with `fz_invert_matrix`), each number the shortest decimal of its
    /// single-precision value.
    pub fn to_pdf(&self, rect: [f64; 4]) -> [f64; 4] {
        let rect = rect.map(|v| v as f32);
        let [a, b, c, d, e, f] = self.ctm;
        let det = a * d - b * c;
        let out = if det.abs() > f32::EPSILON && det.is_finite() {
            let r = 1.0 / det;
            let (ia, ib, ic, id) = (d * r, -b * r, -c * r, a * r);
            let ie = -e * ia - f * ic;
            let iff = -e * ib - f * id;
            transform_rect([ia, ib, ic, id, ie, iff], rect)
        } else {
            transform_rect(self.ctm, rect)
        };
        out.map(shortest)
    }
}

/// An `f32` as the `f64` of its shortest decimal (`742.2`, not
/// `742.2000122070312`), which reads back as the same `f32`.
fn shortest(value: f32) -> f64 {
    if !value.is_finite() {
        return 0.0;
    }
    format!("{value}").parse().unwrap_or(0.0)
}

/// MuPDF's `fz_transform_rect` in single precision: every corner, then
/// their bounds.
fn transform_rect(m: [f32; 6], rect: [f32; 4]) -> [f32; 4] {
    let [a, b, c, d, e, f] = m;
    let corners = [
        (rect[0], rect[1]),
        (rect[2], rect[1]),
        (rect[0], rect[3]),
        (rect[2], rect[3]),
    ];
    let points = corners.map(|(x, y)| (x * a + y * c + e, x * b + y * d + f));
    let xs = points.map(|p| p.0);
    let ys = points.map(|p| p.1);
    let min = |v: [f32; 4]| v.into_iter().fold(f32::INFINITY, f32::min);
    let max = |v: [f32; 4]| v.into_iter().fold(f32::NEG_INFINITY, f32::max);
    // -0 from a rotation reads as 0.
    [min(xs) + 0.0, min(ys) + 0.0, max(xs) + 0.0, max(ys) + 0.0]
}

/// MuPDF's `pdf_to_rect` of an annotation's /Rect: four numbers, normalized;
/// anything else is the empty rectangle at the origin.
pub fn pdf_rect(doc: &Document<'_>, annot: &Dictionary) -> [f64; 4] {
    let Some(items) = doc.dict_get(annot, b"Rect").as_array() else {
        return [0.0; 4];
    };
    let mut v = [0.0f64; 4];
    for (slot, item) in v.iter_mut().zip(items) {
        *slot = doc
            .resolve(item)
            .as_f64()
            .filter(|n| n.is_finite())
            .unwrap_or(0.0);
    }
    [
        v[0].min(v[2]),
        v[1].min(v[3]),
        v[0].max(v[2]),
        v[1].max(v[3]),
    ]
}

/// A rectangle as PDF numbers: integers where exact, reals otherwise.
pub fn rect_object(rect: [f64; 4]) -> Object {
    Object::Array(rect.iter().map(|&v| number(v)).collect())
}

/// A number as a PDF integer when it is one exactly, else a real.
pub fn number(value: f64) -> Object {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        Object::Integer(value as i64)
    } else if value.is_finite() {
        Object::Real(value)
    } else {
        Object::Integer(0)
    }
}

/// Document-wide facts the `scripted` flag needs (engine.ts `fieldContext`).
#[derive(Clone, Debug, Default)]
pub struct ScriptContext {
    doc_scripts: bool,
    co: BTreeSet<u32>,
}

impl ScriptContext {
    pub fn of(doc: &Document<'_>) -> Self {
        fn get<'d>(doc: &'d Document<'_>, dict: Option<&'d Dictionary>, key: &[u8]) -> &'d Object {
            dict.map_or(&NULL, |dict| doc.dict_get(dict, key))
        }
        let catalog = doc.resolve_id(doc.root()).as_dict();
        let acro = get(doc, catalog, b"AcroForm").as_dict();
        let mut co = BTreeSet::new();
        if let Some(items) = get(doc, acro, b"CO").as_array() {
            for item in items {
                co.insert(item.as_reference().map_or(0, |id| id.number()));
            }
        }
        let names = get(doc, catalog, b"Names").as_dict();
        let doc_scripts = !get(doc, names, b"JavaScript").is_null();
        Self { doc_scripts, co }
    }

    /// engine.ts `widgetScripted`.
    pub fn scripted(&self, doc: &Document<'_>, id: Option<ObjectId>, widget: &Dictionary) -> bool {
        if self.doc_scripts {
            return true;
        }
        lineage(doc, id, widget, MAX_FIELD_DEPTH)
            .iter()
            .any(|(id, node)| {
                self.co.contains(&id.map_or(0, |id| id.number()))
                    || has_javascript(doc, doc.dict_get(node, b"AA"))
            })
    }
}

fn has_javascript(doc: &Document<'_>, aa: &Object) -> bool {
    aa.as_dict().is_some_and(|actions| {
        (actions.values()).any(|action| {
            let action = doc.resolve(action).as_dict();
            action.is_some_and(|a| doc.dict_get(a, b"S").as_name() == Some(b"JavaScript"))
        })
    })
}

/// engine.ts `pageTabs`: the page's own /Tabs when it is R, C, S, A or W.
pub fn page_tabs(doc: &Document<'_>, page: &Dictionary) -> Option<&'static str> {
    match page.get(b"Tabs".as_slice()).map(|value| doc.resolve(value)) {
        Some(Object::Name(name)) => match name.as_slice() {
            b"R" => Some("R"),
            b"C" => Some("C"),
            b"S" => Some("S"),
            b"A" => Some("A"),
            b"W" => Some("W"),
            _ => None,
        },
        _ => None,
    }
}

/// engine.ts `structObjectOrder`: object numbers of /OBJR /Obj entries in a
/// depth-first walk of /StructTreeRoot /K (first appearance wins), or None
/// past 50,000 nodes or 64 dictionary levels.
pub fn struct_order(doc: &Document<'_>) -> Option<BTreeMap<u32, usize>> {
    let catalog = doc.resolve_id(doc.root()).as_dict()?;
    let tree = doc.dict_get(catalog, b"StructTreeRoot").as_dict()?;
    let kids = tree
        .get(b"K".as_slice())
        .filter(|k| !doc.resolve(k).is_null())?;
    let mut order = BTreeMap::new();
    let mut visited = BTreeSet::new();
    let mut nodes = 0usize;
    // An explicit stack in the order the recursive walk visits.
    let mut stack: Vec<(&Object, usize)> = vec![(kids, 0)];
    while let Some((node, depth)) = stack.pop() {
        nodes += 1;
        if nodes > 50_000 || depth > 64 {
            return None;
        }
        if let Object::Reference(id) = node
            && !visited.insert(*id)
        {
            continue;
        }
        match doc.resolve(node) {
            Object::Array(items) => stack.extend(items.iter().rev().map(|item| (item, depth))),
            Object::Dictionary(dict) => {
                if let Some(Object::Reference(obj)) = dict.get(b"Obj".as_slice()) {
                    let next = order.len();
                    order.entry(obj.number()).or_insert(next);
                    continue;
                }
                if let Some(next) = dict.get(b"K".as_slice())
                    && !doc.resolve(next).is_null()
                {
                    stack.push((next, depth + 1));
                }
            }
            _ => {}
        }
    }
    Some(order)
}
