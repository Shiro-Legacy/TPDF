// SPDX-License-Identifier: AGPL-3.0-or-later
//! hayro-backed view module against values measured with MuPDF 1.28.1
//! (node `mupdf`, the app's engine). See reference/app-view-20261014.
use tandem_pdf_engine::view::{MAX_SEARCH_QUADS, RenderOptions, ViewDocument, ViewError};

const PASSWORD: &str = "pdf4all-user";

fn fixture(name: &str) -> ViewDocument {
    let bytes = std::fs::read(format!("../fixtures/{name}")).unwrap();
    ViewDocument::open(bytes, PASSWORD).unwrap()
}

/// A classic-xref PDF with one page per content stream and Helvetica as /F1.
fn pdf(media: &str, pages: &[(&str, &str)], extra: &[&str]) -> Vec<u8> {
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
             /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>",
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
    let cases: [(&str, [(u32, u32); 4]); 9] = [
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
    assert!(image.pixels.chunks_exact(4).all(|p| p[3] == 255));
    let at = |x: usize, y: usize| &image.pixels[(y * 612 + x) * 4..][..3];
    assert_eq!(at(0, 0), [255, 255, 255]);
    assert_eq!(at(611, 791), [255, 255, 255]);
    let ink = image
        .pixels
        .chunks_exact(4)
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
    let hits = hello.search("All", MAX_SEARCH_QUADS);
    assert_eq!(hits.len(), 1);
    assert!(close(
        &hits[0][0],
        &[
            164.016, 53.3, 204.012, 53.3, 164.016, 102.764, 204.012, 102.764
        ]
    ));
    // Case-sensitive, as the app's MuPDF build.
    assert!(hello.search("pdf", MAX_SEARCH_QUADS).is_empty());

    let doc = open(pdf(
        "[0 0 612 792]",
        &[(
            "BT /F1 12 Tf 72 700 Td (The Cat sat on the cat-) Tj 0 -14 Td (alog with  double  spaces) Tj 0 -14 Td (CAT cat Cat) Tj ET",
            "",
        )],
        &[],
    ));
    let text = doc.structured_text(0).unwrap();
    let count = |needle: &str| text.search(needle, MAX_SEARCH_QUADS).len();
    assert_eq!(count("cat"), 2);
    assert_eq!(count("CAT"), 1);
    assert_eq!(count("cat-alog"), 0);
    assert_eq!(count("with double"), 1);
    let catalog = text.search("catalog", MAX_SEARCH_QUADS);
    assert_eq!(catalog.len(), 1);
    assert!(close(
        &catalog[0][0],
        &[174.06, 79.1, 190.068, 79.1, 174.06, 95.588, 190.068, 95.588]
    ));
    assert!(close(
        &catalog[0][1],
        &[72.0, 93.1, 94.68, 93.1, 72.0, 109.588, 94.68, 109.588]
    ));
    let across = text.search("spaces CAT", MAX_SEARCH_QUADS);
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
    let hits = text.search("rotated", MAX_SEARCH_QUADS);
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
    assert_eq!(text.search("rotated", 2).len(), 2);
}
