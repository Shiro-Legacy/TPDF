// SPDX-License-Identifier: AGPL-3.0-or-later
//! Acceptance for spec 06 (annotation appearance streams), new cases of
//! section 9.3 and the owner's decisions. Every check is black-box: an
//! appearance is judged by its structure (the content parses, with ISO
//! operators only, balanced `q`/`Q` and `BT`/`ET`), by where and in what
//! colour it paints once drawn through the view, and by the text the
//! engine's own extraction reads back from it. Expected values come from
//! the spec, ISO 32000-2 and the Adobe Core 14 AFM files.
use std::sync::Arc;

use serde_json::{Value, json};
use tandem_pdf_engine::app::annots::appearance::{self, Form, FreeText, Mark, Style};
use tandem_pdf_engine::app::annots::fonts::{self, Base14};
use tandem_pdf_engine::app::annots::{self, Color, Point, Quad};
use tandem_pdf_engine::app::edit::{Builder, Edit};
use tandem_pdf_engine::app::forms::scripts::han_font;
use tandem_pdf_engine::app::ops::annots::NOTE_SIZE;
use tandem_pdf_engine::app::{AppEngine, AppSession};
use tandem_pdf_engine::content::{ContentLimits, ContentOperand, ContentParser, Operation};
use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::lexer::LexLimits;
use tandem_pdf_engine::object::{Dictionary, Object, ObjectId};
use tandem_pdf_engine::stream::StreamLimits;
use tandem_pdf_engine::view::{RenderOptions, RgbaImage, ViewDocument};
use tandem_pdf_engine::writer::{DocumentWriteLimits, WriteOptions, write_document};

// ------------------------------------------------------------------ files

fn id(n: u32) -> ObjectId {
    ObjectId::new(n, 0).unwrap()
}

/// A classic-xref PDF from object bodies numbered from 1.
fn classic(objects: &[String]) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", index + 1).as_bytes());
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

fn stream(dictionary: &str, data: &str) -> String {
    format!(
        "<< {dictionary} /Length {} >>\nstream\n{data}\nendstream",
        data.len()
    )
}

/// One 612 x 792 page (object 3) drawing `content` (object 4) with
/// `resources`, its annotations from object 5.
fn page_with(resources: &str, content: &str, annots: &[&str]) -> Vec<u8> {
    let list: Vec<String> = (0..annots.len())
        .map(|i| format!("{} 0 R", 5 + i))
        .collect();
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << {resources} >> /Contents 4 0 R /Annots [{}] >>",
            list.join(" ")
        ),
        stream("", content),
    ];
    objects.extend(annots.iter().map(|a| a.to_string()));
    classic(&objects)
}

fn blank() -> Vec<u8> {
    page_with("", "", &[])
}

fn open(bytes: &[u8]) -> Document<'static> {
    Document::open_shared(Arc::new(bytes.to_vec()), DocumentLimits::default(), b"").unwrap()
}

fn write(doc: &Document<'_>) -> Vec<u8> {
    write_document(doc, DocumentWriteLimits::default(), WriteOptions::APP).unwrap()
}

/// The matrix that maps a form's /BBox, transformed by its /Matrix, onto
/// `rect` (ISO 32000-2 §12.5.5): the `cm` under which `Do` draws it there.
fn placement(bbox: [f64; 4], matrix: [f64; 6], rect: [f64; 4]) -> [f64; 6] {
    let shown = annots::transform_rect(&matrix, bbox);
    let sx = (rect[2] - rect[0]) / (shown[2] - shown[0]);
    let sy = (rect[3] - rect[1]) / (shown[3] - shown[1]);
    [
        sx,
        0.0,
        0.0,
        sy,
        rect[0] - shown[0] * sx,
        rect[1] - shown[1] * sy,
    ]
}

fn cm(m: [f64; 6]) -> String {
    let numbers: Vec<String> = m.iter().map(|v| format!("{v:.6}")).collect();
    format!("{} cm", numbers.join(" "))
}

/// A page whose content draws `form` (stored as `annots::put_appearance`
/// stores an appearance) into `rect`, user space.
fn form_page(form: &Form, rect: [f64; 4]) -> Vec<u8> {
    let content = format!("q {} /A Do Q", cm(placement(form.bbox, form.matrix, rect)));
    let base = page_with("", &content, &[]);
    let mut builder = Builder::from_document(open(&base));
    let stored = annots::put_appearance(&mut builder, &mut Dictionary::new(), form).unwrap();
    builder
        .update(id(3), |page| {
            let page = page.as_dict_mut().unwrap();
            let mut xobjects = Dictionary::new();
            xobjects.insert(b"A".to_vec(), Object::Reference(stored));
            let mut resources = Dictionary::new();
            resources.insert(b"XObject".to_vec(), Object::Dictionary(xobjects));
            page.insert(b"Resources".to_vec(), Object::Dictionary(resources));
        })
        .unwrap();
    write(builder.document())
}

/// `form` drawn at its own size with its /BBox's lower left at (100, 100).
fn form_at_size(form: &Form) -> (Vec<u8>, [f64; 4]) {
    let shown = annots::transform_rect(&form.matrix, form.bbox);
    let rect = [
        100.0,
        100.0,
        100.0 + shown[2] - shown[0],
        100.0 + shown[3] - shown[1],
    ];
    (form_page(form, rect), rect)
}

/// The saved file with the normal appearance of page 0's first annotation
/// of `subtype` drawn by the page's content where the annotation shows it
/// (the page's /Rotate kept), the annotations themselves dropped: what a
/// viewer shows, as page content the text extraction reads.
fn flattened(saved: &[u8], subtype: &[u8]) -> Vec<u8> {
    let doc = open(saved);
    let pages = tandem_pdf_engine::app::info::pages(&doc);
    let page = pages[0].id.expect("an indirect page");
    let list = doc.dict_get(pages[0].dict, b"Annots").as_array().unwrap();
    let (form, rect) = list
        .iter()
        .filter_map(|a| doc.resolve(a).as_dict())
        .filter(|a| doc.dict_get(a, b"Subtype").as_name() == Some(subtype))
        .find_map(|a| {
            let ap = doc.dict_get(a, b"AP").as_dict()?;
            let n = ap.get(b"N".as_slice())?.as_reference()?;
            let rect = annots::rect(&doc, a.get(b"Rect".as_slice())?)?;
            Some((n, rect))
        })
        .expect("an annotation with a stored appearance");
    let dict = doc.resolve_id(form).as_dict().unwrap();
    let bbox = annots::rect(&doc, dict.get(b"BBox".as_slice()).unwrap()).unwrap();
    let matrix = match dict
        .get(b"Matrix".as_slice())
        .and_then(|m| annots::numbers(&doc, m))
    {
        Some(m) if m.len() == 6 => [m[0], m[1], m[2], m[3], m[4], m[5]],
        _ => appearance::IDENTITY,
    };
    drop(pages);
    let content = format!("q {} /A Do Q", cm(placement(bbox, matrix, rect)));
    let mut builder = Builder::from_document(doc);
    let stream = builder.allocate(Object::Null).unwrap();
    builder
        .put_stream_raw(stream, Dictionary::new(), content.into_bytes())
        .unwrap();
    builder
        .update(page, |page| {
            let page = page.as_dict_mut().unwrap();
            let mut xobjects = Dictionary::new();
            xobjects.insert(b"A".to_vec(), Object::Reference(form));
            let mut resources = Dictionary::new();
            resources.insert(b"XObject".to_vec(), Object::Dictionary(xobjects));
            page.insert(b"Resources".to_vec(), Object::Dictionary(resources));
            page.insert(b"Contents".to_vec(), Object::Reference(stream));
            page.remove(b"Annots".as_slice());
        })
        .unwrap();
    write(builder.document())
}

fn render(pdf: &[u8], scale: f64) -> RgbaImage {
    ViewDocument::open(pdf.to_vec(), "")
        .unwrap()
        .render(0, scale, RenderOptions::default())
        .unwrap()
}

/// The text lines the engine extracts from page 0, each with its
/// characters' baseline origins (page space, y down).
fn text_lines(pdf: &[u8]) -> Vec<(String, Vec<[f64; 2]>)> {
    let text = ViewDocument::open(pdf.to_vec(), "")
        .unwrap()
        .structured_text(0)
        .unwrap();
    let mut lines = Vec::new();
    for block in text.blocks() {
        for line in &block.lines {
            let s: String = line.chars.iter().map(|c| c.c).collect();
            let origins = line.chars.iter().map(|c| c.origin).collect();
            if !s.trim().is_empty() {
                lines.push((s, origins));
            }
        }
    }
    lines
}

/// Whitespace runs as one space, trimmed.
fn normalised(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn text_of(pdf: &[u8]) -> String {
    let lines: Vec<String> = text_lines(pdf).into_iter().map(|(s, _)| s).collect();
    normalised(&lines.join(" "))
}

// ------------------------------------------------------------------ pixels

fn pixel(image: &RgbaImage, x: u32, y: u32) -> [u8; 3] {
    let i = ((y * image.width + x) * 4) as usize;
    [image.pixels[i], image.pixels[i + 1], image.pixels[i + 2]]
}

fn inked(p: [u8; 3]) -> bool {
    p.iter().any(|&c| c < 250)
}

/// The bounds of non-white pixels, x0 y0 x1 y1 inclusive, y down.
fn ink_box(image: &RgbaImage) -> Option<[u32; 4]> {
    let mut found: Option<[u32; 4]> = None;
    for y in 0..image.height {
        for x in 0..image.width {
            if inked(pixel(image, x, y)) {
                let b = found.get_or_insert([x, y, x, y]);
                *b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
            }
        }
    }
    found
}

/// `rect` (user space, on a 792 pt tall page) in pixels at `scale`.
fn pixels_of(rect: [f64; 4], scale: f64) -> [f64; 4] {
    [
        rect[0] * scale,
        (792.0 - rect[3]) * scale,
        rect[2] * scale,
        (792.0 - rect[1]) * scale,
    ]
}

/// Every non-white pixel lies inside `rect` (user space) give or take a
/// pixel, and some pixel does.
fn assert_ink_inside(image: &RgbaImage, rect: [f64; 4], scale: f64, label: &str) {
    let ink = ink_box(image).unwrap_or_else(|| panic!("{label}: nothing drawn"));
    let r = pixels_of(rect, scale);
    assert!(
        f64::from(ink[0]) >= r[0] - 1.0
            && f64::from(ink[1]) >= r[1] - 1.0
            && f64::from(ink[2]) <= r[2] + 1.0
            && f64::from(ink[3]) <= r[3] + 1.0,
        "{label}: ink {ink:?} outside {r:?}"
    );
}

fn luminance(p: [u8; 3]) -> f64 {
    0.299 * f64::from(p[0]) + 0.587 * f64::from(p[1]) + 0.114 * f64::from(p[2])
}

/// The most frequent non-white colour inside a pixel box.
fn dominant(image: &RgbaImage, area: [u32; 4]) -> [u8; 3] {
    let mut counts: std::collections::HashMap<[u8; 3], usize> = Default::default();
    for y in area[1]..=area[3].min(image.height - 1) {
        for x in area[0]..=area[2].min(image.width - 1) {
            let p = pixel(image, x, y);
            if inked(p) {
                *counts.entry(p).or_default() += 1;
            }
        }
    }
    counts
        .into_iter()
        .max_by_key(|&(p, n)| (n, p))
        .map(|(p, _)| p)
        .expect("ink")
}

// ------------------------------------------------------------------ content

fn operations(content: &[u8]) -> Vec<Operation> {
    let defaults = ContentLimits::default();
    let limits = ContentLimits {
        max_operations: 4_000_000,
        max_nodes: 20_000_000,
        lex: LexLimits {
            max_tokens: 20_000_000,
            ..defaults.lex
        },
        ..defaults
    };
    let mut parser = ContentParser::new(content, limits);
    let mut out = Vec::new();
    while let Some(operation) = parser
        .next_operation()
        .unwrap_or_else(|e| panic!("content parses: {e}"))
    {
        out.push(operation);
    }
    out
}

/// Operators of ISO 32000-2 §8.4 (graphics state), §8.5 (paths), §8.6.8
/// (colour), §8.8 (Do), §9.3-9.4 (text state, objects, positioning and
/// showing) and §14.6 (marked content).
const ISO_OPERATORS: &[&str] = &[
    "w", "J", "j", "M", "d", "ri", "i", "gs", "q", "Q", "cm", "m", "l", "c", "v", "y", "h", "re",
    "S", "s", "f", "F", "f*", "B", "B*", "b", "b*", "n", "W", "W*", "CS", "cs", "SC", "SCN", "sc",
    "scn", "G", "g", "RG", "rg", "K", "k", "Do", "BT", "ET", "Tc", "Tw", "Tz", "TL", "Tf", "Tr",
    "Ts", "Td", "TD", "Tm", "T*", "Tj", "TJ", "'", "\"", "BMC", "BDC", "EMC", "MP", "DP",
];

/// The content parses, uses ISO operators only, nests `q`/`Q` and
/// `BT`/`ET` in balance, is ASCII outside string operands and writes no
/// `NaN` or `inf`. Returns its operations.
fn checked(content: &[u8], label: &str) -> Vec<Operation> {
    let ops = operations(content);
    let (mut q, mut bt) = (0i64, 0i64);
    for op in &ops {
        let name = String::from_utf8_lossy(&op.operator).into_owned();
        assert!(
            ISO_OPERATORS.contains(&name.as_str()),
            "{label}: operator {name}"
        );
        match name.as_str() {
            "q" => q += 1,
            "Q" => q -= 1,
            "BT" => bt += 1,
            "ET" => bt -= 1,
            _ => {}
        }
        assert!(
            q >= 0 && (0..=1).contains(&bt),
            "{label}: nesting at {name}"
        );
        for operand in &op.operands {
            if let ContentOperand::Real(v) = operand {
                assert!(v.is_finite(), "{label}: {v}");
            }
        }
    }
    assert_eq!((q, bt), (0, 0), "{label}: q/Q and BT/ET balance");
    let text = String::from_utf8_lossy(content);
    assert!(
        !text.contains("NaN") && !text.contains("inf"),
        "{label}: {}",
        text.chars().take(300).collect::<String>()
    );
    ops
}

fn count(ops: &[Operation], names: &[&str]) -> usize {
    ops.iter()
        .filter(|op| names.iter().any(|n| op.operator == n.as_bytes()))
        .count()
}

fn number_of(operand: &ContentOperand) -> f64 {
    match operand {
        ContentOperand::Integer(v) => *v as f64,
        ContentOperand::Real(v) => *v,
        other => panic!("a number: {other:?}"),
    }
}

fn finite_rect(rect: [f64; 4]) -> bool {
    rect.iter().all(|v| v.is_finite()) && rect[0] <= rect[2] && rect[1] <= rect[3]
}

fn contains(outer: [f64; 4], inner: [f64; 4], slack: f64) -> bool {
    outer[0] <= inner[0] + slack
        && outer[1] <= inner[1] + slack
        && outer[2] >= inner[2] - slack
        && outer[3] >= inner[3] - slack
}

// ------------------------------------------------------------------ session

fn session_with(bytes: Vec<u8>) -> (AppEngine, AppSession) {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    let request = json!({"id": 1, "op": "open", "name": "a.pdf", "pdf": {"$buffer": 0}});
    let (reply, _) = engine.dispatch(&mut session, request, vec![bytes]);
    assert_eq!(reply["ok"], true, "{reply}");
    (engine, session)
}

fn op(engine: &mut AppEngine, session: &mut AppSession, request: Value) -> Value {
    let reply = engine.dispatch(session, request.clone(), Vec::new()).0;
    assert_eq!(reply["ok"], true, "{request} -> {reply}");
    reply
}

fn document(session: &AppSession) -> &Document<'static> {
    session.doc.as_ref().unwrap().document()
}

fn saved(session: &AppSession) -> Vec<u8> {
    write(document(session))
}

fn items(engine: &mut AppEngine, session: &mut AppSession) -> Vec<Value> {
    let reply = op(engine, session, json!({"op": "annots", "page": 0}));
    reply["items"].as_array().unwrap().clone()
}

fn rect_of(item: &Value) -> [f64; 4] {
    let r: Vec<f64> = (item["rect"].as_array().unwrap().iter())
        .map(|v| v.as_f64().unwrap())
        .collect();
    [r[0], r[1], r[2], r[3]]
}

fn annot(session: &AppSession, number: u32) -> Dictionary {
    (document(session).resolve_id(id(number)).as_dict())
        .unwrap()
        .clone()
}

/// An annotation's stored normal appearance: dictionary and content.
fn stored(session: &AppSession, number: u32) -> Option<(Dictionary, Vec<u8>)> {
    let doc = document(session);
    let dict = annot(session, number);
    let n = doc
        .dict_get(&dict, b"AP")
        .as_dict()?
        .get(b"N".as_slice())?
        .as_reference()?;
    let form = doc.resolve_id(n).as_dict()?.clone();
    Some((
        form,
        doc.decoded_stream(n, StreamLimits::default()).unwrap(),
    ))
}

fn view_image(session: &mut AppSession, scale: f64) -> RgbaImage {
    session
        .view()
        .unwrap()
        .render(0, scale, RenderOptions::default())
        .unwrap()
}

fn rgb(r: f64, g: f64, b: f64) -> Color {
    Color::Rgb([r, g, b])
}

fn quad(x0: f64, y0: f64, x1: f64, y1: f64) -> Quad {
    [(x0, y1), (x1, y1), (x0, y0), (x1, y0)]
}

fn free_text_box<'a>(text: &'a str, size: f64) -> FreeText<'a> {
    FreeText {
        text,
        font_name: b"Helv",
        font: Base14::Helvetica,
        han: han_font(None),
        size,
        color: None,
        border: 0.0,
        dash: Vec::new(),
        background: None,
        quadding: 0,
        rotate: 0,
        opacity: 1.0,
    }
}

// ================================================================== 9.3 (1) note

const NOTE_NAMES: [&[u8]; 7] = [
    b"Comment",
    b"Key",
    b"Note",
    b"Help",
    b"NewParagraph",
    b"Paragraph",
    b"Insert",
];

/// A note icon drawn at its own size, at 4 px per point.
fn note_drawn(name: &[u8], color: &Color) -> (RgbaImage, [f64; 4]) {
    let form = appearance::note(name, Some(color), 1.0);
    checked(&form.content, &String::from_utf8_lossy(name));
    let (pdf, rect) = form_at_size(&form);
    (render(&pdf, 4.0), rect)
}

#[test]
fn note_icons_draw_inside_their_box_and_differ_by_name() {
    let yellow = rgb(1.0, 1.0, 0.0);
    let mut drawn = Vec::new();
    for name in NOTE_NAMES {
        let (image, rect) = note_drawn(name, &yellow);
        assert_ink_inside(&image, rect, 4.0, &String::from_utf8_lossy(name));
        drawn.push(image.pixels);
    }
    for i in 0..drawn.len() {
        for j in i + 1..drawn.len() {
            assert!(
                drawn[i] != drawn[j],
                "{} and {} look alike",
                String::from_utf8_lossy(NOTE_NAMES[i]),
                String::from_utf8_lossy(NOTE_NAMES[j])
            );
        }
    }
}

#[test]
fn an_unknown_note_name_draws_the_note_icon() {
    let yellow = rgb(1.0, 1.0, 0.0);
    let (note, _) = note_drawn(b"Note", &yellow);
    for other in [b"Foo".as_slice(), b""] {
        let (image, _) = note_drawn(other, &yellow);
        assert!(image.pixels == note.pixels, "{other:?} draws the Note icon");
    }
}

#[test]
fn a_note_form_is_the_note_size_with_an_identity_matrix() {
    let form = appearance::note(b"Note", None, 1.0);
    assert_eq!(form.bbox, [0.0, 0.0, NOTE_SIZE, NOTE_SIZE]);
    assert_eq!(form.matrix, appearance::IDENTITY);
}

/// The background (the commonest colour) and the glyph (the pixels
/// farthest from it in luminance, outside the frame) of a note icon.
fn note_contrast(color: &Color) -> (f64, f64) {
    let (image, rect) = note_drawn(b"Comment", color);
    let r = pixels_of(rect, 4.0);
    // Away from the edge, where a frame may run.
    let inner = [
        (r[0] + 8.0) as u32,
        (r[1] + 8.0) as u32,
        (r[2] - 8.0) as u32,
        (r[3] - 8.0) as u32,
    ];
    let background = dominant(&image, inner);
    let lb = luminance(background);
    let mut farthest = lb;
    for y in inner[1]..=inner[3] {
        for x in inner[0]..=inner[2] {
            let l = luminance(pixel(&image, x, y));
            if (l - lb).abs() > (farthest - lb).abs() {
                farthest = l;
            }
        }
    }
    (lb, farthest)
}

#[test]
fn a_note_glyph_contrasts_with_its_background() {
    let (background, glyph) = note_contrast(&rgb(1.0, 1.0, 0.0));
    assert!(
        glyph < background - 100.0,
        "yellow: {glyph} on {background}"
    );
    for dark in [rgb(0.0, 0.0, 0.6), rgb(0.5, 0.0, 0.0)] {
        let (background, glyph) = note_contrast(&dark);
        assert!(
            glyph > background + 100.0,
            "{dark:?}: {glyph} on {background}"
        );
    }
}

// ================================================================== 9.3 (2) stamp labels

/// The text a stamp for `name` shows.
fn stamp_label(name: &[u8]) -> String {
    let form = appearance::stamp(name, None, 1.0, appearance::IDENTITY);
    checked(&form.content, &String::from_utf8_lossy(name));
    text_of(&form_at_size(&form).0)
}

#[test]
fn standard_stamp_names_read_as_their_labels() {
    for (name, label) in [
        (b"Approved".as_slice(), "APPROVED"),
        (b"NotForPublicRelease", "NOT FOR PUBLIC RELEASE"),
        (b"AsIs", "AS IS"),
        (b"ForComment", "FOR COMMENT"),
        (b"TopSecret", "TOP SECRET"),
        (b"Draft", "DRAFT"),
        (b"", "DRAFT"),
    ] {
        assert_eq!(stamp_label(name), label, "{name:?}");
    }
}

#[test]
fn any_stamp_name_reads_in_capitals_with_spaces() {
    for (name, label) in [
        (b"Void".as_slice(), "VOID"),
        (b"approved", "APPROVED"),
        (b"Draft2", "DRAFT 2"),
        (b"MyOwnStamp", "MY OWN STAMP"),
    ] {
        assert_eq!(stamp_label(name), label, "{name:?}");
    }
}

/// The longest name a stamp may have shrinks to fit: every letter of its
/// label is drawn inside the box (the form clips the rest away).
#[test]
fn a_long_stamp_name_stays_inside_its_box() {
    let name: Vec<u8> = b"VeryLongName".iter().copied().cycle().take(127).collect();
    let form = appearance::stamp(&name, None, 1.0, appearance::IDENTITY);
    checked(&form.content, "127 bytes");
    let (pdf, rect) = form_at_size(&form);
    let mut label = "VERY LONG NAME ".repeat(10);
    label.push_str("VERY LON");
    assert_eq!(text_of(&pdf), label);
    for (line, origins) in text_lines(&pdf) {
        for o in origins {
            let y = 792.0 - o[1];
            assert!(
                o[0] >= rect[0] && o[0] <= rect[2] && y >= rect[1] && y <= rect[3],
                "{line}: {o:?} outside {rect:?}"
            );
        }
    }
    assert_ink_inside(&render(&pdf, 2.0), rect, 2.0, "127 bytes");
}

#[test]
fn a_stamp_box_is_wider_than_tall_and_takes_the_matrix() {
    let turn = [0.0, 1.0, -1.0, 0.0, 0.0, 0.0];
    let form = appearance::stamp(b"Approved", Some(&rgb(0.0, 0.0, 1.0)), 1.0, turn);
    assert_eq!(form.matrix, turn);
    let [x0, y0, x1, y1] = form.bbox;
    assert_eq!((x0, y0), (0.0, 0.0));
    let aspect = (x1 - x0) / (y1 - y0);
    assert!((2.0..=5.0).contains(&aspect), "{:?}", form.bbox);
    // Drawn upright: label and border in /C.
    let upright = appearance::stamp(
        b"Approved",
        Some(&rgb(0.0, 0.0, 1.0)),
        1.0,
        appearance::IDENTITY,
    );
    let (pdf, rect) = form_at_size(&upright);
    let image = render(&pdf, 2.0);
    assert_ink_inside(&image, rect, 2.0, "stamp");
    let ink = ink_box(&image).unwrap();
    let r = pixels_of(rect, 2.0);
    assert!(
        f64::from(ink[2] - ink[0]) > (r[2] - r[0]) * 0.8,
        "the design fills its box across: {ink:?} in {r:?}"
    );
    for y in ink[1]..=ink[3] {
        for x in ink[0]..=ink[2] {
            let p = pixel(&image, x, y);
            assert!(p[2] >= 240 || !inked(p), "blue only: {p:?}");
        }
    }
}

#[test]
fn a_stamp_label_is_set_in_helvetica_bold() {
    let form = appearance::stamp(b"Approved", None, 1.0, appearance::IDENTITY);
    let fonts = form
        .resources
        .get(b"Font".as_slice())
        .and_then(Object::as_dict)
        .expect("a /Font resource");
    assert!(
        fonts.values().any(|font| font
            .as_dict()
            .and_then(|f| f.get(b"BaseFont".as_slice()))
            .and_then(Object::as_name)
            == Some(b"Helvetica-Bold".as_slice())),
        "{fonts:?}"
    );
}

// ================================================================== 9.3 (3) icon_fit

#[test]
fn icon_fit_keeps_the_stamp_aspect_centred_inside_the_rect() {
    let bbox = appearance::stamp(b"Draft", None, 1.0, appearance::IDENTITY).bbox;
    let aspect = (bbox[2] - bbox[0]) / (bbox[3] - bbox[1]);
    for rect in [
        [0.0, 0.0, 160.0, 100.0],
        [10.0, 10.0, 410.0, 60.0],
        [100.0, 400.0, 300.0, 600.0],
        [-50.0, 20.0, 30.0, 300.0],
    ] {
        let fit = appearance::icon_fit(rect);
        let (w, h) = (fit[2] - fit[0], fit[3] - fit[1]);
        assert!((w / h - aspect).abs() < 1e-6, "{rect:?} -> {fit:?}");
        assert!(contains(rect, fit, 1e-9), "{rect:?} -> {fit:?}");
        let centre = |r: [f64; 4]| ((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0);
        let (a, b) = (centre(rect), centre(fit));
        assert!((a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9);
        // The largest such box: it touches two opposite sides.
        let touches_x = (w - (rect[2] - rect[0])).abs() < 1e-9;
        let touches_y = (h - (rect[3] - rect[1])).abs() < 1e-9;
        assert!(touches_x || touches_y, "{rect:?} -> {fit:?}");
    }
    // A 160 x 100 request keeps its width (the aspect is at least 2:1).
    let fit = appearance::icon_fit([0.0, 0.0, 160.0, 100.0]);
    assert!((fit[2] - fit[0] - 160.0).abs() < 1e-9 && fit[3] - fit[1] < 100.0);
    assert_eq!(
        appearance::icon_fit([5.0, 5.0, 5.0, 9.0]),
        [5.0, 7.0, 5.0, 7.0]
    );
    for rect in [
        [-1e30, -1e30, 1e30, 1e30],
        [1e30, 1e30, -1e30, -1e30],
        [0.0, 0.0, 1e300, 1e300],
    ] {
        let fit = appearance::icon_fit(rect);
        assert!(fit.iter().all(|v| v.is_finite()), "{rect:?} -> {fit:?}");
    }
}

// ================================================================== 9.3 (4) hostile geometry

fn assert_bounded(form: &Form, rect: [f64; 4], label: &str) {
    assert!(finite_rect(rect), "{label}: {rect:?}");
    assert!(
        form.bbox.iter().all(|v| v.is_finite()),
        "{label}: {:?}",
        form.bbox
    );
    assert!(
        form.content.len() < 4 << 20,
        "{label}: {}",
        form.content.len()
    );
    checked(&form.content, label);
}

#[test]
fn hostile_geometry_stays_finite_and_bounded() {
    let big = 1e300;
    let nan = f64::INFINITY - f64::INFINITY;
    let inf = big * 1e10;
    let quads: Vec<Quad> = vec![
        quad(-big, -big, big, big),
        [(nan, 1.0), (2.0, nan), (inf, -inf), (0.0, 0.0)],
        [(5.0, 5.0); 4],
        quad(10.0, 10.0, 10.0, 20.0),
        quad(10.0, 10.0, 20.0, 10.0),
    ];
    let style = Style {
        stroke: Some(rgb(1.0, 0.0, 0.0)),
        fill: Some(rgb(0.0, 0.0, 1.0)),
        width: big,
        opacity: nan,
        dash: vec![3.0],
    };
    let points: Vec<Point> = vec![(-big, big), (nan, 0.0), (inf, inf), (1.0, 2.0)];
    for (index, q) in quads.iter().enumerate() {
        let one = std::slice::from_ref(q);
        let (form, rect) = appearance::highlight(one, None, nan);
        assert_bounded(&form, rect, &format!("highlight {index}"));
        for mark in [Mark::Underline, Mark::StrikeOut, Mark::Squiggly] {
            let (form, rect) = appearance::text_line(mark, one, None, 1.0);
            assert_bounded(&form, rect, &format!("{mark:?} {index}"));
        }
        let (form, rect) = appearance::redact([0.0, 0.0, 10.0, 10.0], one, None);
        assert_bounded(&form, rect, &format!("redact {index}"));
    }
    let (form, rect) = appearance::highlight(&quads, None, 1.0);
    assert_bounded(&form, rect, "highlight all");
    for ending in [b"ClosedArrow".as_slice(), b"Circle", b"Slash"] {
        let (form, rect) = appearance::line((-big, nan), (big, inf), [ending, ending], &style);
        assert_bounded(&form, rect, "line");
        let (form, rect) = appearance::line((1.0, 1.0), (1.0, 1.0), [ending, ending], &style);
        assert_bounded(&form, rect, "zero-length line");
    }
    for closed in [true, false] {
        let (form, rect) = appearance::poly(&points, closed, &style);
        assert_bounded(&form, rect, "poly");
    }
    let (form, rect) = appearance::ink(&[points.clone(), vec![(3.0, 3.0)]], &style);
    assert_bounded(&form, rect, "ink");
    let form = appearance::rectangle([-big, nan, big, inf], [0.0, 0.0, 1.0, 1.0], true, &style);
    assert_bounded(&form, [0.0, 0.0, 1.0, 1.0], "circle");
    let fit = appearance::icon_fit([nan, 0.0, big, inf]);
    assert!(fit.iter().all(|v| v.is_finite()), "{fit:?}");
    let mut text = free_text_box("hostile", inf);
    text.border = big;
    let form = appearance::free_text([0.0, 0.0, 100.0, 100.0], [nan, 0.0, big, 100.0], &text);
    assert_bounded(&form, [0.0, 0.0, 100.0, 100.0], "free text");
}

/// Kinds the view synthesizes, with hostile numbers in their geometry,
/// draw (or not) without panicking, and `annot-set` stores finite
/// appearances for the editable ones.
#[test]
fn hostile_annotations_without_an_appearance_stay_finite() {
    let annots = [
        "<< /Type /Annot /Subtype /Highlight /Rect [0 0 1 1] /QuadPoints [-1e300 1e300 1e300 1e300 -1e300 -1e300 1e300 -1e300 5 5 5 5 5 5 5 5] >>",
        "<< /Type /Annot /Subtype /Underline /Rect [0 0 1 1] /QuadPoints [1 2 1 2 1 2 1 2] >>",
        "<< /Type /Annot /Subtype /Squiggly /Rect [0 0 1 1] /QuadPoints [0 1e300 1e300 1e300 0 0 1e300 0] >>",
        "<< /Type /Annot /Subtype /Square /Rect [-1e300 -1e300 1e300 1e300] /BS << /W 1e300 >> >>",
        "<< /Type /Annot /Subtype /Circle /Rect [0 0 1e300 1e-300] >>",
        "<< /Type /Annot /Subtype /Line /Rect [0 0 1 1] /L [1e300 0 -1e300 1e300] /LE [/ClosedArrow /Circle] >>",
        "<< /Type /Annot /Subtype /Ink /Rect [0 0 1 1] /InkList [[1e300 1e300 -1e300 -1e300] [5 5]] /BS << /W 1e300 >> >>",
        "<< /Type /Annot /Subtype /Polygon /Rect [0 0 1 1] /Vertices [1e300 0 0 1e300 -1e300 -1e300] >>",
        "<< /Type /Annot /Subtype /Stamp /Rect [-1e300 0 1e300 1e-300] /Name /Approved >>",
        "<< /Type /Annot /Subtype /Caret /Rect [0 0 1e300 1e300] >>",
        "<< /Type /Annot /Subtype /Redact /Rect [0 0 1 1] /QuadPoints [0 1e300 1e300 1e300 0 0 1e300 0] >>",
        "<< /Type /Annot /Subtype /FreeText /Rect [0 0 1e300 1e300] /DA (/Helv 1e300 Tf 0 g) /Contents (x) >>",
        "<< /Type /Annot /Subtype /Text /Rect [1e300 1e300 1e300 1e300] >>",
    ];
    let (mut engine, mut session) = session_with(page_with("", "", &annots));
    let _ = view_image(&mut session, 0.25);
    let listed = items(&mut engine, &mut session);
    for item in &listed {
        let rect = rect_of(item);
        assert!(rect.iter().all(|v| v.is_finite()), "listed {item}");
        if item["editable"] != true {
            continue;
        }
        let number = item["id"].as_u64().unwrap() as u32;
        op(
            &mut engine,
            &mut session,
            json!({"op": "annot-set", "page": 0, "annotId": number, "contents": "x"}),
        );
        let dict = annot(&session, number);
        let doc = document(&session);
        let rect = annots::numbers(doc, dict.get(b"Rect".as_slice()).unwrap()).unwrap();
        assert!(rect.iter().all(|v| v.is_finite()), "{item}: {rect:?}");
        if let Some((_, content)) = stored(&session, number) {
            assert!(content.len() < 4 << 20, "{item}");
            checked(&content, &item.to_string());
        }
    }
    let _ = view_image(&mut session, 0.25);
}

// ================================================================== 9.3 (5) line endings

const ENDINGS: [&[u8]; 10] = [
    b"Square",
    b"Circle",
    b"Diamond",
    b"OpenArrow",
    b"ClosedArrow",
    b"None",
    b"Butt",
    b"ROpenArrow",
    b"RClosedArrow",
    b"Slash",
];

/// A horizontal line from (200, 400) to (300, 400) with `ending` at its
/// right end, drawn at 4 px per point: the image and the /Rect.
fn line_drawn(ending: &[u8], width: f64, fill: Option<Color>) -> (RgbaImage, [f64; 4]) {
    let style = Style {
        stroke: Some(rgb(1.0, 0.0, 0.0)),
        fill,
        width,
        opacity: 1.0,
        dash: Vec::new(),
    };
    let (form, rect) = appearance::line((200.0, 400.0), (300.0, 400.0), [b"None", ending], &style);
    checked(&form.content, &String::from_utf8_lossy(ending));
    assert_eq!(form.bbox, rect);
    assert_eq!(form.matrix, appearance::IDENTITY);
    (render(&form_page(&form, rect), 4.0), rect)
}

/// Inked pixels farther than `band` points from the line's centre line,
/// or past its ends, near end `x_end` (within 40 pt).
fn ink_off_the_line(image: &RgbaImage, x_end: f64, band: f64) -> usize {
    let mut n = 0;
    for y in 0..image.height {
        for x in 0..image.width {
            let (ux, uy) = (f64::from(x) / 4.0, 792.0 - f64::from(y) / 4.0);
            if (ux - x_end).abs() > 40.0 || !inked(pixel(image, x, y)) {
                continue;
            }
            if (uy - 400.0).abs() > band || ux > 300.0 + band || ux < 200.0 - band {
                n += 1;
            }
        }
    }
    n
}

#[test]
fn line_endings_draw_past_the_bare_line_inside_the_rect() {
    for width in [1.0, 5.0] {
        let band = width / 2.0 + 0.5;
        for ending in ENDINGS {
            let label = format!("{} at {width}", String::from_utf8_lossy(ending));
            let (image, rect) = line_drawn(ending, width, None);
            assert_ink_inside(&image, rect, 4.0, &label);
            assert!(contains(rect, [200.0, 400.0, 300.0, 400.0], 0.0), "{label}");
            let past = ink_off_the_line(&image, 300.0, band);
            if ending == b"None" {
                assert_eq!(past, 0, "{label}");
            } else {
                assert!(past > 0, "{label}: no ending drawn");
            }
            // The other end has none.
            assert_eq!(ink_off_the_line(&image, 200.0, band), 0, "{label}");
        }
    }
    // An unknown name, and none, draw no ending.
    let (plain, _) = line_drawn(b"None", 1.0, None);
    for ending in [b"Foo".as_slice(), b""] {
        let (image, _) = line_drawn(ending, 1.0, None);
        assert_eq!(ink_off_the_line(&image, 300.0, 1.0), 0, "{ending:?}");
        assert_eq!(ink_box(&image), ink_box(&plain), "{ending:?}");
    }
}

fn blue_pixels(image: &RgbaImage) -> usize {
    let mut n = 0;
    for y in 0..image.height {
        for x in 0..image.width {
            let p = pixel(image, x, y);
            if p[2] > 200 && p[0] < 100 && p[1] < 100 {
                n += 1;
            }
        }
    }
    n
}

#[test]
fn a_closed_arrow_takes_the_interior_colour_and_an_open_one_does_not() {
    let blue = Some(rgb(0.0, 0.0, 1.0));
    for width in [1.0, 5.0] {
        let (closed, _) = line_drawn(b"ClosedArrow", width, blue.clone());
        assert!(blue_pixels(&closed) > 0, "ClosedArrow at {width} is filled");
        let (open, _) = line_drawn(b"OpenArrow", width, blue.clone());
        assert_eq!(blue_pixels(&open), 0, "OpenArrow at {width} is not");
    }
}

#[test]
fn a_zero_length_line_draws_its_endings_without_nan() {
    let style = Style {
        stroke: Some(rgb(1.0, 0.0, 0.0)),
        ..Style::default()
    };
    let (form, rect) = appearance::line(
        (300.0, 400.0),
        (300.0, 400.0),
        [b"Circle", b"OpenArrow"],
        &style,
    );
    checked(&form.content, "zero length");
    assert!(finite_rect(rect), "{rect:?}");
    let image = render(&form_page(&form, rect), 4.0);
    assert_ink_inside(&image, rect, 4.0, "zero length");
}

// ================================================================== 9.3 (6) border width 0

fn shape(extra: &str) -> RgbaImage {
    let annot = format!("<< /Type /Annot /Subtype /Square /Rect [100 600 200 700] {extra} >>");
    let (_, mut session) = session_with(page_with("", "", &[&annot]));
    view_image(&mut session, 2.0)
}

fn red_pixels(image: &RgbaImage) -> usize {
    let mut n = 0;
    for y in 0..image.height {
        for x in 0..image.width {
            let p = pixel(image, x, y);
            if p[0] > 150 && p[1] < 150 && p[2] < 150 {
                n += 1;
            }
        }
    }
    n
}

#[test]
fn a_border_width_of_zero_draws_the_fill_and_no_border() {
    let image = shape("/C [1 0 0] /IC [0 0 1] /BS << /W 0 >>");
    assert!(blue_pixels(&image) > 150 * 150, "the fill draws");
    assert_eq!(red_pixels(&image), 0, "no border");
    let ink = ink_box(&image).unwrap();
    for y in ink[1]..=ink[3] {
        for x in ink[0]..=ink[2] {
            let p = pixel(&image, x, y);
            assert!(
                luminance(p) >= luminance([0, 0, 255]) - 1.0,
                "nothing darker than the fill at {x},{y}: {p:?}"
            );
        }
    }
}

#[test]
fn a_negative_border_width_draws_as_width_one() {
    let one = shape("/C [1 0 0] /BS << /W 1 >>");
    assert!(red_pixels(&one) > 0);
    assert!(
        shape("/C [1 0 0] /BS << /W -3 >>").pixels == one.pixels,
        "-3"
    );
    assert!(
        shape("/C [1 0 0] /BS << /W /Thick >>").pixels == one.pixels,
        "a name"
    );
}

/// Owner decision A6: with /BS /W 0 a Line, an Ink and a PolyLine draw
/// nothing.
#[test]
fn a_zero_width_line_ink_or_polyline_draws_nothing() {
    for annot in [
        "<< /Type /Annot /Subtype /Line /Rect [90 590 310 710] /L [100 600 300 700] /C [1 0 0] /BS << /W 0 >> >>",
        "<< /Type /Annot /Subtype /Ink /Rect [90 590 310 710] /InkList [[100 600 200 700 300 600]] /C [1 0 0] /BS << /W 0 >> >>",
        "<< /Type /Annot /Subtype /PolyLine /Rect [90 590 310 710] /Vertices [100 600 200 700 300 600] /C [1 0 0] /BS << /W 0 >> >>",
    ] {
        let (_, mut session) = session_with(page_with("", "", &[annot]));
        assert_eq!(ink_box(&view_image(&mut session, 2.0)), None, "{annot}");
    }
}

/// Owner decision A6: an Ink stays inside its /Rect, sharp turns and wide
/// pens included (curves through the points must not overshoot it).
#[test]
fn an_ink_stays_inside_its_rect() {
    let annot = "<< /Type /Annot /Subtype /Ink /Rect [0 0 1 1] /C [0 0 1] /BS << /W 6 >> \
                 /InkList [[100 600 110 700 120 600 130 700 140 600 300 610 100 620] [150 650]] >>";
    let (mut engine, mut session) = session_with(page_with("", "", &[annot]));
    op(
        &mut engine,
        &mut session,
        json!({"op": "annot-set", "page": 0, "annotId": 5, "contents": "ink"}),
    );
    let doc = document(&session);
    let dict = annot_dict(&session);
    let rect = annots::rect(doc, dict.get(b"Rect".as_slice()).unwrap()).unwrap();
    let rd = annots::numbers(doc, dict.get(b"RD".as_slice()).unwrap()).unwrap();
    let allowance = appearance::ink_allowance(6.0);
    assert!(allowance >= 3.0);
    for (got, want) in rect
        .iter()
        .zip(appearance::grow([100.0, 600.0, 300.0, 700.0], allowance))
    {
        assert!((got - want).abs() < 1e-3, "{rect:?}");
    }
    assert!(rd.iter().all(|v| (v - allowance).abs() < 1e-3), "{rd:?}");
    let (_, content) = stored(&session, 5).unwrap();
    checked(&content, "ink");
    let image = view_image(&mut session, 2.0);
    assert_ink_inside(&image, rect, 2.0, "ink");
}

fn annot_dict(session: &AppSession) -> Dictionary {
    annot(session, 5)
}

// ================================================================== A5 colours

/// Spec decision A5: a shape or stroke-only mark without a usable /C stays
/// visible in black, and a Highlight without one is a light colour.
#[test]
fn marks_without_a_colour_stay_visible() {
    for annot in [
        "<< /Type /Annot /Subtype /Square /Rect [100 600 200 700] >>",
        "<< /Type /Annot /Subtype /Circle /Rect [100 600 200 700] /C [1 0] >>",
        "<< /Type /Annot /Subtype /Line /Rect [90 590 310 710] /L [100 600 300 700] >>",
        "<< /Type /Annot /Subtype /PolyLine /Rect [90 590 310 710] /Vertices [100 600 200 700 300 600] >>",
        "<< /Type /Annot /Subtype /Ink /Rect [90 590 310 710] /InkList [[100 600 200 700 300 600]] >>",
        "<< /Type /Annot /Subtype /Underline /Rect [90 590 310 710] /QuadPoints [100 620 300 620 100 600 300 600] >>",
        "<< /Type /Annot /Subtype /Caret /Rect [100 600 200 700] >>",
    ] {
        let (_, mut session) = session_with(page_with("", "", &[annot]));
        let image = view_image(&mut session, 2.0);
        let ink = ink_box(&image).unwrap_or_else(|| panic!("{annot}: nothing drawn"));
        let darkest = (ink[1]..=ink[3])
            .flat_map(|y| (ink[0]..=ink[2]).map(move |x| (x, y)))
            .map(|(x, y)| pixel(&image, x, y))
            .min_by(|a, b| luminance(*a).total_cmp(&luminance(*b)))
            .unwrap();
        assert!(
            darkest.iter().all(|&c| c < 64),
            "{annot}: black, not {darkest:?}"
        );
    }
    let (_, mut session) = session_with(page_with(
        "",
        "",
        &[
            "<< /Type /Annot /Subtype /Highlight /Rect [90 590 310 710] /QuadPoints [100 620 300 620 100 600 300 600] >>",
        ],
    ));
    let image = view_image(&mut session, 2.0);
    let p = pixel(&image, 400, (792 - 610) * 2);
    assert!(inked(p) && luminance(p) > 128.0, "a light highlight: {p:?}");
}

// ================================================================== A6 empty geometry

#[test]
fn marks_without_geometry_get_no_appearance_and_keep_their_rect() {
    for annot in [
        "<< /Type /Annot /Subtype /Highlight /Rect [10 20 30 40] /C [1 1 0] >>",
        "<< /Type /Annot /Subtype /Underline /Rect [10 20 30 40] /QuadPoints [] >>",
        "<< /Type /Annot /Subtype /Ink /Rect [10 20 30 40] /InkList [] >>",
        "<< /Type /Annot /Subtype /Ink /Rect [10 20 30 40] /InkList [[]] >>",
    ] {
        let (mut engine, mut session) = session_with(page_with("", "", &[annot]));
        assert_eq!(ink_box(&view_image(&mut session, 1.0)), None, "{annot}");
        op(
            &mut engine,
            &mut session,
            json!({"op": "annot-set", "page": 0, "annotId": 5, "contents": "x"}),
        );
        let dict = annot_dict(&session);
        assert!(!dict.contains_key(b"AP".as_slice()), "{annot}");
        let doc = document(&session);
        assert_eq!(
            annots::rect(doc, dict.get(b"Rect".as_slice()).unwrap()),
            Some([10.0, 20.0, 30.0, 40.0]),
            "{annot}"
        );
    }
    for annot in [
        "<< /Type /Annot /Subtype /Polygon /Rect [10 20 30 40] /Vertices [15 25] /C [1 0 0] /BS << /W 5 >> >>",
        "<< /Type /Annot /Subtype /PolyLine /Rect [10 20 30 40] /Vertices [] /C [1 0 0] >>",
    ] {
        let (_, mut session) = session_with(page_with("", "", &[annot]));
        assert_eq!(ink_box(&view_image(&mut session, 1.0)), None, "{annot}");
    }
}

// ================================================================== A5 owner: icons

/// Owner decision A5: FileAttachment and Sound annotations without an
/// appearance are drawn with an icon inside their /Rect.
#[test]
fn file_attachments_and_sounds_without_an_appearance_draw_an_icon() {
    for annot in [
        "<< /Type /Annot /Subtype /FileAttachment /Rect [100 600 120 620] /C [0 0 1] >>",
        "<< /Type /Annot /Subtype /Sound /Rect [100 600 120 620] /C [0 0 1] >>",
    ] {
        let (_, mut session) = session_with(page_with("", "", &[annot]));
        let image = view_image(&mut session, 4.0);
        assert_ink_inside(&image, [100.0, 600.0, 120.0, 620.0], 4.0, annot);
    }
}

// ================================================================== A3 caret

/// Spec decision A3: a Caret without an appearance fills its /Rect less /RD.
#[test]
fn a_caret_fills_its_rect() {
    for (annot, inner) in [
        (
            "<< /Type /Annot /Subtype /Caret /Rect [100 600 200 700] /C [0 0 1] >>",
            [100.0, 600.0, 200.0, 700.0],
        ),
        (
            // /RD is left, top, right, bottom (ISO 32000-2 Table 183).
            "<< /Type /Annot /Subtype /Caret /Rect [100 600 200 700] /RD [10 20 30 5] /C [0 0 1] >>",
            [110.0, 605.0, 170.0, 680.0],
        ),
    ] {
        let (mut engine, mut session) = session_with(page_with("", "", &[annot]));
        let listed = rect_of(&items(&mut engine, &mut session)[0]);
        assert_eq!(listed, [100.0, 92.0, 200.0, 192.0], "{annot}");
        let image = view_image(&mut session, 2.0);
        assert_ink_inside(&image, inner, 2.0, annot);
        let ink = ink_box(&image).unwrap();
        let r = pixels_of(inner, 2.0);
        assert!(
            f64::from(ink[2] - ink[0]) >= (r[2] - r[0]) * 0.8,
            "{annot}: {ink:?}"
        );
        assert!(
            f64::from(ink[3] - ink[1]) >= (r[3] - r[1]) * 0.6,
            "{annot}: {ink:?}"
        );
    }
    let form = appearance::caret(None, 1.0);
    checked(&form.content, "caret");
    let (pdf, rect) = form_at_size(&form);
    assert_ink_inside(&render(&pdf, 4.0), rect, 4.0, "caret");
}

// ================================================================== 9.3 (7) highlight

#[test]
fn black_text_under_a_highlight_stays_dark() {
    let content = "BT /F1 24 Tf 100 600 Td (HHHH) Tj ET";
    let font = "/Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >>";
    let highlight = "<< /Type /Annot /Subtype /Highlight /Rect [0 0 1 1] /C [1 1 0] \
                     /QuadPoints [95 625 175 625 95 595 175 595] >>";
    let (_, mut session) = session_with(page_with(font, content, &[highlight]));
    let image = view_image(&mut session, 2.0);
    // Inside the text's box: the darkest pixel is still dark, and the
    // highlight shows yellow around the letters.
    let area = [200, (792 - 620) * 2, 340, (792 - 600) * 2];
    let mut darkest = 255.0f64;
    for y in area[1]..area[3] {
        for x in area[0]..area[2] {
            darkest = darkest.min(luminance(pixel(&image, x, y)));
        }
    }
    assert!(darkest < 128.0, "{darkest}");
    let around = pixel(&image, 195, (792 - 597) * 2);
    assert!(
        around[0] > 200 && around[1] > 200 && around[2] < 100,
        "{around:?}"
    );
}

#[test]
fn a_highlight_multiplies_and_covers_its_quads() {
    let quads = [
        quad(100.0, 600.0, 200.0, 620.0),
        // A turned quad: rotated text.
        [
            (300.0, 640.0),
            (360.0, 700.0),
            (310.0, 630.0),
            (370.0, 690.0),
        ],
    ];
    let (form, rect) = appearance::highlight(&quads, Some(&rgb(1.0, 1.0, 0.0)), 1.0);
    checked(&form.content, "highlight");
    assert_eq!(form.bbox, rect);
    let all = appearance::bounds(quads.iter().flatten().copied()).unwrap();
    assert!(contains(rect, all, 1e-9), "{rect:?}");
    // Multiply, through an ExtGState the content selects.
    let states = form
        .resources
        .get(b"ExtGState".as_slice())
        .and_then(Object::as_dict)
        .expect("an ExtGState");
    assert!(states.values().any(|s| {
        s.as_dict()
            .and_then(|s| s.get(b"BM".as_slice()))
            .and_then(Object::as_name)
            == Some(b"Multiply".as_slice())
    }));
    let ops = operations(&form.content);
    assert!(count(&ops, &["gs"]) >= 1);
    let image = render(&form_page(&form, rect), 2.0);
    assert_ink_inside(&image, rect, 2.0, "highlight");
    // The quads' middles are covered.
    for (x, y) in [(150.0, 610.0), (335.0, 665.0)] {
        let p = pixel(&image, (x * 2.0) as u32, ((792.0 - y) * 2.0) as u32);
        assert!(p[0] > 200 && p[1] > 200 && p[2] < 100, "{x},{y}: {p:?}");
    }
}

// ================================================================== 3.7 text lines

#[test]
fn underline_and_strike_out_sit_in_their_bands() {
    // A 20 pt tall quad: x-height band and middle band.
    let q = quad(100.0, 600.0, 300.0, 620.0);
    for (mark, low, high) in [
        (Mark::Underline, 600.0, 610.0),
        (Mark::StrikeOut, 606.0, 610.0),
    ] {
        let (form, rect) = appearance::text_line(mark, &[q], Some(&rgb(1.0, 0.0, 0.0)), 1.0);
        let ops = checked(&form.content, &format!("{mark:?}"));
        assert!(count(&ops, &["S", "s", "B", "b", "f", "F", "B*", "b*", "f*"]) >= 1);
        assert!(
            contains(rect, [100.0, 600.0, 300.0, 620.0], 1e-9),
            "{rect:?}"
        );
        let image = render(&form_page(&form, rect), 4.0);
        assert_ink_inside(&image, rect, 4.0, &format!("{mark:?}"));
        // Every inked row is in the band, and the stroke spans the quad.
        let ink = ink_box(&image).unwrap();
        let top = 792.0 - f64::from(ink[1]) / 4.0;
        let bottom = 792.0 - f64::from(ink[3] + 1) / 4.0;
        let middle = (top + bottom) / 2.0;
        assert!(
            (low..=high).contains(&middle),
            "{mark:?}: stroke centre at {middle}"
        );
        assert!(
            f64::from(ink[2] - ink[0]) / 4.0 >= 190.0,
            "{mark:?}: {ink:?}"
        );
        assert!(bottom >= 599.0, "{mark:?}: above the quad's bottom edge");
    }
}

// ================================================================== 9.2 squiggly bound (unit)

#[test]
fn a_squiggly_waves_along_its_quad_and_stays_bounded() {
    let (form, rect) = appearance::text_line(
        Mark::Squiggly,
        &[quad(100.0, 600.0, 240.0, 614.0)],
        None,
        1.0,
    );
    let ops = checked(&form.content, "squiggly");
    let vertices = count(&ops, &["m", "l", "c", "v", "y"]);
    assert!((10..=200).contains(&vertices), "{vertices}");
    let image = render(&form_page(&form, rect), 4.0);
    assert_ink_inside(&image, rect, 4.0, "squiggly");
    let ink = ink_box(&image).unwrap();
    assert!(f64::from(ink[2] - ink[0]) / 4.0 >= 0.9 * 140.0, "{ink:?}");
    // 400 quads 590 pt long and 0.001 pt tall: bounded content, every quad
    // still waved.
    let thin: Vec<Quad> = (0..400)
        .map(|i| {
            let y = 100.0 + f64::from(i);
            quad(10.0, y, 600.0, y + 0.001)
        })
        .collect();
    let (form, rect) = appearance::text_line(Mark::Squiggly, &thin, None, 1.0);
    assert!(form.content.len() < 2 << 20, "{}", form.content.len());
    assert!(finite_rect(rect));
    let ops = checked(&form.content, "thin");
    assert!(count(&ops, &["m"]) >= 400 || count(&ops, &["m", "l"]) >= 800);
}

/// The app's bound: a Squiggly's vertices over all its quads are at most
/// 32,768, every quad still waved.
#[test]
fn a_squiggly_has_at_most_32768_vertices() {
    let thin: Vec<Quad> = (0..400)
        .map(|i| {
            let y = 100.0 + f64::from(i);
            quad(10.0, y, 600.0, y + 0.001)
        })
        .collect();
    let (form, _) = appearance::text_line(Mark::Squiggly, &thin, None, 1.0);
    let ops = checked(&form.content, "thin");
    let vertices = count(&ops, &["m", "l", "c", "v", "y"]);
    assert!(vertices <= 32_768, "{vertices}");
    assert!(vertices >= 3 * 400, "{vertices}: every quad waves");
}

// ================================================================== 9.2 view bounds

/// A Highlight without an appearance whose /QuadPoints hold 16,884 small
/// quads in a grid: the view draws the first 16,384 and no more.
#[test]
fn a_synthesized_highlight_draws_at_most_16384_quads() {
    let (columns, total) = (200, 16_884);
    let cell = |i: usize| {
        let (col, row) = ((i % columns) as f64, (i / columns) as f64);
        (6.0 + col * 3.0, 780.0 - row * 3.0)
    };
    let mut numbers = String::new();
    for i in 0..total {
        let (x, y) = cell(i);
        numbers.push_str(&format!(
            "{} {} {} {} {} {} {} {} ",
            x,
            y + 1.0,
            x + 1.0,
            y + 1.0,
            x,
            y,
            x + 1.0,
            y
        ));
    }
    let annot = format!(
        "<< /Type /Annot /Subtype /Highlight /Rect [0 0 1 1] /C [0 0 1] /QuadPoints [{numbers}] >>"
    );
    let (_, mut session) = session_with(page_with("", "", &[&annot]));
    let image = view_image(&mut session, 4.0);
    for i in 0..total {
        let (x, y) = cell(i);
        let p = pixel(
            &image,
            ((x + 0.5) * 4.0) as u32,
            ((792.0 - y - 0.5) * 4.0) as u32,
        );
        assert_eq!(inked(p), i < 16_384, "quad {i}: {p:?}");
    }
}

/// An Ink without an appearance whose two strokes hold 131,072 points
/// each: `annot-set` stores an appearance through 131,072 of them.
#[test]
fn a_synthesized_ink_uses_at_most_131072_points() {
    let stroke: Vec<String> = (0..131_072)
        .map(|i| format!("{} {}", 50 + i % 500, 100 + (i / 500) % 600))
        .collect();
    let stroke = stroke.join(" ");
    let annot = format!(
        "<< /Type /Annot /Subtype /Ink /Rect [0 0 1 1] /C [0 0 1] /InkList [[{stroke}] [{stroke}]] >>"
    );
    let (mut engine, mut session) = session_with(page_with("", "", &[&annot]));
    op(
        &mut engine,
        &mut session,
        json!({"op": "annot-set", "page": 0, "annotId": 5, "contents": "x"}),
    );
    let (_, content) = stored(&session, 5).expect("an appearance");
    let ops = checked(&content, "ink");
    // Each point a path passes through ends one construction operator.
    assert_eq!(count(&ops, &["m", "l", "c", "v", "y"]), 131_072);
}

// ================================================================== 9.3 (8) FreeText

#[test]
fn line_height_is_one_factor_of_the_size() {
    let factor = appearance::line_height(1.0);
    assert!(factor > 0.0 && factor.is_finite());
    for size in [0.5, 6.0, 12.0, 36.0, 1000.0] {
        let lh = appearance::line_height(size);
        assert!((lh - factor * size).abs() < 1e-9 * size, "{size}: {lh}");
    }
}

#[test]
fn the_first_line_starts_inside_the_box_top() {
    for size in [6.0, 12.0, 36.0] {
        let rect = [100.0, 300.0, 500.0, 600.0];
        let form = appearance::free_text(rect, rect, &free_text_box("Hg ÉÅ", size));
        checked(&form.content, &format!("{size}"));
        assert_eq!(form.bbox, rect);
        assert_eq!(form.matrix, appearance::IDENTITY);
        let image = render(&form_page(&form, rect), 4.0);
        assert_ink_inside(&image, rect, 4.0, &format!("{size}"));
        let ink = ink_box(&image).unwrap();
        // The first line is near the top: its ink starts within two line
        // heights of it.
        let top = (792.0 - 600.0) * 4.0;
        let below = (f64::from(ink[1]) - top) / 4.0;
        assert!(
            below >= 0.0 && below < 2.0 * appearance::line_height(size),
            "{size}: {below}"
        );
    }
}

#[test]
fn a_long_word_wraps_inside_the_box() {
    let word = "abcdefghij".repeat(30);
    let rect = [100.0, 200.0, 260.0, 700.0];
    let form = appearance::free_text(rect, rect, &free_text_box(&word, 12.0));
    checked(&form.content, "long word");
    let pdf = form_page(&form, rect);
    assert_ink_inside(&render(&pdf, 2.0), rect, 2.0, "long word");
    let lines = text_lines(&pdf);
    assert!(lines.len() > 1, "{} line(s)", lines.len());
    let joined: String = lines.iter().map(|(s, _)| s.trim()).collect();
    assert_eq!(joined, word);
}

#[test]
fn a_turned_free_text_reads_the_same_text() {
    let rect = [100.0, 300.0, 300.0, 500.0];
    for rotate in [0, 90, 180, 270] {
        let mut box_ = free_text_box("turned text", 12.0);
        box_.rotate = rotate;
        let form = appearance::free_text(rect, rect, &box_);
        checked(&form.content, &format!("{rotate}"));
        let pdf = form_page(&form, rect);
        assert_eq!(text_of(&pdf), "turned text", "/Rotate {rotate}");
        assert_ink_inside(&render(&pdf, 2.0), rect, 2.0, &format!("{rotate}"));
    }
}

fn font_resources(form: &Form) -> Vec<Vec<u8>> {
    form.resources
        .get(b"Font".as_slice())
        .and_then(Object::as_dict)
        .map(|fonts| fonts.keys().cloned().collect())
        .unwrap_or_default()
}

#[test]
fn a_no_break_space_and_a_soft_hyphen_stay_in_the_winansi_font() {
    let rect = [100.0, 300.0, 400.0, 500.0];
    let form = appearance::free_text(rect, rect, &free_text_box("nbsp\u{a0}x shy\u{ad}x", 12.0));
    checked(&form.content, "nbsp");
    assert_eq!(font_resources(&form), [b"Helv".to_vec()]);
}

fn free_text_pixels(text: &str) -> Vec<u8> {
    let rect = [100.0, 300.0, 400.0, 500.0];
    let form = appearance::free_text(rect, rect, &free_text_box(text, 12.0));
    render(&form_page(&form, rect), 2.0).pixels
}

#[test]
fn format_characters_draw_nothing_and_accents_compose() {
    let plain = free_text_pixels("ab cd");
    assert!(
        free_text_pixels("a\u{200b}b \u{feff}c\u{200d}d\u{2060}") == plain,
        "format"
    );
    assert!(
        free_text_pixels("Caf\u{e9}") == free_text_pixels("Cafe\u{301}"),
        "é"
    );
    assert!(
        free_text_pixels("\u{17c}") == free_text_pixels("z\u{307}"),
        "ż"
    );
}

#[test]
fn a_cjk_font_in_a_stored_form_is_three_objects() {
    let (mut engine, mut session) = session_with(blank());
    op(
        &mut engine,
        &mut session,
        json!({"op": "annot-add", "page": 0, "type": "FreeText", "point": [40, 60], "contents": "日本語"}),
    );
    let number = items(&mut engine, &mut session)[0]["id"].as_u64().unwrap() as u32;
    let (form, _) = stored(&session, number).unwrap();
    let doc = document(&session);
    let resources = doc.dict_get(&form, b"Resources").as_dict().unwrap();
    let fonts = doc.dict_get(resources, b"Font").as_dict().unwrap();
    let type0: Vec<ObjectId> = fonts
        .values()
        .filter_map(Object::as_reference)
        .filter(|&f| {
            doc.resolve_id(f)
                .as_dict()
                .is_some_and(|f| doc.dict_get(f, b"Subtype").as_name() == Some(b"Type0"))
        })
        .collect();
    assert_eq!(type0.len(), 1, "{fonts:?}");
    let font = doc.resolve_id(type0[0]).as_dict().unwrap();
    let descendant = doc.dict_get(font, b"DescendantFonts").as_array().unwrap()[0]
        .as_reference()
        .expect("an indirect CIDFont");
    let descendant = doc.resolve_id(descendant).as_dict().unwrap();
    let descriptor = descendant.get(b"FontDescriptor".as_slice()).unwrap();
    let descriptor = descriptor.as_reference().expect("an indirect descriptor");
    assert_eq!(
        doc.dict_get(doc.resolve_id(descriptor).as_dict().unwrap(), b"Type")
            .as_name(),
        Some(b"FontDescriptor".as_slice())
    );
    assert_eq!(text_of(&flattened(&saved(&session), b"FreeText")), "日本語");
}

#[test]
fn free_text_paints_background_then_border_then_text_inside_its_rect() {
    let bbox = [100.0, 300.0, 400.0, 500.0];
    let rect = [120.0, 320.0, 380.0, 480.0];
    let mut box_ = free_text_box("Inside", 12.0);
    box_.border = 3.0;
    box_.color = Some(rgb(1.0, 0.0, 0.0));
    box_.background = Some(rgb(0.0, 0.0, 1.0));
    box_.quadding = 1;
    let form = appearance::free_text(bbox, rect, &box_);
    checked(&form.content, "framed");
    assert_eq!(form.bbox, bbox);
    let pdf = form_page(&form, bbox);
    let image = render(&pdf, 2.0);
    assert_ink_inside(&image, rect, 2.0, "framed");
    // The middle is the background, the edge the border.
    let mid = pixel(&image, 500, (792 - 330) * 2);
    assert_eq!(mid, [0, 0, 255], "background");
    let edge = pixel(&image, 500, (792 - 479) * 2);
    assert!(edge[0] > 200 && edge[2] < 60, "border: {edge:?}");
    // Centred text: its middle near the rect's.
    let lines = text_lines(&pdf);
    assert_eq!(normalised(&lines[0].0), "Inside");
    let xs: Vec<f64> = lines[0].1.iter().map(|o| o[0]).collect();
    let first = xs[0];
    assert!(first > 120.0 + 40.0 && first < 250.0, "centred: {xs:?}");
}

// ================================================================== 9.3 (9) fit contract

struct Rng(u64);
impl Rng {
    fn next(&mut self, bound: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % bound.max(1)
    }
}

/// `free_text_fit`'s boxes (an `annot-add` with `fit`) draw every line of
/// random WinAnsi text whole, each baseline inside the box.
#[test]
fn fitted_boxes_draw_every_line_unwrapped() {
    let letters: Vec<char> =
        "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789WWMM.,;:!?@éüßÆœ€"
            .chars()
            .collect();
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    for round in 0..12 {
        let count = 1 + rng.next(5) as usize;
        let lines: Vec<String> = (0..count)
            .map(|_| {
                let words = 1 + rng.next(5);
                (0..words)
                    .map(|_| {
                        let len = 1 + rng.next(9);
                        (0..len)
                            .map(|_| letters[rng.next(letters.len() as u64) as usize])
                            .collect::<String>()
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect();
        let contents = lines.join("\n");
        let (mut engine, mut session) = session_with(blank());
        op(
            &mut engine,
            &mut session,
            json!({"op": "annot-add", "page": 0, "type": "FreeText", "point": [40, 300], "contents": contents, "fit": true}),
        );
        let rect = rect_of(&items(&mut engine, &mut session)[0]);
        let lh = appearance::line_height(12.0);
        assert!(
            (rect[3] - rect[1] - count as f64 * lh).abs() < 1e-6,
            "{round}: {rect:?}"
        );
        assert!(
            (rect[1] + lh / 2.0 - 300.0).abs() < 1e-6,
            "{round}: first-line centre"
        );
        let drawn = text_lines(&flattened(&saved(&session), b"FreeText"));
        let texts: Vec<String> = drawn.iter().map(|(s, _)| normalised(s)).collect();
        assert_eq!(texts, lines, "{round}");
        for (text, origins) in &drawn {
            for o in origins {
                assert!(
                    o[1] > rect[1] - 0.01 && o[1] < rect[3] + 0.01,
                    "{round}: {text} baseline {o:?} outside {rect:?}"
                );
            }
        }
    }
}

// ================================================================== 9.3 (10) fonts

#[test]
fn standard_widths_are_the_afm_widths() {
    let h = Base14::Helvetica;
    assert_eq!(h.advance(b' '), 278.0);
    assert_eq!(h.advance(b'A'), 667.0);
    assert_eq!(h.advance(b'W'), 944.0);
    assert_eq!(Base14::TimesRoman.advance(b'A'), 722.0);
    assert_eq!(Base14::TimesRoman.advance(b'a'), 444.0);
    assert_eq!(Base14::TimesBold.advance(b'A'), 722.0);
    assert_eq!(Base14::TimesBold.advance(b'a'), 500.0);
    for face in [
        Base14::Courier,
        Base14::CourierBold,
        Base14::CourierOblique,
        Base14::CourierBoldOblique,
    ] {
        for byte in [b' ', b'A', b'W', b'i', 0xE9, 0x80] {
            assert_eq!(face.advance(byte), 600.0, "{face:?} {byte}");
        }
    }
    assert!((h.width(b"Hello", 10.0) - 22.78).abs() < 1e-9);
    // Codes without a glyph: control codes and the undefined WinAnsi codes.
    for byte in [0x00, 0x09, 0x1F, 0x81, 0x8D, 0x8F, 0x90, 0x9D] {
        assert_eq!(h.advance(byte), 0.0, "{byte:#x}");
        assert_eq!(Base14::Courier.advance(byte), 0.0, "{byte:#x}");
    }
}

/// Owner decision A4 (spec decision A8): each of the 12 faces measures with
/// its own Adobe Core 14 AFM widths.
#[test]
fn every_standard_face_takes_its_own_afm_widths() {
    // (face, [space, A, a, W, H, m]) from the Core 14 AFMs' WX values.
    let table: [(Base14, [f64; 6]); 12] = [
        (
            Base14::Helvetica,
            [278.0, 667.0, 556.0, 944.0, 722.0, 833.0],
        ),
        (
            Base14::HelveticaOblique,
            [278.0, 667.0, 556.0, 944.0, 722.0, 833.0],
        ),
        (
            Base14::HelveticaBold,
            [278.0, 722.0, 556.0, 944.0, 722.0, 889.0],
        ),
        (
            Base14::HelveticaBoldOblique,
            [278.0, 722.0, 556.0, 944.0, 722.0, 889.0],
        ),
        (
            Base14::TimesRoman,
            [250.0, 722.0, 444.0, 944.0, 722.0, 778.0],
        ),
        (
            Base14::TimesBold,
            [250.0, 722.0, 500.0, 1000.0, 778.0, 833.0],
        ),
        (
            Base14::TimesItalic,
            [250.0, 611.0, 500.0, 833.0, 722.0, 722.0],
        ),
        (
            Base14::TimesBoldItalic,
            [250.0, 667.0, 500.0, 889.0, 778.0, 778.0],
        ),
        (Base14::Courier, [600.0; 6]),
        (Base14::CourierBold, [600.0; 6]),
        (Base14::CourierOblique, [600.0; 6]),
        (Base14::CourierBoldOblique, [600.0; 6]),
    ];
    for (face, widths) in table {
        for (byte, width) in b" AaWHm".iter().copied().zip(widths) {
            assert_eq!(face.advance(byte), width, "{face:?} {}", char::from(byte));
        }
    }
}

#[test]
fn win_ansi_encodes_annex_d() {
    assert_eq!(fonts::win_ansi('\u{20AC}'), Some(0x80));
    assert_eq!(fonts::win_ansi('\u{0081}'), None);
    assert_eq!(fonts::win_ansi('\u{2122}'), Some(0x99));
    assert_eq!(fonts::win_ansi('\u{0152}'), Some(0x8C));
    assert_eq!(fonts::win_ansi('\u{017E}'), Some(0x9E));
    assert_eq!(fonts::win_ansi('a'), Some(b'a'));
    assert_eq!(fonts::win_ansi('\u{e9}'), Some(0xE9));
    assert_eq!(fonts::win_ansi('\u{a0}'), Some(0xA0));
    assert_eq!(fonts::win_ansi('Ω'), None);
    assert_eq!(fonts::encode("a\tb日"), [0x61, 0x09, 0x62, 0x3F]);
    assert_eq!(fonts::encode("€é"), [0x80, 0xE9]);
}

#[test]
fn resource_names_choose_the_face() {
    let table = [
        (
            &[b"Helv".as_slice(), b"Helvetica"],
            Base14::Helvetica,
            "Helvetica",
        ),
        (
            &[b"HeBo", b"Helvetica-Bold"],
            Base14::HelveticaBold,
            "Helvetica-Bold",
        ),
        (
            &[b"HeOb", b"Helvetica-Oblique"],
            Base14::HelveticaOblique,
            "Helvetica-Oblique",
        ),
        (
            &[b"HeBO", b"Helvetica-BoldOblique"],
            Base14::HelveticaBoldOblique,
            "Helvetica-BoldOblique",
        ),
        (
            &[b"TiRo", b"Times-Roman"],
            Base14::TimesRoman,
            "Times-Roman",
        ),
        (&[b"TiBo", b"Times-Bold"], Base14::TimesBold, "Times-Bold"),
        (
            &[b"TiIt", b"Times-Italic"],
            Base14::TimesItalic,
            "Times-Italic",
        ),
        (
            &[b"TiBI", b"Times-BoldItalic"],
            Base14::TimesBoldItalic,
            "Times-BoldItalic",
        ),
        (&[b"Cour", b"Courier"], Base14::Courier, "Courier"),
        (
            &[b"CoBo", b"Courier-Bold"],
            Base14::CourierBold,
            "Courier-Bold",
        ),
        (
            &[b"CoOb", b"Courier-Oblique"],
            Base14::CourierOblique,
            "Courier-Oblique",
        ),
        (
            &[b"CoBO", b"Courier-BoldOblique"],
            Base14::CourierBoldOblique,
            "Courier-BoldOblique",
        ),
    ];
    for (names, face, base) in table {
        for name in names {
            assert_eq!(Base14::from_resource_name(name), face, "{name:?}");
        }
        assert_eq!(face.base_font(), base.as_bytes());
        let dict = face.dictionary();
        assert_eq!(dict.get(b"Type".as_slice()), Some(&Object::name("Font")));
        assert_eq!(
            dict.get(b"Subtype".as_slice()),
            Some(&Object::name("Type1"))
        );
        assert_eq!(dict.get(b"BaseFont".as_slice()), Some(&Object::name(base)));
        assert_eq!(
            dict.get(b"Encoding".as_slice()),
            Some(&Object::name("WinAnsiEncoding"))
        );
    }
    for other in [b"Symb".as_slice(), b"ZaDb", b"F1", b"", b"helv"] {
        assert_eq!(
            Base14::from_resource_name(other),
            Base14::Helvetica,
            "{other:?}"
        );
    }
}

// ================================================================== 9.3 (11) opacity

/// Every kind drawn at `alpha`: the label and the form.
fn every_kind(alpha: f64) -> Vec<(&'static str, Form)> {
    let q = [quad(100.0, 600.0, 200.0, 620.0)];
    let red = rgb(1.0, 0.0, 0.0);
    let style = Style {
        stroke: Some(red.clone()),
        fill: Some(rgb(0.0, 0.0, 1.0)),
        width: 2.0,
        opacity: alpha,
        dash: Vec::new(),
    };
    let mut text = free_text_box("half", 12.0);
    text.opacity = alpha;
    let rect = [100.0, 600.0, 300.0, 700.0];
    vec![
        ("note", appearance::note(b"Note", Some(&red), alpha)),
        ("highlight", appearance::highlight(&q, Some(&red), alpha).0),
        (
            "underline",
            appearance::text_line(Mark::Underline, &q, Some(&red), alpha).0,
        ),
        (
            "strikeout",
            appearance::text_line(Mark::StrikeOut, &q, Some(&red), alpha).0,
        ),
        (
            "squiggly",
            appearance::text_line(Mark::Squiggly, &q, Some(&red), alpha).0,
        ),
        (
            "square",
            appearance::rectangle(rect, appearance::grow(rect, 2.0), false, &style),
        ),
        (
            "circle",
            appearance::rectangle(rect, appearance::grow(rect, 2.0), true, &style),
        ),
        (
            "line",
            appearance::line(
                (100.0, 600.0),
                (300.0, 700.0),
                [b"OpenArrow", b"Circle"],
                &style,
            )
            .0,
        ),
        (
            "polygon",
            appearance::poly(
                &[(100.0, 600.0), (200.0, 700.0), (300.0, 600.0)],
                true,
                &style,
            )
            .0,
        ),
        (
            "polyline",
            appearance::poly(
                &[(100.0, 600.0), (200.0, 700.0), (300.0, 600.0)],
                false,
                &style,
            )
            .0,
        ),
        (
            "ink",
            appearance::ink(&[vec![(100.0, 600.0), (200.0, 700.0)]], &style).0,
        ),
        ("caret", appearance::caret(Some(&red), alpha)),
        (
            "stamp",
            appearance::stamp(b"Approved", Some(&red), alpha, appearance::IDENTITY),
        ),
        ("free text", appearance::free_text(rect, rect, &text)),
    ]
}

/// The ExtGState entries of a form's resources: name and dictionary.
fn states(form: &Form) -> Vec<(Vec<u8>, Dictionary)> {
    form.resources
        .get(b"ExtGState".as_slice())
        .and_then(Object::as_dict)
        .map(|states| {
            states
                .iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_dict()?.clone())))
                .collect()
        })
        .unwrap_or_default()
}

fn real(value: Option<&Object>) -> Option<f64> {
    match value? {
        Object::Integer(v) => Some(*v as f64),
        Object::Real(v) => Some(*v),
        _ => None,
    }
}

#[test]
fn opacity_is_set_by_an_ext_g_state_the_content_selects() {
    for (label, form) in every_kind(0.5) {
        let ops = checked(&form.content, label);
        let half: Vec<Vec<u8>> = states(&form)
            .into_iter()
            .filter(|(_, s)| {
                real(s.get(b"CA".as_slice())) == Some(0.5)
                    && real(s.get(b"ca".as_slice())) == Some(0.5)
            })
            .map(|(name, _)| name)
            .collect();
        assert!(!half.is_empty(), "{label}: {:?}", form.resources);
        // Selected before anything is painted.
        let first_paint = ops
            .iter()
            .position(|op| {
                [
                    b"S".as_slice(),
                    b"s",
                    b"f",
                    b"F",
                    b"f*",
                    b"B",
                    b"B*",
                    b"b",
                    b"b*",
                    b"Tj",
                    b"TJ",
                ]
                .contains(&op.operator.as_slice())
            })
            .unwrap_or_else(|| panic!("{label} paints"));
        let selected = ops[..first_paint].iter().any(|op| {
            op.operator == b"gs"
                && matches!(op.operands.first(), Some(ContentOperand::Name(n)) if half.contains(n))
        });
        assert!(selected, "{label}");
    }
    // Opaque: no constant alpha is set.
    for (label, form) in every_kind(1.0) {
        checked(&form.content, label);
        for (_, s) in states(&form) {
            assert!(
                real(s.get(b"CA".as_slice())).is_none_or(|a| a >= 1.0)
                    && real(s.get(b"ca".as_slice())).is_none_or(|a| a >= 1.0),
                "{label}"
            );
        }
    }
}

// ================================================================== 3.0 numbers, dictionary

#[test]
fn content_numbers_have_at_most_four_decimals() {
    for (label, form) in every_kind(0.5) {
        for op in operations(&form.content) {
            for operand in &op.operands {
                let v = match operand {
                    ContentOperand::Real(v) => *v,
                    _ => continue,
                };
                let scaled = v * 1e4;
                assert!(
                    (scaled - scaled.round()).abs() < 1e-6 * scaled.abs().max(1.0),
                    "{label}: {v} in {}",
                    String::from_utf8_lossy(&op.operator)
                );
            }
        }
    }
}

#[test]
fn numbers_and_geometry_helpers_follow_the_spec() {
    assert_eq!(appearance::number(3.0), Object::Integer(3));
    assert_eq!(appearance::number(-2.0), Object::Integer(-2));
    assert_eq!(appearance::number(0.5), Object::Real(0.5));
    assert_eq!(appearance::number(1e15), Object::Real(1e15));
    assert_eq!(appearance::number(f64::NAN), Object::Integer(0));
    assert_eq!(appearance::number(f64::INFINITY), Object::Integer(0));
    assert_eq!(
        appearance::numbers(&[1.0, 2.5]),
        Object::Array(vec![Object::Integer(1), Object::Real(2.5)])
    );
    assert_eq!(appearance::bounds([]), None);
    assert_eq!(
        appearance::bounds([(3.0, 1.0), (-1.0, 4.0), (2.0, 2.0)]),
        Some([-1.0, 1.0, 3.0, 4.0])
    );
    assert_eq!(
        appearance::grow([0.0, 0.0, 10.0, 10.0], 2.0),
        [-2.0, -2.0, 12.0, 12.0]
    );
    assert_eq!(
        appearance::grow([0.0, 0.0, 10.0, 10.0], -1.0),
        [1.0, 1.0, 9.0, 9.0]
    );
    let style = Style::default();
    assert_eq!(
        (
            style.stroke,
            style.fill,
            style.width,
            style.opacity,
            style.dash.len()
        ),
        (None, None, 1.0, 1.0, 0)
    );
    // Allowances: at least half the width, monotonic, finite.
    let mut last = (0.0, 0.0);
    for w in [0.0, 0.5, 1.0, 2.0, 5.0, 40.0] {
        let (b, i) = (
            appearance::border_allowance(w),
            appearance::ink_allowance(w),
        );
        assert!(
            b >= w / 2.0 && i >= w / 2.0 && b.is_finite() && i.is_finite(),
            "{w}"
        );
        assert!(b >= last.0 && i >= last.1, "{w}");
        last = (b, i);
    }
}

#[test]
fn a_form_dictionary_holds_exactly_its_entries() {
    let form = appearance::caret(None, 1.0);
    let dict = form.dictionary();
    let mut keys: Vec<&[u8]> = dict.keys().map(Vec::as_slice).collect();
    keys.sort();
    let mut want: Vec<&[u8]> = vec![b"BBox", b"Matrix", b"Subtype", b"Type"];
    if !form.resources.is_empty() {
        want.push(b"Resources");
    }
    want.sort();
    assert_eq!(keys, want);
    assert_eq!(dict.get(b"Type".as_slice()), Some(&Object::name("XObject")));
    assert_eq!(dict.get(b"Subtype".as_slice()), Some(&Object::name("Form")));
    assert_eq!(
        dict.get(b"Matrix".as_slice()),
        Some(&appearance::numbers(&appearance::IDENTITY))
    );
    assert_eq!(
        dict.get(b"BBox".as_slice()),
        Some(&appearance::numbers(&form.bbox))
    );
    let half = appearance::caret(None, 0.5).dictionary();
    assert!(half.get(b"Resources".as_slice()).is_some());
}

#[test]
fn an_image_stamp_draws_image_i_under_its_placement() {
    let image = id(42);
    let turn = [0.0, 1.0, -1.0, 0.0, 1.0, 0.0];
    let form = appearance::image_stamp(image, appearance::IDENTITY, turn);
    assert_eq!(form.content, b"/I Do\n");
    assert_eq!(form.bbox, [0.0, 0.0, 1.0, 1.0]);
    assert_eq!(form.matrix, turn);
    let xobjects = form
        .resources
        .get(b"XObject".as_slice())
        .and_then(Object::as_dict);
    assert_eq!(
        xobjects.and_then(|x| x.get(b"I".as_slice())),
        Some(&Object::Reference(image))
    );
    let placed = [0.5, 0.0, 0.0, -1.0, 0.25, 1.0];
    let form = appearance::image_stamp(image, placed, appearance::IDENTITY);
    let ops = operations(&form.content);
    let names: Vec<&[u8]> = ops.iter().map(|op| op.operator.as_slice()).collect();
    assert_eq!(names, [b"q".as_slice(), b"cm", b"Do", b"Q"]);
    let numbers: Vec<f64> = ops[1].operands.iter().map(number_of).collect();
    assert_eq!(numbers, placed);
    assert_eq!(ops[2].operands, [ContentOperand::Name(b"I".to_vec())]);
    assert!(form.content.starts_with(b"q\n") && form.content.ends_with(b"cm\n/I Do\nQ\n"));
}

// ================================================================== 3.4 shapes, 3.11 redact

#[test]
fn squares_and_circles_run_their_border_along_the_inner_rect() {
    let inner = [100.0, 600.0, 300.0, 700.0];
    let style = Style {
        stroke: Some(rgb(1.0, 0.0, 0.0)),
        fill: Some(rgb(0.0, 0.0, 1.0)),
        width: 4.0,
        opacity: 1.0,
        dash: Vec::new(),
    };
    for ellipse in [false, true] {
        let bbox = appearance::grow(inner, appearance::border_allowance(4.0));
        let form = appearance::rectangle(inner, bbox, ellipse, &style);
        checked(&form.content, &format!("ellipse {ellipse}"));
        assert_eq!(form.bbox, bbox);
        assert_eq!(form.matrix, appearance::IDENTITY);
        let image = render(&form_page(&form, bbox), 2.0);
        assert_ink_inside(&image, bbox, 2.0, &format!("ellipse {ellipse}"));
        // Filled with /IC at the centre, stroked with /C on the inner
        // rect's edge (the ellipse's: the middle of each side).
        assert_eq!(pixel(&image, 400, (792 - 650) * 2), [0, 0, 255]);
        let edge = pixel(&image, 400, (792 - 700) * 2);
        assert!(edge[0] > 200 && edge[2] < 60, "ellipse {ellipse}: {edge:?}");
        let side = pixel(&image, 300 * 2, (792 - 650) * 2);
        assert!(side[0] > 200 && side[2] < 60, "ellipse {ellipse}: {side:?}");
    }
}

#[test]
fn a_dashed_border_writes_its_dash_array() {
    let style = Style {
        stroke: Some(rgb(1.0, 0.0, 0.0)),
        fill: None,
        width: 2.0,
        opacity: 1.0,
        dash: vec![3.0, 2.0],
    };
    let inner = [100.0, 600.0, 300.0, 700.0];
    let form = appearance::rectangle(inner, appearance::grow(inner, 2.0), false, &style);
    let ops = checked(&form.content, "dashed");
    let dash = ops
        .iter()
        .position(|op| op.operator == b"d")
        .expect("a d operator");
    let stroke = ops
        .iter()
        .position(|op| [b"S".as_slice(), b"s", b"B", b"b"].contains(&op.operator.as_slice()))
        .expect("a stroke");
    assert!(dash < stroke);
    let ContentOperand::Array(array) = &ops[dash].operands[0] else {
        panic!("an array")
    };
    assert_eq!(array.iter().map(number_of).collect::<Vec<_>>(), [3.0, 2.0]);
}

#[test]
fn a_redact_mark_outlines_its_quads_or_keeps_its_rect() {
    let quads = [
        quad(100.0, 600.0, 200.0, 612.0),
        quad(100.0, 580.0, 150.0, 592.0),
    ];
    let (form, rect) = appearance::redact([0.0, 0.0, 612.0, 792.0], &quads, None);
    let ops = checked(&form.content, "quads");
    assert_eq!(
        count(&ops, &["f", "F", "f*", "B", "B*", "b", "b*"]),
        0,
        "no fill"
    );
    assert!(count(&ops, &["S", "s"]) >= 1);
    let all = [100.0, 580.0, 200.0, 612.0];
    assert!(contains(rect, all, 1e-9) && contains(appearance::grow(all, 2.0), rect, 1e-9));
    assert_eq!(form.bbox, rect);
    let image = render(&form_page(&form, rect), 2.0);
    assert_ink_inside(&image, rect, 2.0, "quads");
    let ink = ink_box(&image).unwrap();
    let colour = dominant(&image, ink);
    assert!(
        colour != [0, 0, 0] && luminance(colour) < 240.0,
        "a conspicuous colour: {colour:?}"
    );
    // Without quads: the user's rect, the outline inside it.
    let area = [100.0, 600.0, 200.0, 650.0];
    let (form, rect) = appearance::redact(area, &[], Some(&rgb(0.0, 0.0, 1.0)));
    assert_eq!(rect, area);
    let image = render(&form_page(&form, rect), 2.0);
    assert_ink_inside(&image, area, 2.0, "area");
    let ink = ink_box(&image).unwrap();
    let r = pixels_of(area, 2.0);
    assert!(f64::from(ink[0]) <= r[0] + 4.0 && f64::from(ink[2]) >= r[2] - 5.0);
}
