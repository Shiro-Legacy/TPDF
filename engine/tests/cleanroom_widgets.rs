// SPDX-License-Identifier: AGPL-3.0-or-later
//! Acceptance for spec 07 (form widget appearances, variable text and
//! fallback fonts), section 9.3 and the owner decisions W1-W5. Every case
//! is black-box: a one-widget form built here, its appearance synthesized
//! through `Context`, stored through `appearance_dict`, and judged by what
//! the engine's own content parser, renderer and text extraction see when
//! that appearance is drawn onto a fresh page the way a viewer draws it
//! (ISO 32000-2 §12.5.5: the /BBox, transformed by /Matrix, mapped onto
//! /Rect). The crate-internal layout and encoding functions are judged
//! through the appearances they make. Expected values come from the spec,
//! ISO 32000-2, the Adobe Glyph List and BCP 47.
use std::cell::OnceCell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::{Duration, Instant};

use tandem_pdf_engine::app::forms::appearance::{Context, MAX_DRAWN_CHARS};
use tandem_pdf_engine::app::forms::scripts::{Eight, han_font};
use tandem_pdf_engine::app::forms::text::Family;
use tandem_pdf_engine::app::forms::write::{Sink, appearance_dict};
use tandem_pdf_engine::content::{ContentLimits, ContentOperand, ContentParser, Operation};
use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::object::{Dictionary, Object, ObjectId};
use tandem_pdf_engine::serialize::{WriteLimits, serialize_object};
use tandem_pdf_engine::simple_encoding::{GlyphUnicode, NamedEncoding, SimpleEncoding};
use tandem_pdf_engine::view::{RenderOptions, RgbaImage, ViewDocument};

// ------------------------------------------------------------------ builders

fn id(n: u32) -> ObjectId {
    ObjectId::new(n, 0).unwrap()
}

/// A classic-xref PDF from object bodies numbered from 1.
fn classic(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        bytes.extend_from_slice(body);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    bytes
}

fn stream(dictionary: &[u8], data: &[u8]) -> Vec<u8> {
    let mut out = b"<< ".to_vec();
    out.extend_from_slice(dictionary);
    out.extend_from_slice(format!(" /Length {} >>\nstream\n", data.len()).as_bytes());
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream");
    out
}

/// A text string as UTF-16BE hex with its byte order mark.
fn text_string(text: &str) -> String {
    let hex: String = text
        .encode_utf16()
        .map(|unit| format!("{unit:04X}"))
        .collect();
    format!("<FEFF{hex}>")
}

/// A one-page form: the widget (object 6) with the entries `widget` and,
/// when given, `value` as its /V; /AcroForm with `acro`, /DA
/// `/Helv 12 Tf 0 g` and /DR fonts Helv, TiRo, Cour and ZaDb (7-10).
fn widget_pdf(widget: &str, value: Option<&str>, acro: &str) -> Vec<u8> {
    let value = value.map(|v| format!(" /V {}", text_string(v)));
    let value = value.unwrap_or_default();
    let objects: Vec<Vec<u8>> = [
        "<< /Type /Catalog /Pages 2 0 R /AcroForm 5 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Annots [6 0 R] >>"
            .into(),
        String::from_utf8(stream(b"", b"q Q")).unwrap(),
        format!(
            "<< /Fields [6 0 R] /DA (/Helv 12 Tf 0 g) /DR << /Font << /Helv 7 0 R /TiRo 8 0 R \
             /Cour 9 0 R /ZaDb 10 0 R >> >> {acro} >>"
        ),
        format!("<< /Type /Annot /Subtype /Widget /F 4 /P 3 0 R {widget}{value} >>"),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".into(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman /Encoding /WinAnsiEncoding >>"
            .into(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Courier /Encoding /WinAnsiEncoding >>".into(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /ZapfDingbats >>".into(),
    ]
    .into_iter()
    .map(String::into_bytes)
    .collect();
    classic(&objects)
}

// ------------------------------------------------------------------ storing

/// An object an appearance needs: its value and, for a stream, its decoded
/// data.
type Loaded = (Object, Option<Vec<u8>>);

/// The objects `appearance_dict` writes, stream data kept.
#[derive(Default)]
struct Collect {
    objects: BTreeMap<ObjectId, Loaded>,
    fonts: BTreeMap<Vec<u8>, ObjectId>,
}

impl Collect {
    fn put(&mut self, value: Object, data: Option<Vec<u8>>) -> Result<ObjectId, ()> {
        let id = id(10_000 + self.objects.len() as u32);
        self.objects.insert(id, (value, data));
        Ok(id)
    }
}

impl Sink for Collect {
    type Error = ();
    fn stream(&mut self, dictionary: Dictionary, content: Vec<u8>) -> Result<ObjectId, ()> {
        self.put(Object::Dictionary(dictionary), Some(content))
    }
    fn object(&mut self, value: Object) -> Result<ObjectId, ()> {
        self.put(value, None)
    }
    fn fonts(&mut self) -> &mut BTreeMap<Vec<u8>, ObjectId> {
        &mut self.fonts
    }
}

/// Object `id` from the sink, else from the document.
fn load(doc: &Document<'_>, sink: &Collect, id: ObjectId) -> Option<Loaded> {
    if let Some(loaded) = sink.objects.get(&id) {
        return Some(loaded.clone());
    }
    let object = doc.object(id).ok()?;
    let data = doc.decoded_stream(id, Default::default()).ok();
    Some((object.value.clone(), data))
}

fn references(value: &Object, out: &mut Vec<ObjectId>) {
    match value {
        Object::Reference(id) => out.push(*id),
        Object::Array(items) => items.iter().for_each(|item| references(item, out)),
        Object::Dictionary(dict) => dict.values().for_each(|item| references(item, out)),
        _ => {}
    }
}

/// Every object `root` reaches.
fn gather(load: &dyn Fn(ObjectId) -> Option<Loaded>, root: ObjectId) -> BTreeMap<ObjectId, Loaded> {
    let mut out = BTreeMap::new();
    let mut queue = VecDeque::from([root]);
    while let Some(next) = queue.pop_front() {
        if out.contains_key(&next) {
            continue;
        }
        let Some(loaded) = load(next) else {
            continue;
        };
        let mut found = Vec::new();
        references(&loaded.0, &mut found);
        queue.extend(found);
        out.insert(next, loaded);
    }
    out
}

fn renumber(value: &mut Object, map: &BTreeMap<ObjectId, u32>) {
    match value {
        Object::Reference(old) => {
            *value = map
                .get(old)
                .map_or(Object::Null, |n| Object::Reference(id(*n)))
        }
        Object::Array(items) => items.iter_mut().for_each(|item| renumber(item, map)),
        Object::Dictionary(dict) => dict.values_mut().for_each(|item| renumber(item, map)),
        _ => {}
    }
}

fn numbers(value: Option<&Object>) -> Option<Vec<f64>> {
    value?.as_array()?.iter().map(Object::as_f64).collect()
}

/// A one-page PDF that draws the form `root` (with the `objects` it
/// reaches) as a viewer draws an appearance: its /BBox, transformed by its
/// /Matrix, mapped onto `rect` (ISO 32000-2 §12.5.5).
fn appearance_page(
    objects: &BTreeMap<ObjectId, Loaded>,
    root: ObjectId,
    rect: [f64; 4],
) -> Vec<u8> {
    let mut map = BTreeMap::new();
    map.insert(root, 5);
    for old in objects.keys().filter(|old| **old != root) {
        let next = 5 + map.len() as u32;
        map.insert(*old, next);
    }
    let mut bodies: Vec<(u32, Vec<u8>)> = Vec::new();
    for (old, (value, data)) in objects {
        let mut value = value.clone();
        renumber(&mut value, &map);
        let body = match (data, &mut value) {
            (Some(data), Object::Dictionary(dict)) => {
                dict.remove(b"Filter".as_slice());
                dict.remove(b"DecodeParms".as_slice());
                dict.insert(b"Length".to_vec(), Object::Integer(data.len() as i64));
                let mut body = serialize_object(&value, WriteLimits::default()).unwrap();
                body.extend_from_slice(b"\nstream\n");
                body.extend_from_slice(data);
                body.extend_from_slice(b"\nendstream");
                body
            }
            _ => serialize_object(&value, WriteLimits::default()).unwrap(),
        };
        bodies.push((map[old], body));
    }
    bodies.sort();
    let form = objects[&root].0.as_dict().unwrap();
    let bbox = numbers(form.get(b"BBox".as_slice())).unwrap();
    let m = numbers(form.get(b"Matrix".as_slice())).unwrap_or(vec![1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    let corners = [
        (bbox[0], bbox[1]),
        (bbox[2], bbox[1]),
        (bbox[0], bbox[3]),
        (bbox[2], bbox[3]),
    ]
    .map(|(x, y)| (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]));
    let xs = corners.map(|c| c.0);
    let ys = corners.map(|c| c.1);
    let lo = |v: [f64; 4]| v.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = |v: [f64; 4]| v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let scale = |from: f64, to: f64| if from.abs() < 1e-9 { 1.0 } else { to / from };
    let sx = scale(hi(xs) - lo(xs), rect[2] - rect[0]);
    let sy = scale(hi(ys) - lo(ys), rect[3] - rect[1]);
    let (e, f) = (rect[0] - lo(xs) * sx, rect[1] - lo(ys) * sy);
    let content = format!("q {sx} 0 0 {sy} {e} {f} cm /A Do Q");
    let mut out: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
          /Resources << /XObject << /A 5 0 R >> >> >>"
            .to_vec(),
        stream(b"", content.as_bytes()),
    ];
    out.extend(bodies.into_iter().map(|(_, body)| body));
    classic(&out)
}

// ------------------------------------------------------------------ drawing

const PAGE_HEIGHT: f64 = 792.0;
/// Pixels per point of every render.
const SCALE: f64 = 2.0;

/// An extracted character, in page space with y up.
#[derive(Clone, Debug)]
struct Char {
    c: char,
    origin: [f64; 2],
    /// x0, y0, x1, y1.
    bbox: [f64; 4],
}

/// What the engine's view makes of a page.
struct Drawn {
    text: String,
    lines: Vec<Vec<Char>>,
    image: RgbaImage,
}

fn draw(page: Vec<u8>) -> Drawn {
    let view = ViewDocument::open(page, "").unwrap();
    let stext = view.structured_text(0).unwrap();
    let mut lines = Vec::new();
    for block in stext.blocks() {
        for line in &block.lines {
            let chars = line.chars.iter().map(|ch| {
                let xs = [ch.quad[0], ch.quad[2], ch.quad[4], ch.quad[6]];
                let ys = [ch.quad[1], ch.quad[3], ch.quad[5], ch.quad[7]].map(|y| PAGE_HEIGHT - y);
                let lo = |v: [f64; 4]| v.iter().copied().fold(f64::INFINITY, f64::min);
                let hi = |v: [f64; 4]| v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                Char {
                    c: ch.c,
                    origin: [ch.origin[0], PAGE_HEIGHT - ch.origin[1]],
                    bbox: [lo(xs), lo(ys), hi(xs), hi(ys)],
                }
            });
            lines.push(chars.collect());
        }
    }
    let image = view
        .render(0, SCALE, RenderOptions { annotations: false })
        .unwrap();
    Drawn {
        text: stext.as_text(),
        lines,
        image,
    }
}

impl Drawn {
    /// The extracted text without whitespace.
    fn compact(&self) -> String {
        self.text.chars().filter(|c| !c.is_whitespace()).collect()
    }
    /// Each extracted line's text, trimmed, empty lines left out.
    fn line_texts(&self) -> Vec<String> {
        self.lines
            .iter()
            .map(|line| {
                line.iter()
                    .map(|ch| ch.c)
                    .collect::<String>()
                    .trim()
                    .to_string()
            })
            .filter(|line| !line.is_empty())
            .collect()
    }
    fn chars(&self) -> impl Iterator<Item = &Char> {
        self.lines.iter().flatten()
    }
    fn pixel(&self, x: u32, y: u32) -> [u8; 3] {
        let at = ((y * self.image.width + x) * 4) as usize;
        [
            self.image.pixels[at],
            self.image.pixels[at + 1],
            self.image.pixels[at + 2],
        ]
    }
    /// The pixel at page point (x, y), y up.
    fn at(&self, x: f64, y: f64) -> [u8; 3] {
        let px = ((x * SCALE) as u32).min(self.image.width - 1);
        let py = (((PAGE_HEIGHT - y) * SCALE) as u32).min(self.image.height - 1);
        self.pixel(px, py)
    }
    /// Pixel bounds (x0, y0, x1, y1, rows down) of a page rect, y up.
    fn pixels_of(&self, r: [f64; 4]) -> [u32; 4] {
        let clamp_x = |v: f64| (v.max(0.0) as u32).min(self.image.width);
        let clamp_y = |v: f64| (v.max(0.0) as u32).min(self.image.height);
        [
            clamp_x((r[0] * SCALE).floor()),
            clamp_y(((PAGE_HEIGHT - r[3]) * SCALE).floor()),
            clamp_x((r[2] * SCALE).ceil()),
            clamp_y(((PAGE_HEIGHT - r[1]) * SCALE).ceil()),
        ]
    }
    /// Pixels painted anything but white inside the page rect `r`.
    fn ink(&self, r: [f64; 4]) -> usize {
        let [x0, y0, x1, y1] = self.pixels_of(r);
        (y0..y1)
            .flat_map(|y| (x0..x1).map(move |x| (x, y)))
            .filter(|&(x, y)| inked(self.pixel(x, y)))
            .count()
    }
    /// Painted pixels farther than one pixel outside `r`.
    fn ink_outside(&self, r: [f64; 4]) -> usize {
        let [x0, y0, x1, y1] = self.pixels_of(r);
        let (x0, y0) = (x0.saturating_sub(1), y0.saturating_sub(1));
        let (x1, y1) = (x1 + 1, y1 + 1);
        let mut count = 0;
        for y in 0..self.image.height {
            for x in 0..self.image.width {
                let inside = x >= x0 && x < x1 && y >= y0 && y < y1;
                if !inside && inked(self.pixel(x, y)) {
                    count += 1;
                }
            }
        }
        count
    }
    /// The page rect (y up) of the painted pixels inside `r`.
    fn ink_box(&self, r: [f64; 4]) -> Option<[f64; 4]> {
        let [x0, y0, x1, y1] = self.pixels_of(r);
        let mut found: Option<[u32; 4]> = None;
        for y in y0..y1 {
            for x in x0..x1 {
                if inked(self.pixel(x, y)) {
                    let b = found.get_or_insert([x, y, x, y]);
                    *b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
                }
            }
        }
        found.map(|[x0, y0, x1, y1]| {
            [
                f64::from(x0) / SCALE,
                PAGE_HEIGHT - f64::from(y1 + 1) / SCALE,
                f64::from(x1 + 1) / SCALE,
                PAGE_HEIGHT - f64::from(y0) / SCALE,
            ]
        })
    }
}

fn inked(rgb: [u8; 3]) -> bool {
    rgb.iter().any(|&c| c < 245)
}

/// Runs of painted pixels along the horizontal pixel row through page y,
/// from page x `from` to `to`.
fn runs_across(drawn: &Drawn, y: f64, from: f64, to: f64) -> usize {
    let mut runs = 0;
    let mut previous = false;
    let mut x = from;
    while x < to {
        let now = inked(drawn.at(x, y));
        if now && !previous {
            runs += 1;
        }
        previous = now;
        x += 0.5 / SCALE;
    }
    runs
}

/// Runs of painted pixels along the vertical pixel column through page x.
fn runs_down(drawn: &Drawn, x: f64, from: f64, to: f64) -> usize {
    let mut runs = 0;
    let mut previous = false;
    let mut y = from;
    while y < to {
        let now = inked(drawn.at(x, y));
        if now && !previous {
            runs += 1;
        }
        previous = now;
        y += 0.5 / SCALE;
    }
    runs
}

// ------------------------------------------------------------------ appearances

/// One stored appearance stream.
struct View {
    /// `N`, `D`, or `N/<state>`.
    key: String,
    root: ObjectId,
    rect: [f64; 4],
    objects: BTreeMap<ObjectId, Loaded>,
    drawn: OnceCell<Drawn>,
}

impl View {
    fn dict(&self) -> &Dictionary {
        self.objects[&self.root].0.as_dict().unwrap()
    }
    fn content(&self) -> &[u8] {
        self.objects[&self.root].1.as_deref().unwrap()
    }
    fn resolve(&self, value: &Object) -> Object {
        match value {
            Object::Reference(id) => self
                .objects
                .get(id)
                .map_or(Object::Null, |(value, _)| value.clone()),
            value => value.clone(),
        }
    }
    fn data(&self, value: &Object) -> Option<&[u8]> {
        let id = value.as_reference()?;
        self.objects.get(&id)?.1.as_deref()
    }
    /// /Resources /Font, resource name → font dictionary.
    fn fonts(&self) -> BTreeMap<Vec<u8>, Dictionary> {
        let resources = self.resolve(
            self.dict()
                .get(b"Resources".as_slice())
                .unwrap_or(&Object::Null),
        );
        let fonts = resources
            .as_dict()
            .and_then(|r| r.get(b"Font".as_slice()))
            .map(|f| self.resolve(f))
            .unwrap_or(Object::Null);
        let Some(fonts) = fonts.as_dict() else {
            return BTreeMap::new();
        };
        fonts
            .iter()
            .filter_map(|(name, font)| Some((name.clone(), self.resolve(font).as_dict()?.clone())))
            .collect()
    }
    fn ops(&self) -> Vec<Operation> {
        operations(self.content())
    }
    fn shows(&self) -> Vec<Show> {
        shows(&self.ops())
    }
    fn drawn(&self) -> &Drawn {
        let page = || appearance_page(&self.objects, self.root, self.rect);
        self.drawn.get_or_init(|| draw(page()))
    }
}

/// A widget's synthesized appearance, stored.
struct Shown {
    lossy: bool,
    rect: [f64; 4],
    forms: Vec<View>,
}

impl Shown {
    fn form(&self, key: &str) -> &View {
        (self.forms.iter().find(|form| form.key == key)).unwrap_or_else(|| panic!("no {key} form"))
    }
    fn normal(&self) -> &View {
        self.form("N")
    }
    /// The on state's form of a check box or radio button.
    fn on(&self) -> &View {
        (self
            .forms
            .iter()
            .find(|f| f.key.starts_with("N/") && f.key != "N/Off"))
        .expect("an on state")
    }
}

fn rect_of(doc: &Document<'_>, widget: &Dictionary) -> [f64; 4] {
    let r = numbers(Some(doc.dict_get(widget, b"Rect"))).unwrap();
    [
        r[0].min(r[2]),
        r[1].min(r[3]),
        r[0].max(r[2]),
        r[1].max(r[3]),
    ]
}

/// `widget` with `value` (see `widget_pdf`), synthesized and stored.
fn show(widget: &str, value: Option<&str>) -> Shown {
    show_in(widget, value, "")
}

fn show_in(widget: &str, value: Option<&str>, acro: &str) -> Shown {
    let bytes = widget_pdf(widget, value, acro);
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let dict = doc.resolve_id(id(6)).as_dict().unwrap();
    let synth = Context::of(&doc).synthesize(dict).expect("an appearance");
    let rect = rect_of(&doc, dict);
    let mut sink = Collect::default();
    let ap = appearance_dict(&mut sink, &synth).unwrap();
    let loader = |id: ObjectId| load(&doc, &sink, id);
    let mut forms = Vec::new();
    for (key, entry) in ap.iter() {
        let key = String::from_utf8_lossy(key).into_owned();
        let mut add = |key: String, value: &Object| {
            let root = value.as_reference().expect("an indirect form");
            forms.push(View {
                key,
                root,
                rect,
                objects: gather(&loader, root),
                drawn: OnceCell::new(),
            });
        };
        match entry {
            Object::Dictionary(states) => {
                for (state, value) in states.iter() {
                    add(format!("{key}/{}", String::from_utf8_lossy(state)), value);
                }
            }
            value => add(key, value),
        }
    }
    Shown {
        lossy: synth.lossy(),
        rect,
        forms,
    }
}

// ------------------------------------------------------------------ content

fn operations(content: &[u8]) -> Vec<Operation> {
    let mut parser = ContentParser::new(content, ContentLimits::default());
    let mut out = Vec::new();
    loop {
        match parser.next_operation() {
            Ok(Some(op)) => out.push(op),
            Ok(None) => return out,
            Err(error) => panic!(
                "content does not parse: {error}\n{}",
                String::from_utf8_lossy(content)
            ),
        }
    }
}

/// The operators of ISO 32000-2 Annex A.
const ISO_OPERATORS: [&str; 73] = [
    "b", "B", "b*", "B*", "BDC", "BI", "BMC", "BT", "BX", "c", "cm", "CS", "cs", "d", "d0", "d1",
    "Do", "DP", "EI", "EMC", "ET", "EX", "f", "F", "f*", "G", "g", "gs", "h", "i", "ID", "j", "J",
    "K", "k", "l", "m", "M", "MP", "n", "q", "Q", "re", "RG", "rg", "ri", "s", "S", "SC", "sc",
    "SCN", "scn", "sh", "T*", "Tc", "Td", "TD", "Tf", "Tj", "TJ", "TL", "Tm", "Tr", "Ts", "Tw",
    "Tz", "v", "w", "W", "W*", "y", "'", "\"",
];

/// ISO operators only, finite numbers, and balanced q/Q, BT/ET and marked
/// content.
fn structure_errors(ops: &[Operation]) -> Vec<String> {
    let mut errors = Vec::new();
    let (mut q, mut bt, mut marked) = (0i64, 0i64, 0i64);
    for op in ops {
        let operator = String::from_utf8_lossy(&op.operator).into_owned();
        if !ISO_OPERATORS.contains(&operator.as_str()) {
            errors.push(format!("operator {operator} is not ISO"));
        }
        let finite = op.operands.iter().all(|operand| match operand {
            ContentOperand::Real(v) => v.is_finite(),
            ContentOperand::Array(items) => items
                .iter()
                .all(|item| !matches!(item, ContentOperand::Real(v) if !v.is_finite())),
            _ => true,
        });
        if !finite {
            errors.push(format!("{operator} has a number that is not finite"));
        }
        match operator.as_str() {
            "q" => q += 1,
            "Q" => q -= 1,
            "BT" => bt += 1,
            "ET" => bt -= 1,
            "BMC" | "BDC" => marked += 1,
            "EMC" => marked -= 1,
            _ => {}
        }
        if q < 0 || !(0..=1).contains(&bt) || marked < 0 {
            errors.push(format!("unbalanced at {operator}"));
        }
    }
    if q != 0 || bt != 0 || marked != 0 {
        errors.push(format!(
            "unbalanced: q {q}, BT {bt}, marked content {marked}"
        ));
    }
    errors
}

fn number(operand: &ContentOperand) -> Option<f64> {
    match operand {
        ContentOperand::Integer(v) => Some(*v as f64),
        ContentOperand::Real(v) => Some(*v),
        _ => None,
    }
}

/// One text-showing operation and the font it shows in.
#[derive(Clone, Debug)]
struct Show {
    font: Vec<u8>,
    bytes: Vec<u8>,
}

fn shows(ops: &[Operation]) -> Vec<Show> {
    let mut out = Vec::new();
    let mut font = Vec::new();
    let mut saved = Vec::new();
    for op in ops {
        let strings = |operands: &[ContentOperand]| -> Vec<u8> {
            let mut bytes = Vec::new();
            for operand in operands {
                match operand {
                    ContentOperand::String(s) => bytes.extend_from_slice(s),
                    ContentOperand::Array(items) => {
                        for item in items {
                            if let ContentOperand::String(s) = item {
                                bytes.extend_from_slice(s);
                            }
                        }
                    }
                    _ => {}
                }
            }
            bytes
        };
        match op.operator.as_slice() {
            b"q" => saved.push(font.clone()),
            b"Q" => font = saved.pop().unwrap_or_default(),
            b"Tf" => {
                if let Some(ContentOperand::Name(name)) = op.operands.first() {
                    font = name.clone();
                }
            }
            b"Tj" | b"TJ" | b"'" | b"\"" => out.push(Show {
                font: font.clone(),
                bytes: strings(&op.operands),
            }),
            _ => {}
        }
    }
    out
}

/// The sizes of every `Tf`.
fn tf_sizes(ops: &[Operation]) -> Vec<f64> {
    ops.iter()
        .filter(|op| op.operator == b"Tf")
        .filter_map(|op| op.operands.get(1).and_then(number))
        .collect()
}

// ------------------------------------------------------------------ glyph names

/// What the engine's text extraction reads for each of `codes` shown in
/// the simple font `font` (direct values): a viewer's view of its glyph
/// names (Adobe Glyph List and `uniXXXX`, ISO 32000-2 §9.10.2).
fn read_codes(font: &Dictionary, codes: &[u8]) -> BTreeMap<u8, String> {
    let mut content = String::new();
    for (line, code) in codes.iter().enumerate() {
        let y = 780 - 6 * line;
        content.push_str(&format!("BT /F 4 Tf 10 {y} Td <{code:02X}> Tj ET\n"));
    }
    let objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
          /Resources << /Font << /F 5 0 R >> >> >>"
            .to_vec(),
        stream(b"", content.as_bytes()),
        serialize_object(&Object::Dictionary(font.clone()), WriteLimits::default()).unwrap(),
    ];
    let view = ViewDocument::open(classic(&objects), "").unwrap();
    let stext = view.structured_text(0).unwrap();
    let mut out: BTreeMap<u8, String> = BTreeMap::new();
    for line in stext.blocks().iter().flat_map(|block| &block.lines) {
        for ch in &line.chars {
            let line = ((780.0 - (PAGE_HEIGHT - ch.origin[1])) / 6.0).round();
            if let Some(code) = codes.get(line as usize) {
                out.entry(*code).or_default().push(ch.c);
            }
        }
    }
    out
}

/// The characters the codes a font's /Differences names read as.
fn named_chars(font: &Dictionary) -> BTreeMap<u8, String> {
    let codes: Vec<u8> = differences(font)
        .into_iter()
        .map(|(code, _)| code)
        .collect();
    read_codes(font, &codes)
}

/// The (code, glyph name) pairs a simple font's /Encoding /Differences sets.
fn differences(font: &Dictionary) -> Vec<(u8, Vec<u8>)> {
    let encoding = font.get(b"Encoding".as_slice()).and_then(Object::as_dict);
    let entries = encoding
        .and_then(|e| e.get(b"Differences".as_slice()))
        .and_then(Object::as_array)
        .unwrap_or(&[]);
    let mut code = 0i64;
    let mut out = Vec::new();
    for entry in entries {
        match entry {
            Object::Integer(next) => code = *next,
            Object::Name(name) => {
                out.push((u8::try_from(code).unwrap(), name.clone()));
                code += 1;
            }
            other => panic!("not a code or a name in /Differences: {other:?}"),
        }
    }
    out
}

/// The characters WinAnsiEncoding encodes.
fn win_ansi_chars() -> BTreeSet<char> {
    let win = SimpleEncoding::from_named(NamedEncoding::WinAnsi);
    (0..=255u8)
        .filter_map(|code| match win.lookup_unicode(code) {
            GlyphUnicode::Unicode([c]) => Some(*c),
            _ => None,
        })
        .collect()
}

/// The CIDSystemInfo /Ordering of the collection `han_font` picks.
fn ordering(lang: Option<&str>) -> String {
    let font = han_font(lang.map(str::as_bytes)).descendant_font(Object::Null);
    let info = font
        .get(b"CIDSystemInfo".as_slice())
        .and_then(Object::as_dict);
    let ordering = info.and_then(|i| i.get(b"Ordering".as_slice()));
    String::from_utf8_lossy(ordering.and_then(Object::as_string).unwrap()).into_owned()
}

const SINGLE: &str = "/FT /Tx /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g)";

fn multiline(rect: &str, extra: &str) -> String {
    format!("/FT /Tx /Ff 4096 /T (x) /Rect [{rect}] /DA (/Helv 12 Tf 0 g) {extra}")
}

/// The fonts of a form other than its DA font `Helv`.
fn fallbacks(form: &View) -> Vec<(Vec<u8>, Dictionary)> {
    (form.fonts().into_iter())
        .filter(|(name, _)| name.as_slice() != b"Helv")
        .collect()
}

// ------------------------------------------------------------------ 1. layout

#[test]
fn line_feeds_returns_and_crlf_each_end_one_line() {
    let shown = show(&multiline("100 500 280 700", ""), Some("a\r\nb\rc"));
    assert_eq!(shown.normal().drawn().line_texts(), ["a", "b", "c"]);
}

#[test]
fn an_empty_line_between_two_breaks_is_kept() {
    let gap = |value: &str| {
        let shown = show(&multiline("100 500 280 700", ""), Some(value));
        let drawn = shown.normal().drawn();
        let baselines: Vec<f64> = (drawn.lines.iter())
            .filter_map(|line| line.first().map(|ch| ch.origin[1]))
            .collect();
        assert_eq!(baselines.len(), 2, "{value:?}: {:?}", drawn.line_texts());
        baselines[0] - baselines[1]
    };
    let (one, two) = (gap("a\nb"), gap("a\n\nb"));
    assert!(one > 0.0);
    assert!(
        (two - 2.0 * one).abs() < 0.5,
        "pitch {one}, across an empty line {two}"
    );
}

/// A wrap breaks after the last space that fits; right-aligned, the space
/// left at the end of a wrapped line does not move its text.
#[test]
fn a_wrap_breaks_after_the_last_space_and_aligns_without_it() {
    let words = "one two three four five six seven eight nine ten";
    let shown = show(&multiline("100 500 190 700", "/Q 2"), Some(words));
    let drawn = shown.normal().drawn();
    let lines = drawn.line_texts();
    assert!(lines.len() > 2, "{lines:?}");
    let rejoined: Vec<&str> = lines.iter().flat_map(|l| l.split_whitespace()).collect();
    assert_eq!(
        rejoined,
        words.split(' ').collect::<Vec<_>>(),
        "whole words per line"
    );
    let ends: Vec<f64> = (drawn.lines.iter())
        .filter_map(|line| {
            let last = line.iter().rev().find(|ch| !ch.c.is_whitespace())?;
            Some(last.bbox[2])
        })
        .collect();
    let (lo, hi) = (
        ends.iter().copied().fold(f64::INFINITY, f64::min),
        ends.iter().copied().fold(f64::NEG_INFINITY, f64::max),
    );
    assert!(hi - lo < 1.0, "right edges {ends:?}");
    assert!(hi <= 190.0 && hi > 180.0, "right edges {ends:?}");
}

#[test]
fn a_word_wider_than_the_line_breaks_between_characters() {
    let word = "abcdefghij".repeat(4);
    let shown = show(&multiline("100 300 160 700", ""), Some(&word));
    let drawn = shown.normal().drawn();
    assert_eq!(drawn.compact(), word);
    assert!(drawn.line_texts().len() >= 3, "{:?}", drawn.line_texts());
    for ch in drawn.chars() {
        assert!(
            ch.bbox[0] >= 100.0 - 0.01 && ch.bbox[2] <= 160.0 + 0.01,
            "{ch:?} is outside the line"
        );
    }
}

#[test]
fn lines_below_the_box_are_not_written() {
    let value: Vec<String> = (1..=30).map(|i| i.to_string()).collect();
    let shown = show(&multiline("100 600 280 650", ""), Some(&value.join("\n")));
    let shows = shown.normal().shows();
    assert!(
        !shows.is_empty() && shows.len() < 12,
        "{} shows",
        shows.len()
    );
    let drawn = shown.normal().drawn();
    let lines = drawn.line_texts();
    assert!(!lines.is_empty() && lines.len() < 30);
    assert_eq!(lines, value[..lines.len()], "the first lines, in order");
}

#[test]
fn a_value_is_drawn_up_to_its_first_65536_characters() {
    assert_eq!(MAX_DRAWN_CHARS, 65_536);
    let shown = show(SINGLE, Some(&"x".repeat(70_000)));
    let drawn: usize = (shown.normal().shows().iter()).map(|s| s.bytes.len()).sum();
    assert!(drawn > 0 && drawn <= 65_536, "{drawn} characters drawn");
}

// ------------------------------------------------------------------ 2. encoding

#[test]
fn no_break_space_and_soft_hyphen_stay_in_the_da_font() {
    let shown = show(SINGLE, Some("nbsp\u{a0}x shy\u{ad}x"));
    assert!(!shown.lossy);
    let form = shown.normal();
    assert_eq!(form.fonts().len(), 1, "only the DA font");
    let bytes: Vec<u8> = form.shows().into_iter().flat_map(|s| s.bytes).collect();
    assert!(bytes.contains(&0xA0) && bytes.contains(&0xAD), "{bytes:?}");
}

#[test]
fn ignorable_characters_are_dropped() {
    let value = "a\u{200B}b\u{FEFF}c\u{2028}d\u{85}e\u{200D}f\u{FE0F}g";
    let shown = show(SINGLE, Some(value));
    assert!(!shown.lossy);
    let form = shown.normal();
    assert_eq!(form.fonts().len(), 1, "only the DA font");
    let bytes: Vec<u8> = form.shows().into_iter().flat_map(|s| s.bytes).collect();
    assert_eq!(bytes, b"abcdefg");
    assert_eq!(form.drawn().compact(), "abcdefg");
}

#[test]
fn a_mark_nothing_composes_with_is_lossy() {
    assert!(show(SINGLE, Some("q\u{301}")).lossy);
}

#[test]
fn decomposed_letters_draw_composed() {
    for (value, composed) in [("e\u{301}", "é"), ("Lo\u{301}dz\u{301}", "Lódź")] {
        let shown = show(SINGLE, Some(value));
        assert!(!shown.lossy, "{value:?}");
        assert_eq!(shown.normal().drawn().compact(), composed);
    }
}

#[test]
fn digits_and_a_space_before_greek_join_its_run() {
    let shown = show(SINGLE, Some("12 Ω"));
    assert!(!shown.lossy);
    let form = shown.normal();
    let shows = form.shows();
    assert_eq!(shows.len(), 1, "one run: {shows:?}");
    assert_ne!(shows[0].font, b"Helv", "in the Greek font");
    assert_eq!(form.drawn().compact(), "12Ω");
}

#[test]
fn a_symbol_winansi_lacks_takes_a_font_that_has_it() {
    let shown = show(SINGLE, Some("a≤b"));
    assert!(!shown.lossy);
    let form = shown.normal();
    assert!(form.fonts().len() >= 2);
    assert_eq!(form.drawn().compact(), "a≤b");
}

#[test]
fn the_euro_after_cyrillic_is_drawn_in_the_da_font() {
    let shown = show(SINGLE, Some("Привет €"));
    assert!(!shown.lossy);
    let form = shown.normal();
    let in_da = (form.shows().into_iter()).any(|s| s.font == b"Helv" && s.bytes.contains(&0x80));
    assert!(in_da, "{:?}", form.shows());
    assert_eq!(form.drawn().compact(), "Привет€");
}

/// The DA font's WinAnsi encoding (Annex D): `a` and `é` and `€` by their
/// codes, and a character no font encodes as `?`.
#[test]
fn win_ansi_maps_latin_and_replaces_the_rest() {
    let shown = show(SINGLE, Some("aé€→"));
    assert!(shown.lossy);
    let bytes: Vec<u8> = (shown.normal().shows().into_iter())
        .filter(|s| s.font == b"Helv")
        .flat_map(|s| s.bytes)
        .collect();
    assert_eq!(bytes, [b'a', 0xE9, 0x80, b'?']);
}

// ------------------------------------------------------------------ 3. tables

const FAMILIES: [(Family, &[u8]); 4] = [
    (Family::Helvetica, b"Helvetica"),
    (Family::HelveticaBold, b"Helvetica-Bold"),
    (Family::Times, b"Times-Roman"),
    (Family::Courier, b"Courier"),
];

const EIGHTS: [Eight; 3] = [Eight::Greek, Eight::Cyrillic, Eight::Latin];

#[test]
fn greek_and_cyrillic_capitals_take_upper_half_codes() {
    for (eight, c) in [(Eight::Cyrillic, 'Ж'), (Eight::Greek, 'Ω')] {
        let font = eight.font(b"Helvetica", Family::Helvetica);
        let read = named_chars(&font);
        let found = read.iter().find(|(_, text)| **text == c.to_string());
        let (code, _) = found.unwrap_or_else(|| panic!("{eight:?} has no {c}: {read:?}"));
        assert!(*code >= 0x80, "{c} at {code:#x}");
        assert!(eight.widths(Family::Helvetica)[usize::from(*code)] > 0.0);
    }
}

#[test]
fn block_elements_are_not_drawable() {
    for eight in EIGHTS {
        let font = eight.font(b"Helvetica", Family::Helvetica);
        for (code, text) in named_chars(&font) {
            assert!(
                !text.chars().any(|c| ('\u{2500}'..='\u{259F}').contains(&c)),
                "{eight:?} draws {text} at {code:#x}"
            );
        }
    }
}

/// Every 8-bit font names upper-half codes only, each by a glyph name a
/// viewer reads back as one distinct character, and its /Widths are the
/// declared advances.
#[test]
fn every_differences_name_maps_back_through_the_glyph_list() {
    for eight in EIGHTS {
        for (family, base) in FAMILIES {
            let font = eight.font(base, family);
            let what = format!("{eight:?} {family:?}");
            assert_eq!(font.get(b"Type".as_slice()), Some(&Object::name("Font")));
            assert_eq!(
                font.get(b"Subtype".as_slice()),
                Some(&Object::name("Type1"))
            );
            assert_eq!(font.get(b"BaseFont".as_slice()), Some(&Object::name(base)));
            let encoding = font.get(b"Encoding".as_slice()).and_then(Object::as_dict);
            assert_eq!(
                encoding.and_then(|e| e.get(b"BaseEncoding".as_slice())),
                Some(&Object::name("WinAnsiEncoding")),
                "{what}"
            );
            let named = differences(&font);
            assert!(!named.is_empty(), "{what}");
            let read = named_chars(&font);
            let mut seen = BTreeSet::new();
            for (code, name) in &named {
                let name = String::from_utf8_lossy(name);
                assert!(*code >= 0x80, "{what}: {name} at {code:#x}");
                let text = read.get(code).cloned().unwrap_or_default();
                let mut chars = text.chars();
                let (Some(c), None) = (chars.next(), chars.next()) else {
                    panic!("{what}: {name} at {code:#x} reads as {text:?}");
                };
                assert!(
                    u32::from(c) >= 0xA0 && c != '\u{FFFD}',
                    "{what}: {name} reads as {c:?}"
                );
                assert!(seen.insert(c), "{what}: {c} twice");
            }
            let widths = eight.widths(family);
            let first = font
                .get(b"FirstChar".as_slice())
                .and_then(Object::as_i64)
                .unwrap();
            let last = font
                .get(b"LastChar".as_slice())
                .and_then(Object::as_i64)
                .unwrap();
            let declared = numbers(font.get(b"Widths".as_slice())).unwrap();
            assert_eq!(declared.len() as i64, last - first + 1, "{what}");
            for (at, width) in declared.iter().enumerate() {
                let code = first as usize + at;
                assert_eq!(*width, widths[code].round(), "{what}: width of {code:#x}");
            }
            for (code, _) in &named {
                let code = i64::from(*code);
                assert!((first..=last).contains(&code), "{what}: {code:#x}");
                assert!(
                    widths[code as usize] > 0.0,
                    "{what}: {code:#x} draws nothing"
                );
            }
        }
    }
}

#[test]
fn ascii_advances_are_the_familys() {
    for eight in EIGHTS {
        assert_eq!(eight.widths(Family::Helvetica)[usize::from(b'A')], 667.0);
        assert_eq!(eight.widths(Family::Times)[usize::from(b'A')], 722.0);
        assert_eq!(eight.widths(Family::Courier)[usize::from(b'A')], 600.0);
        assert_eq!(eight.widths(Family::Helvetica)[usize::from(b' ')], 278.0);
    }
}

/// The extended-Latin font covers every letter of Latin Extended-A
/// WinAnsi lacks, plus S and T with comma below.
#[test]
fn the_latin_font_covers_extended_latin_winansi_lacks() {
    let win = win_ansi_chars();
    let font = Eight::Latin.font(b"Helvetica", Family::Helvetica);
    let covered: BTreeSet<char> = named_chars(&font)
        .values()
        .flat_map(|t| t.chars())
        .collect();
    let wanted = ('\u{100}'..='\u{17F}').chain('\u{218}'..='\u{21B}');
    let wanted: Vec<char> = wanted
        .filter(|c| c.is_alphabetic() && !win.contains(c))
        .collect();
    let missing: Vec<&char> = wanted.iter().filter(|c| !covered.contains(c)).collect();
    assert!(missing.is_empty(), "missing {missing:?}");
}

// ------------------------------------------------------------------ 4. han_font

#[test]
fn han_font_follows_explicit_language_tags() {
    for (lang, expected) in [
        (None, "GB1"),
        (Some("zh-CN"), "GB1"),
        (Some("zh-TW"), "CNS1"),
        (Some("zh-HK"), "CNS1"),
        (Some("zh-Hant"), "CNS1"),
        (Some("zh-Hant-CN"), "CNS1"),
        (Some("ja"), "Japan1"),
        (Some("ja-JP"), "Japan1"),
        (Some("ko"), "Korea1"),
        (Some("ko-KR"), "Korea1"),
        (Some("ZH-tw"), "CNS1"),
        (Some("JA"), "Japan1"),
    ] {
        assert_eq!(ordering(lang), expected, "{lang:?}");
    }
}

#[test]
fn han_font_defaults_to_simplified_chinese() {
    for lang in [
        Some("zh"),
        Some("zh-SG"),
        Some("zh-Hans"),
        Some("ZH-cn"),
        Some("en"),
        Some("fr"),
        Some(""),
    ] {
        assert_eq!(ordering(lang), "GB1", "{lang:?}");
    }
}

// ------------------------------------------------------------------ 5. single line

#[test]
fn single_line_text_stays_inside_its_box() {
    for height in [8.0, 12.0, 24.0, 100.0] {
        for size in [0.0, 8.0, 12.0] {
            let widget = format!(
                "/FT /Tx /T (x) /Rect [100 500 280 {}] /DA (/Helv {size} Tf 0 g)",
                500.0 + height
            );
            let shown = show(&widget, Some("Hxg"));
            let rect = shown.rect;
            let drawn = shown.normal().drawn();
            let what = format!("height {height}, size {size}");
            assert_eq!(drawn.ink_outside(rect), 0, "{what}");
            for ch in drawn.chars() {
                let y = ch.origin[1];
                assert!(y >= rect[1] && y <= rect[3], "{what}: baseline {y}");
            }
            let fits = size == 0.0 || height >= size + 4.0;
            if fits {
                assert_eq!(drawn.compact(), "Hxg", "{what}");
                let ink = drawn
                    .ink_box(rect)
                    .unwrap_or_else(|| panic!("{what}: no ink"));
                assert!(
                    ink[1] > rect[1] + 0.4 && ink[3] < rect[3] - 0.4,
                    "{what}: ink {ink:?} is cut by the box {rect:?}"
                );
            }
        }
    }
}

#[test]
fn auto_size_fills_a_tall_box_and_fits_a_long_value() {
    let tall = show(
        "/FT /Tx /T (x) /Rect [100 500 280 600] /DA (/Helv 0 Tf 0 g)",
        Some("Hxg"),
    );
    let ink = tall.normal().drawn().ink_box(tall.rect).unwrap();
    assert!(ink[3] - ink[1] >= 0.5 * 98.0, "ink {ink:?}");
    let value = "The quick brown fox jumps over the lazy dog";
    let long = show(
        "/FT /Tx /T (x) /Rect [100 700 200 724] /DA (/Helv 0 Tf 0 g)",
        Some(value),
    );
    let drawn = long.normal().drawn();
    assert_eq!(drawn.compact(), value.replace(' ', ""));
    let ink = drawn.ink_box(long.rect).unwrap();
    assert!(ink[0] > 100.4 && ink[2] < 199.6, "ink {ink:?}");
}

// ------------------------------------------------------------------ 6. multiline auto size

fn words(count: usize) -> String {
    let mut out = String::new();
    let mut i = 0;
    while out.len() < count {
        out.push_str(&format!("word{i} "));
        i += 1;
    }
    out.truncate(count);
    out.trim_end().to_string()
}

#[test]
fn multiline_auto_size_shrinks_a_long_text_to_fit() {
    let value = words(300);
    let shown = show(
        "/FT /Tx /Ff 4096 /T (x) /Rect [100 600 200 640] /DA (/Helv 0 Tf 0 g)",
        Some(&value),
    );
    let form = shown.normal();
    let drawn = form.drawn();
    assert_eq!(
        drawn.compact(),
        value.replace(' ', ""),
        "all text extracted"
    );
    assert_eq!(drawn.ink_outside(shown.rect), 0);
    for size in tf_sizes(&form.ops()) {
        assert!((4.0..=12.0).contains(&size), "size {size}");
    }
}

#[test]
fn multiline_auto_size_is_between_4_and_12_points() {
    let auto = "/FT /Tx /Ff 4096 /T (x) /DA (/Helv 0 Tf 0 g)";
    let size = |rect: &str, value: &str| {
        let shown = show(&format!("{auto} /Rect [{rect}]"), Some(value));
        let sizes = tf_sizes(&shown.normal().ops());
        assert!(!sizes.is_empty());
        sizes.into_iter().fold(0.0, f64::max)
    };
    assert_eq!(
        size("100 400 400 600", "Hi"),
        12.0,
        "a short text takes the cap"
    );
    assert_eq!(
        size("100 600 200 640", &words(4000)),
        4.0,
        "nothing fits: the minimum"
    );
    let middle = size("100 600 200 640", &words(300));
    assert!((4.0..12.0).contains(&middle), "{middle}");
}

// ------------------------------------------------------------------ 7. comb

const COMB: &str = "/FT /Tx /Ff 16777216 /MaxLen 6 /T (x) /Rect [100 700 280 724] \
                    /DA (/Helv 12 Tf 0 g) /MK << /BC [0] >>";

#[test]
fn comb_characters_are_centred_in_their_cells_between_dividers() {
    let shown = show(COMB, Some("123456"));
    let drawn = shown.normal().drawn();
    assert_eq!(drawn.compact(), "123456");
    let cell = 178.0 / 6.0;
    let chars: Vec<&Char> = drawn.chars().filter(|ch| !ch.c.is_whitespace()).collect();
    assert_eq!(chars.len(), 6);
    for (i, ch) in chars.iter().enumerate() {
        let centre = (ch.bbox[0] + ch.bbox[2]) / 2.0;
        let expected = 101.0 + (i as f64 + 0.5) * cell;
        assert!(
            (centre - expected).abs() <= 1.0,
            "{} at {centre}, cell centre {expected}",
            ch.c
        );
    }
    // Below the digits: only the five dividers cross the box.
    assert_eq!(runs_across(drawn, 702.5, 102.0, 278.0), 5);
}

#[test]
fn a_comb_field_draws_at_most_max_len_characters() {
    let shown = show(COMB, Some("12345678"));
    assert_eq!(shown.normal().drawn().compact(), "123456");
}

// ------------------------------------------------------------------ 8. quadding

#[test]
fn quadding_places_left_centre_and_right() {
    let starts: Vec<f64> = (0..3)
        .map(|q| {
            let shown = show(&format!("{SINGLE} /Q {q}"), Some("Hi"));
            let drawn = shown.normal().drawn();
            let chars: Vec<&Char> = drawn.chars().collect();
            let (first, last) = (chars[0], chars[chars.len() - 1]);
            if q == 1 {
                let centre = (first.bbox[0] + last.bbox[2]) / 2.0;
                assert!((centre - 190.0).abs() <= 1.0, "centred at {centre}");
            }
            if q == 2 {
                assert!(last.bbox[2] <= 280.0 && last.bbox[2] > 270.0, "{last:?}");
            }
            first.bbox[0]
        })
        .collect();
    assert!(starts[0] < starts[1] && starts[1] < starts[2], "{starts:?}");
    assert!(starts[0] >= 100.0 && starts[0] < 106.0, "{starts:?}");
}

// ------------------------------------------------------------------ 9. rotation

#[test]
fn a_rotated_field_extracts_its_value_inside_the_rect() {
    let shown = show(
        "/FT /Tx /T (x) /Rect [100 500 124 680] /DA (/Helv 12 Tf 0 g) /MK << /R 90 >>",
        Some("Rotated"),
    );
    let form = shown.normal();
    let bbox = numbers(form.dict().get(b"BBox".as_slice())).unwrap();
    assert_eq!(bbox, [0.0, 0.0, 180.0, 24.0]);
    let drawn = form.drawn();
    assert_eq!(drawn.compact(), "Rotated");
    assert_eq!(drawn.ink_outside(shown.rect), 0);
    assert!(drawn.ink(shown.rect) > 50);
}

// ------------------------------------------------------------------ 10. check box and radio

const CHECK: &str =
    "/FT /Btn /T (x) /V /Off /AS /Off /Rect [100 700 124 724] /MK << /BC [0] /BG [1]";

/// Pixels painted in `on` that `off` leaves white, as page rects of one pixel.
fn mark(off: &Drawn, on: &Drawn, rect: [f64; 4]) -> Vec<(u32, u32)> {
    let [x0, y0, x1, y1] = on.pixels_of(rect);
    (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .filter(|&(x, y)| inked(on.pixel(x, y)) && !inked(off.pixel(x, y)))
        .collect()
}

#[test]
fn a_check_box_mark_is_inside_the_box_and_only_on() {
    let shown = show(&format!("{CHECK} >> /DA (/ZaDb 0 Tf 0 g)"), None);
    let (off, on) = (shown.form("N/Off").drawn(), shown.on().drawn());
    let rect = shown.rect;
    let inner = [rect[0] + 2.0, rect[1] + 2.0, rect[2] - 2.0, rect[3] - 2.0];
    assert_eq!(off.ink(inner), 0, "Off draws no mark");
    let pixels = mark(off, on, rect);
    assert!(pixels.len() > 40, "{} mark pixels", pixels.len());
    let ys: Vec<u32> = pixels.iter().map(|p| p.1).collect();
    let span = |v: &[u32]| f64::from(v.iter().max().unwrap() - v.iter().min().unwrap() + 1) / SCALE;
    assert!(span(&ys) >= 0.4 * 22.0, "mark height {}", span(&ys));
    let [bx0, by0, bx1, by1] =
        on.pixels_of([rect[0] + 1.0, rect[1] + 1.0, rect[2] - 1.0, rect[3] - 1.0]);
    for (x, y) in &pixels {
        assert!(
            (bx0..bx1).contains(x) && (by0..by1).contains(y),
            "mark pixel {x},{y} on the border"
        );
    }
}

#[test]
fn a_check_box_mark_follows_the_caption_and_the_da_colour() {
    let plain = show(&format!("{CHECK} >> /DA (/ZaDb 0 Tf 0 g)"), None);
    let cross = show(&format!("{CHECK} /CA (8) >> /DA (/ZaDb 0 Tf 0 g)"), None);
    let rect = plain.rect;
    let (a, b) = (plain.on().drawn(), cross.on().drawn());
    let [x0, y0, x1, y1] = a.pixels_of(rect);
    let differ = (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .filter(|&(x, y)| inked(a.pixel(x, y)) != inked(b.pixel(x, y)))
        .count();
    assert!(
        differ > 20,
        "/CA 8 draws another mark: {differ} pixels differ"
    );
    let red = show(&format!("{CHECK} >> /DA (/ZaDb 0 Tf 1 0 0 rg)"), None);
    let on = red.on().drawn();
    let [x0, y0, x1, y1] = on.pixels_of(rect);
    let reds = (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let [r, g, b] = on.pixel(x, y);
            r > 200 && g < 80 && b < 80
        })
        .count();
    assert!(reds > 20, "{reds} red mark pixels");
}

const RADIO: &str = "/FT /Btn /Ff 32768 /T (x) /V /Off /AS /Off /Rect [100 700 124 724] \
                     /DA (/ZaDb 0 Tf 0 g) /MK << /BC [0] /BG [1]";

#[test]
fn a_radio_button_is_a_circle_with_a_dot() {
    let shown = show(&format!("{RADIO} >>"), None);
    let rect = shown.rect;
    let (off, on) = (shown.form("N/Off").drawn(), shown.on().drawn());
    for drawn in [off, on] {
        assert_eq!(drawn.ink_outside(rect), 0);
        assert!(drawn.ink(rect) > 30, "the circle");
        for (x, y) in [
            (100.0, 700.0),
            (122.5, 700.0),
            (100.0, 722.5),
            (122.5, 722.5),
        ] {
            assert_eq!(
                drawn.ink([x, y, x + 1.5, y + 1.5]),
                0,
                "corner {x},{y} is outside the circle"
            );
        }
    }
    let centre = [110.0, 710.0, 114.0, 714.0];
    assert_eq!(off.ink(centre), 0);
    assert!(on.ink(centre) > 10, "the dot");
    let star = show(&format!("{RADIO} /CA (u) >>"), None);
    let other = star.on().drawn();
    let [x0, y0, x1, y1] = on.pixels_of(rect);
    let differ = (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .filter(|&(x, y)| inked(on.pixel(x, y)) != inked(other.pixel(x, y)))
        .count();
    assert!(differ > 10, "/CA u draws its glyph: {differ} pixels differ");
    assert_eq!(other.ink_outside(rect), 0);
}

// ------------------------------------------------------------------ 11. push button

const PUSH: &str = "/FT /Btn /Ff 65536 /T (x) /Rect [100 700 200 730] /DA (/Helv 12 Tf 0 g)";

#[test]
fn a_push_button_shows_its_caption_and_a_different_down_state() {
    let shown = show(&format!("{PUSH} /MK << /BG [0.75] /CA (Go) >>"), None);
    let rect = shown.rect;
    let (normal, down) = (shown.normal().drawn(), shown.form("D").drawn());
    assert_eq!(normal.compact(), "Go");
    assert_eq!(down.compact(), "Go");
    for (x, y) in [
        (100.5, 700.5),
        (199.0, 700.5),
        (100.5, 729.0),
        (199.0, 729.0),
    ] {
        let [r, g, b] = normal.at(x, y);
        assert!(
            [r, g, b].iter().all(|c| (150..=230).contains(c)),
            "background at {x},{y}: {r} {g} {b}"
        );
    }
    let [x0, y0, x1, y1] = normal.pixels_of(rect);
    let differ = (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .filter(|&(x, y)| normal.pixel(x, y) != down.pixel(x, y))
        .count();
    assert!(differ > 50, "down differs from normal in {differ} pixels");
}

#[test]
fn a_push_button_without_a_caption_draws_no_text() {
    let shown = show(&format!("{PUSH} /MK << /BC [0] >>"), None);
    for form in &shown.forms {
        let shows = form.shows();
        assert!(
            shows.iter().all(|s| s.bytes.is_empty()),
            "{}: {shows:?}",
            form.key
        );
        assert_eq!(form.drawn().compact(), "", "{}", form.key);
    }
}

#[test]
fn the_down_state_shows_the_alternate_caption() {
    let shown = show(
        &format!("{PUSH} /MK << /BC [0] /CA (Go) /AC (Down) >>"),
        None,
    );
    assert_eq!(shown.normal().drawn().compact(), "Go");
    assert_eq!(shown.form("D").drawn().compact(), "Down");
}

// ------------------------------------------------------------------ 12. list box

const LIST: &str = "/FT /Ch /T (x) /Rect [100 600 200 700] /DA (/Helv 12 Tf 0 g)";

#[test]
fn a_list_box_bands_each_selected_option() {
    let shown = show(
        &format!("{LIST} /Opt [(alpha) (beta) (gamma) (delta)] /V [(beta) (delta)]"),
        None,
    );
    let drawn = shown.normal().drawn();
    assert_eq!(drawn.line_texts(), ["alpha", "beta", "gamma", "delta"]);
    // Right of the text, down the box: one band per selected option.
    assert_eq!(runs_down(drawn, 196.0, 600.5, 699.5), 2);
}

#[test]
fn a_list_box_shows_from_its_top_index() {
    let shown = show(
        &format!("{LIST} /TI 2 /Opt [(alpha) (beta) (gamma) (delta) (epsilon)] /V (delta)"),
        None,
    );
    let lines = shown.normal().drawn().line_texts();
    assert_eq!(
        lines.first().map(String::as_str),
        Some("gamma"),
        "{lines:?}"
    );
    assert!(
        !lines.iter().any(|l| l == "alpha" || l == "beta"),
        "{lines:?}"
    );
}

#[test]
fn a_list_box_draws_one_option_per_line() {
    let shown = show(
        &format!("{LIST} /Opt [(a long option that is wider than the box itself) (b)] /V (b)"),
        None,
    );
    let lines = shown.normal().drawn().line_texts();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!("a long option that is wider than the box itself".starts_with(&lines[0]));
    assert_eq!(lines[1], "b");
}

// ------------------------------------------------------------------ 13. border styles

const BORDERED: &str =
    "/FT /Tx /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g) /MK << /BC [0] >>";

#[test]
fn a_solid_border_is_drawn_whole() {
    let shown = show(&format!("{BORDERED} /BS << /W 2 /S /S >>"), None);
    let drawn = shown.normal().drawn();
    assert_eq!(runs_across(drawn, 723.0, 100.5, 279.5), 1);
    assert_eq!(runs_down(drawn, 100.5, 700.5, 723.5), 1);
}

#[test]
fn dashed_and_underline_borders_follow_the_border_style() {
    let dashed = show(&format!("{BORDERED} /BS << /W 2 /S /D >>"), None);
    let drawn = dashed.normal().drawn();
    assert!(
        runs_across(drawn, 723.0, 100.5, 279.5) >= 3,
        "dashes along the top"
    );
    let underline = show(&format!("{BORDERED} /BS << /W 2 /S /U >>"), None);
    let drawn = underline.normal().drawn();
    assert_eq!(
        drawn.ink([100.0, 703.0, 280.0, 724.0]),
        0,
        "nothing above the bottom edge"
    );
    assert!(
        drawn.ink([100.0, 700.0, 280.0, 702.0]) > 300,
        "the bottom edge"
    );
}

// ------------------------------------------------------------------ 14. hostile

#[test]
fn hostile_inputs_give_finite_content_quickly() {
    let started = Instant::now();
    let das = ["/Helv 1e-30 Tf 0 g", "/Helv -5 Tf 0 g", "Tf Tf rg /"];
    let widgets = [
        "/FT /Tx /T (x) /Rect [100 700 280 724]",
        "/FT /Tx /Ff 4096 /T (x) /Rect [100 600 280 724]",
        "/FT /Tx /Ff 16777216 /MaxLen 100000000 /T (x) /Rect [100 700 280 724] /MK << /BC [0] >>",
        "/FT /Tx /T (x) /Rect [1 1 2 2]",
        "/FT /Tx /T (x) /Rect [1000000000000000000000000000000 1 2 2000000000000000000000000000000]",
        "/FT /Btn /Ff 65536 /T (x) /Rect [1 1 2 2] /MK << /CA (Go) >>",
        "/FT /Ch /T (x) /Opt [(a) (b)] /Rect [1 1 2 2]",
        "/FT /Btn /T (x) /Rect [0 0 0 0]",
    ];
    for da in das {
        for widget in widgets {
            let bytes = widget_pdf(&format!("{widget} /DA ({da})"), Some("value 12"), "");
            let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
            let dict = doc.resolve_id(id(6)).as_dict().unwrap();
            let Some(synth) = Context::of(&doc).synthesize(dict) else {
                continue;
            };
            for form in synth.forms() {
                let text = String::from_utf8_lossy(&form.content).to_lowercase();
                assert!(
                    !text.contains("nan") && !text.contains("inf"),
                    "{widget} {da}: {text}"
                );
                let errors = structure_errors(&operations(&form.content));
                assert!(errors.is_empty(), "{widget} {da}: {errors:?}");
                assert!(form.width.is_finite() && form.height.is_finite());
            }
        }
    }
    assert!(started.elapsed() < Duration::from_secs(20));
}

#[test]
fn a_negative_da_size_is_auto_size() {
    let shown = show(
        "/FT /Tx /T (x) /Rect [100 700 280 724] /DA (/Helv -5 Tf 0 g)",
        Some("Mirror"),
    );
    let form = shown.normal();
    let sizes = tf_sizes(&form.ops());
    assert!(
        !sizes.is_empty() && sizes.iter().all(|s| *s > 0.0),
        "{sizes:?}"
    );
    assert_eq!(form.drawn().compact(), "Mirror");
}

// ------------------------------------------------------------------ owner decisions

/// W1: Greek, Cyrillic and extended-Latin field text embeds the fallback
/// font (a subset) with a /ToUnicode, so every viewer draws it.
#[test]
fn greek_cyrillic_and_extended_latin_fallbacks_are_embedded() {
    for value in ["Привет", "Ελληνικά", "Łódź"] {
        let shown = show(SINGLE, Some(value));
        assert!(!shown.lossy, "{value}");
        let form = shown.normal();
        let fallbacks = fallbacks(form);
        assert_eq!(fallbacks.len(), 1, "{value}");
        for (name, font) in fallbacks {
            let what = format!("{value} {}", String::from_utf8_lossy(&name));
            let descriptor = font
                .get(b"FontDescriptor".as_slice())
                .map(|d| form.resolve(d))
                .unwrap_or(Object::Null);
            let descriptor = descriptor
                .as_dict()
                .unwrap_or_else(|| panic!("{what}: no descriptor"));
            let program = [b"FontFile".as_slice(), b"FontFile2", b"FontFile3"]
                .iter()
                .find_map(|key| descriptor.get(*key))
                .unwrap_or_else(|| panic!("{what}: no font program"));
            assert!(form.data(program).is_some_and(|d| !d.is_empty()), "{what}");
            let to_unicode = font.get(b"ToUnicode".as_slice());
            assert!(
                to_unicode.and_then(|t| form.data(t)).is_some(),
                "{what}: /ToUnicode"
            );
        }
        assert_eq!(form.drawn().compact(), value);
    }
}

/// W2: the CJK fonts carry the names Adobe's non-embedded CJK fonts have.
#[test]
fn cjk_fonts_use_adobe_names() {
    let names: [(&str, &[&str]); 4] = [
        (
            "GB1",
            &["STSong-Light", "STHeiti-Regular", "AdobeSongStd-Light"],
        ),
        (
            "CNS1",
            &["MSung-Light", "MHei-Medium", "AdobeMingStd-Light"],
        ),
        (
            "Japan1",
            &[
                "HeiseiMin-W3",
                "KozMinPro-Regular",
                "HeiseiKakuGo-W5",
                "KozGoPro-Medium",
            ],
        ),
        (
            "Korea1",
            &[
                "HYSMyeongJo-Medium",
                "HYGoThic-Medium",
                "AdobeMyungjoStd-Medium",
            ],
        ),
    ];
    for lang in [None, Some("zh-TW"), Some("ja"), Some("ko")] {
        let cjk = han_font(lang.map(str::as_bytes));
        let descendant = cjk.descendant_font(Object::Null);
        let base = descendant
            .get(b"BaseFont".as_slice())
            .and_then(Object::as_name)
            .unwrap();
        let base = String::from_utf8_lossy(base).into_owned();
        let order = ordering(lang);
        let allowed = names.iter().find(|(o, _)| *o == order).unwrap().1;
        assert!(allowed.contains(&base.as_str()), "{order}: {base}");
        // ISO 32000-2 §9.7.6.1: the Type0 font's name, or it with the CMap's.
        let font = cjk.font(Object::Null);
        let cmap = font
            .get(b"Encoding".as_slice())
            .and_then(Object::as_name)
            .unwrap();
        let cmap = String::from_utf8_lossy(cmap);
        let type0 = font
            .get(b"BaseFont".as_slice())
            .and_then(Object::as_name)
            .unwrap();
        let type0 = String::from_utf8_lossy(type0);
        assert!(
            type0 == base || type0 == format!("{base}-{cmap}"),
            "{order}: {type0}"
        );
        let descriptor = cjk.descriptor();
        assert_eq!(
            descriptor.get(b"FontName".as_slice()),
            Some(&Object::name(base.as_bytes())),
            "{order}"
        );
    }
}

/// W5: the Cyrillic layout is wider than KOI8-U, so Serbian and
/// Macedonian letters draw (Belarusian: `app_forms.rs`).
#[test]
fn cyrillic_letters_beyond_koi8_u_draw() {
    for value in ["Ђђ Љљ Њњ Ћћ Џџ", "Ѓѓ Ќќ Ѕѕ"] {
        let shown = show(SINGLE, Some(value));
        assert!(!shown.lossy, "{value}");
        assert_eq!(shown.normal().drawn().compact(), value.replace(' ', ""));
    }
}
