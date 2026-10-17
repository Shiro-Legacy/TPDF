// SPDX-License-Identifier: AGPL-3.0-or-later
//! Round 3 of the acceptance review: the cheap minor findings on accepted
//! ops (design/app-accept-forms.md, app-accept-annots.md and the other
//! groups' "Round 3 minors" sections).
use serde_json::{Value, json};
use tandem_pdf_engine::app::{AppEngine, AppSession};
use tandem_pdf_engine::document::Document;
use tandem_pdf_engine::object::{Dictionary, Object, ObjectId};
use tandem_pdf_engine::stream::StreamLimits;
use tandem_pdf_engine::view::RenderOptions;
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

// ------------------------------------------------------------------- forms

/// Latin letters WinAnsi lacks (Polish, Czech, Turkish, Romanian) fill a
/// widget through the re-encoded Latin font, as FreeText does: the widget
/// keeps a stored appearance that draws the letters, not `?`, and nothing
/// document-wide is written (design/app-accept-forms.md, round 3).
#[test]
fn latin_letters_winansi_lacks_store_a_widget_appearance() {
    let (mut engine, mut session) = session_with(text_field());
    let before = serialized(&session);
    for value in ["Zażółć", "Příliš", "Işık", "Țară"] {
        ok(
            &mut engine,
            &mut session,
            json!({"op": "field-set", "page": 0, "widgetId": 6, "value": value}),
        );
        let (form, content) = normal(&session, 6).expect("a stored appearance");
        assert!(!content.contains('?'), "{value}: {content}");
        assert!(content.contains("/HelvLAT 12 Tf"), "{value}: {content}");
        let names = font_names(&session, &form);
        assert!(names.contains(&b"HelvLAT".to_vec()), "{value}");
        let acro = dict(&session, 5);
        assert!(acro.get(b"NeedAppearances".as_slice()).is_none());
    }
    // The Latin font names its glyphs in /Differences.
    let (form, _) = normal(&session, 6).unwrap();
    let resources = doc(&session).dict_get(&form, b"Resources");
    let fonts = doc(&session).dict_get(resources.as_dict().unwrap(), b"Font");
    let latin = doc(&session).dict_get(fonts.as_dict().unwrap(), b"HelvLAT");
    let latin = latin.as_dict().unwrap();
    let encoding = doc(&session).dict_get(latin, b"Encoding");
    let differences = doc(&session).dict_get(encoding.as_dict().unwrap(), b"Differences");
    assert!(
        (differences.as_array().unwrap()).contains(&Object::name("uni021B")),
        "ț"
    );
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
/// appearance (design/app-accept-annots.md, round 3).
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
    assert!(content.contains("[3 2]0 d\n2 w\n"), "{content}");
    // The border's top edge (y down 92..94) has gaps where a solid one has
    // none.
    let edge = pixels(&mut session, [110, 92, 290, 94]);
    assert!(edge.iter().any(|&v| v < 128), "the border draws");
    assert!(edge.iter().any(|&v| v > 200), "with gaps");
    ok(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(serialized(&session), before);
}
