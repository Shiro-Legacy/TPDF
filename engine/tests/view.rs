// SPDX-License-Identifier: AGPL-3.0-or-later
//! hayro-backed view module against values measured with MuPDF 1.28.1
//! (node `mupdf`, the app's engine). See reference/app-view-20261014.
use std::sync::mpsc;
use std::time::Duration;
use tandem_pdf_engine::view::{
    MAX_RENDER_PIXELS, MAX_SEARCH_QUADS, RenderOptions, ViewDocument, ViewError,
};

const PASSWORD: &str = "pdf4all-user";

fn fixture(name: &str) -> ViewDocument {
    let bytes = std::fs::read(format!("../fixtures/{name}")).unwrap();
    ViewDocument::open(bytes, PASSWORD).unwrap()
}

/// A classic-xref PDF with one page per content stream and Helvetica as /F1.
fn pdf(media: &str, pages: &[(&str, &str)], extra: &[&str]) -> Vec<u8> {
    pdf_res(media, "<< /Font << /F1 3 0 R >> >>", pages, extra)
}

/// As `pdf`, with `resources` as every page's /Resources value. Helvetica is
/// 3 0 R and the extra objects are numbered from 4.
fn pdf_res(media: &str, resources: &str, pages: &[(&str, &str)], extra: &[&str]) -> Vec<u8> {
    let first = 4 + extra.len();
    let kids: Vec<String> = (0..pages.len())
        .map(|i| format!("{} 0 R", first + 2 * i + 1))
        .collect();
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            kids.join(" "),
            pages.len()
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_string(),
    ];
    objects.extend(extra.iter().map(|s| s.to_string()));
    for (i, (content, page_extra)) in pages.iter().enumerate() {
        objects.push(format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len() + 1
        ));
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox {media} {page_extra} \
             /Resources {resources} /Contents {} 0 R >>",
            first + 2 * i
        ));
    }
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n{body}\nendobj\n", i + 1).bytes());
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
    for offset in offsets {
        out.extend(format!("{offset:010} 00000 n \n").bytes());
    }
    out.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .bytes(),
    );
    out
}

fn open(bytes: Vec<u8>) -> ViewDocument {
    ViewDocument::open(bytes, "").unwrap()
}

fn stream(dict: &str, data: &str) -> String {
    format!(
        "<< {dict} /Length {} >>\nstream\n{data}\nendstream",
        data.len()
    )
}

fn size(doc: &ViewDocument, page: usize, scale: f64) -> (u32, u32) {
    let image = doc.render(page, scale, RenderOptions::default()).unwrap();
    (image.width, image.height)
}

fn close(a: &[f64], b: &[f64]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.01)
}

const SCALES: [f64; 5] = [0.25, 1.0, 1.5, 4.0, 1.173];
/// Pixel sizes at each of `SCALES`.
type Sizes = [(u32, u32); 5];

#[test]
fn render_size_matches_mupdf_on_fixtures() {
    let cases: [(&str, usize, Sizes); 4] = [
        (
            "hello.pdf",
            0,
            [
                (153, 198),
                (612, 792),
                (918, 1188),
                (2448, 3168),
                (718, 930),
            ],
        ),
        (
            "edit-tj.pdf",
            1,
            [
                (198, 153),
                (792, 612),
                (1188, 918),
                (3168, 2448),
                (930, 718),
            ],
        ),
        (
            "p3/form-design.pdf",
            0,
            [
                (135, 180),
                (540, 720),
                (810, 1080),
                (2160, 2880),
                (634, 845),
            ],
        ),
        (
            "p3/form-design.pdf",
            1,
            [
                (198, 153),
                (792, 612),
                (1188, 918),
                (3168, 2448),
                (930, 718),
            ],
        ),
    ];
    for (name, page, sizes) in cases {
        let doc = fixture(name);
        for (scale, want) in SCALES.into_iter().zip(sizes) {
            assert_eq!(
                size(&doc, page, scale),
                want,
                "{name} page {page} at {scale}"
            );
        }
    }
}

#[test]
fn render_size_follows_mupdf_page_geometry() {
    let content = "BT /F1 24 Tf 30 30 Td (Geo) Tj ET";
    let cases: [(&str, [(u32, u32); 4]); 11] = [
        (
            "/CropBox [10.3 20.7 300.55 400.25]",
            [(73, 95), (291, 380), (436, 570), (1161, 1519)],
        ),
        (
            "/Rotate 90 /CropBox [10.3 20.7 300.55 400.25]",
            [(95, 73), (380, 291), (570, 436), (1519, 1161)],
        ),
        (
            "/Rotate -90",
            [(126, 101), (501, 401), (751, 601), (2001, 1602)],
        ),
        (
            "/Rotate 44",
            [(101, 126), (401, 501), (601, 751), (1602, 2001)],
        ),
        (
            "/Rotate 135",
            [(101, 126), (401, 501), (601, 751), (1602, 2001)],
        ),
        // A real /Rotate rounds half up before snapping, as MuPDF's pdf_to_int.
        (
            "/Rotate 44.6",
            [(126, 101), (501, 401), (751, 601), (2001, 1602)],
        ),
        (
            "/Rotate 134.6",
            [(101, 126), (401, 501), (601, 751), (1602, 2001)],
        ),
        (
            "/Rotate 180 /UserUnit 2",
            [(201, 251), (801, 1001), (1202, 1501), (3204, 4002)],
        ),
        (
            "/UserUnit 0.5 /Rotate 270",
            [(63, 51), (251, 201), (376, 301), (1001, 801)],
        ),
        (
            "/CropBox [0 0 0 0]",
            [(153, 198), (612, 792), (918, 1188), (2448, 3168)],
        ),
        ("/CropBox [0 0 0.5 0.5]", [(1, 1), (1, 1), (2, 2), (4, 4)]),
    ];
    let pages: Vec<(&str, &str)> = cases.iter().map(|(extra, _)| (content, *extra)).collect();
    let doc = open(pdf("[0 0 400.5 500.25]", &pages, &[]));
    for (page, (extra, sizes)) in cases.iter().enumerate() {
        for (scale, want) in SCALES.into_iter().zip(sizes) {
            assert_eq!(size(&doc, page, scale), *want, "{extra} at {scale}");
        }
    }
    let (w, h) = doc.page_size(0).unwrap();
    assert!(
        (w - 290.25).abs() < 1e-3 && (h - 379.55).abs() < 1e-3,
        "{w} {h}"
    );
}

#[test]
fn render_is_opaque_rgba_on_white() {
    let doc = fixture("hello.pdf");
    let image = doc.render(0, 1.0, RenderOptions::default()).unwrap();
    assert_eq!(image.pixels.len(), 612 * 792 * 4);
    assert!(image.pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
    let at = |x: usize, y: usize| &image.pixels[(y * 612 + x) * 4..][..3];
    assert_eq!(at(0, 0), [255, 255, 255]);
    assert_eq!(at(611, 791), [255, 255, 255]);
    let ink = image
        .pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[..3] != [255, 255, 255])
        .count();
    assert!(ink > 1000, "{ink}");
    // "PDF4All" sits at x 72..204, y 53..103 in page points (top-left origin).
    let in_text = (53..103)
        .flat_map(|y| (72..204).map(move |x| (x, y)))
        .filter(|&(x, y)| at(x, y) != [255, 255, 255])
        .count();
    assert_eq!(in_text, ink);
}

#[test]
fn render_options_and_no_view_annotations() {
    let square = |flags: u32| {
        format!(
            "<< /Type /Annot /Subtype /Square /Rect [100 600 200 700] /F {flags} /AP << /N {} 0 R >> >>",
            if flags == 4 { 6 } else { 7 }
        )
    };
    let ap = "<< /Type /XObject /Subtype /Form /BBox [0 0 100 100] /Length 23 >>\nstream\n0 g 0 0 100 100 re f\nendstream";
    let extra = [square(4), square(36), ap.to_string(), ap.to_string()];
    let extra: Vec<&str> = extra.iter().map(String::as_str).collect();
    let bytes = pdf(
        "[0 0 612 792]",
        &[("", "/Annots [4 0 R]"), ("", "/Annots [5 0 R]")],
        &extra,
    );
    let doc = open(bytes);
    let dark = |page: usize, annotations: bool| {
        let image = doc
            .render(page, 1.0, RenderOptions { annotations })
            .unwrap();
        image.pixels[(150 * 612 + 150) * 4]
    };
    assert_eq!(dark(0, true), 0);
    assert_eq!(dark(0, false), 255);
    // NoView (32) hides the appearance, as MuPDF's View usage does.
    assert_eq!(dark(1, true), 255);
}

#[test]
fn encrypted_documents_need_their_password() {
    for name in [
        "encrypted.pdf",
        "encrypted-aes256.pdf",
        "encrypted-noedit.pdf",
    ] {
        let bytes = std::fs::read(format!("../fixtures/{name}")).unwrap();
        assert_eq!(
            ViewDocument::open(bytes.clone(), "").err(),
            Some(ViewError::NeedsPassword)
        );
        assert_eq!(
            ViewDocument::open(bytes.clone(), "wrong").err(),
            Some(ViewError::NeedsPassword)
        );
        let doc = ViewDocument::open(bytes, PASSWORD).unwrap();
        assert_eq!(doc.page_count(), 1);
        assert!(doc.render(0, 0.25, RenderOptions::default()).is_ok());
    }
    let text = fixture("encrypted-aes256.pdf").structured_text(0).unwrap();
    assert_eq!(text.as_text(), "PDF4All encrypted AES-256\n\n");
    assert!(matches!(
        ViewDocument::open(b"not a pdf at all".to_vec(), ""),
        Err(ViewError::Invalid(_))
    ));
    assert_eq!(
        fixture("hello.pdf").render(1, 1.0, RenderOptions::default()),
        Err(ViewError::PageOutOfRange { page: 1, count: 1 })
    );
    assert_eq!(
        fixture("hello.pdf").render(0, f64::NAN, RenderOptions::default()),
        Err(ViewError::BadScale)
    );
}

#[test]
fn page_text_matches_mupdf() {
    let cases = [
        ("hello.pdf", 0, "PDF4All\n\n"),
        (
            "p6/paragraph.pdf",
            0,
            "The quick brown fox jumps over\nthe lazy dog while the cat\nwatches from a sunny window\nand the birds sing.\n\nBelow the paragraph\n\nMove me\n\nHead runtail run\n\n",
        ),
        (
            "p6/paragraph.pdf",
            1,
            "Tagged line one\nTagged line two\nTagged line three\n\nLeading one\nLeading two\nLeading three\n\nMiddle block\nCentred text\n\nKerned line one\nKerned line two\n\nTD one\nTD two\nAfter TD\n\n",
        ),
        (
            "edit-tj.pdf",
            0,
            "Kerned TEXThere\n\nshorttail follows\n\ntighttail two\nblock\n\nMARGIN\n\nquoted\n\nscaled\n\n",
        ),
        ("rtl.pdf", 0, "PDF4All\n\nשלום PDF4All\n\nمرحبا PDF4All\n\n"),
        ("cjk.pdf", 0, "PDF4All\n\n日本語テスト\n\n中文测试\n\n"),
        (
            "rotated.pdf",
            0,
            "PDF4All rotated\n\nPDF4All rotated\n\nPDF4All rotated\n\n",
        ),
        (
            "redact.pdf",
            0,
            "visible P0B-SECRET-7731 here\n\ninvisible P0B-SECRET-7731\n\nwhite P0B-SECRET-7731\n\nunderimg P0B-SECRET-7731\n\nformxo P0B-SECRET-7731\n\nlayer P0B-SECRET-7731\n\nP0B-SECRET-7731\n\nP4-ALT-4242\n\n",
        ),
        ("p3/field-defs.pdf", 0, ""),
    ];
    for (name, page, want) in cases {
        let text = fixture(name).structured_text(page).unwrap();
        assert_eq!(text.as_text(), want, "{name} page {page}");
    }
}

#[test]
fn structure_actual_text_keeps_the_matching_glyphs() {
    // MCID 0 maps to a StructElem whose ActualText is the glyphs' suffix.
    let text = fixture("redact.pdf").structured_text(0).unwrap();
    let line = text
        .blocks()
        .iter()
        .flat_map(|b| &b.lines)
        .find(|l| {
            l.chars
                .first()
                .is_some_and(|c| c.c == 'P' && c.origin[1] == 632.0)
        })
        .unwrap();
    assert!((line.chars[0].origin[0] - 112.032).abs() < 0.01);
}

#[test]
fn line_summaries_match_mupdf_parse_lines() {
    let summary = |name: &str| {
        fixture(name)
            .structured_text(0)
            .unwrap()
            .line_summaries()
            .into_iter()
            .map(|l| {
                format!(
                    "{}|{} {} {} {} {} {} {} {}",
                    l.text, l.x, l.y, l.w, l.h, l.size, l.family, l.weight, l.style
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        summary("hello.pdf"),
        ["PDF4All|72 92 132 49 36 sans-serif normal normal"]
    );
    assert_eq!(
        summary("rtl.pdf"),
        [
            "PDF4All|72 52 73 27 20 sans-serif normal normal",
            "שלום|72 132 58 38 28 serif normal normal",
            " PDF4All|160 132 100 38 28 serif normal normal",
            "مرحبا|72 202 60 59 28 serif normal normal",
            " PDF4All|170 202 100 38 28 serif normal normal",
        ]
    );
    assert_eq!(
        summary("rotated.pdf"),
        [
            "PDF4All rotated|72 92 211 41 30 sans-serif normal normal",
            "PDF4All rotated|387 592 41 211 30 sans-serif normal normal",
            "PDF4All rotated|97 432 178 178 30 sans-serif normal normal",
        ]
    );
    assert_eq!(
        summary("p6/paragraph.pdf")[6],
        "Head runtail run|300 342 78 15 11 sans-serif normal normal"
    );
}

#[test]
fn spans_split_lines_at_text_objects_and_fonts() {
    let doc = open(pdf(
        "[0 0 612 792]",
        &[
            (
                "BT /F1 12 Tf 72 700 Td (Hello) Tj ET BT /F1 12 Tf 105 700 Td (World) Tj ET",
                "",
            ),
            (
                "BT /F1 12 Tf 72 700 Td (Hello) Tj 33 0 Td (World) Tj ET",
                "",
            ),
            ("BT /F1 12 Tf 72 700 Td (Red) Tj 1 0 0 rg (Blue) Tj ET", ""),
        ],
        &[],
    ));
    let texts = |page: usize| {
        let text = doc.structured_text(page).unwrap();
        let lines: Vec<String> = text.line_summaries().into_iter().map(|l| l.text).collect();
        (text.as_text(), lines)
    };
    assert_eq!(
        texts(0),
        (
            "Hello World\n\n".into(),
            vec!["Hello".into(), " World".into()]
        )
    );
    assert_eq!(
        texts(1),
        ("Hello World\n\n".into(), vec!["Hello World".into()])
    );
    assert_eq!(
        texts(2),
        ("RedBlue\n\n".into(), vec!["Red".into(), "Blue".into()])
    );
}

#[test]
fn layout_and_clipping_follow_mupdf() {
    let doc = open(pdf(
        "[0 0 612 792]",
        &[
            // Indent over half a point opens a paragraph; outdent does not.
            (
                "BT /F1 12 Tf 72 700 Td (First line of para) Tj 0 -14 Td (second line) Tj 20 -14 Td (indented) Tj -20 -14 Td (back) Tj ET",
                "",
            ),
            // Glyphs outside the clip drop out; invisible and fill+stroke text count once.
            (
                "q 50 580 30 40 re W n BT /F1 12 Tf 72 600 Td (PartClip) Tj ET Q \
                 BT 3 Tr /F1 12 Tf 72 500 Td (Invis) Tj ET BT 2 Tr /F1 12 Tf 72 400 Td (FillStroke) Tj ET",
                "",
            ),
            // Character spacing over 0.15 em synthesizes spaces; TJ gaps over 0.8 em break lines.
            (
                "BT /F1 12 Tf 2 Tc 72 700 Td (Spaced) Tj ET BT /F1 12 Tf 72 600 Td [(A) -200 (B) -500 (C) -1000 (D)] TJ ET",
                "",
            ),
            // Text wholly off the page is not extracted.
            ("BT /F1 12 Tf -100 700 Td (Outside) Tj ET", ""),
        ],
        &[],
    ));
    let text = |page: usize| doc.structured_text(page).unwrap().as_text();
    assert_eq!(
        text(0),
        "First line of para\nsecond line\n\nindented\nback\n\n"
    );
    assert_eq!(text(1), "P\n\nInvis\n\nFillStroke\n\n");
    assert_eq!(text(2), "S p a c e d\n\nA B C\nD\n\n");
    assert_eq!(text(3), "");
}

#[test]
fn search_matches_mupdf_hits_and_quads() {
    let hello = fixture("hello.pdf").structured_text(0).unwrap();
    let hits = hello.search("All", MAX_SEARCH_QUADS).hits;
    assert_eq!(hits.len(), 1);
    assert!(close(
        &hits[0][0],
        &[
            164.016, 53.3, 204.012, 53.3, 164.016, 102.764, 204.012, 102.764
        ]
    ));
    // Case-sensitive, as the app's MuPDF build.
    assert!(hello.search("pdf", MAX_SEARCH_QUADS).hits.is_empty());

    let doc = open(pdf(
        "[0 0 612 792]",
        &[(
            "BT /F1 12 Tf 72 700 Td (The Cat sat on the cat-) Tj 0 -14 Td (alog with  double  spaces) Tj 0 -14 Td (CAT cat Cat) Tj ET",
            "",
        )],
        &[],
    ));
    let text = doc.structured_text(0).unwrap();
    let count = |needle: &str| text.search(needle, MAX_SEARCH_QUADS).hits.len();
    assert_eq!(count("cat"), 2);
    assert_eq!(count("CAT"), 1);
    assert_eq!(count("cat-alog"), 0);
    assert_eq!(count("with double"), 1);
    let catalog = text.search("catalog", MAX_SEARCH_QUADS).hits;
    assert_eq!(catalog.len(), 1);
    assert!(close(
        &catalog[0][0],
        &[174.06, 79.1, 190.068, 79.1, 174.06, 95.588, 190.068, 95.588]
    ));
    assert!(close(
        &catalog[0][1],
        &[72.0, 93.1, 94.68, 93.1, 72.0, 109.588, 94.68, 109.588]
    ));
    let across = text.search("spaces CAT", MAX_SEARCH_QUADS).hits;
    assert_eq!(across[0].len(), 2);
    assert!(close(
        &across[0][0],
        &[
            168.72, 93.1, 206.736, 93.1, 168.72, 109.588, 206.736, 109.588
        ]
    ));
}

#[test]
fn rotated_page_quads_match_mupdf() {
    let text = fixture("rotated.pdf").structured_text(0).unwrap();
    let hits = text.search("rotated", MAX_SEARCH_QUADS).hits;
    let want: [[f64; 8]; 3] = [
        [190.35, 59.75, 283.74, 59.75, 190.35, 100.97, 283.74, 100.97],
        [
            387.75, 473.65, 387.75, 380.26, 428.97, 473.65, 428.97, 380.26,
        ],
        [
            180.882, 325.51, 246.919, 259.473, 210.029, 354.657, 276.066, 288.62,
        ],
    ];
    assert_eq!(hits.len(), 3);
    for (hit, want) in hits.iter().zip(want) {
        assert_eq!(hit.len(), 1);
        assert!(close(&hit[0], &want), "{:?}", hit[0]);
    }
    let vertical = &text.blocks()[1].lines[0];
    assert!(close(&vertical.dir, &[0.0, -1.0]));
    let p = &vertical.chars[0];
    assert!(close(&p.origin, &[420.0, 592.0]));
    assert!(close(
        &p.quad,
        &[387.75, 592.0, 387.75, 571.99, 428.97, 592.0, 428.97, 571.99]
    ));
    let capped = text.search("rotated", 2);
    assert_eq!((capped.hits.len(), capped.truncated), (2, true));
}

/// Runs `work` on its own thread; the test fails if it takes over 20 seconds.
fn within_time<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(work());
    });
    rx.recv_timeout(Duration::from_secs(20))
        .expect("view call did not return")
}

#[test]
fn self_invoking_and_exploding_content_is_refused() {
    // A form without /Resources inherits the page's, whose /X0 is the form itself.
    let inherit = pdf_res(
        "[0 0 200 200]",
        "<< /XObject << /X0 4 0 R >> /Font << /F1 3 0 R >> >>",
        &[("/X0 Do", "")],
        &[&stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 200 200]",
            "BT /F1 12 Tf 20 20 Td (R) Tj ET /X0 Do /X0 Do",
        )],
    );
    // The page and the form share one indirect /Resources.
    let shared = pdf_res(
        "[0 0 200 200]",
        "5 0 R",
        &[("/X0 Do", "")],
        &[
            &stream(
                "/Type /XObject /Subtype /Form /BBox [0 0 200 200] /Resources 5 0 R",
                "BT /F1 12 Tf 20 20 Td (S) Tj ET /X0 Do /X0 Do",
            ),
            "<< /XObject << /X0 4 0 R >> /Font << /F1 3 0 R >> >>",
        ],
    );
    // A Type3 glyph that shows itself; text extraction does not run glyph procedures.
    let type3 = pdf_res(
        "[0 0 200 200]",
        "6 0 R",
        &[("BT /T3 12 Tf 20 20 Td (A) Tj ET", "")],
        &[
            "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1000 1000] \
             /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << /A 5 0 R >> \
             /Encoding << /Differences [65 /A] >> /FirstChar 65 /LastChar 65 \
             /Widths [1000] /Resources 6 0 R >>",
            &stream("", "1000 0 d0 BT /T3 1 Tf (A) Tj ET"),
            "<< /Font << /T3 4 0 R /F1 3 0 R >> >>",
        ],
    );
    // 25 nested forms that each draw the next twice: 2^25 runs and no cycle.
    let forms: Vec<String> = (0..25)
        .map(|i| match i {
            24 => stream(
                "/Type /XObject /Subtype /Form /BBox [0 0 10 10]",
                "0 0 1 1 re f",
            ),
            _ => stream(
                &format!(
                    "/Type /XObject /Subtype /Form /BBox [0 0 10 10] \
                     /Resources << /XObject << /X {} 0 R >> >>",
                    5 + i
                ),
                "/X Do /X Do",
            ),
        })
        .collect();
    let forms: Vec<&str> = forms.iter().map(String::as_str).collect();
    let dag = pdf_res(
        "[0 0 200 200]",
        "<< /XObject << /X 4 0 R >> >>",
        &[("/X Do", "")],
        &forms,
    );
    // A form listing itself in its own /Resources: hayro does not resolve the
    // inner reference, and MuPDF skips the recursive draw; both give the text once.
    let own = pdf_res(
        "[0 0 200 200]",
        "<< /XObject << /X0 4 0 R >> /Font << /F1 3 0 R >> >>",
        &[("BT /F1 12 Tf 10 10 Td (Hi) Tj ET /X0 Do", "")],
        &[&stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 200 200] \
             /Resources << /XObject << /X0 4 0 R >> /Font << /F1 3 0 R >> >>",
            "BT /F1 12 Tf 20 20 Td (R) Tj ET /X0 Do",
        )],
    );
    let results = within_time(move || {
        [inherit, shared, type3, dag, own].map(|bytes| {
            let doc = open(bytes);
            (
                doc.render(0, 0.25, RenderOptions::default()).err(),
                doc.structured_text(0).map(|t| t.as_text()),
            )
        })
    });
    let cycle = ViewError::TooComplex("page content refers to itself");
    let budget = ViewError::TooComplex("page content expands past the work limit");
    assert_eq!(results[0], (Some(cycle.clone()), Err(cycle.clone())));
    assert_eq!(results[1], (Some(cycle.clone()), Err(cycle.clone())));
    assert_eq!(results[2], (Some(cycle), Ok("A\n\n".into())));
    assert_eq!(results[3], (Some(budget.clone()), Err(budget)));
    assert_eq!(results[4], (None, Ok("Hi\n\nR\n\n".into())));
}

#[test]
fn search_reads_clipped_and_off_page_text() {
    let doc = open(pdf(
        "[0 0 612 792]",
        &[(
            "q 0 0 1 1 re W n BT /F1 12 Tf 72 650 Td (SECRET-CLIPPED-1234) Tj ET Q \
             BT /F1 12 Tf 72 900 Td (SECRET-OFFPAGE-5678) Tj ET \
             q 50 580 30 40 re W n BT /F1 12 Tf 72 600 Td (PartClip) Tj ET Q \
             BT /F1 12 Tf -100 500 Td (Outside) Tj ET",
            "",
        )],
        &[],
    ));
    let text = doc.structured_text(0).unwrap();
    // MuPDF's page text keeps only the visible "P"; its page.search finds all four.
    assert_eq!(text.as_text(), "P\n\n");
    let cases: [(&str, [f64; 8]); 4] = [
        (
            "SECRET-CLIPPED-1234",
            [72.0, 129.1, 206.7, 129.1, 72.0, 145.588, 206.7, 145.588],
        ),
        (
            "SECRET-OFFPAGE-5678",
            [72.0, -120.9, 212.7, -120.9, 72.0, -104.412, 212.7, -104.412],
        ),
        (
            "PartClip",
            [72.0, 179.1, 114.672, 179.1, 72.0, 195.588, 114.672, 195.588],
        ),
        (
            "Outside",
            [
                -100.0, 279.1, -58.648, 279.1, -100.0, 295.588, -58.648, 295.588,
            ],
        ),
    ];
    for (needle, quad) in cases {
        let hits = text.search(needle, MAX_SEARCH_QUADS).hits;
        assert_eq!(hits.len(), 1, "{needle}");
        assert!(close(&hits[0][0], &quad), "{needle} {:?}", hits[0]);
    }
    // edit-tj.pdf page 3 draws "(unbalanced)" above the page.
    let hits = fixture("edit-tj.pdf")
        .structured_text(2)
        .unwrap()
        .search("unbalanced", MAX_SEARCH_QUADS)
        .hits;
    assert_eq!(hits.len(), 1);
    assert!(close(
        &hits[0][0],
        &[
            244.0, -733.8, 368.08, -733.8, 244.0, -700.824, 368.08, -700.824
        ]
    ));
}

#[test]
fn search_whitespace_needles_and_truncation() {
    let doc = open(pdf(
        "[0 0 612 792]",
        &[
            (
                "BT /F1 12 Tf 72 700 Td (Hello world  again) Tj 0 -14 Td (next line) Tj ET",
                "",
            ),
            (
                "BT /F1 12 Tf 72 700 Td (one cap-) Tj 0 -14 Td (ture two cap-) Tj \
                 0 -14 Td (ture three cap-) Tj 0 -14 Td (ture) Tj ET",
                "",
            ),
        ],
        &[],
    ));
    let text = doc.structured_text(0).unwrap();
    // Line feed, carriage return and tab in a needle match whitespace, as in MuPDF.
    assert_eq!(text.search("\n", MAX_SEARCH_QUADS).hits.len(), 3);
    for needle in ["again\nnext", "again\r\nnext", "again next"] {
        let hits = text.search(needle, MAX_SEARCH_QUADS).hits;
        assert_eq!(hits.len(), 1, "{needle:?}");
        assert!(close(
            &hits[0].concat(),
            &[
                138.012, 79.1, 167.364, 79.1, 138.012, 95.588, 167.364, 95.588, 72.0, 93.1, 94.68,
                93.1, 72.0, 109.588, 94.68, 109.588
            ]
        ));
    }
    let tab = text.search("o\tw", MAX_SEARCH_QUADS).hits;
    assert!(close(
        &tab.concat().concat(),
        &[92.664, 79.1, 111.336, 79.1, 92.664, 95.588, 111.336, 95.588]
    ));

    // Three matches of two quads each. MuPDF's page.search returns the same six
    // quads but reports every match after the first as one hit per quad.
    let text = doc.structured_text(1).unwrap();
    let all = text.search("capture", MAX_SEARCH_QUADS);
    assert!(!all.truncated);
    assert_eq!(all.hits.iter().map(Vec::len).collect::<Vec<_>>(), [2, 2, 2]);
    let want = [
        [95.352, 79.1, 114.696, 79.1, 95.352, 95.588, 114.696, 95.588],
        [72.0, 93.1, 92.676, 93.1, 72.0, 109.588, 92.676, 109.588],
        [
            118.02, 93.1, 137.364, 93.1, 118.02, 109.588, 137.364, 109.588,
        ],
        [72.0, 107.1, 92.676, 107.1, 72.0, 123.588, 92.676, 123.588],
        [
            126.696, 107.1, 146.04, 107.1, 126.696, 123.588, 146.04, 123.588,
        ],
        [72.0, 121.1, 92.676, 121.1, 72.0, 137.588, 92.676, 137.588],
    ];
    assert!(close(&all.hits.concat().concat(), &want.concat()));
    // A hit that does not fit is left out whole and reported.
    let capped = text.search("capture", 5);
    assert_eq!(capped.hits, all.hits[..2]);
    assert!(capped.truncated);
    let exact = text.search("capture", 6);
    assert_eq!((exact.hits.len(), exact.truncated), (3, false));
    let none = text.search("capture", 0);
    assert!(none.hits.is_empty() && none.truncated);
    assert!(!text.search("absent", 0).truncated);
}

#[test]
fn oversized_images_are_refused_before_allocation() {
    let doc = open(pdf(
        "[0 0 612 792]",
        &[
            ("", "/MediaBox [0 0 65535 16385]"),
            ("", "/MediaBox [0 0 14400 3600]"),
            ("", "/MediaBox [0 0 8192 8193]"),
        ],
        &[],
    ));
    assert_eq!(
        doc.render(0, 1.0, RenderOptions::default()),
        Err(ViewError::BadScale)
    );
    assert_eq!(
        doc.render(1, 4.0, RenderOptions::default()),
        Err(ViewError::BadScale)
    );
    // 8192 x 8193 pixels is one row over the budget.
    assert_eq!(8192 * 8192, MAX_RENDER_PIXELS);
    assert_eq!(
        doc.render(2, 1.0, RenderOptions::default()),
        Err(ViewError::BadScale)
    );
    assert_eq!(size(&doc, 2, 0.25), (2048, 2049));
}

#[test]
fn owner_password_opens_encrypted_documents() {
    for (name, text) in [
        ("encrypted.pdf", "PDF4All encrypted\n\n"),
        ("encrypted-noedit.pdf", "PDF4All encrypted no-edit\n\n"),
        ("encrypted-noprint.pdf", "PDF4All encrypted no-print\n\n"),
        ("encrypted-aes256.pdf", "PDF4All encrypted AES-256\n\n"),
    ] {
        let bytes = std::fs::read(format!("../fixtures/{name}")).unwrap();
        let doc = ViewDocument::open(bytes.clone(), "pdf4all-owner").unwrap();
        assert_eq!(doc.structured_text(0).unwrap().as_text(), text, "{name}");
        assert_eq!(
            ViewDocument::open(bytes, "pdf4all-owner2").err(),
            Some(ViewError::NeedsPassword),
            "{name}"
        );
    }
}

#[test]
fn spans_layout_matches_mupdf_preserve_spans() {
    let rtl = fixture("rtl.pdf").structured_text(0).unwrap();
    assert_eq!(
        rtl.spans_text(),
        "PDF4All\n\nשלום\n PDF4All\n\nمرحبا\n PDF4All\n\n"
    );
    assert_eq!(
        rtl.line_summaries()
            .iter()
            .map(|l| l.top)
            .collect::<Vec<_>>(),
        [30, 102, 102, 163, 172]
    );
    let quartz = fixture("p6/quartz.pdf").structured_text(0).unwrap();
    assert_eq!(
        quartz.spans_text(),
        include_str!("../reference/app-view-20261014/quartz-spans.txt")
    );
    let lines: usize = quartz.span_blocks().iter().map(|b| b.lines.len()).sum();
    assert_eq!(lines, quartz.line_summaries().len());
    let hello = fixture("hello.pdf").structured_text(0).unwrap();
    assert_eq!(hello.line_summaries()[0].top, 53);
}

#[test]
fn type3_text_comes_from_encoding_names_and_widths() {
    // Codes 161 to 169 name these glyphs; every glyph draws the same procedure.
    let names = [
        "uni0041", "a67", "A.sc", "f_i", "T_h", "foo", "a5", "u00E9", "alpha",
    ];
    let procs: String = ["A", "B", "space"]
        .iter()
        .chain(&names)
        .map(|n| format!("/{n} 5 0 R "))
        .collect();
    let font = format!(
        "<< /Type /Font /Subtype /Type3 /FontBBox [0 -100 750 800] \
         /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << {procs}>> \
         /Encoding << /Type /Encoding /Differences [32 /space 65 /A /B 161 /{}] >> \
         /FirstChar 32 /LastChar 169 /Widths [250 {} 600 700 {}] >>",
        names.join(" /"),
        ["0"; 32].join(" "),
        ["500"; 103].join(" ")
    );
    let doc = open(pdf_res(
        "[0 0 612 792]",
        "<< /Font << /F1 3 0 R /T3 4 0 R >> >>",
        &[(
            "BT /T3 20 Tf 72 700 Td (ABA B) Tj ET BT /T3 20 Tf 72 600 Td (AB) Tj ET \
             BT /T3 20 Tf 2 Tc 72 500 Td [(A) -500 (B)] TJ ET \
             BT /T3 10 Tf 72 300 Td <a1a2a3a4a5a6a7a8a9> Tj ET BT /T3 10 Tf 72 200 Td <41a0> Tj ET",
            "",
        )],
        &[&font, &stream("", "600 0 0 0 550 700 d1 0 0 550 700 re f")],
    ));
    let text = doc.structured_text(0).unwrap();
    // Measured with MuPDF 1.28.1: glyph names through the glyph list and the
    // uni/u/a forms, unnamed printable ASCII codes as themselves, /Widths advances.
    assert_eq!(
        text.as_text(),
        "ABA B\n\nAB\n\nA B\n\nA C A fi T \u{fffd}\u{fffd}é α\n\nA \u{fffd}\n\n"
    );
    let first = &text.line_summaries()[0];
    assert_eq!(
        (first.x, first.y, first.top, first.w, first.h, first.size),
        (72, 92, 76, 57, 18, 20)
    );
    let ab = text.search("AB", MAX_SEARCH_QUADS).hits;
    assert!(close(
        &ab.concat().concat(),
        &[
            72.0, 76.0, 98.0, 76.0, 72.0, 94.0, 98.0, 94.0, 72.0, 176.0, 98.0, 176.0, 72.0, 194.0,
            98.0, 194.0
        ]
    ));
    let spaced = text.search("A B", MAX_SEARCH_QUADS).hits;
    assert!(close(
        &spaced.concat().concat(),
        &[
            98.0, 76.0, 129.0, 76.0, 98.0, 94.0, 129.0, 94.0, 72.0, 276.0, 110.0, 276.0, 72.0,
            294.0, 110.0, 294.0
        ]
    ));
    let font = &text.fonts()[text.blocks()[0].lines[0].chars[0].font];
    assert_eq!(font.name, "Type3 (4 0 R)");
}

#[test]
fn popup_and_no_view_annotations_are_not_drawn() {
    let annot = |subtype: &str, flags: &str| {
        format!(
            "<< /Type /Annot /Subtype /{subtype} /Rect [100 600 200 700] {flags} \
             /AP << /N 4 0 R >> >>"
        )
    };
    let extra = [
        stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 100 100]",
            "0 g 0 0 100 100 re f",
        ),
        annot("Popup", "/F 0"),
        annot("Popup", ""),
        annot("Square", "/F 8 0 R"),
        "32".to_string(),
        annot("Square", "/F 8"),
        annot("Square", "/F 40"),
    ];
    let extra: Vec<&str> = extra.iter().map(String::as_str).collect();
    let pages: Vec<String> = [5, 6, 7, 9, 10]
        .iter()
        .map(|n| format!("/Annots [{n} 0 R]"))
        .collect();
    let pages: Vec<(&str, &str)> = pages.iter().map(|p| ("", p.as_str())).collect();
    let doc = open(pdf("[0 0 612 792]", &pages, &extra));
    let centre = |page: usize| {
        let image = doc.render(page, 1.0, RenderOptions::default()).unwrap();
        image.pixels[(150 * 612 + 150) * 4]
    };
    // As MuPDF: Popups and NoView (32, also through an indirect /F) are not
    // drawn; NoZoom (8) is.
    assert_eq!([0, 1, 2, 3, 4].map(centre), [255, 255, 255, 0, 255]);
}

#[test]
fn rotated_page_text_quads_match_mupdf() {
    let content = "BT /F1 24 Tf 60 400 Td (Turn me) Tj ET \
                   BT /F1 10 Tf 1 0 0 1 200 300 Tm (second) Tj ET";
    let doc = open(pdf(
        "[0 0 612 792]",
        &[
            (content, "/Rotate 90 /CropBox [20 30 500 700]"),
            (content, "/Rotate 270 /UserUnit 2"),
            (content, "/Rotate 180 /CropBox [10 10 600 780]"),
        ],
        &[],
    ));
    // Per page: line summaries (text x y top w h size), then the quads of
    // "Turn" and "second".
    let want: [([&str; 2], [[f64; 8]; 2]); 3] = [
        (
            ["Turn me 362 40 40 32 89 24", "second 267 180 180 13 32 10"],
            [
                [395.8, 40.0, 395.8, 89.344, 362.824, 40.0, 362.824, 89.344],
                [280.75, 180.0, 280.75, 212.24, 267.01, 180.0, 267.01, 212.24],
            ],
        ),
        (
            [
                "Turn me 732 1104 925 65 178 48",
                "second 962 824 759 27 64 20",
            ],
            [
                [
                    732.4, 1104.0, 732.4, 1005.312, 798.352, 1104.0, 798.352, 1005.312,
                ],
                [962.5, 824.0, 962.5, 759.52, 989.98, 824.0, 989.98, 759.52],
            ],
        ),
        (
            [
                "Turn me 450 390 382 89 32 24",
                "second 367 290 287 32 13 10",
            ],
            [
                [
                    540.0, 415.8, 490.656, 415.8, 540.0, 382.824, 490.656, 382.824,
                ],
                [400.0, 300.75, 367.76, 300.75, 400.0, 287.01, 367.76, 287.01],
            ],
        ),
    ];
    for (page, (lines, quads)) in want.iter().enumerate() {
        let text = doc.structured_text(page).unwrap();
        assert_eq!(text.as_text(), "Turn me\n\nsecond\n\n");
        let got: Vec<String> = text
            .line_summaries()
            .iter()
            .map(|l| {
                format!(
                    "{} {} {} {} {} {} {}",
                    l.text, l.x, l.y, l.top, l.w, l.h, l.size
                )
            })
            .collect();
        assert_eq!(got, lines, "page {page}");
        for (needle, quad) in ["Turn", "second"].iter().zip(quads) {
            let hits = text.search(needle, MAX_SEARCH_QUADS).hits;
            assert_eq!(hits.len(), 1, "page {page} {needle}");
            assert!(
                close(&hits[0][0], quad),
                "page {page} {needle} {:?}",
                hits[0]
            );
        }
    }
}
