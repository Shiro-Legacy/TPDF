// SPDX-License-Identifier: MIT
//! Round 3 of the acceptance review: the cheap minor findings on accepted
//! ops (form fields, annotations, pages and content editing).
use std::sync::Arc;

use serde_json::{Value, json};
use tandem_pdf_engine::app::annots;
use tandem_pdf_engine::app::edit::{Builder, Edit};
use tandem_pdf_engine::app::{AppEngine, AppSession};
use tandem_pdf_engine::content::{ContentLimits, ContentOperand, ContentParser, Operation};
use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::object::{Dictionary, Object, ObjectId};
use tandem_pdf_engine::stream::StreamLimits;
use tandem_pdf_engine::view::{RenderOptions, ViewDocument};
use tandem_pdf_engine::writer::{DocumentWriteLimits, WriteOptions, write_document};

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

/// One blank page (object 3) with `annots` (objects from 5) in /Annots;
/// object 4 is its content.
fn page_with(annots: &[&str], extra: &[String]) -> Vec<u8> {
    let refs: Vec<String> = (0..annots.len())
        .map(|i| format!("{} 0 R", 5 + i))
        .collect();
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Annots [{}] >>",
            refs.join(" ")
        ),
        stream("", "q Q"),
    ];
    objects.extend(annots.iter().map(|a| a.to_string()));
    objects.extend(extra.iter().cloned());
    classic(&objects)
}

/// One page with a text field (widget 6) in /AcroForm 5, /DR /Helv 7.
fn text_field() -> Vec<u8> {
    classic(&[
        "<< /Type /Catalog /Pages 2 0 R /AcroForm 5 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Annots [6 0 R] >>"
            .into(),
        stream("", "q Q"),
        "<< /Fields [6 0 R] /DA (/Helv 12 Tf 0 g) /DR << /Font << /Helv 7 0 R >> >> >>".into(),
        "<< /Type /Annot /Subtype /Widget /F 4 /P 3 0 R /FT /Tx /T (t) /V () \
         /Rect [72 700 252 724] /DA (/Helv 12 Tf 0 g) >>"
            .into(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".into(),
    ])
}

fn session_with(bytes: Vec<u8>) -> (AppEngine, AppSession) {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    let request = json!({"id": 1, "op": "open", "name": "a.pdf", "pdf": {"$buffer": 0}});
    let (reply, _) = engine.dispatch(&mut session, request, vec![bytes]);
    assert_eq!(reply["ok"], true, "{reply}");
    (engine, session)
}

fn dispatch(engine: &mut AppEngine, session: &mut AppSession, request: Value) -> Value {
    engine.dispatch(session, request, Vec::new()).0
}

fn ok(engine: &mut AppEngine, session: &mut AppSession, request: Value) -> Value {
    let reply = dispatch(engine, session, request.clone());
    assert_eq!(reply["ok"], true, "{request} -> {reply}");
    reply
}

fn doc(session: &AppSession) -> &Document<'static> {
    session.doc.as_ref().unwrap().document()
}

fn dict(session: &AppSession, n: u32) -> Dictionary {
    doc(session).resolve_id(id(n)).as_dict().unwrap().clone()
}

/// A deterministic full rewrite of the current state, every object kept.
fn serialized(session: &AppSession) -> Vec<u8> {
    let options = WriteOptions {
        garbage: false,
        ..WriteOptions::APP
    };
    write_document(doc(session), DocumentWriteLimits::default(), options).unwrap()
}

/// The app's saved bytes.
fn saved(session: &AppSession) -> Vec<u8> {
    write_document(
        doc(session),
        DocumentWriteLimits::default(),
        WriteOptions::APP,
    )
    .unwrap()
}

/// The red channel of page 0 inside a y-down rect, at 2 px per pt.
fn pixels(session: &mut AppSession, rect: [u32; 4]) -> Vec<u8> {
    let image = (session.view().unwrap())
        .render(0, 2.0, RenderOptions::default())
        .unwrap();
    let mut region = Vec::new();
    for y in rect[1] * 2..rect[3] * 2 {
        for x in rect[0] * 2..rect[2] * 2 {
            region.push(image.pixels[((y * image.width + x) * 4) as usize]);
        }
    }
    region
}

/// A form XObject's dictionary and its decoded content, as Latin-1 text.
fn form_content(session: &AppSession, form: &Object) -> (Dictionary, String) {
    let doc = doc(session);
    let id = form.as_reference().expect("an appearance object");
    let dictionary = doc.stream_dict(id).unwrap().clone();
    let data = doc.decoded_stream(id, StreamLimits::default()).unwrap();
    (dictionary, data.iter().map(|&b| char::from(b)).collect())
}

/// The /AP /N stream of annotation or widget `n`, if any.
fn normal(session: &AppSession, n: u32) -> Option<(Dictionary, String)> {
    let annot = dict(session, n);
    let ap = doc(session).dict_get(&annot, b"AP").as_dict()?.clone();
    let normal = ap.get(b"N".as_slice())?;
    normal.as_reference()?;
    Some(form_content(session, normal))
}

fn font_names(session: &AppSession, form: &Dictionary) -> Vec<Vec<u8>> {
    let doc = doc(session);
    let resources = doc.dict_get(form, b"Resources");
    let fonts = doc.dict_get(resources.as_dict().unwrap(), b"Font");
    fonts.as_dict().unwrap().keys().cloned().collect()
}

// ------------------------------------------------------------------ appearance checks

/// The content's operations; it must parse.
fn operations(content: &[u8]) -> Vec<Operation> {
    let mut parser = ContentParser::new(content, ContentLimits::default());
    let mut ops = Vec::new();
    while let Some(op) = parser
        .next_operation()
        .unwrap_or_else(|e| panic!("content parses: {e}"))
    {
        ops.push(op);
    }
    ops
}

fn number_of(operand: &ContentOperand) -> f64 {
    match operand {
        ContentOperand::Integer(v) => *v as f64,
        ContentOperand::Real(v) => *v,
        other => panic!("a number: {other:?}"),
    }
}

/// The dash array and line width in effect at the first stroke of a
/// content stream (Latin-1 text), following `q`/`Q`.
fn border_stroke(content: &str) -> Option<(Vec<f64>, f64)> {
    let bytes: Vec<u8> = content.chars().map(|c| c as u8).collect();
    let mut state = (Vec::new(), 1.0);
    let mut saved = Vec::new();
    for op in operations(&bytes) {
        match op.operator.as_slice() {
            b"q" => saved.push(state.clone()),
            b"Q" => state = saved.pop().unwrap_or(state),
            b"w" => state.1 = number_of(&op.operands[0]),
            b"d" => {
                let ContentOperand::Array(dash) = &op.operands[0] else {
                    panic!("a dash array")
                };
                state.0 = dash.iter().map(number_of).collect();
            }
            b"S" | b"s" | b"B" | b"B*" | b"b" | b"b*" => return Some(state),
            _ => {}
        }
    }
    None
}

/// What annotation or widget `number` shows, as the engine's text
/// extraction reads it: `bytes` (the session's object numbers) with its
/// normal appearance drawn by page 0's content where it shows it (ISO
/// 32000-2 §12.5.5), the page's annotations dropped.
fn drawn_text(bytes: &[u8], number: u32) -> String {
    let doc =
        Document::open_shared(Arc::new(bytes.to_vec()), DocumentLimits::default(), b"").unwrap();
    let page = tandem_pdf_engine::app::info::pages(&doc)[0]
        .id
        .expect("an indirect page");
    let annot = doc.resolve_id(id(number)).as_dict().unwrap();
    let form = doc.dict_get(annot, b"AP").as_dict().unwrap()[b"N".as_slice()]
        .as_reference()
        .unwrap();
    let rect = annots::rect(&doc, annot.get(b"Rect".as_slice()).unwrap()).unwrap();
    let form_dict = doc.resolve_id(form).as_dict().unwrap();
    let bbox = annots::rect(&doc, form_dict.get(b"BBox".as_slice()).unwrap()).unwrap();
    let matrix = match form_dict
        .get(b"Matrix".as_slice())
        .and_then(|m| annots::numbers(&doc, m))
    {
        Some(m) if m.len() == 6 => [m[0], m[1], m[2], m[3], m[4], m[5]],
        _ => [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
    };
    let shown = annots::transform_rect(&matrix, bbox);
    let sx = (rect[2] - rect[0]) / (shown[2] - shown[0]);
    let sy = (rect[3] - rect[1]) / (shown[3] - shown[1]);
    let content = format!(
        "q {sx:.6} 0 0 {sy:.6} {:.6} {:.6} cm /A Do Q",
        rect[0] - shown[0] * sx,
        rect[1] - shown[1] * sy
    );
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
    let pdf = write_document(
        builder.document(),
        DocumentWriteLimits::default(),
        WriteOptions::APP,
    )
    .unwrap();
    let text = ViewDocument::open(pdf, "")
        .unwrap()
        .structured_text(0)
        .unwrap()
        .as_text();
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Some font of `form` names `glyphs` in its /Differences (Adobe Glyph
/// List names), or every font beside the /DA font maps its codes to Unicode
/// through a /ToUnicode CMap.
fn fonts_name_their_glyphs(session: &AppSession, form: &Dictionary, glyphs: &[&str]) {
    let doc = doc(session);
    let resources = doc.dict_get(form, b"Resources").as_dict().unwrap();
    let fonts = doc.dict_get(resources, b"Font").as_dict().unwrap();
    let fonts: Vec<(&Vec<u8>, &Dictionary)> = fonts
        .iter()
        .filter_map(|(name, font)| Some((name, doc.resolve(font).as_dict()?)))
        .collect();
    let named = fonts.iter().any(|(_, font)| {
        let differences = doc
            .dict_get(font, b"Encoding")
            .as_dict()
            .and_then(|e| doc.dict_get(e, b"Differences").as_array());
        differences.is_some_and(|d| {
            glyphs.iter().all(|g| {
                d.iter()
                    .any(|n| doc.resolve(n).as_name() == Some(g.as_bytes()))
            })
        })
    });
    let mapped = fonts
        .iter()
        .filter(|(name, _)| name.as_slice() != b"Helv")
        .all(|(_, font)| font.contains_key(b"ToUnicode".as_slice()));
    assert!(named || mapped, "{glyphs:?}");
}

// ------------------------------------------------------------------- forms

/// Latin letters WinAnsi lacks (Polish, Czech, Turkish, Romanian) fill a
/// widget through a fallback font, as FreeText does: the widget keeps a
/// stored appearance that draws the letters, not `?`, and nothing
/// document-wide is written.
#[test]
fn latin_letters_winansi_lacks_store_a_widget_appearance() {
    let (mut engine, mut session) = session_with(text_field());
    let before = serialized(&session);
    for (value, glyphs) in [
        ("Zażółć", &["zdotaccent", "lslash", "cacute"][..]),
        ("Příliš", &["rcaron"]),
        ("Işık", &["dotlessi", "scedilla"]),
        ("Țară", &["abreve"]),
    ] {
        ok(
            &mut engine,
            &mut session,
            json!({"op": "field-set", "page": 0, "widgetId": 6, "value": value}),
        );
        let (form, content) = normal(&session, 6).expect("a stored appearance");
        let bytes: Vec<u8> = content.chars().map(|c| c as u8).collect();
        operations(&bytes);
        // The DA font and a font for the other letters, which a viewer
        // reads back as them.
        let fonts = font_names(&session, &form);
        assert!(fonts.len() >= 2, "{value}");
        assert_eq!(drawn_text(&serialized(&session), 6), value);
        fonts_name_their_glyphs(&session, &form, glyphs);
        let acro = dict(&session, 5);
        assert!(acro.get(b"NeedAppearances".as_slice()).is_none());
    }
    // The reopened save draws each letter, not a `?` (y down 68..92).
    let drawn = |value: &str| {
        let (mut engine, mut session) = session_with(text_field());
        ok(
            &mut engine,
            &mut session,
            json!({"op": "field-set", "page": 0, "widgetId": 6, "value": value}),
        );
        let (_, mut reopened) = session_with(saved(&session));
        assert!(!String::from_utf8_lossy(&saved(&reopened)).contains("NeedAppearances"));
        pixels(&mut reopened, [72, 68, 100, 92])
    };
    let question = drawn("?");
    for letter in ["ż", "ł", "ř", "ı", "ș", "ț"] {
        let region = drawn(letter);
        assert!(region.iter().any(|&v| v < 128), "{letter} draws");
        assert_ne!(region, question, "{letter} is not a ?");
    }
    for _ in 0..4 {
        ok(&mut engine, &mut session, json!({"op": "undo"}));
    }
    assert_eq!(serialized(&session), before);
}

// ------------------------------------------------------------------ annots

/// The reopened save of a page with one FreeText added at [40 60] with
/// `text`: its pixels.
fn free_text_drawn(text: &str) -> Vec<u8> {
    let (mut engine, mut session) = session_with(page_with(&[], &[]));
    ok(
        &mut engine,
        &mut session,
        json!({"op": "annot-add", "page": 0, "type": "FreeText", "point": [40, 60], "contents": text}),
    );
    let (_, mut reopened) = session_with(saved(&session));
    pixels(&mut reopened, [40, 60, 240, 100])
}

/// Zero-width and format characters draw nothing, a no-break space draws
/// as a space and a decomposed accent as the composed letter, in the saved
/// appearance.
#[test]
fn free_text_draws_format_characters_as_nothing_and_composes_accents() {
    let plain = free_text_drawn("ab cd");
    assert!(plain.iter().any(|&v| v < 128));
    assert_eq!(
        free_text_drawn("a\u{200b}b\u{feff} c\u{200d}d\u{fe0f}"),
        plain
    );
    assert_eq!(free_text_drawn("ab\u{a0}cd"), plain);
    assert_eq!(free_text_drawn("Caf\u{e9}"), free_text_drawn("Cafe\u{301}"));
    assert_eq!(free_text_drawn("\u{17c}"), free_text_drawn("z\u{307}"));
}

/// annot-set regenerates a dashed FreeText border dashed, and the view's
/// synthesis of one without an appearance draws it dashed too.
#[test]
fn a_dashed_free_text_border_is_regenerated_dashed() {
    let free_text = "<< /Type /Annot /Subtype /FreeText /Rect [100 600 300 700] \
                     /DA (/Helv 12 Tf 0 g) /Contents (x) /BS << /W 2 /S /D /D [3 2] >> >>";
    let (mut engine, mut session) = session_with(page_with(&[free_text], &[]));
    let before = serialized(&session);
    ok(
        &mut engine,
        &mut session,
        json!({"op": "annot-set", "page": 0, "annotId": 5, "contents": "dashed"}),
    );
    let (_, content) = normal(&session, 5).expect("a regenerated appearance");
    assert_eq!(
        border_stroke(&content),
        Some((vec![3.0, 2.0], 2.0)),
        "{content}"
    );
    // The border's top edge (y down 92..94) has gaps where a solid one has
    // none.
    let edge = pixels(&mut session, [110, 92, 290, 94]);
    assert!(edge.iter().any(|&v| v < 128), "the border draws");
    assert!(edge.iter().any(|&v| v > 200), "with gaps");
    ok(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(serialized(&session), before);
}

/// A dash array that is not a valid `d` operand (negative, all zero or
/// past what a content stream writes) regenerates with the default `[3]`.
#[test]
fn an_invalid_free_text_dash_array_falls_back_to_the_default() {
    for dash in ["[0 0]", "[-3 2]", "[0]", "[1e30]"] {
        let free_text = format!(
            "<< /Type /Annot /Subtype /FreeText /Rect [100 600 300 700] \
             /DA (/Helv 12 Tf 0 g) /Contents (x) /BS << /W 2 /S /D /D {dash} >> >>"
        );
        let (mut engine, mut session) = session_with(page_with(&[&free_text], &[]));
        ok(
            &mut engine,
            &mut session,
            json!({"op": "annot-set", "page": 0, "annotId": 5, "contents": "dashed"}),
        );
        let (_, content) = normal(&session, 5).expect("a regenerated appearance");
        assert_eq!(
            border_stroke(&content),
            Some((vec![3.0], 2.0)),
            "{dash}: {content}"
        );
    }
}

// ------------------------------------------------------------------- pages

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & (crc & 1).wrapping_neg());
        }
    }
    !crc
}

/// A small RGB PNG.
fn small_png() -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, 4, 4);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[200; 48]).unwrap();
    }
    out
}

/// Where chunk `kind` starts (its length field).
fn chunk(png: &[u8], kind: &[u8; 4]) -> usize {
    png.windows(4).position(|w| w == kind).unwrap() - 4
}

fn images_to_pdf(bytes: Vec<u8>) -> Value {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    let request =
        json!({"op": "pdf-from-images", "images": [{"name": "a.png", "bytes": {"$buffer": 0}}]});
    engine.dispatch(&mut session, request, vec![bytes]).0
}

/// A PNG whose chunk CRC does not match is still converted, as the earlier app engine
/// converted it; a PNG without IEND stays refused, a stricter policy kept
/// on purpose.
#[test]
fn a_png_with_a_bad_chunk_crc_converts() {
    assert_eq!(images_to_pdf(small_png())["ok"], true);
    for kind in [b"IHDR", b"IDAT"] {
        let mut png = small_png();
        let at = chunk(&png, kind);
        let length = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
        png[at + 8 + length] ^= 0xFF;
        let reply = images_to_pdf(png);
        assert_eq!(reply["ok"], true, "{kind:?}: {reply}");
    }
    let mut png = small_png();
    png.truncate(chunk(&png, b"IEND"));
    let reply = images_to_pdf(png);
    assert_eq!(reply["reason"], "invalid", "{reply}");
}

/// A valid PNG whose samples pass the 256 MiB decoding budget is refused as
/// too large, not as unreadable.
#[test]
fn a_png_past_the_decoding_budget_is_too_large() {
    let mut png = small_png();
    let at = chunk(&png, b"IHDR");
    // 8200 x 8200 RGBA: 269 MB of samples.
    png[at + 8..at + 16].copy_from_slice(&[0, 0, 0x20, 0x08, 0, 0, 0x20, 0x08]);
    png[at + 17] = 6;
    let crc = crc32(&png[at + 4..at + 8 + 13]);
    png[at + 21..at + 25].copy_from_slice(&crc.to_be_bytes());
    let reply = images_to_pdf(png);
    assert_eq!(reply["reason"], "too-large", "{reply}");
    assert_eq!(
        reply["error"],
        "pdf-from-images: the document would exceed the 52428800 byte save ceiling"
    );
}

// -------------------------------------------------------------------- edit

/// A font whose /BaseFont is the empty name has no display name: null, as
/// the earlier app engine lists it, so the renderer shows the font's type and state.
#[test]
fn an_empty_base_font_name_is_listed_as_null() {
    let page = |base_font: &str| {
        classic(&[
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
             /Resources << /Font << /F1 5 0 R >> >> >>"
                .into(),
            stream("", "BT /F1 24 Tf 72 700 Td (Hello) Tj ET"),
            format!(
                "<< /Type /Font /Subtype /TrueType /BaseFont {base_font} \
                 /Encoding /WinAnsiEncoding >>"
            ),
        ])
    };
    let font_of = |base_font: &str| {
        let (mut engine, mut session) = session_with(page(base_font));
        let reply = ok(
            &mut engine,
            &mut session,
            json!({"op": "edit-targets", "page": 0}),
        );
        let items = reply["items"].as_array().unwrap();
        let item = (items.iter())
            .find(|item| item["text"] == "Hello")
            .unwrap_or_else(|| panic!("no Hello in {reply}"));
        item["font"].clone()
    };
    let font = font_of("/");
    assert_eq!(font["name"], Value::Null, "{font}");
    assert_eq!(font["subtype"], "TrueType", "{font}");
    assert_eq!(font_of("/Arial")["name"], "Arial");
}
