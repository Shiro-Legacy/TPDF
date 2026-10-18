// SPDX-License-Identifier: AGPL-3.0-or-later
//! Structured text of synthetic pages (spec 04): clipping-mode text, clip
//! visibility, right-to-left lines, text runs, ActualText, structure
//! lookups, text drawn before a font is set, and which show drew a glyph.
use std::time::{Duration, Instant};
use tandem_pdf_engine::view::{MAX_SEARCH_QUADS, RenderOptions, ViewDocument};

/// A classic-xref PDF of `objects` (numbered from 1).
fn pdf(objects: &[String]) -> Vec<u8> {
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

fn stream(dict: &str, data: &str) -> String {
    format!(
        "<< {dict} /Length {} >>\nstream\n{data}\nendstream",
        data.len()
    )
}

/// One 300 × 300 page with `resources` and `content`; `extra` objects are
/// numbered from 5.
fn page(content: &str, resources: &str, extra: &[String]) -> ViewDocument {
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Resources {resources} /Contents 4 0 R >>"
        ),
        stream("", content),
    ];
    objects.extend(extra.iter().cloned());
    ViewDocument::open(pdf(&objects), "").unwrap()
}

fn cmap(entries: &str, count: usize) -> String {
    stream(
        "",
        &format!(
            "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /X def \
             1 begincodespacerange <00> <FF> endcodespacerange\n{count} beginbfchar\n{entries}\n\
             endbfchar endcmap CMapName currentdict /CMap defineresource pop end end"
        ),
    )
}

/// Helvetica as /F1 whose ToUnicode reads code 0x1F as "fi".
fn ligature_page(content: &str) -> ViewDocument {
    page(
        content,
        "<< /Font << /F1 5 0 R >> >>",
        &[
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /ToUnicode 6 0 R >>".into(),
            cmap("<1f> <00660069> <41> <0041>", 2),
        ],
    )
}

/// (`as_text`, `spans_text`) of the first page.
fn texts(doc: &ViewDocument) -> (String, String) {
    let text = doc.structured_text(0).unwrap();
    (text.as_text(), text.spans_text())
}

/// Whether the page renders anything but white.
fn has_ink(doc: &ViewDocument) -> bool {
    let image = doc.render(0, 1.0, RenderOptions::default()).unwrap();
    image
        .pixels
        .chunks(4)
        .any(|px| px[..3].iter().any(|&v| v < 200))
}

#[test]
fn clipping_text_is_extracted_once() {
    // Fill (or stroke) and clip: the shown text once, its ligature as its
    // components.
    for mode in 4..=6 {
        let doc = ligature_page(&format!("BT /F1 10 Tf {mode} Tr 10 100 Td (A\\037B) Tj ET"));
        assert_eq!(texts(&doc).0, "AfiB\n\n", "mode {mode}");
        let doc = ligature_page(&format!(
            "BT /F1 20 Tf {mode} Tr 10 100 Td (Hello) Tj 0 -30 Td (World) Tj ET"
        ));
        let text = doc.structured_text(0).unwrap();
        assert_eq!(text.as_text(), "Hello\nWorld\n\n", "mode {mode}");
        assert_eq!(text.search("Hello", MAX_SEARCH_QUADS).hits.len(), 1);
    }
    // Clip-only text in a closed text object.
    let doc = ligature_page(
        "q BT /F1 10 Tf 7 Tr 10 100 Td (AB) Tj ET 0 0 1 rg 0 0 300 300 re f Q \
         BT /F1 10 Tf 10 50 Td (CD) Tj ET",
    );
    assert_eq!(texts(&doc).0, "AB\n\nCD\n\n");
    // The text that only clips a fill is searchable (the common use), once.
    let doc = ligature_page("q BT /F1 30 Tf 7 Tr 20 100 Td (AB) Tj ET 0 0 1 rg 0 0 300 300 re f Q");
    let hits = doc
        .structured_text(0)
        .unwrap()
        .search("AB", MAX_SEARCH_QUADS);
    assert_eq!(hits.hits.len(), 1);
}

#[test]
#[ignore = "clean-room: 04§3.1 clipping-mode text is extracted where it is drawn, in content order"]
fn clipping_text_is_extracted_where_it_is_drawn() {
    let cases = [
        ("BT /F1 10 Tf 7 Tr 10 100 Td (A\\037B) Tj ET", "AfiB\n\n"),
        (
            "BT /F1 10 Tf 7 Tr 10 100 Td (AB) Tj 4 Tr (CD) Tj ET",
            "ABCD\n\n",
        ),
        (
            "BT /F1 10 Tf 7 Tr 10 100 Td (AB) Tj 0 Tr (CD) Tj ET",
            "ABCD\n\n",
        ),
        (
            "BT /F1 10 Tf 7 Tr 10 100 Td (AB) Tj 3 Tr (CD) Tj ET",
            "ABCD\n\n",
        ),
        // Without ET the clip-only text is still text.
        ("BT /F1 10 Tf 7 Tr 10 100 Td (AB) Tj", "AB\n\n"),
    ];
    for (content, want) in cases {
        assert_eq!(texts(&ligature_page(content)).0, want, "{content}");
    }
    let doc = ligature_page("BT /F1 10 Tf 7 Tr 10 100 Td (AB) Tj 4 Tr (CD) Tj ET");
    let text = doc.structured_text(0).unwrap();
    assert_eq!(text.search("ABCD", MAX_SEARCH_QUADS).hits.len(), 1);
}

#[test]
fn glyphs_wholly_outside_the_clip_are_not_visible() {
    // The space is kerned back, so a dropped space leaves no gap behind.
    // Glyphs partly inside the clip are visible; how an inkless glyph (the
    // space) is judged is left open, so spaces are compared loosely.
    let shown = "BT /F1 10 Tf 10 100 Td [(A) ( ) 278 (B)] TJ ET Q";
    let squeeze = |s: &str| s.split_whitespace().collect::<String>();
    let cases = [
        ("q 10 99 100 5 re W n", "AB"),
        ("q 10 101 100 5 re W n", "AB"),
        ("q 10 98 6.5 9 re W n", "A"),
        ("q 10 98 6.8 9 re W n", "A"),
        // Below the baseline: the letters' ink lies wholly above it.
        ("q 10 90 100 9.9 re W n", ""),
    ];
    for (clip, want) in cases {
        let doc = ligature_page(&format!("{clip} {shown}"));
        assert_eq!(squeeze(&texts(&doc).0), want, "{clip}");
    }
    let doc = ligature_page(&format!("q 10 99 100 5 re W n {shown}"));
    assert_eq!(texts(&doc).0, "A B\n\n");
    // Search still reads everything drawn.
    let doc = ligature_page(&format!("q 10 90 100 9.9 re W n {shown}"));
    let text = doc.structured_text(0).unwrap();
    assert_eq!(text.search("A", MAX_SEARCH_QUADS).hits.len(), 1);
}

/// Helvetica whose ToUnicode reads a..e as Hebrew letters, f as an Arabic
/// alef and g as an Old Hungarian letter.
fn hebrew_page(content: &str) -> ViewDocument {
    let entries = "<61> <05d0>\n<62> <05d1>\n<63> <05d2>\n<64> <05d3>\n<65> <05d4>\n\
                   <66> <0627>\n<67> <d803dc80>";
    page(
        content,
        "<< /Font << /F1 5 0 R /F2 7 0 R >> >>",
        &[
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /ToUnicode 6 0 R >>".into(),
            cmap(entries, 7),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman /ToUnicode 6 0 R >>".into(),
        ],
    )
}

/// Text with A..E read as U+05D0..U+05D4 and G as U+10C80.
fn hebrew(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='E' => char::from_u32(0x5d0 + (c as u32 - 'A' as u32)).unwrap(),
            'G' => '\u{10c80}',
            c => c,
        })
        .collect()
}

#[test]
fn right_to_left_words_read_in_logical_order() {
    // Glyphs drawn left to right (visual order); letters as drawn: a..e are
    // U+05D0..U+05D4 (A..E below).
    let shown = |s: &str| format!("BT /F1 10 Tf 10 100 Td ({s}) Tj ET");
    let cases = [
        (shown("abc de"), "ED CBA\n\n"),
        // The period at the visual left end of the first word is its
        // logical end.
        (shown("abc. de"), "ED .CBA\n\n"),
        (shown("ggg ggg"), "GGG GGG\n\n"),
    ];
    for (content, want) in cases {
        let doc = hebrew_page(&content);
        assert_eq!(texts(&doc).0, hebrew(want), "{content}");
    }
}

#[test]
#[ignore = "clean-room: 04§3.6 right-to-left lines read by the Unicode bidirectional algorithm (T2)"]
fn right_to_left_lines_read_in_logical_order() {
    let shown = |s: &str| format!("BT /F1 10 Tf 10 100 Td ({s}) Tj ET");
    let cases = [
        (shown("abc de"), "ED CBA"),
        (shown("abc 12 de"), "ED 12 CBA"),
        (shown("abc. de"), "ED .CBA"),
        (shown("XY abc de ZW"), "XY ED CBA ZW"),
        (shown("(abc)"), "(CBA)"),
    ];
    for (content, want) in cases {
        let doc = hebrew_page(&content);
        let text = doc.structured_text(0).unwrap();
        assert_eq!(text.as_text(), format!("{}\n\n", hebrew(want)), "{content}");
        // Each word is found as read.
        for word in want.split(' ') {
            let word = hebrew(word);
            assert_eq!(
                text.search(&word, MAX_SEARCH_QUADS).hits.len(),
                1,
                "{content}: {word}"
            );
        }
    }
}

/// Helvetica as /F1 and Times-Roman as /F2.
fn two_font_page(content: &str) -> ViewDocument {
    page(
        content,
        "<< /Font << /F1 5 0 R /F2 6 0 R >> >>",
        &[
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman >>".into(),
        ],
    )
}

#[test]
fn text_runs_break_at_font_size_colour_mode_and_text_object() {
    // `AB` at 10 pt Helvetica is 13.34 pt wide; `CD` follows it on the line.
    let split = [
        "/F2 10 Tf",
        "/F1 12 Tf",
        "1 0 0 rg",
        "2 Tr",
        "ET BT /F1 10 Tf 23.34 100 Td",
    ];
    let joined = ["0 Tc", "0 Tw", "100 Tz", "13.34 0 Td"];
    for (op, runs) in split
        .iter()
        .map(|op| (op, ["AB", "CD"].as_slice()))
        .chain(joined.iter().map(|op| (op, ["ABCD"].as_slice())))
    {
        let doc = two_font_page(&format!("BT /F1 10 Tf 10 100 Td (AB) Tj {op} (CD) Tj ET"));
        let text = doc.structured_text(0).unwrap();
        // The reading layout is never split by any of them.
        assert_eq!(text.as_text(), "ABCD\n\n", "{op}");
        let lines: Vec<String> = text
            .line_summaries()
            .into_iter()
            .map(|l| l.text.trim().to_string())
            .collect();
        assert_eq!(lines, runs, "{op}");
        let span_lines: usize = text.span_blocks().iter().map(|b| b.lines.len()).sum();
        assert_eq!(span_lines, runs.len(), "{op}");
    }
    // Setting the same mode again is not a change.
    let doc = two_font_page("BT /F1 10 Tf 3 Tr 10 100 Td (AB) Tj 0 Tr 3 Tr (CD) Tj ET");
    assert_eq!(doc.structured_text(0).unwrap().line_summaries().len(), 1);
}

fn image_page(content: &str) -> ViewDocument {
    let image = stream(
        "/Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceGray \
         /BitsPerComponent 8 /Filter /ASCIIHexDecode",
        "80402010>",
    );
    page(
        content,
        "<< /Font << /F1 6 0 R >> /XObject << /Im 5 0 R >> >>",
        &[
            image,
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        ],
    )
}

#[test]
fn actual_text_over_no_text_is_placed_on_the_images_it_covers() {
    let over_image =
        image_page("/Span << /ActualText (Bookkeeper) >> BDC q 50 0 0 20 10 100 cm /Im Do Q EMC");
    // Repeated letters stay: the characters do not read as fake bold.
    assert_eq!(texts(&over_image).0, "Bookkeeper\n\n");
    // At the image's bottom left (page space, y down), an em of its size.
    let text = over_image.structured_text(0).unwrap();
    let first = &text.blocks()[0].lines[0].chars[0];
    assert!((first.origin[0] - 10.0).abs() < 0.01, "{:?}", first.origin);
    assert!((first.origin[1] - 200.0).abs() < 0.01, "{:?}", first.origin);
    assert!((first.size - 1000f64.sqrt()).abs() < 1e-6);
    // Painting nothing at all, the ActualText is dropped.
    assert_eq!(
        texts(&image_page("/Span << /ActualText (Gone) >> BDC EMC")).0,
        ""
    );
}

#[test]
#[ignore = "clean-room: 04§3.7 ActualText over painted paths is placed on their bounds"]
fn actual_text_over_a_path_is_placed_on_its_bounds() {
    let over_path = image_page("/Span << /ActualText (Hello) >> BDC 10 100 50 20 re f EMC");
    let text = over_path.structured_text(0).unwrap();
    assert_eq!(text.as_text(), "Hello\n\n");
    let first = &text.blocks()[0].lines[0].chars[0];
    assert!((first.origin[0] - 10.0).abs() < 0.01, "{:?}", first.origin);
    assert!((first.origin[1] - 200.0).abs() < 0.01, "{:?}", first.origin);
}

#[test]
fn structure_elements_are_found_by_mcid_in_linear_time() {
    // 6,000 marked-content sequences; the last one's element has ActualText,
    // behind a null entry. Resolving every element before each one took
    // 8.7 s natively for such a page. (The element is the one at the
    // MCID's index of the parent-tree array, ISO 32000-2 §14.7.5.4.)
    const N: usize = 6000;
    let mut content = String::from("BT /F1 0.04 Tf 10 100 Td ");
    for mcid in 0..N {
        content.push_str(&format!("/P << /MCID {mcid} >> BDC (x) Tj EMC "));
    }
    content.push_str("ET");
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R /StructTreeRoot 5 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /StructParents 0 \
         /Resources << /Font << /F1 6 0 R >> >> /Contents 4 0 R >>"
            .to_string(),
        stream("", &content),
    ];
    let first = 8;
    let parents: Vec<String> = (0..N)
        .map(|i| match i {
            i if i == N - 2 => "null".to_string(),
            i => format!("{} 0 R", first + i),
        })
        .collect();
    objects.push("<< /Type /StructTreeRoot /ParentTree << /Nums [0 7 0 R] >> >>".to_string());
    objects.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string());
    objects.push(format!("[{}]", parents.join(" ")));
    for i in 0..N {
        objects.push(if i == N - 1 {
            format!("<< /Type /StructElem /S /Span /K {i} /ActualText (Last) >>")
        } else {
            format!("<< /Type /StructElem /S /Span /K {i} >>")
        });
    }
    let doc = ViewDocument::open(pdf(&objects), "").unwrap();
    let started = Instant::now();
    let text = doc.structured_text(0).unwrap().as_text();
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    assert!(
        text.ends_with("xxLast\n\n"),
        "{:?}",
        &text[text.len() - 10..]
    );
    assert_eq!(text.matches('x').count(), N - 1);
}

#[test]
fn text_shown_before_any_font_is_set_is_extracted_as_drawn() {
    // hayro draws `Tj` and `TJ` without a font in Helvetica (ASCII only),
    // and `'` and `"` not at all; the text is what the page shows.
    // Text matrices of 40 points: the font size is 1 without a `Tf`.
    let cases = [
        ("BT 40 0 0 40 10 100 Tm (AB) Tj ET", "AB\n\n"),
        (
            "q BT /F1 10 Tf ET Q BT 40 0 0 40 10 50 Tm (CD) Tj ET",
            "CD\n\n",
        ),
        ("BT 40 0 0 40 10 100 Tm [(A) 50 (B)] TJ ET", "AB\n\n"),
        ("BT 40 0 0 40 10 150 Tm 1 TL (AB) ' ET", ""),
        ("BT 40 0 0 40 10 150 Tm 1 TL 0 0 (AB) \" ET", ""),
        (
            "BT 40 0 0 40 10 150 Tm 1 TL (AB) Tj (CD) ' ET",
            "AB\nCD\n\n",
        ),
        (
            "BT 40 0 0 40 10 150 Tm 1 TL (AB) ' ET BT /Missing 40 Tf 10 50 Td (CD) Tj ET",
            "CD\n\n",
        ),
        ("BT /F9 40 Tf 10 100 Td (AB) Tj ET", "AB\n\n"),
        ("BT 40 0 0 40 10 100 Tm 11 Tf (AB) Tj ET", "AB\n\n"),
    ];
    for (content, want) in cases {
        let doc = ligature_page(content);
        assert_eq!(texts(&doc).0, want, "{content}");
        assert_eq!(has_ink(&doc), !want.is_empty(), "{content}");
    }
    // A form draws with the font of the content that draws it.
    let with_form = |content: &str, form: &str| {
        page(
            content,
            "<< /Font << /F1 5 0 R >> /XObject << /X 6 0 R >> >>",
            &[
                "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
                stream("/Type /XObject /Subtype /Form /BBox [0 0 300 300]", form),
            ],
        )
    };
    let form = "BT 40 0 0 40 10 100 Tm (AB) Tj ET";
    for content in ["BT /F1 1 Tf ET /X Do", "/X Do"] {
        let doc = with_form(content, form);
        assert_eq!(texts(&doc).0, "AB\n\n", "{content}");
        assert!(has_ink(&doc), "{content}");
    }
}

/// Helvetica as /F1 and a form /Fm showing `form` (with a bounding box
/// when `bbox`).
fn form_page(content: &str, form: &str, bbox: bool) -> ViewDocument {
    let bbox = if bbox { "/BBox [0 0 300 300]" } else { "" };
    page(
        content,
        "<< /Font << /F1 5 0 R >> /XObject << /Fm 6 0 R >> >>",
        &[
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
            stream(&format!("/Type /XObject /Subtype /Form {bbox}"), form),
        ],
    )
}

#[test]
fn each_glyph_run_reads_the_show_operation_that_drew_it() {
    // hayro draws no form without a bounding box: its shows must not lend
    // their text to the page's (which read "XY" at CD's place, and "CD" at
    // EF's).
    let shows = "BT /F1 12 Tf 10 50 Td (CD) Tj ET BT /F1 12 Tf 10 30 Td (EF) Tj ET";
    let form = "BT /F1 12 Tf 10 100 Td (XY) Tj ET";
    let doc = form_page(&format!("/Fm Do {shows}"), form, false);
    assert_eq!(texts(&doc).0, "CD\n\nEF\n\n");
    let text = doc.structured_text(0).unwrap();
    assert!(text.search("XY", MAX_SEARCH_QUADS).hits.is_empty());
    let ef = text.search("EF", MAX_SEARCH_QUADS);
    assert_eq!(ef.hits.len(), 1);
    // Page space, y down: EF's baseline is at 270.
    assert!((ef.hits[0][0][5] - 270.0).abs() < 4.0, "{:?}", ef.hits);
    let doc = form_page(&format!("/Fm Do {shows}"), form, true);
    assert_eq!(texts(&doc).0, "XY\n\nCD\n\nEF\n\n");
    // Shows after one that was not drawn, in the same text object.
    let doc = form_page(
        "/Fm Do BT /F1 12 Tf 10 50 Td (CD) Tj (EF) Tj ET",
        "BT /F1 12 Tf 10 100 Td (AB) Tj (XY) Tj ET",
        false,
    );
    assert_eq!(texts(&doc).0, "CDEF\n\n");
    // hayro reads an operator's last operands: `1 7 Tr` is mode 7, drawn
    // as clipping text.
    let doc = form_page(
        "q BT /F1 10 Tf 1 7 Tr 10 200 Td (AB) Tj ET Q BT /F1 10 Tf 0 Tr 10 50 Td (CD) Tj ET",
        "",
        true,
    );
    assert_eq!(texts(&doc).0, "AB\n\nCD\n\n");
    // Mode 7 set by the page reaches the form.
    let doc = form_page(
        "q BT /F1 10 Tf 7 Tr 10 200 Td (AB) Tj ET Q q 7 Tr /Fm Do Q \
         BT /F1 10 Tf 0 Tr 10 50 Td (CD) Tj ET",
        "BT /F1 10 Tf 10 100 Td (XY) Tj ET",
        true,
    );
    assert_eq!(texts(&doc).0, "AB\n\nXY\n\nCD\n\n");
}

#[test]
#[ignore = "clean-room: 04§3.1 mode 7 text a form sets is extracted"]
fn mode_7_text_a_form_sets_is_extracted() {
    let doc = form_page(
        "/Fm Do BT /F1 10 Tf 10 50 Td (CD) Tj ET",
        "BT /F1 10 Tf 7 Tr 10 100 Td (XY) Tj ET",
        true,
    );
    assert_eq!(texts(&doc).0, "XY\n\nCD\n\n");
}
