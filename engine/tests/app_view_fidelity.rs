// SPDX-License-Identifier: AGPL-3.0-or-later
//! The app's view against MuPDF 1.28.1 where hayro reads a document
//! differently: page attributes inherited through /Parent, the page list of
//! a document without a usable page tree, and text in base 14 fonts outside
//! WinAnsi (the URW outlines the view builds fonts from).
use serde_json::{Value, json};
use std::sync::Arc;
use tandem_pdf_engine::app::{AppDocument, AppEngine, AppSession};
use tandem_pdf_engine::object::{Object, ObjectId};
use tandem_pdf_engine::view::{RenderOptions, ViewDocument};

/// A classic-xref PDF from object bodies numbered from 1.
fn classic(objects: &[String]) -> Vec<u8> {
    let order: Vec<usize> = (0..objects.len()).collect();
    classic_in(objects, &order)
}

/// `classic` with the objects written in the file in `order` (indices).
fn classic_in(objects: &[String], order: &[usize]) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = vec![0; objects.len()];
    for &index in order {
        offsets[index] = bytes.len();
        let body = &objects[index];
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

fn stream(data: &str) -> String {
    format!("<< /Length {} >>\nstream\n{data}\nendstream", data.len())
}

/// A Form XObject over [0 0 size size] filling itself blue.
fn blue_form(size: u32) -> String {
    let data = format!("0 0 1 rg 0 0 {size} {size} re f");
    format!(
        "<< /Type /XObject /Subtype /Form /BBox [0 0 {size} {size}] /Length {} >>\nstream\n{data}\nendstream",
        data.len()
    )
}

fn session_with(bytes: Vec<u8>) -> (AppEngine, AppSession) {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    let request = json!({"id": 1, "op": "open", "name": "a.pdf", "pdf": {"$buffer": 0}});
    let (reply, _) = engine.dispatch(&mut session, request, vec![bytes]);
    assert_eq!(reply["ok"], true, "{reply}");
    (engine, session)
}

/// The render reply of `page` at scale 1 and its RGBA pixels.
fn render(engine: &mut AppEngine, session: &mut AppSession, page: usize) -> (Value, Vec<u8>) {
    let request = json!({"op": "render", "page": page, "scale": 1});
    let (reply, mut buffers) = engine.dispatch(session, request, Vec::new());
    assert_eq!(reply["ok"], true, "{reply}");
    (reply, buffers.remove(0))
}

/// Pixels in rows `y0..y1`, columns `x0..x1` that are not white.
fn ink(pixels: &[u8], width: usize, (x0, y0, x1, y1): (usize, usize, usize, usize)) -> usize {
    (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (y * width + x) * 4))
        .filter(|&at| pixels[at..at + 3] != [255, 255, 255])
        .count()
}

fn id(number: u32) -> ObjectId {
    ObjectId::new(number, 0).unwrap()
}

#[test]
fn resources_inherited_through_a_direct_parent_are_drawn() {
    // The page's /Parent is a direct dictionary holding the Form the content
    // draws; MuPDF reads it through /Parent, hayro would not.
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent << /Type /Pages /Resources << /XObject << /X0 5 0 R >> >> >> \
         /MediaBox [0 0 200 200] /Contents 4 0 R >>"
            .to_string(),
        stream("/X0 Do"),
        blue_form(200),
    ];
    let (mut engine, mut session) = session_with(classic(&objects));
    let (reply, pixels) = render(&mut engine, &mut session, 0);
    assert_eq!(
        (reply["pixelWidth"].as_u64(), reply["pixelHeight"].as_u64()),
        (Some(200), Some(200))
    );
    assert_eq!(
        ink(&pixels, 200, (0, 0, 200, 200)),
        200 * 200,
        "the Form fills the page"
    );
    // The saved document keeps the page as it was.
    let doc = session.doc.as_ref().unwrap().document();
    let page = doc.resolve_id(id(3)).as_dict().unwrap();
    assert!(page.get(b"Resources".as_slice()).is_none());
}

#[test]
fn boxes_inherited_through_parent_decide_the_clip() {
    // The page sits in the root's /Kids, whose CropBox hayro would apply;
    // its /Parent is another node without one, so MuPDF shows all of it.
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /CropBox [0 0 50 50] /Rotate 90 >>".to_string(),
        "<< /Type /Page /Parent 5 0 R /Contents 4 0 R >>".to_string(),
        stream("0 1 0 rg 0 0 200 100 re f"),
        "<< /Type /Pages /Kids [] /Count 0 /MediaBox [0 0 200 100] >>".to_string(),
    ];
    let (mut engine, mut session) = session_with(classic(&objects));
    let (reply, pixels) = render(&mut engine, &mut session, 0);
    // MuPDF's page: 200 x 100, unrotated, filled.
    assert_eq!(
        (reply["width"].as_f64(), reply["height"].as_f64()),
        (Some(200.0), Some(100.0))
    );
    assert_eq!(ink(&pixels, 200, (0, 0, 200, 100)), 200 * 100);
}

#[test]
fn a_page_without_parent_inherits_nothing_as_mupdf_reads_it() {
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 300] \
         /Resources << /XObject << /X0 5 0 R >> >> >>"
            .to_string(),
        "<< /Type /Page /Contents 4 0 R >>".to_string(),
        stream("/X0 Do"),
        blue_form(300),
    ];
    let (mut engine, mut session) = session_with(classic(&objects));
    let (reply, pixels) = render(&mut engine, &mut session, 0);
    // No MediaBox through /Parent: Letter; no resources: nothing drawn.
    assert_eq!(
        (reply["width"].as_f64(), reply["height"].as_f64()),
        (Some(612.0), Some(792.0))
    );
    assert_eq!(ink(&pixels, 612, (0, 0, 612, 792)), 0);
}

#[test]
fn without_a_page_tree_only_page_objects_are_pages() {
    // /Pages has no /Kids: the pages are the /Type /Page objects in object
    // order; the Text annotation with /Contents (object 4) is not one.
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Count 1 /Resources << >> >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 5 0 R /Annots [4 0 R] >>"
            .to_string(),
        "<< /Type /Annot /Subtype /Text /Rect [10 10 30 30] /Contents (my note) >>".to_string(),
        stream("0 0 1 rg 0 0 50 50 re f"),
        "<< /Type /Page /MediaBox [0 0 300 100] /Contents 5 0 R >>".to_string(),
        "<< /MediaBox [0 0 400 400] /Contents 5 0 R >>".to_string(),
    ];
    // The second page is written first: hayro alone lists by file offset.
    let (mut engine, mut session) = session_with(classic_in(&objects, &[0, 1, 5, 4, 3, 2, 6]));
    let doc = session.doc.as_mut().unwrap();
    assert_eq!(doc.page_ids(), [Some(id(3)), Some(id(6))]);
    let sizes = doc.page_state().sizes;
    assert_eq!(sizes, [(200.0, 200.0), (300.0, 100.0)]);
    let view = doc.view().unwrap();
    assert_eq!(view.page_count(), 2);
    assert_eq!(view.page_size(0).unwrap(), (200.0, 200.0));
    assert_eq!(view.page_size(1).unwrap(), (300.0, 100.0));
    let (_, pixels) = render(&mut engine, &mut session, 1);
    assert_eq!(ink(&pixels, 300, (0, 50, 50, 100)), 50 * 50);
    // No page object at all: no pages (open then refuses the document, as
    // engine.ts does), where hayro alone fails to read it.
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Count 0 >>".to_string(),
    ];
    let mut doc = AppDocument::load(Arc::new(classic(&objects)), b"").unwrap();
    assert_eq!(doc.page_count(), 0);
    assert_eq!(doc.view().unwrap().page_count(), 0);
}

/// One page drawing `text` (hex codes) in each of `faces`, the codes
/// above 0x7F named by `differences`, with /Widths of 600.
fn coded_page(faces: &[&str], differences: &str, text: &str) -> Vec<u8> {
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
    ];
    let fonts: Vec<String> = (0..faces.len())
        .map(|i| format!("/F{i} {} 0 R", 5 + i))
        .collect();
    objects.push(format!(
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 {}] /Resources << /Font << {} >> >> \
         /Contents 4 0 R >>",
        40 * faces.len(),
        fonts.join(" ")
    ));
    let content: Vec<String> = (0..faces.len())
        .map(|i| {
            format!(
                "BT /F{i} 20 Tf 10 {} Td <{text}> Tj ET",
                40 * (faces.len() - i) - 30
            )
        })
        .collect();
    objects.push(stream(&content.join("\n")));
    let widths = vec!["600"; 128].join(" ");
    for face in faces {
        objects.push(format!(
            "<< /Type /Font /Subtype /Type1 /BaseFont /{face} /FirstChar 128 /LastChar 255 \
             /Widths [{widths}] /Encoding << /BaseEncoding /WinAnsiEncoding \
             /Differences [128 {differences}] >> >>"
        ));
    }
    classic(&objects)
}

#[test]
fn cyrillic_and_greek_draw_in_the_faces_the_engine_has_urw_outlines_for() {
    // Code 0x80: MuPDF's name for А, 0x81 the URW one, 0x82 Greek alpha.
    let faces = [
        "Helvetica",
        "Helvetica-Bold",
        "Helvetica-Oblique",
        "Helvetica-BoldOblique",
        "Times-Roman",
        "Courier",
        "Times-Bold",
    ];
    let bytes = coded_page(&faces, "/Acyrillic /afii10017 /alpha", "808182");
    let view = ViewDocument::open(bytes, "").unwrap();
    let image = view.render(0, 1.0, RenderOptions::default()).unwrap();
    let row = |i: usize| ink(&image.pixels, 400, (0, 40 * i, 400, 40 * i + 40));
    for (i, face) in faces.iter().enumerate() {
        let drawn = row(i);
        // Courier and Times-Bold have Latin glyphs only (hayro's Foxit fonts).
        if i < 5 {
            assert!(drawn > 150, "{face}: {drawn} pixels");
        } else {
            assert_eq!(drawn, 0, "{face}");
        }
    }
    let text = view.structured_text(0).unwrap().as_text();
    assert_eq!(text.matches("ААα").count(), faces.len(), "{text:?}");
}

#[test]
fn objects_an_edit_nests_too_deep_for_hayro_are_cut_in_the_view() {
    // The first view finds no deep object; an edit then replaces the page
    // with a new object nested 100 levels deep, which hayro skips (the page
    // list empties) unless the view cuts it. The set of deep objects
    // follows the edit and its undo, and an open operation's writes.
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R >>".to_string(),
        stream("0 0 1 rg 0 0 100 100 re f"),
    ];
    let (mut engine, mut session) = session_with(classic(&objects));
    assert_eq!(session.view().unwrap().page_count(), 1);
    let deepen = |session: &mut AppSession, look: bool| {
        let mut deep = Object::Null;
        for _ in 0..100 {
            deep = Object::Array(vec![deep]);
        }
        session
            .mutate("deepen", |session| {
                let doc = session.doc.as_ref().unwrap().document();
                let mut page = doc.resolve_id(id(3)).clone();
                if let Some(page) = page.as_dict_mut() {
                    page.insert(b"Deep".to_vec(), deep);
                }
                let mut tx = session.tx()?;
                let new = tx.allocate(page, None).unwrap();
                tx.update(id(2), |pages| {
                    let kids = Object::Array(vec![Object::Reference(new)]);
                    pages.as_dict_mut().unwrap().insert(b"Kids".to_vec(), kids);
                })
                .unwrap();
                if look {
                    assert_eq!(session.view()?.page_count(), 1, "inside the operation");
                }
                Ok(())
            })
            .unwrap();
    };
    let filled = |engine: &mut AppEngine, session: &mut AppSession| {
        let (_, pixels) = render(engine, session, 0);
        assert_eq!(ink(&pixels, 100, (0, 0, 100, 100)), 100 * 100);
    };
    deepen(&mut session, false);
    filled(&mut engine, &mut session);
    let (reply, _) = engine.dispatch(&mut session, json!({"op": "undo"}), Vec::new());
    assert_eq!(reply["ok"], true);
    filled(&mut engine, &mut session);
    deepen(&mut session, true);
    filled(&mut engine, &mut session);
}
