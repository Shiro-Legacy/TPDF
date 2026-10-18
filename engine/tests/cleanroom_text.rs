// SPDX-License-Identifier: AGPL-3.0-or-later
//! Acceptance for spec 04 (structured text, page frame and pixel size): the
//! new cases N1 to N16, each built from a constructed page. Expected values
//! come from ISO 32000-2 (§9.4, §14.9.4, §14.11.2, §7.7.3.3), Unicode
//! Standard Annex #9, the public Adobe Core 14 AFM metrics and the spec's
//! own rules and bands.
use std::collections::BTreeMap;
use tandem_pdf_engine::view::{
    MAX_SEARCH_QUADS, RenderOptions, StructuredText, TextChar, ViewDocument,
};

// ------------------------------------------------------------------ builders

fn pdf(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n", i + 1).bytes());
        out.extend(body);
        out.extend(b"\nendobj\n");
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

fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut out = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
    out.extend(data);
    out.extend(b"\nendstream");
    out
}

/// One 300 × 300 page; `extra` objects are numbered from 5.
fn page(content: &str, resources: &str, extra: &[Vec<u8>]) -> ViewDocument {
    let objects = [
        vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Resources {resources} /Contents 4 0 R >>"
            )
            .into_bytes(),
            stream("", content.as_bytes()),
        ],
        extra.to_vec(),
    ]
    .concat();
    ViewDocument::open(pdf(&objects), "").unwrap()
}

const HELVETICA: &str =
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>";

/// Helvetica as /F1.
fn helvetica(content: &str) -> ViewDocument {
    page(content, "<< /Font << /F1 5 0 R >> >>", &[HELVETICA.into()])
}

fn text(doc: &ViewDocument) -> String {
    doc.structured_text(0).unwrap().as_text()
}

fn chars(text: &StructuredText) -> Vec<&TextChar> {
    text.blocks()
        .iter()
        .flat_map(|b| &b.lines)
        .flat_map(|l| &l.chars)
        .collect()
}

fn lines(text: &StructuredText) -> usize {
    text.blocks().iter().map(|b| b.lines.len()).sum()
}

/// A page whose ToUnicode reads a..e as U+05D0..U+05D4, f as U+0627 (alef),
/// h as U+064E (fatha) and i as U+0628 (beh).
fn right_to_left_page(content: &str) -> ViewDocument {
    let entries = "<61> <05d0>\n<62> <05d1>\n<63> <05d2>\n<64> <05d3>\n<65> <05d4>\n\
                   <66> <0627>\n<68> <064e>\n<69> <0628>";
    let cmap = format!(
        "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /R def \
         1 begincodespacerange <00> <FF> endcodespacerange\n8 beginbfchar\n{entries}\n\
         endbfchar endcmap CMapName currentdict /CMap defineresource pop end end"
    );
    page(
        content,
        "<< /Font << /F1 5 0 R >> >>",
        &[
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /ToUnicode 6 0 R >>".to_vec(),
            stream("", cmap.as_bytes()),
        ],
    )
}

/// Text with A..E read as U+05D0..U+05D4.
fn hebrew(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='E' => char::from_u32(0x5d0 + (c as u32 - 'A' as u32)).unwrap(),
            c => c,
        })
        .collect()
}

// ------------------------------------------------------------------ N1, N2

#[test]
fn n1_a_skipped_form_lends_no_text_to_the_page() {
    // The form has no /BBox, so it is not drawn; the page then shows the
    // same font and as many glyphs.
    let doc = page(
        "/Fm Do BT /F1 12 Tf 10 50 Td (CD) Tj ET",
        "<< /Font << /F1 5 0 R >> /XObject << /Fm 6 0 R >> >>",
        &[
            HELVETICA.into(),
            stream(
                "/Type /XObject /Subtype /Form",
                b"BT /F1 12 Tf 10 100 Td (XY) Tj ET",
            ),
        ],
    );
    let text = doc.structured_text(0).unwrap();
    assert_eq!(text.as_text(), "CD\n\n");
    let chars = chars(&text);
    assert!((chars[0].origin[0] - 10.0).abs() < 0.01, "{:?}", chars[0]);
    assert!((chars[0].origin[1] - 250.0).abs() < 0.01, "{:?}", chars[0]);
    // C is 722/1000 em wide in Helvetica.
    assert!((chars[1].origin[0] - 18.664).abs() < 0.05, "{:?}", chars[1]);
}

#[test]
fn n2_type3_glyphs_drawn_through_forms_read_their_names() {
    let doc = page(
        "BT /F1 12 Tf 50 100 Td (abc) Tj ET",
        "<< /Font << /F1 5 0 R >> >>",
        &[
            b"<< /Type /Font /Subtype /Type3 /FontBBox [0 0 750 750] \
              /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << /A 6 0 R /B 6 0 R /C 6 0 R >> \
              /Encoding << /Differences [97 /A /B /C] >> /FirstChar 97 /LastChar 99 \
              /Widths [800 800 800] /Resources << /XObject << /X0 7 0 R >> >> >>"
                .to_vec(),
            stream("", b"800 0 d0 /X0 Do"),
            stream(
                "/Type /XObject /Subtype /Form /BBox [0 0 1000 1000] /Resources << >>",
                b"0 0 m 750 750 l 375 0 l f",
            ),
        ],
    );
    let text = doc.structured_text(0).unwrap();
    assert_eq!(text.as_text(), "ABC\n\n");
    assert_eq!(lines(&text), 1);
}

// ------------------------------------------------------------------ N3, N4 spacing and layout

#[test]
fn n3_spaces_follow_the_gap_band() {
    for size in [10, 12] {
        for (n, want) in [
            (50, "ab"),
            (100, "ab"),
            (250, "a b"),
            (300, "a b"),
            (500, "a b"),
        ] {
            let doc = helvetica(&format!("BT /F1 {size} Tf 10 100 Td [(a) -{n} (b)] TJ ET"));
            assert_eq!(text(&doc), format!("{want}\n\n"), "{size} pt, {n}");
        }
    }
    // Letter spacing of 0.3 em reads as spaces, of 0.05 em as none.
    assert_eq!(
        text(&helvetica("BT /F1 10 Tf 3 Tc 10 100 Td (abc) Tj ET")),
        "a b c\n\n"
    );
    assert_eq!(
        text(&helvetica("BT /F1 10 Tf 0.5 Tc 10 100 Td (abc) Tj ET")),
        "abc\n\n"
    );
    // A drawn space then a gap of 0.3 em: one space.
    assert_eq!(
        text(&helvetica("BT /F1 10 Tf 10 100 Td [(a ) -300 (b)] TJ ET")),
        "a b\n\n"
    );
}

#[test]
fn n4_lines_and_blocks_follow_baselines_and_indents() {
    let two = |content: &str| text(&helvetica(content));
    // Leading of 1.0 and 1.2 em: one block; 3.5 em: two.
    assert_eq!(
        two("BT /F1 10 Tf 10 200 Td (one) Tj 0 -10 Td (two) Tj ET"),
        "one\ntwo\n\n"
    );
    assert_eq!(
        two("BT /F1 10 Tf 10 200 Td (one) Tj 0 -12 Td (two) Tj ET"),
        "one\ntwo\n\n"
    );
    assert_eq!(
        two("BT /F1 10 Tf 10 200 Td (one) Tj 0 -35 Td (two) Tj ET"),
        "one\n\ntwo\n\n"
    );
    // An indent of 2 em starts a block; one of 0.05 em does not.
    assert_eq!(
        two("BT /F1 10 Tf 10 200 Td (one) Tj 0 -12 Td (two) Tj 20 -12 Td (three) Tj ET"),
        "one\ntwo\n\nthree\n\n"
    );
    assert_eq!(
        two("BT /F1 10 Tf 10 200 Td (one) Tj 0 -12 Td (two) Tj 0.5 -12 Td (three) Tj ET"),
        "one\ntwo\nthree\n\n"
    );
    // A rise of 0.33 em stays on the line.
    let doc = helvetica("BT /F1 10 Tf 10 200 Td (x) Tj 3.3 Ts (2) Tj 0 Ts (y) Tj ET");
    assert_eq!(text(&doc), "x2y\n\n");
}

// ------------------------------------------------------------------ N5 overprinting

#[test]
fn n5_fill_and_stroke_reads_once() {
    let doc = helvetica("BT /F1 10 Tf 2 Tr 10 100 Td (Bold) Tj ET");
    assert_eq!(text(&doc), "Bold\n\n");
}

#[test]
fn n5_copies_overprinted_for_fake_bold_read_once() {
    let twice = "BT /F1 10 Tf 10 100 Td (Bold) Tj ET BT /F1 10 Tf 10 100 Td (Bold) Tj ET";
    assert_eq!(text(&helvetica(twice)), "Bold\n\n");
    // 0.3 pt at 10 pt is 0.03 em.
    let doc =
        helvetica("BT /F1 10 Tf 10 100 Td (Bold) Tj ET BT /F1 10 Tf 10.3 100 Td (Bold) Tj ET");
    assert_eq!(text(&doc), "Bold\n\n");
    let doc =
        helvetica("BT /F1 10 Tf 10 100 Td (Bold) Tj ET BT /F1 10 Tf 10.3 100.3 Td (Bold) Tj ET");
    assert_eq!(text(&doc), "Bold\n\n");
}

// ------------------------------------------------------------------ N6 right to left

#[test]
fn n6_right_to_left_words_read_the_same_however_they_are_drawn() {
    // Visual order: the glyphs of the logical word ABC (U+05D0 U+05D1
    // U+05D2) drawn left to right as c b a.
    let visual = right_to_left_page("BT /F1 10 Tf 100 100 Td (cba) Tj ET");
    // Logical order: a drawn rightmost, each next one 6 pt to its left.
    let logical =
        right_to_left_page("BT /F1 10 Tf 105.56 100 Td (a) Tj -6 0 Td (b) Tj -6 0 Td (c) Tj ET");
    for doc in [&visual, &logical] {
        let text = doc.structured_text(0).unwrap();
        assert_eq!(text.as_text(), format!("{}\n\n", hebrew("ABC")));
        assert_eq!(text.search(&hebrew("ABC"), MAX_SEARCH_QUADS).hits.len(), 1);
        assert!(
            text.search(&hebrew("CBA"), MAX_SEARCH_QUADS)
                .hits
                .is_empty()
        );
    }
}

#[test]
fn n6_digits_latin_and_marks_in_right_to_left_lines() {
    // Two words, visual order, with digits and Latin around them.
    let doc = right_to_left_page("BT /F1 10 Tf 10 100 Td (XY abc 12 de ZW) Tj ET");
    assert_eq!(text(&doc), format!("XY {} ZW\n\n", hebrew("ED 12 CBA")));
    // Arabic beh with a fatha, then alef, drawn in visual order: alef, the
    // fatha over the beh, the beh. The mark follows its letter.
    let doc = right_to_left_page("BT /F1 10 Tf 10 100 Td [(f) (h) 556 (i)] TJ ET");
    assert_eq!(text(&doc), "\u{628}\u{64e}\u{627}\n\n");
}

// ------------------------------------------------------------------ N7 clipping mode 7

#[test]
fn n7_clip_only_text_is_extracted_once() {
    // No ET.
    assert_eq!(
        text(&helvetica("BT /F1 10 Tf 7 Tr 10 100 Td (AB) Tj")),
        "AB\n\n"
    );
    // Then mode 0 in the same object.
    assert_eq!(
        text(&helvetica(
            "BT /F1 10 Tf 7 Tr 10 100 Td (AB) Tj 0 Tr (CD) Tj ET"
        )),
        "ABCD\n\n"
    );
    // Inside a form.
    let doc = page(
        "/Fm Do",
        "<< /Font << /F1 5 0 R >> /XObject << /Fm 6 0 R >> >>",
        &[
            HELVETICA.into(),
            stream(
                "/Type /XObject /Subtype /Form /BBox [0 0 300 300]",
                b"BT /F1 10 Tf 7 Tr 10 100 Td (XY) Tj ET",
            ),
        ],
    );
    assert_eq!(text(&doc), "XY\n\n");
}

// ------------------------------------------------------------------ N8, N9 ActualText

#[test]
fn n8_actual_text_replaces_the_glyphs_it_spans() {
    // More characters than glyphs: all on the glyphs' line.
    let doc =
        helvetica("/Span << /ActualText (ABCDEFG) >> BDC BT /F1 10 Tf 10 100 Td (AB) Tj ET EMC");
    let st = doc.structured_text(0).unwrap();
    assert_eq!(st.as_text(), "ABCDEFG\n\n");
    assert_eq!(lines(&st), 1);
    // A search for it finds one hit inside the glyphs' bounds (AB is 13.34
    // pt wide at 10 pt).
    let hits = st.search("ABCDEFG", MAX_SEARCH_QUADS).hits;
    assert_eq!(hits.len(), 1);
    for quad in &hits[0] {
        for x in [quad[0], quad[2], quad[4], quad[6]] {
            assert!((10.0 - 0.05..=23.34 + 0.05).contains(&x), "{quad:?}");
        }
    }
    // An empty ActualText removes the text.
    let doc = helvetica(
        "BT /F1 10 Tf 10 100 Td (A) Tj /Span << /ActualText () >> BDC (B) Tj EMC (C) Tj ET",
    );
    // B's place is a gap of 0.67 em: a space.
    assert_eq!(text(&doc), "A C\n\n");
}

#[test]
fn n8_nested_actual_text_and_nuls() {
    let doc = helvetica(
        "/Span << /ActualText (outer) >> BDC BT /F1 10 Tf 10 100 Td (AB) Tj \
         /Span << /ActualText (inner) >> BDC (CD) Tj EMC ET EMC",
    );
    assert_eq!(text(&doc), "outer\n\n");
    let doc =
        helvetica("/Span << /ActualText (A\\000B) >> BDC BT /F1 10 Tf 10 100 Td (XY) Tj ET EMC");
    assert_eq!(text(&doc), "AB\n\n");
    let doc = helvetica(
        "/Span << /ActualText <FEFF004100000042> >> BDC BT /F1 10 Tf 10 100 Td (XY) Tj ET EMC",
    );
    assert_eq!(text(&doc), "AB\n\n");
}

#[test]
fn n9_actual_text_over_nothing_is_dropped() {
    let doc =
        helvetica("/Span << /ActualText (Gone) >> BDC EMC BT /F1 10 Tf 10 100 Td (Kept) Tj ET");
    assert_eq!(text(&doc), "Kept\n\n");
}

#[test]
fn n9_actual_text_over_a_path_is_placed_on_its_bounds() {
    // A triangle from (20, 40) to (120, 90): bottom left (20, 260) in page
    // space, 100 × 50.
    let doc = helvetica("/Span << /ActualText (Logo) >> BDC 20 40 m 120 40 l 70 90 l f EMC");
    let text = doc.structured_text(0).unwrap();
    assert_eq!(text.as_text(), "Logo\n\n");
    let first = chars(&text)[0];
    assert!((first.origin[0] - 20.0).abs() < 0.01, "{first:?}");
    assert!((first.origin[1] - 260.0).abs() < 0.01, "{first:?}");
    // One em the size of the painted area: sqrt(100 × 50).
    assert!((first.size - 5000f64.sqrt()).abs() < 0.01, "{first:?}");
}

// ------------------------------------------------------------------ N10, N11 frame and pixels

/// A document of one page per entry of `pages` (page dictionary entries),
/// each showing `Geo` at user space (30, 30) in Helvetica.
fn frames(pages: &[&str], pages_entries: &str) -> ViewDocument {
    let content = b"BT /F1 10 Tf 30 30 Td (Geo) Tj ET".to_vec();
    let kids: Vec<String> = (0..pages.len()).map(|i| format!("{} 0 R", 5 + i)).collect();
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        format!(
            "<< /Type /Pages /Kids [{}] /Count {} {pages_entries} >>",
            kids.join(" "),
            pages.len()
        )
        .into_bytes(),
        stream("", &content),
        HELVETICA.into(),
    ];
    for entries in pages {
        objects.push(
            format!(
                "<< /Type /Page /Parent 2 0 R {entries} /Resources << /Font << /F1 4 0 R >> >> /Contents 3 0 R >>"
            )
            .into_bytes(),
        );
    }
    ViewDocument::open(pdf(&objects), "").unwrap()
}

/// The pixel count of `v` points at `scale` (spec 04 §5.2, ε = 1e-3).
fn px(v: f64, scale: f64) -> u32 {
    let product = v * scale;
    let floor = product.floor();
    if product - floor <= 1e-3 {
        floor as u32
    } else {
        product.ceil() as u32
    }
}

/// Checks each page's size, its render size at four scales, and where the
/// text drawn at user space (30, 30) lands: `origin` in page space.
fn check_frames(doc: &ViewDocument, rows: &[(&str, (f64, f64), [f64; 2])]) {
    for (page, (label, (w, h), origin)) in rows.iter().enumerate() {
        let (gw, gh) = doc.page_size(page).unwrap();
        assert!(
            (gw - w).abs() < 1e-3 && (gh - h).abs() < 1e-3,
            "{label}: {gw} × {gh}, not {w} × {h}"
        );
        for scale in [0.25, 1.0, 1.5, 4.0] {
            let image = doc.render(page, scale, RenderOptions::default()).unwrap();
            assert_eq!(
                (image.width, image.height),
                (px(*w, scale), px(*h, scale)),
                "{label} at {scale}"
            );
        }
        let text = doc.structured_text(page).unwrap();
        let first = text
            .unclipped_blocks()
            .iter()
            .flat_map(|b| &b.lines)
            .flat_map(|l| &l.chars)
            .next()
            .unwrap_or_else(|| panic!("{label}: no text"));
        assert!(
            (first.origin[0] - origin[0]).abs() < 1e-3
                && (first.origin[1] - origin[1]).abs() < 1e-3,
            "{label}: origin {:?}, not {origin:?}",
            first.origin
        );
    }
}

#[test]
fn n10_page_frames() {
    let m = "/MediaBox [0 0 400.5 500.25]";
    let rows: Vec<(String, (f64, f64), [f64; 2])> = vec![
        (m.into(), (400.5, 500.25), [30.0, 470.25]),
        (
            format!("{m} /CropBox [10.3 20.7 300.55 400.25]"),
            (290.25, 379.55),
            [19.7, 370.25],
        ),
        (
            format!("{m} /CropBox [0 0 0.5 0.5]"),
            (1.0, 1.0),
            [30.0, -29.0],
        ),
        (format!("{m} /Rotate 90"), (500.25, 400.5), [30.0, 30.0]),
        (format!("{m} /Rotate -90"), (500.25, 400.5), [470.25, 370.5]),
        (format!("{m} /Rotate 450"), (500.25, 400.5), [30.0, 30.0]),
        (format!("{m} /Rotate 44"), (400.5, 500.25), [30.0, 470.25]),
        (format!("{m} /Rotate 45"), (500.25, 400.5), [30.0, 30.0]),
        (format!("{m} /Rotate 46"), (500.25, 400.5), [30.0, 30.0]),
        (format!("{m} /Rotate 135"), (400.5, 500.25), [370.5, 30.0]),
        (format!("{m} /Rotate 44.4"), (400.5, 500.25), [30.0, 470.25]),
        (format!("{m} /Rotate 89.5"), (500.25, 400.5), [30.0, 30.0]),
        (format!("{m} /Rotate 91"), (500.25, 400.5), [30.0, 30.0]),
        (format!("{m} /Rotate 179.6"), (400.5, 500.25), [370.5, 30.0]),
        (format!("{m} /Rotate (90)"), (400.5, 500.25), [30.0, 470.25]),
        (format!("{m} /UserUnit 2"), (801.0, 1000.5), [60.0, 940.5]),
        (
            format!("{m} /UserUnit 0.5 /Rotate 270"),
            (250.125, 200.25),
            [235.125, 185.25],
        ),
        (format!("{m} /UserUnit -1"), (400.5, 500.25), [30.0, 470.25]),
        (
            "/MediaBox [400.5 500.25 0 0]".into(),
            (400.5, 500.25),
            [30.0, 470.25],
        ),
        ("/MediaBox [0 0 400]".into(), (612.0, 792.0), [30.0, 762.0]),
        ("/MediaBox [0 0 0 0]".into(), (612.0, 792.0), [30.0, 762.0]),
        ("".into(), (612.0, 792.0), [30.0, 762.0]),
        (
            "/MediaBox [-100 -200 200 100]".into(),
            (300.0, 300.0),
            [130.0, 70.0],
        ),
    ];
    let pages: Vec<&str> = rows
        .iter()
        .map(|(entries, _, _)| entries.as_str())
        .collect();
    let doc = frames(&pages, "");
    let rows: Vec<(&str, (f64, f64), [f64; 2])> = rows
        .iter()
        .map(|(label, size, origin)| (label.as_str(), *size, *origin))
        .collect();
    check_frames(&doc, &rows);
    // /Rotate inherited from /Pages.
    let doc = frames(&["/MediaBox [0 0 200 100]"], "/Rotate 90");
    check_frames(&doc, &[("inherited 90", (100.0, 200.0), [30.0, 30.0])]);
}

#[test]
fn n10_page_frames_the_spec_changes() {
    let m = "/MediaBox [0 0 400.5 500.25]";
    let rows: Vec<(String, (f64, f64), [f64; 2])> = vec![
        (
            format!("{m} /CropBox [500 600 700 800]"),
            (400.5, 500.25),
            [30.0, 470.25],
        ),
        (
            format!("{m} /CropBox [0 0 100 0.4]"),
            (100.0, 1.0),
            [30.0, -29.0],
        ),
        (format!("{m} /Rotate 44.6"), (400.5, 500.25), [30.0, 470.25]),
        (format!("{m} /UserUnit 0"), (400.5, 500.25), [30.0, 470.25]),
        (
            "/MediaBox [10 20 10.5 400]".into(),
            (1.0, 380.0),
            [20.0, 370.0],
        ),
    ];
    let pages: Vec<&str> = rows
        .iter()
        .map(|(entries, _, _)| entries.as_str())
        .collect();
    let doc = frames(&pages, "");
    let rows: Vec<(&str, (f64, f64), [f64; 2])> = rows
        .iter()
        .map(|(label, size, origin)| (label.as_str(), *size, *origin))
        .collect();
    check_frames(&doc, &rows);
}

/// Render sizes of a page whose MediaBox is `media`, at each scale.
fn render_sizes(media: &str, scales: &[f64]) -> Vec<(u32, u32)> {
    let doc = frames(&[media], "");
    scales
        .iter()
        .map(|&s| {
            let image = doc.render(0, s, RenderOptions::default()).unwrap();
            (image.width, image.height)
        })
        .collect()
}

#[test]
fn n11_pixel_sizes() {
    let scales = [1.0, 1.5, 1.173, 0.25, 3.0];
    let heights = [100, 150, 118, 25, 300];
    let rows: [(&str, [u32; 5]); 4] = [
        ("100", [100, 150, 118, 25, 300]),
        ("100.01", [101, 151, 118, 26, 301]),
        ("100.5", [101, 151, 118, 26, 302]),
        ("99.9995", [100, 150, 118, 25, 300]),
    ];
    for (w, widths) in rows {
        let got = render_sizes(&format!("/MediaBox [0 0 {w} 100]"), &scales);
        let want: Vec<(u32, u32)> = widths.into_iter().zip(heights).collect();
        assert_eq!(got, want, "width {w}");
    }
    assert_eq!(
        render_sizes("/MediaBox [0 0 612 100]", &scales)
            .iter()
            .map(|s| s.0)
            .collect::<Vec<_>>(),
        [612, 918, 718, 153, 1836]
    );
    // The size does not depend on the box's origin.
    for media in ["[0.4 0 100.4 100]", "[0.7 0.7 100.2 100.2]"] {
        assert_eq!(
            render_sizes(&format!("/MediaBox {media}"), &[1.0]),
            [(100, 100)],
            "{media}"
        );
    }
}

#[test]
fn n11_pixel_sizes_at_the_rounding_edge() {
    // These rows place the tolerance ε between 0.0009 and 0.0011 pixel
    // (spec 04 §7.3, the corpus gate's render-size equality).
    let scales = [1.0, 1.5, 3.0];
    let rows: [(&str, [u32; 3]); 3] = [
        ("100.0004", [100, 150, 301]),
        ("100.0009", [100, 151, 301]),
        ("100.0011", [101, 151, 301]),
    ];
    for (w, widths) in rows {
        let got: Vec<u32> = render_sizes(&format!("/MediaBox [0 0 {w} 100]"), &scales)
            .iter()
            .map(|s| s.0)
            .collect();
        assert_eq!(got, widths, "width {w}");
    }
}

// ------------------------------------------------------------------ N12 ligatures

#[test]
fn n12_ligatures_read_and_search_as_their_letters() {
    let cmap = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /L def \
                1 begincodespacerange <00> <FF> endcodespacerange\n5 beginbfchar\n\
                <1e> <FB03> <1f> <FB01> <6f> <006F> <63> <0063> <65> <0065>\n\
                endbfchar endcmap CMapName currentdict /CMap defineresource pop end end";
    let doc = page(
        "BT /F1 10 Tf 10 100 Td (o\\036ce) Tj ET BT /F1 10 Tf 10 50 Td (\\037) Tj ET",
        "<< /Font << /F1 5 0 R >> >>",
        &[
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /ToUnicode 6 0 R >>".to_vec(),
            stream("", cmap.as_bytes()),
        ],
    );
    let text = doc.structured_text(0).unwrap();
    assert_eq!(text.as_text(), "office\n\nfi\n\n");
    assert_eq!(text.search("office", MAX_SEARCH_QUADS).hits.len(), 1);
    assert_eq!(text.search("ffi", MAX_SEARCH_QUADS).hits.len(), 1);
}

// ------------------------------------------------------------------ N13 font facts

/// Cap height and descender of the 12 Latin standard faces, 1/1000 em
/// (Adobe Core 14 AFMs).
const FACES: [(&str, bool, bool, bool, bool, f64, f64); 12] = [
    // name, bold, italic, serif, monospace, cap height, descender
    ("Helvetica", false, false, false, false, 718.0, -207.0),
    ("Helvetica-Bold", true, false, false, false, 718.0, -207.0),
    (
        "Helvetica-Oblique",
        false,
        true,
        false,
        false,
        718.0,
        -207.0,
    ),
    (
        "Helvetica-BoldOblique",
        true,
        true,
        false,
        false,
        718.0,
        -207.0,
    ),
    ("Times-Roman", false, false, true, false, 662.0, -217.0),
    ("Times-Bold", true, false, true, false, 676.0, -217.0),
    ("Times-Italic", false, true, true, false, 653.0, -205.0),
    ("Times-BoldItalic", true, true, true, false, 669.0, -205.0),
    ("Courier", false, false, false, true, 562.0, -157.0),
    ("Courier-Bold", true, false, false, true, 562.0, -157.0),
    ("Courier-Oblique", false, true, false, true, 562.0, -157.0),
    (
        "Courier-BoldOblique",
        true,
        true,
        false,
        true,
        562.0,
        -157.0,
    ),
];

/// The first character's font and quad for `font` (a font dictionary)
/// showing `H` at 100 pt, baseline at y = 200 in page space.
fn first_char(font: Vec<u8>, extra: &[Vec<u8>]) -> (tandem_pdf_engine::view::TextFont, [f64; 8]) {
    let mut objects = vec![font];
    objects.extend(extra.iter().cloned());
    let doc = page(
        "BT /F1 100 Tf 10 100 Td (H) Tj ET",
        "<< /Font << /F1 5 0 R >> >>",
        &objects,
    );
    let text = doc.structured_text(0).unwrap();
    let c = chars(&text)[0].clone();
    (text.fonts()[c.font].clone(), c.quad)
}

#[test]
fn n13_standard_faces_describe_themselves() {
    for (name, bold, italic, serif, monospace, cap, descender) in FACES {
        let (font, quad) = first_char(
            format!("<< /Type /Font /Subtype /Type1 /BaseFont /{name} >>").into_bytes(),
            &[],
        );
        assert_eq!(
            (font.bold, font.italic, font.serif, font.monospace),
            (bold, italic, serif, monospace),
            "{name}"
        );
        // ul.y is the top, ll.y the bottom; baseline at 200, 100 pt.
        let (top, bottom) = (quad[1], quad[5]);
        assert!(top <= 200.0 - cap / 10.0 + 1e-6, "{name}: top {top}");
        assert!(
            bottom >= 200.0 - descender / 10.0 - 1e-6,
            "{name}: bottom {bottom}"
        );
        assert!(bottom - top <= 160.0, "{name}: {top}..{bottom}");
    }
    for name in ["Symbol", "ZapfDingbats"] {
        let (_, quad) = first_char(
            format!("<< /Type /Font /Subtype /Type1 /BaseFont /{name} >>").into_bytes(),
            &[],
        );
        assert!(
            quad[5] - quad[1] <= 160.0 && quad[5] > quad[1],
            "{name}: {quad:?}"
        );
    }
}

#[test]
fn n13_descriptor_flags_and_weight_describe_the_font() {
    let font = |descriptor: &str| {
        first_char(
            format!(
                "<< /Type /Font /Subtype /Type1 /BaseFont /Unknown /FirstChar 72 /LastChar 72 \
                 /Widths [700] /FontDescriptor << /Type /FontDescriptor /FontName /Unknown \
                 /FontBBox [0 -200 1000 800] /ItalicAngle 0 /Ascent 800 /Descent -200 \
                 /CapHeight 700 /StemV 80 {descriptor} >> >>"
            )
            .into_bytes(),
            &[],
        )
        .0
    };
    let serif_italic = font("/Flags 98");
    assert!(
        serif_italic.serif && serif_italic.italic,
        "{serif_italic:?}"
    );
    let forced = font("/Flags 262176");
    assert!(forced.bold, "{forced:?}");
    let heavy = font("/Flags 32 /FontWeight 700");
    assert!(heavy.bold, "{heavy:?}");
    let fixed = font("/Flags 33");
    assert!(fixed.monospace, "{fixed:?}");
    let plain = font("/Flags 32");
    assert!(
        !plain.bold && !plain.italic && !plain.serif && !plain.monospace,
        "{plain:?}"
    );
}

/// A TrueType program of empty glyphs mapping H (3,1) to glyph 1, with
/// OS/2 bold and `post` fixed pitch.
fn bold_fixed_truetype() -> Vec<u8> {
    let be16s =
        |values: &[u16]| -> Vec<u8> { values.iter().flat_map(|v| v.to_be_bytes()).collect() };
    let mut head = vec![0u8; 54];
    head[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    head[44..46].copy_from_slice(&1u16.to_be_bytes()); // macStyle bold
    let mut hhea = vec![0u8; 36];
    hhea[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    hhea[4..6].copy_from_slice(&800i16.to_be_bytes());
    hhea[6..8].copy_from_slice(&(-200i16).to_be_bytes());
    hhea[34..36].copy_from_slice(&2u16.to_be_bytes());
    let mut maxp = vec![0u8; 6];
    maxp[0..4].copy_from_slice(&0x0000_5000u32.to_be_bytes());
    maxp[4..6].copy_from_slice(&2u16.to_be_bytes());
    let mut os2 = vec![0u8; 96];
    os2[0..2].copy_from_slice(&4u16.to_be_bytes());
    os2[4..6].copy_from_slice(&700u16.to_be_bytes()); // usWeightClass
    os2[62..64].copy_from_slice(&0x0020u16.to_be_bytes()); // fsSelection bold
    os2[68..70].copy_from_slice(&800i16.to_be_bytes()); // sTypoAscender
    os2[70..72].copy_from_slice(&(-200i16).to_be_bytes()); // sTypoDescender
    let mut post = vec![0u8; 32];
    post[0..4].copy_from_slice(&0x0003_0000u32.to_be_bytes());
    post[12..16].copy_from_slice(&1u32.to_be_bytes()); // isFixedPitch
    let mut cmap = be16s(&[0, 1, 3, 1]);
    cmap.extend(12u32.to_be_bytes());
    // Format 4: H -> glyph 1, then the closing segment.
    cmap.extend(be16s(&[4, 32, 0, 4, 4, 1, 0]));
    cmap.extend(be16s(&[0x48, 0xFFFF, 0, 0x48, 0xFFFF]));
    cmap.extend(be16s(&[1u16.wrapping_sub(0x48), 1, 0, 0]));
    let tables: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"OS/2", os2),
        (b"cmap", cmap),
        (b"glyf", vec![]),
        (b"head", head),
        (b"hhea", hhea),
        (b"hmtx", be16s(&[600, 0, 600, 0])),
        (b"loca", vec![0; 6]),
        (b"maxp", maxp),
        (b"post", post),
    ];
    let mut font = Vec::new();
    font.extend(0x0001_0000u32.to_be_bytes());
    font.extend(be16s(&[tables.len() as u16, 0, 0, 0]));
    let mut offset = 12 + 16 * tables.len();
    for (tag, data) in &tables {
        font.extend(*tag);
        for v in [0, offset as u32, data.len() as u32] {
            font.extend(v.to_be_bytes());
        }
        offset += data.len().next_multiple_of(4);
    }
    for (_, data) in &tables {
        font.extend(data);
        font.resize(font.len().next_multiple_of(4), 0);
    }
    font
}

#[test]
fn n13_an_embedded_program_describes_its_weight_and_pitch() {
    let (font, _) = first_char(
        b"<< /Type /Font /Subtype /TrueType /BaseFont /ABCDEF+Plain /FirstChar 72 \
          /LastChar 72 /Widths [600] /Encoding /WinAnsiEncoding /FontDescriptor << \
          /Type /FontDescriptor /FontName /ABCDEF+Plain /Flags 32 /FontBBox [0 -200 1000 800] \
          /ItalicAngle 0 /Ascent 800 /Descent -200 /CapHeight 700 /StemV 80 \
          /FontFile2 6 0 R >> >>"
            .to_vec(),
        &[stream("", &bold_fixed_truetype())],
    );
    assert!(font.bold && font.monospace, "{font:?}");
    assert_eq!(font.name, "ABCDEF+Plain");
}

// ------------------------------------------------------------------ N14, N15 direction

#[test]
fn n14_vertical_writing_reads_down_one_line() {
    let cmap = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /V def \
                1 begincodespacerange <0000> <FFFF> endcodespacerange\n3 beginbfchar\n\
                <0001> <65E5> <0002> <672C> <0003> <8A9E>\n\
                endbfchar endcmap CMapName currentdict /CMap defineresource pop end end";
    let doc = page(
        "BT /F1 20 Tf 150 250 Td <000100020003> Tj ET",
        "<< /Font << /F1 5 0 R >> >>",
        &[
            b"<< /Type /Font /Subtype /Type0 /BaseFont /Mincho /Encoding /Identity-V \
              /DescendantFonts [6 0 R] /ToUnicode 7 0 R >>"
                .to_vec(),
            b"<< /Type /Font /Subtype /CIDFontType0 /BaseFont /Mincho \
              /CIDSystemInfo << /Registry (Adobe) /Ordering (Japan1) /Supplement 2 >> \
              /FontDescriptor << /Type /FontDescriptor /FontName /Mincho /Flags 4 \
              /FontBBox [0 -120 1000 880] /ItalicAngle 0 /Ascent 880 /Descent -120 \
              /CapHeight 700 /StemV 80 >> >>"
                .to_vec(),
            stream("", cmap.as_bytes()),
        ],
    );
    let text = doc.structured_text(0).unwrap();
    assert_eq!(text.as_text(), "日本語\n\n");
    let line = &text.blocks()[0].lines[0];
    assert!(
        (line.dir[0]).abs() < 1e-6 && (line.dir[1] - 1.0).abs() < 1e-6,
        "{:?}",
        line.dir
    );
    let ys: Vec<f64> = line.chars.iter().map(|c| c.origin[1]).collect();
    assert!(ys.windows(2).all(|w| w[0] < w[1]), "{ys:?}");
}

#[test]
fn n15_text_turned_a_quarter_reads_up_on_its_own_line() {
    let doc = helvetica(
        "BT /F1 10 Tf 10 250 Td (Across) Tj ET BT /F1 10 Tf 0 1 -1 0 200 50 Tm (Up) Tj ET",
    );
    let text = doc.structured_text(0).unwrap();
    assert_eq!(text.as_text(), "Across\n\nUp\n\n");
    let up = text
        .blocks()
        .iter()
        .flat_map(|b| &b.lines)
        .find(|l| l.chars[0].c == 'U')
        .unwrap();
    assert!(
        up.dir[0].abs() < 1e-6 && (up.dir[1] + 1.0).abs() < 1e-6,
        "{:?}",
        up.dir
    );
}

// ------------------------------------------------------------------ N16 visual lines

#[test]
fn n16_runs_of_one_visual_line_are_two_summaries() {
    let doc = helvetica("BT /F1 10 Tf 10 100 Td (Red) Tj 1 0 0 rg (Blue) Tj ET");
    let text = doc.structured_text(0).unwrap();
    let summaries = text.line_summaries();
    assert_eq!(summaries.len(), 2);
    assert_eq!(
        summaries
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>(),
        ["Red", "Blue"]
    );
}

#[test]
fn n16_spans_text_keeps_one_visual_line() {
    let doc = helvetica("BT /F1 10 Tf 10 100 Td (Red) Tj 1 0 0 rg (Blue) Tj ET");
    assert_eq!(doc.structured_text(0).unwrap().spans_text(), "RedBlue\n\n");
}

// ------------------------------------------------------------------ search rules (2.5)

#[test]
fn search_rules() {
    let doc = helvetica(
        "BT /F1 10 Tf 10 250 Td (The Cat sat on the cat-) Tj 0 -14 Td (alog with  double  spaces) Tj \
         0 -14 Td (CAT cat Cat caf\\351) Tj ET",
    );
    let text = doc.structured_text(0).unwrap();
    let count = |needle: &str| text.search(needle, MAX_SEARCH_QUADS).hits.len();
    let counts: BTreeMap<&str, usize> = [
        "cat",
        "Cat",
        "CAT",
        "catalog",
        "cat-alog",
        "with double",
        "spaces CAT",
        "cafe",
        "café",
        "",
    ]
    .into_iter()
    .map(|n| (n, count(n)))
    .collect();
    let want: BTreeMap<&str, usize> = [
        ("cat", 2),
        ("Cat", 2),
        ("CAT", 1),
        ("catalog", 1),
        ("cat-alog", 0),
        ("with double", 1),
        ("spaces CAT", 1),
        ("cafe", 0),
        ("café", 1),
        ("", 0),
    ]
    .into_iter()
    .collect();
    assert_eq!(counts, want);
    // Hits never overlap; the budget never cuts a hit.
    let doc = helvetica("BT /F1 10 Tf 10 250 Td (aaaa) Tj ET");
    let text = doc.structured_text(0).unwrap();
    assert_eq!(text.search("aa", MAX_SEARCH_QUADS).hits.len(), 2);
    let capped = text.search_hits("aa", 1);
    assert_eq!((capped.hits.len(), capped.truncated), (1, true));
    let none = text.search_hits("aa", 0);
    assert!(none.hits.is_empty() && none.truncated);
}
