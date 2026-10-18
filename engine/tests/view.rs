// SPDX-License-Identifier: AGPL-3.0-or-later
//! The hayro-backed view module: render sizes and page geometry, page
//! text, line summaries, search quads, refusals and annotation drawing.
//! Geometry is compared with the quad tolerance of spec 04 §8.
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

/// The quad tolerance of spec 04 §8: along the baseline (ll → lr of the
/// expected quad) each corner within 0.05 pt (plus rounding of the listed
/// values); across it within 0.35 em, since the ascender and descender
/// source is the implementation's.
fn quad_near(actual: &[f64], expected: &[f64], em: f64) -> bool {
    if actual.len() != 8 || expected.len() != 8 {
        return false;
    }
    let (dx, dy) = (expected[6] - expected[4], expected[7] - expected[5]);
    let length = dx.hypot(dy);
    if length == 0.0 {
        return false;
    }
    let (ux, uy) = (dx / length, dy / length);
    (0..4).all(|corner| {
        let ex = actual[2 * corner] - expected[2 * corner];
        let ey = actual[2 * corner + 1] - expected[2 * corner + 1];
        let along = ex * ux + ey * uy;
        let across = -ex * uy + ey * ux;
        along.abs() <= 0.055 && across.abs() <= 0.35 * em
    })
}

/// Every quad of `hits` (flattened) near `expected` at `em`.
fn quads_near(hits: &[Vec<[f64; 8]>], expected: &[[f64; 8]], em: f64) -> bool {
    let quads: Vec<&[f64; 8]> = hits.iter().flatten().collect();
    quads.len() == expected.len()
        && quads
            .iter()
            .zip(expected)
            .all(|(a, e)| quad_near(&a[..], e, em))
}

const SCALES: [f64; 5] = [0.25, 1.0, 1.5, 4.0, 1.173];
/// Pixel sizes at each of `SCALES`.
type Sizes = [(u32, u32); 5];

#[test]
fn render_sizes_on_fixtures() {
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
fn render_size_follows_page_geometry() {
    let content = "BT /F1 24 Tf 30 30 Td (Geo) Tj ET";
    let cases: [(&str, [(u32, u32); 4]); 8] = [
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
#[ignore = "clean-room: 04§5.1 rotation snaps to the nearest quarter turn; an empty crop shows the MediaBox (T4)"]
fn render_size_follows_the_nearest_quarter_turn_and_an_empty_crop() {
    let content = "BT /F1 24 Tf 30 30 Td (Geo) Tj ET";
    let cases: [(&str, [(u32, u32); 4]); 4] = [
        (
            "/Rotate 44.6",
            [(101, 126), (401, 501), (601, 751), (1602, 2001)],
        ),
        (
            "/Rotate 134.6",
            [(126, 101), (501, 401), (751, 601), (2001, 1602)],
        ),
        (
            "/CropBox [0 0 0 0]",
            [(101, 126), (401, 501), (601, 751), (1602, 2001)],
        ),
        (
            "/CropBox [500 600 700 800]",
            [(101, 126), (401, 501), (601, 751), (1602, 2001)],
        ),
    ];
    let pages: Vec<(&str, &str)> = cases.iter().map(|(extra, _)| (content, *extra)).collect();
    let doc = open(pdf("[0 0 400.5 500.25]", &pages, &[]));
    for (page, (extra, sizes)) in cases.iter().enumerate() {
        for (scale, want) in SCALES.into_iter().zip(sizes) {
            assert_eq!(size(&doc, page, scale), *want, "{extra} at {scale}");
        }
    }
    // The crop that misses the MediaBox shows the page's content.
    let text = doc.structured_text(3).unwrap();
    assert_eq!(text.as_text(), "Geo\n\n");
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
    // NoView (32) hides the appearance.
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

/// `text` with each whitespace run as one space, trimmed.
fn squeezed(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn page_text_of_fixtures() {
    let cases = [
        ("hello.pdf", 0, "PDF4All\n\n"),
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
        assert_eq!(
            squeezed(&text.as_text()),
            squeezed(want),
            "{name} page {page}"
        );
    }
    // The paragraph page, with one gap whose reading the space band of
    // spec 04 §3.4 decides.
    let text = fixture("p6/paragraph.pdf").structured_text(0).unwrap();
    let got = squeezed(&text.as_text());
    let before = "The quick brown fox jumps over the lazy dog while the cat watches \
                  from a sunny window and the birds sing. Below the paragraph Move me";
    assert!(got.starts_with(before), "{got}");
    let tail = got[before.len()..].trim();
    let line = text
        .blocks()
        .iter()
        .flat_map(|b| &b.lines)
        .find(|l| l.chars.first().is_some_and(|c| c.c == 'H'))
        .unwrap();
    let drawn: Vec<_> = line
        .chars
        .iter()
        .filter(|c| !c.synthetic && c.c != ' ')
        .collect();
    let (n, t) = (drawn[6], drawn[7]);
    assert_eq!((n.c, t.c), ('n', 't'));
    let gap = (t.origin[0] - n.quad[2]) / t.size;
    match gap {
        g if g <= 0.10 => assert_eq!(tail, "Head runtail run", "gap {g} em"),
        g if g >= 0.25 => assert_eq!(tail, "Head run tail run", "gap {g} em"),
        g => assert!(
            tail == "Head runtail run" || tail == "Head run tail run",
            "gap {g} em: {tail}"
        ),
    }
}

#[test]
fn structure_actual_text_keeps_the_matching_glyphs() {
    // MCID 0 maps to a StructElem whose ActualText is the glyphs' suffix:
    // the replaced glyphs "tagged P0B-SECRET-7731" are drawn from x 72 on
    // the baseline at y 632 (page space).
    let text = fixture("redact.pdf").structured_text(0).unwrap();
    let line = text
        .blocks()
        .iter()
        .flat_map(|b| &b.lines)
        .find(|l| {
            l.chars
                .first()
                .is_some_and(|c| c.c == 'P' && (c.origin[1] - 632.0).abs() < 0.01)
        })
        .unwrap();
    let text: String = line.chars.iter().map(|c| c.c).collect();
    assert_eq!(text.trim(), "P0B-SECRET-7731");
    let tagged = "tagged P0B-SECRET-7731";
    // Helvetica 12: the drawn string's advance.
    let end = 72.0 + 12.0 * helvetica_width(tagged) / 1000.0;
    for c in &line.chars {
        assert!(
            c.origin[0] >= 72.0 - 0.01 && c.origin[0] <= end + 0.01,
            "{c:?} outside 72..{end}"
        );
    }
}

/// Helvetica advances (1/1000 em) of the ASCII used above (public AFM).
fn helvetica_width(text: &str) -> f64 {
    text.chars()
        .map(|c| match c {
            ' ' => 278.0,
            '-' => 333.0,
            'a' | 'e' | 'g' | 'd' => 556.0,
            't' => 278.0,
            'P' | 'B' | 'E' => 667.0,
            'S' => 667.0,
            'C' | 'R' => 722.0,
            'T' => 611.0,
            '0'..='9' => 556.0,
            _ => panic!("no width for {c}"),
        })
        .sum()
}

/// A line summary against an expected (text, x, y, w, h, size, family,
/// weight, style) and top, with the tolerances of spec 04 §8.1: text
/// trimmed, x, y and size exact, w ±1, top and h within 0.35 em.
fn summary_near(
    got: &tandem_pdf_engine::view::LineSummary,
    want: (&str, i32, i32, i32, i32, i32, &str, &str, &str),
    top: i32,
    x_slack: i32,
) -> bool {
    let (text, x, y, w, h, size, family, weight, style) = want;
    let em = (0.35 * f64::from(size)).ceil() as i32;
    got.text.trim() == text.trim()
        && (got.x - x).abs() <= x_slack
        && got.y == y
        && (got.w - w).abs() <= 1 + x_slack
        && (got.h - h).abs() <= em
        && (got.top - top).abs() <= em
        && got.size == size
        && (got.family, got.weight, got.style) == (family, weight, style)
}

#[test]
fn line_summaries_of_fixtures() {
    let summaries = |name: &str| fixture(name).structured_text(0).unwrap().line_summaries();
    let hello = summaries("hello.pdf");
    assert_eq!(hello.len(), 1);
    assert!(
        summary_near(
            &hello[0],
            (
                "PDF4All",
                72,
                92,
                132,
                49,
                36,
                "sans-serif",
                "normal",
                "normal"
            ),
            53,
            0
        ),
        "{hello:?}"
    );
    // Turned lines: their bbox edges follow the quads' height, so x, w, h
    // and top are compared within 0.35 em; the baseline y is exact.
    let rotated = summaries("rotated.pdf");
    let want = [
        (
            (
                "PDF4All rotated",
                72,
                92,
                211,
                41,
                30,
                "sans-serif",
                "normal",
                "normal",
            ),
            59,
            0,
        ),
        (
            (
                "PDF4All rotated",
                387,
                592,
                41,
                211,
                30,
                "sans-serif",
                "normal",
                "normal",
            ),
            380,
            11,
        ),
        (
            (
                "PDF4All rotated",
                97,
                432,
                178,
                178,
                30,
                "sans-serif",
                "normal",
                "normal",
            ),
            259,
            11,
        ),
    ];
    assert_eq!(rotated.len(), 3);
    for (got, (want, top, x_slack)) in rotated.iter().zip(want) {
        assert!(summary_near(got, want, top, x_slack), "{got:?}");
    }
    let paragraph = summaries("p6/paragraph.pdf");
    let head = paragraph
        .iter()
        .find(|l| l.text.starts_with("Head"))
        .unwrap();
    assert_eq!((head.x, head.y, head.size), (300, 342, 11));
    assert_eq!(
        (head.family, head.weight, head.style),
        ("sans-serif", "normal", "normal")
    );
    // The right-to-left fixture: the Hebrew and Arabic runs in logical
    // order, each summary in one direction.
    let rtl = summaries("rtl.pdf");
    let texts: Vec<&str> = rtl.iter().map(|l| l.text.trim()).collect();
    assert_eq!(texts, ["PDF4All", "שלום", "PDF4All", "مرحبا", "PDF4All"]);
    let first: Vec<(i32, i32, i32)> = rtl.iter().map(|l| (l.x, l.y, l.size)).collect();
    assert_eq!(first[0], (72, 52, 20));
    assert_eq!(first[1], (72, 132, 28));
    assert_eq!(first[3], (72, 202, 28));
    for line in &rtl[1..] {
        assert_eq!(line.family, "serif", "{line:?}");
    }
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
fn layout_and_clipping() {
    let doc = open(pdf(
        "[0 0 612 792]",
        &[
            // An indent of 20 pt opens a block; an outdent does not.
            (
                "BT /F1 12 Tf 72 700 Td (First line of para) Tj 0 -14 Td (second line) Tj 20 -14 Td (indented) Tj -20 -14 Td (back) Tj ET",
                "",
            ),
            // Glyphs outside the clip drop out; invisible and fill+stroke
            // text count once.
            (
                "q 50 580 30 40 re W n BT /F1 12 Tf 72 600 Td (PartClip) Tj ET Q \
                 BT 3 Tr /F1 12 Tf 72 500 Td (Invis) Tj ET BT 2 Tr /F1 12 Tf 72 400 Td (FillStroke) Tj ET",
                "",
            ),
            // Letter spacing of 2 pt at 12 pt (0.17 em) and TJ gaps of 0.2,
            // 0.5 and 1 em.
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
    let clipped = text(1);
    assert!(
        clipped == "P\n\nInvis\n\nFillStroke\n\n" || clipped == "Pa\n\nInvis\n\nFillStroke\n\n",
        "{clipped:?}"
    );
    let spaced = text(2);
    let (first, rest) = spaced.split_once('\n').unwrap();
    assert!(first == "S p a c e d" || first == "Spaced", "{first:?}");
    // 0.2 em lies in the band where either reading is allowed; 0.5 em is a
    // space; 1 em is a space or a new line.
    let rest = rest.trim_start_matches('\n');
    let letters: String = rest.split_whitespace().collect();
    assert_eq!(letters, "ABCD", "{rest:?}");
    assert!(rest.contains("B C"), "{rest:?}");
    assert_eq!(text(3), "");
}

#[test]
fn search_hits_and_quads() {
    let hello = fixture("hello.pdf").structured_text(0).unwrap();
    let hits = hello.search("All", MAX_SEARCH_QUADS).hits;
    assert_eq!(hits.len(), 1);
    assert!(quads_near(
        &hits,
        &[[
            164.016, 53.3, 204.012, 53.3, 164.016, 102.764, 204.012, 102.764
        ]],
        36.0
    ));
    // Case-sensitive.
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
    assert!(quads_near(
        &catalog,
        &[
            [174.06, 79.1, 190.068, 79.1, 174.06, 95.588, 190.068, 95.588],
            [72.0, 93.1, 94.68, 93.1, 72.0, 109.588, 94.68, 109.588]
        ],
        12.0
    ));
    let across = text.search("spaces CAT", MAX_SEARCH_QUADS).hits;
    assert_eq!(across[0].len(), 2);
    assert!(quad_near(
        &across[0][0],
        &[
            168.72, 93.1, 206.736, 93.1, 168.72, 109.588, 206.736, 109.588
        ],
        12.0
    ));
}

#[test]
fn rotated_page_quads() {
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
        assert!(quad_near(&hit[0], &want, 30.0), "{:?}", hit[0]);
    }
    let vertical = &text.blocks()[1].lines[0];
    assert!(close(&vertical.dir, &[0.0, -1.0]));
    let p = &vertical.chars[0];
    assert!(close(&p.origin, &[420.0, 592.0]));
    assert!(quad_near(
        &p.quad,
        &[387.75, 592.0, 387.75, 571.99, 428.97, 592.0, 428.97, 571.99],
        30.0
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
    // inner reference, so the text is read once.
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
    // The page text keeps only the visible "P"; search finds all four.
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
        assert!(
            quad_near(&hits[0][0], &quad, 12.0),
            "{needle} {:?}",
            hits[0]
        );
    }
    // edit-tj.pdf page 3 draws "(unbalanced)" above the page.
    let hits = fixture("edit-tj.pdf")
        .structured_text(2)
        .unwrap()
        .search("unbalanced", MAX_SEARCH_QUADS)
        .hits;
    assert_eq!(hits.len(), 1);
    assert!(quad_near(
        &hits[0][0],
        &[
            244.0, -733.8, 368.08, -733.8, 244.0, -700.824, 368.08, -700.824
        ],
        24.0
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
    // Line feed, carriage return and tab in a needle match whitespace.
    assert_eq!(text.search("\n", MAX_SEARCH_QUADS).hits.len(), 3);
    for needle in ["again\nnext", "again\r\nnext", "again next"] {
        let hits = text.search(needle, MAX_SEARCH_QUADS).hits;
        assert_eq!(hits.len(), 1, "{needle:?}");
        assert!(quads_near(
            &hits,
            &[
                [
                    138.012, 79.1, 167.364, 79.1, 138.012, 95.588, 167.364, 95.588
                ],
                [72.0, 93.1, 94.68, 93.1, 72.0, 109.588, 94.68, 109.588]
            ],
            12.0
        ));
    }
    let tab = text.search("o\tw", MAX_SEARCH_QUADS).hits;
    assert!(quads_near(
        &tab,
        &[[92.664, 79.1, 111.336, 79.1, 92.664, 95.588, 111.336, 95.588]],
        12.0
    ));

    // Three matches of two quads each: a hyphen at a line end joins the
    // next line and gets no quad.
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
    assert!(quads_near(&all.hits, &want, 12.0));
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
fn spans_layout_of_fixtures() {
    let rtl = fixture("rtl.pdf").structured_text(0).unwrap();
    assert_eq!(
        squeezed(&rtl.spans_text()),
        "PDF4All שלום PDF4All مرحبا PDF4All"
    );
    let tops: Vec<i32> = rtl.line_summaries().iter().map(|l| l.top).collect();
    let sizes: Vec<i32> = rtl.line_summaries().iter().map(|l| l.size).collect();
    for ((top, size), want) in tops.iter().zip(&sizes).zip([30, 102, 102, 163, 172]) {
        assert!(
            f64::from((top - want).abs()) <= 0.35 * f64::from(*size),
            "{tops:?}"
        );
    }
    // The run layout and the summaries cover the same characters as the
    // reading layout.
    let quartz = fixture("p6/quartz.pdf").structured_text(0).unwrap();
    let letters = |text: &str| text.split_whitespace().collect::<String>();
    assert_eq!(letters(&quartz.spans_text()), letters(&quartz.as_text()));
    let lines: usize = quartz.span_blocks().iter().map(|b| b.lines.len()).sum();
    assert_eq!(lines, quartz.line_summaries().len());
    let hello = fixture("hello.pdf").structured_text(0).unwrap();
    assert!((hello.line_summaries()[0].top - 53).abs() <= 13);
}

#[test]
#[ignore = "clean-room: 04§2.4 spans_text joins the runs of one visual line (T5)"]
fn spans_text_keeps_a_visual_line_on_one_line() {
    let rtl = fixture("rtl.pdf").structured_text(0).unwrap();
    assert_eq!(
        rtl.spans_text(),
        "PDF4All\n\nשלום PDF4All\n\nمرحبا PDF4All\n\n"
    );
    // A page whose glyphs are drawn one by one reads as words.
    let quartz = fixture("p6/quartz.pdf").structured_text(0).unwrap();
    let words = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
    assert_eq!(words(&quartz.spans_text()), words(&quartz.as_text()));
}

/// A page showing a Type3 font whose codes 161 to 169 name these glyphs;
/// every glyph draws the same procedure.
fn type3_page() -> ViewDocument {
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
    open(pdf_res(
        "[0 0 612 792]",
        "<< /Font << /F1 3 0 R /T3 4 0 R >> >>",
        &[(
            "BT /T3 20 Tf 72 700 Td (ABA B) Tj ET BT /T3 20 Tf 72 600 Td (AB) Tj ET \
             BT /T3 20 Tf 2 Tc 72 500 Td [(A) -500 (B)] TJ ET \
             BT /T3 10 Tf 72 300 Td <a1a2a3a4a5a6a7a8a9> Tj ET BT /T3 10 Tf 72 200 Td <41a0> Tj ET",
            "",
        )],
        &[&font, &stream("", "600 0 0 0 550 700 d1 0 0 550 700 re f")],
    ))
}

#[test]
fn type3_glyphs_take_their_widths() {
    let doc = type3_page();
    let text = doc.structured_text(0).unwrap();
    let lines: Vec<String> = text
        .as_text()
        .split("\n\n")
        .map(|l| l.to_string())
        .collect();
    assert_eq!(&lines[..3], ["ABA B", "AB", "A B"]);
    let first = &text.line_summaries()[0];
    assert_eq!((first.x, first.y, first.size), (72, 92, 20));
    assert!((first.w - 57).abs() <= 1, "{first:?}");
    // /Widths advances: A 600, B 700 at 20 pt.
    let ab = text.search("AB", MAX_SEARCH_QUADS).hits;
    assert!(quads_near(
        &ab,
        &[
            [72.0, 76.0, 98.0, 76.0, 72.0, 94.0, 98.0, 94.0],
            [72.0, 176.0, 98.0, 176.0, 72.0, 194.0, 98.0, 194.0]
        ],
        20.0
    ));
    let spaced = text.search("A B", MAX_SEARCH_QUADS).hits;
    assert!(quads_near(
        &spaced,
        &[
            [98.0, 76.0, 129.0, 76.0, 98.0, 94.0, 129.0, 94.0],
            [72.0, 276.0, 110.0, 276.0, 72.0, 294.0, 110.0, 294.0]
        ],
        20.0
    ));
    // A Type3 font without /Name has a stable, non-empty name.
    let name = |text: &tandem_pdf_engine::view::StructuredText| {
        text.fonts()[text.blocks()[0].lines[0].chars[0].font]
            .name
            .clone()
    };
    let again = doc.structured_text(0).unwrap();
    assert!(!name(&text).is_empty());
    assert_eq!(name(&text), name(&again));
}

#[test]
#[ignore = "clean-room: 05§4.1-4.2 glyph names by the AGL specification; a<n> only at its own code"]
fn type3_text_comes_from_encoding_names() {
    let text = type3_page().structured_text(0).unwrap();
    // uni0041, a67 at 162 (not its code), A.sc, f_i, T_h, foo, a5 (not its
    // code), u00E9, alpha; code 160 has no name.
    let got = squeezed(&text.as_text());
    assert_eq!(
        got,
        "ABA B AB A B A \u{fffd} A fi Th \u{fffd}\u{fffd}é α A \u{fffd}"
    );
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
    // Popups and NoView (32, also through an indirect /F) are not drawn;
    // NoZoom (8) is.
    assert_eq!([0, 1, 2, 3, 4].map(centre), [255, 255, 255, 0, 255]);
}

#[test]
fn rotated_page_text_quads() {
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
    // Per page: line summaries (text x y size), then the quads of "Turn"
    // and "second" with their font sizes in page space.
    type Want = ([(&'static str, i32, i32, i32); 2], [([f64; 8], f64); 2]);
    let want: [Want; 3] = [
        (
            [("Turn me", 362, 40, 24), ("second", 267, 180, 10)],
            [
                (
                    [395.8, 40.0, 395.8, 89.344, 362.824, 40.0, 362.824, 89.344],
                    24.0,
                ),
                (
                    [280.75, 180.0, 280.75, 212.24, 267.01, 180.0, 267.01, 212.24],
                    10.0,
                ),
            ],
        ),
        (
            [("Turn me", 732, 1104, 48), ("second", 962, 824, 20)],
            [
                (
                    [
                        732.4, 1104.0, 732.4, 1005.312, 798.352, 1104.0, 798.352, 1005.312,
                    ],
                    48.0,
                ),
                (
                    [962.5, 824.0, 962.5, 759.52, 989.98, 824.0, 989.98, 759.52],
                    20.0,
                ),
            ],
        ),
        (
            [("Turn me", 450, 390, 24), ("second", 367, 290, 10)],
            [
                (
                    [
                        540.0, 415.8, 490.656, 415.8, 540.0, 382.824, 490.656, 382.824,
                    ],
                    24.0,
                ),
                (
                    [400.0, 300.75, 367.76, 300.75, 400.0, 287.01, 367.76, 287.01],
                    10.0,
                ),
            ],
        ),
    ];
    for (page, (lines, quads)) in want.iter().enumerate() {
        let text = doc.structured_text(page).unwrap();
        assert_eq!(text.as_text(), "Turn me\n\nsecond\n\n");
        let summaries = text.line_summaries();
        assert_eq!(summaries.len(), 2, "page {page}");
        for (got, (line, x, y, size)) in summaries.iter().zip(lines) {
            assert_eq!(got.text, *line, "page {page}");
            assert_eq!(got.size, *size, "page {page} {line}");
            // On a turned page the line's x0 or y is the baseline side;
            // the other is the bbox edge whose height is the font's.
            let slack = (0.35 * f64::from(*size)).ceil() as i32;
            assert!((got.x - x).abs() <= slack, "page {page} {line}: {got:?}");
            assert!((got.y - y).abs() <= slack, "page {page} {line}: {got:?}");
        }
        for (needle, (quad, em)) in ["Turn", "second"].iter().zip(quads) {
            let hits = text.search(needle, MAX_SEARCH_QUADS).hits;
            assert_eq!(hits.len(), 1, "page {page} {needle}");
            assert!(
                quad_near(&hits[0][0], quad, *em),
                "page {page} {needle} {:?}",
                hits[0]
            );
        }
    }
}
