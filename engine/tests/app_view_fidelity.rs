// SPDX-License-Identifier: MIT
//! The app's view matches reference viewers where hayro reads a document
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
    // draws; reference viewers read it through /Parent, hayro would not.
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
    // its /Parent is another node without one, so reference viewers show all of it.
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /CropBox [0 0 50 50] /Rotate 90 >>".to_string(),
        "<< /Type /Page /Parent 5 0 R /Contents 4 0 R >>".to_string(),
        stream("0 1 0 rg 0 0 200 100 re f"),
        "<< /Type /Pages /Kids [] /Count 0 /MediaBox [0 0 200 100] >>".to_string(),
    ];
    let (mut engine, mut session) = session_with(classic(&objects));
    let (reply, pixels) = render(&mut engine, &mut session, 0);
    // The page reference viewers show: 200 x 100, unrotated, filled.
    assert_eq!(
        (reply["width"].as_f64(), reply["height"].as_f64()),
        (Some(200.0), Some(100.0))
    );
    assert_eq!(ink(&pixels, 200, (0, 0, 200, 100)), 200 * 100);
}

#[test]
fn a_page_without_parent_inherits_nothing() {
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
    // the earlier app engine does), where hayro alone fails to read it.
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
    // Code 0x80: /Acyrillic for А, 0x81 the URW name /afii10017, 0x82 Greek alpha.
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

#[test]
fn a_direct_resources_dictionary_many_pages_inherit_is_written_once() {
    // 300 pages inherit, through a /Parent off their /Kids path, one direct
    // /Resources dictionary of 2,000 fonts and the Form their content draws.
    // Each page carrying its own copy made the view's bytes grow with pages
    // times entries (23 MB from 118 KB at 1,000 x 2,000).
    let pages = 300;
    let fonts: String = (0..2000).map(|i| format!("/F{i} 4 0 R ")).collect();
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!(
            "<< /Type /Pages /Kids [{}] /Count {pages} >>",
            (0..pages)
                .map(|i| format!("{} 0 R ", 7 + i))
                .collect::<String>()
        ),
        format!(
            "<< /Type /Pages /Kids [] /Count 0 /Resources << /XObject << /X0 5 0 R >> \
             /Font << {fonts}>> >> >>"
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        blue_form(100),
        stream("/X0 Do"),
    ];
    for _ in 0..pages {
        objects
            .push("<< /Type /Page /Parent 3 0 R /MediaBox [0 0 100 100] /Contents 6 0 R >>".into());
    }
    let input = classic(&objects);
    let (mut engine, mut session) = session_with(input.clone());
    for page in [0, pages - 1] {
        let (_, pixels) = render(&mut engine, &mut session, page);
        assert_eq!(
            ink(&pixels, 100, (0, 0, 100, 100)),
            100 * 100,
            "page {page}"
        );
    }
    // The base, the dictionary once and each page's redefinition.
    let view = session.view().unwrap().byte_len();
    assert!(
        view < 3 * input.len(),
        "view {view} bytes from {}",
        input.len()
    );
}

/// A page tree `depth` /Pages nodes deep, each holding one 10 pt page and
/// the next node, every /Parent naming the node that lists it.
fn chain(depth: usize) -> Vec<u8> {
    let node = |i: usize| 3 + i;
    let page = |i: usize| 3 + depth + i;
    let mut objects = vec![
        "<< /Type /Catalog /Pages 3 0 R >>".to_string(),
        stream("0 0 1 rg 0 0 10 10 re f"),
    ];
    for i in 0..depth {
        let next = match i + 1 < depth {
            true => format!(" {} 0 R", node(i + 1)),
            false => String::new(),
        };
        let parent = match i {
            0 => "/MediaBox [0 0 10 10]".to_string(),
            _ => format!("/Parent {} 0 R", node(i - 1)),
        };
        objects.push(format!(
            "<< /Type /Pages {parent} /Kids [{} 0 R{next}] /Count {} >>",
            page(i),
            depth - i
        ));
    }
    for i in 0..depth {
        objects.push(format!(
            "<< /Type /Page /Parent {} 0 R /Contents 2 0 R >>",
            node(i)
        ));
    }
    classic(&objects)
}

#[test]
fn a_deep_page_tree_costs_the_view_time_in_proportion_to_its_size() {
    // Checking each page's /Parent chain against its whole path made a view
    // of 20,000 levels take 10.5 s (natively, release); each node's chain
    // is now checked once.
    // Measured against a tree a tenth as deep, built and timed the same
    // way, so a loaded machine slows both alike (a wall-clock bound of 5 s
    // failed under the gates' load at 5.1 s, as on main): in proportion,
    // the deep one takes about 10 times as long; checked against whole
    // paths, about 100 times.
    let timed = |depth: usize| {
        let (engine, mut session) = session_with(chain(depth));
        let start = std::time::Instant::now();
        let view = session.view().unwrap();
        assert_eq!(view.page_count(), depth);
        (start.elapsed(), engine, session)
    };
    let (small, _, _) = timed(2_000);
    let (elapsed, mut engine, mut session) = timed(20_000);
    let depth = 20_000;
    assert!(
        elapsed < small * 40 + std::time::Duration::from_millis(200),
        "{elapsed:?} for {depth} levels, {small:?} for 2,000"
    );
    let limit = if cfg!(debug_assertions) { 600 } else { 60 };
    assert!(elapsed.as_secs() < limit, "{elapsed:?}");
    // Within the 64 levels inheritance is read through.
    let (_, pixels) = render(&mut engine, &mut session, 30);
    assert_eq!(ink(&pixels, 10, (0, 0, 10, 10)), 100);
}

/// One 200 pt page whose content is empty, listing `annots` (bodies of
/// objects 4 on) in its /Annots, with a red Form as object 4.
fn annotated(annots: &[&str]) -> Vec<u8> {
    let red = "1 0 0 rg 0 0 200 200 re f";
    let refs: String = (0..annots.len())
        .map(|i| format!("{} 0 R ", 6 + i))
        .collect();
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 5 0 R \
             /Annots [{refs}] >>"
        ),
        format!(
            "<< /Type /XObject /Subtype /Form /BBox [0 0 200 200] /Length {} >>\nstream\n{red}\nendstream",
            red.len()
        ),
        stream(""),
    ];
    objects.extend(annots.iter().map(|annot| annot.to_string()));
    classic(&objects)
}

#[test]
fn annotation_flags_are_read_as_reference_viewers_read_them() {
    // Reference viewers round a real /F half up and keep an integer's
    // low 32 bits; hayro truncates and reads anything outside u32 as 0.
    // Ink as reference viewers draw each (View usage).
    for (flags, drawn) in [
        ("32.0", false),
        ("1.6", false),
        ("2.5", false),
        ("34.4", false),
        ("3.5", true),
        ("4.0", true),
        ("-1", false),
        ("4294967328", false),
        ("4294967296.0", true),
    ] {
        let annot = format!(
            "<< /Type /Annot /Subtype /Square /Rect [0 0 200 200] /F {flags} /AP << /N 4 0 R >> >>"
        );
        let (mut engine, mut session) = session_with(annotated(&[&annot]));
        let (_, pixels) = render(&mut engine, &mut session, 0);
        let expected = if drawn { 200 * 200 } else { 0 };
        assert_eq!(ink(&pixels, 200, (0, 0, 200, 200)), expected, "/F {flags}");
    }
}

#[test]
fn a_widget_view_usage_leaves_out_gets_no_appearance_drawn() {
    // A NoView text field without an appearance: the forms view gives it
    // one, which View usage still leaves out.
    for (flags, drawn) in [(32, false), (4, true)] {
        let objects = vec![
            "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [4 0 R] \
             /DA (/Helv 12 Tf 0 g) /DR << /Font << /Helv 5 0 R >> >> >> >>"
                .to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Annots [4 0 R] \
             /Contents 6 0 R >>"
                .to_string(),
            format!(
                "<< /Type /Annot /Subtype /Widget /FT /Tx /T (a) /V (Hello) \
                 /Rect [10 10 190 190] /F {flags} /DA (/Helv 40 Tf 1 0 0 rg) /MK << /BG [0 0 1] >> >>"
            ),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
            stream(""),
        ];
        let (mut engine, mut session) = session_with(classic(&objects));
        let (_, pixels) = render(&mut engine, &mut session, 0);
        let ink = ink(&pixels, 200, (0, 0, 200, 200));
        assert_eq!(ink > 30_000, drawn, "/F {flags}: {ink}");
        assert_eq!(ink == 0, !drawn, "/F {flags}: {ink}");
    }
}

#[test]
fn pages_inside_direct_page_tree_nodes_are_fixed_where_they_are_written() {
    // The page is written in the /Kids of a /Pages node written in the
    // root's /Kids: its direct NoView or Popup annotation is hidden as
    // reference viewers hide it, a drawn one still drawn.
    for (annot, drawn) in [
        ("/Subtype /Square /F 32", false),
        ("/Subtype /Popup", false),
        ("/Subtype /Square", true),
    ] {
        let objects = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            format!(
                "<< /Type /Pages /Count 1 /Kids [ << /Type /Pages /Count 1 /Kids [ \
                 << /Type /Page /MediaBox [0 0 200 200] /Contents 4 0 R /Annots [ \
                 << /Type /Annot {annot} /Rect [0 0 200 200] /AP << /N 3 0 R >> >> ] >> ] >> ] >>"
            ),
            blue_form(200),
            stream(""),
        ];
        let (mut engine, mut session) = session_with(classic(&objects));
        let (_, pixels) = render(&mut engine, &mut session, 0);
        let expected = if drawn { 200 * 200 } else { 0 };
        assert_eq!(ink(&pixels, 200, (0, 0, 200, 200)), expected, "{annot}");
    }
    // Two levels of direct nodes, the page without /Parent or MediaBox:
    // Letter and no resources as reference viewers read it, where hayro would inherit
    // the nodes' box and Form.
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Count 2 /Kids [ 5 0 R << /Type /Pages /Count 1 /MediaBox [0 0 300 300] \
         /Resources << /XObject << /X0 3 0 R >> >> /Kids [ << /Type /Pages /Count 1 /Kids [ \
         << /Type /Page /Contents 4 0 R >> ] >> ] >> ] >>"
            .to_string(),
        blue_form(300),
        stream("/X0 Do"),
        "<< /Type /Page /MediaBox [0 0 10 10] >>".to_string(),
    ];
    let (mut engine, mut session) = session_with(classic(&objects));
    let (reply, pixels) = render(&mut engine, &mut session, 1);
    assert_eq!(
        (reply["width"].as_f64(), reply["height"].as_f64()),
        (Some(612.0), Some(792.0))
    );
    let view = session.view().unwrap();
    assert_eq!(view.page_size(1).unwrap(), (612.0, 792.0));
    assert_eq!(ink(&pixels, 612, (0, 0, 612, 792)), 0);
}

#[test]
fn a_direct_page_tree_root_is_drawn_with_the_pages_the_app_lists() {
    // hayro reads the page tree only from a /Pages reference; reference viewers
    // read a direct one too. The direct page's NoView annotation is hidden.
    let objects = vec![
        "<< /Type /Catalog /Pages << /Type /Pages /Count 2 /MediaBox [0 0 300 300] /Kids [ 2 0 R \
         << /Type /Page /MediaBox [0 0 200 200] /Contents 3 0 R /Annots [ << /Type /Annot \
         /Subtype /Square /F 32 /Rect [0 0 200 200] /AP << /N 4 0 R >> >> ] >> ] >> >>"
            .to_string(),
        "<< /Type /Page /MediaBox [0 0 300 100] /Contents 3 0 R >>".to_string(),
        stream("0 0 1 rg 0 0 50 50 re f"),
        blue_form(200),
    ];
    let (mut engine, mut session) = session_with(classic(&objects));
    let doc = session.doc.as_mut().unwrap();
    assert_eq!(doc.page_state().sizes, [(300.0, 100.0), (200.0, 200.0)]);
    let view = doc.view().unwrap();
    assert_eq!(view.page_count(), 2);
    assert_eq!(view.page_size(0).unwrap(), (300.0, 100.0));
    assert_eq!(view.page_size(1).unwrap(), (200.0, 200.0));
    let (_, pixels) = render(&mut engine, &mut session, 0);
    assert_eq!(ink(&pixels, 300, (0, 0, 300, 100)), 50 * 50);
    let (_, pixels) = render(&mut engine, &mut session, 1);
    assert_eq!(ink(&pixels, 200, (0, 0, 200, 200)), 50 * 50);
}
